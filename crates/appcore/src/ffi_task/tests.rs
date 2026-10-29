use super::*;
use std::time::{Duration, Instant};

struct Observed {
    thread: std::thread::ThreadId,
    retained: Cell<usize>,
    released: Cell<usize>,
    delivered: Cell<u64>,
    destroy: Cell<u64>,
}
unsafe extern "C" fn retain(raw: u64) {
    let descriptor = unsafe { &*(raw as *const TaskCompletion) };
    let observed = unsafe { &*(descriptor.environment as *const Observed) };
    assert_eq!(observed.thread, std::thread::current().id());
    observed.retained.set(observed.retained.get() + 1);
}
unsafe extern "C" fn release(raw: u64) {
    let descriptor = unsafe { &*(raw as *const TaskCompletion) };
    let observed = unsafe { &*(descriptor.environment as *const Observed) };
    assert_eq!(observed.thread, std::thread::current().id());
    observed.released.set(observed.released.get() + 1);
}
unsafe extern "C" fn completed(environment: u64, task: u64) {
    let observed = unsafe { &*(environment as *const Observed) };
    assert_eq!(observed.thread, std::thread::current().id());
    assert_eq!(observed.delivered.replace(task), 0);
    if observed.destroy.get() != 0 {
        assert_eq!(mochios_task_scope_destroy(observed.destroy.get()), 0);
    }
}
fn callback() -> (Box<Observed>, Box<TaskCompletion>) {
    let observed = Box::new(Observed {
        thread: std::thread::current().id(),
        retained: Cell::new(0),
        released: Cell::new(0),
        delivered: Cell::new(0),
        destroy: Cell::new(0),
    });
    let descriptor = Box::new(TaskCompletion {
        code: completed as *const () as u64,
        environment: &*observed as *const Observed as u64,
        retain,
        release,
    });
    (observed, descriptor)
}
fn pump(scope: u64, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        let scope = SCOPES.with_borrow(|s| s.get(&scope).unwrap().scope.clone());
        scope.dispatch_for_test();
        assert!(Instant::now() < deadline, "UI completion timed out");
        std::thread::yield_now();
    }
}
fn read(scope: u64, path: &str, limit: usize, callback: &TaskCompletion) -> u64 {
    unsafe { mochios_task_read_text_utf8(scope, path.as_ptr(), path.len(), limit, callback) }
}
#[test]
fn owned_results_and_closure_lifetime_stay_on_ui() {
    let scope = mochios_task_scope_create();
    assert_ne!(scope, 0);
    let path = std::env::temp_dir().join(format!("appcore-background-{scope}.txt"));
    std::fs::write(&path, "もち\n").unwrap();
    let (observed, callback) = callback();
    let task = read(scope, path.to_str().unwrap(), 64, &callback);
    assert_ne!(task, 0);
    assert_eq!(observed.retained.get(), 1);
    assert_eq!(observed.delivered.get(), 0);
    pump(scope, || observed.delivered.get() != 0);
    assert_eq!(observed.released.get(), 1);
    assert_eq!(mochios_task_result_status(task), 0);
    let text = unsafe {
        std::slice::from_raw_parts(mochios_task_text_data(task), mochios_task_text_length(task))
    };
    assert_eq!(text, "もち\n".as_bytes());
    // A lookup on another thread cannot resolve this identity or affect its owner.
    std::thread::spawn(move || {
        assert_eq!(mochios_task_status(task), 7);
        assert_eq!(mochios_task_release(task), 1003);
    })
    .join()
    .unwrap();
    assert_eq!(mochios_task_result_status(task), 0);
    assert_eq!(mochios_task_release(task), 0);
    assert_eq!(mochios_task_result_status(task), 5);
    assert_eq!(mochios_task_scope_destroy(scope), 0);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn startup_validation_and_file_errors_are_distinct() {
    let scope = mochios_task_scope_create();
    let (observed, callback) = callback();
    assert_eq!(read(scope, "unused", 0, &callback), 0);
    assert_eq!(mochios_task_last_status(), 3);
    assert_eq!(observed.retained.get(), 0);
    let task = read(
        scope,
        "/nonexistent/appcore-background-test",
        1024,
        &callback,
    );
    assert_ne!(task, 0);
    pump(scope, || observed.delivered.get() != 0);
    assert_eq!(mochios_task_result_status(task), 1);
    assert!(mochios_task_error_length(task) > 0);
    assert_eq!(mochios_task_scope_destroy(scope), 0);
}
#[test]
fn completion_can_destroy_its_own_scope() {
    let scope = mochios_task_scope_create();
    let (observed, callback) = callback();
    observed.destroy.set(scope);
    let task = read(
        scope,
        "/nonexistent/appcore-background-reentrant",
        64,
        &callback,
    );
    assert_ne!(task, 0);
    pump(scope, || observed.delivered.get() != 0);
    assert_eq!(observed.released.get(), 1);
    assert_eq!(mochios_task_result_status(task), 5);
}
#[test]
fn dropping_scope_suppresses_callback_and_releases_on_ui() {
    let scope = mochios_task_scope_create();
    let (observed, callback) = callback();
    let task = read(scope, "/nonexistent/appcore-background-drop", 64, &callback);
    assert_ne!(task, 0);
    assert_eq!(mochios_task_scope_destroy(scope), 0);
    assert_eq!(observed.delivered.get(), 0);
    assert_eq!(observed.released.get(), 1);
    assert_eq!(mochios_task_result_status(task), 5);
}

#[test]
fn oversized_and_invalid_utf8_files_fail_without_returning_partial_text() {
    for contents in [vec![b'a'; 129], vec![0xff, 0xfe]] {
        let scope = mochios_task_scope_create();
        let path = std::env::temp_dir().join(format!("appcore-background-limit-{scope}.txt"));
        std::fs::write(&path, contents).unwrap();
        let (observed, callback) = callback();
        let task = read(scope, path.to_str().unwrap(), 128, &callback);
        assert_ne!(task, 0);
        pump(scope, || observed.delivered.get() != 0);
        assert_eq!(mochios_task_result_status(task), 1);
        assert!(mochios_task_text_data(task).is_null());
        assert!(mochios_task_error_length(task) > 0);
        assert_eq!(mochios_task_scope_destroy(scope), 0);
        std::fs::remove_file(path).unwrap();
    }
}
