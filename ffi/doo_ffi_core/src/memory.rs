//! Memory Functions
//!
//! Centralized memory allocation/deallocation for FFI.

use std::ffi::c_void;

/// Allocate memory.
#[no_mangle]
pub extern "C" fn doo_alloc(size: usize) -> *mut u8 {
    if size == 0 {
        return std::ptr::null_mut();
    }
    unsafe { libc::malloc(size) as *mut u8 }
}

/// Reallocate memory.
#[no_mangle]
pub extern "C" fn doo_realloc(ptr: *mut u8, new_size: usize) -> *mut u8 {
    if ptr.is_null() {
        return doo_alloc(new_size);
    }
    if new_size == 0 {
        doo_free(ptr);
        return std::ptr::null_mut();
    }
    unsafe { libc::realloc(ptr as *mut c_void, new_size) as *mut u8 }
}

/// Free memory.
#[no_mangle]
pub extern "C" fn doo_free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    unsafe { libc::free(ptr as *mut c_void) }
}

/// Zero memory.
#[no_mangle]
pub extern "C" fn doo_zero(ptr: *mut u8, size: usize) {
    if ptr.is_null() || size == 0 {
        return;
    }
    unsafe {
        std::ptr::write_bytes(ptr, 0, size);
    }
}

/// Copy memory.
#[no_mangle]
pub extern "C" fn doo_memcpy(dest: *mut u8, src: *const u8, size: usize) {
    if dest.is_null() || src.is_null() || size == 0 {
        return;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(src, dest, size);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alloc_free() {
        let ptr = doo_alloc(64);
        assert!(!ptr.is_null());
        doo_free(ptr);
    }

    #[test]
    fn test_zero_size_alloc() {
        let ptr = doo_alloc(0);
        assert!(ptr.is_null());
    }
}
