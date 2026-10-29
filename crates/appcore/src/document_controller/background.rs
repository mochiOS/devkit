use super::*;
use crate::task::{TaskError, TaskScope, TaskStartError};

enum Source {
    Path(PathBuf),
    File(DocumentFile),
}
impl DocumentController {
    /// 文書のI/Oをワーカーで実行するコントローラーを作成します。
    ///
    /// `info`は初期表示、`root_directory`はパネルの参照範囲、`revision`はUI上の編集世代です。
    /// `io`に形式の読み書きとUIのスナップショット・反映処理を渡します。
    /// 保存中の追加編集は未保存のまま残します。処理中の重複操作と終了は保留します。
    /// snapshot/applyにはUIデータを捕捉し、コントローラー自身の強い参照を捕捉しないでください。
    /// 旧newは同期コールバックの互換APIとして維持します。
    ///
    /// ## Errors
    /// ワーカーやUI通知を作成できない場合はTaskStartErrorです。I/Oの失敗は
    /// 完了後にstatusと標準エラー表示へ反映し、成功扱いにしません。
    ///
    /// ## Example
    /// ```rust,no_run
    /// # use appcore::{DocumentController, DocumentInfo, DocumentMetadata};
    /// # use appcore::document_controller::{DocumentIo, DocumentFormat};
    /// fn make<F: DocumentFormat>(io: DocumentIo<F>) -> Result<DocumentController, appcore::task::TaskStartError> {
    ///     DocumentController::new_background(
    ///         DocumentInfo::untitled(DocumentMetadata::new("Document", "Binary")),
    ///         None, || 0, io,
    ///     )
    /// }
    /// ```
    pub fn new_background<F: DocumentFormat>(
        info: DocumentInfo,
        root_directory: Option<PathBuf>,
        revision: impl Fn() -> u64 + 'static,
        io: DocumentIo<F>,
    ) -> Result<Self, TaskStartError> {
        let scope = Rc::new(TaskScope::new()?);
        let revision: Rc<RevisionProvider> = Rc::new(revision);
        let owner = Rc::new(RefCell::new(std::rc::Weak::<ControllerState>::new()));
        let read_owner = owner.clone();
        let read_scope = scope.clone();
        let format = io.format.clone();
        let apply = io.apply;
        let read_revision = revision.clone();
        let start_read: Rc<dyn Fn(Source) -> Result<OperationOutcome, String>> = Rc::new(
            move |source| {
                let state = read_owner
                    .borrow()
                    .upgrade()
                    .ok_or("The document has closed.")?;
                if state.busy.get() {
                    return Err("A document operation is already running.".into());
                }
                let opening_revision = read_revision();
                let current = state.file.borrow().clone();
                let weak = Rc::downgrade(&state);
                let format = format.clone();
                let apply = apply.clone();
                let revision = read_revision.clone();
                read_scope
                    .spawn(
                        move |task| {
                            task.check_cancelled()?;
                            let file = match source {
                                Source::File(file) => file,
                                Source::Path(path) => {
                                    match current.filter(|file| file.path() == path) {
                                        Some(file) => file,
                                        None => DocumentFile::open(path)
                                            .map_err(|e| TaskError::Operation(e.to_string()))?,
                                    }
                                }
                            };
                            let (content, mut metadata) =
                                format.read(&file).map_err(TaskError::Operation)?;
                            metadata.writable &= file.is_writable();
                            task.check_cancelled()?;
                            Ok((file, content, metadata))
                        },
                        move |result| {
                            if let Some(state) = weak.upgrade() {
                                state.busy.set(false);
                                match result {
                                    Ok((file, content, metadata)) => {
                                        if revision() != opening_revision {
                                            fail(&state, "The document could not be opened.", "The document changed while reading. Your edits have been kept. Try opening the file again.".into());
                                            return;
                                        }
                                        apply(content);
                                        *state.info.borrow_mut() =
                                            DocumentInfo::at(file.path().to_path_buf(), metadata);
                                        *state.file.borrow_mut() = Some(file);
                                        state.saved_revision.set(revision());
                                        state.status.borrow_mut().take();
                                    }
                                    Err(error) => fail(
                                        &state,
                                        "The document could not be opened.",
                                        error.to_string(),
                                    ),
                                }
                            }
                        },
                    )
                    .map_err(|e| e.to_string())?;
                state.busy.set(true);
                *state.status.borrow_mut() = Some("Opening…".into());
                Ok(OperationOutcome::Pending)
            },
        );
        let open = start_read.clone();
        let save_owner = owner.clone();
        let save_revision = revision.clone();
        let snapshot = io.snapshot;
        let format = io.format;
        let save_scope = scope.clone();
        let controller = Self::with_operations(
            info,
            root_directory,
            move || revision(),
            move |path| open(Source::Path(path.to_path_buf())),
            move |path| {
                let state = save_owner
                    .borrow()
                    .upgrade()
                    .ok_or("The document has closed.")?;
                if state.busy.get() {
                    return Err("A document operation is already running.".into());
                }
                let saved_revision = save_revision();
                let content = snapshot()?;
                let current = state
                    .file
                    .borrow()
                    .clone()
                    .filter(|file| file.path() == path && file.is_writable());
                let path = path.to_path_buf();
                let weak = Rc::downgrade(&state);
                let format = format.clone();
                let revision = save_revision.clone();
                save_scope
                    .spawn(
                        move |task| {
                            task.check_cancelled()?;
                            let file = match current {
                                Some(file) => file,
                                None => DocumentFile::create(path)
                                    .map_err(|e| TaskError::Operation(e.to_string()))?,
                            };
                            // No cancellation checkpoint after writing: cancellation cannot undo a save.
                            let mut metadata =
                                format.write(&file, content).map_err(TaskError::Operation)?;
                            metadata.writable &= file.is_writable();
                            Ok((file, metadata))
                        },
                        move |result| {
                            if let Some(state) = weak.upgrade() {
                                state.busy.set(false);
                                match result {
                                    Ok((file, metadata)) => {
                                        *state.info.borrow_mut() =
                                            DocumentInfo::at(file.path().to_path_buf(), metadata);
                                        *state.file.borrow_mut() = Some(file);
                                        state.saved_revision.set(saved_revision);
                                        *state.status.borrow_mut() = Some("Saved".into());
                                        if revision() == saved_revision {
                                            let panel = state
                                                .open_panel
                                                .borrow()
                                                .as_ref()
                                                .and_then(|panel| panel.upgrade());
                                            if let Some(panel) = panel {
                                                complete_pending(&state, &panel);
                                            }
                                        } else {
                                            *state.status.borrow_mut() = Some("Saved earlier changes; new changes remain unsaved.".into());
                                            state.pending.set(PendingAction::None);
                                            state.pending_open.borrow_mut().take();
                                        }
                                    }
                                    Err(error) => fail(
                                        &state,
                                        "The document could not be saved.",
                                        error.to_string(),
                                    ),
                                }
                            }
                        },
                    )
                    .map_err(|e| e.to_string())?;
                state.busy.set(true);
                *state.status.borrow_mut() = Some("Saving…".into());
                Ok(OperationOutcome::Pending)
            },
        );
        *owner.borrow_mut() = Rc::downgrade(&controller.state);
        *controller.state.tasks.borrow_mut() = Some(scope);
        *controller.operations.received.borrow_mut() = Some(Rc::new(move |file| {
            start_read(Source::File(file)).map(|_| ())
        }));
        Ok(controller)
    }

