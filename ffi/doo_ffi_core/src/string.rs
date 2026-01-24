//! String Type
//!
//! The ONE string type for FFI.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

/// FFI string type - length-prefixed for safety.
#[repr(C)]
pub struct DooString {
    /// String length (not including null terminator)
    pub len: u32,
    /// Capacity
    pub cap: u32,
    /// Pointer to null-terminated string data
    pub data: *mut c_char,
}

impl DooString {
    /// Create a DooString from a Rust string.
    pub fn from_str(s: &str) -> Self {
        let c_string = CString::new(s).unwrap_or_else(|_| CString::new("").unwrap());
        let len = c_string.as_bytes().len() as u32;
        let cap = len;
        let data = c_string.into_raw();
        Self { len, cap, data }
    }

    /// Create an empty DooString.
    pub fn empty() -> Self {
        Self {
            len: 0,
            cap: 0,
            data: std::ptr::null_mut(),
        }
    }

    /// Convert to Rust string (unsafe - caller must ensure valid).
    pub unsafe fn to_str(&self) -> Option<&str> {
        if self.data.is_null() {
            return None;
        }
        CStr::from_ptr(self.data).to_str().ok()
    }

    /// Convert to owned Rust String.
    pub unsafe fn to_string(&self) -> String {
        self.to_str().unwrap_or("").to_string()
    }
}

/// Create a DooString from a C string.
#[no_mangle]
pub extern "C" fn doo_string_from_c(ptr: *const c_char) -> DooString {
    if ptr.is_null() {
        return DooString::empty();
    }
    unsafe {
        let c_str = CStr::from_ptr(ptr);
        DooString::from_str(c_str.to_str().unwrap_or(""))
    }
}

/// Create a DooString from a Rust string slice.
#[no_mangle]
pub extern "C" fn doo_string_new(ptr: *const u8, len: u32) -> DooString {
    if ptr.is_null() || len == 0 {
        return DooString::empty();
    }
    unsafe {
        let slice = std::slice::from_raw_parts(ptr, len as usize);
        let s = std::str::from_utf8_unchecked(slice);
        DooString::from_str(s)
    }
}

/// Free a DooString.
#[no_mangle]
pub extern "C" fn doo_string_free(s: *mut DooString) {
    if s.is_null() {
        return;
    }
    unsafe {
        let string = Box::from_raw(s);
        if !string.data.is_null() {
            drop(CString::from_raw(string.data));
        }
    }
}

/// Get string length.
#[no_mangle]
pub extern "C" fn doo_string_len(s: *const DooString) -> u32 {
    if s.is_null() {
        return 0;
    }
    unsafe { (*s).len }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_from_str() {
        let s = DooString::from_str("hello");
        assert_eq!(s.len, 5);
        unsafe {
            assert_eq!(s.to_str(), Some("hello"));
            // Clean up
            if !s.data.is_null() {
                drop(CString::from_raw(s.data));
            }
        }
    }

    #[test]
    fn test_empty_string() {
        let s = DooString::empty();
        assert_eq!(s.len, 0);
        assert!(s.data.is_null());
    }
}
