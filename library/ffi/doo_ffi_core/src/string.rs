//! String Type
//!
//! The ONE string type for FFI.
//! MEMORY MODEL: Pure Ownership/Borrow - No RC, No GC
//! All string data uses doo_alloc_string from memory.rs (single source of truth).

use std::ffi::CStr;
use std::os::raw::c_char;

use crate::memory::{doo_alloc_empty_string, doo_alloc_string, doo_free};

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
        // Guard against u32 overflow: strings > 4GB are truncated with a warning.
        // In practice, HTTP bodies and Doo strings will never be this large.
        let byte_len = s.len();
        let len = if byte_len > u32::MAX as usize {
            u32::MAX
        } else {
            byte_len as u32
        };
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

/// Create a DooString from a byte slice with length.
/// SAFETY: If bytes are not valid UTF-8, they are replaced with U+FFFD (lossy).
/// This prevents UB from `from_utf8_unchecked` on invalid input.
#[no_mangle]
pub extern "C" fn doo_string_new(ptr: *const u8, len: u32) -> DooString {
    if ptr.is_null() || len == 0 {
        return DooString::empty();
    }
    unsafe {
        let slice = std::slice::from_raw_parts(ptr, len as usize);
        // SAFE: from_utf8_lossy handles invalid UTF-8 gracefully
        let cow = String::from_utf8_lossy(slice);
        DooString::from_str(&cow)
    }
}

/// Free a DooString.
/// OWNERSHIP: Takes ownership of the DooString and frees all memory.
/// NOTE: Does NOT use Box::from_raw — the DooString may not have been Box-allocated.
/// Instead, reads fields directly and frees inner data with libc (matching doo_alloc_string).
#[no_mangle]
pub extern "C" fn doo_string_free(s: *mut DooString) {
    if s.is_null() {
        return;
    }
    unsafe {
        // Read the data pointer before freeing anything
        let data = (*s).data;
        if !data.is_null() {
            // Free the string data (allocated with doo_alloc_string → libc::malloc)
            doo_free(data as *mut u8);
        }
        // Zero out the struct to prevent use-after-free
        (*s).data = std::ptr::null_mut();
        (*s).len = 0;
        (*s).cap = 0;
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
// Split
// ============================================================================

/// Split a string by a delimiter, returning a Doo array of C-string pointers.
/// Returns a pointer to the data section of a Doo array header ([len][cap][data...]).
/// Each element is a `*mut c_char` pointing to a heap-allocated null-terminated string.
#[no_mangle]
pub extern "C" fn doo_string_split(str_ptr: *const c_char, delim_ptr: *const c_char) -> *mut u8 {
    use crate::memory::doo_alloc_array;

    let ptr_size = std::mem::size_of::<*mut c_char>();

    if str_ptr.is_null() || delim_ptr.is_null() {
        return doo_alloc_array(0, ptr_size);
    }

    unsafe {
        let s = match CStr::from_ptr(str_ptr).to_str() {
            Ok(v) => v,
            Err(_) => return doo_alloc_array(0, ptr_size),
        };
        let delim = match CStr::from_ptr(delim_ptr).to_str() {
            Ok(v) => v,
            Err(_) => return doo_alloc_array(0, ptr_size),
        };

        let parts: Vec<&str> = if delim.is_empty() {
            vec![s]
        } else {
            s.split(delim).collect()
        };

        let count = parts.len();
        let data_ptr = doo_alloc_array(count, ptr_size);
        if data_ptr.is_null() {
            return data_ptr;
        }

        for (i, part) in parts.iter().enumerate() {
            let part_ptr = doo_alloc_string(part);
            let elem_offset = i * ptr_size;
            *(data_ptr.add(elem_offset) as *mut *mut c_char) = part_ptr;
        }

        data_ptr
    }
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
