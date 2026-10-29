#![deny(missing_docs)]
//! UIを止めずに同期処理を実行し、結果を作成元のUIスレッドへ配送します。
//!
//! ワーカーにはSendな入力だけを移動し、UI状態は完了コールバックで更新します。
//! キャンセルは協調的な要求です。要求だけで処理終了やロールバックを宣言しません。
//! ScopeのDropは通知を解除して中止を要求しますが、実行中のI/Oを強制停止しません。
//! 保存など完了までアプリを閉じられない処理では、close_requestedで終了を保留してください。
//!
//! ```rust,no_run
//! use appcore::task::{TaskScope, TaskError, TaskResult};
//! let tasks = TaskScope::new()?;
//! let handle = tasks.spawn(
//!     |task| -> TaskResult<String, std::io::Error> {
//!         task.check_cancelled()?;
//!         std::fs::read_to_string("/home/user/note.txt").map_err(TaskError::Operation)
//!     },
//!     |result| { /* このコールバックでViewKitのStateを更新する */ },
//! )?;
//! # Ok::<(), appcore::task::TaskStartError>(())
//! ```
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
};
use viewkit::dispatch::{UiNotifier, UiSubscription};
mod executor;
pub use executor::TaskExecutor;
const QUEUED: u8 = 0;
const RUNNING: u8 = 1;
const CANCEL_QUEUED: u8 = 2;
const CANCEL_RUNNING: u8 = 3;
const SUCCEEDED: u8 = 4;
const FAILED: u8 = 5;
const CANCELLED: u8 = 6;
const STOPPED: u8 = 7;
const MAX_SCOPE_TASKS: usize = 64;

/// タスクを受け付けられなかった理由です。処理本体は実行されていません。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskStartError {
    /// 実行数または受付数の指定が範囲外です。
    InvalidLimits,
    /// ワーカーを作成できない、または実行器が終了しています。
    WorkerUnavailable,
    /// 実行器の待ち行列が満杯です。既存タスクは変更しません。
    QueueFull,
    /// このScopeが保持する未配送タスクが上限に達しています。
    ScopeFull,
    /// UI通知の登録上限に達しています。
    UiCapacity,
    /// ScopeまたはSlotの識別子をこれ以上発行できません。
    IdentityExhausted,
}
impl std::fmt::Display for TaskStartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "task could not start: {self:?}")
    }
}
impl std::error::Error for TaskStartError {}

/// 実行後の失敗理由です。キャンセル要求だけでCancelledにはなりません。
#[derive(Debug, PartialEq, Eq)]
pub enum TaskError<E> {
    /// 処理本体が返したエラーです。
    Operation(E),
    /// 処理開始前、または処理が安全な地点でキャンセルに応答しました。
    Cancelled,
    /// 実行器の終了、またはunwind可能なワーカーのpanicで結果を得られませんでした。
    /// panic=abort環境ではプロセスが終了するため、この値への変換はできません。
    WorkerStopped,
}
impl<E: std::fmt::Display> std::fmt::Display for TaskError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Operation(e) => write!(f, "{e}"),
            Self::Cancelled => f.write_str("task cancelled"),
            Self::WorkerStopped => f.write_str("task worker stopped"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for TaskError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let Self::Operation(e) = self {
            Some(e)
        } else {
            None
        }
    }
}
/// 型付きの処理結果です。完了コールバックへ一度だけ移動します。
pub type TaskResult<T, E> = Result<T, TaskError<E>>;

