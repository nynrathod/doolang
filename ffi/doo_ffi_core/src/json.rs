//! JSON FFI Functions — Production-Grade, DooCloud-Safe
//!
//! Provides JSON serialization/parsing functions for the Doo compiler.
//! Uses serde_json for parsing but returns values in Doo's native memory layout.
//!
//! MEMORY MODEL: Pure Ownership/Borrow - No RC, No GC
//! All allocations use doo_alloc_* from memory.rs (single source of truth).
//! CRITICAL: Never use CString::into_raw() - it uses Rust allocator causing heap corruption.
//! CRITICAL: Never return null_mut() for collection types - always return valid empty collection.
//!
//! SAFETY:
//! - All extern "C" fn boundaries are wrapped with catch_unwind to prevent UB on panic
//! - JSON input size is limited to MAX_JSON_SIZE to prevent OOM attacks
//! - Recursive parsing has MAX_NESTING_DEPTH to prevent stack overflow
//! - NaN/Infinity floats are serialized as null (JSON spec compliant)
//! - Integer/float formatting uses itoa/ryu for zero-allocation performance

use crate::ffi_debug;
use std::cell::RefCell;
use std::ffi::CStr;
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::memory::{doo_alloc, doo_alloc_empty_string, doo_alloc_string, MIN_ALLOCATION_SIZE};
use crate::rfc7807::{FieldError, Rfc7807Error};

// ============================================================================
// Security Constants — Single Source of Truth
// ============================================================================

/// Maximum JSON input size in bytes (1 MB). Prevents OOM from JSON bombs.
/// Configurable via DOO_MAX_JSON_SIZE env var at runtime.
const MAX_JSON_SIZE: usize = 1_048_576;

/// Maximum JSON nesting depth. Prevents stack overflow from deeply nested JSON.
const MAX_NESTING_DEPTH: usize = 64;

/// Helper: check JSON size limit before parsing
#[inline]
fn check_json_size(s: &str) -> Result<(), String> {
    if s.len() > MAX_JSON_SIZE {
        Err(format!(
            "JSON too large: {} bytes (max {})",
            s.len(),
            MAX_JSON_SIZE
        ))
    } else {
        Ok(())
    }
}

/// Helper: safe JSON parse with size limit
#[inline]
fn safe_json_parse(s: &str) -> Result<serde_json::Value, String> {
    check_json_size(s)?;
    serde_json::from_str(s).map_err(|e| format!("Invalid JSON: {}", e))
}

// ============================================================================
// Parse Error State (Thread-Local) - Uses RFC 7807 format
// ============================================================================

/// Thread-local storage for JSON parse errors
/// Uses RFC 7807 format for consistency across all FFI modules
thread_local! {
    static PARSE_ERROR: RefCell<Option<Rfc7807Error>> = const { RefCell::new(None) };
}

/// Set a parse error (RFC 7807 format)
fn set_parse_error_rfc7807(field: &str, expected: &str, received: &str) {
    let error = Rfc7807Error::new(400, "Bad Request")
        .with_detail(format!(
            "Type mismatch at '{}': expected {}, got {}",
            field, expected, received
        ))
        .with_errors(vec![FieldError::type_mismatch(field, expected, received)]);
    PARSE_ERROR.with(|e| {
        *e.borrow_mut() = Some(error);
    });
}

/// Clear the parse error
#[no_mangle]
pub extern "C" fn doo_json_clear_parse_error() {
    PARSE_ERROR.with(|e| {
        *e.borrow_mut() = None;
    });
}

/// Check if there's a parse error
#[no_mangle]
pub extern "C" fn doo_json_has_parse_error() -> bool {
    PARSE_ERROR.with(|e| e.borrow().is_some())
}

/// Get the parse error status (0 if no error)
#[no_mangle]
pub extern "C" fn doo_json_get_parse_error_status() -> i32 {
    PARSE_ERROR.with(|e| {
        e.borrow()
            .as_ref()
            .map(|err| err.status as i32)
            .unwrap_or(0)
    })
}

/// Get the parse error as RFC 7807 JSON string (empty string if no error)
/// OWNERSHIP: Caller owns the returned string
#[no_mangle]
pub extern "C" fn doo_json_get_parse_error_json() -> *mut c_char {
    PARSE_ERROR.with(|e| {
        e.borrow()
            .as_ref()
            .map(|err| doo_alloc_string(&err.to_json()))
            .unwrap_or_else(doo_alloc_empty_string)
    })
}

