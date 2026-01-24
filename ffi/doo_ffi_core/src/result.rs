//! Result Type
//!
//! The ONE result type for all FFI operations.

use std::ffi::c_void;

/// Result tag indicating Ok or Err.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultTag {
    Ok = 0,
    Err = 1,
}

/// The unified result type for all FFI operations.
#[repr(C)]
pub struct DooResult {
    /// Tag: 0 = Ok, 1 = Err
    pub tag: u8,
    /// Error code (0 if Ok)
    pub code: u16,
    /// Pointer to data (owned)
    pub data: *mut c_void,
    /// Data length
    pub len: u32,
}

impl DooResult {
    /// Create an Ok result with data.
    pub fn ok(data: *mut c_void, len: u32) -> Self {
        Self {
            tag: ResultTag::Ok as u8,
            code: 0,
            data,
            len,
        }
    }

    /// Create an Ok result with no data.
    pub fn ok_empty() -> Self {
        Self {
            tag: ResultTag::Ok as u8,
            code: 0,
            data: std::ptr::null_mut(),
            len: 0,
        }
    }

    /// Create an Ok result from a string.
    pub fn ok_string(message: &str) -> Self {
        let msg = message.to_string();
        let len = msg.len() as u32;
        let ptr = msg.as_ptr() as *mut c_void;
        std::mem::forget(msg);
        Self::ok(ptr, len)
    }

    /// Create an Err result.
    pub fn err(code: u16, data: *mut c_void, len: u32) -> Self {
        Self {
            tag: ResultTag::Err as u8,
            code,
            data,
            len,
        }
    }

    /// Create an Err result from a string.
    pub fn err_str(code: u16, message: &str) -> Self {
        let msg = message.to_string();
        let len = msg.len() as u32;
        let ptr = msg.as_ptr() as *mut c_void;
        std::mem::forget(msg);
        Self::err(code, ptr, len)
    }

    /// Check if result is Ok.
    pub fn is_ok(&self) -> bool {
        self.tag == ResultTag::Ok as u8
    }

    /// Check if result is Err.
    pub fn is_err(&self) -> bool {
        self.tag == ResultTag::Err as u8
    }

    /// Convert to raw pointer (consumer must free).
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
