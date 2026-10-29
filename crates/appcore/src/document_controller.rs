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

use crate::{Alert, OpenPanel, OpenPanelOptions, SavePanel, SavePanelOptions, request_exit};

type RevisionProvider = dyn Fn() -> u64;
mod background;
mod io;
pub use io::{DocumentFile, DocumentFormat, DocumentIo};

enum OperationOutcome {
    Complete(DocumentMetadata),
    Pending,
}
type DocumentOperation = dyn Fn(&Path) -> Result<OperationOutcome, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
/// 文書形式・エンコーディング・保存可否の表示情報です。権限自体は付与しません。
pub struct DocumentMetadata {
    pub file_type: String,
    pub encoding: String,
    pub writable: bool,
}

impl DocumentMetadata {
    /// 形式名とエンコーディング名を指定します。保存可否の既定値はtrueです。
    pub fn new(file_type: impl Into<String>, encoding: impl Into<String>) -> Self {
        Self {
            file_type: file_type.into(),
            encoding: encoding.into(),
            writable: true,
        }
    }

    #[must_use]
    /// 同じ場所へ保存可能かを設定します。falseなら通常の保存もSave Asを使います。
    pub const fn writable(mut self, writable: bool) -> Self {
        self.writable = writable;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// 現在の文書の表示名・パス・メタデータです。パスはアクセス権限を意味しません。
pub struct DocumentInfo {
    pub display_name: String,
    pub path: Option<PathBuf>,
    pub metadata: DocumentMetadata,
}

impl DocumentInfo {
    /// 名前のない新規文書の表示情報を作成します。
    pub fn untitled(metadata: DocumentMetadata) -> Self {
        Self {
            display_name: String::from("Untitled"),
            path: None,
            metadata,
        }
    }

    /// パスから表示名を作成します。ファイルのオープンや存在確認は行いません。
    pub fn at(path: PathBuf, metadata: DocumentMetadata) -> Self {
        Self {
            display_name: display_name(&path),
            path: Some(path),
            metadata,
        }
    }
}

/// 保存要求を受け付けた時点の結果です。StartedやSelectingDestinationは保存完了ではありません。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveRequest {
    /// 同期保存が完了しました。
    Completed,
    /// ワーカーへ保存を依頼しました。is_busyとstatusで実際の完了を確認します。
    Started,
    /// 保存先選択を要求しました。選択のキャンセルや後続のI/O失敗も起こり得ます。
    SelectingDestination,
    /// 別のI/Oが実行中のため受け付けませんでした。
    Busy,
    /// 保存の受付または同期保存に失敗し、標準エラーを表示しました。
    Failed,
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
    Pending,
}

struct ControllerState {
    info: RefCell<DocumentInfo>,
    saved_revision: Cell<u64>,
    status: RefCell<Option<String>>,
    confirmation_window: Cell<Option<WindowId>>,
    pending: Cell<PendingAction>,
    close: RefCell<Rc<dyn Fn()>>,
    busy: Cell<bool>,
    close_approved: Cell<Option<u64>>,
    tasks: RefCell<Option<Rc<crate::task::TaskScope>>>,
    alert: Alert,
    file: RefCell<Option<DocumentFile>>,
    open_panel: RefCell<Option<crate::panel::WeakOpenPanel>>,
    revision: Rc<RevisionProvider>,
    pending_open: RefCell<Option<Box<dyn FnOnce()>>>,
}

struct Operations {
    revision: Rc<RevisionProvider>,
    open: Rc<DocumentOperation>,
    save: Rc<DocumentOperation>,
    received: RefCell<Option<Rc<dyn Fn(DocumentFile) -> Result<(), String>>>>,
}

/// Coordinates the standard lifecycle of one file-backed document window.
#[derive(Clone)]
pub struct DocumentController {
    state: Rc<ControllerState>,
    operations: Rc<Operations>,
    open_panel: OpenPanel,
    save_panel: SavePanel,
    alert: Alert,
}

impl DocumentController {
    /// UI上で同期コールバックを実行する互換コンストラクターです。
    /// revisionは編集世代、open/saveはパスでの読み書きとメタデータの返却を行います。
    /// エラーは文字列で返し、成功扱いにしないでください。重いI/Oにはnew_backgroundを使います。
    pub fn new(
        info: DocumentInfo,
        root_directory: Option<PathBuf>,
        revision: impl Fn() -> u64 + 'static,
        open: impl Fn(&Path) -> Result<DocumentMetadata, String> + 'static,
        save: impl Fn(&Path) -> Result<DocumentMetadata, String> + 'static,
    ) -> Self {
        Self::with_operations(
            info,
            root_directory,
            revision,
            move |path| open(path).map(OperationOutcome::Complete),
            move |path| save(path).map(OperationOutcome::Complete),
        )
    }

