//! Background I/O ABI. Kome closures are retained, invoked and released on UI only.
//! All identities are monotonic and looked up in the creating UI thread's table.
#![deny(missing_docs)]
use crate::task::{
    TaskContext, TaskError, TaskHandle, TaskResult, TaskScope, TaskStartError, TaskStatus,
};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    io::{self, Read},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};
const MAX_TASKS: usize = 1024;
const MAX_SCOPES: usize = 64;
const MAX_FILE: usize = 16 * 1024 * 1024;
const BYTE_BUDGET: usize = 64 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(1);
thread_local! {
    static LAST:Cell<i32>=const{Cell::new(0)};
    static SCOPES:RefCell<BTreeMap<u64,Scope>>=const{RefCell::new(BTreeMap::new())};
    static TASKS:RefCell<BTreeMap<u64,Record>>=const{RefCell::new(BTreeMap::new())};
    static BUDGET:Arc<AtomicUsize>=Arc::new(AtomicUsize::new(0));
}
struct Scope {
    tasks: BTreeSet<u64>,
    scope: Rc<TaskScope>,
}
struct Lease {
    budget: Arc<AtomicUsize>,
    bytes: usize,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.budget.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
struct Text {
    value: String,
    _lease: Arc<Lease>,
}
struct Record {
    owner: u64,
    handle: TaskHandle,
    result: Option<Result<Text, String>>,
    _lease: Arc<Lease>,
    outcome: u32,
}

/// KomeのUIクロージャに使用する、既存のクロージャABIと同じ先頭レイアウトです。
///
/// C利用者もこのレイアウトを使用できます。関数と所有権操作はUIスレッド専用です。
#[repr(C)]
pub struct TaskCompletion {
    /// 環境とタスクIDを受け取る `extern C fn(u64, u64)` のアドレスです。
    pub code: u64,
    /// コールバックへ渡す環境です。
    pub environment: u64,
    /// クロージャ自身のアドレスを受け取り、UIで参照を保持します。
    pub retain: unsafe extern "C" fn(u64),
    /// クロージャ自身のアドレスを受け取り、UIで参照を解放します。
    pub release: unsafe extern "C" fn(u64),
}
struct Completion {
    raw: *const TaskCompletion,
}
impl Completion {
    unsafe fn new(raw: *const TaskCompletion) -> Result<Self, i32> {
        let value = unsafe { raw.as_ref() }.ok_or(1)?;
        if value.code == 0 {
            return Err(3);
        }
        unsafe { (value.retain)(raw as u64) };
        Ok(Self { raw })
    }
    fn invoke(&self, id: u64) {
        // SAFETY: the caller supplied the documented closure ABI. This value
        // never enters a worker and remains retained until this UI callback ends.
        let value = unsafe { &*self.raw };
        let function: unsafe extern "C" fn(u64, u64) =
            unsafe { std::mem::transmute(value.code as usize) };
        unsafe { function(value.environment, id) };
    }
}
impl Drop for Completion {
    fn drop(&mut self) {
        let value = unsafe { &*self.raw };
        unsafe { (value.release)(self.raw as u64) }
    }
}
fn id() -> Result<u64, i32> {
    NEXT.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
        .map_err(|_| 1004)
}
fn start_error(e: TaskStartError) -> i32 {
    match e {
        TaskStartError::InvalidLimits => 3,
        TaskStartError::QueueFull => 1001,
        TaskStartError::ScopeFull | TaskStartError::UiCapacity => 1002,
        _ => 1004,
    }
}
fn enter<T: Default>(f: impl FnOnce() -> Result<T, i32>) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(v)) => {
            LAST.set(0);
            v
        }
        Ok(Err(e)) => {
            LAST.set(e);
            T::default()
        }
        Err(_) => {
            LAST.set(255);
            T::default()
        }
    }
}
fn read_text(
    ctx: TaskContext,
    path: String,
    max: usize,
    lease: Arc<Lease>,
) -> TaskResult<Text, io::Error> {
    ctx.check_cancelled()?;
    let mut file = std::fs::File::open(path).map_err(TaskError::Operation)?;
    if !file.metadata().map_err(TaskError::Operation)?.is_file() {
        return Err(TaskError::Operation(io::Error::other(
            "Background text reads require a regular file",
        )));
    }
    let mut bytes = Vec::new();
    // Keep allocated result capacity within the reservation instead of letting
    // Vec's geometric growth overshoot the per-UI budget for arbitrary limits.
    bytes.try_reserve_exact(max).map_err(|error| {
        TaskError::Operation(io::Error::other(format!("Text allocation failed: {error}")))
    })?;
    let mut chunk = [0; 8192];
    loop {
        ctx.check_cancelled()?;
        let count = file.read(&mut chunk).map_err(TaskError::Operation)?;
        if count == 0 {
            break;
        }
        if count > max - bytes.len() {
            return Err(TaskError::Operation(io::Error::other(
                "Text file exceeds its configured size limit",
            )));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    ctx.check_cancelled()?;
    let value = String::from_utf8(bytes)
        .map_err(|_| TaskError::Operation(io::Error::other("The file is not UTF-8 text")))?;
    Ok(Text {
        value,
        _lease: lease,
    })
}
/// 直前のバックグラウンドAPI呼び出しの受付状態を、呼び出したスレッドで返します。
///
/// 0=成功、1=ポインター不正、2=UTF-8不正、3=引数不正、
/// 1001=実行待ち満杯、1002=保持数/メモリ上限、1003=無効な識別子/スレッド、1004=実行環境なし、255=unwind可能なpanicです。
/// 非同期処理の結果はこの値に保存しません。
#[unsafe(no_mangle)]
pub extern "C" fn mochios_task_last_status() -> i32 {
    LAST.get()
}
/// 現在のUIスレッドにScopeを作成します。失敗時は0と受付状態を返します。
#[unsafe(no_mangle)]
pub extern "C" fn mochios_task_scope_create() -> u64 {
    enter(|| {
        if SCOPES.with_borrow(|s| s.len() >= MAX_SCOPES) {
            return Err(1002);
        }
        let scope = Rc::new(TaskScope::new().map_err(start_error)?);
        let id = id()?;
        SCOPES.with_borrow_mut(|s| {
            s.insert(
                id,
                Scope {
                    scope,
                    tasks: BTreeSet::new(),
                },
            )
        });
        Ok(id)
    })
}
/// Scopeの全タスクへ中止を要求し、UI通知と保持結果を破棄します。
///
/// 実行中のI/Oは強制停止せず、終了待ちもしません。成功時は0です。
#[unsafe(no_mangle)]
pub extern "C" fn mochios_task_scope_destroy(scope: u64) -> i32 {
    enter(|| {
        let owned = SCOPES.with_borrow_mut(|s| s.remove(&scope)).ok_or(1003)?;
        let removed = TASKS.with_borrow_mut(|tasks| {
            owned
                .tasks
                .iter()
                .filter_map(|id| tasks.remove(id))
                .collect::<Vec<_>>()
        });
        drop(removed);
        drop(owned);
        Ok(0)
    });
    LAST.get()
}
/// UTF-8の通常ファイルをワーカーで読み、結果のタスクIDをUIへ通知します。
///
/// max_bytesは1〜16MiBです。同一UIスレッドの予約メモリは合計64MiBまでです。
/// 返すIDはScope破棄またはtask_releaseまで有効です。失敗時は0です。
/// Capabilityは追加せず、通常のファイルアクセス制約に従います。
///
/// # Safety
/// pathはlengthバイトの有効なUTF-8を指してください。completionは
/// `extern C fn(environment, task_id)`を持つ有効なTaskCompletionを指し、
/// retain/releaseで寿命を管理できなければなりません。全操作は作成元のUIで行います。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mochios_task_read_text_utf8(
    scope: u64,
    path: *const u8,
    length: usize,
    max_bytes: usize,
    completion: *const TaskCompletion,
) -> u64 {
    enter(|| {
        if path.is_null() {
            return Err(1);
        }
        if length == 0 || length > 4096 || max_bytes == 0 || max_bytes > MAX_FILE {
            return Err(3);
        }
        if TASKS.with_borrow(|t| t.len() >= MAX_TASKS) {
            return Err(1002);
        }
        let path = std::str::from_utf8(unsafe { std::slice::from_raw_parts(path, length) })
            .map_err(|_| 2)?
            .to_owned();
        let owner = SCOPES
            .with_borrow(|s| s.get(&scope).map(|s| s.scope.clone()))
            .ok_or(1003)?;
        let budget = BUDGET.with(Clone::clone);
        budget
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                n.checked_add(max_bytes).filter(|sum| *sum <= BYTE_BUDGET)
            })
            .map_err(|_| 1002)?;
        let lease = Arc::new(Lease {
            budget,
            bytes: max_bytes,
        });
        let worker_lease = lease.clone();
        let action = unsafe { Completion::new(completion) }?;
        let id = id()?;
        let handle = owner
            .spawn(
                move |ctx| read_text(ctx, path, max_bytes, worker_lease),
                move |result| {
                    let present = TASKS.with_borrow_mut(|tasks| {
                        let Some(record) = tasks.get_mut(&id) else {
                            return false;
                        };
                        record.outcome = match &result {
                            Ok(_) => 0,
                            Err(TaskError::Operation(_)) => 1,
                            Err(TaskError::Cancelled) => 2,
                            Err(TaskError::WorkerStopped) => 3,
                        };
                        record.result = Some(result.map_err(|e| e.to_string()));
                        true
                    });
                    if present {
                        action.invoke(id)
                    }
                },
            )
            .map_err(start_error)?;
        TASKS.with_borrow_mut(|tasks| {
            tasks.insert(
                id,
                Record {
                    owner: scope,
                    handle,
                    result: None,
                    _lease: lease,
                    outcome: 4,
                },
            )
        });
        SCOPES.with_borrow_mut(|scopes| {
            if let Some(scope) = scopes.get_mut(&scope) {
                scope.tasks.insert(id);
            }
        });
        Ok(id)
    })
}
/// タスクの実行状態を返します。0=待機、1=実行中、2=中止要求、3=成功、
/// 4=失敗、5=中止終了、6=ワーカー停止、7=無効なIDです。
#[unsafe(no_mangle)]
pub extern "C" fn mochios_task_status(task: u64) -> u32 {
    let value = enter(|| {
        TASKS.with_borrow(|t| {
            t.get(&task)
                .map(|r| match r.handle.status() {
                    TaskStatus::Queued => 0,
                    TaskStatus::Running => 1,
                    TaskStatus::CancellationRequested => 2,
                    TaskStatus::Succeeded => 3,
                    TaskStatus::Failed => 4,
                    TaskStatus::Cancelled => 5,
                    TaskStatus::WorkerStopped => 6,
                })
                .ok_or(1003)
        })
    });
    if LAST.get() == 0 { value } else { 7 }
}
/// タスクへ中止を要求します。初回要求なら1、それ以外は0です。終了確認ではありません。
#[unsafe(no_mangle)]
pub extern "C" fn mochios_task_cancel(task: u64) -> u8 {
    enter(|| {
        TASKS.with_borrow(|t| {
            t.get(&task)
                .map(|r| u8::from(r.handle.cancel()))
                .ok_or(1003)
        })
    })
}
/// 配送済みの結果の種類を返します。0=成功、1=処理エラー、2=中止、3=停止、
/// 4=UIへの結果配送待ち、5=無効なIDです。ワーカーの実行状態とは別です。
#[unsafe(no_mangle)]
pub extern "C" fn mochios_task_result_status(task: u64) -> u32 {
    let v = enter(|| TASKS.with_borrow(|t| t.get(&task).map(|r| r.outcome).ok_or(1003)));
    if LAST.get() == 0 { v } else { 5 }
}
/// 成功結果の文字列を借用します。task_releaseまたはScope破棄まで、UIでのみ有効です。
///
/// 読み取る長さはtext_lengthで確認し、他のスレッドへポインターを持ち出さないでください。
#[unsafe(no_mangle)]
pub extern "C" fn mochios_task_text_data(task: u64) -> *const u8 {
    enter(|| {
        TASKS.with_borrow(|t| match t.get(&task).and_then(|r| r.result.as_ref()) {
            Some(Ok(text)) => Ok(text.value.as_ptr()),
            _ => Err(1003),
        })
    })
}
/// 成功結果のUTF-8バイト数を返します。無効なIDや未成功では0と受付エラーを返します。
#[unsafe(no_mangle)]
pub extern "C" fn mochios_task_text_length(task: u64) -> usize {
    enter(|| {
        TASKS.with_borrow(|t| match t.get(&task).and_then(|r| r.result.as_ref()) {
            Some(Ok(text)) => Ok(text.value.len()),
            _ => Err(1003),
        })
    })
}
/// 失敗結果の説明を借用します。error_lengthバイトがtask_release/Scope破棄まで有効です。
#[unsafe(no_mangle)]
pub extern "C" fn mochios_task_error_data(task: u64) -> *const u8 {
    enter(|| {
        TASKS.with_borrow(|t| match t.get(&task).and_then(|r| r.result.as_ref()) {
            Some(Err(text)) => Ok(text.as_ptr()),
            _ => Err(1003),
        })
    })
}
/// 失敗結果の説明のUTF-8バイト数を返します。未失敗なら0と受付エラーを返します。
#[unsafe(no_mangle)]
pub extern "C" fn mochios_task_error_length(task: u64) -> usize {
    enter(|| {
        TASKS.with_borrow(|t| match t.get(&task).and_then(|r| r.result.as_ref()) {
            Some(Err(text)) => Ok(text.len()),
            _ => Err(1003),
        })
    })
}
/// 中止を要求してタスクIDと保持結果を解放します。以後の完了通知を止めます。
///
/// 成功時は0です。ワーカー終了待ちはせず、使用済みIDも再利用しません。
#[unsafe(no_mangle)]
pub extern "C" fn mochios_task_release(task: u64) -> i32 {
    enter(|| {
        let record = TASKS.with_borrow_mut(|t| t.remove(&task)).ok_or(1003)?;
        record.handle.cancel();
        SCOPES.with_borrow_mut(|scopes| {
            if let Some(scope) = scopes.get_mut(&record.owner) {
                scope.tasks.remove(&task);
            }
        });
        drop(record);
        Ok(0)
    });
    LAST.get()
}

#[cfg(test)]
mod tests;
