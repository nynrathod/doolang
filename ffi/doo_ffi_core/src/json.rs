//! JSON FFI Functions
//!
//! Provides JSON serialization/parsing functions for the Doo compiler.
//! Uses serde_json for parsing but returns values in Doo's native memory layout.
//!
//! MEMORY MODEL: Pure Ownership/Borrow - No RC, No GC
//! All allocations use doo_alloc_* from memory.rs (single source of truth).
//! CRITICAL: Never use CString::into_raw() - it uses Rust allocator causing heap corruption.

use std::ffi::CStr;
use std::os::raw::c_char;

use crate::memory::{doo_alloc, doo_alloc_empty_string, doo_alloc_string};

// ============================================================================
// JSON Writer
// ============================================================================

/// Internal JSON writer buffer
pub struct JsonWriter {
    buffer: Vec<u8>,
}

impl JsonWriter {
    fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(1024),
        }
    }
    fn write_raw(&mut self, s: &[u8]) {
        self.buffer.extend_from_slice(s);
    }
}

/// Create a new JSON writer
#[no_mangle]
pub extern "C" fn doo_json_writer_new() -> *mut JsonWriter {
    Box::into_raw(Box::new(JsonWriter::new()))
}

/// Free a JSON writer (without finishing)
#[no_mangle]
pub extern "C" fn doo_json_writer_free(writer: *mut JsonWriter) {
    if !writer.is_null() {
        unsafe {
            let _ = Box::from_raw(writer);
        }
    }
}

/// Finish writing and return the JSON string (consumes writer)
/// OWNERSHIP: Caller owns the returned string.
#[no_mangle]
pub extern "C" fn doo_json_writer_finish(writer: *mut JsonWriter) -> *mut c_char {
    if writer.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        let writer_box = Box::from_raw(writer);
        let s = String::from_utf8_lossy(&writer_box.buffer);
        doo_alloc_string(&s)
    }
}

/// Write object start '{'
#[no_mangle]
pub extern "C" fn doo_json_write_start_object(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b"{");
    }
}

/// Write object end '}'
#[no_mangle]
pub extern "C" fn doo_json_write_end_object(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b"}");
    }
}

/// Write array start '['
#[no_mangle]
pub extern "C" fn doo_json_write_start_array(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b"[");
    }
}

/// Write array end ']'
#[no_mangle]
pub extern "C" fn doo_json_write_end_array(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b"]");
    }
}

/// Write comma ','
#[no_mangle]
pub extern "C" fn doo_json_write_comma(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b",");
    }
}

/// Write colon ':'
#[no_mangle]
pub extern "C" fn doo_json_write_colon(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b":");
    }
}

/// Write null literal
#[no_mangle]
pub extern "C" fn doo_json_write_null(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b"null");
    }
}

/// Write integer value
#[no_mangle]
pub extern "C" fn doo_json_write_int(writer: *mut JsonWriter, val: i64) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(val.to_string().as_bytes());
    }
}

/// Write float value
#[no_mangle]
pub extern "C" fn doo_json_write_float(writer: *mut JsonWriter, val: f64) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(val.to_string().as_bytes());
    }
}

/// Write boolean value
#[no_mangle]
pub extern "C" fn doo_json_write_bool(writer: *mut JsonWriter, val: bool) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(if val { b"true" } else { b"false" });
    }
}

/// Write string value (with proper escaping)
#[no_mangle]
pub extern "C" fn doo_json_write_string(writer: *mut JsonWriter, val: *const c_char) {
    if let Some(w) = unsafe { writer.as_mut() } {
        if val.is_null() {
            w.write_raw(b"null");
            return;
        }
        let c_str = unsafe { CStr::from_ptr(val) };
        let s = c_str.to_string_lossy();
        let escaped = serde_json::to_string(&s as &str).unwrap_or("null".to_string());
        w.write_raw(escaped.as_bytes());
    }
}

/// Write string as object key (alias for doo_json_write_string)
#[no_mangle]
pub extern "C" fn doo_json_write_key(writer: *mut JsonWriter, key: *const c_char) {
    doo_json_write_string(writer, key);
}

