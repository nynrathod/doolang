//! # Doo FFI Process
//!
//! Production-grade process management for Doo & DooCloud.
//! Single source of truth for all process/command execution.
//!
//! ## Architecture
//!
//! - `process.run(cmd, args)` — Run command to completion, return structured result
//! - `process.spawn(cmd, args)` — Spawn long-running process, return ProcessHandle
//! - `ProcessHandle.kill()` — Kill a spawned process
//! - `ProcessHandle.status()` — Get exit status (or "running")
//! - `ProcessHandle.waitForOutput()` — Block until process finishes, return output
//! - `ProcessHandle.isRunning()` — Check if process is still running
//!
//! ## Security (DooCloud-grade)
//!
//! - Windows shell injection prevention (cmd.exe /c argument sanitization)
//! - Environment variable leak prevention (strips JWT_SECRET, DATABASE_URL, etc.)
//! - Command validation (no path traversal, null bytes, newlines)
//! - Docker argument sanitization (no --privileged, socket mounts, etc.)
//! - Input/output size limits (prevent DoS / OOM)
//! - Concurrent process limit (prevent resource exhaustion)
//! - catch_unwind at every FFI boundary (no panic across FFI = no UB)
//!
//! ## Concurrency
//!
//! - Uses a dedicated single-threaded Tokio runtime (avoids nested block_on)
//! - `spawn()` runs processes asynchronously
//! - `run()` blocks on the runtime for synchronous result
//! - All process handles stored in a global DashMap registry
//! - Processes auto-removed from registry after completion/kill

mod handle;
mod helpers;
mod registry;
pub mod security;

pub use handle::*;
pub use helpers::*;
pub use registry::*;

use std::os::raw::c_char;
use std::panic;

use doo_ffi_core::ffi_debug;
use doo_ffi_core::helpers::{
    c_to_string_lossy, make_err as core_make_err, make_ok_string as make_ok_str, make_ok_void,
    make_panic_err as core_make_panic_err,
};
use doo_ffi_core::memory::doo_alloc_string;
use doo_ffi_core::result::DooResult;

// ============================================================================
// Runtime initialization — Single Source of Truth for Process Library
// ============================================================================
// On Linux, dynamic symbol interposition can route doo_runtime_init() and
// get_runtime() to DIFFERENT copies of GLOBAL_RUNTIME across .so files.
// Solution: each library owns its runtime via OnceLock::get_or_init().
// Zero cross-library calls → works identically on Windows, Mac, Linux.
//
// Uses current_thread (single-threaded) to avoid:
// 1. Wasting OS threads (process ops only need I/O polling)
// 2. Nested block_on panics (separate runtime from doo_ffi_runtime)
// ============================================================================

use std::sync::OnceLock;
static PROCESS_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Get or create the Tokio runtime local to this library.
pub(crate) fn ensure_runtime() -> &'static tokio::runtime::Runtime {
    PROCESS_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|e| {
                doo_ffi_core::ffi_fatal!(
                    "Failed to create Tokio runtime for process module: {}",
                    e
                );
                std::process::exit(1);
            })
    })
}

// ============================================================================
// FFI Helpers — delegated to doo_ffi_core::helpers (single source of truth)
// c_to_string (lossy), make_ok_void, make_ok_str, make_err, make_panic_err
// all imported from doo_ffi_core::helpers above.
// ============================================================================

/// Allocate a C string from a Rust &str (caller owns).
fn string_to_c(s: &str) -> *const c_char {
    doo_alloc_string(s) as *const c_char
}

/// Wrap core make_err with default 500 status code — uses RFC 7807.
fn make_err(msg: &str) -> *mut DooResult {
    doo_ffi_core::helpers::make_err_rfc7807(500, msg)
}

/// Wrap core make_panic_err for process module.
fn make_panic_err(payload: Box<dyn std::any::Any + Send>) -> *mut DooResult {
    core_make_panic_err("Process", payload)
}

// ============================================================================
// process.run(cmd, args) — Run to completion, return JSON result
// ============================================================================

