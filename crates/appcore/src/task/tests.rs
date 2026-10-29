use super::*;
use std::{
    sync::{
        Barrier,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "task test timed out");
        std::thread::yield_now();
    }
}
fn scope() -> TaskScope {
    TaskScope::with_executor(&TaskExecutor::new(2, 32).unwrap()).unwrap()
}
#[test]
fn typed_results_are_delivered_once_on_ui_and_never_on_workers() {
    let scope = scope();
    let main = std::thread::current().id();
    let value = Rc::new(RefCell::new(None));
    let v = value.clone();
    let h = scope
        .spawn(
            move |_| -> TaskResult<_, String> {
                assert_ne!(std::thread::current().id(), main);
                Ok(42)
            },
            move |r| {
                assert_eq!(std::thread::current().id(), main);
                assert!(v.borrow().is_none());
                *v.borrow_mut() = Some(r);
            },
        )
        .unwrap();
    wait_until(|| h.status().is_finished());
    assert!(value.borrow().is_none());
    scope.state.drain();
    scope.state.drain();
    assert_eq!(*value.borrow(), Some(Ok(42)));
    assert!(!scope.has_pending());
    assert!(!h.cancel());
}
#[test]
fn cancellation_request_does_not_mean_worker_stopped_or_overwrite_committed_success() {
    let scope = scope();
    let (ready, seen) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let r = Rc::new(RefCell::new(None));
    let result = r.clone();
    let h = scope
        .spawn(
            move |ctx| -> TaskResult<_, ()> {
                ready.send(()).unwrap();
                wait.recv().unwrap();
                assert!(ctx.is_cancel_requested());
                Ok("committed")
            },
            move |v| *result.borrow_mut() = Some(v),
        )
        .unwrap();
    seen.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(h.cancel());
    assert!(!h.cancel());
    assert_eq!(h.status(), TaskStatus::CancellationRequested);
    assert!(!h.status().is_finished());
    release.send(()).unwrap();
    wait_until(|| h.status().is_finished());
    scope.state.drain();
    assert_eq!(*r.borrow(), Some(Ok("committed")));
    assert_eq!(h.status(), TaskStatus::Succeeded);
}
#[test]
fn cooperative_cancellation_returns_cancelled_only_after_checkpoint() {
    let scope = scope();
    let barrier = Arc::new(Barrier::new(2));
    let b = barrier.clone();
    let (ready, seen) = mpsc::channel();
    let result = Rc::new(RefCell::new(None));
    let r = result.clone();
    let h = scope
        .spawn(
            move |ctx| -> TaskResult<(), ()> {
                ready.send(()).unwrap();
                b.wait();
                ctx.check_cancelled()?;
                Ok(())
            },
            move |v| *r.borrow_mut() = Some(v),
        )
        .unwrap();
    seen.recv_timeout(Duration::from_secs(5)).unwrap();
    h.cancel();
    assert_eq!(h.status(), TaskStatus::CancellationRequested);
    barrier.wait();
    wait_until(|| h.status().is_finished());
    scope.state.drain();
    assert_eq!(*result.borrow(), Some(Err(TaskError::Cancelled)));
}
#[test]
fn queue_is_bounded_and_cancelled_queued_work_is_never_called() {
    let executor = TaskExecutor::new(1, 1).unwrap();
    let scope = TaskScope::with_executor(&executor).unwrap();
    let (ready, seen) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let first = scope
        .spawn(
            move |_| -> TaskResult<(), ()> {
                ready.send(()).unwrap();
                wait.recv().unwrap();
                Ok(())
            },
            |_| {},
        )
        .unwrap();
    seen.recv_timeout(Duration::from_secs(5)).unwrap();
    let called = Arc::new(AtomicUsize::new(0));
    let count = called.clone();
    let second = scope
        .spawn(
            move |_| -> TaskResult<(), ()> {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            |r| assert_eq!(r, Err(TaskError::Cancelled)),
        )
        .unwrap();
    assert!(matches!(
        scope.spawn(
            |_| -> TaskResult<(), ()> { panic!("full queue") },
            |_| panic!("not accepted")
        ),
        Err(TaskStartError::QueueFull)
    ));
    second.cancel();
    release.send(()).unwrap();
    wait_until(|| first.status().is_finished() && second.status().is_finished());
    scope.state.drain();
    assert_eq!(called.load(Ordering::SeqCst), 0);
}
#[test]
fn scope_drop_suppresses_callbacks_without_joining_or_stopping_worker() {
    let scope = scope();
    let (ready, seen) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let (ended, done) = mpsc::channel();
    let h = scope
        .spawn(
            move |ctx| -> TaskResult<(), ()> {
                ready.send(()).unwrap();
                wait.recv().unwrap();
                assert!(ctx.is_cancel_requested());
                ended.send(()).unwrap();
                Ok(())
            },
            |_| panic!("closed UI must not receive result"),
        )
        .unwrap();
    seen.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(scope);
    assert_eq!(h.status(), TaskStatus::CancellationRequested);
    release.send(()).unwrap();
    done.recv_timeout(Duration::from_secs(5)).unwrap();
    wait_until(|| h.status().is_finished());
}
#[test]
fn progress_is_coalesced_and_does_not_replace_terminal_result() {
    let scope = scope();
    let events = Rc::new(RefCell::new(Vec::new()));
    let p = events.clone();
    let r = events.clone();
    let h = scope.spawn_with_progress(
        |ctx| -> TaskResult<_, ()> {
            for n in 0..=10000 {
                ctx.report(Progress::measured("Working", n, 10000).unwrap());
            }
            Ok(19)
        },
        move |progress| {
            p.borrow_mut().push(progress.completed().unwrap());
        },
        move |v| {
            assert_eq!(v, Ok(19));
            r.borrow_mut().push(20000)
        },
    );
    let h = h.unwrap();
    wait_until(|| h.status().is_finished());
    assert_eq!(h.progress().unwrap().completed(), Some(10000));
    scope.state.drain();
    assert_eq!(*events.borrow(), vec![10000, 20000]);
}
#[test]
fn progress_then_completion_order_and_reentrant_scope_access() {
    let scope = Rc::new(scope());
    let weak = Rc::downgrade(&scope);
    let order = Rc::new(RefCell::new(Vec::new()));
    let p = order.clone();
    let c = order.clone();
    let h = scope
        .spawn_with_progress(
            |ctx| -> TaskResult<(), ()> {
                ctx.report(Progress::measured("Done", 1, 1).unwrap());
                Ok(())
            },
            move |_| {
                let scope = weak.upgrade().unwrap();
                assert!(scope.has_pending());
                scope.cancel_all();
                p.borrow_mut().push("progress")
            },
            move |r| {
                assert_eq!(r, Ok(()));
                c.borrow_mut().push("complete")
            },
        )
        .unwrap();
    wait_until(|| h.status().is_finished());
    scope.state.drain();
    assert_eq!(*order.borrow(), vec!["progress", "complete"]);
    assert!(!scope.has_pending());
}
#[test]
fn latest_slot_ignores_older_success_even_if_work_ignores_cancellation() {
    let scope = scope();
    let mut slot = TaskSlot::new();
    let (ready, seen) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let values = Rc::new(RefCell::new(Vec::new()));
    let v = values.clone();
    let old = slot
        .spawn(
            &scope,
            move |_| -> TaskResult<_, ()> {
                ready.send(()).unwrap();
                wait.recv().unwrap();
                Ok(1)
            },
            move |r| v.borrow_mut().push(r.unwrap()),
        )
        .unwrap();
    seen.recv_timeout(Duration::from_secs(5)).unwrap();
    let v = values.clone();
    let new = slot
        .spawn(
            &scope,
            |_| -> TaskResult<_, ()> { Ok(2) },
            move |r| v.borrow_mut().push(r.unwrap()),
        )
        .unwrap();
    wait_until(|| new.status().is_finished());
    scope.state.drain();
    release.send(()).unwrap();
    wait_until(|| old.status().is_finished());
    scope.state.drain();
    assert_eq!(*values.borrow(), vec![2]);
}
#[test]
fn terminal_operation_error_is_not_changed_into_cancellation() {
    let scope = scope();
    let result = Rc::new(RefCell::new(None));
    let r = result.clone();
    let h = scope
        .spawn(
            |_| -> TaskResult<(), _> { Err(TaskError::Operation("device failure")) },
            move |v| *r.borrow_mut() = Some(v),
        )
        .unwrap();
    wait_until(|| h.status().is_finished());
    assert!(!h.cancel());
    scope.state.drain();
    assert_eq!(
        *result.borrow(),
        Some(Err(TaskError::Operation("device failure")))
    );
}
#[cfg(panic = "unwind")]
#[test]
fn panic_finishes_once_and_worker_pool_remains_usable() {
    let scope = scope();
    let h = scope
        .spawn(
            |_| -> TaskResult<(), ()> { panic!("injected worker panic") },
            |r| assert_eq!(r, Err(TaskError::WorkerStopped)),
        )
        .unwrap();
    wait_until(|| h.status().is_finished());
    scope.state.drain();
    let h = scope
        .spawn(|_| -> TaskResult<_, ()> { Ok(1) }, |r| assert_eq!(r, Ok(1)))
        .unwrap();
    wait_until(|| h.status().is_finished());
    scope.state.drain();
}
#[test]
fn invalid_limits_and_progress_are_explicit_errors() {
    assert!(matches!(
        TaskExecutor::new(0, 1),
        Err(TaskStartError::InvalidLimits)
    ));
    assert!(Progress::measured("x", 1, 0).is_err());
    assert!(Progress::measured("x", 3, 2).is_err());
    assert_eq!(Progress::indeterminate("Starting").fraction(), None);
}

#[test]
fn completed_but_undelivered_results_still_count_toward_scope_limit() {
    let executor = TaskExecutor::new(1, 128).unwrap();
    let scope = TaskScope::with_executor(&executor).unwrap();
    let mut handles = Vec::new();
    for _ in 0..64 {
        handles.push(
            scope
                .spawn(|_| -> TaskResult<(), ()> { Ok(()) }, |_| {})
                .unwrap(),
        );
    }
    wait_until(|| handles.iter().all(|handle| handle.status().is_finished()));
    assert!(matches!(
        scope.spawn(|_| -> TaskResult<(), ()> { Ok(()) }, |_| {}),
        Err(TaskStartError::ScopeFull)
    ));
    scope.state.drain();
    assert!(
        scope
            .spawn(|_| -> TaskResult<(), ()> { Ok(()) }, |_| {})
            .is_ok()
    );
}

#[test]
fn failed_slot_submission_preserves_previous_request() {
    let executor = TaskExecutor::new(1, 1).unwrap();
    let scope = TaskScope::with_executor(&executor).unwrap();
    let mut slot = TaskSlot::new();
    let (started, ready) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let seen = Rc::new(Cell::new(false));
    let result = seen.clone();
    let old = slot
        .spawn(
            &scope,
            move |_| -> TaskResult<_, ()> {
                started.send(()).unwrap();
                wait.recv().unwrap();
                Ok(7)
            },
            move |r| {
                assert_eq!(r, Ok(7));
                result.set(true)
            },
        )
        .unwrap();
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    let queued = scope
        .spawn(|_| -> TaskResult<(), ()> { Ok(()) }, |_| {})
        .unwrap();
    assert!(matches!(
        slot.spawn(
            &scope,
            |_| -> TaskResult<(), ()> { Ok(()) },
            |_| panic!("rejected")
        ),
        Err(TaskStartError::QueueFull)
    ));
    assert_eq!(old.status(), TaskStatus::Running);
    release.send(()).unwrap();
    wait_until(|| old.status().is_finished() && queued.status().is_finished());
    scope.state.drain();
    assert!(seen.get());
}