    fn with_operations(
        info: DocumentInfo,
        root_directory: Option<PathBuf>,
        revision: impl Fn() -> u64 + 'static,
        open: impl Fn(&Path) -> Result<OperationOutcome, String> + 'static,
        save: impl Fn(&Path) -> Result<OperationOutcome, String> + 'static,
    ) -> Self {
        let revision = Rc::new(revision);
        let state = Rc::new(ControllerState {
            info: RefCell::new(info),
            saved_revision: Cell::new(revision()),
            status: RefCell::new(None),
            confirmation_window: Cell::new(None),
            pending: Cell::new(PendingAction::None),
            close: RefCell::new(Rc::new(request_exit)),
            busy: Cell::new(false),
            close_approved: Cell::new(None),
            tasks: RefCell::new(None),
            alert: Alert::new(),
            file: RefCell::new(None),
            open_panel: RefCell::new(None),
            revision: revision.clone(),
            pending_open: RefCell::new(None),
        });
        let operations = Rc::new(Operations {
            revision,
            open: Rc::new(open),
            save: Rc::new(save),
            received: RefCell::new(None),
        });
        let alert = state.alert.clone();

        let open_state = Rc::clone(&state);
        let open_operations = Rc::clone(&operations);
        let open_panel = OpenPanel::new(
            OpenPanelOptions {
                root_directory: root_directory.clone(),
                ..OpenPanelOptions::default()
            },
            move |path| {
                if let OperationOutcome::Complete(metadata) = (open_operations.open)(path)? {
                    *open_state.info.borrow_mut() = DocumentInfo::at(path.to_path_buf(), metadata);
                    open_state.saved_revision.set((open_operations.revision)());
                    open_state.status.borrow_mut().take();
                }
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
                if save_to_path(&save_state, &save_operations, path)? {
                    complete_pending(&save_state, &after_save_open_panel);
                }
                Ok(())
            },
            move || {
                save_cancel_state.pending.set(PendingAction::None);
                save_cancel_state.pending_open.borrow_mut().take();
                save_cancel_state.confirmation_window.set(None);
            },
        );

        *state.open_panel.borrow_mut() = Some(open_panel.downgrade());
        Self {
            state,
            operations,
            open_panel,
            save_panel,
            alert,
        }
    }

    /// 最後に正常に開いた、または保存した文書の表示情報を複製します。
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

    /// 現在の処理状況または直前の結果・エラーを複製します。
    pub fn status(&self) -> Option<String> {
        self.state.status.borrow().clone()
    }

    /// 表示用の状況文を消します。処理や未保存状態は変更しません。
    pub fn clear_status(&self) {
        self.state.status.borrow_mut().take();
    }

    /// 現在の編集世代が最後に正常保存したスナップショットと違えばtrueを返します。
    pub fn is_edited(&self) -> bool {
        (self.operations.revision)() != self.state.saved_revision.get()
    }

    /// 未保存確認を通してOpenPanelを表示します。I/O実行中は要求を受け付けません。
    pub fn open(&self) {
        if self.is_busy() {
            return;
        }
        self.state.pending_open.borrow_mut().take();
        if self.is_edited() {
            self.confirm(PendingAction::Open);
        } else {
            present_open_panel(&self.state, &self.open_panel);
        }
    }

    /// 互換用の保存操作です。同期保存完了だけtrueを返します。
    /// 非同期受付やパネル表示はfalseです。新しいコードではrequest_saveを使用してください。
    pub fn save(&self) -> bool {
        self.request_save() == SaveRequest::Completed
    }

