//! Task Spawning, Sleep, Timeout — FFI entry points
//!
//! All task operations use the global runtime from `runtime.rs`.
//!
//! SAFETY:
//! - All FFI functions wrapped in `catch_unwind` — no panics cross boundary
//! - `is_shutdown()` checked before spawning — reject work after shutdown
//! - Detached tasks limited by `try_acquire_detached_slot` — prevents thread exhaustion
//! - Timeouts use `spawn_blocking` so the timer can actually fire
//! - `safe_block_on` avoids nested `block_on` panics

use crate::runtime::{
    get_runtime, is_shutdown, make_err_result, make_ok_result, release_detached_slot,
    safe_block_on, try_acquire_detached_slot,
};
use crate::task_handle::{SendResult, TaskHandle};
use doo_ffi_core::DooResult;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Duration;

/// Type alias for async task function pointers from Doo codegen.
/// These are `extern "C"` functions that accept an environment pointer
/// (for captured variables) and return `*mut DooResult`.
pub type DooAsyncFn = extern "C" fn(*mut std::ffi::c_void) -> *mut DooResult;

/// Send-safe wrapper for a function pointer + env pointer pair.
/// SAFETY: `func` is a static code address (always safe to send).
/// `env` is either null or a heap-allocated struct exclusively owned
/// by the spawned task (ownership transfers on spawn).
pub(crate) struct SpawnPayload {
    pub(crate) func: DooAsyncFn,
    pub(crate) env: *mut std::ffi::c_void,
}
unsafe impl Send for SpawnPayload {}

impl SpawnPayload {
    /// Execute the function with its environment pointer.
    #[inline]
    pub(crate) fn call(self) -> *mut DooResult {
        (self.func)(self.env)
    }
}

// ============================================================================
// Spawn
// ============================================================================

/// Spawn a task on a dedicated blocking thread pool.
/// Returns a TaskHandle pointer that can be awaited or cancelled.
///
/// Uses `spawn_blocking` so the task body runs on a dedicated OS thread,
/// NOT on Tokio async worker threads. This means blocking operations
/// (sleep, file I/O, sync HTTP) inside `go {}` blocks don't starve
/// the async runtime — matching Go's goroutine design.
///
/// Returns null if runtime is shut down.
#[no_mangle]
pub extern "C" fn doo_spawn(func: DooAsyncFn, env: *mut std::ffi::c_void) -> *mut TaskHandle {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if is_shutdown() {
            return std::ptr::null_mut();
        }
        let rt = get_runtime();
        let payload = SpawnPayload { func, env };
        let handle = rt.spawn_blocking(move || {
            let r = catch_unwind(AssertUnwindSafe(|| payload.call()));
            match r {
                Ok(ptr) => SendResult(ptr),
                Err(_) => SendResult(make_err_result("task panicked")),
            }
        });
        TaskHandle::new(handle).into_raw()
    }));
    result.unwrap_or(std::ptr::null_mut())
}

/// Spawn a detached task — fire-and-forget.
/// No handle is returned; the task runs independently on a blocking thread.
///
/// SAFETY:
/// - Rejects work after shutdown
/// - Limited by MAX_DETACHED_TASKS to prevent thread pool exhaustion
/// - Panics inside the task are caught and do not propagate
#[no_mangle]
pub extern "C" fn doo_spawn_detach(func: DooAsyncFn, env: *mut std::ffi::c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if is_shutdown() {
            return;
        }
        if !try_acquire_detached_slot() {
            return;
        }
        let rt = get_runtime();
        let payload = SpawnPayload { func, env };
        rt.spawn_blocking(move || {
            let _guard = DetachedSlotGuard;
            let _ = catch_unwind(AssertUnwindSafe(|| {
                payload.call();
            }));
        });
    }));
}

/// RAII guard that releases a detached task slot when the task completes.
struct DetachedSlotGuard;
impl Drop for DetachedSlotGuard {
    fn drop(&mut self) {
        release_detached_slot();
    }
}

// ============================================================================
// Sleep
// ============================================================================

/// Sleep for the given number of milliseconds.
/// Uses `block_in_place` when inside a Tokio runtime context,
/// so Tokio can move other tasks off this thread.
/// Falls back to simple `thread::sleep` outside async contexts.
#[no_mangle]
pub extern "C" fn doo_sleep(ms: i64) -> *mut DooResult {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let duration = Duration::from_millis(ms.max(0) as u64);
        if tokio::runtime::Handle::try_current().is_ok() {
            // Inside a Tokio task — tell runtime we're about to block
            tokio::task::block_in_place(|| {
                std::thread::sleep(duration);
            });
        } else {
            std::thread::sleep(duration);
        }
        make_ok_result()
    }));
    result.unwrap_or_else(|_| make_err_result("sleep panicked"))
}

