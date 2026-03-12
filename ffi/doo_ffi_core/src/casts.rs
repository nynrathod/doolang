//! Type Casting Functions - Single Source of Truth
//!
//! Centralized type conversion for ALL FFI casts.
//! PURE OWNERSHIP MODEL - Returned strings are owned by caller.
//!
//! Rules:
//! 1. All returned strings use doo_alloc_string (centralized allocation)
//! 2. Ownership transfers to caller on return
//! 3. Caller is responsible for calling doo_free on returned strings

use std::ffi::CStr;
use std::os::raw::c_char;

use crate::memory::doo_alloc_string;

// ============================================================================
// String -> Primitive Conversions
// ============================================================================

/// Convert a C string to an integer.
/// Returns 0 on invalid input (matches JavaScript-like coercion behavior).
///
/// # Safety
/// - `ptr` must be a valid null-terminated C string or null
#[no_mangle]
pub extern "C" fn doo_cast_str_to_int(ptr: *const c_char) -> i64 {
    if ptr.is_null() {
        return 0;
    }

    unsafe {
        let c_str = match CStr::from_ptr(ptr).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        };

        // Trim whitespace and parse
        let trimmed = c_str.trim();

        // Handle empty string
        if trimmed.is_empty() {
            return 0;
        }

        // Parse as integer (handle both decimal and potential hex/octal)
        trimmed.parse::<i64>().unwrap_or(0)
    }
}

/// Convert a C string to a float.
/// Returns 0.0 on invalid input.
///
/// # Safety
/// - `ptr` must be a valid null-terminated C string or null
#[no_mangle]
pub extern "C" fn doo_cast_str_to_float(ptr: *const c_char) -> f64 {
    if ptr.is_null() {
        return 0.0;
    }

    unsafe {
        let c_str = match CStr::from_ptr(ptr).to_str() {
            Ok(s) => s,
            Err(_) => return 0.0,
        };

        // Trim whitespace and parse
        let trimmed = c_str.trim();

        // Handle empty string
        if trimmed.is_empty() {
            return 0.0;
        }

        // Parse as float
        trimmed.parse::<f64>().unwrap_or(0.0)
    }
}

// ============================================================================
// Primitive -> String Conversions
// ============================================================================

/// Convert an integer to a C string.
/// OWNERSHIP: Caller owns the returned string and must call doo_free.
#[no_mangle]
pub extern "C" fn doo_cast_int_to_str(value: i64) -> *mut c_char {
    let s = value.to_string();
    doo_alloc_string(&s)
}

/// Convert a float to a C string using ryu for clean shortest representation.
/// OWNERSHIP: Caller owns the returned string and must call doo_free.
/// Handles NaN and Infinity edge cases explicitly.
#[no_mangle]
pub extern "C" fn doo_cast_float_to_str(value: f64) -> *mut c_char {
    doo_format_float(value)
}

/// Format a float using ryu — shortest decimal representation.
/// Used by print, cast, and anywhere a float needs clean display.
/// OWNERSHIP: Caller owns the returned string and must call doo_free.
#[no_mangle]
pub extern "C" fn doo_format_float(value: f64) -> *mut c_char {
    if value.is_nan() {
        return doo_alloc_string("NaN");
    }
    if value.is_infinite() {
        return doo_alloc_string(if value.is_sign_positive() {
            "Infinity"
        } else {
            "-Infinity"
        });
    }
    let mut buf = ryu::Buffer::new();
    let s = buf.format(value);
    doo_alloc_string(s)
}

/// Convert a boolean to a C string.
/// OWNERSHIP: Caller owns the returned string and must call doo_free.
#[no_mangle]
pub extern "C" fn doo_cast_bool_to_str(value: bool) -> *mut c_char {
    let s = if value { "true" } else { "false" };
    doo_alloc_string(s)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::doo_free;
    use std::ffi::CString;

    #[test]
    fn test_str_to_int() {
        let s = CString::new("123").unwrap();
        assert_eq!(doo_cast_str_to_int(s.as_ptr()), 123);

        let s = CString::new("-456").unwrap();
        assert_eq!(doo_cast_str_to_int(s.as_ptr()), -456);

        let s = CString::new("  42  ").unwrap();
        assert_eq!(doo_cast_str_to_int(s.as_ptr()), 42);

        let s = CString::new("invalid").unwrap();
        assert_eq!(doo_cast_str_to_int(s.as_ptr()), 0);

        assert_eq!(doo_cast_str_to_int(std::ptr::null()), 0);
    }

    #[test]
    fn test_str_to_float() {
        let s = CString::new("3.14").unwrap();
        assert!((doo_cast_str_to_float(s.as_ptr()) - 3.14).abs() < 0.0001);

        let s = CString::new("-2.5").unwrap();
        assert!((doo_cast_str_to_float(s.as_ptr()) - (-2.5)).abs() < 0.0001);

        let s = CString::new("invalid").unwrap();
        assert_eq!(doo_cast_str_to_float(s.as_ptr()), 0.0);

        assert_eq!(doo_cast_str_to_float(std::ptr::null()), 0.0);
    }

    #[test]
    fn test_int_to_str() {
        let ptr = doo_cast_int_to_str(42);
        assert!(!ptr.is_null());
        unsafe {
            let s = CStr::from_ptr(ptr).to_str().unwrap();
            assert_eq!(s, "42");
            doo_free(ptr as *mut u8);
        }

        let ptr = doo_cast_int_to_str(-123);
        assert!(!ptr.is_null());
        unsafe {
            let s = CStr::from_ptr(ptr).to_str().unwrap();
            assert_eq!(s, "-123");
            doo_free(ptr as *mut u8);
        }
    }

    #[test]
    fn test_float_to_str() {
        let ptr = doo_cast_float_to_str(3.14);
        assert!(!ptr.is_null());
        unsafe {
            let s = CStr::from_ptr(ptr).to_str().unwrap();
            assert!(s.starts_with("3.14"));
            doo_free(ptr as *mut u8);
        }
    }

    #[test]
    fn test_bool_to_str() {
        let ptr = doo_cast_bool_to_str(true);
        assert!(!ptr.is_null());
        unsafe {
            let s = CStr::from_ptr(ptr).to_str().unwrap();
            assert_eq!(s, "true");
            doo_free(ptr as *mut u8);
        }

        let ptr = doo_cast_bool_to_str(false);
        assert!(!ptr.is_null());
        unsafe {
            let s = CStr::from_ptr(ptr).to_str().unwrap();
            assert_eq!(s, "false");
            doo_free(ptr as *mut u8);
        }
    }
}