/// Write integer as object key (quoted string)
/// JSON standard requires all object keys to be strings
#[no_mangle]
pub extern "C" fn doo_json_write_key_int(writer: *mut JsonWriter, key: i64) {
    if let Some(w) = unsafe { writer.as_mut() } {
        let s = format!("\"{}\"", key);
        w.write_raw(s.as_bytes());
    }
}

/// Write float as object key (quoted string)
/// JSON standard requires all object keys to be strings
#[no_mangle]
pub extern "C" fn doo_json_write_key_float(writer: *mut JsonWriter, key: f64) {
    if let Some(w) = unsafe { writer.as_mut() } {
        let s = format!("\"{}\"", key);
        w.write_raw(s.as_bytes());
    }
}

/// Write bool as object key (quoted string)
/// JSON standard requires all object keys to be strings
#[no_mangle]
pub extern "C" fn doo_json_write_key_bool(writer: *mut JsonWriter, key: bool) {
    if let Some(w) = unsafe { writer.as_mut() } {
        let s = format!("\"{}\"", key);
        w.write_raw(s.as_bytes());
    }
}

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
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(v) => json_value_to_doo_ptr(v),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Convert serde_json::Value to Doo-native format pointer.
/// Returns data in Doo's expected memory layout for all types.
fn json_value_to_doo_ptr(v: serde_json::Value) -> *mut std::ffi::c_void {
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

            let ptr = doo_alloc(total_size.max(24));
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
                    let elem_ptr = json_value_to_doo_ptr(elem);
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

            let ptr = doo_alloc(total_size.max(24));
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
                            let val_ptr = json_value_to_doo_ptr(value);
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

// ============================================================================
// Type-Specific JSON Parse Functions
// ============================================================================

/// Parse JSON to Int
#[no_mangle]
pub extern "C" fn doo_json_parse_int(json_str: *const c_char) -> i64 {
    if json_str.is_null() {
        return 0;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
}

/// Parse JSON to Float
#[no_mangle]
pub extern "C" fn doo_json_parse_float(json_str: *const c_char) -> f64 {
    if json_str.is_null() {
        return 0.0;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
}

/// Parse JSON to Bool
#[no_mangle]
pub extern "C" fn doo_json_parse_bool(json_str: *const c_char) -> bool {
    if json_str.is_null() {
        return false;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Parse JSON to Str (returns C string pointer)
/// Returns empty string if parsing fails or null input, never returns null
/// OWNERSHIP: Caller owns the returned string.
#[no_mangle]
pub extern "C" fn doo_json_parse_str(json_str: *const c_char) -> *mut c_char {
    if json_str.is_null() {
        return doo_alloc_empty_string();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    let result = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    doo_alloc_string(&result)
}

/// Parse JSON array to [Int]
/// Layout: [Len: i64][Cap: i64][elements...]
/// Returns pointer to data section (after 16-byte header)
/// OWNERSHIP: Caller owns the returned array.
#[no_mangle]
pub extern "C" fn doo_json_parse_array_int(json_str: *const c_char) -> *mut i64 {
    if json_str.is_null() {
        if std::env::var("DOO_DEBUG_FFI").is_ok() {
            eprintln!("[FFI] doo_json_parse_array_int: null input");
        }
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if std::env::var("DOO_DEBUG_FFI").is_ok() {
        eprintln!("[FFI] doo_json_parse_array_int: input={:?}", s);
    }

    let elements: Vec<i64> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_array()
                .map(|arr| arr.iter().filter_map(|e| e.as_i64()).collect())
        })
        .unwrap_or_default();

    if std::env::var("DOO_DEBUG_FFI").is_ok() {
        eprintln!("[FFI] doo_json_parse_array_int: parsed {} elements", elements.len());
    }

    let data_size = elements.len() * std::mem::size_of::<i64>();
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(32)); // Minimum 32 bytes
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        // Header: [len: i64][cap: i64]
        *(ptr as *mut i64) = elements.len() as i64; // Length at offset 0
        *(ptr.add(8) as *mut i64) = elements.len() as i64; // Capacity at offset 8

        let data_ptr = ptr.add(16) as *mut i64; // Data at offset 16
        for (i, val) in elements.iter().enumerate() {
            *data_ptr.add(i) = *val;
        }
        if std::env::var("DOO_DEBUG_FFI").is_ok() {
            eprintln!("[FFI] doo_json_parse_array_int: returning ptr={:p}", data_ptr);
        }
        data_ptr
    }
}

/// Parse JSON array to [Float]
/// OWNERSHIP: Caller owns the returned array.
#[no_mangle]
pub extern "C" fn doo_json_parse_array_float(json_str: *const c_char) -> *mut f64 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let elements: Vec<f64> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_array()
                .map(|arr| arr.iter().filter_map(|e| e.as_f64()).collect())
        })
        .unwrap_or_default();

    let data_size = elements.len() * std::mem::size_of::<f64>();
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(32));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        // Header: [len: i64][cap: i64]
        *(ptr as *mut i64) = elements.len() as i64;
        *(ptr.add(8) as *mut i64) = elements.len() as i64;

        let data_ptr = ptr.add(16) as *mut f64;
        for (i, val) in elements.iter().enumerate() {
            *data_ptr.add(i) = *val;
        }
        data_ptr
    }
}

