//! Persistent restoration state for document windows.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"MOCHSES1";
const MAX_WINDOWS: usize = 256;
const MAX_FIELD_BYTES: usize = 16 * 1024;

/// The last known logical-pixel frame of a document window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowFrame {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl WindowFrame {
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn is_valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.0
            && self.height > 0.0
    }
}

/// One independently restorable document window.
#[derive(Clone, Debug, PartialEq)]
pub struct RestorableWindow {
    pub identifier: String,
    pub document_path: Option<PathBuf>,
    pub recovery_identifier: Option<String>,
    pub frame: Option<WindowFrame>,
    pub maximized: bool,
    pub fullscreen: bool,
}

impl RestorableWindow {
    #[must_use]
    pub fn new(identifier: impl Into<String>) -> Self {
        Self {
            identifier: identifier.into(),
            document_path: None,
            recovery_identifier: None,
            frame: None,
            maximized: false,
            fullscreen: false,
        }
    }
}

/// The document windows which should be reopened on the next launch.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ApplicationSession {
    pub windows: Vec<RestorableWindow>,
}

/// Atomically persists one application's restoration state.
#[derive(Clone, Debug)]
pub struct SessionStore {
    path: PathBuf,
}

impl SessionStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn save(&self, session: &ApplicationSession) -> std::io::Result<()> {
        let bytes = encode(session)?;
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let temporary = temporary_path(&self.path);
        let mut file = File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &self.path)?;
        sync_directory(parent);
        Ok(())
    }

    pub fn load(&self) -> std::io::Result<Option<ApplicationSession>> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let maximum = 12 + MAX_WINDOWS * (32 + MAX_FIELD_BYTES * 3);
        let mut bytes = Vec::new();
        file.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > maximum {
            return Err(invalid_data("session exceeds restoration limits"));
        }
        decode(&bytes).map(Some)
    }

    pub fn clear(&self) -> std::io::Result<bool> {
        match fs::remove_file(&self.path) {
            Ok(()) => {
                if let Some(parent) = self.path.parent() {
                    sync_directory(parent);
                }
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
}

fn encode(session: &ApplicationSession) -> std::io::Result<Vec<u8>> {
    if session.windows.len() > MAX_WINDOWS {
        return Err(invalid_input("too many windows in session"));
    }
    let mut identifiers = HashSet::new();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(session.windows.len() as u32).to_le_bytes());
    for window in &session.windows {
        validate_identifier(&window.identifier)?;
        if !identifiers.insert(window.identifier.as_str()) {
            return Err(invalid_input("duplicate restoration identifier"));
        }
        if let Some(identifier) = window.recovery_identifier.as_deref() {
            validate_identifier(identifier)?;
        }
        if window.frame.is_some_and(|frame| !frame.is_valid()) {
            return Err(invalid_input("invalid window frame"));
        }
        let path = path_string(window.document_path.as_deref())?;
        let recovery = window.recovery_identifier.as_deref().unwrap_or("");
        for field in [window.identifier.as_str(), path, recovery] {
            if field.len() > MAX_FIELD_BYTES {
                return Err(invalid_input("session field is too large"));
            }
        }
        bytes.extend_from_slice(&(window.identifier.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(path.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(recovery.len() as u32).to_le_bytes());
        let frame = window.frame.unwrap_or(WindowFrame::new(0.0, 0.0, 0.0, 0.0));
        for value in [frame.x, frame.y, frame.width, frame.height] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let mut flags = 0u8;
        flags |= u8::from(window.frame.is_some());
        flags |= u8::from(window.maximized) << 1;
        flags |= u8::from(window.fullscreen) << 2;
        bytes.extend_from_slice(&[flags, 0, 0, 0]);
        bytes.extend_from_slice(window.identifier.as_bytes());
        bytes.extend_from_slice(path.as_bytes());
        bytes.extend_from_slice(recovery.as_bytes());
    }
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> std::io::Result<ApplicationSession> {
    if bytes.len() < 12 || bytes.get(..8) != Some(MAGIC) {
        return Err(invalid_data("invalid session record"));
    }
    let count = read_u32(bytes, 8)? as usize;
    if count > MAX_WINDOWS {
        return Err(invalid_data("too many windows in session"));
    }
    let mut offset = 12usize;
    let mut windows = Vec::with_capacity(count);
    let mut identifiers = HashSet::new();
    for _ in 0..count {
        let identifier_len = read_u32_at(bytes, &mut offset)? as usize;
        let path_len = read_u32_at(bytes, &mut offset)? as usize;
        let recovery_len = read_u32_at(bytes, &mut offset)? as usize;
        if [identifier_len, path_len, recovery_len]
            .into_iter()
            .any(|length| length > MAX_FIELD_BYTES)
        {
            return Err(invalid_data("session field exceeds limits"));
        }
        let x = read_f32_at(bytes, &mut offset)?;
        let y = read_f32_at(bytes, &mut offset)?;
        let width = read_f32_at(bytes, &mut offset)?;
        let height = read_f32_at(bytes, &mut offset)?;
        let flags = *bytes
            .get(offset)
            .ok_or_else(|| invalid_data("truncated session record"))?;
        if flags & !0b111 != 0 || bytes.get(offset + 1..offset + 4) != Some(&[0, 0, 0]) {
            return Err(invalid_data("invalid session flags"));
        }
        offset += 4;
        let identifier = read_string_at(bytes, &mut offset, identifier_len)?;
        validate_identifier(&identifier).map_err(|_| invalid_data("invalid window identifier"))?;
        if !identifiers.insert(identifier.clone()) {
            return Err(invalid_data("duplicate restoration identifier"));
        }
        let path = read_string_at(bytes, &mut offset, path_len)?;
        let recovery = read_string_at(bytes, &mut offset, recovery_len)?;
        if !recovery.is_empty() {
            validate_identifier(&recovery)
                .map_err(|_| invalid_data("invalid recovery identifier"))?;
        }
        let frame = WindowFrame::new(x, y, width, height);
        let frame = if flags & 1 != 0 {
            if !frame.is_valid() {
                return Err(invalid_data("invalid window frame"));
            }
            Some(frame)
        } else {
            None
        };
        windows.push(RestorableWindow {
            identifier,
            document_path: (!path.is_empty()).then(|| PathBuf::from(path)),
            recovery_identifier: (!recovery.is_empty()).then_some(recovery),
            frame,
            maximized: flags & 0b010 != 0,
            fullscreen: flags & 0b100 != 0,
        });
    }
    if offset != bytes.len() {
        return Err(invalid_data("trailing session data"));
    }
    Ok(ApplicationSession { windows })
}

fn validate_identifier(identifier: &str) -> std::io::Result<()> {
    if identifier.is_empty()
        || identifier.len() > 128
        || !identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(invalid_input("invalid restoration identifier"));
    }
    Ok(())
}

fn path_string(path: Option<&Path>) -> std::io::Result<&str> {
    path.map_or(Ok(""), |path| {
        path.to_str()
            .ok_or_else(|| invalid_input("document path is not UTF-8"))
    })
}

fn read_u32(bytes: &[u8], offset: usize) -> std::io::Result<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| invalid_data("truncated session record"))?;
    Ok(u32::from_le_bytes(value.try_into().unwrap()))
}

fn read_u32_at(bytes: &[u8], offset: &mut usize) -> std::io::Result<u32> {
    let value = read_u32(bytes, *offset)?;
    *offset += 4;
    Ok(value)
}

fn read_f32_at(bytes: &[u8], offset: &mut usize) -> std::io::Result<f32> {
    let value = bytes
        .get(*offset..*offset + 4)
        .ok_or_else(|| invalid_data("truncated session record"))?;
    *offset += 4;
    Ok(f32::from_le_bytes(value.try_into().unwrap()))
}

fn read_string_at(bytes: &[u8], offset: &mut usize, length: usize) -> std::io::Result<String> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| invalid_data("invalid session length"))?;
    let value = std::str::from_utf8(
        bytes
            .get(*offset..end)
            .ok_or_else(|| invalid_data("truncated session record"))?,
    )
    .map_err(|_| invalid_data("session field is not UTF-8"))?
    .to_owned();
    *offset = end;
    Ok(value)
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "session".into());
    name.push(".new");
    path.with_file_name(name)
}

