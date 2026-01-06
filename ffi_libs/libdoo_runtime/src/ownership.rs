//! Ownership tracking for FFI memory management
//!
//! This module implements Rust-like ownership semantics for memory allocated
//! across FFI boundaries. It tracks who allocated memory (LLVM JIT, FFI, or Rust)
//! so cleanup code can use the correct deallocator.

use std::os::raw::c_char;

/// Ownership tag indicating which allocator owns the memory
/// This must match the values set by the LLVM compiler
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// LLVM JIT allocated - uses RC (reference counting), don't free directly
    LLVM = 0,
    /// FFI allocated via libc::malloc - free with libc::free
    FFI = 1,
    /// Rust Box allocated - free with Box::from_raw
    Rust = 2,
}

impl Default for Owner {
    fn default() -> Self {
        Owner::FFI
    }
}

/// Result type for FFI returns with ownership tracking
/// Layout: { i32 tag, ptr value, u8 owner }
///
/// IMPORTANT: This struct layout MUST match what LLVM JIT generates.
/// The owner field is added at the end for backward compatibility.
#[repr(C)]
pub struct DooResult {
    /// 0 = Ok, 1 = Err
    pub tag: i32,
    /// Pointer to data or error struct
    pub value: *mut std::ffi::c_void,
    /// Who allocated this memory (Owner enum)
    pub owner: Owner,
}

/// HTTP Error struct layout
#[repr(C)]
pub struct DooHttpError {
    pub status: i32,
    pub message: *const c_char,
}

/// HTTP Response struct layout
#[repr(C)]
pub struct DooResponse {
    pub status: i32,
    pub body: *const c_char,
    pub content_type: *const c_char,
}

// ============================================================================
// Factory functions for creating results with proper ownership
// ============================================================================

/// Create an Ok result with void value (no data)
/// Ownership: FFI (we allocate with libc::malloc)
#[inline]
pub fn make_ok_void() -> *mut DooResult {
    unsafe {
        let size = std::mem::size_of::<DooResult>();
        let ptr = libc::malloc(size) as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        (*ptr).tag = 0;
        (*ptr).value = std::ptr::null_mut();
        (*ptr).owner = Owner::FFI;
        ptr
    }
}

/// Create an Ok result with a string value
/// The string is allocated with dooruntime_malloc (RC compatible)
/// Ownership: FFI (wrapper) + the string uses RC header
#[inline]
pub fn make_ok_string(s: &str) -> *mut DooResult {
    let c_str = string_to_c(s);
    unsafe {
        let size = std::mem::size_of::<DooResult>();
        let ptr = libc::malloc(size) as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        (*ptr).tag = 0;
        (*ptr).value = c_str as *mut std::ffi::c_void;
        (*ptr).owner = Owner::FFI;
        ptr
    }
}

/// Create an Ok result with an integer value
/// Ownership: FFI
#[inline]
pub fn make_ok_int(n: i64) -> *mut DooResult {
    unsafe {
        let size = std::mem::size_of::<DooResult>();
        let ptr = libc::malloc(size) as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        (*ptr).tag = 0;
        (*ptr).value = n as *mut std::ffi::c_void;
        (*ptr).owner = Owner::FFI;
        ptr
    }
}

/// Create an Ok result with a boolean value
/// Ownership: FFI
#[inline]
pub fn make_ok_bool(b: bool) -> *mut DooResult {
    unsafe {
        let size = std::mem::size_of::<DooResult>();
        let ptr = libc::malloc(size) as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        (*ptr).tag = 0;
        (*ptr).value = (b as i32) as *mut std::ffi::c_void;
        (*ptr).owner = Owner::FFI;
        ptr
    }
}

/// Create an error result with HTTP error
/// Ownership: FFI
#[inline]
pub fn make_err_http(status: u16, message: &str) -> *mut DooResult {
    unsafe {
        // Allocate error struct
        let error_size = std::mem::size_of::<DooHttpError>();
        let error_ptr = libc::malloc(error_size) as *mut DooHttpError;
        if error_ptr.is_null() {
            return std::ptr::null_mut();
        }
        (*error_ptr).status = status as i32;
        (*error_ptr).message = string_to_c(message);

        // Allocate result wrapper
        let result_size = std::mem::size_of::<DooResult>();
        let result_ptr = libc::malloc(result_size) as *mut DooResult;
        if result_ptr.is_null() {
            libc::free(error_ptr as *mut libc::c_void);
            return std::ptr::null_mut();
        }
        (*result_ptr).tag = 1;
        (*result_ptr).value = error_ptr as *mut std::ffi::c_void;
        (*result_ptr).owner = Owner::FFI;
        result_ptr
    }
}

/// Create an error result with a simple message
/// Ownership: FFI
#[inline]
pub fn make_err_string(message: &str) -> *mut DooResult {
    make_err_http(500, message)
}

// ============================================================================
// Ownership-aware deallocation
// ============================================================================

/// Free an RC-compatible string (allocated with 8-byte header)
/// Memory layout: [RC:4][Len:4][data...][null]
/// The pointer points to data, so we subtract 8 to get the base
#[inline]
unsafe fn free_rc_string_internal(ptr: *const c_char) {
    if ptr.is_null() {
        return;
    }

    let ptr_addr = ptr as usize;
    if ptr_addr < 16 {
        // Invalid pointer, not from heap
        return;
    }

    let base_ptr = (ptr as *mut u8).sub(8);

    // Validate RC header looks reasonable
    let rc = *(base_ptr as *const i32);
    let len = *((base_ptr as *const i32).add(1));

    if rc <= 0 || rc > 1_000_000 || len < 0 || len > 100_000_000 {
        return;
    }

    if rc > 1 {
        *(base_ptr as *mut i32) = rc - 1;
        return;
    }

    libc::free(base_ptr as *mut libc::c_void);
}

