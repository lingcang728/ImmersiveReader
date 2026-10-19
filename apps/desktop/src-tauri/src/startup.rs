//! Recovery blocks writers, while the window and bookshelf can paint immediately.
use std::sync::{Condvar, Mutex, OnceLock};
fn state() -> &'static (Mutex<bool>, Condvar) {
    static STATE: OnceLock<(Mutex<bool>, Condvar)> = OnceLock::new();
    STATE.get_or_init(|| (Mutex::new(false), Condvar::new()))
}
pub(crate) fn begin() {
    *state().0.lock().unwrap_or_else(|e| e.into_inner()) = true;
}
pub(crate) fn defer(work: impl FnOnce() + Send + 'static) {
    begin();
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
        let (busy, changed) = state();
        *busy.lock().unwrap_or_else(|e| e.into_inner()) = false;
        changed.notify_all();
        if result.is_err() {
            crate::storage::app_log("startup", "background recovery failed; writers released");
        }
    });
}
pub(crate) fn wait() {
    let (busy, changed) = state();
    let mut guard = busy.lock().unwrap_or_else(|e| e.into_inner());
    while *guard {
        guard = changed.wait(guard).unwrap_or_else(|e| e.into_inner());
    }
}
