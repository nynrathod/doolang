//! Global Tokio Runtime — Single Source of Truth
//!
//! One multi-threaded runtime for ALL async execution in Doo:
//! HTTP server, go blocks, scopes, sleep, timeout — everything.

use doo_ffi_core::DooResult;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use tokio::runtime::Runtime;

/// The ONE global runtime. Initialized exactly once via `doo_runtime_init`.
static GLOBAL_RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Shutdown flag to prevent new async work after shutdown.
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Get a reference to the global runtime.
/// Panics if `doo_runtime_init` was not called first.
pub fn get_runtime() -> &'static Runtime {
    GLOBAL_RUNTIME
        .get()
        .expect("doo_runtime_init() must be called before using async features")
}

/// Check if the runtime has been initialized.
pub fn is_runtime_initialized() -> bool {
    GLOBAL_RUNTIME.get().is_some()
}

// ============================================================================
// FFI Functions
// ============================================================================

/// Initialize the global multi-threaded Tokio runtime.
/// Must be called exactly once at program startup (before any async work).
/// Returns 0 on success, 1 if already initialized, -1 on error.
#[no_mangle]
pub extern "C" fn doo_runtime_init() -> i32 {
    if GLOBAL_RUNTIME.get().is_some() {
        return 1; // Already initialized
    }

    match Runtime::new() {
        Ok(rt) => {
            let _ = GLOBAL_RUNTIME.set(rt);
            0 // Success
        }
        Err(_) => -1, // Failed to create runtime
    }
}

/// Block the calling thread on an async function pointer.
/// This is the main entry point for running async code from synchronous `main()`.
///
/// `func` is an `extern "C" fn() -> *mut DooResult` that will be executed
/// on the global runtime via `block_on`.
#[no_mangle]
pub extern "C" fn doo_runtime_block_on(
    func: extern "C" fn() -> *mut DooResult,
) -> *mut DooResult {
    let rt = get_runtime();
    rt.block_on(async { func() })
}

/// Gracefully shut down the global runtime.
/// After this call, `get_runtime()` will still return the runtime reference
/// (OnceLock can't be emptied), but the shutdown flag is set to signal
/// no new work should be submitted. Existing tasks will complete.
/// Returns 0 on success, 1 if not initialized.
#[no_mangle]
pub extern "C" fn doo_runtime_shutdown() -> i32 {
    if GLOBAL_RUNTIME.get().is_none() {
        return 1; // Not initialized
    }
    SHUTDOWN.store(true, Ordering::SeqCst);
    // The runtime itself will shut down when the process exits.
    // Tokio's Runtime::drop waits for spawned tasks to complete.
    0
}

/// Check if the runtime has been shut down.
pub fn is_shutdown() -> bool {
    SHUTDOWN.load(Ordering::SeqCst)
}

/// Check if we are currently inside the runtime context (on a Tokio worker thread).
/// Returns 1 if yes, 0 if no.
#[no_mangle]
pub extern "C" fn doo_runtime_is_async_context() -> i32 {
    if tokio::runtime::Handle::try_current().is_ok() {
        1
    } else {
        0
    }
}

// ============================================================================
// Helper: Create DooResult — matches { i64 tag, *mut c_void data } layout
// ============================================================================

/// Create a success DooResult with no value.
pub fn make_ok_result() -> *mut DooResult {
    DooResult::ok_empty().into_raw()
}

/// Create a success DooResult with a string value.
pub fn make_ok_string_result(s: &str) -> *mut DooResult {
    DooResult::ok_string(s).into_raw()
}

/// Create an error DooResult with a message.
pub fn make_err_result(msg: &str) -> *mut DooResult {
    DooResult::err_str(0, msg).into_raw()
}