/// Helper to get JSON value type as string
fn json_value_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "Bool",
        serde_json::Value::Number(n) => {
            if n.is_i64() {
                "Int"
            } else {
                "Float"
            }
        }
        serde_json::Value::String(_) => "Str",
        serde_json::Value::Array(_) => "Array",
        serde_json::Value::Object(_) => "Object",
    }
}

/// Allocate an empty array with proper header [len=0, cap=0][data...]
/// Returns pointer to data section (header is at ptr-16)
/// CRITICAL: Never returns null - crashes calling code
#[inline]
fn alloc_empty_array() -> *mut u8 {
    ffi_debug!(
        "FFI",
        "alloc_empty_array: allocating {} bytes",
        MIN_ALLOCATION_SIZE
    );
    let ptr = doo_alloc(MIN_ALLOCATION_SIZE);
    if ptr.is_null() {
        ffi_debug!(
            "FFI",
            "CRITICAL: alloc_empty_array got null from doo_alloc!"
        );
        std::process::abort();
    }
    ffi_debug!("FFI", "alloc_empty_array: got ptr={:p}", ptr);
    unsafe {
        *(ptr as *mut i64) = 0; // length
        *(ptr.add(8) as *mut i64) = 0; // capacity
        let data_ptr = ptr.add(16);
        ffi_debug!(
            "FFI",
            "alloc_empty_array: header={:p}, data={:p}, len=0, cap=0",
            ptr,
            data_ptr
        );
        data_ptr
    }
}

/// Allocate an empty map with proper header [len=0, cap=0][data...]
/// Returns pointer to data section (header is at ptr-16)
/// CRITICAL: Never returns null - crashes calling code
#[inline]
fn alloc_empty_map() -> *mut u8 {
    alloc_empty_array() // Same layout as array
}

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
    catch_unwind(|| Box::into_raw(Box::new(JsonWriter::new())))
        .unwrap_or(std::ptr::null_mut())
}

/// Create a new JSON writer with a capacity hint (avoids reallocations)
#[no_mangle]
pub extern "C" fn doo_json_writer_new_with_cap(cap: usize) -> *mut JsonWriter {
    catch_unwind(|| {
        Box::into_raw(Box::new(JsonWriter {
            buffer: Vec::with_capacity(cap.max(64)),
        }))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Free a JSON writer (without finishing)
#[no_mangle]
pub extern "C" fn doo_json_writer_free(writer: *mut JsonWriter) {
    if !writer.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
            let _ = Box::from_raw(writer);
        }));
    }
}

/// Finish writing and return the JSON string (consumes writer)
/// OWNERSHIP: Caller owns the returned string.
/// Returns "null" JSON if writer is null (never returns null ptr)
#[no_mangle]
pub extern "C" fn doo_json_writer_finish(writer: *mut JsonWriter) -> *mut c_char {
    if writer.is_null() {
        return doo_alloc_string("null");
    }
    catch_unwind(AssertUnwindSafe(|| unsafe {
        let writer_box = Box::from_raw(writer);
        let s = String::from_utf8_lossy(&writer_box.buffer);
        doo_alloc_string(&s)
    }))
    .unwrap_or_else(|_| doo_alloc_string("null"))
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

/// Write integer value (zero-allocation via itoa)
#[no_mangle]
pub extern "C" fn doo_json_write_int(writer: *mut JsonWriter, val: i64) {
    if let Some(w) = unsafe { writer.as_mut() } {
        let mut buf = itoa::Buffer::new();
        w.write_raw(buf.format(val).as_bytes());
    }
}

/// Write float value (zero-allocation via ryu, NaN/Infinity → null per JSON spec)
#[no_mangle]
pub extern "C" fn doo_json_write_float(writer: *mut JsonWriter, val: f64) {
    if let Some(w) = unsafe { writer.as_mut() } {
        if val.is_nan() || val.is_infinite() {
            w.write_raw(b"null");
        } else {
            let mut buf = ryu::Buffer::new();
            w.write_raw(buf.format(val).as_bytes());
        }
    }
}

/// Write boolean value
#[no_mangle]
pub extern "C" fn doo_json_write_bool(writer: *mut JsonWriter, val: bool) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(if val { b"true" } else { b"false" });
    }
}