/// Parse JSON array to [Bool]
/// OWNERSHIP: Caller owns the returned array.
#[no_mangle]
pub extern "C" fn doo_json_parse_array_bool(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let elements: Vec<u8> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|e| e.as_bool())
                    .map(|b| if b { 1u8 } else { 0u8 })
                    .collect()
            })
        })
        .unwrap_or_default();

    // Doo uses i1 (1 byte) for bools in arrays
    let data_size = elements.len() * std::mem::size_of::<u8>();
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(32));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        // Header: [len: i64][cap: i64]
        *(ptr as *mut i64) = elements.len() as i64;
        *(ptr.add(8) as *mut i64) = elements.len() as i64;

        let data_ptr = ptr.add(16) as *mut u8;
        for (i, val) in elements.iter().enumerate() {
            *data_ptr.add(i) = *val;
        }
        data_ptr
    }
}

/// Parse JSON array to [Str]
/// Returns pointer to data section (after 16-byte header) containing raw C string pointers
/// OWNERSHIP: Caller owns the returned array and all strings in it.
#[no_mangle]
pub extern "C" fn doo_json_parse_array_str(json_str: *const c_char) -> *mut *mut c_char {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let elements: Vec<String> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|e| e.as_str().map(|s| s.to_string()))
                    .collect()
            })
        })
        .unwrap_or_default();

    let data_size = elements.len() * std::mem::size_of::<*mut c_char>();
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(32));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        // Header: [len: i64][cap: i64]
        *(ptr as *mut i64) = elements.len() as i64;
        *(ptr.add(8) as *mut i64) = elements.len() as i64;

        let data_ptr = ptr.add(16) as *mut *mut c_char;
        for (i, val) in elements.iter().enumerate() {
            // Use centralized string allocation - NOT CString::into_raw()!
            *data_ptr.add(i) = doo_alloc_string(val);
        }
        data_ptr
    }
}

// ============================================================================
// Map Parse Functions - {Str: *}
// ============================================================================

/// Parse JSON map {Str: Int}
/// Layout: [Len: i64][Cap: i64][entries...]
/// Returns pointer to data section (after 16-byte header)
/// OWNERSHIP: Caller owns the returned map and all keys.
#[no_mangle]
pub extern "C" fn doo_json_parse_map_str_int(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let pairs: Vec<(String, i64)> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_object().map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_i64().map(|i| (k.clone(), i)))
                    .collect()
            })
        })
        .unwrap_or_default();

    let pair_size = 16; // ptr(8) + i64(8)
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(24));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len; // offset 0: length
        *(ptr.add(8) as *mut i64) = cap; // offset 8: capacity

        let data_ptr = ptr.add(16); // data starts after 16-byte header
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut *mut c_char) = doo_alloc_string(key);
            *(pair_ptr.add(8) as *mut i64) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Str: Float}
/// OWNERSHIP: Caller owns the returned map and all keys.
#[no_mangle]
pub extern "C" fn doo_json_parse_map_str_float(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let pairs: Vec<(String, f64)> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_object().map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_f64().map(|f| (k.clone(), f)))
                    .collect()
            })
        })
        .unwrap_or_default();

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(24));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut *mut c_char) = doo_alloc_string(key);
            *(pair_ptr.add(8) as *mut f64) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Str: Bool}