#[inline]
unsafe fn free_any_string_internal(ptr: *const c_char) {
    if ptr.is_null() {
        return;
    }

    let ptr_addr = ptr as usize;
    if ptr_addr < 16 {
        return;
    }

    let base_ptr = (ptr as *mut u8).wrapping_sub(8);
    let rc = *(base_ptr as *const i32);
    let len = *((base_ptr as *const i32).add(1));

    // Heuristic: treat as RC-layout only if header is sane and there's a null terminator
    // at exactly data[len]. Avoid scanning memory.
    if rc > 0 && rc <= 1_000_000 && !(len < 0 || len > 1_000_000) {
        let data_ptr = base_ptr.add(8);
        if data_ptr == (ptr as *mut u8) {
            let end = (ptr as *const u8).add(len as usize);
            if *end == 0 {
                if rc > 1 {
                    *(base_ptr as *mut i32) = rc - 1;
                    return;
                }
                libc::free(base_ptr as *mut libc::c_void);
                return;
            }
        }
    }

    libc::free(ptr as *mut libc::c_void);
}

/// Public FFI export for freeing RC strings
#[no_mangle]
pub unsafe extern "C" fn dooruntime_free_rc_string(ptr: *const c_char) {
    free_rc_string_internal(ptr);
}

#[no_mangle]
pub unsafe extern "C" fn dooruntime_free_any_string(ptr: *const c_char) {
    free_any_string_internal(ptr);
}

/// Free a DooResult and its contents based on ownership
///
/// CRITICAL: This function respects ownership to prevent double-free:
/// - Owner::LLVM: Don't free value (RC handles it), only free wrapper if we own it
/// - Owner::FFI: Free everything with libc::free
/// - Owner::Rust: This shouldn't happen in practice, but would use Box::from_raw
#[no_mangle]
pub unsafe extern "C" fn dooruntime_free_result(result: *mut DooResult) {
    if result.is_null() {
        return;
    }

    let result_ref = &*result;
    let tag = result_ref.tag;
    let value = result_ref.value;
    let owner = result_ref.owner;

    match owner {
        Owner::LLVM => {
            // LLVM allocated the value - RC will handle it
            // We only free the wrapper if it was allocated by FFI
            // But since owner is LLVM, the whole thing is LLVM's responsibility
            // Don't free anything - LLVM's RC will handle cleanup
        }
        Owner::FFI => {
            // We allocated everything, free it all
            if !value.is_null() {
                if tag == 0 {
                    // Ok value - check if it's a small int (primitive)
                    if (value as usize) < 4096 {
                        // Primitive value, no heap allocation
                    } else {
                        // Check if it's a DooResponse (first field is status 100-599)
                        let first_i32 = *(value as *const i32);
                        if first_i32 >= 100 && first_i32 <= 599 {
                            // DooResponse - free the struct
                            let response = value as *mut DooResponse;
                            free_any_string_internal((*response).body);
                            free_any_string_internal((*response).content_type);
                            libc::free(value);
                        } else {
                            // Assume it's an RC string
                            free_any_string_internal(value as *const c_char);
                        }
                    }
                } else {
                    // Error value - DooHttpError
                    let error = value as *mut DooHttpError;
                    free_any_string_internal((*error).message);
                    libc::free(value);
                }
            }
            // Free the result wrapper
            libc::free(result as *mut libc::c_void);
        }
        Owner::Rust => {
            // Rust Box allocated - shouldn't happen in normal flow
            // Use libc::free for safety (may leak inner data but won't crash)
            libc::free(result as *mut libc::c_void);
        }
    }
}

/// Check if a result is an error
#[no_mangle]
pub extern "C" fn dooruntime_is_error(result: *const DooResult) -> i32 {
    if result.is_null() {
        return 1; // Treat null as error
    }
    unsafe { (*result).tag }
}

/// Get the owner of a result
#[no_mangle]
pub extern "C" fn dooruntime_get_owner(result: *const DooResult) -> u8 {
    if result.is_null() {
        return Owner::FFI as u8;
    }
    unsafe { (*result).owner as u8 }
}

// ============================================================================
// String allocation helpers (RC-compatible)
// ============================================================================

/// Convert a Rust string to a C string with RC header
/// Memory layout: [RC:4][Len:4][data...][null]
/// Returns pointer to data (offset +8 from base)
#[inline]
pub fn string_to_c(s: &str) -> *const c_char {
    extern "C" {
        fn dooruntime_malloc(size: usize) -> *mut u8;
    }

    unsafe {
        let len = s.len();
        let total_size = len + 1 + 8; // data + null + header
        let alloc_size = (total_size + 15) & !15; // Align to 16 bytes

        let ptr = dooruntime_malloc(alloc_size);
        if ptr.is_null() {
            return std::ptr::null();
        }

        // Zero memory
        std::ptr::write_bytes(ptr, 0, alloc_size);

        // RC header
        *(ptr as *mut i32) = 1; // RC = 1
        *(ptr.add(4) as *mut i32) = len as i32; // Length

        // Copy string data
        let data_ptr = ptr.add(8);
        std::ptr::copy_nonoverlapping(s.as_ptr(), data_ptr, len);
        *data_ptr.add(len) = 0; // Null terminate

        data_ptr as *const c_char
    }
}

/// Convert a C string to a Rust String
#[inline]
pub fn c_to_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}
