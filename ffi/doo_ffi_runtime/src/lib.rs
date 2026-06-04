//! # Doo FFI Runtime
//!
//! Single source of truth for async runtime, task spawning, structured concurrency,
//! and task handle management.
//!
//! ## Architecture
//!
//! - **One global multi-threaded Tokio runtime** — initialized once, used everywhere
//! - **All async execution** (HTTP server, go blocks, scopes) runs on this runtime
//! - **Pure ownership** — no Rc/Arc exposed to user code; handles are opaque pointers
//!
//! ## Safety Guarantees (Production-Grade)
//!
//! - **No panics cross FFI boundary** — all `extern "C"` functions wrapped in `catch_unwind`
//! - **No nested `block_on` panics** — `safe_block_on` uses `block_in_place` when needed
//! - **No thread pool exhaustion** — detached tasks limited by semaphore-like counter
//! - **No TOCTOU races** — `get_or_init` for runtime initialization
//! - **Shutdown-safe** — all spawn operations check `is_shutdown()` before proceeding
//! - **Mutex-protected scopes** — `ScopeHandle` uses `Mutex<JoinSet>` for safe concurrent access
//! - **No memory leaks on error** — scope tracks Ok results and frees them on error path
//!
//! ## Modules
//!
//! - `runtime` — Global runtime init/shutdown/block_on + `safe_block_on` helper
//! - `task` — Spawn, sleep, timeout FFI functions
//! - `scope` — Structured concurrency (JoinSet-based)
//! - `task_handle` — Awaitable/cancellable task handle

pub mod runtime;
pub mod scope;
pub mod task;
pub mod task_handle;

// ============================================================================
// Random module — Single source of truth for all random value generation
// ============================================================================

use std::os::raw::c_char;

/// FFI: `doo_random_base62(len: i64) -> *const c_char`
///
/// Generate a random base62 string (a-z, A-Z, 0-9) of the given length.
/// Declared in `std/Random.doo` as `Random.string(len: Int) -> Str`.
/// Returns a heap-allocated C string (caller-owned via doo_alloc_string).
#[no_mangle]
pub extern "C" fn doo_random_base62(len: i64) -> *const c_char {
    use rand::Rng;

    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let len = len.max(0) as usize;
    let mut rng = rand::thread_rng();
    let result: String = (0..len)
        .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
        .collect();
    doo_ffi_core::memory::doo_alloc_string(&result) as *const c_char
}

// ============================================================================
// String replace-all — replaces ALL occurrences, unlike the first-only compiler builtin
// ============================================================================

/// FFI: `doo_string_replace_all(haystack: *const c_char, needle: *const c_char, replacement: *const c_char) -> *const c_char`
///
/// Replace ALL occurrences of `needle` with `replacement` in `haystack`.
/// Returns a heap-allocated C string (caller-owned via doo_alloc_string).
/// If needle is empty, returns a copy of haystack.
#[no_mangle]
pub extern "C" fn doo_string_replace_all(
    haystack: *const c_char,
    needle: *const c_char,
    replacement: *const c_char,
) -> *const c_char {
    let haystack = unsafe { std::ffi::CStr::from_ptr(haystack) }
        .to_str()
        .unwrap_or("");
    let needle = unsafe { std::ffi::CStr::from_ptr(needle) }
        .to_str()
        .unwrap_or("");
    let replacement = unsafe { std::ffi::CStr::from_ptr(replacement) }
        .to_str()
        .unwrap_or("");

    if needle.is_empty() {
        return doo_ffi_core::memory::doo_alloc_string(haystack) as *const c_char;
    }

    let result = haystack.replace(needle, replacement);
    doo_ffi_core::memory::doo_alloc_string(&result) as *const c_char
}