/// OWNERSHIP: Caller owns the returned map and all keys.
#[no_mangle]
pub extern "C" fn doo_json_parse_map_str_bool(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let pairs: Vec<(String, u8)> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_object().map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        v.as_bool().map(|b| (k.clone(), if b { 1u8 } else { 0u8 }))
                    })
                    .collect()
            })
        })
        .unwrap_or_default();

    // Doo uses { ptr, i1 } which LLVM pads to 16 bytes
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(24));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut *mut c_char) = doo_alloc_string(key);
            // Store bool as u8 (i1) at offset 8
            *(pair_ptr.add(8) as *mut u8) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Str: Str}
/// OWNERSHIP: Caller owns the returned map and all keys/values.
#[no_mangle]
pub extern "C" fn doo_json_parse_map_str_str(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let pairs: Vec<(String, String)> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_object().map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
        })
        .unwrap_or_default();

    let pair_size = 16; // ptr(8) + ptr(8)
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(24));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut *mut c_char) = doo_alloc_string(key);
            *(pair_ptr.add(8) as *mut *mut c_char) = doo_alloc_string(value);
        }
        data_ptr
    }
}

// ============================================================================
// Map Parse Functions - {Int: *}
// ============================================================================

/// Parse JSON map {Int: Int} - keys are stringified integers
/// OWNERSHIP: Caller owns the returned map.
#[no_mangle]
pub extern "C" fn doo_json_parse_map_int_int(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let pairs: Vec<(i64, i64)> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_object().map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        k.parse::<i64>()
                            .ok()
                            .and_then(|ki| v.as_i64().map(|vi| (ki, vi)))
                    })
                    .collect()
            })
        })
        .unwrap_or_default();

    let pair_size = 16; // i64(8) + i64(8)
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(24));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut i64) = *key;
            *(pair_ptr.add(8) as *mut i64) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Int: Float}
/// OWNERSHIP: Caller owns the returned map.
#[no_mangle]
pub extern "C" fn doo_json_parse_map_int_float(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let pairs: Vec<(i64, f64)> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_object().map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        k.parse::<i64>()
                            .ok()
                            .and_then(|ki| v.as_f64().map(|vi| (ki, vi)))
                    })
                    .collect()
            })
        })
        .unwrap_or_default();

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(24));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut i64) = *key;
            *(pair_ptr.add(8) as *mut f64) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Int: Bool}
/// OWNERSHIP: Caller owns the returned map.
#[no_mangle]
pub extern "C" fn doo_json_parse_map_int_bool(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let pairs: Vec<(i64, u8)> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_object().map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        k.parse::<i64>()
                            .ok()
                            .and_then(|ki| v.as_bool().map(|b| (ki, if b { 1u8 } else { 0u8 })))
                    })
                    .collect()
            })
        })
        .unwrap_or_default();

    // Doo uses { i64, i1 } which LLVM pads to 16 bytes
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(24));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut i64) = *key;
            // Store bool as u8 (i1) at offset 8
            *(pair_ptr.add(8) as *mut u8) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Int: Str}
/// OWNERSHIP: Caller owns the returned map and all string values.
#[no_mangle]
pub extern "C" fn doo_json_parse_map_int_str(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let pairs: Vec<(i64, String)> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_object().map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        k.parse::<i64>()
                            .ok()
                            .and_then(|ki| v.as_str().map(|s| (ki, s.to_string())))
                    })
                    .collect()
            })
        })
        .unwrap_or_default();

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(24));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut i64) = *key;
            *(pair_ptr.add(8) as *mut *mut c_char) = doo_alloc_string(value);
        }
        data_ptr
    }
}

// ============================================================================
// Map Parse Functions - {Float: *}
// ============================================================================

/// Parse JSON map {Float: Int}
/// OWNERSHIP: Caller owns the returned map.
#[no_mangle]
pub extern "C" fn doo_json_parse_map_float_int(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let pairs: Vec<(f64, i64)> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_object().map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        k.parse::<f64>()
                            .ok()
                            .and_then(|ki| v.as_i64().map(|vi| (ki, vi)))
                    })
                    .collect()
            })
        })
        .unwrap_or_default();

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(24));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut f64) = *key;
            *(pair_ptr.add(8) as *mut i64) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Float: Float}