    /// 保存を要求し、受付結果を区別して返します。UIスレッド専用です。
    ///
    /// Startedは完了ではありません。保存先未設定・読み取り専用ならSave Asを要求します。
    /// エラーはstatusと標準エラー表示へ反映します。重複要求はBusyです。
    pub fn request_save(&self) -> SaveRequest {
        if self.is_busy() {
            return SaveRequest::Busy;
        }
        match save_current_document(&self.state, &self.operations) {
            Ok(SaveCurrentResult::Saved) => SaveRequest::Completed,
            Ok(SaveCurrentResult::Pending) => SaveRequest::Started,
            Ok(SaveCurrentResult::NeedsSaveAs) => {
                present_save_panel(&self.state, &self.save_panel);
                SaveRequest::SelectingDestination
            }
            Err(error) => {
                *self.state.status.borrow_mut() = Some(error.clone());
                self.alert
                    .present_error("The document could not be saved.", error);
                SaveRequest::Failed
            }
        }
    }

    /// 保存先・名前を選ぶSavePanelを表示します。I/O実行中は要求を受け付けません。
    pub fn save_as(&self) {
        if self.is_busy() {
            return;
        }
        present_save_panel(&self.state, &self.save_panel);
    }

    /// Saves an already named, writable document without presenting UI.
    /// Returns `false` for clean, untitled, read-only, failed or asynchronous saves.
    /// 非同期保存の受付後はis_busyがtrueです。falseを保存失敗とは解釈しないでください。
    pub fn autosave(&self) -> bool {
        if self.is_busy() || !self.is_edited() {
            return false;
        }
        matches!(
            save_current_document(&self.state, &self.operations),
            Ok(SaveCurrentResult::Saved)
        )
    }

    /// Reloads the current on-disk representation and discards local edits.
    /// Untitled documents cannot be reverted.
    /// new_backgroundでは完了前にfalseを返します。読み込み失敗時や処理中の追加編集は保持します。
    pub fn revert(&self) -> bool {
        if self.is_busy() {
            return false;
        }
        let Some(path) = self.state.info.borrow().path.clone() else {
            return false;
        };
        match (self.operations.open)(&path) {
            Ok(OperationOutcome::Complete(metadata)) => {
                *self.state.info.borrow_mut() = DocumentInfo::at(path, metadata);
                self.state.saved_revision.set((self.operations.revision)());
                *self.state.status.borrow_mut() = Some(String::from("Reverted"));
                true
            }
            Ok(OperationOutcome::Pending) => false,
            Err(error) => {
                let error = format!("Unable to revert: {error}");
                *self.state.status.borrow_mut() = Some(error.clone());
                self.alert
                    .present_error("The document could not be reverted.", error);
                false
            }
        }
    }