/// タスクの実行状態です。完了状態は処理本体の実際の結果に従います。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskStatus {
    /// 実行待ちです。
    Queued,
    /// 処理本体を実行しています。
    Running,
    /// 中止を要求しましたが、処理の終了は未確認です。
    CancellationRequested,
    /// 処理本体が成功しました。遅れて届いた中止要求では書き換えません。
    Succeeded,
    /// 処理本体が失敗しました。
    Failed,
    /// キャンセルに応答して終了しました。
    Cancelled,
    /// 結果を得られずワーカーが終了しました。
    WorkerStopped,
}
impl TaskStatus {
    /// 処理本体が終了して結果を確定した場合にtrueを返します。
    ///
    /// UIコールバックが配送済みかどうかは表しません。
    pub fn is_finished(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::WorkerStopped
        )
    }
}
/// 不正な進捗量（総量0、または処理済み量が総量を超過）を表します。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidProgress;
impl std::fmt::Display for InvalidProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("progress requires 0 <= completed <= total and total > 0")
    }
}
impl std::error::Error for InvalidProgress {}
/// 現在の工程と進捗量です。100%は処理成功の通知ではありません。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    message: String,
    units: Option<(u64, u64)>,
}
impl Progress {
    /// 総量が不明な工程を作成します。
    ///
    /// - `message`: UIに表示する工程名です。製品UIの言語に従って指定します。
    pub fn indeterminate(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            units: None,
        }
    }
    /// 処理済み量と総量を指定して工程を作成します。
    ///
    /// ## Parameters
    /// - `message`: 工程名です。
    /// - `completed`: 処理済みの量です。
    /// - `total`: 0より大きい総量です。
    /// ## Errors
    /// - [`InvalidProgress`]: 総量が0、またはcompletedがtotalを超える場合。
    /// ## Example
    /// ```rust
    /// let progress = appcore::task::Progress::measured("Reading files", 3, 10)?;
    /// assert_eq!(progress.completed(), Some(3));
    /// # Ok::<(), appcore::task::InvalidProgress>(())
    /// ```
    pub fn measured(
        message: impl Into<String>,
        completed: u64,
        total: u64,
    ) -> Result<Self, InvalidProgress> {
        if total == 0 || completed > total {
            Err(InvalidProgress)
        } else {
            Ok(Self {
                message: message.into(),
                units: Some((completed, total)),
            })
        }
    }
    /// 工程名を借用して返します。
    pub fn message(&self) -> &str {
        &self.message
    }
    /// 総量が分かる場合は処理済み量を返し、不明ならNoneを返します。
    pub fn completed(&self) -> Option<u64> {
        self.units.map(|u| u.0)
    }
    /// 総量が分かる場合は総量を返し、不明ならNoneを返します。
    pub fn total(&self) -> Option<u64> {
        self.units.map(|u| u.1)
    }
    /// 総量が分かる場合は0〜1の割合を返し、不明ならNoneを返します。
    pub fn fraction(&self) -> Option<f64> {
        self.units.map(|(done, total)| done as f64 / total as f64)
    }
}
struct Control {
    status: AtomicU8,
    progress: Mutex<(Option<Progress>, bool)>,
    notify: UiNotifier,
}
impl Control {
    fn cancel(&self) -> bool {
        let mut s = self.status.load(Ordering::Acquire);
        loop {
            let next = match s {
                QUEUED => CANCEL_QUEUED,
                RUNNING => CANCEL_RUNNING,
                _ => return false,
            };
            match self
                .status
                .compare_exchange_weak(s, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return true,
                Err(actual) => s = actual,
            }
        }
    }
}
/// タスクの状態確認とキャンセル要求に使う参照です。Cloneして他スレッドへ渡せます。
///
/// Dropだけではキャンセルしません。所有者のTaskScopeが寿命を管理します。
/// 結果の取得はUI側の完了コールバックに一本化しています。
#[derive(Clone)]
pub struct TaskHandle {
    control: Arc<Control>,
}
impl TaskHandle {
    /// 現在の実行状態を返します。UIコールバックの配送を待ちません。
    pub fn status(&self) -> TaskStatus {
        match self.control.status.load(Ordering::Acquire) {
            QUEUED => TaskStatus::Queued,
            RUNNING => TaskStatus::Running,
            CANCEL_QUEUED | CANCEL_RUNNING => TaskStatus::CancellationRequested,
            SUCCEEDED => TaskStatus::Succeeded,
            FAILED => TaskStatus::Failed,
            CANCELLED => TaskStatus::Cancelled,
            _ => TaskStatus::WorkerStopped,
        }
    }
    /// 中止を要求します。初回の要求を受け付けた場合にtrueを返します。
    ///
    /// 強制停止、I/Oの取り消し、終了待ちは行いません。処理が既に終了した場合や
    /// 要求済みの場合はfalseです。要求後に処理本体が成功を返すこともあります。
    pub fn cancel(&self) -> bool {
        self.control.cancel()
    }
    /// 最後に報告された進捗を複製します。進捗未報告ならNoneを返します。
    pub fn progress(&self) -> Option<Progress> {
        self.control
            .progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .0
            .clone()
    }
}
/// ワーカーへ渡される、キャンセル確認と進捗報告のためのコンテキストです。
///
/// UI状態やUIコールバックは保持しません。アプリのCapabilityは変更しません。
pub struct TaskContext {
    control: Arc<Control>,
}
impl TaskContext {
    /// 中止が要求されている場合にtrueを返します。終了済みかどうかは表しません。
    pub fn is_cancel_requested(&self) -> bool {
        matches!(
            self.control.status.load(Ordering::Acquire),
            CANCEL_QUEUED | CANCEL_RUNNING
        )
    }
    /// 安全に中断できる地点で中止要求を確認します。
    ///
    /// ## Errors
    /// - [`TaskError::Cancelled`]: 中止が要求されている場合。
    /// ## Example
    /// ```rust
    /// use appcore::task::{TaskContext, TaskResult};
    /// fn step(task: &TaskContext) -> TaskResult<(), std::io::Error> {
    ///     task.check_cancelled()?;
    ///     Ok(())
    /// }
    /// ```
    pub fn check_cancelled<E>(&self) -> TaskResult<(), E> {
        if self.is_cancel_requested() {
            Err(TaskError::Cancelled)
        } else {
            Ok(())
        }
    }
    /// 進捗を報告します。未配送の古い進捗は最新値へまとめられます。
    ///
    /// - `progress`: 現在の工程と進捗量です。
    ///
    /// 完了結果は別に保持するため、進捗が多くても結果を失いません。
    /// 進捗100%を報告しても、タスクの状態はSucceededになりません。
    pub fn report(&self, progress: Progress) {
        if self.control.status.load(Ordering::Acquire) >= SUCCEEDED {
            return;
        }
        let mut p = self
            .control
            .progress
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if p.0.as_ref() == Some(&progress) {
            return;
        }
        p.0 = Some(progress);
        p.1 = true;
        drop(p);
        self.control.notify.notify();
    }
}
trait UiJob {
    fn handle(&self) -> TaskHandle;
    fn deliver(&mut self, active: &Cell<bool>) -> bool;
    fn cancel(&self);
}
struct Job<T, E> {
    control: Arc<Control>,
    result: Arc<Mutex<Option<TaskResult<T, E>>>>,
    progress: Box<dyn FnMut(Progress)>,
    complete: Option<Box<dyn FnOnce(TaskResult<T, E>)>>,
}
impl<T, E> UiJob for Job<T, E> {
    fn handle(&self) -> TaskHandle {
        TaskHandle {
            control: self.control.clone(),
        }
    }
    fn cancel(&self) {
        self.control.cancel();
    }
    fn deliver(&mut self, active: &Cell<bool>) -> bool {
        let progress = {
            let mut p = self
                .control
                .progress
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if std::mem::take(&mut p.1) {
                p.0.clone()
            } else {
                None
            }
        };
        if let Some(p) = progress {
            (self.progress)(p)
        }
        if !active.get() {
            return true;
        }
        let result = self.result.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(result) = result {
            if let Some(complete) = self.complete.take() {
                complete(result)
            }
            true
        } else {
            false
        }
    }
}
struct ScopeState {
    active: Cell<bool>,
    next: Cell<u64>,
    jobs: RefCell<BTreeMap<u64, Box<dyn UiJob>>>,
    processing: RefCell<Vec<TaskHandle>>,
}
impl ScopeState {
    fn drain(&self) {
        let ids = self.jobs.borrow().keys().copied().collect::<Vec<_>>();
        for id in ids {
            let job = self.jobs.borrow_mut().remove(&id);
            if let Some(mut job) = job {
                self.processing.borrow_mut().push(job.handle());
                let finished = job.deliver(&self.active);
                self.processing.borrow_mut().pop();
                if !self.active.get() {
                    job.cancel();
                } else if !finished {
                    self.jobs.borrow_mut().insert(id, job);
                }
            }
        }
    }
}
/// 画面・ウィンドウ・アプリが所有するタスク群です。作成元のUIスレッドで使います。
///
/// この値を保持している間だけ結果をUIへ配送します。Dropは中止を要求して通知を
/// 解除しますが、ワーカー終了待ちはしません。UIコールバックにScopeの所有者を
/// 強く捕捉すると参照循環になるため、StateまたはWeakな所有者参照を捕捉してください。
/// 既定の実行器はプロセスで共有し、現在は2ワーカー・32実行待ちです。
/// Scopeごとの未配送タスクは64件までです。これらはSDKの実装上限でOS仕様ではありません。
///
/// UI状態をワーカーへ捕捉するとコンパイル時に拒否されます。
/// ```rust,compile_fail
/// use appcore::task::{TaskScope, TaskResult};
/// let ui = std::rc::Rc::new(std::cell::Cell::new(0));
/// let scope = TaskScope::new().unwrap();
/// scope.spawn(move |_| -> TaskResult<(), ()> { ui.set(1); Ok(()) }, |_| {});
/// ```
pub struct TaskScope {
    state: Rc<ScopeState>,
    subscription: UiSubscription,
    executor: TaskExecutor,
}
impl TaskScope {
    /// 現在のUIスレッドで、共有実行器を使うタスク群を作成します。
    ///
    /// ## Errors
    /// - [`TaskStartError::WorkerUnavailable`]: ワーカーを作成できない場合。
    /// - [`TaskStartError::UiCapacity`]: UI通知登録が上限に達した場合。
    /// ## Example
    /// ```rust,no_run
    /// let tasks = appcore::task::TaskScope::new()?;
    /// # Ok::<(), appcore::task::TaskStartError>(())
    /// ```
    pub fn new() -> Result<Self, TaskStartError> {
        Self::with_executor(&TaskExecutor::shared()?)
    }
    /// 指定した実行器を共有するタスク群を現在のUIスレッドで作成します。
    ///
    /// - `executor`: このScopeの処理を受け付ける実行器です。
    /// ## Errors
    /// - [`TaskStartError::UiCapacity`]: UI通知登録が上限に達した場合。
    pub fn with_executor(executor: &TaskExecutor) -> Result<Self, TaskStartError> {
        let state = Rc::new(ScopeState {
            active: Cell::new(true),
            next: Cell::new(1),
            jobs: RefCell::new(BTreeMap::new()),
            processing: RefCell::new(Vec::new()),
        });
        let weak = Rc::downgrade(&state);
        let subscription = UiSubscription::new(move || {
            if let Some(state) = weak.upgrade() {
                state.drain();
            }
        })
        .map_err(|_| TaskStartError::UiCapacity)?;
        Ok(Self {
            state,
            subscription,
            executor: executor.clone(),
        })
    }
    /// 同期処理を受け付け、結果を作成元のUIスレッドへ一度だけ配送します。
    ///
    /// ## Parameters
    /// - `work`: Sendな入力だけを捕捉する処理です。タスクの権限を追加しません。
    /// - `complete`: UI側で結果を受け取る処理です。Sendである必要はありません。
    /// ## Errors
    /// 実行器の満杯、Scope上限、識別子の枯渇は[`TaskStartError`]です。
    /// 受付失敗ではworkもcompleteも呼びません。登録成功後の失敗はcompleteへ配送します。
    /// ## Example
    /// ```rust,no_run
    /// use appcore::task::{TaskScope, TaskResult};
    /// let scope=TaskScope::new()?;
    /// scope.spawn(|_| -> TaskResult<_, ()> { Ok(42) }, |result| { assert_eq!(result, Ok(42)); })?;
    /// # Ok::<(), appcore::task::TaskStartError>(())
    /// ```
    pub fn spawn<T: Send + 'static, E: Send + 'static>(
        &self,
        work: impl FnOnce(TaskContext) -> TaskResult<T, E> + Send + 'static,
        complete: impl FnOnce(TaskResult<T, E>) + 'static,
    ) -> Result<TaskHandle, TaskStartError> {
        self.spawn_with_progress(work, |_| {}, complete)
    }
    /// 処理、UI進捗通知、UI完了通知をまとめて登録します。
    ///
    /// ## Parameters
    /// - `work`: ワーカーで実行する処理です。
    /// - `progress`: 最新の進捗をUIで受け取ります。全ての中間値の配送は保証しません。
    /// - `complete`: 最後の進捗通知の後に一度だけ結果を受け取ります。
    /// ## Errors
    /// [`Self::spawn`]と同じ受付制限です。Scopeを破棄した場合は以後のUI通知を止めます。
    pub fn spawn_with_progress<T: Send + 'static, E: Send + 'static>(
        &self,
        work: impl FnOnce(TaskContext) -> TaskResult<T, E> + Send + 'static,
        progress: impl FnMut(Progress) + 'static,
        complete: impl FnOnce(TaskResult<T, E>) + 'static,
    ) -> Result<TaskHandle, TaskStartError> {
        if self.state.jobs.borrow().len() + self.state.processing.borrow().len() >= MAX_SCOPE_TASKS
        {
            return Err(TaskStartError::ScopeFull);
        }
        let id = self.state.next.get();
        let next = id.checked_add(1).ok_or(TaskStartError::IdentityExhausted)?;
        let control = Arc::new(Control {
            status: AtomicU8::new(QUEUED),
            progress: Mutex::new((None, false)),
            notify: self.subscription.notifier(),
        });
        let result = Arc::new(Mutex::new(None));
        let worker_control = control.clone();
        let worker_result = result.clone();
        let worker = Box::new(move |closed: bool| {
            let outcome = if closed {
                Err(TaskError::WorkerStopped)
            } else if worker_control
                .status
                .compare_exchange(QUEUED, RUNNING, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                Err(TaskError::Cancelled)
            } else {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    work(TaskContext {
                        control: worker_control.clone(),
                    })
                }))
                .unwrap_or(Err(TaskError::WorkerStopped))
            };
            let status = match &outcome {
                Ok(_) => SUCCEEDED,
                Err(TaskError::Operation(_)) => FAILED,
                Err(TaskError::Cancelled) => CANCELLED,
                Err(TaskError::WorkerStopped) => STOPPED,
            };
            let mut slot = worker_result.lock().unwrap_or_else(|e| e.into_inner());
            *slot = Some(outcome);
            worker_control.status.store(status, Ordering::Release);
            drop(slot);
            worker_control.notify.notify();
        });
        self.state.jobs.borrow_mut().insert(
            id,
            Box::new(Job {
                control: control.clone(),
                result,
                progress: Box::new(progress),
                complete: Some(Box::new(complete)),
            }),
        );
        if let Err(error) = self.executor.submit(worker) {
            self.state.jobs.borrow_mut().remove(&id);
            return Err(error);
        }
        self.state.next.set(next);
        Ok(TaskHandle { control })
    }
    #[cfg(test)]
    pub(crate) fn dispatch_for_test(&self) {
        self.state.drain();
    }

    /// 未配送の結果または実行中の処理がある場合にtrueを返します。
    ///
    /// ウィンドウ終了を保留すべきか判断する際に使えます。終了待ちはしません。
    pub fn has_pending(&self) -> bool {
        !self.state.jobs.borrow().is_empty() || !self.state.processing.borrow().is_empty()
    }
    /// 所有している全てのタスクへ中止を要求します。終了待ちはしません。
    pub fn cancel_all(&self) {
        for job in self.state.jobs.borrow().values() {
            job.cancel();
        }
        for handle in self.state.processing.borrow().iter() {
            handle.cancel();
        }
    }
}
impl Drop for TaskScope {
    fn drop(&mut self) {
        self.state.active.set(false);
        self.cancel_all();
    }
}

