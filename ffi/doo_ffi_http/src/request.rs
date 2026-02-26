//! Request Helpers
//!
//! Functions for extracting data from HTTP requests:
//! query parameters, path parameters, headers, typed parameter extraction,
//! struct population from request body/params/query, and validated body extraction.

use std::collections::HashMap;
use std::ffi::c_void;
use std::os::raw::c_char;

use doo_ffi_core::{ffi_safe_cstr, ffi_safe_f64, ffi_safe_i32, ffi_safe_i64};

use crate::error::*;
use crate::helpers::{
    c_to_string, clear_last_error, get_current_request_path, set_current_request_path,
    set_last_error, string_to_c,
};
use crate::metadata::get_struct_metadata;
use crate::router::get_frozen_routes;
use crate::types::*;
use crate::validation::validate_decorator;

// ============================================================================
// REQUEST HELPERS
// ============================================================================

#[no_mangle]
pub extern "C" fn doo_http_req_query(req: *const DooRequest, key: *const c_char) -> *const c_char {
    ffi_safe_cstr!({
        if req.is_null() {
            return std::ptr::null();
        }
        unsafe {
            let query_map = (*req).query as *const HashMap<String, String>;
            if query_map.is_null() {
                return std::ptr::null();
            }
            let key_str = c_to_string(key);
            (*query_map)
                .get(&key_str)
                .map(|v| string_to_c(v))
                .unwrap_or(std::ptr::null())
        }
    })
}

#[no_mangle]
pub extern "C" fn doo_http_req_param(req: *const DooRequest, key: *const c_char) -> *const c_char {
    ffi_safe_cstr!({
        if req.is_null() {
            return std::ptr::null();
        }
        unsafe {
            // params is now stored as a JSON string pointer, not HashMap
            let params_json = (*req).params as *const c_char;
            if params_json.is_null() {
                return std::ptr::null();
            }
            let key_str = c_to_string(key);
            let json_str = match std::ffi::CStr::from_ptr(params_json).to_str() {
                Ok(s) => s,
                Err(_) => return std::ptr::null(),
            };
            // Parse JSON and extract field
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) {
                if let Some(v) = value.get(&key_str) {
                    if let Some(s) = v.as_str() {
                        // String value — return directly
                        return string_to_c(s);
                    }
                    // Non-string value (number, bool) — convert to string representation
                    // e.g. route param "id":2 → "2" for req.param("id")
                    if !v.is_null() {
                        return string_to_c(&v.to_string());
                    }
                }
            }
            std::ptr::null()
        }
    })
}

#[no_mangle]
pub extern "C" fn doo_http_req_header(req: *const DooRequest, key: *const c_char) -> *const c_char {
    ffi_safe_cstr!({
        if req.is_null() {
            return std::ptr::null();
        }
        unsafe {
            let headers_map = (*req).headers as *const HashMap<String, String>;
            if headers_map.is_null() {
                return std::ptr::null();
            }
            let key_str = c_to_string(key).to_lowercase();
            (*headers_map)
                .get(&key_str)
                .map(|v| string_to_c(v))
                .unwrap_or(string_to_c(""))
        }
    })
}

#[no_mangle]
pub extern "C" fn doohttp_extract_param_int(
    req: *const DooRequest,
    param_name: *const c_char,
) -> i64 {
    ffi_safe_i64!({
        clear_last_error();
        if req.is_null() || param_name.is_null() {
            return 0;
        }
        let value_ptr = doo_http_req_param(req, param_name);
        if value_ptr.is_null() {
            return 0;
        }
        let value_str = c_to_string(value_ptr);
        match value_str.parse::<i64>() {
            Ok(v) => v,
            Err(_) => {
                let param_name_str = c_to_string(param_name);
                let path = get_current_request_path();
                let param = ParameterError::new(&param_name_str)
                    .with_expected("Int")
                    .with_received(&value_str);
                let err = Rfc7807Error::invalid_path_param(&path, param);
                set_last_error(400, err.to_json());
                0
            }
        }
    })
}