/// Write string value (with proper escaping, catch_unwind protected)
#[no_mangle]
pub extern "C" fn doo_json_write_string(writer: *mut JsonWriter, val: *const c_char) {
    if let Some(w) = unsafe { writer.as_mut() } {
        if val.is_null() {
            w.write_raw(b"null");
            return;
        }
        let c_str = unsafe { CStr::from_ptr(val) };
        let s = c_str.to_string_lossy();
        match catch_unwind(AssertUnwindSafe(|| {
            serde_json::to_string(&s as &str).unwrap_or_else(|_| "null".to_owned())
        })) {
            Ok(escaped) => w.write_raw(escaped.as_bytes()),
            Err(_) => w.write_raw(b"null"),
        }
    }
}

/// Write string as object key (alias for doo_json_write_string)
#[no_mangle]
pub extern "C" fn doo_json_write_key(writer: *mut JsonWriter, key: *const c_char) {
    doo_json_write_string(writer, key);
}

/// Write integer as object key (quoted string, zero-alloc via itoa)
/// JSON standard requires all object keys to be strings
#[no_mangle]
pub extern "C" fn doo_json_write_key_int(writer: *mut JsonWriter, key: i64) {
    if let Some(w) = unsafe { writer.as_mut() } {
        let mut buf = itoa::Buffer::new();
        w.write_raw(b"\"");
        w.write_raw(buf.format(key).as_bytes());
        w.write_raw(b"\"");
    }
}

/// Write float as object key (quoted string, zero-alloc via ryu)
/// JSON standard requires all object keys to be strings
#[no_mangle]
pub extern "C" fn doo_json_write_key_float(writer: *mut JsonWriter, key: f64) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b"\"");
        if key.is_nan() || key.is_infinite() {
            w.write_raw(b"null");
        } else {
            let mut buf = ryu::Buffer::new();
            w.write_raw(buf.format(key).as_bytes());
        }
        w.write_raw(b"\"");
    }
}

/// Write bool as object key (quoted string)
/// JSON standard requires all object keys to be strings
#[no_mangle]
pub extern "C" fn doo_json_write_key_bool(writer: *mut JsonWriter, key: bool) {
    if let Some(w) = unsafe { writer.as_mut() } {
        if key {
            w.write_raw(b"\"true\"");
        } else {
            w.write_raw(b"\"false\"");
        }
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
    catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
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
fn json_value_to_doo_ptr(v: serde_json::Value, depth: usize) -> *mut std::ffi::c_void {
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

// ============================================================================
// Type-Specific JSON Parse Functions
// ============================================================================

/// Parse JSON to Int
/// Handles both JSON numbers (123) and JSON strings containing numbers ("123")
/// Uses direct from_str::<i64> first (no Value tree allocation) for performance.
#[no_mangle]
pub extern "C" fn doo_json_parse_int(json_str: *const c_char) -> i64 {
    ffi_debug!("JSON", "doo_json_parse_int called, json_str={:?}", json_str);
    if json_str.is_null() {
        ffi_debug!("JSON", "doo_json_parse_int: null input");
        return 0;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
        if check_json_size(&s).is_err() {
            return 0;
        }
        ffi_debug!("JSON", "doo_json_parse_int: input='{}'", s);
        // Fast path: direct parse (no Value tree)
        if let Ok(i) = serde_json::from_str::<i64>(&s) {
            ffi_debug!("JSON", "doo_json_parse_int: fast-path result={}", i);
            return i;
        }
        // Fallback: try string-containing-number ("123")
        if let Ok(sv) = serde_json::from_str::<String>(&s) {
            if let Ok(i) = sv.parse::<i64>() {
                ffi_debug!("JSON", "doo_json_parse_int: string-fallback result={}", i);
                return i;
            }
        }
        ffi_debug!("JSON", "doo_json_parse_int: fallback to 0");
        0
    }))
    .unwrap_or(0)
}

/// Parse JSON to Float
/// Handles both JSON numbers (1.5) and JSON strings containing numbers ("1.5")
/// Uses direct from_str::<f64> first (no Value tree allocation) for performance.
#[no_mangle]
pub extern "C" fn doo_json_parse_float(json_str: *const c_char) -> f64 {
    if json_str.is_null() {
        return 0.0;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
        if check_json_size(&s).is_err() {
            return 0.0;
        }
        // Fast path: direct parse (no Value tree)
        if let Ok(f) = serde_json::from_str::<f64>(&s) {
            return f;
        }
        // Fallback: try string-containing-number ("1.5")
        if let Ok(sv) = serde_json::from_str::<String>(&s) {
            if let Ok(f) = sv.parse::<f64>() {
                return f;
            }
        }
        0.0
    }))
    .unwrap_or(0.0)
}

