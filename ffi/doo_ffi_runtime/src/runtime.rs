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
use std::time::Instant;
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

/// The ONE global runtime. Initialized exactly once via `doo_runtime_init`
/// or lazily via `get_or_init_runtime`.
static GLOBAL_RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Program start instant — captured at `doo_runtime_init()` for accurate boot time.
/// Used by the HTTP server banner to show total program startup time.
static PROGRAM_START: OnceLock<Instant> = OnceLock::new();

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
        // Worker thread stack size: 8 MB (default on Windows is ~1 MB).
        // HTTP handlers call deep FFI chains (HandleGenerate → EnsureRepo →
        // doo_git_init → libgit2 internals). The combined stack usage of
        // Doo-generated code + Rust FFI + libgit2 can exceed 1 MB.
        // Configurable via DOO_THREAD_STACK_SIZE (bytes).
        .thread_stack_size(env_usize("DOO_THREAD_STACK_SIZE", 8 * 1024 * 1024))
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

/// Get the program start instant (captured at `doo_runtime_init`).
/// Used by the HTTP server to measure total boot time.
pub fn program_start() -> Option<&'static Instant> {
    PROGRAM_START.get()
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
        // Install panic hook that prints to stderr before abort.
        // With panic=abort, the default hook may not flush output before the process dies.
        // This ensures panic messages are ALWAYS visible for debugging.
        std::panic::set_hook(Box::new(|info| {
            use std::io::Write;
            let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = info.payload().downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };
            let location = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_default();
            let _ = writeln!(std::io::stderr(), "\n[Doo PANIC] {} at {}", msg, location);
            let _ = std::io::stderr().flush();
        }));

        // Install Windows crash handler to catch access violations / segfaults.
        // Without this, segfaults silently kill the process with no output.
        #[cfg(target_os = "windows")]
        {
            use std::io::Write;
            unsafe {
                #[repr(C)]
                struct ExceptionRecord {
                    code: u32,
                    flags: u32,
                    record: *mut ExceptionRecord,
                    address: *mut std::ffi::c_void,
                    num_params: u32,
                    info: [usize; 15],
                }
                #[repr(C)]
                struct ExceptionPointers {
                    record: *mut ExceptionRecord,
                    context: *mut std::ffi::c_void,
                }
                #[repr(C)]
                #[allow(dead_code)]
                struct MemoryBasicInformation {
                    base_address: *mut std::ffi::c_void,
                    allocation_base: *mut std::ffi::c_void,
                    allocation_protect: u32,
                    partition_id: u16,
                    region_size: usize,
                    state: u32,
                    protect: u32,
                    type_: u32,
                }
                type ExceptionFilter = unsafe extern "system" fn(*mut ExceptionPointers) -> i32;
                extern "system" {
                    fn SetUnhandledExceptionFilter(
                        filter: ExceptionFilter,
                    ) -> *mut std::ffi::c_void;
                    fn VirtualQuery(
                        address: *const std::ffi::c_void,
                        buffer: *mut MemoryBasicInformation,
                        length: usize,
                    ) -> usize;
                    fn RtlCaptureStackBackTrace(
                        frames_to_skip: u32,
                        frames_to_capture: u32,
                        back_trace: *mut *mut std::ffi::c_void,
                        back_trace_hash: *mut u32,
                    ) -> u16;
                    fn GetModuleFileNameA(
                        module: *mut std::ffi::c_void,
                        filename: *mut u8,
                        size: u32,
                    ) -> u32;
                }
                unsafe extern "system" fn crash_handler(info: *mut ExceptionPointers) -> i32 {
                    let rec = &*(*info).record;
                    let code = rec.code;
                    let addr = rec.address;
                    let access_type = if rec.num_params >= 2 {
                        match rec.info[0] {
                            0 => "READ",
                            1 => "WRITE",
                            8 => "DEP",
                            _ => "UNKNOWN",
                        }
                    } else {
                        "N/A"
                    };
                    let target_addr = if rec.num_params >= 2 { rec.info[1] } else { 0 };
                    let _ = writeln!(
                        std::io::stderr(),
                        "\n[Doo CRASH] Exception 0x{:08X} at RIP {:?} — {} access to 0x{:X}",
                        code,
                        addr,
                        access_type,
                        target_addr
                    );

                    // Query memory info for the crash address
                    let mut mbi: MemoryBasicInformation = std::mem::zeroed();
                    let mbi_size = std::mem::size_of::<MemoryBasicInformation>();
                    let result =
                        VirtualQuery(target_addr as *const std::ffi::c_void, &mut mbi, mbi_size);
                    if result > 0 {
                        let state_str = match mbi.state {
                            0x1000 => "COMMITTED",
                            0x2000 => "RESERVED",
                            0x10000 => "FREE",
                            _ => "UNKNOWN",
                        };
                        let protect_str = match mbi.protect {
                            0x01 => "NOACCESS",
                            0x02 => "READONLY",
                            0x04 => "READWRITE",
                            0x08 => "WRITECOPY",
                            0x10 => "EXECUTE",
                            0x20 => "EXECUTE_READ",
                            0x40 => "EXECUTE_READWRITE",
                            0x80 => "EXECUTE_WRITECOPY",
                            _ => "OTHER",
                        };
                        let type_str = match mbi.type_ {
                            0x20000 => "PRIVATE",
                            0x40000 => "MAPPED",
                            0x1000000 => "IMAGE",
                            _ => "UNKNOWN",
                        };
                        let _ = writeln!(
                            std::io::stderr(),
                            "[Doo CRASH] Memory at 0x{:X}: state={}, protect={} (0x{:X}), type={}, base=0x{:X}, size=0x{:X}",
                            target_addr, state_str, protect_str, mbi.protect, type_str,
                            mbi.base_address as usize, mbi.region_size
                        );
                    }

                    // Get module name for the crash address
                    let mut mod_name = [0u8; 260];
                    let len = GetModuleFileNameA(
                        mbi.allocation_base as *mut std::ffi::c_void,
                        mod_name.as_mut_ptr(),
                        260,
                    );
                    if len > 0 {
                        let name = std::str::from_utf8_unchecked(&mod_name[..len as usize]);
                        let _ = writeln!(std::io::stderr(), "[Doo CRASH] Module: {}", name);
                    }

                    // Capture stack backtrace (up to 32 frames)
                    let mut frames: [*mut std::ffi::c_void; 32] = [std::ptr::null_mut(); 32];
                    let count =
                        RtlCaptureStackBackTrace(0, 32, frames.as_mut_ptr(), std::ptr::null_mut());
                    if count > 0 {
                        let _ = writeln!(
                            std::io::stderr(),
                            "[Doo CRASH] Stack trace ({} frames):",
                            count
                        );
                        for i in 0..count as usize {
                            let _ = writeln!(
                                std::io::stderr(),
                                "  [{:2}] 0x{:X}",
                                i,
                                frames[i] as usize
                            );
                        }
                    }

                    let _ = std::io::stderr().flush();
                    0 // EXCEPTION_CONTINUE_SEARCH — let the OS terminate the process
                }
                SetUnhandledExceptionFilter(crash_handler);
            }
        }

        // Capture program start time — the runtime init is the first FFI call
        // in main(), so this gives accurate boot time measurement.
        let _ = PROGRAM_START.set(Instant::now());

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