    /// Returns `true` when the window may close immediately. A `false` result
    /// means I/O is pending or the controller presented its unsaved-changes confirmation.
    pub fn request_close(&self) -> bool {
        if self.is_busy() {
            return false;
        }
        if self.state.close_approved.take() == Some((self.operations.revision)())
            || !self.is_edited()
        {
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

    /// ファイルパネル、未保存確認、または標準エラーを表示中か返します。I/O状態はis_busyで確認します。
    pub fn is_presenting(&self) -> bool {
        self.alert.is_visible()
            || self.state.confirmation_window.get().is_some()
            || self.save_panel.is_visible()
            || self.open_panel.is_visible()
    }

    fn confirm(&self, action: PendingAction) {
        if self.state.confirmation_window.get().is_some() {
            return;
        }
        self.state.pending.set(action);
        let name = self.state.info.borrow().display_name.clone();
        let message = format!("Your changes to {name} will be lost if you don't save them.");

        let save_state = Rc::clone(&self.state);
        let save_operations = Rc::clone(&self.operations);
        let save_panel = self.save_panel.clone();
        let save_open_panel = self.open_panel.clone();
        let save_alert = self.alert.clone();
        let discard_state = Rc::clone(&self.state);
        let discard_open_panel = self.open_panel.clone();
        let cancel_state = Rc::clone(&self.state);
        let window = request_document_confirmation_window(
            "Save changes?",
            message,
            move || {
                save_state.confirmation_window.set(None);
                match save_current_document(&save_state, &save_operations) {
                    Ok(SaveCurrentResult::Saved) => {
                        complete_pending(&save_state, &save_open_panel);
                    }
                    Ok(SaveCurrentResult::Pending) => {}
                    Ok(SaveCurrentResult::NeedsSaveAs) => {
                        present_save_panel(&save_state, &save_panel);
                    }
                    Err(error) => {
                        *save_state.status.borrow_mut() = Some(error.clone());
                        save_alert.present_error("The document could not be saved.", error);
                    }
                }
            },
            move || {
                discard_state.confirmation_window.set(None);
                complete_pending(&discard_state, &discard_open_panel);
            },
            move || {
                cancel_state.confirmation_window.set(None);
                cancel_state.pending.set(PendingAction::None);
                cancel_state.pending_open.borrow_mut().take();
            },
        );
        self.state.confirmation_window.set(Some(window));
    }
}

impl View for DocumentController {
    fn measure(&self, constraints: Constraints, _context: &mut MeasureContext<'_>) -> Size {
        constraints.constrain(constraints.maximum)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        let info = self.state.info.borrow();
        context.record_command_status(CommandStatus::new(commands::OPEN, bounds, !self.is_busy()));
        context.record_command_status(CommandStatus::new(
            commands::SAVE,
            bounds,
            !self.is_busy() && self.is_edited(),
        ));
        context.record_command_status(CommandStatus::new(
            commands::SAVE_AS,
            bounds,
            !self.is_busy(),
        ));
        context.record_command_status(CommandStatus::new(
            commands::REVERT,
            bounds,
            info.path.is_some(),
        ));
        context.record_command_status(CommandStatus::new(commands::CLOSE, bounds, true));
        drop(info);
        if self.alert.is_visible() {
            self.alert.paint(bounds, context);
            return;
        }
        if self.save_panel.is_visible() {
            self.save_panel.paint(bounds, context);
            return;
        }
        if self.open_panel.is_visible() {
            self.open_panel.paint(bounds, context);
        }
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        if self.alert.is_visible() {
            return self.alert.handle_event(bounds, event, context);
        }
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
                && !self.is_busy()
                && self.state.pending.get() != PendingAction::None
            {
                self.state.pending.set(PendingAction::None);
            }
            return result;
        }
        if self.open_panel.is_visible() {
            return self.open_panel.handle_event(bounds, event, context);
        }
        EventResult::Ignored
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
    // The system panel completes synchronously and updates document metadata.
    // Release the borrow before entering the panel's operation callback.
    let info = state.info.borrow().clone();
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
) -> Result<bool, String> {
    let metadata =
        match (operations.save)(path).map_err(|error| format!("Unable to save: {error}"))? {
            OperationOutcome::Pending => return Ok(false),
            OperationOutcome::Complete(metadata) => metadata,
        };
    *state.info.borrow_mut() = DocumentInfo::at(path.to_path_buf(), metadata);
    state.saved_revision.set((operations.revision)());
    *state.status.borrow_mut() = Some(String::from("Saved"));
    Ok(true)
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
    if save_to_path(state, operations, &path)? {
        Ok(SaveCurrentResult::Saved)
    } else {
        Ok(SaveCurrentResult::Pending)
    }
}

fn complete_pending(state: &ControllerState, open_panel: &OpenPanel) {
    state.confirmation_window.set(None);
    match state.pending.replace(PendingAction::None) {
        PendingAction::None => {}
        PendingAction::Open => {
            let action = state.pending_open.borrow_mut().take();
            if let Some(action) = action {
                action();
            } else {
                present_open_panel(state, open_panel);
            }
        }
        PendingAction::Close => {
            state.close_approved.set(Some((state.revision)()));
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
    fn failed_direct_save_presents_an_error_alert() {
        let root = temporary_directory();
        let path = root.join("document.txt");
        let controller = DocumentController::new(
            DocumentInfo::at(path, DocumentMetadata::new("Plain Text", "UTF-8")),
            Some(root.clone()),
            || 1,
            |_| Ok(DocumentMetadata::new("Plain Text", "UTF-8")),
            |_| Err(String::from("The disk is full.")),
        );

        assert!(!controller.save());
        assert!(controller.alert.is_visible());
        assert!(controller.is_presenting());
        assert_eq!(
            controller.status().as_deref(),
            Some("Unable to save: The disk is full.")
        );
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

#[cfg(test)]
mod background_tests;
