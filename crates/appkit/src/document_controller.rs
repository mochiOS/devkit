//! Reusable lifecycle management for file-backed application documents.
//!
//! `DocumentController` owns Open, Save, Save As, edited-state tracking and
//! the unsaved-changes decision shown before replacing or closing a document.
//! Applications remain responsible for decoding and encoding their document
//! format through the callbacks supplied at construction.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use viewkit::command::CommandStatus;
use viewkit::command::standard as commands;
use viewkit::event::{EventContext, EventResult, ViewEvent};
use viewkit::prelude::*;
use viewkit::view::{Constraints, MeasureContext, PaintContext};

use crate::{OpenPanel, OpenPanelOptions, SavePanel, SavePanelOptions, request_exit};

type RevisionProvider = dyn Fn() -> u64;
type DocumentOperation = dyn Fn(&Path) -> Result<DocumentMetadata, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentMetadata {
    pub file_type: String,
    pub encoding: String,
    pub writable: bool,
}

impl DocumentMetadata {
    pub fn new(file_type: impl Into<String>, encoding: impl Into<String>) -> Self {
        Self {
            file_type: file_type.into(),
            encoding: encoding.into(),
            writable: true,
        }
    }

    #[must_use]
    pub const fn writable(mut self, writable: bool) -> Self {
        self.writable = writable;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentInfo {
    pub display_name: String,
    pub path: Option<PathBuf>,
    pub metadata: DocumentMetadata,
}

impl DocumentInfo {
    pub fn untitled(metadata: DocumentMetadata) -> Self {
        Self {
            display_name: String::from("Untitled"),
            path: None,
            metadata,
        }
    }

    pub fn at(path: PathBuf, metadata: DocumentMetadata) -> Self {
        Self {
            display_name: display_name(&path),
            path: Some(path),
            metadata,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PendingAction {
    #[default]
    None,
    Open,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SaveCurrentResult {
    Saved,
    NeedsSaveAs,
}

struct ControllerState {
    info: RefCell<DocumentInfo>,
    saved_revision: Cell<u64>,
    status: RefCell<Option<String>>,
    confirmation_visible: Cell<bool>,
    pending: Cell<PendingAction>,
    close: RefCell<Rc<dyn Fn()>>,
}

struct Operations {
    revision: Rc<RevisionProvider>,
    open: Rc<DocumentOperation>,
    save: Rc<DocumentOperation>,
}

/// Coordinates the standard lifecycle of one file-backed document window.
#[derive(Clone)]
pub struct DocumentController {
    state: Rc<ControllerState>,
    operations: Rc<Operations>,
    open_panel: OpenPanel,
    save_panel: SavePanel,
    cancel: Rc<Button>,
    discard: Rc<Button>,
    save: Rc<Button>,
}

impl DocumentController {
    pub fn new(
        info: DocumentInfo,
        root_directory: Option<PathBuf>,
        revision: impl Fn() -> u64 + 'static,
        open: impl Fn(&Path) -> Result<DocumentMetadata, String> + 'static,
        save: impl Fn(&Path) -> Result<DocumentMetadata, String> + 'static,
    ) -> Self {
        let revision = Rc::new(revision);
        let state = Rc::new(ControllerState {
            info: RefCell::new(info),
            saved_revision: Cell::new(revision()),
            status: RefCell::new(None),
            confirmation_visible: Cell::new(false),
            pending: Cell::new(PendingAction::None),
            close: RefCell::new(Rc::new(request_exit)),
        });
        let operations = Rc::new(Operations {
            revision,
            open: Rc::new(open),
            save: Rc::new(save),
        });

        let open_state = Rc::clone(&state);
        let open_operations = Rc::clone(&operations);
        let open_panel = OpenPanel::new(
            OpenPanelOptions {
                root_directory: root_directory.clone(),
                ..OpenPanelOptions::default()
            },
            move |path| {
                let metadata = (open_operations.open)(path)?;
                *open_state.info.borrow_mut() = DocumentInfo::at(path.to_path_buf(), metadata);
                open_state.saved_revision.set((open_operations.revision)());
                open_state.status.borrow_mut().take();
                Ok(())
            },
        );

        let save_state = Rc::clone(&state);
        let save_operations = Rc::clone(&operations);
        let after_save_open_panel = open_panel.clone();
        let save_cancel_state = Rc::clone(&state);
        let save_panel = SavePanel::new_with_cancel(
            SavePanelOptions {
                root_directory,
                ..SavePanelOptions::default()
            },
            move |path| {
                save_to_path(&save_state, &save_operations, path)?;
                complete_pending(&save_state, &after_save_open_panel);
                Ok(())
            },
            move || {
                save_cancel_state.pending.set(PendingAction::None);
                save_cancel_state.confirmation_visible.set(false);
            },
        );

        let cancel_state = Rc::clone(&state);
        let cancel = Rc::new(
            Button::new("Cancel")
                .size(ButtonSize::Small)
                .on_click(move || {
                    cancel_state.confirmation_visible.set(false);
                    cancel_state.pending.set(PendingAction::None);
                }),
        );

        let discard_state = Rc::clone(&state);
        let discard_open_panel = open_panel.clone();
        let discard = Rc::new(
            Button::new("Don't Save")
                .size(ButtonSize::Small)
                .on_click(move || complete_pending(&discard_state, &discard_open_panel)),
        );

        let confirm_state = Rc::clone(&state);
        let confirm_operations = Rc::clone(&operations);
        let confirm_save_panel = save_panel.clone();
        let confirm_open_panel = open_panel.clone();
        let save = Rc::new(
            Button::new("Save")
                .size(ButtonSize::Small)
                .style(ButtonStyle::Primary)
                .on_click(move || {
                    match save_current_document(&confirm_state, &confirm_operations) {
                        Ok(SaveCurrentResult::Saved) => {
                            complete_pending(&confirm_state, &confirm_open_panel);
                        }
                        Ok(SaveCurrentResult::NeedsSaveAs) => {
                            confirm_state.confirmation_visible.set(false);
                            present_save_panel(&confirm_state, &confirm_save_panel);
                        }
                        Err(error) => *confirm_state.status.borrow_mut() = Some(error),
                    }
                }),
        );

        Self {
            state,
            operations,
            open_panel,
            save_panel,
            cancel,
            discard,
            save,
        }
    }

    pub fn info(&self) -> DocumentInfo {
        self.state.info.borrow().clone()
    }

    /// Sets the action used after this document has been cleared to close.
    /// Multi-window applications should close only the owning window here.
    #[must_use]
    pub fn on_close(self, action: impl Fn() + 'static) -> Self {
        *self.state.close.borrow_mut() = Rc::new(action);
        self
    }

    pub fn status(&self) -> Option<String> {
        self.state.status.borrow().clone()
    }

    pub fn clear_status(&self) {
        self.state.status.borrow_mut().take();
    }

    pub fn is_edited(&self) -> bool {
        (self.operations.revision)() != self.state.saved_revision.get()
    }

    pub fn open(&self) {
        if self.is_edited() {
            self.confirm(PendingAction::Open);
        } else {
            present_open_panel(&self.state, &self.open_panel);
        }
    }

    pub fn save(&self) -> bool {
        match save_current_document(&self.state, &self.operations) {
            Ok(SaveCurrentResult::Saved) => true,
            Ok(SaveCurrentResult::NeedsSaveAs) => {
                present_save_panel(&self.state, &self.save_panel);
                false
            }
            Err(error) => {
                *self.state.status.borrow_mut() = Some(error);
                false
            }
        }
    }

    pub fn save_as(&self) {
        present_save_panel(&self.state, &self.save_panel);
    }

    /// Saves an already named, writable document without presenting UI.
    /// Returns `false` for clean, untitled, read-only, or failed saves.
    pub fn autosave(&self) -> bool {
        if !self.is_edited() {
            return false;
        }
        matches!(
            save_current_document(&self.state, &self.operations),
            Ok(SaveCurrentResult::Saved)
        )
    }

    /// Reloads the current on-disk representation and discards local edits.
    /// Untitled documents cannot be reverted.
    pub fn revert(&self) -> bool {
        let Some(path) = self.state.info.borrow().path.clone() else {
            return false;
        };
        match (self.operations.open)(&path) {
            Ok(metadata) => {
                *self.state.info.borrow_mut() = DocumentInfo::at(path, metadata);
                self.state.saved_revision.set((self.operations.revision)());
                *self.state.status.borrow_mut() = Some(String::from("Reverted"));
                true
            }
            Err(error) => {
                *self.state.status.borrow_mut() = Some(format!("Unable to revert: {error}"));
                false
            }
        }
    }

    /// Returns `true` when the window may close immediately. A `false` result
    /// means the controller presented its unsaved-changes confirmation.
    pub fn request_close(&self) -> bool {
        if !self.is_edited() {
            true
        } else {
            self.confirm(PendingAction::Close);
            false
        }
    }

    /// Requests close and installs the action used if asynchronous unsaved
    /// changes handling later approves it.
    pub fn request_close_with(&self, action: impl Fn() + 'static) -> bool {
        *self.state.close.borrow_mut() = Rc::new(action);
        self.request_close()
    }

    pub fn is_presenting(&self) -> bool {
        self.state.confirmation_visible.get()
            || self.save_panel.is_visible()
            || self.open_panel.is_visible()
    }

    fn confirm(&self, action: PendingAction) {
        self.state.pending.set(action);
        self.state.confirmation_visible.set(true);
    }

    fn dialog_geometry(&self, bounds: Rect, theme: &Theme) -> (Rect, Rect, Rect, Rect) {
        let width = (bounds.size.width - 64.0).clamp(400.0, 520.0);
        let height = 176.0_f32.min((bounds.size.height - 32.0).max(0.0));
        let dialog = Rect::new(
            bounds.origin.x + (bounds.size.width - width) / 2.0,
            bounds.origin.y + (bounds.size.height - height) / 2.0,
            width,
            height,
        );
        let button_y = dialog.origin.y + height - theme.spacing.large - 32.0;
        let save = Rect::new(dialog.origin.x + width - 100.0, button_y, 80.0, 32.0);
        let discard = Rect::new(save.origin.x - 108.0, button_y, 96.0, 32.0);
        let cancel = Rect::new(discard.origin.x - 88.0, button_y, 76.0, 32.0);
        (dialog, cancel, discard, save)
    }
}

impl View for DocumentController {
    fn measure(&self, constraints: Constraints, _context: &mut MeasureContext<'_>) -> Size {
        constraints.constrain(constraints.maximum)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        let info = self.state.info.borrow();
        context.record_command_status(CommandStatus::new(commands::OPEN, bounds, true));
        context.record_command_status(CommandStatus::new(commands::SAVE, bounds, self.is_edited()));
        context.record_command_status(CommandStatus::new(commands::SAVE_AS, bounds, true));
        context.record_command_status(CommandStatus::new(
            commands::REVERT,
            bounds,
            info.path.is_some(),
        ));
        context.record_command_status(CommandStatus::new(commands::CLOSE, bounds, true));
        drop(info);
        if self.save_panel.is_visible() {
            self.save_panel.paint(bounds, context);
            return;
        }
        if self.open_panel.is_visible() {
            self.open_panel.paint(bounds, context);
            return;
        }
        if !self.state.confirmation_visible.get() {
            return;
        }

        let (dialog, cancel, discard, save) = self.dialog_geometry(bounds, context.theme);
        Rectangle::new()
            .color(RectangleColor::Custom(context.theme.shell.scrim))
            .paint(bounds, context);
        Rectangle::new()
            .color(RectangleColor::Custom(context.theme.dialog.background))
            .radius(context.theme.dialog.radius)
            .border(BorderStyle::custom(
                context.theme.dialog.border,
                context.theme.dialog.stroke_width,
            ))
            .paint(dialog, context);
        Text::styled("Save changes?", TextRole::TitleSmall).paint(
            Rect::new(
                dialog.origin.x + 20.0,
                dialog.origin.y + 20.0,
                dialog.size.width - 40.0,
                28.0,
            ),
            context,
        );
        let name = self.state.info.borrow().display_name.clone();
        Text::body(format!(
            "Your changes to {name} will be lost if you don't save them."
        ))
        .paint(
            Rect::new(
                dialog.origin.x + 20.0,
                dialog.origin.y + 58.0,
                dialog.size.width - 40.0,
                44.0,
            ),
            context,
        );
        self.cancel.paint(cancel, context);
        self.discard.paint(discard, context);
        self.save.paint(save, context);
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        if !self.is_presenting()
            && let ViewEvent::Command { command, .. } = event
        {
            let handled = if *command == commands::OPEN {
                self.open();
                true
            } else if *command == commands::SAVE {
                let _ = self.save();
                true
            } else if *command == commands::SAVE_AS {
                self.save_as();
                true
            } else if *command == commands::REVERT {
                let _ = self.revert();
                true
            } else if *command == commands::CLOSE {
                if self.request_close() {
                    let close = Rc::clone(&self.state.close.borrow());
                    close();
                }
                true
            } else {
                false
            };
            if handled {
                context.request_redraw();
                return EventResult::Consumed;
            }
        }
        if self.save_panel.is_visible() {
            let was_visible = true;
            let result = self.save_panel.handle_event(bounds, event, context);
            if was_visible
                && !self.save_panel.is_visible()
                && self.state.pending.get() != PendingAction::None
            {
                self.state.pending.set(PendingAction::None);
            }
            return result;
        }
        if self.open_panel.is_visible() {
            return self.open_panel.handle_event(bounds, event, context);
        }
        if !self.state.confirmation_visible.get() {
            return EventResult::Ignored;
        }
        if matches!(
            event,
            ViewEvent::KeyPressed {
                key: Key::Escape,
                ..
            }
        ) {
            self.state.confirmation_visible.set(false);
            self.state.pending.set(PendingAction::None);
            context.request_redraw();
            return EventResult::Consumed;
        }
        let (_, cancel, discard, save) = self.dialog_geometry(bounds, context.theme());
        let result = self
            .cancel
            .handle_event(cancel, event, context)
            .merge(self.discard.handle_event(discard, event, context))
            .merge(self.save.handle_event(save, event, context));
        if result.is_consumed() {
            context.request_redraw();
        }
        EventResult::Consumed
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Untitled")
        .to_owned()
}

fn current_directory(state: &ControllerState) -> Option<PathBuf> {
    state
        .info
        .borrow()
        .path
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

fn present_open_panel(state: &ControllerState, panel: &OpenPanel) {
    let directory = current_directory(state);
    panel.show_at(directory.as_deref());
}

fn present_save_panel(state: &ControllerState, panel: &SavePanel) {
    let info = state.info.borrow();
    let suggested_name = if info.display_name == "Untitled" {
        "Untitled.txt"
    } else {
        info.display_name.as_str()
    };
    let directory = info
        .path
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    panel.show_for(directory.as_deref(), suggested_name);
}

fn save_to_path(
    state: &ControllerState,
    operations: &Operations,
    path: &Path,
) -> Result<(), String> {
    let metadata = (operations.save)(path).map_err(|error| format!("Unable to save: {error}"))?;
    *state.info.borrow_mut() = DocumentInfo::at(path.to_path_buf(), metadata);
    state.saved_revision.set((operations.revision)());
    *state.status.borrow_mut() = Some(String::from("Saved"));
    Ok(())
}

fn save_current_document(
    state: &ControllerState,
    operations: &Operations,
) -> Result<SaveCurrentResult, String> {
    let info = state.info.borrow().clone();
    let Some(path) = info.path else {
        return Ok(SaveCurrentResult::NeedsSaveAs);
    };
    if !info.metadata.writable {
        return Ok(SaveCurrentResult::NeedsSaveAs);
    }
    save_to_path(state, operations, &path)?;
    Ok(SaveCurrentResult::Saved)
}

fn complete_pending(state: &ControllerState, open_panel: &OpenPanel) {
    state.confirmation_visible.set(false);
    match state.pending.replace(PendingAction::None) {
        PendingAction::None => {}
        PendingAction::Open => present_open_panel(state, open_panel),
        PendingAction::Close => {
            let close = Rc::clone(&state.close.borrow());
            close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_directory() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mochios-document-controller-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn controller(revision: Rc<Cell<u64>>, root: &Path) -> DocumentController {
        let current_revision = Rc::clone(&revision);
        DocumentController::new(
            DocumentInfo::untitled(DocumentMetadata::new("Plain Text", "UTF-8")),
            Some(root.to_path_buf()),
            move || current_revision.get(),
            |_| Ok(DocumentMetadata::new("Plain Text", "UTF-8")),
            |path| {
                fs::write(path, "saved").map_err(|error| error.to_string())?;
                Ok(DocumentMetadata::new("Plain Text", "UTF-8"))
            },
        )
    }

    #[test]
    fn edited_document_defers_close_and_presents_confirmation() {
        let root = temporary_directory();
        let revision = Rc::new(Cell::new(0));
        let controller = controller(Rc::clone(&revision), &root);
        assert!(controller.request_close());

        revision.set(1);
        assert!(!controller.request_close());
        assert!(controller.is_presenting());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn untitled_document_save_uses_save_panel() {
        let root = temporary_directory();
        let revision = Rc::new(Cell::new(1));
        let controller = controller(revision, &root);
        assert!(!controller.save());
        assert!(controller.save_panel.is_visible());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn non_writable_document_save_uses_save_as_without_writing_in_place() {
        let root = temporary_directory();
        let path = root.join("unsupported.bin");
        fs::write(&path, b"original").unwrap();
        let writes = Rc::new(Cell::new(0));
        let save_writes = Rc::clone(&writes);
        let controller = DocumentController::new(
            DocumentInfo::at(
                path.clone(),
                DocumentMetadata::new("Binary", "Unsupported").writable(false),
            ),
            Some(root.clone()),
            || 1,
            |_| Ok(DocumentMetadata::new("Binary", "Unsupported").writable(false)),
            move |_| {
                save_writes.set(save_writes.get() + 1);
                Ok(DocumentMetadata::new("Binary", "UTF-8"))
            },
        );

        assert!(!controller.save());
        assert!(controller.save_panel.is_visible());
        assert_eq!(writes.get(), 0);
        assert_eq!(fs::read(path).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn close_command_closes_only_through_the_configured_window_action() {
        let root = temporary_directory();
        let closed = Rc::new(Cell::new(false));
        let close_flag = Rc::clone(&closed);
        let controller = controller(Rc::new(Cell::new(0)), &root).on_close(move || {
            close_flag.set(true);
        });
        let theme = Theme::LIGHT;
        let mut text_measurer = viewkit::typography::TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut text_measurer);

        assert_eq!(
            controller.handle_event(
                Rect::new(0.0, 0.0, 800.0, 600.0),
                &ViewEvent::Command {
                    command: commands::CLOSE,
                    target: None,
                },
                &mut context,
            ),
            EventResult::Consumed
        );
        assert!(closed.get());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn revert_reloads_the_current_path_and_resets_edited_state() {
        let root = temporary_directory();
        let path = root.join("document.txt");
        fs::write(&path, "disk").unwrap();
        let revision = Rc::new(Cell::new(2));
        let open_revision = Rc::clone(&revision);
        let current_revision = Rc::clone(&revision);
        let controller = DocumentController::new(
            DocumentInfo::at(path.clone(), DocumentMetadata::new("Plain Text", "UTF-8")),
            Some(root.clone()),
            move || current_revision.get(),
            move |opened| {
                assert_eq!(opened, path);
                open_revision.set(3);
                Ok(DocumentMetadata::new("Plain Text", "UTF-8"))
            },
            |_| Ok(DocumentMetadata::new("Plain Text", "UTF-8")),
        );
        revision.set(4);
        assert!(controller.is_edited());

        assert!(controller.revert());
        assert!(!controller.is_edited());
        assert_eq!(controller.status().as_deref(), Some("Reverted"));
        fs::remove_dir_all(root).unwrap();
    }
}
