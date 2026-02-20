//! Global Tokio Runtime — Single Source of Truth
//!
//! One multi-threaded runtime for ALL async execution in Doo:
//! HTTP server, go blocks, scopes, sleep, timeout — everything.
//!
//! ## Environment Variable Configuration
//!
//! All tuning parameters can be overridden via environment variables
//! for container-based deployments (Docker, GKE, Fly.io, Railway, etc.):
//!
//! | Variable              | Default         | Description                      |
//! |-----------------------|-----------------|----------------------------------|
//! | `DOO_WORKERS`         | num_cpus        | Tokio worker threads             |
//! | `DOO_MAX_BLOCKING`    | num_cpus * 4    | Max blocking thread pool size    |
//! | `DOO_MAX_DETACHED`    | num_cpus * 4    | Max concurrent detached tasks    |
//! | `DOO_SHUTDOWN_TIMEOUT`| 30000 (30s)     | Graceful shutdown timeout (ms)   |
//!
//! ## Safety Guarantees
//!
//! - `get_or_init` eliminates TOCTOU race on initialization
//! - `is_shutdown()` checked before every spawn/task operation
//! - All FFI entry points wrapped in `catch_unwind` — no panics cross FFI boundary
//! - `block_in_place` used for nested block_on safety
//! - `CancellationToken` for cooperative shutdown signaling

use doo_ffi_core::helpers::{make_err, make_ok_string, make_ok_void};
use doo_ffi_core::DooResult;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::OnceLock;
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

/// The ONE global runtime. Initialized exactly once via `doo_runtime_init`
/// or lazily via `get_or_init_runtime`.
static GLOBAL_RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Shutdown flag to prevent new async work after shutdown.
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Track number of active detached tasks for backpressure.
static DETACHED_TASK_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Maximum concurrent detached tasks — prevents thread pool exhaustion.
/// Set at runtime init from DOO_MAX_DETACHED env var or num_cpus * 4.
static MAX_DETACHED_TASKS: AtomicUsize = AtomicUsize::new(512);

/// Shutdown timeout in milliseconds. Default 30s (standard GKE SIGTERM grace period).
static SHUTDOWN_TIMEOUT_MS: AtomicUsize = AtomicUsize::new(30_000);

/// Global cancellation token for cooperative shutdown.
/// HTTP server, scopes, and long-running tasks can listen on this.
static CANCEL_TOKEN: OnceLock<CancellationToken> = OnceLock::new();

/// Read an env var as usize, with fallback.
fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

/// Build the runtime with production-grade tuning.
/// All parameters configurable via environment variables.
fn build_runtime() -> Runtime {
    let cpus = num_cpus::get().max(1);
    let workers = env_usize("DOO_WORKERS", cpus);
    let max_blocking = env_usize("DOO_MAX_BLOCKING", cpus * 4);

    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .max_blocking_threads(max_blocking)
        .thread_name("doo-worker")
        // Tuned for high-throughput benchmarks:
        // - event_interval=31: Check I/O more frequently (default 61), reduces latency
        // - global_queue_interval=7: Check global queue more often, better work distribution
        // Configurable via DOO_EVENT_INTERVAL / DOO_GLOBAL_QUEUE_INTERVAL
        .event_interval(env_usize("DOO_EVENT_INTERVAL", 31) as u32)
        .global_queue_interval(env_usize("DOO_GLOBAL_QUEUE_INTERVAL", 7) as u32)
        .enable_all()
        .build()
        .unwrap_or_else(|e| {
            doo_ffi_core::ffi_fatal!("Failed to create Tokio runtime: {}", e);
            std::process::exit(1);
        })
}

/// Get a reference to the global runtime.
/// Uses `get_or_init` for lazy initialization — never panics,
/// eliminates TOCTOU race, safe to call from any context.
pub fn get_runtime() -> &'static Runtime {
    GLOBAL_RUNTIME.get_or_init(build_runtime)
}

/// Check if the runtime has been initialized.
pub fn is_runtime_initialized() -> bool {
    GLOBAL_RUNTIME.get().is_some()
}

/// Get or init the global cancellation token.
pub fn get_cancel_token() -> &'static CancellationToken {
    CANCEL_TOKEN.get_or_init(CancellationToken::new)
}

// ============================================================================
// FFI Functions
// ============================================================================

