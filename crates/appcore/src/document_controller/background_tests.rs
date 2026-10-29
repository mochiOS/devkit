use super::*;
use std::{
    fs,
    sync::{Mutex, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Gate {
    started: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}
impl Gate {
    fn wait(&self) -> Result<(), String> {
        self.started.send(()).map_err(|e| e.to_string())?;
        self.release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())
    }
}
#[derive(Default)]
struct Format {
    read: Option<Gate>,
    write: Option<Gate>,
    fail_write: bool,
}
impl DocumentFormat for Format {
    type Content = String;
    fn read(&self, file: &DocumentFile) -> Result<(String, DocumentMetadata), String> {
        if let Some(gate) = &self.read {
            gate.wait()?;
        }
        let text = String::from_utf8(file.read_limited(1024).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        Ok((text, DocumentMetadata::new("Text", "UTF-8")))
    }
    fn write(&self, file: &DocumentFile, content: String) -> Result<DocumentMetadata, String> {
        if let Some(gate) = &self.write {
            gate.wait()?;
        }
        if self.fail_write {
            return Err("Test disk failure".into());
        }
        file.replace_contents(content.as_bytes())
            .map_err(|e| e.to_string())?;
        Ok(DocumentMetadata::new("Text", "UTF-8"))
    }
}
struct Fixture {
    root: PathBuf,
    revision: Rc<Cell<u64>>,
    text: Rc<RefCell<String>>,
    controller: DocumentController,
}
impl Fixture {
    fn new(format: Format, named: bool) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("appcore-document-{}-{nonce}", std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("note.txt"), "original").unwrap();
        let revision = Rc::new(Cell::new(0));
        let text = Rc::new(RefCell::new(String::from("initial")));
        let r = revision.clone();
        let t = text.clone();
        let applied = text.clone();
        let a = revision.clone();
        let info = if named {
            DocumentInfo::at(
                root.join("note.txt"),
                DocumentMetadata::new("Text", "UTF-8"),
            )
        } else {
            DocumentInfo::untitled(DocumentMetadata::new("Text", "UTF-8"))
        };
        let controller = DocumentController::new_background(
            info,
            Some(root.clone()),
            move || r.get(),
            DocumentIo::new(
                format,
                move || Ok(t.borrow().clone()),
                move |value| {
                    *applied.borrow_mut() = value;
                    a.set(a.get() + 1);
                },
            ),
        )
        .unwrap();
        Self {
            root,
            revision,
            text,
            controller,
        }
    }
    fn edit(&self, text: &str) {
        *self.text.borrow_mut() = text.into();
        self.revision.set(self.revision.get() + 1);
    }
    fn pump(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.controller.is_busy() {
            let scope = self
                .controller
                .state
                .tasks
                .borrow()
                .as_ref()
                .unwrap()
                .clone();
            scope.dispatch_for_test();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn gate() -> (Gate, mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (started, ready) = mpsc::channel();
    let (release, waiting) = mpsc::channel();
    (
        Gate {
            started,
            release: Mutex::new(waiting),
        },
        ready,
        release,
    )
}
#[test]
fn reading_is_deferred_and_only_success_changes_document_identity() {
    let f = Fixture::new(Format::default(), false);
    f.controller.open_path(f.root.join("note.txt"));
    assert!(f.controller.is_busy());
    assert_eq!(*f.text.borrow(), "initial");
    assert!(f.controller.info().path.is_none());
    f.pump();
    assert_eq!(*f.text.borrow(), "original");
    assert!(!f.controller.is_edited());
    f.controller.open_path(f.root.join("missing.txt"));
    f.pump();
    assert_eq!(*f.text.borrow(), "original");
    assert_eq!(f.controller.info().path, Some(f.root.join("note.txt")));
    assert!(f.controller.status().is_some());
}
#[test]
fn edits_made_during_read_are_never_overwritten() {
    let (gate, ready, release) = gate();
    let f = Fixture::new(
        Format {
            read: Some(gate),
            ..Format::default()
        },
        false,
    );
    f.controller.open_path(f.root.join("note.txt"));
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    f.edit("new typing");
    release.send(()).unwrap();
    f.pump();
    assert_eq!(*f.text.borrow(), "new typing");
    assert!(f.controller.is_edited());
    assert!(f.controller.info().path.is_none());
}
#[test]
fn save_snapshot_does_not_mark_later_edits_clean_or_close_the_window() {
    let (gate, ready, release) = gate();
    let f = Fixture::new(
        Format {
            write: Some(gate),
            ..Format::default()
        },
        true,
    );
    f.edit("first snapshot");
    let closed = Rc::new(Cell::new(false));
    let c = closed.clone();
    *f.controller.state.close.borrow_mut() = Rc::new(move || c.set(true));
    f.controller.state.pending.set(PendingAction::Close);
    assert_eq!(f.controller.request_save(), SaveRequest::Started);
    assert_eq!(f.controller.request_save(), SaveRequest::Busy);
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(!f.controller.request_close());
    f.edit("second snapshot");
    release.send(()).unwrap();
    f.pump();
    assert_eq!(
        fs::read_to_string(f.root.join("note.txt")).unwrap(),
        "first snapshot"
    );
    assert!(f.controller.is_edited());
    assert!(!closed.get());
    assert!(f.controller.status().unwrap().contains("unsaved"));
}
#[test]
fn failed_save_preserves_dirty_state_and_never_runs_close_continuation() {
    let f = Fixture::new(
        Format {
            fail_write: true,
            ..Format::default()
        },
        true,
    );
    f.edit("unsaved");
    let closed = Rc::new(Cell::new(false));
    let c = closed.clone();
    *f.controller.state.close.borrow_mut() = Rc::new(move || c.set(true));
    f.controller.state.pending.set(PendingAction::Close);
    assert!(!f.controller.save());
    f.pump();
    assert!(!closed.get());
    assert!(f.controller.is_edited());
    assert_eq!(
        fs::read_to_string(f.root.join("note.txt")).unwrap(),
        "original"
    );
}
#[test]
fn save_as_only_closes_after_successful_background_completion() {
    let (gate, ready, release) = gate();
    let f = Fixture::new(
        Format {
            write: Some(gate),
            ..Format::default()
        },
        false,
    );
    f.edit("saved text");
    let closed = Rc::new(Cell::new(false));
    let c = closed.clone();
    *f.controller.state.close.borrow_mut() = Rc::new(move || c.set(true));
    f.controller.state.pending.set(PendingAction::Close);
    f.controller.save_panel.show_for(Some(&f.root), "new.txt");
    let theme = Theme::LIGHT;
    let mut text = viewkit::typography::TextMeasurer::new();
    let mut context = EventContext::new(&theme, &theme.typography, &mut text);
    f.controller.handle_event(
        Rect::new(0., 0., 800., 600.),
        &ViewEvent::KeyPressed {
            key: Key::Enter,
            modifiers: Default::default(),
        },
        &mut context,
    );
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(!closed.get());
    assert!(f.controller.info().path.is_none());
    release.send(()).unwrap();
    f.pump();
    assert!(closed.get());
    assert!(!f.controller.is_edited());
    assert_eq!(f.controller.info().path, Some(f.root.join("new.txt")));
}
#[test]
fn owned_file_does_not_follow_path_replacement_and_limits_reads() {
    let f = Fixture::new(Format::default(), false);
    let path = f.root.join("note.txt");
    let file = DocumentFile::open(&path).unwrap();
    fs::rename(&path, f.root.join("original.txt")).unwrap();
    fs::write(&path, "replacement").unwrap();
    assert!(file.read_limited(2).is_err());
    assert_eq!(file.read_limited(1024).unwrap(), b"original");
    file.replace_contents(b"short").unwrap();
    assert_eq!(fs::read(f.root.join("original.txt")).unwrap(), b"short");
    assert_eq!(fs::read(path).unwrap(), b"replacement");
}

#[test]
fn discard_approval_survives_one_quit_guard_and_does_not_approve_later_edits() {
    let f = Fixture::new(Format::default(), false);
    f.edit("unsaved");
    let controller = Rc::new(f.controller.clone());
    let weak = Rc::downgrade(&controller);
    let accepted = Rc::new(Cell::new(false));
    let seen = accepted.clone();
    *controller.state.close.borrow_mut() = Rc::new(move || {
        seen.set(weak.upgrade().unwrap().request_close());
    });
    controller.state.pending.set(PendingAction::Close);
    complete_pending(&controller.state, &controller.open_panel);
    assert!(accepted.get());
    assert!(controller.state.close_approved.get().is_none());
    f.edit("later edits");
    assert!(!controller.request_close());
}

#[test]
fn dropping_controller_releases_ui_callbacks_without_waiting_for_read() {
    let (gate, ready, release) = gate();
    let f = Fixture::new(
        Format {
            read: Some(gate),
            ..Format::default()
        },
        false,
    );
    let text = f.text.clone();
    let scope = Rc::downgrade(f.controller.state.tasks.borrow().as_ref().unwrap());
    f.controller.open_path(f.root.join("note.txt"));
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(f);
    assert!(scope.upgrade().is_none());
    assert_eq!(Rc::strong_count(&text), 1);
    release.send(()).unwrap();
    assert_eq!(*text.borrow(), "initial");
}
