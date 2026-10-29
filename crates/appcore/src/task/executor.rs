//! Bounded blocking workers. No UI callback or UI state enters this queue.
use super::TaskStartError;
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex, OnceLock},
};
type Work = Box<dyn FnOnce(bool) + Send>;
struct Queue {
    pending: VecDeque<Work>,
    closed: bool,
}
struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
    capacity: usize,
}
struct Owner {
    shared: Arc<Shared>,
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.shared
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .closed = true;
        self.shared.ready.notify_all();
    }
}

/// 同期処理をUIスレッドの外で実行する、実行数と受付数が制限された実行器です。
///
/// Cloneは同じ実行器を共有します。最後の参照を破棄すると新規受付を閉じ、
/// 待機中の処理を実行せず終了させます。実行中の処理を強制停止せず、joinもしません。
#[derive(Clone)]
pub struct TaskExecutor {
    owner: Arc<Owner>,
}
impl TaskExecutor {
    /// 制限を指定してワーカースレッドを作成します。
    ///
    /// ## Parameters
    /// - `worker_count`: 同時実行数です。1〜32を指定します。
    /// - `queue_capacity`: 実行待ちの上限です。1〜1024を指定します。
    /// ## Errors
    /// - [`TaskStartError::InvalidLimits`]: 引数が範囲外の場合。
    /// - [`TaskStartError::WorkerUnavailable`]: スレッドを作成できない場合。
    /// ## Example
    /// ```rust,no_run
    /// let executor = appcore::task::TaskExecutor::new(2, 32)?;
    /// # Ok::<(), appcore::task::TaskStartError>(())
    /// ```
    pub fn new(worker_count: usize, queue_capacity: usize) -> Result<Self, TaskStartError> {
        if !(1..=32).contains(&worker_count) || !(1..=1024).contains(&queue_capacity) {
            return Err(TaskStartError::InvalidLimits);
        }
        let owner = Arc::new(Owner {
            shared: Arc::new(Shared {
                queue: Mutex::new(Queue {
                    pending: VecDeque::new(),
                    closed: false,
                }),
                ready: Condvar::new(),
                capacity: queue_capacity,
            }),
        });
        for index in 0..worker_count {
            let shared = owner.shared.clone();
            std::thread::Builder::new()
                .name(format!("appcore-task-{index}"))
                .spawn(move || {
                    loop {
                        let (work, closed) = {
                            let mut q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
                            loop {
                                if let Some(work) = q.pending.pop_front() {
                                    break (work, q.closed);
                                }
                                if q.closed {
                                    return;
                                }
                                q = shared.ready.wait(q).unwrap_or_else(|e| e.into_inner());
                            }
                        };
                        work(closed);
                    }
                })
                .map_err(|_| TaskStartError::WorkerUnavailable)?;
        }
        Ok(Self { owner })
    }
    pub(super) fn shared() -> Result<Self, TaskStartError> {
        static EXECUTOR: OnceLock<Result<TaskExecutor, TaskStartError>> = OnceLock::new();
        EXECUTOR.get_or_init(|| Self::new(2, 32)).clone()
    }
    pub(super) fn submit(&self, work: Work) -> Result<(), TaskStartError> {
        let mut q = self
            .owner
            .shared
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if q.closed {
            return Err(TaskStartError::WorkerUnavailable);
        }
        if q.pending.len() >= self.owner.shared.capacity {
            return Err(TaskStartError::QueueFull);
        }
        q.pending.push_back(work);
        drop(q);
        self.owner.shared.ready.notify_one();
        Ok(())
    }
}
