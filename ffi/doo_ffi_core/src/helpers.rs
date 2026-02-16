//! Centralized FFI Helpers — Single Source of Truth
//!
//! All cross-crate utility functions live here. No other crate should
//! define its own `c_to_string`, `string_to_c`, `make_ok_*`, `make_err`,
//! `safe_ffi`, or `make_panic_err`.
//!
//! ## Ownership Model
//!
//! - `c_to_string`: Borrows the C string (does NOT take ownership)
//! - `string_to_c`: Allocates via `doo_alloc_string` (caller owns result)
//! - `make_ok_*` / `make_err`: Returns `*mut DooResult` via `into_raw` (caller owns)
//! - `safe_ffi`: Wraps FFI functions in `catch_unwind` for panic safety

use crate::memory::doo_alloc_string;
use crate::result::DooResult;
use std::ffi::CStr;
use std::os::raw::c_char;

// ============================================================================
// String Conversion — Single Source of Truth
// ============================================================================

/// Convert a C string pointer to a Rust `String`.
///
/// - Null pointer → `Err("Null pointer")`
/// - Invalid UTF-8 → `Err("Invalid UTF-8: ...")`
/// - Valid → `Ok(String)`
///
/// This is the ONE canonical implementation. All FFI crates must use this.
#[inline]
pub fn c_to_string(ptr: *const c_char) -> Result<String, String> {
    if ptr.is_null() {
        return Err("Null pointer".to_string());
    }
    unsafe {
        CStr::from_ptr(ptr)
            .to_str()
            .map(|s| s.to_string())
            .map_err(|e| format!("Invalid UTF-8: {}", e))
    }
}

/// Convert a C string pointer to a Rust `String`, lossy.
///
/// - Null pointer → empty string
/// - Invalid UTF-8 → replacement characters (U+FFFD)
///
/// Use this when you want a best-effort conversion without error handling.
#[inline]
pub fn c_to_string_lossy(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

/// Allocate a C string from a Rust `&str`.
///
/// Uses `doo_alloc_string` (libc::malloc) — NOT `CString::into_raw()`.
/// Caller owns the returned pointer and must free via `doo_free`.
///
/// CRITICAL: Never use `CString::into_raw()` for FFI strings. It uses
/// Rust's allocator, causing heap corruption when freed with `libc::free`.
#[inline]
pub fn string_to_c(s: &str) -> *const c_char {
    doo_alloc_string(s) as *const c_char
}

// ============================================================================
// DooResult Constructors — Single Source of Truth
// ============================================================================

/// Create an Ok `DooResult` with no data.
#[inline]
pub fn make_ok_void() -> *mut DooResult {
    DooResult::ok_empty().into_raw()
}

/// Create an Ok `DooResult` with a string value.
/// The string is allocated via `doo_alloc_string` (libc::malloc).
#[inline]
pub fn make_ok_string(s: &str) -> *mut DooResult {
    DooResult::ok_string(s).into_raw()
}

/// Create an Ok `DooResult` with an integer value (as pointer-sized).
#[inline]
pub fn make_ok_int(n: i64) -> *mut DooResult {
    DooResult::ok(n as *mut std::ffi::c_void, 0).into_raw()
}

/// Create an Ok `DooResult` with a boolean value (0 or 1).
#[inline]
pub fn make_ok_bool(b: bool) -> *mut DooResult {
    DooResult::ok((b as i64) as *mut std::ffi::c_void, 0).into_raw()
}

/// Create an Err `DooResult` with a message.
/// Uses `DooResult::err_str` which wraps the string in `{ *char }`.
#[inline]
pub fn make_err(code: u16, message: &str) -> *mut DooResult {
    DooResult::err_str(code, message).into_raw()
}

/// Create an Err `DooResult` with an RFC 7807 JSON body.
/// The error string is formatted as RFC 7807 JSON, then wrapped in `{ *char }`.
#[inline]
pub fn make_err_rfc7807(status: u16, detail: &str) -> *mut DooResult {
    let json = crate::Rfc7807Error::new(status, detail).to_json();
    DooResult::err_str(status, &json).into_raw()
}

/// Create an Err `DooResult` from a panic payload.
/// Extracts a human-readable message from the `Box<dyn Any + Send>`.
#[inline]
pub fn make_panic_err(component: &str, payload: Box<dyn std::any::Any + Send>) -> *mut DooResult {
    let msg = if let Some(s) = payload.downcast_ref::<&str>() {
        format!("{} FFI panic: {}", component, s)
    } else if let Some(s) = payload.downcast_ref::<String>() {
        format!("{} FFI panic: {}", component, s)
    } else {
        format!("{} FFI panic: unknown error", component)
    };
    make_err(500, &msg)
}

// ============================================================================
// Panic Safety — Single Source of Truth
// ============================================================================

/// Wrap an FFI function body in `catch_unwind` for panic safety.
///
/// Every `extern "C"` function MUST use this (or `catch_unwind` directly)
/// to prevent panics from crossing the FFI boundary (which is UB).
///
/// # Example
///
/// ```ignore
/// #[no_mangle]
/// pub extern "C" fn my_ffi_fn(arg: *const c_char) -> *mut DooResult {
///     safe_ffi("MyModule", || {
///         let s = c_to_string(arg).map_err(|e| e)?;
///         Ok(make_ok_string(&s))
///     })
/// }
/// ```
#[inline]
pub fn safe_ffi<F>(component: &str, f: F) -> *mut DooResult
where
    F: std::panic::UnwindSafe + FnOnce() -> *mut DooResult,
{
    match std::panic::catch_unwind(f) {
        Ok(result) => result,
        Err(payload) => make_panic_err(component, payload),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_c_to_string_null() {
        assert!(c_to_string(std::ptr::null()).is_err());
    }

    #[test]
    fn test_c_to_string_lossy_null() {
        assert_eq!(c_to_string_lossy(std::ptr::null()), "");
    }

    #[test]
    fn test_string_to_c_roundtrip() {
        let original = "hello world";
        let c_ptr = string_to_c(original);
        assert!(!c_ptr.is_null());
        let back = c_to_string(c_ptr).unwrap();
        assert_eq!(back, original);
        // Cleanup
        unsafe {
            crate::memory::doo_free(c_ptr as *mut u8);
        }
    }

    #[test]
    fn test_make_ok_void() {
        let result = make_ok_void();
        assert!(!result.is_null());
        unsafe {
            assert_eq!((*result).tag, 0);
            crate::result::doo_result_free(result);
        }
    }

    #[test]
    fn test_make_err() {
        let result = make_err(500, "test error");
        assert!(!result.is_null());
        unsafe {
            assert_eq!((*result).tag, 1);
            crate::result::doo_result_free(result);
        }
    }

    #[test]
    fn test_safe_ffi_ok() {
        let result = safe_ffi("Test", || make_ok_void());
        assert!(!result.is_null());
        unsafe {
            assert_eq!((*result).tag, 0);
            crate::result::doo_result_free(result);
        }
    }

    #[test]
    fn test_safe_ffi_panic() {
        let result = safe_ffi("Test", || panic!("intentional panic"));
        assert!(!result.is_null());
        unsafe {
            assert_eq!((*result).tag, 1);
            crate::result::doo_result_free(result);
        }
    }
}
