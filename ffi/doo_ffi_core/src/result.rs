//! Result Type
//!
//! The ONE result type for all FFI operations.
//! MEMORY MODEL: Pure Ownership/Borrow - No RC, No GC
//! All string data uses doo_alloc_string (simple C strings).
//! Codegen converts to Doo format automatically via clone_ffi_string_to_rc.
//!
//! CRITICAL: Layout MUST match codegen expectation: { i32 tag, ptr value }

use crate::memory::doo_alloc_string;
use std::ffi::c_void;

/// Result tag indicating Ok or Err.
/// Using i32 to match LLVM codegen expectations
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultTag {
    Ok = 0,
    Err = 1,
}

/// The unified result type for all FFI operations.
/// CRITICAL: Layout MUST be { i32, ptr } to match codegen Result struct.
/// The codegen generates: struct { i32 tag, ptr value }
#[repr(C)]
pub struct DooResult {
    /// Tag: 0 = Ok, 1 = Err (i32 to match LLVM codegen)
    pub tag: i32,
    /// Pointer to data (owned, allocated with libc)
    /// - For Ok: points to actual value (string, struct, etc.)
    /// - For Err: points to error struct (e.g., FileError)
    pub data: *mut c_void,
}

impl DooResult {
    /// Create an Ok result with data.
    /// OWNERSHIP: Takes ownership of data pointer.
    #[inline]
    pub fn ok(data: *mut c_void, _len: u32) -> Self {
        Self {
            tag: ResultTag::Ok as i32,
            data,
        }
    }

    /// Create an Ok result with no data.
    #[inline]
    pub fn ok_empty() -> Self {
        Self {
            tag: ResultTag::Ok as i32,
            data: std::ptr::null_mut(),
        }
    }

    /// Create an Ok result from a string.
    /// OWNERSHIP: Allocates simple C string, codegen converts to Doo format.
    #[inline]
    pub fn ok_string(message: &str) -> Self {
        // Use simple C string - codegen will convert it
        let ptr = doo_alloc_string(message) as *mut c_void;
        Self {
            tag: ResultTag::Ok as i32,
            data: ptr,
        }
    }

    /// Create an Err result.
    /// OWNERSHIP: Takes ownership of data pointer.
    #[inline]
    pub fn err(_code: u16, data: *mut c_void, _len: u32) -> Self {
        Self {
            tag: ResultTag::Err as i32,
            data,
        }
    }

    /// Create an Err result from a string.
    /// OWNERSHIP: Allocates simple C string, codegen converts to Doo format.
    #[inline]
    pub fn err_str(_code: u16, message: &str) -> Self {
        // Use simple C string - codegen will convert it
        let ptr = doo_alloc_string(message) as *mut c_void;
        Self {
            tag: ResultTag::Err as i32,
            data: ptr,
        }
    }

    /// Check if result is Ok.
    #[inline]
    pub fn is_ok(&self) -> bool {
        self.tag == ResultTag::Ok as i32
    }

    /// Check if result is Err.
    #[inline]
    pub fn is_err(&self) -> bool {
        self.tag == ResultTag::Err as i32
    }

    /// Convert to raw pointer (consumer must free with doo_result_free).
    /// OWNERSHIP: Transfers ownership to caller.
    #[inline]
    pub fn into_raw(self) -> *mut Self {
        Box::into_raw(Box::new(self))
    }
}

/// Free a DooResult (must be called by Doo code after use).
#[no_mangle]
pub extern "C" fn doo_result_free(result: *mut DooResult) {
    if result.is_null() {
        return;
    }
    unsafe {
        let res = Box::from_raw(result);
        if !res.data.is_null() {
            libc::free(res.data);
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_result_ok() {
        let result = DooResult::ok_empty();
        assert!(result.is_ok());
        assert!(!result.is_err());
    }

    #[test]
    fn test_result_err() {
        let result = DooResult::err(500, std::ptr::null_mut(), 0);
        assert!(result.is_err());
        assert!(!result.is_ok());
    }
}
