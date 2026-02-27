//! Response Helpers
//!
//! RFC 7807 error response builders, struct serialization to JSON,
//! result-to-response conversion, and array serialization.

use std::collections::HashMap;
use std::ffi::c_void;
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};

use doo_ffi_core::{ffi_safe_cstr, ffi_safe_i32};

use crate::error::*;
use crate::helpers::{
    c_to_string, get_current_request_path, get_last_error_json, get_last_error_status, string_to_c,
    CONTENT_TYPE_JSON,
};
use crate::router::get_frozen_routes;
use crate::types::*;

// ============================================================================
// ERROR STATUS MAPPING
// ============================================================================

/// Map error enum variant name to HTTP status code.
/// This is the centralized mapping for all error enums.
/// Uses common HTTP error naming conventions (Unauthorized -> 401, Forbidden -> 403, etc.)
#[no_mangle]
pub extern "C" fn doohttp_error_variant_to_status(
    enum_name: *const c_char,
    variant_name: *const c_char,
    variant_index: i32,
) -> i32 {
    ffi_safe_i32!({
        let _enum_name = c_to_string(enum_name);
        let variant_str = c_to_string(variant_name);

        // Common error status mappings based on variant name
        match variant_str.as_str() {
            "Unauthorized" => 401,
            "Forbidden" => 403,
            "NotFound" => 404,
            "MethodNotAllowed" => 405,
            "Conflict" => 409,
            "ValidationError" => 422,
            "TooManyRequests" => 429,
            "InternalError" | "ServerError" => 500,
            "BadRequest" => 400,
            "NotImplemented" => 501,
            "ServiceUnavailable" => 503,
            // Default to 500 + variant_index for unknown variants
            _ => 500 + variant_index,
        }
    })
}

/// Build RFC 7807 error JSON from status code and title
#[no_mangle]
pub extern "C" fn doohttp_build_rfc7807_error(status: i32, title: *const c_char) -> *const c_char {
    ffi_safe_cstr!({
        let title_str = c_to_string(title);

        // Build RFC 7807 compliant error JSON using centralized source
        let err = Rfc7807Error::new(status as u16, &title_str);
        string_to_c(&err.to_json())
    })
}