/// OWNERSHIP: Caller owns the returned map.
#[no_mangle]
pub extern "C" fn doo_json_parse_map_float_float(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let pairs: Vec<(f64, f64)> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_object().map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        k.parse::<f64>()
                            .ok()
                            .and_then(|ki| v.as_f64().map(|vi| (ki, vi)))
                    })
                    .collect()
            })
        })
        .unwrap_or_default();

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(24));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut f64) = *key;
            *(pair_ptr.add(8) as *mut f64) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Float: Bool}
/// OWNERSHIP: Caller owns the returned map.
#[no_mangle]
pub extern "C" fn doo_json_parse_map_float_bool(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let pairs: Vec<(f64, u8)> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_object().map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        k.parse::<f64>()
                            .ok()
                            .and_then(|ki| v.as_bool().map(|b| (ki, if b { 1u8 } else { 0u8 })))
                    })
                    .collect()
            })
        })
        .unwrap_or_default();

    // Doo uses { f64, i1 } which LLVM pads to 16 bytes
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(24));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut f64) = *key;
            // Store bool as u8 (i1) at offset 8
            *(pair_ptr.add(8) as *mut u8) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Float: Str}
/// OWNERSHIP: Caller owns the returned map and all string values.
#[no_mangle]
pub extern "C" fn doo_json_parse_map_float_str(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let pairs: Vec<(f64, String)> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_object().map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        k.parse::<f64>()
                            .ok()
                            .and_then(|ki| v.as_str().map(|s| (ki, s.to_string())))
                    })
                    .collect()
            })
        })
        .unwrap_or_default();

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(24));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut f64) = *key;
            *(pair_ptr.add(8) as *mut *mut c_char) = doo_alloc_string(value);
        }
        data_ptr
    }
}

// ============================================================================
// Map Parse Functions - {Bool: *}
// ============================================================================

/// Parse JSON map {Bool: Int} - keys are "true" or "false"
/// OWNERSHIP: Caller owns the returned map.
#[no_mangle]
pub extern "C" fn doo_json_parse_map_bool_int(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let pairs: Vec<(u8, i64)> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_object().map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        let kb = match k.as_str() {
                            "true" => Some(1u8),
                            "false" => Some(0u8),
                            _ => None,
                        };
                        kb.and_then(|ki| v.as_i64().map(|vi| (ki, vi)))
                    })
                    .collect()
            })
        })
        .unwrap_or_default();

    // Doo uses { i1, i64 } which LLVM pads to 16 bytes
    // Key (i1) at offset 0, value (i64) at offset 8
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(24));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            // Store bool key as u8 (i1) at offset 0
            *(pair_ptr as *mut u8) = *key;
            // Store i64 value at offset 8 (due to alignment)
            *(pair_ptr.add(8) as *mut i64) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Bool: Float}
/// OWNERSHIP: Caller owns the returned map.
#[no_mangle]
pub extern "C" fn doo_json_parse_map_bool_float(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let pairs: Vec<(u8, f64)> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_object().map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        let kb = match k.as_str() {
                            "true" => Some(1u8),
                            "false" => Some(0u8),
                            _ => None,
                        };
                        kb.and_then(|ki| v.as_f64().map(|vi| (ki, vi)))
                    })
                    .collect()
            })
        })
        .unwrap_or_default();

    // Doo uses { i1, f64 } which LLVM pads to 16 bytes
    // Key (i1) at offset 0, value (f64) at offset 8
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(24));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            // Store bool key as u8 (i1) at offset 0
            *(pair_ptr as *mut u8) = *key;
            // Store f64 value at offset 8 (due to alignment)
            *(pair_ptr.add(8) as *mut f64) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Bool: Bool}
