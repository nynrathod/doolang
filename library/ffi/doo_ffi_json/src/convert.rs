//! JSON Type Conversion — generic JSON to Doo-native format conversion.
//!
//! Converts serde_json::Value trees into Doo's native memory layout
//! (pointers, arrays with 16-byte headers, maps with key-value entries).
//! Includes depth tracking to prevent stack overflow from deeply nested JSON.

use doo_ffi_core::helpers::c_to_string_lossy;
use doo_ffi_core::memory::{doo_alloc, doo_alloc_string, MIN_ALLOCATION_SIZE};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::{safe_json_parse, MAX_NESTING_DEPTH};

// ============================================================================
// Generic JSON Parse
// ============================================================================

/// Parse JSON string and return pointer to parsed value in Doo-native format.
///
/// This generic parse function handles all JSON types and returns data in
/// Doo's native memory layout:
/// - Primitives: i64, f64, bool, *const c_char
/// - Arrays: [len: i64][cap: i64][elements...] (data ptr returned)
/// - Objects/Maps: [len: i64][cap: i64][entries...] (data ptr returned)
///   where each entry is (key_ptr: *mut u8, value_ptr: *mut u8)
#[no_mangle]
pub extern "C" fn doo_json_parse(json_str: *const c_char) -> *mut std::ffi::c_void {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = c_to_string_lossy(json_str);
        match safe_json_parse(&s) {
            Ok(v) => json_value_to_doo_ptr(v, 0),
            Err(_) => std::ptr::null_mut(),
        }
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Convert serde_json::Value to Doo-native format pointer.
/// Returns data in Doo's expected memory layout for all types.
/// Tracks nesting depth to prevent stack overflow.
pub(crate) fn json_value_to_doo_ptr(v: serde_json::Value, depth: usize) -> *mut std::ffi::c_void {
    if depth > MAX_NESTING_DEPTH {
        return std::ptr::null_mut();
    }
    match v {
        serde_json::Value::Null => std::ptr::null_mut(),
        serde_json::Value::Bool(b) => {
            // Return pointer to bool value
            let ptr = doo_alloc(1);
            if ptr.is_null() {
                return std::ptr::null_mut();
            }
            unsafe {
                *(ptr as *mut u8) = if b { 1 } else { 0 };
            }
            ptr as *mut std::ffi::c_void
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                let ptr = doo_alloc(8);
                if ptr.is_null() {
                    return std::ptr::null_mut();
                }
                unsafe {
                    *(ptr as *mut i64) = i;
                }
                ptr as *mut std::ffi::c_void
            } else if let Some(f) = n.as_f64() {
                let ptr = doo_alloc(8);
                if ptr.is_null() {
                    return std::ptr::null_mut();
                }
                unsafe {
                    *(ptr as *mut f64) = f;
                }
                ptr as *mut std::ffi::c_void
            } else {
                std::ptr::null_mut()
            }
        }
        serde_json::Value::String(s) => doo_alloc_string(&s) as *mut std::ffi::c_void,
        serde_json::Value::Array(arr) => {
            // Convert to Doo array layout: [len: i64][cap: i64][elements...]
            // Each element is recursively converted to Doo format (as pointers)
            let len = arr.len();
            let ptr_size = std::mem::size_of::<*mut std::ffi::c_void>();
            let total_size = 16 + (len * ptr_size); // 16-byte header + element pointers

            let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
            if ptr.is_null() {
                return std::ptr::null_mut();
            }

            unsafe {
                // Header
                *(ptr as *mut i64) = len as i64; // length at offset 0
                *(ptr.add(8) as *mut i64) = len as i64; // capacity at offset 8

                // Elements (each as a pointer to converted value)
                let data_ptr = ptr.add(16) as *mut *mut std::ffi::c_void;
                for (i, elem) in arr.into_iter().enumerate() {
                    let elem_ptr = json_value_to_doo_ptr(elem, depth + 1);
                    *data_ptr.add(i) = elem_ptr;
                }

                // Return data pointer (after header)
                ptr.add(16) as *mut std::ffi::c_void
            }
        }
        serde_json::Value::Object(obj) => {
            // Convert to Doo map layout: [len: i64][cap: i64][entries...]
            // Each entry is (key_ptr: *mut u8, value: i64/f64/ptr)
            let len = obj.len();
            let entry_size = 16; // ptr(8) + value(8)
            let total_size = 16 + (len * entry_size);

            let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
            if ptr.is_null() {
                return std::ptr::null_mut();
            }

            unsafe {
                // Header
                *(ptr as *mut i64) = len as i64;
                *(ptr.add(8) as *mut i64) = len as i64;

                // Entries
                let data_ptr = ptr.add(16);
                for (i, (key, value)) in obj.into_iter().enumerate() {
                    let entry_ptr = data_ptr.add(i * entry_size);

                    // Key as raw C string using centralized allocator
                    *(entry_ptr as *mut *mut c_char) = doo_alloc_string(&key);

                    // Value: store primitives directly, others as pointers
                    match &value {
                        serde_json::Value::Number(n) => {
                            if let Some(i) = n.as_i64() {
                                *(entry_ptr.add(8) as *mut i64) = i;
                            } else if let Some(f) = n.as_f64() {
                                *(entry_ptr.add(8) as *mut f64) = f;
                            } else {
                                *(entry_ptr.add(8) as *mut i64) = 0;
                            }
                        }
                        serde_json::Value::Bool(b) => {
                            *(entry_ptr.add(8) as *mut i64) = if *b { 1 } else { 0 };
                        }
                        _ => {
                            // For strings, null, arrays, objects: store pointer
                            let val_ptr = json_value_to_doo_ptr(value, depth + 1);
                            *(entry_ptr.add(8) as *mut *mut std::ffi::c_void) = val_ptr;
                        }
                    }
                }

                // Return data pointer
                ptr.add(16) as *mut std::ffi::c_void
            }
        }
    }
}
