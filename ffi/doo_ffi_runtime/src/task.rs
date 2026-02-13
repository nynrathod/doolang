//! Task Spawning, Sleep, Timeout — FFI entry points
//!
//! All task operations use the global runtime from `runtime.rs`.

use crate::runtime::{get_runtime, make_err_result, make_ok_result};
use crate::task_handle::{SendResult, TaskHandle};
use doo_ffi_core::DooResult;
use std::time::Duration;

/// Type alias for async task function pointers from Doo codegen.
/// These are `extern "C"` functions that return `*mut DooResult`.
pub type DooAsyncFn = extern "C" fn() -> *mut DooResult;

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
#[no_mangle]
pub extern "C" fn doo_spawn(func: DooAsyncFn) -> *mut TaskHandle {
    let rt = get_runtime();
    let handle = rt.spawn_blocking(move || SendResult(func()));
    TaskHandle::new(handle).into_raw()
}

/// Spawn a detached task — fire-and-forget.
/// No handle is returned; the task runs independently on a blocking thread.
#[no_mangle]
pub extern "C" fn doo_spawn_detach(func: DooAsyncFn) {
    let rt = get_runtime();
    rt.spawn_blocking(move || {
        let _ = func();
    });
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
}

/// Async-friendly sleep — spawns a sleep task and returns a handle.
/// The caller should await the returned handle.
#[no_mangle]
pub extern "C" fn doo_sleep_async(ms: i64) -> *mut TaskHandle {
    let duration = Duration::from_millis(ms.max(0) as u64);
    let rt = get_runtime();
    let handle = rt.spawn(async move {
        tokio::time::sleep(duration).await;
        SendResult(make_ok_result())
    });
    TaskHandle::new(handle).into_raw()
}

// ============================================================================
// Timeout
// ============================================================================

/// Run a function with a timeout in milliseconds.
/// Returns Ok with the function's result if it completes in time,
/// or Err with "timeout" if it exceeds the deadline.
#[no_mangle]
pub extern "C" fn doo_timeout(ms: i64, func: DooAsyncFn) -> *mut DooResult {
    let duration = Duration::from_millis(ms.max(0) as u64);
    let rt = get_runtime();

    rt.block_on(async move {
        match tokio::time::timeout(duration, async { SendResult(func()) }).await {
            Ok(result) => result.0,
            Err(_) => make_err_result("timeout"),
        }
    })
}

/// Async-friendly timeout — spawns the timeout as a task.
/// The caller should await the returned handle.
#[no_mangle]
pub extern "C" fn doo_timeout_async(ms: i64, func: DooAsyncFn) -> *mut TaskHandle {
    let duration = Duration::from_millis(ms.max(0) as u64);
    let rt = get_runtime();
    let handle = rt.spawn(async move {
        match tokio::time::timeout(duration, async { SendResult(func()) }).await {
            Ok(result) => result,
            Err(_) => SendResult(make_err_result("timeout")),
        }
    });
    TaskHandle::new(handle).into_raw()
}

// ============================================================================
// Spawn Blocking — for CPU-heavy work
// ============================================================================

/// Spawn a blocking task on a dedicated thread pool.
/// Use this for CPU-heavy work that shouldn't block the async runtime.
/// Returns a TaskHandle that can be awaited.
#[no_mangle]
pub extern "C" fn doo_spawn_blocking(func: DooAsyncFn) -> *mut TaskHandle {
    let rt = get_runtime();
    let handle = rt.spawn_blocking(move || SendResult(func()));
    TaskHandle::new(handle).into_raw()
}