    /// I/Oが実行中、またはUIへの完了配送待ちの場合にtrueを返します。UIスレッド専用です。
    pub fn is_busy(&self) -> bool {
        self.state.busy.get()
    }

    /// 権限付きの受信資源を開きます。パスを再オープンしません。UIスレッド専用です。
    ///
    /// 未保存なら通常の保存確認を通します。Okは受付済みを表し、読み込み完了ではありません。
    /// ## Errors
    /// new_background以外のコントローラー、I/O実行中はErrです。
    /// 読み込み失敗は後で標準エラー表示とstatusに反映します。
    pub fn open_file(&self, file: DocumentFile) -> Result<(), String> {
        if self.is_busy() {
            return Err("A document operation is already running.".into());
        }
        let read = self
            .operations
            .received
            .borrow()
            .clone()
            .ok_or("This controller does not accept owned document files.")?;
        let weak = Rc::downgrade(&self.state);
        let operation = Box::new(move || {
            if let Err(error) = read(file) {
                if let Some(state) = weak.upgrade() {
                    fail(&state, "The document could not be opened.", error);
                }
            }
        });
        self.queue_open(operation);
        Ok(())
    }

    /// 現在のアプリの権限でパスを開くよう要求します。未保存確認を省略しません。
    ///
    /// new_backgroundではオープン・読み取りをワーカーで実行します。戻った時点で
    /// 完了したとは限りません。失敗はstatusと標準エラー表示に反映します。
    pub fn open_path(&self, path: impl Into<PathBuf>) {
        if self.is_busy() {
            return;
        }
        let path = path.into();
        let weak = Rc::downgrade(&self.state);
        let operations = self.operations.clone();
        self.queue_open(Box::new(move || {
            if let Some(state) = weak.upgrade() {
                match (operations.open)(&path) {
                    Ok(OperationOutcome::Complete(metadata)) => {
                        *state.info.borrow_mut() = DocumentInfo::at(path, metadata);
                        state.saved_revision.set((operations.revision)());
                    }
                    Ok(OperationOutcome::Pending) => {}
                    Err(error) => fail(&state, "The document could not be opened.", error),
                }
            }
        }));
    }
    fn queue_open(&self, operation: Box<dyn FnOnce()>) {
        if self.is_edited() {
            *self.state.pending_open.borrow_mut() = Some(operation);
            self.confirm(PendingAction::Open);
        } else {
            operation();
        }
    }
}
fn fail(state: &ControllerState, title: &str, error: String) {
    state.busy.set(false);
    state.pending.set(PendingAction::None);
    state.pending_open.borrow_mut().take();
    *state.status.borrow_mut() = Some(error.clone());
    state.alert.present_error(title, error);
}