#[no_mangle]
pub extern "C" fn doohttp_extract_param_float(
    req: *const DooRequest,
    param_name: *const c_char,
) -> f64 {
    ffi_safe_f64!({
        clear_last_error();
        if req.is_null() || param_name.is_null() {
            return 0.0;
        }
        let value_ptr = doo_http_req_param(req, param_name);
        if value_ptr.is_null() {
            return 0.0;
        }
        let value_str = c_to_string(value_ptr);
        match value_str.parse::<f64>() {
            Ok(v) => v,
            Err(_) => {
                let param_name_str = c_to_string(param_name);
                let path = get_current_request_path();
                let param = ParameterError::new(&param_name_str)
                    .with_expected("Float")
                    .with_received(&value_str);
                let err = Rfc7807Error::invalid_path_param(&path, param);
                set_last_error(400, err.to_json());
                0.0
            }
        }
    })
}

/// Extract typed path parameter from request with type validation
/// Returns: converted value as C string (caller must free), or null on error
#[no_mangle]
pub extern "C" fn doohttp_extract_param_typed(
    req: *const DooRequest,
    param_name: *const c_char,
    param_type: *const c_char,
) -> *const c_char {
    ffi_safe_cstr!({
        clear_last_error();
        if req.is_null() || param_name.is_null() || param_type.is_null() {
            return std::ptr::null();
        }

        let param_name_str = c_to_string(param_name);
        let param_type_str = c_to_string(param_type);
        let value_ptr = doo_http_req_param(req, param_name);

        if value_ptr.is_null() {
            let path = get_current_request_path();
            let param = ParameterError::new(&param_name_str)
                .with_message(format!("Path parameter '{}' is required", param_name_str));
            let err = Rfc7807Error::missing_path_param(&path, param);
            set_last_error(400, err.to_json());
            return std::ptr::null();
        }

        let value = c_to_string(value_ptr);

        // Type conversion validation
        match param_type_str.as_str() {
            "Int" => {
                if value.parse::<i64>().is_ok() {
                    string_to_c(&value)
                } else {
                    let path = get_current_request_path();
                    let param = ParameterError::new(&param_name_str)
                        .with_expected("Int")
                        .with_received(&value);
                    let err = Rfc7807Error::invalid_path_param(&path, param);
                    set_last_error(400, err.to_json());
                    std::ptr::null()
                }
            }
            "Float" => {
                if value.parse::<f64>().is_ok() {
                    string_to_c(&value)
                } else {
                    let path = get_current_request_path();
                    let param = ParameterError::new(&param_name_str)
                        .with_expected("Float")
                        .with_received(&value);
                    let err = Rfc7807Error::invalid_path_param(&path, param);
                    set_last_error(400, err.to_json());
                    std::ptr::null()
                }
            }
            "Bool" => {
                if value == "true" || value == "false" {
                    string_to_c(&value)
                } else {
                    let path = get_current_request_path();
                    let param = ParameterError::new(&param_name_str)
                        .with_expected("Bool")
                        .with_received(&value);
                    let err = Rfc7807Error::invalid_path_param(&path, param);
                    set_last_error(400, err.to_json());
                    std::ptr::null()
                }
            }
            _ => string_to_c(&value), // String or other types - return as-is
        }
    })
}

// ============================================================================
// STRUCT POPULATION FROM REQUEST
// ============================================================================

