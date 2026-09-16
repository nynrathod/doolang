//! Task Handle — Awaitable, cancellable handle to a spawned task
//!
//! Wraps `tokio::task::JoinHandle` behind an opaque FFI pointer.
//! Doo code interacts with this through FFI functions only.
//!
//! SAFETY:
//! - All FFI functions wrapped in `catch_unwind`
//! - `safe_block_on` avoids nested `block_on` panics in `doo_task_await`
//! - `doo_task_cancel` aborts + frees in one call (no handle leak)

use crate::runtime::{make_err_result, safe_block_on};
use doo_ffi_core::DooResult;
use std::panic::{catch_unwind, AssertUnwindSafe};
use tokio::task::JoinHandle;

/// Wrapper for *mut DooResult to satisfy Send bounds on JoinHandle output.
/// SAFETY: DooResult pointers are heap-allocated and exclusively owned.
pub(crate) struct SendResult(pub *mut DooResult);
unsafe impl Send for SendResult {}

/// Opaque task handle wrapping a Tokio JoinHandle.
/// The inner handle produces `SendResult` when awaited.
pub struct TaskHandle {
    handle: JoinHandle<SendResult>,
}

// SAFETY: The JoinHandle is Send, and we only access it from async contexts.
unsafe impl Send for TaskHandle {}
unsafe impl Sync for TaskHandle {}

impl TaskHandle {
    /// Create a new TaskHandle from a JoinHandle.
    pub fn new(handle: JoinHandle<SendResult>) -> Self {
        Self { handle }
    }

    /// Convert to raw pointer for FFI.
    pub fn into_raw(self) -> *mut TaskHandle {
        Box::into_raw(Box::new(self))
    }

    /// Reconstruct from raw pointer.
    ///
    /// # Safety
    /// Pointer must have been created by `into_raw` and not freed yet.
    pub unsafe fn from_raw(ptr: *mut TaskHandle) -> Box<TaskHandle> {
        Box::from_raw(ptr)
    }
}

// ============================================================================
// FFI Functions
// ============================================================================

/// Await a task handle, blocking the current thread until the task completes.
/// Returns the task's result. Consumes the handle (frees it).
///
/// Uses `safe_block_on` to handle the case where we're already inside
/// a `block_on` context (e.g., called from `doo_runtime_block_on`).
/// This avoids the "Cannot start a runtime from within a runtime" panic.
///
/// # Safety
/// `handle` must be a valid pointer from `doo_spawn`.
#[no_mangle]
pub extern "C" fn doo_task_await(handle: *mut TaskHandle) -> *mut DooResult {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() {
            return make_err_result("TaskHandle is null");
        }

        let task = unsafe { TaskHandle::from_raw(handle) };

        // Use safe_block_on to avoid nested block_on panic
        safe_block_on(async {
            match task.handle.await {
                Ok(send_result) => send_result.0,
                Err(e) => {
                    if e.is_cancelled() {
                        make_err_result("task cancelled")
                    } else {
                        make_err_result(&format!("Task panicked: {}", e))
                    }
                }
            }
        })
    }));
    result.unwrap_or_else(|_| make_err_result("task_await panicked"))
}

/// Cancel a task and free the handle in one operation.
/// The task will be aborted and the handle memory released.
/// This prevents the handle leak that occurred when cancel and free
/// were separate operations.
///
/// # Safety
/// `handle` must be a valid pointer from `doo_spawn`.
#[no_mangle]
pub extern "C" fn doo_task_cancel(handle: *mut TaskHandle) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() {
            return;
        }
        let task = unsafe { TaskHandle::from_raw(handle) };
        task.handle.abort();
        // Box drops here — handle freed, no leak
    }));
}

/// Check if a task has finished.
/// Returns 1 if finished, 0 if still running.
///
/// # Safety
/// `handle` must be a valid pointer from `doo_spawn`.
#[no_mangle]
pub extern "C" fn doo_task_is_finished(handle: *mut TaskHandle) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() {
            return 1; // Null handle is considered "finished"
        }
        let task = unsafe { &*handle };
        if task.handle.is_finished() {
            1
        } else {
            0
        }
    }));
    result.unwrap_or(1) // On panic, treat as finished
}

/// Free a task handle without awaiting it.
/// If the task is still running, it will be detached (not cancelled).
///
/// # Safety
/// `handle` must be a valid pointer from `doo_spawn`.
#[no_mangle]
pub extern "C" fn doo_task_free(handle: *mut TaskHandle) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() {
            return;
        }
        unsafe {
            let _ = TaskHandle::from_raw(handle);
            // Box drops here, JoinHandle drops → task is detached
        }
    }));
}