fn invalid_input(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
}

fn invalid_data(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

fn sync_directory(directory: &Path) {
    if let Ok(file) = File::open(directory) {
        let _ = file.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temporary_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "mochios-session-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn round_trips_document_windows_and_clears() {
        let path = temporary_path();
        let store = SessionStore::new(&path);
        let session = ApplicationSession {
            windows: vec![RestorableWindow {
                identifier: "document-1".into(),
                document_path: Some(PathBuf::from("/home/user/file.txt")),
                recovery_identifier: Some("recovery-1".into()),
                frame: Some(WindowFrame::new(12.0, 24.0, 900.0, 700.0)),
                maximized: false,
                fullscreen: true,
            }],
        };
        store.save(&session).unwrap();
        assert_eq!(store.load().unwrap(), Some(session));
        assert!(store.clear().unwrap());
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn rejects_duplicate_ids_and_invalid_frames() {
        let store = SessionStore::new(temporary_path());
        let mut first = RestorableWindow::new("same");
        first.frame = Some(WindowFrame::new(0.0, 0.0, f32::NAN, 100.0));
        assert_eq!(
            store
                .save(&ApplicationSession {
                    windows: vec![first]
                })
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert!(
            store
                .save(&ApplicationSession {
                    windows: vec![RestorableWindow::new("same"), RestorableWindow::new("same")]
                })
                .is_err()
        );
    }
}