/// Initialize the global multi-threaded Tokio runtime.
/// Reads configuration from environment variables.
/// Idempotent: returns 1 if already initialized.
/// Returns 0 on success, 1 if already initialized, -1 on panic.
#[no_mangle]
pub extern "C" fn doo_runtime_init() -> i32 {
    let result = catch_unwind(|| {
        let was_new = GLOBAL_RUNTIME.get().is_none();
        let _ = get_runtime();

        // Initialize settings from env vars
        let cpus = num_cpus::get().max(1);
        MAX_DETACHED_TASKS.store(env_usize("DOO_MAX_DETACHED", cpus * 4), Ordering::Relaxed);
        SHUTDOWN_TIMEOUT_MS.store(env_usize("DOO_SHUTDOWN_TIMEOUT", 30_000), Ordering::Relaxed);

        // Initialize the cancellation token
        let _ = get_cancel_token();

        if was_new && GLOBAL_RUNTIME.get().is_some() {
            0
        } else {
            1
        }
    });
    result.unwrap_or(-1)
}

/// Block the calling thread on an async function pointer.
/// This is the main entry point for running async code from synchronous `main()`.
///
/// `func` is an `extern "C" fn() -> *mut DooResult` that will be executed
/// on the global runtime via `block_on`.
///
/// SAFETY: Uses `catch_unwind` to prevent panics from crossing FFI boundary.
/// Uses `block_in_place` when already inside a Tokio context.
#[no_mangle]
pub extern "C" fn doo_runtime_block_on(func: extern "C" fn() -> *mut DooResult) -> *mut DooResult {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let rt = get_runtime();
        rt.block_on(async { func() })
    }));
    match result {
        Ok(ptr) => ptr,
        Err(_) => make_err(500, "runtime block_on panicked"),
    }
}

/// Gracefully shut down the global runtime.
/// 1. Sets the shutdown flag to reject new work
/// 2. Cancels the global CancellationToken (signals HTTP server, scopes, etc.)
/// 3. Existing in-flight tasks complete within the shutdown timeout
///
/// Returns 0 on success, 1 if not initialized, -1 on panic.
#[no_mangle]
pub extern "C" fn doo_runtime_shutdown() -> i32 {
    let result = catch_unwind(|| {
        if GLOBAL_RUNTIME.get().is_none() {
            return 1;
        }
        SHUTDOWN.store(true, Ordering::SeqCst);
        // Cancel the global token — HTTP server and other listeners will see this
        if let Some(token) = CANCEL_TOKEN.get() {
            token.cancel();
        }
        0
    });
    result.unwrap_or(-1)
}

/// Check if the runtime has been shut down.
pub fn is_shutdown() -> bool {
    SHUTDOWN.load(Ordering::SeqCst)
}

/// Get the shutdown timeout in milliseconds.
pub fn get_shutdown_timeout_ms() -> u64 {
    SHUTDOWN_TIMEOUT_MS.load(Ordering::Relaxed) as u64
}

/// Try to acquire a detached task slot. Returns true if allowed.
pub fn try_acquire_detached_slot() -> bool {
    let max = MAX_DETACHED_TASKS.load(Ordering::Relaxed);
    loop {
        let current = DETACHED_TASK_COUNT.load(Ordering::Relaxed);
        if current >= max {
            return false;
        }
        match DETACHED_TASK_COUNT.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Relaxed,
        ) {
            Ok(_) => return true,
            Err(_) => continue,
        }
    }
}

/// Release a detached task slot.
pub fn release_detached_slot() {
    DETACHED_TASK_COUNT.fetch_sub(1, Ordering::Release);
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
// Helper: DooResult — delegated to doo_ffi_core::helpers (single source of truth)
// make_ok_void, make_ok_string, make_err imported above.
// ============================================================================

/// Alias for backward compatibility:
pub fn make_ok_result() -> *mut DooResult {
    make_ok_void()
}

pub fn make_ok_string_result(s: &str) -> *mut DooResult {
    make_ok_string(s)
}

pub fn make_err_result(msg: &str) -> *mut DooResult {
    make_err(0, msg)
}

/// Helper: run an async block on the runtime, handling nested contexts
/// via `block_in_place`. This is the ONLY correct way to call `block_on`
/// from code that might already be inside a Tokio worker thread.
pub fn safe_block_on<F, T>(f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    let rt = get_runtime();
    if tokio::runtime::Handle::try_current().is_ok() {
        // Already inside Tokio — use block_in_place to avoid nested block_on panic
        tokio::task::block_in_place(|| rt.block_on(f))
    } else {
        rt.block_on(f)
    }
}