/// Run a command to completion synchronously.
/// Returns JSON: `{ "exit_code": N, "stdout": "...", "stderr": "..." }`
///
/// Doo syntax: `let result = process.run("ls", ["-la"])?`
/// FFI: `doo_process_run(cmd, args_json) -> *mut DooResult`
///
/// `args_json` is a JSON array string: `["arg1", "arg2"]`
///
/// Security: catch_unwind at boundary, input validation, output limits.
#[no_mangle]
pub extern "C" fn doo_process_run(cmd: *const c_char, args_json: *const c_char) -> *mut DooResult {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let cmd_str = c_to_string_lossy(cmd);
        let args_str = c_to_string_lossy(args_json);

        ffi_debug!("PROCESS", "run({}, {})", cmd_str, args_str);

        // Parse args from JSON array (with validation)
        let args = match parse_args_json(&args_str) {
            Ok(a) => a,
            Err(e) => return make_err(&e),
        };

        // Use the local tokio runtime to run the command
        let rt = ensure_runtime();

        let result: Result<String, String> =
            rt.block_on(async { run_command(&cmd_str, &args).await });

        match result {
            Ok(ref output) => make_ok_str(output),
            Err(ref e) => make_err(e),
        }
    }));

    match result {
        Ok(ptr) => ptr,
        Err(payload) => make_panic_err(payload),
    }
}

// ============================================================================
// process.spawn(cmd, args) — Spawn background process, return handle ID
// ============================================================================

/// Spawn a long-running process in the background.
/// Returns the process handle ID (string) for subsequent operations.
///
/// Doo syntax: `let proc = process.spawn("docker", ["run", ...])?`
/// FFI: `doo_process_spawn(cmd, args_json) -> *mut DooResult` (ok = handle_id string)
///
/// Security: enforces concurrent process limit, validates command & args.
#[no_mangle]
pub extern "C" fn doo_process_spawn(
    cmd: *const c_char,
    args_json: *const c_char,
) -> *mut DooResult {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let cmd_str = c_to_string_lossy(cmd);
        let args_str = c_to_string_lossy(args_json);

        ffi_debug!("PROCESS", "spawn({}, {})", cmd_str, args_str);

        let args = match parse_args_json(&args_str) {
            Ok(a) => a,
            Err(e) => return make_err(&e),
        };

        let rt = ensure_runtime();

        let result: Result<String, String> =
            rt.block_on(async { spawn_process(&cmd_str, &args).await });

        match result {
            Ok(ref handle_id) => make_ok_str(handle_id),
            Err(ref e) => make_err(e),
        }
    }));

    match result {
        Ok(ptr) => ptr,
        Err(payload) => make_panic_err(payload),
    }
}

// ============================================================================
// ProcessHandle.kill() — Kill a spawned process
// ============================================================================

/// Kill a spawned process by handle ID.
/// Also removes the process from the registry to prevent leaks.
///
/// Doo syntax: `proc.kill()?`
/// FFI: `doo_process_kill(handle_ptr) -> *mut DooResult`
#[no_mangle]
pub extern "C" fn doo_process_kill(handle_ptr: *const c_char) -> *mut DooResult {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let handle_id = c_to_string_lossy(handle_ptr);
        ffi_debug!("PROCESS", "kill({})", handle_id);

        match get_registry().kill_process(&handle_id) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err(&e),
        }
    }));

    match result {
        Ok(ptr) => ptr,
        Err(payload) => make_panic_err(payload),
    }
}

// ============================================================================
// ProcessHandle.status() — Get process status
// ============================================================================

/// Get the status of a spawned process.
/// Returns JSON: `{ "status": "running"|"exited", "exit_code": N|null }`
///
/// Doo syntax: `let status = proc.status()?`
/// FFI: `doo_process_status(handle_ptr) -> *mut DooResult`
#[no_mangle]
pub extern "C" fn doo_process_status(handle_ptr: *const c_char) -> *mut DooResult {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let handle_id = c_to_string_lossy(handle_ptr);
        ffi_debug!("PROCESS", "status({})", handle_id);

        match get_registry().get_status(&handle_id) {
            Ok(status_json) => make_ok_str(&status_json),
            Err(e) => make_err(&e),
        }
    }));

    match result {
        Ok(ptr) => ptr,
        Err(payload) => make_panic_err(payload),
    }
}

// ============================================================================
// ProcessHandle.waitForOutput() — Wait for completion, return output
// ============================================================================

/// Wait for a spawned process to complete and return its output.
/// Returns JSON: `{ "exit_code": N, "stdout": "...", "stderr": "..." }`
/// Automatically removes the process from the registry after completion.
///
/// Doo syntax: `let output = proc.waitForOutput()?`
/// FFI: `doo_process_wait_output(handle_ptr) -> *mut DooResult`
#[no_mangle]
pub extern "C" fn doo_process_wait_output(handle_ptr: *const c_char) -> *mut DooResult {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let handle_id = c_to_string_lossy(handle_ptr);
        ffi_debug!("PROCESS", "waitForOutput({})", handle_id);

        let rt = ensure_runtime();

        let result: Result<String, String> =
            rt.block_on(async { get_registry().wait_for_output(&handle_id).await });

        match result {
            Ok(ref output) => make_ok_str(output),
            Err(ref e) => make_err(e),
        }
    }));

    match result {
        Ok(ptr) => ptr,
        Err(payload) => make_panic_err(payload),
    }
}