/// Format an error message string as JSON with {"error": "message"} format.
/// This is used by generated wrapper code to format database/FFI errors for HTTP responses.
#[no_mangle]
pub extern "C" fn doohttp_format_error_as_json(error_msg: *const c_char) -> *const c_char {
    ffi_safe_cstr!({
        let msg = if error_msg.is_null() {
            "Unknown error".to_string()
        } else {
            c_to_string(error_msg)
        };

        // Escape any special JSON characters in the message
        let escaped = msg
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");

        let error_json = format!(r#"{{"error":"{}"}}"#, escaped);
        string_to_c(&error_json)
    })
}

// ============================================================================
// RFC 7807 ERROR FUNCTIONS
// ============================================================================

#[no_mangle]
pub extern "C" fn doohttp_error_rfc7807(
    status: i32,
    detail: *const c_char,
    instance: *const c_char,
) -> *const c_char {
    ffi_safe_cstr!({
        let detail_str = c_to_string(detail);
        let instance_str = if instance.is_null() {
            get_current_request_path()
        } else {
            c_to_string(instance)
        };

        let err = match status {
            400 => bad_request(detail_str, instance_str),
            401 => unauthorized(detail_str, instance_str),
            403 => forbidden(detail_str, instance_str),
            404 => not_found(detail_str, instance_str),
            409 => conflict(detail_str, instance_str),
            429 => too_many_requests(detail_str, instance_str),
            _ => internal_error(detail_str, instance_str),
        };
        string_to_c(&err.to_json())
    })
}

#[no_mangle]
pub extern "C" fn doohttp_error_rfc7807_auto_instance(
    status: i32,
    detail: *const c_char,
) -> *const c_char {
    ffi_safe_cstr!({ doohttp_error_rfc7807(status, detail, std::ptr::null()) })
}

#[no_mangle]
pub extern "C" fn doohttp_last_error_status() -> i32 {
    ffi_safe_i32!({ get_last_error_status() })
}

#[no_mangle]
pub extern "C" fn doohttp_last_error_json() -> *const c_char {
    ffi_safe_cstr!({ string_to_c(&get_last_error_json()) })
}

#[no_mangle]
pub extern "C" fn doohttp_error_to_status(
    _error_type: *const c_char,
    variant: *const c_char,
) -> i32 {
    ffi_safe_i32!({
        let variant_str = c_to_string(variant);
        match variant_str.as_str() {
            "NotFound" => 404,
            "InvalidInput" | "ValidationError" => 422,
            "Unauthorized" => 401,
            "Forbidden" => 403,
            "Conflict" | "AlreadyExists" => 409,
            "BadRequest" => 400,
            _ => 500,
        }
    })
}

// ============================================================================
// RESPONSE CREATION
// ============================================================================

#[no_mangle]
pub extern "C" fn doohttp_create_response_from_result(
    tag: i32,
    value_ptr: *const c_void,
    success_body_ptr: *const c_char,
) -> *mut DooResponse {
    let result = catch_unwind(AssertUnwindSafe(|| {
        unsafe {
            let response = libc::malloc(std::mem::size_of::<DooResponse>()) as *mut DooResponse;
            if response.is_null() {
                return std::ptr::null_mut();
            }

            if tag == 1 {
                // Error
                (*response).status = 500;
                (*response).body = if value_ptr.is_null() {
                    string_to_c(r#"{"error":"Unknown error"}"#)
                } else {
                    value_ptr as *const c_char
                };
            } else {
                // Success
                (*response).status = 200;
                (*response).body = success_body_ptr;
            }
            (*response).content_type = string_to_c(CONTENT_TYPE_JSON);
            response
        }
    }));
    match result {
        Ok(v) => v,
        Err(_) => {
            doo_ffi_core::ffi_fatal!("Panic in doohttp_create_response_from_result");
            std::ptr::null_mut()
        }
    }
}

// ============================================================================
// STRUCT SERIALIZATION
// ============================================================================

/// Serialize a struct to JSON for HTTP response.
/// Takes struct pointer and handler name, looks up metadata to serialize.
///
/// Performance: uses direct buffer writing instead of serde_json::Value tree.
/// References frozen registry metadata — zero cloning per request.
#[no_mangle]
pub extern "C" fn doohttp_serialize_struct_to_json(
    struct_ptr: *const c_void,
    handler_name: *const c_char,
) -> *const c_char {
    if struct_ptr.is_null() || handler_name.is_null() {
        return string_to_c("{}");
    }

    let handler_name_str = unsafe { std::ffi::CStr::from_ptr(handler_name) };
    let handler_name_str = match handler_name_str.to_str() {
        Ok(s) => s,
        Err(_) => return string_to_c("{}"),
    };

    // Get handler metadata from frozen registry — no lock, NO clone
    let registry = get_frozen_routes();
    let metadata = match registry.handler_metadata.get(handler_name_str) {
        Some(m) => m,
        None => return string_to_c("{}"),
    };

    let return_type = &metadata.return_type;

    // Primitives — fast path with minimal allocation
    match return_type.as_str() {
        "Str" => {
            let cstr = unsafe { std::ffi::CStr::from_ptr(struct_ptr as *const c_char) };
            let s = cstr.to_bytes();
            let mut buf = Vec::with_capacity(s.len() + 2);
            buf.push(b'"');
            json_escape_bytes_into(&mut buf, s);
            buf.push(b'"');
            return bytes_to_c(&buf);
        }
        "Int" => {
            let i = unsafe { *(struct_ptr as *const i64) };
            let mut ibuf = itoa::Buffer::new();
            let formatted = ibuf.format(i);
            return string_to_c(formatted);
        }
        "Float" => {
            let f = unsafe { *(struct_ptr as *const f64) };
            let mut fbuf = ryu::Buffer::new();
            let formatted = fbuf.format(f);
            return string_to_c(formatted);
        }
        "Bool" => {
            let b = unsafe { *(struct_ptr as *const i8) != 0 };
            return string_to_c(if b { "true" } else { "false" });
        }
        "Void" => return string_to_c("{}"),
        _ => {}
    }

    // Array check: e.g. [Post] from db.raw() returns JSON string passthrough
    if return_type.starts_with('[') && return_type.ends_with(']') {
        if let Some(json_str) = try_read_as_json_string(struct_ptr) {
            let elem_type = &return_type[1..return_type.len() - 1];
            let filtered =
                filter_response_json_by_layout(&json_str, elem_type, &metadata.struct_layouts);
            return string_to_c(&filtered);
        }
    }

    // Serialize struct directly to buffer — no serde_json::Value intermediary
    let mut buf = Vec::with_capacity(128);
    write_struct_to_buf(
        &mut buf,
        struct_ptr as *const u8,
        return_type,
        &metadata.struct_layouts,
    );
    bytes_to_c(&buf)
}

/// Filter a JSON string by removing fields with @writeOnly or @internal decorators.
/// Uses the struct layout from handler metadata to determine which fields to strip.
/// Works for both JSON objects and arrays of objects.
pub(crate) fn filter_response_json_by_layout(
    json_str: &str,
    struct_name: &str,
    struct_layouts: &HashMap<String, serde_json::Value>,
) -> String {
    // Get the list of fields to exclude from the struct layout
    let fields_to_exclude: Vec<String> = struct_layouts
        .get(struct_name)
        .and_then(|layout| layout.get("fields"))
        .and_then(|f| f.as_array())
        .map(|fields| {
            fields
                .iter()
                .filter_map(|field| {
                    let obj = field.as_object()?;
                    let name = obj.get("name")?.as_str()?;
                    let decorators = obj.get("decorators")?.as_array()?;
                    if decorators.iter().any(|d| {
                        d.as_str()
                            .map_or(false, |s| s == "writeOnly" || s == "internal")
                    }) {
                        Some(name.to_string())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // No fields to exclude — return as-is
    if fields_to_exclude.is_empty() {
        return json_str.to_string();
    }

    // Parse, filter, re-serialize
    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(mut value) => {
            strip_fields_recursive(&mut value, &fields_to_exclude);
            value.to_string()
        }
        Err(_) => json_str.to_string(), // Can't parse — return as-is
    }
}

/// Recursively strip excluded fields from a JSON value (object or array of objects).
/// Handles both PascalCase and snake_case keys by using normalized comparison.
fn strip_fields_recursive(value: &mut serde_json::Value, fields_to_exclude: &[String]) {
    match value {
        serde_json::Value::Object(map) => {
            // Collect keys to remove (matching by normalized name)
            let keys_to_remove: Vec<String> = map
                .keys()
                .filter(|key| {
                    fields_to_exclude
                        .iter()
                        .any(|excl| crate::metadata::field_names_match(key, excl))
                })
                .cloned()
                .collect();
            for key in keys_to_remove {
                map.remove(&key);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                strip_fields_recursive(item, fields_to_exclude);
            }
        }
        _ => {}
    }
}

/// Try to read a pointer as a JSON string. Returns Some(json) if successful, None otherwise.
/// This is used to detect when db.raw() returns a JSON string that should be passed through.
pub(crate) fn try_read_as_json_string(ptr: *const c_void) -> Option<String> {
    if ptr.is_null() {
        return None;
    }

    unsafe {
        // First, check if this looks like a valid C string pointer
        // A valid JSON array from db.raw() starts with '['
        let byte_ptr = ptr as *const u8;

        // Safety check: try to read the first byte
        // If the pointer is to a struct with header, the first bytes would be
        // length/capacity (integers), not a printable character like '['
        let first_byte = *byte_ptr;

        // JSON arrays start with '[', JSON objects start with '{'
        // These are the only valid starts for db.raw() results
        if first_byte == b'[' || first_byte == b'{' {
            // This looks like it could be a JSON string, try to read it
            let c_str = std::ffi::CStr::from_ptr(ptr as *const c_char);
            if let Ok(s) = c_str.to_str() {
                // Validate that it's actually valid JSON
                if serde_json::from_str::<serde_json::Value>(s).is_ok() {
                    return Some(s.to_string());
                }
            }
        }
    }

    None
}

/// Allocate a C string directly from a byte buffer (no intermediate String).
/// Uses doo_alloc_string-compatible allocation (libc::malloc).
#[inline]
fn bytes_to_c(buf: &[u8]) -> *const c_char {
    unsafe {
        let ptr = libc::malloc(buf.len() + 1) as *mut u8;
        if ptr.is_null() {
            return std::ptr::null();
        }
        std::ptr::copy_nonoverlapping(buf.as_ptr(), ptr, buf.len());
        *ptr.add(buf.len()) = 0; // null terminator
        ptr as *const c_char
    }
}

/// Escape a byte slice as JSON string content into a buffer (no allocation).
#[inline]
fn json_escape_bytes_into(buf: &mut Vec<u8>, bytes: &[u8]) {
    for &b in bytes {
        match b {
            b'"' => buf.extend_from_slice(b"\\\""),
            b'\\' => buf.extend_from_slice(b"\\\\"),
            b'\n' => buf.extend_from_slice(b"\\n"),
            b'\r' => buf.extend_from_slice(b"\\r"),
            b'\t' => buf.extend_from_slice(b"\\t"),
            b if b < 0x20 => {
                // Control characters as \u00XX
                buf.extend_from_slice(b"\\u00");
                let hi = b >> 4;
                let lo = b & 0xf;
                buf.push(if hi < 10 { b'0' + hi } else { b'a' + hi - 10 });
                buf.push(if lo < 10 { b'0' + lo } else { b'a' + lo - 10 });
            }
            _ => buf.push(b),
        }
    }
}

/// Write a struct to a buffer as JSON — zero serde_json::Value allocation.
/// Reads field layout from pre-computed metadata and writes directly.
pub(crate) fn write_struct_to_buf(
    buf: &mut Vec<u8>,
    struct_ptr: *const u8,
    struct_name: &str,
    struct_layouts: &HashMap<String, serde_json::Value>,
) {
    if struct_ptr.is_null() {
        buf.extend_from_slice(b"null");
        return;
    }

    let layout = match struct_layouts.get(struct_name) {
        Some(l) => l,
        None => {
            buf.extend_from_slice(b"null");
            return;
        }
    };

    let fields = match layout.get("fields").and_then(|f| f.as_array()) {
        Some(f) => f,
        None => {
            buf.extend_from_slice(b"null");
            return;
        }
    };

    buf.push(b'{');
    let mut first = true;

    for field in fields {
        let field_obj = match field.as_object() {
            Some(obj) => obj,
            None => continue,
        };

        let field_name = match field_obj.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => continue,
        };

        // Skip @writeOnly / @internal fields
        if let Some(decorators) = field_obj.get("decorators").and_then(|v| v.as_array()) {
            if decorators.iter().any(|d| {
                d.as_str()
                    .map_or(false, |s| s == "writeOnly" || s == "internal")
            }) {
                continue;
            }
        }

        let field_type = match field_obj.get("type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => continue,
        };

        let offset = match field_obj.get("offset").and_then(|v| v.as_u64()) {
            Some(o) => o as usize,
            None => continue,
        };

        if !first {
            buf.push(b',');
        }
        first = false;

        // Write key
        buf.push(b'"');
        buf.extend_from_slice(field_name.as_bytes());
        buf.push(b'"');
        buf.push(b':');

        // Write value
        unsafe {
            let field_ptr = struct_ptr.add(offset);
            write_value_to_buf(buf, field_ptr, field_type, struct_layouts);
        }
    }

    buf.push(b'}');
}

/// Write a single value to the buffer based on its type.
#[inline]
unsafe fn write_value_to_buf(
    buf: &mut Vec<u8>,
    field_ptr: *const u8,
    field_type: &str,
    struct_layouts: &HashMap<String, serde_json::Value>,
) {
    match field_type {
        "Str" => {
            let str_ptr = *(field_ptr as *const *const c_char);
            if str_ptr.is_null() {
                buf.extend_from_slice(b"\"\"");
            } else {
                let cstr = std::ffi::CStr::from_ptr(str_ptr);
                let bytes = cstr.to_bytes();
                buf.push(b'"');
                json_escape_bytes_into(buf, bytes);
                buf.push(b'"');
            }
        }
        "Int" => {
            let i = *(field_ptr as *const i64);
            let mut tmp = itoa::Buffer::new();
            buf.extend_from_slice(tmp.format(i).as_bytes());
        }
        "Float" => {
            let f = *(field_ptr as *const f64);
            let mut tmp = ryu::Buffer::new();
            buf.extend_from_slice(tmp.format(f).as_bytes());
        }
        "Bool" => {
            let b = *(field_ptr as *const i8) != 0;
            buf.extend_from_slice(if b { b"true" } else { b"false" });
        }
        t if t.starts_with('[') && t.ends_with(']') => {
            let arr_data = *(field_ptr as *const *const u8);
            if arr_data.is_null() {
                buf.extend_from_slice(b"[]");
            } else {
                let elem_type = &t[1..t.len() - 1];
                write_array_to_buf(buf, arr_data, elem_type, struct_layouts);
            }
        }
        _ if struct_layouts.contains_key(field_type) => {
            let nested_ptr = *(field_ptr as *const *const u8);
            write_struct_to_buf(buf, nested_ptr, field_type, struct_layouts);
        }
        _ => buf.extend_from_slice(b"null"),
    }
}

/// Write an array to the buffer as JSON.
pub(crate) fn write_array_to_buf(
    buf: &mut Vec<u8>,
    arr_data_ptr: *const u8,
    elem_type: &str,
    struct_layouts: &HashMap<String, serde_json::Value>,
) {
    if arr_data_ptr.is_null() {
        buf.extend_from_slice(b"[]");
        return;
    }

    unsafe {
        let header_ptr = arr_data_ptr.offset(-16);
        let len = *(header_ptr as *const i64) as usize;

        if len == 0 {
            buf.extend_from_slice(b"[]");
            return;
        }

        let elem_size: usize = match elem_type {
            "Str" | "Int" | "Float" => 8,
            "Bool" => 1,
            _ if struct_layouts.contains_key(elem_type) => 8,
            _ => 8,
        };

        buf.push(b'[');
        for i in 0..len {
            if i > 0 {
                buf.push(b',');
            }
            let elem_ptr = arr_data_ptr.add(i * elem_size);
            write_value_to_buf(buf, elem_ptr, elem_type, struct_layouts);
        }
        buf.push(b']');
    }
}

/// Recursively serialize a struct to JSON value (kept for filter_response_json_by_layout compatibility).
pub(crate) fn serialize_struct_recursive(
    struct_ptr: *const u8,
    struct_name: &str,
    struct_layouts: &HashMap<String, serde_json::Value>,
) -> serde_json::Value {
    if struct_ptr.is_null() {
        return serde_json::Value::Null;
    }

    let layout = match struct_layouts.get(struct_name) {
        Some(l) => l,
        None => return serde_json::Value::Null,
    };

    let fields = match layout.get("fields").and_then(|f| f.as_array()) {
        Some(f) => f,
        None => return serde_json::Value::Null,
    };

    let mut json_obj = serde_json::Map::new();

    for field in fields {
        let field_obj = match field.as_object() {
            Some(obj) => obj,
            None => continue,
        };

        let field_name = match field_obj
            .get("name")
            .and_then(|v: &serde_json::Value| v.as_str())
        {
            Some(n) => n,
            None => continue,
        };

        // Skip fields with @writeOnly or @internal decorators — they must not appear in responses
        if let Some(decorators) = field_obj.get("decorators").and_then(|v| v.as_array()) {
            if decorators.iter().any(|d| {
                d.as_str()
                    .map_or(false, |s| s == "writeOnly" || s == "internal")
            }) {
                continue;
            }
        }

        let field_type = match field_obj
            .get("type")
            .and_then(|v: &serde_json::Value| v.as_str())
        {
            Some(t) => t,
            None => continue,
        };

        // Use pre-computed offset from metadata (critical for correct struct layout)
        let offset = match field_obj.get("offset").and_then(|v| v.as_u64()) {
            Some(o) => o as usize,
            None => continue, // Skip fields without offset
        };

        unsafe {
            let field_ptr = struct_ptr.add(offset);
            let field_value = match field_type {
                "Str" => {
                    let str_ptr = *(field_ptr as *const *const c_char);
                    let s = if str_ptr.is_null() {
                        String::new()
                    } else {
                        c_to_string(str_ptr)
                    };
                    serde_json::Value::String(s)
                }
                "Int" => {
                    let i = *(field_ptr as *const i64);
                    serde_json::json!(i)
                }
                "Float" => {
                    let f = *(field_ptr as *const f64);
                    serde_json::json!(f)
                }
                "Bool" => {
                    let b = *(field_ptr as *const i8) != 0;
                    serde_json::json!(b)
                }
                t if t.starts_with("[") && t.ends_with("]") => {
                    // Array - struct stores pointer to data section
                    let arr_data = *(field_ptr as *const *const u8);
                    if arr_data.is_null() {
                        serde_json::Value::Array(vec![])
                    } else {
                        let elem_type = &t[1..t.len() - 1];
                        serialize_array(arr_data, elem_type, struct_layouts)
                    }
                }
                _ if struct_layouts.contains_key(field_type) => {
                    // Nested struct - pointer to struct
                    let nested_ptr = *(field_ptr as *const *const u8);
                    serialize_struct_recursive(nested_ptr, field_type, struct_layouts)
                }
                _ => serde_json::Value::Null,
            };
            json_obj.insert(field_name.to_string(), field_value);
        }
    }

    serde_json::Value::Object(json_obj)
}

/// Align a value up to the given alignment
#[allow(dead_code)]
pub(crate) fn align_up(offset: usize, align: usize) -> usize {
    if align == 0 {
        return offset;
    }
    (offset + align - 1) & !(align - 1)
}

/// Get the size and alignment for a type
#[allow(dead_code)]
pub(crate) fn get_type_size_align(
    type_name: &str,
    struct_layouts: &HashMap<String, serde_json::Value>,
) -> (usize, usize) {
    match type_name {
        "Str" => (8, 8),                                       // pointer
        "Int" => (8, 8),                                       // i64
        "Float" => (8, 8),                                     // double
        "Bool" => (1, 1),                                      // i1/i8
        t if t.starts_with("[") && t.ends_with("]") => (8, 8), // pointer to array data
        _ if struct_layouts.contains_key(type_name) => (8, 8), // pointer to struct
        _ => (8, 8),                                           // default to pointer size
    }
}

/// Serialize array to JSON (legacy — delegates to buffer writer for new code paths).
pub(crate) fn serialize_array(
    arr_data_ptr: *const u8,
    elem_type: &str,
    struct_layouts: &HashMap<String, serde_json::Value>,
) -> serde_json::Value {
    if arr_data_ptr.is_null() {
        return serde_json::Value::Array(vec![]);
    }

    // Use buffer writer and parse back — avoids duplicating logic
    let mut buf = Vec::with_capacity(256);
    write_array_to_buf(&mut buf, arr_data_ptr, elem_type, struct_layouts);
    // Safety: write_array_to_buf produces valid UTF-8 JSON
    let json_str = unsafe { std::str::from_utf8_unchecked(&buf) };
    serde_json::from_str(json_str).unwrap_or(serde_json::Value::Array(vec![]))
}
