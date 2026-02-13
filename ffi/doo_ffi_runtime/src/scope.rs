//! Structured Concurrency Scope — JoinSet-based
//!
//! `scope { go { ... }; go { ... }; }` in Doo.
//! All tasks spawned within a scope are joined before the scope exits.
//! If any task errors, all remaining tasks are cancelled.

use crate::runtime::{get_runtime, make_err_result, make_ok_result};
use crate::task::DooAsyncFn;
use crate::task_handle::SendResult;
use doo_ffi_core::DooResult;
use tokio::task::JoinSet;

/// Opaque scope handle wrapping a Tokio JoinSet.
pub struct ScopeHandle {
    join_set: JoinSet<SendResult>,
}

// SAFETY: ScopeHandle is only accessed through serialized FFI calls.
unsafe impl Send for ScopeHandle {}
unsafe impl Sync for ScopeHandle {}

impl ScopeHandle {
    pub fn new() -> Self {
        Self {
            join_set: JoinSet::new(),
        }
    }

    pub fn into_raw(self) -> *mut ScopeHandle {
        Box::into_raw(Box::new(self))
    }

    /// # Safety
    /// Pointer must have been created by `into_raw`.
    pub unsafe fn from_raw(ptr: *mut ScopeHandle) -> Box<ScopeHandle> {
        Box::from_raw(ptr)
    }
}

// ============================================================================
// FFI Functions
// ============================================================================

/// Create a new scope.
/// Returns an opaque pointer to a ScopeHandle.
#[no_mangle]
pub extern "C" fn doo_scope_create() -> *mut ScopeHandle {
    ScopeHandle::new().into_raw()
}

/// Spawn a task within a scope on a dedicated blocking thread.
/// The task will be tracked by the scope's JoinSet.
/// Uses `spawn_blocking` so blocking operations inside scope tasks
/// don't starve the async runtime — matching Go's concurrency model.
///
/// # Safety
/// `scope` must be a valid pointer from `doo_scope_create`.
#[no_mangle]
pub extern "C" fn doo_scope_spawn(scope: *mut ScopeHandle, func: DooAsyncFn) {
    if scope.is_null() {
        return;
    }
    let scope = unsafe { &mut *scope };
    // Use the global runtime handle to ensure we have a Tokio context
    let _guard = get_runtime().enter();
    scope.join_set.spawn_blocking(move || SendResult(func()));
}

/// Wait for all tasks in the scope to complete.
/// If any task fails (returns Err), remaining tasks are cancelled and
/// the first error is returned.
/// On success, returns Ok.
/// Consumes the scope handle (frees it).
///
/// # Safety
/// `scope` must be a valid pointer from `doo_scope_create`.
#[no_mangle]
pub extern "C" fn doo_scope_wait(scope: *mut ScopeHandle) -> *mut DooResult {
    if scope.is_null() {
        return make_err_result("ScopeHandle is null");
    }

    let mut scope = unsafe { ScopeHandle::from_raw(scope) };
    let rt = get_runtime();

    rt.block_on(async {
        while let Some(result) = scope.join_set.join_next().await {
            match result {
                Ok(send_result) => {
                    let doo_result = send_result.0;
                    // Check if the task returned an error
                    if !doo_result.is_null() {
                        let res = unsafe { &*doo_result };
                        if res.tag != 0 {
                            // Error — abort remaining tasks and return this error
                            scope.join_set.abort_all();
                            return doo_result;
                        }
                    }
                }
                Err(e) => {
                    // Task panicked — abort remaining and return error
                    scope.join_set.abort_all();
                    return make_err_result(&format!("Scope task panicked: {}", e));
                }
            }
        }
        // All tasks completed successfully
        make_ok_result()
    })
}

/// Free a scope handle without waiting.
/// All tasks in the scope will be cancelled (aborted).
///
/// # Safety
/// `scope` must be a valid pointer from `doo_scope_create`.
#[no_mangle]
pub extern "C" fn doo_scope_free(scope: *mut ScopeHandle) {
    if scope.is_null() {
        return;
    }
    let mut scope = unsafe { ScopeHandle::from_raw(scope) };
    scope.join_set.abort_all();
    // Box drops here
}