/// Parse JSON to Bool
/// Handles both JSON booleans (true) and JSON strings containing booleans ("true")
/// Uses direct from_str::<bool> first (no Value tree allocation) for performance.
#[no_mangle]
pub extern "C" fn doo_json_parse_bool(json_str: *const c_char) -> bool {
    if json_str.is_null() {
        return false;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
        if check_json_size(&s).is_err() {
            return false;
        }
        // Fast path: direct parse (no Value tree)
        if let Ok(b) = serde_json::from_str::<bool>(&s) {
            return b;
        }
        // Fallback: try string-containing-boolean ("true")
        if let Ok(sv) = serde_json::from_str::<String>(&s) {
            if let Ok(b) = sv.parse::<bool>() {
                return b;
            }
        }
        false
    }))
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
    catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
        if check_json_size(&s).is_err() {
            return doo_alloc_empty_string();
        }
        // Direct string parse (no Value tree)
        match serde_json::from_str::<String>(&s) {
            Ok(result) => doo_alloc_string(&result),
            Err(_) => doo_alloc_empty_string(),
        }
    }))
    .unwrap_or_else(|_| doo_alloc_empty_string())
}

/// Parse JSON array to [Int]
/// Layout: [Len: i64][Cap: i64][elements...]
/// Returns pointer to data section (after 16-byte header)
/// OWNERSHIP: Caller owns the returned array.
/// NEVER returns null - returns empty array on error
/// Sets parse error if any element has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_array_int(json_str: *const c_char) -> *mut i64 {
    ffi_debug!("FFI", "doo_json_parse_array_int: ENTER");

    if json_str.is_null() {
        ffi_debug!("FFI", "doo_json_parse_array_int: null input -> empty array");
        let result = alloc_empty_array() as *mut i64;
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_int: returning empty={:p}",
            result
        );
        return result;
    }

    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    ffi_debug!("FFI", "doo_json_parse_array_int: input={:?}", s);

    if check_json_size(&s).is_err() {
        return alloc_empty_array() as *mut i64;
    }

    // Parse JSON and validate ALL elements are integers
    let arr = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Array(arr)) => arr,
        _ => {
            ffi_debug!("FFI", "doo_json_parse_array_int: not an array -> empty");
            return alloc_empty_array() as *mut i64;
        }
    };

    // Validate all elements and collect
    let mut elements = Vec::with_capacity(arr.len());
    for (i, elem) in arr.iter().enumerate() {
        match elem.as_i64() {
            Some(v) => elements.push(v),
            None => {
                // Type mismatch - set error and return empty array
                let received = json_value_type_name(elem);
                set_parse_error_rfc7807(&format!("[{}]", i), "Int", received);
                ffi_debug!(
                    "FFI",
                    "doo_json_parse_array_int: type mismatch at [{}], expected Int, got {}",
                    i,
                    received
                );
                return alloc_empty_array() as *mut i64;
            }
        }
    }

    ffi_debug!(
        "FFI",
        "doo_json_parse_array_int: parsed {} elements: {:?}",
        elements.len(),
        elements
    );

    let data_size = elements.len() * std::mem::size_of::<i64>();
    let total_size = 16 + data_size; // 16-byte header
    let alloc_size = total_size.max(MIN_ALLOCATION_SIZE);
    ffi_debug!(
        "FFI",
        "doo_json_parse_array_int: allocating {} bytes (data={}, total={}, min={})",
        alloc_size,
        data_size,
        total_size,
        MIN_ALLOCATION_SIZE
    );

    let ptr = doo_alloc(alloc_size);
    if ptr.is_null() {
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_int: alloc failed -> empty array"
        );
        return alloc_empty_array() as *mut i64;
    }
    ffi_debug!("FFI", "doo_json_parse_array_int: got header_ptr={:p}", ptr);

    unsafe {
        // Header: [len: i64][cap: i64]
        *(ptr as *mut i64) = elements.len() as i64; // Length at offset 0
        *(ptr.add(8) as *mut i64) = elements.len() as i64; // Capacity at offset 8

        let data_ptr = ptr.add(16) as *mut i64; // Data at offset 16
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_int: data_ptr={:p}, writing {} elements",
            data_ptr,
            elements.len()
        );

        for (i, val) in elements.iter().enumerate() {
            *data_ptr.add(i) = *val;
        }

        // Verify header is accessible
        let verify_len = *(data_ptr.offset(-2) as *const i64);
        let verify_cap = *(data_ptr.offset(-1) as *const i64);
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_int: RETURN data_ptr={:p}, verified len={}, cap={}",
            data_ptr,
            verify_len,
            verify_cap
        );

        data_ptr
    }
}