/// 検索やプレビューで、最後に受け付けた要求の結果だけをUIへ反映するスロットです。
///
/// 副作用を取り消す機能ではないため、保存・送信・インストールには使用しません。
/// 新しい要求の受付失敗では現在の要求を維持します。Dropすると中止を要求します。
pub struct TaskSlot {
    generation: Rc<Cell<u64>>,
    current: Option<TaskHandle>,
}
impl TaskSlot {
    /// 空のスロットを現在のUIスレッドで作成します。
    pub fn new() -> Self {
        Self {
            generation: Rc::new(Cell::new(0)),
            current: None,
        }
    }
    /// 新しい処理を受け付け、受付成功後に以前の要求を中止・無効化します。
    ///
    /// ## Parameters
    /// - `scope`: タスクの寿命と配送先を所有するScopeです。
    /// - `work`: ワーカーで実行する検索などの処理です。
    /// - `complete`: 最新の要求が完了した場合だけUIで呼ぶ処理です。
    /// ## Errors
    /// [`TaskScope::spawn`]の受付エラー、または世代値枯渇を返します。
    pub fn spawn<T: Send + 'static, E: Send + 'static>(
        &mut self,
        scope: &TaskScope,
        work: impl FnOnce(TaskContext) -> TaskResult<T, E> + Send + 'static,
        complete: impl FnOnce(TaskResult<T, E>) + 'static,
    ) -> Result<TaskHandle, TaskStartError> {
        let next = self
            .generation
            .get()
            .checked_add(1)
            .ok_or(TaskStartError::IdentityExhausted)?;
        let weak = Rc::downgrade(&self.generation);
        let handle = scope.spawn(work, move |result| {
            if weak.upgrade().is_some_and(|g| g.get() == next) {
                complete(result)
            }
        })?;
        self.generation.set(next);
        if let Some(old) = self.current.replace(handle.clone()) {
            old.cancel();
        }
        Ok(handle)
    }
}
impl Default for TaskSlot {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for TaskSlot {
    fn drop(&mut self) {
        if let Some(handle) = &self.current {
            handle.cancel();
        }
    }
}

#[cfg(test)]
mod tests;