/// Populate struct from request data with JSON parsing and validation
///
/// Parameters:
/// - request_ptr: Pointer to DooRequest
/// - struct_ptr: Pointer to allocated struct to populate
/// - source_type: 0=body (JSON), 1=params, 2=query
/// - handler_name: Name of handler (used to get metadata)
///
/// Returns: 0 on success, error code on failure
#[no_mangle]
pub extern "C" fn doohttp_populate_struct_from_request(
    request_ptr: *const c_void,
    struct_ptr: *mut c_void,
    _source_type: i32,
    handler_name: *const c_char,
) -> i32 {
    ffi_safe_i32!({
        clear_last_error();

        // Request pointer is required, but struct_ptr can be null for validation-only mode
        if request_ptr.is_null() {
            return -1;
        }

        if handler_name.is_null() {
            return 0; // No handler name, can't look up metadata
        }

        // Track if we're in validation-only mode (struct_ptr is null)
        let _validation_only = struct_ptr.is_null();

        let handler_name_str = c_to_string(handler_name);

        // Cast request to get fields
        let request = unsafe { &*(request_ptr as *const DooRequest) };
        let path_str = c_to_string(request.path);
        set_current_request_path(&path_str);

        // Get handler metadata from frozen registry — no lock
        let registry = get_frozen_routes();
        let metadata = registry.handler_metadata.get(&handler_name_str).cloned();

        let metadata = match metadata {
            Some(m) => m,
            None => return 0, // No metadata, skip validation
        };

        // Look up the expected type for a field from struct metadata.
        // Returns "Str" if unknown.
        fn get_field_type(key: &str, metadata: &HandlerMetadata) -> String {
            for (_struct_name, layout) in &metadata.struct_layouts {
                if let Some(fields) = layout.get("fields").and_then(|f| f.as_array()) {
                    for field in fields {
                        if let Some(obj) = field.as_object() {
                            let name = obj.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            if name == key {
                                return obj
                                    .get("type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Str")
                                    .to_string();
                            }
                        }
                    }
                }
            }
            "Str".to_string()
        }

        // Helper to coerce string values to typed JSON based on struct layout from metadata
        // Searches ALL struct layouts to find the field type (for multi-param handlers)
        fn coerce_string_to_typed_value(
            key: &str,
            value: &str,
            metadata: &HandlerMetadata,
        ) -> serde_json::Value {
            // Search ALL struct layouts to find the expected type for this field
            for (_struct_name, layout) in &metadata.struct_layouts {
                if let Some(fields) = layout.get("fields").and_then(|f| f.as_array()) {
                    for field in fields {
                        if let Some(obj) = field.as_object() {
                            let field_name = obj
                                .get("name")
                                .and_then(|v: &serde_json::Value| v.as_str())
                                .unwrap_or("");
                            let field_type = obj
                                .get("type")
                                .and_then(|v: &serde_json::Value| v.as_str())
                                .unwrap_or("Str");
                            if field_name == key {
                                return match field_type {
                                    "Int" => value
                                        .parse::<i64>()
                                        .map(serde_json::Value::from)
                                        .unwrap_or_else(|_| {
                                            serde_json::Value::String(value.to_string())
                                        }),
                                    "Float" => value
                                        .parse::<f64>()
                                        .map(|f| serde_json::Value::from(f))
                                        .unwrap_or_else(|_| {
                                            serde_json::Value::String(value.to_string())
                                        }),
                                    "Bool" => {
                                        let lower = value.to_lowercase();
                                        match lower.as_str() {
                                            "true" | "1" => serde_json::Value::Bool(true),
                                            "false" | "0" => serde_json::Value::Bool(false),
                                            _ => serde_json::Value::String(value.to_string()),
                                        }
                                    }
                                    _ => serde_json::Value::String(value.to_string()),
                                };
                            }
                        }
                    }
                }
            }
            // Default: keep as string
            serde_json::Value::String(value.to_string())
        }

        // Build source_data by merging ALL sources for multi-param handler support
        // Priority: path params > query params > body (later sources don't override earlier)
        let mut source_data: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();

        // 1. Start with path params (highest priority for path-based fields like userId)
        // NOTE: params is stored as a JSON string (e.g., '{"authorId":"1"}') by the server,
        // NOT as a HashMap pointer. This is consistent with how codegen reads it via doo_json_get_field.
        if !request.params.is_null() {
            let params_json_ptr = request.params as *const c_char;
            let params_json_str = c_to_string(params_json_ptr);
            if !params_json_str.is_empty() {
                if let Ok(serde_json::Value::Object(params_obj)) =
                    serde_json::from_str::<serde_json::Value>(&params_json_str)
                {
                    for (k, v) in params_obj {
                        // Path params may be pre-typed by the server (numbers, bools)
                        // or strings that need coercion to the struct field type.
                        if v.is_string() {
                            let value_str = v.as_str().unwrap_or_default();
                            source_data.insert(
                                k.clone(),
                                coerce_string_to_typed_value(&k, value_str, &metadata),
                            );
                        } else {
                            // Already typed (number, bool) by server.
                            // But if the struct field expects Str, convert to string.
                            let expected_type = get_field_type(&k, &metadata);
                            if expected_type == "Str" {
                                // Field expects string — stringify the value
                                let s = match &v {
                                    serde_json::Value::Bool(b) => b.to_string(),
                                    serde_json::Value::Number(n) => n.to_string(),
                                    other => other.to_string(),
                                };
                                source_data.insert(k, serde_json::Value::String(s));
                            } else {
                                source_data.insert(k, v);
                            }
                        }
                    }
                }
            }
        }

        // 2. Add query params (for GET requests with query strings)
        if !request.query.is_null() {
            let query_map = unsafe { &*(request.query as *const HashMap<String, String>) };
            for (k, v) in query_map.iter() {
                // Don't override path params
                if !source_data.contains_key(k as &str) {
                    source_data.insert(k.clone(), coerce_string_to_typed_value(k, v, &metadata));
                }
            }
        }

        // 3. Merge body JSON (for POST/PUT/PATCH with JSON body)
        if !request.body.is_null() {
            let body_str = c_to_string(request.body);
            if !body_str.is_empty() {
                if let Ok(serde_json::Value::Object(body_obj)) =
                    serde_json::from_str::<serde_json::Value>(&body_str)
                {
                    for (k, v) in body_obj {
                        // Don't override path params or query params
                        if !source_data.contains_key(&k) {
                            source_data.insert(k, v);
                        }
                    }
                }
            }
        }

        // Check if first param type is a special raw request type - skip validation
        let first_param = metadata.param_types.first().cloned().unwrap_or_default();
        if first_param == "Request" || first_param == "DooRequest" {
            return 0;
        }

        // Skip validation if no param types defined
        if metadata.param_types.is_empty() {
            return 0;
        }

        // Recursive validation helper
        fn validate_struct_fields(
            source_data: &serde_json::Map<String, serde_json::Value>,
            struct_name: &str,
            field_prefix: &str,
            metadata: &HandlerMetadata,
            field_errors: &mut HashMap<String, FieldError>,
        ) {
            let struct_layout = match metadata.struct_layouts.get(struct_name) {
                Some(layout) => layout,
                None => return, // Unknown struct, skip validation
            };

            let fields = match struct_layout.get("fields").and_then(|f| f.as_array()) {
                Some(f) => f,
                None => return,
            };

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

                // Skip fields with @readOnly or @internal decorators — they are not accepted from requests
                if let Some(decorators) = field_obj.get("decorators").and_then(|v| v.as_array()) {
                    if decorators.iter().any(|d| {
                        d.as_str()
                            .map_or(false, |s| s == "readOnly" || s == "internal")
                    }) {
                        continue;
                    }
                }

                // Build full field path for error messages
                let full_field_name = if field_prefix.is_empty() {
                    field_name.to_string()
                } else {
                    format!("{}.{}", field_prefix, field_name)
                };

                // Check if field is missing - Optional fields are allowed to be missing
                if !source_data.contains_key(field_name)
                    && !(field_type.starts_with("Optional(") && field_type.ends_with(')'))
                {
                    let err = FieldError::required();
                    field_errors.insert(full_field_name, err);
                    continue;
                }

                if let Some(value) = source_data.get(field_name) {
                    // Validate array element types
                    if field_type.starts_with("[") && field_type.ends_with("]") {
                        let elem_type = &field_type[1..field_type.len() - 1];

                        if let serde_json::Value::Array(arr) = value {
                            for (_i, elem) in arr.iter().enumerate() {
                                let elem_valid = match elem_type {
                                    "Int" => elem.is_i64() || elem.is_u64(),
                                    "Float" => elem.is_f64(),
                                    "Bool" => elem.is_boolean(),
                                    "Str" | "String" => elem.is_string(),
                                    _ => true, // Nested struct arrays - skip for now
                                };

                                if !elem_valid {
                                    let received_type = match elem {
                                        serde_json::Value::Null => "null",
                                        serde_json::Value::Bool(_) => "Bool",
                                        serde_json::Value::Number(n) => {
                                            if n.is_i64() || n.is_u64() {
                                                "Int"
                                            } else {
                                                "Float"
                                            }
                                        }
                                        serde_json::Value::String(_) => "Str",
                                        serde_json::Value::Array(_) => "Array",
                                        serde_json::Value::Object(_) => "Object",
                                    };

                                    let elem_value_str = match elem {
                                        serde_json::Value::String(s) => s.clone(),
                                        serde_json::Value::Null => "null".to_string(),
                                        serde_json::Value::Bool(b) => b.to_string(),
                                        serde_json::Value::Number(n) => n.to_string(),
                                        _ => elem.to_string(),
                                    };
                                    let err = FieldError::type_mismatch(elem_type, received_type)
                                        .with_value(elem_value_str);
                                    field_errors.entry(full_field_name.clone()).or_insert(err);
                                }
                            }
                        } else {
                            let received_type = match value {
                                serde_json::Value::Null => "null",
                                serde_json::Value::Bool(_) => "Bool",
                                serde_json::Value::Number(_) => "Number",
                                serde_json::Value::String(_) => "Str",
                                serde_json::Value::Object(_) => "Object",
                                serde_json::Value::Array(_) => "Array",
                            };
                            let value_str = match value {
                                serde_json::Value::String(s) => s.clone(),
                                serde_json::Value::Null => "null".to_string(),
                                serde_json::Value::Bool(b) => b.to_string(),
                                serde_json::Value::Number(n) => n.to_string(),
                                _ => value.to_string(),
                            };
                            let err = FieldError::type_mismatch(field_type, received_type)
                                .with_value(value_str);
                            field_errors.insert(full_field_name, err);
                        }
                    } else {
                        // Check primitive types first
                        let is_primitive =
                            matches!(field_type, "Int" | "Float" | "Bool" | "Str" | "String");

                        if is_primitive {
                            let type_valid = match field_type {
                                "Int" => value.is_i64() || value.is_u64(),
                                "Float" => value.is_f64(),
                                "Bool" => value.is_boolean(),
                                "Str" | "String" => {
                                    // Str must be a string and not empty
                                    match value.as_str() {
                                        Some(s) => !s.is_empty(),
                                        None => false,
                                    }
                                }
                                _ => true,
                            };

                            if !type_valid {
                                let received_type = match value {
                                    serde_json::Value::Null => "null",
                                    serde_json::Value::Bool(_) => "Bool",
                                    serde_json::Value::Number(n) => {
                                        if n.is_i64() || n.is_u64() {
                                            "Int"
                                        } else {
                                            "Float"
                                        }
                                    }
                                    serde_json::Value::String(_) => "String",
                                    serde_json::Value::Array(_) => "Array",
                                    serde_json::Value::Object(_) => "Object",
                                };
                                // Get the raw value string for the response
                                let value_str = match value {
                                    serde_json::Value::String(s) => s.clone(),
                                    serde_json::Value::Null => "null".to_string(),
                                    serde_json::Value::Bool(b) => b.to_string(),
                                    serde_json::Value::Number(n) => n.to_string(),
                                    _ => value.to_string(),
                                };
                                let err = FieldError::type_mismatch(field_type, received_type)
                                    .with_value(value_str);
                                field_errors.insert(full_field_name, err);
                            }
                        } else if let Some(variants) = metadata.enum_variants.get(field_type) {
                            // Enum validation - case-insensitive matching
                            let valid = if let Some(s) = value.as_str() {
                                let s_lower = s.to_lowercase();
                                variants.iter().any(|v| v.to_lowercase() == s_lower)
                            } else {
                                false
                            };

                            if !valid {
                                let received_str = match value {
                                    serde_json::Value::String(s) => s.clone(),
                                    serde_json::Value::Null => "null".to_string(),
                                    serde_json::Value::Bool(b) => b.to_string(),
                                    serde_json::Value::Number(n) => n.to_string(),
                                    serde_json::Value::Array(_) => "Array".to_string(),
                                    serde_json::Value::Object(_) => "Object".to_string(),
                                };
                                let err = FieldError::new(format!(
                                    "Must be one of: {}",
                                    variants.join(", ")
                                ))
                                .with_rule(format!("enum:{}", variants.join("|")))
                                .with_value(received_str);
                                field_errors.insert(full_field_name, err);
                            }
                        } else if metadata.struct_layouts.contains_key(field_type) {
                            // Nested struct validation - recurse
                            if let serde_json::Value::Object(nested_obj) = value {
                                validate_struct_fields(
                                    nested_obj,
                                    field_type,
                                    &full_field_name,
                                    metadata,
                                    field_errors,
                                );
                            } else {
                                let received_type = match value {
                                    serde_json::Value::Null => "null",
                                    serde_json::Value::Bool(_) => "Bool",
                                    serde_json::Value::Number(_) => "Number",
                                    serde_json::Value::String(_) => "Str",
                                    serde_json::Value::Array(_) => "Array",
                                    serde_json::Value::Object(_) => "Object",
                                };
                                let value_str = match value {
                                    serde_json::Value::String(s) => s.clone(),
                                    serde_json::Value::Null => "null".to_string(),
                                    serde_json::Value::Bool(b) => b.to_string(),
                                    serde_json::Value::Number(n) => n.to_string(),
                                    _ => value.to_string(),
                                };
                                let err = FieldError::type_mismatch(field_type, received_type)
                                    .with_value(value_str);
                                field_errors.insert(full_field_name, err);
                            }
                        }
                        // Unknown types are skipped
                    }
                }
            }
        }

        // Validate fields for ALL param types (multi-param handler support)
        let mut field_errors: HashMap<String, FieldError> = HashMap::new();
        for param_type in &metadata.param_types {
            // Skip special/injected types — not sourced from request data
            if param_type == "Request" || param_type == "DooRequest" || param_type == "Server" {
                continue;
            }
            validate_struct_fields(&source_data, param_type, "", &metadata, &mut field_errors);
        }

        // Decorator validation: validate @email, @min, @max etc. for fields that
        // passed type validation. Uses StructMetadata registry populated by codegen.
        for param_type in &metadata.param_types {
            if param_type == "Request" || param_type == "DooRequest" || param_type == "Server" {
                continue;
            }
            if let Some(struct_meta) = get_struct_metadata(param_type) {
                for field_meta in &struct_meta.fields {
                    // Skip fields that already have type errors
                    if field_errors.contains_key(&field_meta.name) {
                        continue;
                    }
                    if let Some(value) = source_data.get(&field_meta.name) {
                        for decorator in &field_meta.decorators {
                            if let Err(err_json) =
                                validate_decorator(decorator, &field_meta.name, value, &path_str)
                            {
                                // Parse the RFC 7807 JSON to extract the field error
                                // The validate_decorator returns a full RFC 7807 JSON string
                                // We need to set the error and return 422
                                set_last_error(422, err_json);
                                return 422;
                            }
                        }
                    }
                }
            }
        }

        if !field_errors.is_empty() {
            // Convert to core FieldErrors in a HashMap preserving field names
            let core_fields: HashMap<String, doo_ffi_core::FieldError> = field_errors
                .into_iter()
                .map(|(field, err)| (field.clone(), err.to_core(&field)))
                .collect();

            // Check if any param struct has decorator fields → use 422
            // Structs with decorators use semantic validation (422 Unprocessable Entity)
            // Plain structs use parsing validation (400 Bad Request)
            let has_decorators = metadata.param_types.iter().any(|pt| {
                get_struct_metadata(pt).map_or(false, |sm| {
                    sm.fields.iter().any(|f| !f.decorators.is_empty())
                })
            });

            // Determine detail based on error types
            let has_required = core_fields
                .values()
                .any(|e| e.error.as_deref() == Some("required"));
            let has_type_mismatch = core_fields
                .values()
                .any(|e| e.expected.is_some() && e.received.is_some() && e.rule.is_none());

            let base_status: u16 = if has_decorators { 422 } else { 400 };

            let (status, detail) = if has_required && !has_type_mismatch {
                (base_status, "Required field missing in request body")
            } else if has_type_mismatch && !has_required {
                (base_status, "Type mismatch in request body")
            } else {
                (base_status, "Request body parsing failed")
            };

            let err = doo_ffi_core::Rfc7807Error::new(status, detail)
                .with_instance(path_str)
                .with_fields(core_fields)
                .with_type("validation_error");
            set_last_error(status as i32, err.to_json());
            return status as i32;
        }

        // Always update request.body with the merged/typed JSON
        // so that codegen's body parsing works correctly for all sources
        if !source_data.is_empty() {
            let json_obj = serde_json::Value::Object(source_data);
            if let Ok(json_str) = serde_json::to_string(&json_obj) {
                let request_mut = unsafe { &mut *(request_ptr as *mut DooRequest) };
                request_mut.body = string_to_c(&json_str);
            }
        }

        0 // Success
    })
}