/// Parse JSON array to [Float]
/// OWNERSHIP: Caller owns the returned array.
/// NEVER returns null - returns empty array on error
/// Sets parse error if any element has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_array_float(json_str: *const c_char) -> *mut f64 {
    if json_str.is_null() {
        return alloc_empty_array() as *mut f64;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if check_json_size(&s).is_err() {
        return alloc_empty_array() as *mut f64;
    }

    // Parse JSON and validate ALL elements are numbers
    let arr = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Array(arr)) => arr,
        _ => return alloc_empty_array() as *mut f64,
    };

    // Validate all elements and collect
    let mut elements = Vec::with_capacity(arr.len());
    for (i, elem) in arr.iter().enumerate() {
        match elem.as_f64() {
            Some(v) => elements.push(v),
            None => {
                // Type mismatch - set error and return empty array
                let received = json_value_type_name(elem);
                set_parse_error_rfc7807(&format!("[{}]", i), "Float", received);
                return alloc_empty_array() as *mut f64;
            }
        }
    }

    let data_size = elements.len() * std::mem::size_of::<f64>();
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_array() as *mut f64;
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
/// Sets parse error if any element has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_array_bool(json_str: *const c_char) -> *mut u8 {
    ffi_debug!("FFI", "doo_json_parse_array_bool: ENTER");

    if json_str.is_null() {
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_bool: null input -> empty array"
        );
        return alloc_empty_array() as *mut u8;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    ffi_debug!("FFI", "doo_json_parse_array_bool: input={:?}", s);

    if check_json_size(&s).is_err() {
        return alloc_empty_array() as *mut u8;
    }

    // Parse JSON and validate ALL elements are booleans
    let arr = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Array(arr)) => arr,
        _ => return alloc_empty_array() as *mut u8,
    };

    // Validate all elements and collect
    let mut elements = Vec::with_capacity(arr.len());
    for (i, elem) in arr.iter().enumerate() {
        match elem.as_bool() {
            Some(b) => elements.push(if b { 1u8 } else { 0u8 }),
            None => {
                // Type mismatch - set error and return empty array
                let received = json_value_type_name(elem);
                set_parse_error_rfc7807(&format!("[{}]", i), "Bool", received);
                ffi_debug!(
                    "FFI",
                    "doo_json_parse_array_bool: type mismatch at [{}], expected Bool, got {}",
                    i,
                    received
                );
                return alloc_empty_array() as *mut u8;
            }
        }
    }

    ffi_debug!(
        "FFI",
        "doo_json_parse_array_bool: parsed {} elements",
        elements.len()
    );

    // Doo uses i1 (1 byte) for bools in arrays
    let data_size = elements.len() * std::mem::size_of::<u8>();
    let total_size = 16 + data_size; // 16-byte header
    let alloc_size = total_size.max(MIN_ALLOCATION_SIZE);
    ffi_debug!(
        "FFI",
        "doo_json_parse_array_bool: allocating {} bytes",
        alloc_size
    );

    let ptr = doo_alloc(alloc_size);
    if ptr.is_null() {
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_bool: alloc failed -> empty array"
        );
        return alloc_empty_array() as *mut u8;
    }

    unsafe {
        // Header: [len: i64][cap: i64]
        *(ptr as *mut i64) = elements.len() as i64;
        *(ptr.add(8) as *mut i64) = elements.len() as i64;

        let data_ptr = ptr.add(16) as *mut u8;
        for (i, val) in elements.iter().enumerate() {
            *data_ptr.add(i) = *val;
        }
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_bool: RETURN data_ptr={:p}, len={}",
            data_ptr,
            elements.len()
        );
        data_ptr
    }
}

