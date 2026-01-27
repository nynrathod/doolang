//! String Type
//!
//! The ONE string type for FFI.
//! MEMORY MODEL: Pure Ownership/Borrow - No RC, No GC
//! All string data uses doo_alloc_string from memory.rs (single source of truth).

use std::ffi::CStr;
use std::os::raw::c_char;

use crate::memory::{doo_alloc_string, doo_alloc_empty_string, doo_free};

/// FFI string type - length-prefixed for safety.
#[repr(C)]
pub struct DooString {
    /// String length (not including null terminator)
    pub len: u32,
    /// Capacity
    pub cap: u32,
    /// Pointer to null-terminated string data (allocated with libc)
    pub data: *mut c_char,
}

impl DooString {
    /// Create a DooString from a Rust string.
    /// OWNERSHIP: Allocates string using libc malloc (centralized).
    pub fn from_str(s: &str) -> Self {
        let len = s.len() as u32;
        let cap = len;
        // Use centralized string allocation - NOT CString::into_raw()!
        let data = doo_alloc_string(s);
        Self { len, cap, data }
    }

    /// Create an empty DooString.
    pub fn empty() -> Self {
        Self {
            len: 0,
            cap: 0,
            data: doo_alloc_empty_string(),
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
/// OWNERSHIP: Takes ownership of the DooString and frees all memory.
#[no_mangle]
pub extern "C" fn doo_string_free(s: *mut DooString) {
    if s.is_null() {
        return;
    }
    unsafe {
        let string = Box::from_raw(s);
        if !string.data.is_null() {
            // Use centralized free - NOT CString::from_raw()!
            doo_free(string.data as *mut u8);
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
            // Clean up using centralized free
            if !s.data.is_null() {
                doo_free(s.data as *mut u8);
            }
        }
    }

    #[test]
    fn test_empty_string() {
        let s = DooString::empty();
        assert_eq!(s.len, 0);
        // Now empty() allocates a single-byte empty string
        unsafe {
            assert_eq!(s.to_str(), Some(""));
            if !s.data.is_null() {
                doo_free(s.data as *mut u8);
            }
        }
    }
}