/// Async-friendly sleep — spawns a sleep task and returns a handle.
/// The caller should await the returned handle.
#[no_mangle]
pub extern "C" fn doo_sleep_async(ms: i64) -> *mut TaskHandle {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if is_shutdown() {
            return std::ptr::null_mut();
        }
        let duration = Duration::from_millis(ms.max(0) as u64);
        let rt = get_runtime();
        let handle = rt.spawn(async move {
            tokio::time::sleep(duration).await;
            SendResult(make_ok_result())
        });
        TaskHandle::new(handle).into_raw()
    }));
    result.unwrap_or(std::ptr::null_mut())
}

// ============================================================================
// Timeout
// ============================================================================

/// Run a function with a timeout in milliseconds.
/// Returns Ok with the function's result if it completes in time,
/// or Err with "timeout" if it exceeds the deadline.
///
/// Uses `spawn_blocking` for the function body so the timeout timer
/// can actually fire (blocking code on the async executor would prevent it).
/// Uses `safe_block_on` to avoid nested `block_on` panics.
#[no_mangle]
pub extern "C" fn doo_timeout(ms: i64, func: DooAsyncFn) -> *mut DooResult {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if is_shutdown() {
            return make_err_result("runtime shut down");
        }
        let duration = Duration::from_millis(ms.max(0) as u64);

        let payload = SpawnPayload {
            func,
            env: std::ptr::null_mut(),
        };
        safe_block_on(async move {
            let handle = tokio::task::spawn_blocking(move || {
                let r = catch_unwind(AssertUnwindSafe(|| payload.call()));
                match r {
                    Ok(ptr) => SendResult(ptr),
                    Err(_) => SendResult(make_err_result("timed task panicked")),
                }
            });
            match tokio::time::timeout(duration, handle).await {
                Ok(Ok(send_result)) => send_result.0,
                Ok(Err(e)) => make_err_result(&format!("timed task failed: {}", e)),
                Err(_) => make_err_result("timeout"),
            }
        })
    }));
    result.unwrap_or_else(|_| make_err_result("timeout panicked"))
}

/// Async-friendly timeout — spawns the timeout as a task.
/// The caller should await the returned handle.
///
/// Uses `spawn_blocking` for the function body so timeout can fire.
#[no_mangle]
pub extern "C" fn doo_timeout_async(ms: i64, func: DooAsyncFn) -> *mut TaskHandle {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if is_shutdown() {
            return std::ptr::null_mut();
        }
        let duration = Duration::from_millis(ms.max(0) as u64);
        let rt = get_runtime();
        let payload = SpawnPayload {
            func,
            env: std::ptr::null_mut(),
        };
        let handle = rt.spawn(async move {
            let blocking_handle = tokio::task::spawn_blocking(move || {
                let r = catch_unwind(AssertUnwindSafe(|| payload.call()));
                match r {
                    Ok(ptr) => SendResult(ptr),
                    Err(_) => SendResult(make_err_result("timed task panicked")),
                }
            });
            match tokio::time::timeout(duration, blocking_handle).await {
                Ok(Ok(send_result)) => send_result,
                Ok(Err(e)) => SendResult(make_err_result(&format!("timed task failed: {}", e))),
                Err(_) => SendResult(make_err_result("timeout")),
            }
        });
        TaskHandle::new(handle).into_raw()
    }));
    result.unwrap_or(std::ptr::null_mut())
}

// ============================================================================
// Spawn Blocking — for CPU-heavy work
// ============================================================================

/// Spawn a blocking task on a dedicated thread pool.
/// Use this for CPU-heavy work that shouldn't block the async runtime.
/// Returns a TaskHandle that can be awaited.
#[no_mangle]
pub extern "C" fn doo_spawn_blocking(func: DooAsyncFn) -> *mut TaskHandle {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if is_shutdown() {
            return std::ptr::null_mut();
        }
        let rt = get_runtime();
        let payload = SpawnPayload {
            func,
            env: std::ptr::null_mut(),
        };
        let handle = rt.spawn_blocking(move || {
            let r = catch_unwind(AssertUnwindSafe(|| payload.call()));
            match r {
                Ok(ptr) => SendResult(ptr),
                Err(_) => SendResult(make_err_result("blocking task panicked")),
            }
        });
        TaskHandle::new(handle).into_raw()
    }));
    result.unwrap_or(std::ptr::null_mut())
}