/// Parse request body JSON and return a raw pointer suitable for passing to user function.
///
/// This function:
/// 1. Extracts the JSON body from the request
/// 2. Validates it against the handler's expected struct type
/// 3. Returns the raw JSON string pointer (user function receives this)
///
/// The user function's wrapper in codegen will handle the actual struct parsing.
/// This is a simpler approach that just validates and passes through.
///
/// Returns: Pointer to body JSON string, or null on error (check doohttp_last_error_*)
#[no_mangle]
pub extern "C" fn doohttp_get_validated_body(
    request_ptr: *const c_void,
    handler_name: *const c_char,
) -> *const c_char {
    ffi_safe_cstr!({
        clear_last_error();

        if request_ptr.is_null() {
            set_last_error(400, Rfc7807Error::bad_request("Null request").to_json());
            return std::ptr::null();
        }

        let _handler_name_str = if handler_name.is_null() {
            return std::ptr::null();
        } else {
            c_to_string(handler_name)
        };

        let request = unsafe { &*(request_ptr as *const DooRequest) };
        let path_str = c_to_string(request.path);
        set_current_request_path(&path_str);

        // Get body
        if request.body.is_null() {
            set_last_error(
                400,
                bad_request("Missing request body", path_str.clone()).to_json(),
            );
            return std::ptr::null();
        }

        let body_str = c_to_string(request.body);
        if body_str.is_empty() {
            set_last_error(
                400,
                bad_request("Empty request body", path_str.clone()).to_json(),
            );
            return std::ptr::null();
        }

        // Validate body using populate_struct_from_request
        let validate_result = doohttp_populate_struct_from_request(
            request_ptr,
            std::ptr::null_mut(), // validation only
            0,                    // body
            handler_name,
        );

        if validate_result != 0 {
            // Error already set by populate_struct_from_request
            return std::ptr::null();
        }

        // Return the body string (the codegen's JSON parser will use this)
        string_to_c(&body_str)
    })
}
