//! Task Handle — Awaitable, cancellable handle to a spawned task
//!
//! Wraps `tokio::task::JoinHandle` behind an opaque FFI pointer.
//! Doo code interacts with this through FFI functions only.

use crate::runtime::make_err_result;
use doo_ffi_core::DooResult;
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
/// # Safety
/// `handle` must be a valid pointer from `doo_spawn`.
#[no_mangle]
pub extern "C" fn doo_task_await(handle: *mut TaskHandle) -> *mut DooResult {
    if handle.is_null() {
        return make_err_result("TaskHandle is null");
    }

    let task = unsafe { TaskHandle::from_raw(handle) };
    let rt = crate::runtime::get_runtime();

    // Use the global runtime to block on the task.
    // If we're already inside a block_on context, use spawn_blocking to avoid nesting.
    rt.block_on(async {
        match task.handle.await {
            Ok(send_result) => send_result.0,
            Err(e) => make_err_result(&format!("Task panicked: {}", e)),
        }
    })
}

/// Cancel a task. The task will be aborted.
/// Does NOT free the handle — call `doo_task_free` after.
///
/// # Safety
/// `handle` must be a valid pointer from `doo_spawn`.
#[no_mangle]
pub extern "C" fn doo_task_cancel(handle: *mut TaskHandle) {
    if handle.is_null() {
        return;
    }
    let task = unsafe { &*handle };
    task.handle.abort();
}

/// Check if a task has finished.
/// Returns 1 if finished, 0 if still running.
///
/// # Safety
/// `handle` must be a valid pointer from `doo_spawn`.
#[no_mangle]
pub extern "C" fn doo_task_is_finished(handle: *mut TaskHandle) -> i32 {
    if handle.is_null() {
        return 1; // Null handle is considered "finished"
    }
    let task = unsafe { &*handle };
    if task.handle.is_finished() { 1 } else { 0 }
}

/// Free a task handle without awaiting it.
/// If the task is still running, it will be detached (not cancelled).
///
/// # Safety
/// `handle` must be a valid pointer from `doo_spawn`.
#[no_mangle]
pub extern "C" fn doo_task_free(handle: *mut TaskHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        let _ = TaskHandle::from_raw(handle);
        // Box drops here, JoinHandle drops → task is detached
    }
}