// ============================================================================
// ProcessHandle.isRunning() — Check if process is still alive
// ============================================================================

/// Check if a spawned process is still running.
/// Returns 1 if running, 0 if exited/not found.
///
/// Doo syntax: `if proc.isRunning() { ... }`
/// FFI: `doo_process_is_running(handle_ptr) -> i64`
#[no_mangle]
pub extern "C" fn doo_process_is_running(handle_ptr: *const c_char) -> i64 {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let handle_id = c_to_string_lossy(handle_ptr);
        if get_registry().is_running(&handle_id) {
            1i64
        } else {
            0i64
        }
    }));

    match result {
        Ok(v) => v,
        Err(_) => 0, // On panic, report not running (safe default)
    }
}

// ============================================================================
// process.output(cmd, args) — Run and return just stdout string
// ============================================================================

/// Run a command and return just the stdout as a string.
/// Convenience wrapper over run() for simple command output capture.
///
/// Doo syntax: `let out = process.output("echo", ["hello"])?`
/// FFI: `doo_process_output(cmd, args_json) -> *mut DooResult`
#[no_mangle]
pub extern "C" fn doo_process_output(
    cmd: *const c_char,
    args_json: *const c_char,
) -> *mut DooResult {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let cmd_str = c_to_string_lossy(cmd);
        let args_str = c_to_string_lossy(args_json);

        ffi_debug!("PROCESS", "output({}, {})", cmd_str, args_str);

        let args = match parse_args_json(&args_str) {
            Ok(a) => a,
            Err(e) => return make_err(&e),
        };

        let rt = ensure_runtime();

        let result: Result<String, String> =
            rt.block_on(async { run_command_stdout(&cmd_str, &args).await });

        match result {
            Ok(ref stdout) => make_ok_str(stdout),
            Err(ref e) => make_err(e),
        }
    }));

    match result {
        Ok(ptr) => ptr,
        Err(payload) => make_panic_err(payload),
    }
}

// ============================================================================
// ProcessHandle.readStdout() — Read current buffered stdout
// ============================================================================

/// Read buffered stdout from a spawned process.
///
/// Doo syntax: `let out = proc.readStdout()?`
/// FFI: `doo_process_read_stdout(handle_ptr) -> *mut DooResult`
#[no_mangle]
pub extern "C" fn doo_process_read_stdout(handle_ptr: *const c_char) -> *mut DooResult {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let handle_id = c_to_string_lossy(handle_ptr);
        ffi_debug!("PROCESS", "readStdout({})", handle_id);

        match get_registry().read_stdout(&handle_id) {
            Ok(out) => make_ok_str(&out),
            Err(e) => make_err(&e),
        }
    }));

    match result {
        Ok(ptr) => ptr,
        Err(payload) => make_panic_err(payload),
    }
}

/// Read buffered stderr from a spawned process.
///
/// Doo syntax: `let err = proc.readStderr()?`
/// FFI: `doo_process_read_stderr(handle_ptr) -> *mut DooResult`
#[no_mangle]
pub extern "C" fn doo_process_read_stderr(handle_ptr: *const c_char) -> *mut DooResult {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let handle_id = c_to_string_lossy(handle_ptr);
        ffi_debug!("PROCESS", "readStderr({})", handle_id);

        match get_registry().read_stderr(&handle_id) {
            Ok(out) => make_ok_str(&out),
            Err(e) => make_err(&e),
        }
    }));

    match result {
        Ok(ptr) => ptr,
        Err(payload) => make_panic_err(payload),
    }
}

// ============================================================================
// Cleanup
// ============================================================================

/// Kill all spawned processes (graceful shutdown).
///
/// FFI: `doo_process_shutdown()`
#[no_mangle]
pub extern "C" fn doo_process_shutdown() {
    let _ = panic::catch_unwind(|| {
        ffi_debug!("PROCESS", "Shutting down all processes");
        get_registry().shutdown_all();
    });
}

/// Get count of active spawned processes.
///
/// FFI: `doo_process_active_count() -> i64`
#[no_mangle]
pub extern "C" fn doo_process_active_count() -> i64 {
    let result = panic::catch_unwind(|| get_registry().count() as i64);
    result.unwrap_or(0)
}