/// OWNERSHIP: Caller owns the returned map.
#[no_mangle]
pub extern "C" fn doo_json_parse_map_bool_bool(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let pairs: Vec<(u8, u8)> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_object().map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        let kb = match k.as_str() {
                            "true" => Some(1u8),
                            "false" => Some(0u8),
                            _ => None,
                        };
                        kb.and_then(|ki| v.as_bool().map(|b| (ki, if b { 1u8 } else { 0u8 })))
                    })
                    .collect()
            })
        })
        .unwrap_or_default();

    // Doo uses { i1, i1 } which is 2 bytes (1 + 1, no padding between i1s)
    let pair_size = 2;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(24));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            // Store bool key as u8 (i1)
            *(pair_ptr as *mut u8) = *key;
            // Store bool value as u8 (i1) at offset 1 (next byte)
            *(pair_ptr.add(1) as *mut u8) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Bool: Str}
/// OWNERSHIP: Caller owns the returned map and all string values.
#[no_mangle]
pub extern "C" fn doo_json_parse_map_bool_str(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    let pairs: Vec<(u8, String)> = serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_object().map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        let kb = match k.as_str() {
                            "true" => Some(1u8),
                            "false" => Some(0u8),
                            _ => None,
                        };
                        kb.and_then(|ki| v.as_str().map(|s| (ki, s.to_string())))
                    })
                    .collect()
            })
        })
        .unwrap_or_default();

    // Doo uses { i1, ptr } which LLVM pads to 16 bytes
    // Key (i1) at offset 0, value (ptr) at offset 8
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(24));
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            // Store bool key as u8 (i1) at offset 0
            *(pair_ptr as *mut u8) = *key;
            // Store ptr value at offset 8 (due to alignment)
            *(pair_ptr.add(8) as *mut *mut c_char) = doo_alloc_string(value);
        }
        data_ptr
    }
}

// ============================================================================
// Struct/Enum JSON Parse Helpers
// ============================================================================

/// Extract a field from a JSON object by name, returning it as a JSON string
/// This allows recursive parsing of complex types
/// OWNERSHIP: Caller owns the returned string.
#[no_mangle]
pub extern "C" fn doo_json_get_field(
    json_str: *const c_char,
    field_name: *const c_char,
) -> *mut c_char {
    if json_str.is_null() || field_name.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    let field = unsafe { CStr::from_ptr(field_name).to_string_lossy() };

    serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| {
            v.as_object()
                .and_then(|obj| obj.get(field.as_ref()).cloned())
        })
        .and_then(|field_val| serde_json::to_string(&field_val).ok())
        .map(|s| doo_alloc_string(&s))
        .unwrap_or(std::ptr::null_mut())
}

/// Get enum variant name from JSON
/// JSON could be either:
/// - A string: "VariantName" (for unit variants)
/// - An object: {"VariantName": payload} (for variants with data)
/// Returns the variant name as a C string
/// OWNERSHIP: Caller owns the returned string.
#[no_mangle]
pub extern "C" fn doo_json_get_variant_name(json_str: *const c_char) -> *mut c_char {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| match v {
            serde_json::Value::String(s) => Some(s),
            serde_json::Value::Object(obj) if obj.len() == 1 => {
                obj.keys().next().map(|k| k.clone())
            }
            _ => None,
        })
        .map(|s| doo_alloc_string(&s))
        .unwrap_or(std::ptr::null_mut())
}

/// Get enum variant payload from JSON
/// JSON should be: {"VariantName": payload}
/// Returns the payload as a JSON string for recursive parsing
/// OWNERSHIP: Caller owns the returned string.
#[no_mangle]
pub extern "C" fn doo_json_get_variant_payload(json_str: *const c_char) -> *mut c_char {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .and_then(|v| match v {
            serde_json::Value::Object(obj) if obj.len() == 1 => obj.values().next().cloned(),
            _ => None,
        })
        .and_then(|payload| serde_json::to_string(&payload).ok())
        .map(|s| doo_alloc_string(&s))
        .unwrap_or(std::ptr::null_mut())
}

/// Check if JSON represents a unit variant (plain string like "VariantName")
/// Returns 1 if unit variant, 0 otherwise
#[no_mangle]
pub extern "C" fn doo_json_is_unit_variant(json_str: *const c_char) -> i32 {
    if json_str.is_null() {
        return 0;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };

    serde_json::from_str::<serde_json::Value>(&s)
        .ok()
        .map(|v| if v.is_string() { 1 } else { 0 })
        .unwrap_or(0)
}
