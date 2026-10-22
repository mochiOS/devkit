//! Files.app panels in the isolated Linux desktop session.
use super::*;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::{UnixListener, UnixStream};
use viewkit::platform::linux::preview::{Connection, Packet, SOCKET_ENV};

struct ReplyDirectory(PathBuf);
impl Drop for ReplyDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0.join("reply.sock"));
        let _ = fs::remove_dir(&self.0);
    }
}

pub(super) fn show(panel: &FilePanel) -> Result<(), String> {
    let fail = |error: io::Error| error.to_string();
    let socket = std::env::var_os(SOCKET_ENV)
        .ok_or_else(|| "The preview desktop is unavailable.".to_owned())?;
    let broker_socket = PathBuf::from(socket);
    let parent = broker_socket
        .parent()
        .ok_or("Invalid preview socket.")?
        .join("panels")
        .join(std::process::id().to_string());
    fs::create_dir_all(&parent).map_err(fail)?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let nonce = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let directory = parent.join(nonce.to_string());
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .map_err(fail)?;
    let cleanup = ReplyDirectory(directory);
    let reply_socket = cleanup.0.join("reply.sock");
    let listener = UnixListener::bind(&reply_socket).map_err(fail)?;
    listener.set_nonblocking(true).map_err(fail)?;
    let configuration = configuration(panel)?;
    let mut broker =
        Connection::new(UnixStream::connect(broker_socket).map_err(fail)?).map_err(fail)?;
    broker
        .send(Packet::FilePanelRequest {
            configuration,
            reply_socket,
        })
        .map_err(fail)?;
    panel.state.pending_replace.borrow_mut().take();
    let mut selection: Option<Connection> = None;
    loop {
        // The shell holds this request until the panel exits, including cancellation.
        match broker.poll() {
            Ok(packets) => {
                for packet in packets {
                    if let Packet::FilePanelOperation { error: Some(error) } = packet {
                        return Err(error);
                    }
                }
            }
            Err(_) => {
                if let Some(handler) = &panel.state.cancel_handler {
                    handler();
                }
                return Ok(());
            }
        }
        if selection.is_none() {
            match listener.accept() {
                Ok((stream, _)) => selection = Some(Connection::new(stream).map_err(fail)?),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(fail(e)),
            }
        }
        if let Some(connection) = &mut selection {
            for packet in connection.poll().map_err(fail)? {
                let Packet::FilePanelSelection { path } = packet else {
                    return Err("Invalid file panel response.".to_owned());
                };
                let Some(path) = path else {
                    connection
                        .send(Packet::FilePanelOperation { error: None })
                        .map_err(fail)?;
                    connection.flush().map_err(fail)?;
                    if let Some(handler) = &panel.state.cancel_handler {
                        handler();
                    }
                    return Ok(());
                };
                let result = validate_selection(&panel.state, &path)
                    .and_then(|path| (panel.state.handler)(&path));
                let succeeded = result.is_ok();
                connection
                    .send(Packet::FilePanelOperation {
                        error: result.err(),
                    })
                    .map_err(fail)?;
                connection.flush().map_err(fail)?;
                if succeeded {
                    return Ok(());
                }
                // Files keeps its navigation and shows the operation error for retry.
                selection = None;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(8));
    }
}

fn configuration(panel: &FilePanel) -> Result<Vec<u8>, String> {
    let directory = panel
        .state
        .directory
        .borrow()
        .to_string_lossy()
        .into_owned();
    let allowed = panel
        .state
        .allowed_content_types
        .borrow()
        .iter()
        .map(ContentType::identifier)
        .collect::<Vec<_>>()
        .join("\x1f");
    let suggested_name = panel.state.name.value();
    let executable = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .into_owned();
    let mut configuration = vec![0; mochios_workspace_protocol::MAX_MESSAGE_LEN];
    let length = mochios_workspace_protocol::encode_file_panel_request(
        mochios_workspace_protocol::FilePanelRequest {
            mode: if panel.state.mode == Mode::Open {
                mochios_workspace_protocol::FILE_PANEL_MODE_OPEN
            } else {
                mochios_workspace_protocol::FILE_PANEL_MODE_SAVE
            },
            title: &panel.title,
            initial_directory: &directory,
            suggested_name: &suggested_name,
            allowed_content_types: &allowed,
            executable: &executable,
        },
        &mut configuration,
    )
    .map_err(|_| "The file panel request could not be encoded.".to_owned())?;
    configuration.truncate(length);
    Ok(configuration)
}

fn validate_selection(state: &PanelState, path: &Path) -> Result<PathBuf, String> {
    let destination = match state.mode {
        Mode::Open => fs::canonicalize(path).map_err(|e| e.to_string())?,
        Mode::Save { .. } => {
            let name = path
                .file_name()
                .and_then(|v| v.to_str())
                .filter(|v| valid_file_name(v))
                .ok_or("Enter a valid file name.")?;
            let parent = fs::canonicalize(path.parent().ok_or("Select a folder.")?)
                .map_err(|e| e.to_string())?;
            let destination = parent.join(name);
            // Reject symlinks escaping the permitted root, including existing destinations.
            if fs::symlink_metadata(&destination).is_ok() {
                fs::canonicalize(destination).map_err(|e| e.to_string())?
            } else {
                destination
            }
        }
    };
    if !destination.starts_with(&state.root) {
        return Err("This folder is outside the permitted location.".to_owned());
    }
    if destination.is_dir() {
        return Err("Select a file, rather than a folder.".to_owned());
    }
    let allowed = state.allowed_content_types.borrow();
    if !allowed.is_empty()
        && !allowed
            .iter()
            .any(|kind| ContentType::for_path(&destination).conforms_to(kind))
    {
        return Err("This file type is not allowed.".to_owned());
    }
    if let Mode::Save {
        confirms_replacement: true,
    } = state.mode
    {
        if destination.exists() && state.pending_replace.borrow().as_ref() != Some(&destination) {
            *state.pending_replace.borrow_mut() = Some(destination);
            return Err(
                "A file already uses this name. Select Save again to replace it.".to_owned(),
            );
        }
    }
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_encodes_open_and_save_requests_for_files() {
        let root = super::super::tests::temporary_directory("preview-configuration");
        let open = OpenPanel::new(
            OpenPanelOptions {
                root_directory: Some(root.clone()),
                ..Default::default()
            },
            |_| Ok(()),
        );
        let bytes = configuration(&open.0).unwrap();
        let request = mochios_workspace_protocol::decode_file_panel_request(&bytes).unwrap();
        assert_eq!(
            request.mode,
            mochios_workspace_protocol::FILE_PANEL_MODE_OPEN
        );
        assert!(request.suggested_name.is_empty());
        assert!(!request.executable.is_empty());
        let save = SavePanel::new(
            SavePanelOptions {
                root_directory: Some(root.clone()),
                suggested_name: "日本語.txt".into(),
                ..Default::default()
            },
            |_| Ok(()),
        );
        let bytes = configuration(&save.0).unwrap();
        let request = mochios_workspace_protocol::decode_file_panel_request(&bytes).unwrap();
        assert_eq!(
            request.mode,
            mochios_workspace_protocol::FILE_PANEL_MODE_SAVE
        );
        assert_eq!(request.suggested_name, "日本語.txt");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn preview_selection_validates_root_type_and_replacement() {
        let root = super::super::tests::temporary_directory("preview-selection");
        let file = root.join("notes.txt");
        fs::write(&file, "old").unwrap();
        let panel = SavePanel::new(
            SavePanelOptions {
                root_directory: Some(root.clone()),
                allowed_content_types: vec![ContentType::parse("text/plain").unwrap()],
                ..Default::default()
            },
            |_| Ok(()),
        );
        let state = &panel.0.state;
        assert!(validate_selection(state, &root.join("new.txt")).is_ok());
        assert!(validate_selection(state, &root.join("new.png")).is_err());
        assert!(validate_selection(state, &root).is_err());
        assert!(validate_selection(state, &file).is_err());
        assert_eq!(validate_selection(state, &file).unwrap(), file);
        let outside = root.with_extension("outside.txt");
        fs::write(&outside, "outside").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape.txt")).unwrap();
        assert!(validate_selection(state, &root.join("escape.txt")).is_err());
        std::os::unix::fs::symlink(
            root.with_extension("missing.txt"),
            root.join("dangling.txt"),
        )
        .unwrap();
        assert!(validate_selection(state, &root.join("dangling.txt")).is_err());
        fs::remove_file(outside).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
