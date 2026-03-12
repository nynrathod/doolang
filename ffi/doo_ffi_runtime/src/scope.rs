//! Structured Concurrency Scope — JoinSet-based
//!
//! `scope { go { ... }; go { ... }; }` in Doo.
//! All tasks spawned within a scope are joined before the scope exits.
//! If any task errors, all remaining tasks are cancelled.
//!
//! SAFETY:
//! - `Mutex<JoinSet>` prevents data races on concurrent `doo_scope_spawn` calls
//! - `catch_unwind` on all FFI entry points — no panics cross FFI boundary
//! - `safe_block_on` avoids nested `block_on` panics
//! - Ok results tracked and freed on error path — no memory leaks
//! - Shutdown check prevents spawning after runtime shutdown

use crate::runtime::{get_runtime, is_shutdown, make_err_result, make_ok_result, safe_block_on};
use crate::task::DooAsyncFn;
use crate::task_handle::SendResult;
use doo_ffi_core::result::doo_result_free;
use doo_ffi_core::DooResult;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;
use tokio::task::JoinSet;

/// Opaque scope handle wrapping a Tokio JoinSet behind a Mutex.
/// The Mutex is required because multiple `doo_scope_spawn` calls
/// can race from LLVM-compiled code. Without it, `&mut *scope` in
/// the old code was UB on concurrent access.
pub struct ScopeHandle {
    join_set: Mutex<JoinSet<SendResult>>,
}

// SAFETY: ScopeHandle's JoinSet is protected by Mutex.
// SendResult wraps a *mut DooResult that is heap-allocated and exclusively owned.
unsafe impl Send for ScopeHandle {}
unsafe impl Sync for ScopeHandle {}

impl ScopeHandle {
    pub fn new() -> Self {
        Self {
            join_set: Mutex::new(JoinSet::new()),
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
    let result = catch_unwind(|| ScopeHandle::new().into_raw());
    result.unwrap_or(std::ptr::null_mut())
}

/// Spawn a task within a scope on a dedicated blocking thread.
/// The task will be tracked by the scope's JoinSet.
/// Uses `spawn_blocking` so blocking operations inside scope tasks
/// don't starve the async runtime — matching Go's concurrency model.
///
/// SAFETY:
/// - Mutex protects JoinSet from concurrent access
/// - Shutdown check prevents spawning after runtime shutdown
/// - `catch_unwind` on task body prevents panics from unwinding
///
/// # Safety
/// `scope` must be a valid pointer from `doo_scope_create`.
#[no_mangle]
pub extern "C" fn doo_scope_spawn(
    scope: *mut ScopeHandle,
    func: DooAsyncFn,
    env: *mut std::ffi::c_void,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if scope.is_null() || is_shutdown() {
            return;
        }
        let scope = unsafe { &*scope };
        // Lock the JoinSet — safe for concurrent scope_spawn calls
        let mut join_set = match scope.join_set.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(), // Recover from poisoned mutex
        };
        // Use the global runtime handle to ensure we have a Tokio context
        let _guard = get_runtime().enter();
        // SAFETY: env pointer is either null (no captures) or a heap-allocated
        // struct created by codegen. The spawned function takes ownership and frees it.
        let payload = crate::task::SpawnPayload { func, env };
        join_set.spawn_blocking(move || {
            let r = catch_unwind(AssertUnwindSafe(|| payload.call()));
            match r {
                Ok(ptr) => SendResult(ptr),
                Err(_) => SendResult(make_err_result("scope task panicked")),
            }
        });
    }));
}

/// Wait for all tasks in the scope to complete.
/// If any task fails (returns Err), remaining tasks are cancelled,
/// previously collected Ok results are freed (no leaks), and
/// the first error is returned.
/// On success, returns Ok.
/// Consumes the scope handle (frees it).
///
/// Uses `safe_block_on` to avoid nested `block_on` panics.
///
/// # Safety
/// `scope` must be a valid pointer from `doo_scope_create`.
#[no_mangle]
pub extern "C" fn doo_scope_wait(scope: *mut ScopeHandle) -> *mut DooResult {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if scope.is_null() {
            return make_err_result("ScopeHandle is null");
        }

        let scope = unsafe { ScopeHandle::from_raw(scope) };

        // Take the JoinSet out of the Mutex — we own it exclusively now
        let mut join_set = match scope.join_set.into_inner() {
            Ok(js) => js,
            Err(poisoned) => poisoned.into_inner(),
        };

        safe_block_on(async {
            // Track Ok results so we can free them if a later task errors
            let mut ok_results: Vec<*mut DooResult> = Vec::new();

            while let Some(result) = join_set.join_next().await {
                match result {
                    Ok(send_result) => {
                        let doo_result = send_result.0;
                        if !doo_result.is_null() {
                            let res = unsafe { &*doo_result };
                            if res.tag != 0 {
                                // Error — abort remaining tasks
                                join_set.abort_all();
                                // Free all previously collected Ok results to prevent leaks
                                for ok_ptr in ok_results {
                                    if !ok_ptr.is_null() {
                                        doo_result_free(ok_ptr);
                                    }
                                }
                                return doo_result;
                            }
                            // Track this Ok result for cleanup on later error
                            ok_results.push(doo_result);
                        }
                    }
                    Err(e) => {
                        // Task panicked or was cancelled — abort remaining
                        join_set.abort_all();
                        // Free all previously collected Ok results
                        for ok_ptr in ok_results {
                            if !ok_ptr.is_null() {
                                doo_result_free(ok_ptr);
                            }
                        }
                        return make_err_result(&format!("Scope task panicked: {}", e));
                    }
                }
            }

            // All tasks completed successfully — free collected results
            for ok_ptr in ok_results {
                if !ok_ptr.is_null() {
                    doo_result_free(ok_ptr);
                }
            }
            make_ok_result()
        })
    }));
    result.unwrap_or_else(|_| make_err_result("scope_wait panicked"))
}

/// Free a scope handle without waiting.
/// All tasks in the scope will be cancelled (aborted).
///
/// # Safety
/// `scope` must be a valid pointer from `doo_scope_create`.
#[no_mangle]
pub extern "C" fn doo_scope_free(scope: *mut ScopeHandle) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if scope.is_null() {
            return;
        }
        let scope = unsafe { ScopeHandle::from_raw(scope) };
        let mut join_set = match scope.join_set.into_inner() {
            Ok(js) => js,
            Err(poisoned) => poisoned.into_inner(),
        };
        join_set.abort_all();
        // Box drops here
    }));
}