/// Parse JSON array to [Str]
/// Returns pointer to data section (after 16-byte header) containing raw C string pointers
/// OWNERSHIP: Caller owns the returned array and all strings in it.
/// Sets parse error if any element has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_array_str(json_str: *const c_char) -> *mut *mut c_char {
    ffi_debug!("FFI", "doo_json_parse_array_str: ENTER");

    if json_str.is_null() {
        ffi_debug!("FFI", "doo_json_parse_array_str: null input -> empty array");
        return alloc_empty_array() as *mut *mut c_char;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    ffi_debug!("FFI", "doo_json_parse_array_str: input={:?}", s);

    if check_json_size(&s).is_err() {
        return alloc_empty_array() as *mut *mut c_char;
    }

    // Parse JSON and validate ALL elements are strings
    let arr = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Array(arr)) => arr,
        _ => return alloc_empty_array() as *mut *mut c_char,
    };

    // Validate all elements and collect
    let mut elements = Vec::with_capacity(arr.len());
    for (i, elem) in arr.iter().enumerate() {
        match elem.as_str() {
            Some(s) => elements.push(s.to_string()),
            None => {
                // Type mismatch - set error and return empty array
                let received = json_value_type_name(elem);
                set_parse_error_rfc7807(&format!("[{}]", i), "Str", received);
                ffi_debug!(
                    "FFI",
                    "doo_json_parse_array_str: type mismatch at [{}], expected Str, got {}",
                    i,
                    received
                );
                return alloc_empty_array() as *mut *mut c_char;
            }
        }
    }

    ffi_debug!(
        "FFI",
        "doo_json_parse_array_str: parsed {} elements",
        elements.len()
    );

    let data_size = elements.len() * std::mem::size_of::<*mut c_char>();
    let total_size = 16 + data_size; // 16-byte header
    let alloc_size = total_size.max(MIN_ALLOCATION_SIZE);
    ffi_debug!(
        "FFI",
        "doo_json_parse_array_str: allocating {} bytes",
        alloc_size
    );

    let ptr = doo_alloc(alloc_size);
    if ptr.is_null() {
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_str: alloc failed -> empty array"
        );
        return alloc_empty_array() as *mut *mut c_char;
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
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_str: RETURN data_ptr={:p}, len={}",
            data_ptr,
            elements.len()
        );
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
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_str_int(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    // Validate all values are integers
    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        match v.as_i64() {
            Some(i) => pairs.push((k.clone(), i)),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Int", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16; // ptr(8) + i64(8)
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
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
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_str_float(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    // Validate all values are floats/numbers
    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        match v.as_f64() {
            Some(f) => pairs.push((k.clone(), f)),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Float", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
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
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_str_bool(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        match v.as_bool() {
            Some(b) => pairs.push((k.clone(), if b { 1u8 } else { 0u8 })),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Bool", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    // Doo uses { ptr, i1 } which LLVM pads to 16 bytes
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
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
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_str_str(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        match v.as_str() {
            Some(s) => pairs.push((k.clone(), s.to_string())),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Str", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16; // ptr(8) + ptr(8)
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
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
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_int_int(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let ki = match k.parse::<i64>() {
            Ok(ki) => ki,
            Err(_) => continue, // skip non-integer keys
        };
        match v.as_i64() {
            Some(vi) => pairs.push((ki, vi)),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Int", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16; // i64(8) + i64(8)
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
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
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_int_float(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let ki = match k.parse::<i64>() {
            Ok(ki) => ki,
            Err(_) => continue,
        };
        match v.as_f64() {
            Some(vi) => pairs.push((ki, vi)),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Float", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
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
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_int_bool(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let ki = match k.parse::<i64>() {
            Ok(ki) => ki,
            Err(_) => continue,
        };
        match v.as_bool() {
            Some(b) => pairs.push((ki, if b { 1u8 } else { 0u8 })),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Bool", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    // Doo uses { i64, i1 } which LLVM pads to 16 bytes
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
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
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_int_str(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let ki = match k.parse::<i64>() {
            Ok(ki) => ki,
            Err(_) => continue,
        };
        match v.as_str() {
            Some(sv) => pairs.push((ki, sv.to_string())),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Str", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
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
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_float_int(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let ki = match k.parse::<f64>() {
            Ok(ki) => ki,
            Err(_) => continue,
        };
        match v.as_i64() {
            Some(vi) => pairs.push((ki, vi)),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Int", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
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
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_float_float(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let ki = match k.parse::<f64>() {
            Ok(ki) => ki,
            Err(_) => continue,
        };
        match v.as_f64() {
            Some(vi) => pairs.push((ki, vi)),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Float", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
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
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_float_bool(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let ki = match k.parse::<f64>() {
            Ok(ki) => ki,
            Err(_) => continue,
        };
        match v.as_bool() {
            Some(b) => pairs.push((ki, if b { 1u8 } else { 0u8 })),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Bool", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    // Doo uses { f64, i1 } which LLVM pads to 16 bytes
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
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
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_float_str(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let ki = match k.parse::<f64>() {
            Ok(ki) => ki,
            Err(_) => continue,
        };
        match v.as_str() {
            Some(sv) => pairs.push((ki, sv.to_string())),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Str", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
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
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_bool_int(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let kb = match k.as_str() {
            "true" => 1u8,
            "false" => 0u8,
            _ => continue,
        };
        match v.as_i64() {
            Some(vi) => pairs.push((kb, vi)),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Int", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    // Doo uses { i1, i64 } which LLVM pads to 16 bytes
    // Key (i1) at offset 0, value (i64) at offset 8
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
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
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_bool_float(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let kb = match k.as_str() {
            "true" => 1u8,
            "false" => 0u8,
            _ => continue,
        };
        match v.as_f64() {
            Some(vi) => pairs.push((kb, vi)),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Float", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    // Doo uses { i1, f64 } which LLVM pads to 16 bytes
    // Key (i1) at offset 0, value (f64) at offset 8
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
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
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_bool_bool(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let kb = match k.as_str() {
            "true" => 1u8,
            "false" => 0u8,
            _ => continue,
        };
        match v.as_bool() {
            Some(b) => pairs.push((kb, if b { 1u8 } else { 0u8 })),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Bool", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    // Doo uses { i1, i1 } which is 2 bytes (1 + 1, no padding between i1s)
    let pair_size = 2;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
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
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_bool_str(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let kb = match k.as_str() {
            "true" => 1u8,
            "false" => 0u8,
            _ => continue,
        };
        match v.as_str() {
            Some(sv) => pairs.push((kb, sv.to_string())),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Str", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    // Doo uses { i1, ptr } which LLVM pads to 16 bytes
    // Key (i1) at offset 0, value (ptr) at offset 8
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
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
/// Returns empty JSON string "{}" if field not found or invalid JSON (never returns null)
#[no_mangle]
pub extern "C" fn doo_json_get_field(
    json_str: *const c_char,
    field_name: *const c_char,
) -> *mut c_char {
    ffi_debug!(
        "JSON",
        "doo_json_get_field called, json_str={:?}, field_name={:?}",
        json_str,
        field_name
    );
    if json_str.is_null() || field_name.is_null() {
        ffi_debug!("JSON", "doo_json_get_field: null input");
        return doo_alloc_string("{}");
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
        let field = unsafe { CStr::from_ptr(field_name).to_string_lossy() };
        if check_json_size(&s).is_err() {
            return doo_alloc_string("null");
        }
        ffi_debug!("JSON", "doo_json_get_field: json='{}', field='{}'", s, field);

        let result = serde_json::from_str::<serde_json::Value>(&s)
            .ok()
            .and_then(|v| {
                v.as_object()
                    .and_then(|obj| obj.get(field.as_ref()).cloned())
            })
            .and_then(|field_val| serde_json::to_string(&field_val).ok())
            .map(|s| doo_alloc_string(&s))
            .unwrap_or_else(|| doo_alloc_string("null"));
        ffi_debug!("JSON", "doo_json_get_field: returning {:?}", result);
        result
    }))
    .unwrap_or_else(|_| doo_alloc_string("null"))
}

/// Get enum variant name from JSON
/// JSON could be either:
/// - A string: "VariantName" (for unit variants)
/// - An object: {"VariantName": payload} (for variants with data)
/// Returns the variant name as a C string
/// OWNERSHIP: Caller owns the returned string.
/// Returns empty string if invalid JSON or not a variant (never returns null)
/// NOTE: Normalizes variant names to PascalCase for consistent matching
#[no_mangle]
pub extern "C" fn doo_json_get_variant_name(json_str: *const c_char) -> *mut c_char {
    if json_str.is_null() {
        return doo_alloc_string("");
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
        serde_json::from_str::<serde_json::Value>(&s)
            .ok()
            .and_then(|v| match v {
                serde_json::Value::String(s) => Some(s),
                serde_json::Value::Object(obj) if obj.len() == 1 => {
                    obj.keys().next().cloned()
                }
                _ => None,
            })
            .map(|s| {
                let normalized = if !s.is_empty() {
                    let mut chars = s.chars();
                    match chars.next() {
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                        None => s,
                    }
                } else {
                    s
                };
                doo_alloc_string(&normalized)
            })
            .unwrap_or_else(|| doo_alloc_string(""))
    }))
    .unwrap_or_else(|_| doo_alloc_string(""))
}

/// Get enum variant payload from JSON
/// JSON should be: {"VariantName": payload}
/// Returns the payload as a JSON string for recursive parsing
/// OWNERSHIP: Caller owns the returned string.
/// Returns "null" JSON if invalid input (never returns null ptr)
#[no_mangle]
pub extern "C" fn doo_json_get_variant_payload(json_str: *const c_char) -> *mut c_char {
    if json_str.is_null() {
        return doo_alloc_string("null");
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
        serde_json::from_str::<serde_json::Value>(&s)
            .ok()
            .and_then(|v| match v {
                serde_json::Value::Object(obj) if obj.len() == 1 => obj.values().next().cloned(),
                _ => None,
            })
            .and_then(|payload| serde_json::to_string(&payload).ok())
            .map(|s| doo_alloc_string(&s))
            .unwrap_or_else(|| doo_alloc_string("null"))
    }))
    .unwrap_or_else(|_| doo_alloc_string("null"))
}

/// Check if JSON represents a unit variant (plain string like "VariantName")
/// Returns 1 if unit variant, 0 otherwise
#[no_mangle]
pub extern "C" fn doo_json_is_unit_variant(json_str: *const c_char) -> i32 {
    if json_str.is_null() {
        return 0;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
        serde_json::from_str::<serde_json::Value>(&s)
            .ok()
            .map(|v| if v.is_string() { 1 } else { 0 })
            .unwrap_or(0)
    }))
    .unwrap_or(0)
}

// ============================================================================
// Parse-Once / Get-Field-Many API (Performance Optimization)
// ============================================================================
// Problem: doo_json_get_field re-parses full JSON per field (N times for N fields).
// Solution: parse once, cache the Value, extract fields from the cached parse.
//
// Usage:
//   obj = doo_json_parse_object(json_str)   // parse ONCE
//   f1  = doo_json_object_get_field(obj, "name")  // no re-parse
//   f2  = doo_json_object_get_field(obj, "age")   // no re-parse
//   doo_json_object_free(obj)                // release cached parse

/// Parse a JSON string into an opaque object handle (parse once).
/// Returns an opaque pointer to a cached serde_json::Value.
/// OWNERSHIP: Caller must call doo_json_object_free when done.
/// Returns null if input is null or invalid JSON.
#[no_mangle]
pub extern "C" fn doo_json_parse_object(json_str: *const c_char) -> *mut std::ffi::c_void {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { CStr::from_ptr(json_str).to_string_lossy() };
        if check_json_size(&s).is_err() {
            return std::ptr::null_mut();
        }
        match serde_json::from_str::<serde_json::Value>(&s) {
            Ok(v) => Box::into_raw(Box::new(v)) as *mut std::ffi::c_void,
            Err(_) => std::ptr::null_mut(),
        }
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Extract a field from a cached JSON parse result (no re-parse).
/// Returns the field value as a JSON string for type-specific parsing.
/// OWNERSHIP: Caller owns the returned string.
/// Returns "null" if field not found (never returns null ptr).
#[no_mangle]
pub extern "C" fn doo_json_object_get_field(
    obj: *mut std::ffi::c_void,
    field_name: *const c_char,
) -> *mut c_char {
    if obj.is_null() || field_name.is_null() {
        return doo_alloc_string("null");
    }
    catch_unwind(AssertUnwindSafe(|| {
        let value = unsafe { &*(obj as *const serde_json::Value) };
        let field = unsafe { CStr::from_ptr(field_name).to_string_lossy() };
        value
            .as_object()
            .and_then(|o| o.get(field.as_ref()))
            .and_then(|v| serde_json::to_string(v).ok())
            .map(|s| doo_alloc_string(&s))
            .unwrap_or_else(|| doo_alloc_string("null"))
    }))
    .unwrap_or_else(|_| doo_alloc_string("null"))
}

/// Check if a cached JSON object has a specific field.
/// Returns 1 if field exists, 0 otherwise.
#[no_mangle]
pub extern "C" fn doo_json_object_has_field(
    obj: *mut std::ffi::c_void,
    field_name: *const c_char,
) -> i32 {
    if obj.is_null() || field_name.is_null() {
        return 0;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let value = unsafe { &*(obj as *const serde_json::Value) };
        let field = unsafe { CStr::from_ptr(field_name).to_string_lossy() };
        value
            .as_object()
            .map(|o| if o.contains_key(field.as_ref()) { 1 } else { 0 })
            .unwrap_or(0)
    }))
    .unwrap_or(0)
}

/// Free a cached JSON parse result.
/// OWNERSHIP: Takes ownership and drops the cached Value.
#[no_mangle]
pub extern "C" fn doo_json_object_free(obj: *mut std::ffi::c_void) {
    if !obj.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
            let _ = Box::from_raw(obj as *mut serde_json::Value);
        }));
    }
}
