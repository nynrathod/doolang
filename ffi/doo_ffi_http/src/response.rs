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
    c_to_string, get_current_request_path, get_last_error_json, get_last_error_status,
    string_to_c, CONTENT_TYPE_JSON,
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
#[no_mangle]
pub extern "C" fn doohttp_serialize_struct_to_json(
    struct_ptr: *const c_void,
    handler_name: *const c_char,
) -> *const c_char {
    ffi_safe_cstr!({
        if struct_ptr.is_null() || handler_name.is_null() {
            return string_to_c("{}");
        }

        let handler_name_str = c_to_string(handler_name);

        // Get handler metadata from frozen registry — no lock
        let registry = get_frozen_routes();
        let metadata = registry.handler_metadata.get(&handler_name_str).cloned();

        let metadata = match metadata {
            Some(m) => m,
            None => return string_to_c("{}"),
        };

        let return_type = &metadata.return_type;

        // If return type is a primitive, just return it as-is
        if return_type == "Str" {
            let s = c_to_string(struct_ptr as *const c_char);
            return string_to_c(&format!("\"{}\"", s.replace("\"", "\\\"")));
        }
        if return_type == "Int" {
            let i = unsafe { *(struct_ptr as *const i64) };
            return string_to_c(&i.to_string());
        }
        if return_type == "Float" {
            let f = unsafe { *(struct_ptr as *const f64) };
            return string_to_c(&f.to_string());
        }
        if return_type == "Bool" {
            let b = unsafe { *(struct_ptr as *const i8) != 0 };
            return string_to_c(if b { "true" } else { "false" });
        }
        if return_type == "Void" {
            return string_to_c("null");
        }

        // CRITICAL FIX: When return type is an array (e.g., [Post]) but the actual value
        // is a JSON string from db.raw(), detect this and pass through directly.
        // This happens when user writes: let result: [Post] = db.raw("SELECT ...")?;
        // The db.raw() returns a JSON string, not an in-memory struct array.
        if return_type.starts_with('[') && return_type.ends_with(']') {
            // Try to read as a C string first - if it's valid JSON, pass through
            if let Some(json_str) = try_read_as_json_string(struct_ptr) {
                // It's already a valid JSON string, return it directly
                return string_to_c(&json_str);
            }
            // Otherwise, fall through to struct serialization (for actual in-memory arrays)
        }

        // Serialize struct recursively
        let json = serialize_struct_recursive(
            struct_ptr as *const u8,
            return_type,
            &metadata.struct_layouts,
        );

        string_to_c(&json.to_string())
    })
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

/// Recursively serialize a struct to JSON value.
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

/// Serialize array to JSON.
pub(crate) fn serialize_array(
    arr_data_ptr: *const u8,
    elem_type: &str,
    struct_layouts: &HashMap<String, serde_json::Value>,
) -> serde_json::Value {
    if arr_data_ptr.is_null() {
        return serde_json::Value::Array(vec![]);
    }

    // The arr_data_ptr points to the data section
    // The header (len, cap) is 16 bytes before the data
    unsafe {
        let header_ptr = arr_data_ptr.offset(-16);
        let len = *(header_ptr as *const i64) as usize;

        if len == 0 {
            return serde_json::Value::Array(vec![]);
        }

        let mut arr = Vec::with_capacity(len);

        // Get element size for iteration
        let elem_size = match elem_type {
            "Str" => 8,                                       // pointer
            "Int" => 8,                                       // i64
            "Float" => 8,                                     // double
            "Bool" => 1,                                      // i1/i8
            _ if struct_layouts.contains_key(elem_type) => 8, // pointer
            _ => 8,
        };

        for i in 0..len {
            let elem_offset = i * elem_size;
            let elem_ptr = arr_data_ptr.add(elem_offset);

            let elem = match elem_type {
                "Str" => {
                    let str_ptr = *(elem_ptr as *const *const c_char);
                    let s = if str_ptr.is_null() {
                        String::new()
                    } else {
                        c_to_string(str_ptr)
                    };
                    serde_json::Value::String(s)
                }
                "Int" => {
                    let val = *(elem_ptr as *const i64);
                    serde_json::json!(val)
                }
                "Float" => {
                    let val = *(elem_ptr as *const f64);
                    serde_json::json!(val)
                }
                "Bool" => {
                    let val = *(elem_ptr as *const i8) != 0;
                    serde_json::json!(val)
                }
                _ if struct_layouts.contains_key(elem_type) => {
                    // Array of structs (each element is a pointer)
                    let nested_ptr = *(elem_ptr as *const *const u8);
                    serialize_struct_recursive(nested_ptr, elem_type, struct_layouts)
                }
                _ => serde_json::Value::Null,
            };
            arr.push(elem);
        }

        serde_json::Value::Array(arr)
    }
}
