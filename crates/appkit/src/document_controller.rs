//! Reusable lifecycle management for file-backed application documents.
//!
//! `DocumentController` owns Open, Save, Save As, edited-state tracking and
//! the unsaved-changes decision shown before replacing or closing a document.
//! Applications remain responsible for decoding and encoding their document
//! format through the callbacks supplied at construction.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

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

struct ControllerState {
    info: RefCell<DocumentInfo>,
    saved_revision: Cell<u64>,
    status: RefCell<Option<String>>,
    confirmation_visible: Cell<bool>,
    pending: Cell<PendingAction>,
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
        let save_panel = SavePanel::new(
            SavePanelOptions {
                root_directory,
                ..SavePanelOptions::default()
            },
            move |path| {
                save_to_path(&save_state, &save_operations, path)?;
                complete_pending(&save_state, &after_save_open_panel);
                Ok(())
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
                    let path = confirm_state.info.borrow().path.clone();
                    let Some(path) = path else {
                        confirm_state.confirmation_visible.set(false);
                        present_save_panel(&confirm_state, &confirm_save_panel);
                        return;
                    };
                    match save_to_path(&confirm_state, &confirm_operations, &path) {
                        Ok(()) => complete_pending(&confirm_state, &confirm_open_panel),
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
        let info = self.state.info.borrow().clone();
        let Some(path) = info.path else {
            present_save_panel(&self.state, &self.save_panel);
            return false;
        };
        if !info.metadata.writable {
            *self.state.status.borrow_mut() = Some(String::from("This document cannot be saved"));
            return false;
        }
        match save_to_path(&self.state, &self.operations, &path) {
            Ok(()) => true,
            Err(error) => {
                *self.state.status.borrow_mut() = Some(error);
                false
            }
        }
    }

    pub fn save_as(&self) {
        present_save_panel(&self.state, &self.save_panel);
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

fn complete_pending(state: &ControllerState, open_panel: &OpenPanel) {
    state.confirmation_visible.set(false);
    match state.pending.replace(PendingAction::None) {
        PendingAction::None => {}
        PendingAction::Open => present_open_panel(state, open_panel),
        PendingAction::Close => request_exit(),
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
}
