//! Memory Functions - Single Source of Truth
//!
//! Centralized memory allocation/deallocation for ALL FFI.
//! PURE OWNERSHIP MODEL - No RC, No GC.
//!
//! Rules:
//! 1. All FFI memory MUST be allocated with doo_alloc/doo_alloc_string
//! 2. Ownership transfers to caller on return
//! 3. Caller is responsible for calling doo_free
//! 4. NEVER use CString::into_raw() - it uses Rust allocator, not libc
//!
//! IMPORTANT: Minimum allocation size ensures data pointer is always within
//! valid memory, even for empty arrays/maps.

use crate::ffi_debug;
use std::ffi::c_void;
use std::os::raw::c_char;

// ============================================================================
// Memory Layout Constants - Single Source of Truth
// ============================================================================

/// Array/Map header size (len: i64, cap: i64)
pub const HEADER_SIZE: usize = 16;

/// Minimum allocation size for arrays/maps.
/// Even empty arrays need header (16 bytes) + minimum data space to ensure
/// the data pointer (header + 16) is within allocated memory.
/// This prevents crashes when accessing empty arrays via data_ptr - 16.
pub const MIN_ALLOCATION_SIZE: usize = 32;

// ============================================================================
// Core Allocation - Single Source of Truth
// ============================================================================

/// Allocate memory using libc malloc.
/// OWNERSHIP: Caller owns the returned memory and must call doo_free.
#[no_mangle]
#[inline]
pub extern "C" fn doo_alloc(size: usize) -> *mut u8 {
    if size == 0 {
        ffi_debug!("MEMORY", "doo_alloc: size=0 -> returning null");
        return std::ptr::null_mut();
    }

    ffi_debug!("MEMORY", "doo_alloc: requesting {} bytes...", size);

    let ptr = unsafe { libc::malloc(size) as *mut u8 };

    if ptr.is_null() {
        ffi_debug!("MEMORY", "doo_alloc: FAILED to allocate {} bytes!", size);
    } else {
        ffi_debug!("MEMORY", "doo_alloc: allocated {} bytes at {:p}", size, ptr);
    }

    ptr
}

/// Reallocate memory using libc realloc.
/// OWNERSHIP: Takes ownership of ptr, returns new owned pointer.
#[no_mangle]
#[inline]
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

/// Free memory allocated with doo_alloc.
/// OWNERSHIP: Takes ownership of ptr and deallocates it.
#[no_mangle]
#[inline]
pub extern "C" fn doo_free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    unsafe { libc::free(ptr as *mut c_void) }
}

// ============================================================================
// String Allocation - CRITICAL: Use these instead of CString::into_raw()
// ============================================================================

/// Allocate a C string using libc malloc.
/// OWNERSHIP: Caller owns the returned string and must call doo_free.
/// CRITICAL: This MUST be used instead of CString::into_raw() to prevent heap corruption.
#[inline]
pub fn doo_alloc_string(s: &str) -> *mut c_char {
    let bytes = s.as_bytes();
    let len = bytes.len();

    // Allocate len + 1 for null terminator
    let ptr = doo_alloc(len + 1);
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        // Copy string bytes
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, len);
        // Null terminate
        *ptr.add(len) = 0;
    }

    ptr as *mut c_char
}

/// Allocate an empty C string.
/// OWNERSHIP: Caller owns the returned string and must call doo_free.
#[inline]
pub fn doo_alloc_empty_string() -> *mut c_char {
    let ptr = doo_alloc(1);
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        *ptr = 0; // Null terminator only
    }
    ptr as *mut c_char
}

/// Clone a C string using libc malloc.
/// OWNERSHIP: Caller owns the returned string and must call doo_free.
#[inline]
pub fn doo_clone_string(src: *const c_char) -> *mut c_char {
    if src.is_null() {
        return doo_alloc_empty_string();
    }
    unsafe {
        let len = libc::strlen(src);
        let ptr = doo_alloc(len + 1);
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(src as *const u8, ptr, len + 1);
        ptr as *mut c_char
    }
}

// ============================================================================
// Array/Map Header Allocation
// ============================================================================

/// Allocate array with header: [len: i64][cap: i64][data...]
/// OWNERSHIP: Caller owns the returned pointer (points to data section).
/// To get header, subtract HEADER_SIZE from returned pointer.
#[inline]
pub fn doo_alloc_array(element_count: usize, element_size: usize) -> *mut u8 {
    let data_size = element_count * element_size;
    let total_size = HEADER_SIZE + data_size;

    // Minimum allocation to prevent empty array crashes
    let alloc_size = total_size.max(MIN_ALLOCATION_SIZE);

    let ptr = doo_alloc(alloc_size);
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        // Write header
        *(ptr as *mut i64) = element_count as i64; // len at offset 0
        *(ptr.add(8) as *mut i64) = element_count as i64; // cap at offset 8

        // Return pointer to data section (after header)
        ptr.add(HEADER_SIZE)
    }
}

/// Allocate map with header: [len: i64][cap: i64][entries...]
/// Same as array allocation but named for clarity.
#[inline]
pub fn doo_alloc_map(entry_count: usize, entry_size: usize) -> *mut u8 {
    doo_alloc_array(entry_count, entry_size)
}

// ============================================================================
// Utility Functions
// ============================================================================

/// Zero memory.
#[no_mangle]
#[inline]
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
#[inline]
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

    #[test]
    fn test_alloc_string() {
        let s = "hello";
        let ptr = doo_alloc_string(s);
        assert!(!ptr.is_null());
        unsafe {
            assert_eq!(*ptr.add(0) as u8, b'h');
            assert_eq!(*ptr.add(5) as u8, 0); // null terminator
        }
        doo_free(ptr as *mut u8);
    }
}
