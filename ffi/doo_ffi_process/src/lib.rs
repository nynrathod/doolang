//! # Doo FFI Process
//!
//! Production-grade process management for Doo.
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
//! ## Concurrency
//!
//! - Uses the global Tokio runtime from `doo_ffi_runtime`
//! - `spawn()` runs processes asynchronously
//! - `run()` blocks on the runtime for synchronous result
//! - All process handles stored in a global DashMap registry

mod handle;
mod helpers;
mod registry;

pub use handle::*;
pub use helpers::*;
pub use registry::*;

use std::ffi::CStr;
use std::os::raw::c_char;

use doo_ffi_core::ffi_debug;
use doo_ffi_core::memory::doo_alloc_string;
use doo_ffi_core::result::DooResult;

// ============================================================================
// Runtime initialization — Single Source of Truth for Process Library
// ============================================================================
// On Linux, dynamic symbol interposition can route doo_runtime_init() and
// get_runtime() to DIFFERENT copies of GLOBAL_RUNTIME across .so files.
// Solution: each library owns its runtime via OnceLock::get_or_init().
// Zero cross-library calls → works identically on Windows, Mac, Linux.
// ============================================================================

use std::sync::OnceLock;
static PROCESS_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Get or create the Tokio runtime local to this library.
/// Uses current_thread (single-threaded) to avoid wasting resources —
/// process operations only need I/O polling, not a full multi-threaded pool.
/// This prevents doubling the worker thread count vs doo_ffi_runtime.
pub(crate) fn ensure_runtime() -> &'static tokio::runtime::Runtime {
    PROCESS_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime for process module")
    })
}

// ============================================================================
// FFI Helper: C string conversion (single source of truth)
// ============================================================================

/// Convert C string pointer to Rust String. Null → empty string.
fn c_to_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

/// Allocate a C string from a Rust &str (caller owns).
fn string_to_c(s: &str) -> *const c_char {
    doo_alloc_string(s) as *const c_char
}

/// Create an Ok DooResult with no data.
/// Uses `into_raw()` (libc::malloc) to match `doo_result_free` (libc::free).
fn make_ok_void() -> *mut DooResult {
    DooResult::ok_empty().into_raw()
}

/// Create an Ok DooResult with a string value.
/// Uses `into_raw()` (libc::malloc) to match `doo_result_free` (libc::free).
fn make_ok_str(s: &str) -> *mut DooResult {
    DooResult::ok_string(s).into_raw()
}

/// Create an Err DooResult with a message.
/// Uses `into_raw()` (libc::malloc) to match `doo_result_free` (libc::free).
fn make_err(msg: &str) -> *mut DooResult {
    DooResult::err_str(500, msg).into_raw()
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
#[no_mangle]
pub extern "C" fn doo_process_run(cmd: *const c_char, args_json: *const c_char) -> *mut DooResult {
    let cmd_str = c_to_string(cmd);
    let args_str = c_to_string(args_json);

    ffi_debug!("PROCESS", "run({}, {})", cmd_str, args_str);

    // Parse args from JSON array
    let args = parse_args_json(&args_str);

    // Use the global tokio runtime to run the command
    let rt = ensure_runtime();

    let result: Result<String, String> = rt.block_on(async { run_command(&cmd_str, &args).await });

    match result {
        Ok(ref output) => make_ok_str(output),
        Err(ref e) => make_err(e),
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
#[no_mangle]
pub extern "C" fn doo_process_spawn(
    cmd: *const c_char,
    args_json: *const c_char,
) -> *mut DooResult {
    let cmd_str = c_to_string(cmd);
    let args_str = c_to_string(args_json);

    ffi_debug!("PROCESS", "spawn({}, {})", cmd_str, args_str);

    let args = parse_args_json(&args_str);

    let rt = ensure_runtime();

    let result: Result<String, String> =
        rt.block_on(async { spawn_process(&cmd_str, &args).await });

    match result {
        Ok(ref handle_id) => make_ok_str(handle_id),
        Err(ref e) => make_err(e),
    }
}

// ============================================================================
// ProcessHandle.kill() — Kill a spawned process
// ============================================================================

/// Kill a spawned process by handle ID.
///
/// Doo syntax: `proc.kill()?`
/// FFI: `doo_process_kill(handle_ptr) -> *mut DooResult`
#[no_mangle]
pub extern "C" fn doo_process_kill(handle_ptr: *const c_char) -> *mut DooResult {
    let handle_id = c_to_string(handle_ptr);
    ffi_debug!("PROCESS", "kill({})", handle_id);

    match get_registry().kill_process(&handle_id) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err(&e),
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
    let handle_id = c_to_string(handle_ptr);
    ffi_debug!("PROCESS", "status({})", handle_id);

    match get_registry().get_status(&handle_id) {
        Ok(status_json) => make_ok_str(&status_json),
        Err(e) => make_err(&e),
    }
}

// ============================================================================
// ProcessHandle.waitForOutput() — Wait for completion, return output
// ============================================================================

/// Wait for a spawned process to complete and return its output.
/// Returns JSON: `{ "exit_code": N, "stdout": "...", "stderr": "..." }`
///
/// Doo syntax: `let output = proc.waitForOutput()?`
/// FFI: `doo_process_wait_output(handle_ptr) -> *mut DooResult`
#[no_mangle]
pub extern "C" fn doo_process_wait_output(handle_ptr: *const c_char) -> *mut DooResult {
    let handle_id = c_to_string(handle_ptr);
    ffi_debug!("PROCESS", "waitForOutput({})", handle_id);

    let rt = ensure_runtime();

    let result: Result<String, String> =
        rt.block_on(async { get_registry().wait_for_output(&handle_id).await });

    match result {
        Ok(ref output) => make_ok_str(output),
        Err(ref e) => make_err(e),
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
    let handle_id = c_to_string(handle_ptr);
    if get_registry().is_running(&handle_id) {
        1
    } else {
        0
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
    let cmd_str = c_to_string(cmd);
    let args_str = c_to_string(args_json);

    ffi_debug!("PROCESS", "output({}, {})", cmd_str, args_str);

    let args = parse_args_json(&args_str);

    let rt = ensure_runtime();

    let result: Result<String, String> =
        rt.block_on(async { run_command_stdout(&cmd_str, &args).await });

    match result {
        Ok(ref stdout) => make_ok_str(stdout),
        Err(ref e) => make_err(e),
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
    let handle_id = c_to_string(handle_ptr);
    ffi_debug!("PROCESS", "readStdout({})", handle_id);

    match get_registry().read_stdout(&handle_id) {
        Ok(out) => make_ok_str(&out),
        Err(e) => make_err(&e),
    }
}

/// Read buffered stderr from a spawned process.
///
/// Doo syntax: `let err = proc.readStderr()?`
/// FFI: `doo_process_read_stderr(handle_ptr) -> *mut DooResult`
#[no_mangle]
pub extern "C" fn doo_process_read_stderr(handle_ptr: *const c_char) -> *mut DooResult {
    let handle_id = c_to_string(handle_ptr);
    ffi_debug!("PROCESS", "readStderr({})", handle_id);

    match get_registry().read_stderr(&handle_id) {
        Ok(out) => make_ok_str(&out),
        Err(e) => make_err(&e),
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
    ffi_debug!("PROCESS", "Shutting down all processes");
    get_registry().shutdown_all();
}

/// Get count of active spawned processes.
///
/// FFI: `doo_process_active_count() -> i64`
#[no_mangle]
pub extern "C" fn doo_process_active_count() -> i64 {
    get_registry().count() as i64
}
