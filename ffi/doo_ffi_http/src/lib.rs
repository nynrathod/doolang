//! doo_ffi_http - Complete HTTP FFI Library
//!
//! Provides all HTTP functionality for Doo applications:
//! - Route registration (GET, POST, PUT, DELETE, PATCH)
//! - Middleware (JWT, CORS, Rate Limiting)
//! - RFC 7807 error responses
//! - Request helpers (params, query, headers)
//! - Server lifecycle

mod error;
mod helpers;
mod middleware;
mod router;
mod server;
mod types;

use std::collections::HashMap;
use std::ffi::c_void;
use std::os::raw::c_char;

pub use error::*;
pub use helpers::*;
pub use middleware::*;
pub use router::*;
pub use types::*;

// ============================================================================
// SERVER LIFECYCLE
// ============================================================================

#[no_mangle]
pub extern "C" fn doo_http_server_new(host_port: *const c_char) -> *mut c_void {
    let host_port_str = if host_port.is_null() {
        ":3000".to_string()
    } else {
        c_to_string(host_port)
    };

    // Parse host:port
    let (host, port) = if let Some(colon) = host_port_str.rfind(':') {
        let h = &host_port_str[..colon];
        let p = host_port_str[colon + 1..].parse().unwrap_or(3000);
        (if h.is_empty() { "127.0.0.1" } else { h }.to_string(), p)
    } else {
        ("127.0.0.1".to_string(), 3000)
    };

    // Allocate server struct
    unsafe {
        let ptr = libc::malloc(16) as *mut u8;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        *(ptr as *mut i32) = port;
        *(ptr.add(8) as *mut *const c_char) = string_to_c(&host);
        ptr as *mut c_void
    }
}

#[no_mangle]
pub extern "C" fn doo_http_listen(server_ptr: *const c_void) -> *mut DooResult {
    let (host, port) = if server_ptr.is_null() {
        ("0.0.0.0".to_string(), 3000)
    } else {
        unsafe {
            let port = *(server_ptr as *const i32);
            let host_ptr = *((server_ptr as *const u8).add(8) as *const *const c_char);
            (c_to_string(host_ptr), port as u16)
        }
    };

    match server::start_server(&host, port as u16) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err_http(500, &e),
    }
}

// ============================================================================
// ROUTE REGISTRATION
// ============================================================================

#[no_mangle]
pub extern "C" fn doo_http_get(
    _server: *const c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    register_route("GET", path, handler_name)
}

#[no_mangle]
pub extern "C" fn doo_http_get_fn(
    _server: *const c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    register_route_fn("GET", path, handler)
}

#[no_mangle]
pub extern "C" fn doo_http_post(
    _server: *const c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    register_route("POST", path, handler_name)
}

#[no_mangle]
pub extern "C" fn doo_http_post_fn(
    _server: *const c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    register_route_fn("POST", path, handler)
}

#[no_mangle]
pub extern "C" fn doo_http_put(
    _server: *const c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    register_route("PUT", path, handler_name)
}

#[no_mangle]
pub extern "C" fn doo_http_put_fn(
    _server: *const c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    register_route_fn("PUT", path, handler)
}

#[no_mangle]
pub extern "C" fn doo_http_delete(
    _server: *const c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    register_route("DELETE", path, handler_name)
}

#[no_mangle]
pub extern "C" fn doo_http_delete_fn(
    _server: *const c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    register_route_fn("DELETE", path, handler)
}

#[no_mangle]
pub extern "C" fn doo_http_patch(
    _server: *const c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    register_route("PATCH", path, handler_name)
}

#[no_mangle]
pub extern "C" fn doo_http_patch_fn(
    _server: *const c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    register_route_fn("PATCH", path, handler)
}

fn register_route(
    method: &str,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    let handler_str = c_to_string(handler_name);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.register_by_name(method, &path_str, &handler_str);
    make_ok_void()
}

fn register_route_fn(method: &str, path: *const c_char, handler: DooHandlerFn) -> *mut DooResult {
    let path_str = c_to_string(path);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.register(method, &path_str, handler);
    make_ok_void()
}

// ============================================================================
// MIDDLEWARE
// ============================================================================

#[no_mangle]
pub extern "C" fn doo_http_use(
    server: *const c_void,
    middleware_name: *const c_char,
) -> *const c_void {
    let mw_str = c_to_string(middleware_name);
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();

    if let Some(&mw_fn) = registry.middleware_handlers.get(&mw_str) {
        registry.add_middleware(mw_fn);
    }
    server
}

#[no_mangle]
pub extern "C" fn doo_http_jwt() -> *const c_char {
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    if !registry.middleware_handlers.contains_key("jwt") {
        registry
            .middleware_handlers
            .insert("jwt".to_string(), jwt_middleware_handler);
    }
    string_to_c("jwt")
}

#[no_mangle]
pub extern "C" fn doo_http_cors(server: *mut c_void) -> *mut c_void {
    let config = CorsConfig::default();
    *get_cors_config().lock().unwrap() = Some(config);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    if !registry.middleware_handlers.contains_key("cors") {
        registry
            .middleware_handlers
            .insert("cors".to_string(), cors_middleware_handler);
    }
    registry.add_middleware(cors_middleware_handler);
    server
}

#[no_mangle]
pub extern "C" fn doo_http_cors_custom(server: *mut c_void, options: *mut c_void) -> *mut c_void {
    // Parse options map for CORS configuration
    let config = if options.is_null() {
        CorsConfig::default()
    } else {
        // Parse origins - comma-separated string
        let origins_ptr = doo_map_get_str(options, "origins");
        let origins = if origins_ptr.is_null() {
            vec!["*".to_string()]
        } else {
            parse_json_string_or_default(origins_ptr, "*")
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        };

        // Parse methods - comma-separated string
        let methods_ptr = doo_map_get_str(options, "methods");
        let methods = if methods_ptr.is_null() {
            vec!["GET", "POST", "PUT", "DELETE", "OPTIONS", "PATCH"]
                .into_iter()
                .map(String::from)
                .collect()
        } else {
            parse_json_string_or_default(methods_ptr, "GET,POST,PUT,DELETE,OPTIONS,PATCH")
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        };

        // Parse headers - comma-separated string
        let headers_ptr = doo_map_get_str(options, "headers");
        let headers = if headers_ptr.is_null() {
            vec!["Content-Type", "Authorization"]
                .into_iter()
                .map(String::from)
                .collect()
        } else {
            parse_json_string_or_default(headers_ptr, "Content-Type,Authorization")
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        };

        // Parse credentials - boolean
        let credentials_ptr = doo_map_get_str(options, "credentials");
        let credentials = parse_json_bool_or_default(credentials_ptr, false);

        // Parse max_age - integer (seconds)
        let max_age_ptr = doo_map_get_str(options, "max_age");
        let max_age = if max_age_ptr.is_null() {
            None
        } else {
            let val = parse_json_i64_or_default(max_age_ptr, 0);
            if val > 0 {
                Some(val as i32)
            } else {
                None
            }
        };

        CorsConfig {
            origins,
            methods,
            headers,
            credentials,
            max_age,
        }
    };

    *get_cors_config().lock().unwrap() = Some(config);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    if !registry.middleware_handlers.contains_key("cors") {
        registry
            .middleware_handlers
            .insert("cors".to_string(), cors_middleware_handler);
    }
    registry.add_middleware(cors_middleware_handler);
    server
}

#[no_mangle]
pub extern "C" fn doo_http_ratelimit(server: *mut c_void) -> *mut c_void {
    let config = RateLimitConfig::default();
    *get_ratelimit_config().lock().unwrap() = Some(config);

    // Clear state for fresh start
    get_ratelimit_state().lock().unwrap().clear();

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    if !registry.middleware_handlers.contains_key("ratelimit") {
        registry
            .middleware_handlers
            .insert("ratelimit".to_string(), ratelimit_middleware_handler);
    }
    registry.add_middleware(ratelimit_middleware_handler);
    server
}

#[no_mangle]
pub extern "C" fn doo_http_ratelimit_custom(
    server: *mut c_void,
    options: *mut c_void,
) -> *mut c_void {
    // Parse options map for rate limit configuration
    let config = if options.is_null() {
        RateLimitConfig::default()
    } else {
        let max = parse_json_i64_or_default(doo_map_get_str(options, "max"), 100);
        let window = parse_json_i64_or_default(doo_map_get_str(options, "window"), 60);
        let per_str = parse_json_string_or_default(doo_map_get_str(options, "per"), "ip");

        RateLimitConfig {
            max: if max > 0 { max as u32 } else { 100 },
            window: if window > 0 { window as u64 } else { 60 },
            per: per_str,
        }
    };

    *get_ratelimit_config().lock().unwrap() = Some(config);

    // Clear state for fresh start
    get_ratelimit_state().lock().unwrap().clear();

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    if !registry.middleware_handlers.contains_key("ratelimit") {
        registry
            .middleware_handlers
            .insert("ratelimit".to_string(), ratelimit_middleware_handler);
    }
    registry.add_middleware(ratelimit_middleware_handler);
    server
}

#[no_mangle]
pub extern "C" fn doo_http_group(
    _server: *const c_void,
    _prefix: *const c_char,
    _handler: extern "C" fn(),
) -> *mut DooResult {
    // Groups handled at compile-time, no-op at runtime
    make_ok_void()
}

// ============================================================================
// REQUEST HELPERS
// ============================================================================

#[no_mangle]
pub extern "C" fn doo_http_req_query(req: *const DooRequest, key: *const c_char) -> *const c_char {
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
}

#[no_mangle]
pub extern "C" fn doo_http_req_param(req: *const DooRequest, key: *const c_char) -> *const c_char {
    if req.is_null() {
        return std::ptr::null();
    }
    unsafe {
        let params_map = (*req).params as *const HashMap<String, String>;
        if params_map.is_null() {
            return std::ptr::null();
        }
        let key_str = c_to_string(key);
        (*params_map)
            .get(&key_str)
            .map(|v| string_to_c(v))
            .unwrap_or(std::ptr::null())
    }
}

#[no_mangle]
pub extern "C" fn doo_http_req_header(req: *const DooRequest, key: *const c_char) -> *const c_char {
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
}

#[no_mangle]
pub extern "C" fn doohttp_extract_param_int(
    req: *const DooRequest,
    param_name: *const c_char,
) -> i64 {
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
            let err = bad_request(
                format!(
                    "Invalid path parameter type: expected Int, got '{}'",
                    value_str
                ),
                get_current_request_path(),
            );
            set_last_error(400, err.to_json());
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn doohttp_extract_param_float(
    req: *const DooRequest,
    param_name: *const c_char,
) -> f64 {
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
            let err = bad_request(
                format!(
                    "Invalid path parameter type: expected Float, got '{}'",
                    value_str
                ),
                get_current_request_path(),
            );
            set_last_error(400, err.to_json());
            0.0
        }
    }
}

/// Extract typed path parameter from request with type validation
/// Returns: converted value as C string (caller must free), or null on error
#[no_mangle]
pub extern "C" fn doohttp_extract_param_typed(
    req: *const DooRequest,
    param_name: *const c_char,
    param_type: *const c_char,
) -> *const c_char {
    clear_last_error();
    if req.is_null() || param_name.is_null() || param_type.is_null() {
        return std::ptr::null();
    }

    let param_name_str = c_to_string(param_name);
    let param_type_str = c_to_string(param_type);
    let value_ptr = doo_http_req_param(req, param_name);

    if value_ptr.is_null() {
        let err = bad_request(
            format!("Path parameter '{}' not found", param_name_str),
            get_current_request_path(),
        );
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
                let err = bad_request(
                    format!(
                        "Invalid path parameter type for '{}': expected Int, got '{}'",
                        param_name_str, value
                    ),
                    get_current_request_path(),
                );
                set_last_error(400, err.to_json());
                std::ptr::null()
            }
        }
        "Float" => {
            if value.parse::<f64>().is_ok() {
                string_to_c(&value)
            } else {
                let err = bad_request(
                    format!(
                        "Invalid path parameter type for '{}': expected Float, got '{}'",
                        param_name_str, value
                    ),
                    get_current_request_path(),
                );
                set_last_error(400, err.to_json());
                std::ptr::null()
            }
        }
        "Bool" => {
            if value == "true" || value == "false" {
                string_to_c(&value)
            } else {
                let err = bad_request(
                    format!(
                        "Invalid path parameter type for '{}': expected Bool, got '{}'",
                        param_name_str, value
                    ),
                    get_current_request_path(),
                );
                set_last_error(400, err.to_json());
                std::ptr::null()
            }
        }
        _ => string_to_c(&value), // String or other types - return as-is
    }
}

// ============================================================================
// RFC 7807 ERROR FUNCTIONS
// ============================================================================

// Helper for map value parsing (used by populate_struct)
fn doo_map_get_str(map_ptr: *const c_void, key: &str) -> *const c_char {
    if map_ptr.is_null() {
        return std::ptr::null();
    }
    unsafe {
        let map = &*(map_ptr as *const HashMap<String, String>);
        match map.get(key) {
            Some(v) => string_to_c(v),
            None => std::ptr::null(),
        }
    }
}

fn parse_json_i64_or_default(ptr: *const c_char, default: i64) -> i64 {
    if ptr.is_null() {
        return default;
    }
    let s = c_to_string(ptr);
    // Handle JSON-encoded numbers (might be quoted or unquoted)
    let trimmed = s.trim().trim_matches('"');
    trimmed.parse::<i64>().unwrap_or(default)
}

fn parse_json_string_or_default(ptr: *const c_char, default: &str) -> String {
    if ptr.is_null() {
        return default.to_string();
    }
    let s = c_to_string(ptr);
    // Remove JSON quotes if present
    let trimmed = s.trim().trim_matches('"');
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    }
}

fn parse_json_bool_or_default(ptr: *const c_char, default: bool) -> bool {
    if ptr.is_null() {
        return default;
    }
    let s = c_to_string(ptr).trim().to_lowercase();
    match s.as_str() {
        "true" | "\"true\"" | "1" => true,
        "false" | "\"false\"" | "0" => false,
        _ => default,
    }
}

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
    source_type: i32,
    handler_name: *const c_char,
) -> i32 {
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

    // Get handler metadata from registry
    let routes = get_routes();
    let registry = routes.lock().unwrap();
    let metadata = registry.handler_metadata.get(&handler_name_str).cloned();
    drop(registry);

    let metadata = match metadata {
        Some(m) => m,
        None => return 0, // No metadata, skip validation
    };

    // Get HTTP method for smart source detection
    let method_str = c_to_string(request.method).to_uppercase();

    // Smart source_type detection:
    // For GET/DELETE with no body, automatically use query params
    let effective_source_type = if source_type == 0 {
        let has_body = if request.body.is_null() {
            false
        } else {
            let body_str = c_to_string(request.body);
            !body_str.is_empty()
        };

        if !has_body && (method_str == "GET" || method_str == "DELETE") {
            2 // Use query params for GET/DELETE without body
        } else {
            0 // Use body for POST/PUT/PATCH or when body exists
        }
    } else {
        source_type
    };

    // Parse source data based on effective_source_type
    let source_data: serde_json::Map<String, serde_json::Value> = match effective_source_type {
        0 => {
            // Parse body JSON
            if request.body.is_null() {
                return 0;
            }
            let body_str = c_to_string(request.body);
            if body_str.is_empty() {
                return 0;
            }
            match serde_json::from_str::<serde_json::Value>(&body_str) {
                Ok(serde_json::Value::Object(obj)) => obj,
                _ => {
                    let err = bad_request("Invalid JSON body", path_str.clone());
                    set_last_error(400, err.to_json());
                    return 400;
                }
            }
        }
        1 => {
            // Extract from path params
            if request.params.is_null() {
                serde_json::Map::new()
            } else {
                let params_map = unsafe { &*(request.params as *const HashMap<String, String>) };
                params_map
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect()
            }
        }
        2 => {
            // Extract from query params
            if request.query.is_null() {
                serde_json::Map::new()
            } else {
                let query_map = unsafe { &*(request.query as *const HashMap<String, String>) };
                query_map
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect()
            }
        }
        _ => serde_json::Map::new(),
    };

    // Get struct name from param_types
    let struct_name = if !metadata.param_types.is_empty() {
        metadata
            .param_types
            .first()
            .cloned()
            .unwrap_or_else(|| "Unknown".to_string())
    } else {
        "Unknown".to_string()
    };

    // Special types that receive raw request pointer - skip validation
    if struct_name == "Request" || struct_name == "DooRequest" || struct_name == "Unknown" {
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

            let field_name = match field_obj.get("name").and_then(|v| v.as_str()) {
                Some(n) => n,
                None => continue,
            };

            let field_type = match field_obj.get("type").and_then(|v| v.as_str()) {
                Some(t) => t,
                None => continue,
            };

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
                let err = FieldError::new("Field is required").with_rule("required");
                field_errors.insert(full_field_name, err);
                continue;
            }

            if let Some(value) = source_data.get(field_name) {
                // Validate array element types
                if field_type.starts_with("[") && field_type.ends_with("]") {
                    let elem_type = &field_type[1..field_type.len() - 1];

                    if let serde_json::Value::Array(arr) = value {
                        for (i, elem) in arr.iter().enumerate() {
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
                                        if n.is_i64() || n.is_u64() { "Int" } else { "Float" }
                                    }
                                    serde_json::Value::String(_) => "Str",
                                    serde_json::Value::Array(_) => "Array",
                                    serde_json::Value::Object(_) => "Object",
                                };

                                let err = FieldError::new(format!(
                                    "Element [{}] has wrong type: expected {}, got {}",
                                    i, elem_type, received_type
                                ))
                                .with_rule("type_mismatch")
                                .with_expected(elem_type.to_string())
                                .with_received(received_type.to_string());
                                field_errors.insert(format!("{}[{}]", full_field_name, i), err);
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
                        let err = FieldError::new(format!("Expected array, got {}", received_type))
                            .with_rule("type_mismatch")
                            .with_expected(field_type.to_string())
                            .with_received(received_type.to_string());
                        field_errors.insert(full_field_name, err);
                    }
                } else {
                    // Check primitive types first
                    let is_primitive = matches!(field_type, "Int" | "Float" | "Bool" | "Str" | "String");
                    
                    if is_primitive {
                        let type_valid = match field_type {
                            "Int" => value.is_i64() || value.is_u64(),
                            "Float" => value.is_f64(),
                            "Bool" => value.is_boolean(),
                            "Str" | "String" => value.is_string(),
                            _ => true,
                        };

                        if !type_valid {
                            let received_type = match value {
                                serde_json::Value::Null => "null",
                                serde_json::Value::Bool(_) => "Bool",
                                serde_json::Value::Number(n) => {
                                    if n.is_i64() || n.is_u64() { "Int" } else { "Float" }
                                }
                                serde_json::Value::String(_) => "Str",
                                serde_json::Value::Array(_) => "Array",
                                serde_json::Value::Object(_) => "Object",
                            };
                            let err = FieldError::new(format!("Invalid type, expected {}", field_type))
                                .with_rule("type_mismatch")
                                .with_expected(field_type.to_string())
                                .with_received(received_type.to_string());
                            field_errors.insert(full_field_name, err);
                        }
                    } else if let Some(variants) = metadata.enum_variants.get(field_type) {
                        // Enum validation
                        let valid = if let Some(s) = value.as_str() {
                            variants.contains(&s.to_string())
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
                                "Invalid enum value '{}', expected one of: {:?}",
                                received_str, variants
                            ))
                            .with_rule("enum_value")
                            .with_expected(format!("{:?}", variants))
                            .with_received(received_str);
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
                            let err = FieldError::new(format!("Expected object, got {}", received_type))
                                .with_rule("type_mismatch")
                                .with_expected(field_type.to_string())
                                .with_received(received_type.to_string());
                            field_errors.insert(full_field_name, err);
                        }
                    }
                    // Unknown types are skipped
                }
            }
        }
    }

    // Validate fields and populate struct using struct_layouts
    let mut field_errors: HashMap<String, FieldError> = HashMap::new();
    validate_struct_fields(&source_data, &struct_name, "", &metadata, &mut field_errors);

    if !field_errors.is_empty() {
        // Type mismatch errors are parsing errors (400), not validation errors (422)
        // Convert to core FieldErrors
        let core_errors: Vec<doo_ffi_core::FieldError> = field_errors
            .into_iter()
            .map(|(field, err)| err.to_core(&field))
            .collect();
        
        let err = doo_ffi_core::Rfc7807Error::bad_request("Request body parsing failed")
            .with_instance(path_str)
            .with_errors(core_errors);
        set_last_error(400, err.to_json());
        return 400;
    }

    0 // Success
}

#[no_mangle]
pub extern "C" fn doohttp_error_rfc7807(
    status: i32,
    detail: *const c_char,
    instance: *const c_char,
) -> *const c_char {
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
}

#[no_mangle]
pub extern "C" fn doohttp_error_rfc7807_auto_instance(
    status: i32,
    detail: *const c_char,
) -> *const c_char {
    doohttp_error_rfc7807(status, detail, std::ptr::null())
}

#[no_mangle]
pub extern "C" fn doohttp_last_error_status() -> i32 {
    get_last_error_status()
}

#[no_mangle]
pub extern "C" fn doohttp_last_error_json() -> *const c_char {
    string_to_c(&get_last_error_json())
}

#[no_mangle]
pub extern "C" fn doohttp_error_to_status(
    _error_type: *const c_char,
    variant: *const c_char,
) -> i32 {
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
}

// ============================================================================
// HANDLER REGISTRATION
// ============================================================================

#[no_mangle]
pub extern "C" fn doo_http_register_handler(name: *const c_char, handler: DooHandlerFn) {
    let name_str = c_to_string(name);
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.register_handler(&name_str, handler);
}

#[no_mangle]
pub extern "C" fn doo_http_register_handler_with_metadata(
    name: *const c_char,
    handler: DooHandlerFn,
    metadata_json: *const c_char,
) {
    let name_str = c_to_string(name);
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();

    // Parse metadata JSON properly
    let metadata = if metadata_json.is_null() {
        HandlerMetadata::default()
    } else {
        let json_str = c_to_string(metadata_json);
        parse_handler_metadata(&json_str).unwrap_or_default()
    };

    registry.register_handler_with_metadata(&name_str, handler, metadata);
}

/// Parse handler metadata JSON into HandlerMetadata struct
fn parse_handler_metadata(json_str: &str) -> Option<HandlerMetadata> {
    let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let obj = parsed.as_object()?;

    // Extract param_types array
    let param_types = obj
        .get("param_types")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // Extract struct_layouts map
    let struct_layouts = obj
        .get("struct_layouts")
        .and_then(|v| v.as_object())
        .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    // Extract enum_variants map
    let enum_variants = obj
        .get("enum_variants")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| {
                    v.as_array().map(|arr| {
                        let variants: Vec<String> = arr
                            .iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect();
                        (k.clone(), variants)
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Extract return_type
    let return_type = obj
        .get("return_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Void".to_string());

    Some(HandlerMetadata {
        param_types,
        return_type,
        struct_decorators: HashMap::new(),
        struct_layouts,
        enum_variants,
    })
}

// ============================================================================
// RESPONSE HELPERS
// ============================================================================

#[no_mangle]
pub extern "C" fn doohttp_create_response_from_result(
    tag: i32,
    value_ptr: *const c_void,
    success_body_ptr: *const c_char,
) -> *mut DooResponse {
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
        (*response).content_type = string_to_c("application/json");
        response
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
    if struct_ptr.is_null() || handler_name.is_null() {
        return string_to_c("{}");
    }

    let handler_name_str = c_to_string(handler_name);

    // Get handler metadata from registry
    let routes = get_routes();
    let registry = routes.lock().unwrap();
    let metadata = registry.handler_metadata.get(&handler_name_str).cloned();
    drop(registry);

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

    // Serialize struct recursively
    let json = serialize_struct_recursive(
        struct_ptr as *const u8,
        return_type,
        &metadata.struct_layouts,
    );
    
    string_to_c(&json.to_string())
}

/// Recursively serialize a struct to JSON value.
fn serialize_struct_recursive(
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
    let mut offset: usize = 0;

    for field in fields {
        let field_obj = match field.as_object() {
            Some(obj) => obj,
            None => continue,
        };

        let field_name = match field_obj.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => continue,
        };

        let field_type = match field_obj.get("type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => continue,
        };

        // Calculate alignment and size for the field type
        let (field_size, field_align) = get_type_size_align(field_type, struct_layouts);
        
        // Align offset to field alignment
        offset = align_up(offset, field_align);

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
                        let elem_type = &t[1..t.len()-1];
                        serialize_array(arr_data, elem_type, struct_layouts)
                    }
                }
                _ if struct_layouts.contains_key(field_type) => {
                    // Nested struct - pointer to struct
                    let nested_ptr = *(field_ptr as *const *const u8);
                    serialize_struct_recursive(nested_ptr, field_type, struct_layouts)
                }
                _ => {
                    serde_json::Value::Null
                }
            };
            json_obj.insert(field_name.to_string(), field_value);
            offset += field_size;
        }
    }

    serde_json::Value::Object(json_obj)
}

/// Align a value up to the given alignment
fn align_up(offset: usize, align: usize) -> usize {
    if align == 0 {
        return offset;
    }
    (offset + align - 1) & !(align - 1)
}

/// Get the size and alignment for a type
fn get_type_size_align(type_name: &str, struct_layouts: &HashMap<String, serde_json::Value>) -> (usize, usize) {
    match type_name {
        "Str" => (8, 8),    // pointer
        "Int" => (8, 8),    // i64
        "Float" => (8, 8),  // double
        "Bool" => (1, 1),   // i1/i8
        t if t.starts_with("[") && t.ends_with("]") => (8, 8), // pointer to array data
        _ if struct_layouts.contains_key(type_name) => (8, 8), // pointer to struct
        _ => (8, 8), // default to pointer size
    }
}

/// Serialize array to JSON.
fn serialize_array(
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
            "Str" => 8,    // pointer
            "Int" => 8,    // i64
            "Float" => 8,  // double
            "Bool" => 1,   // i1/i8
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

// ============================================================================
// UTILITY FUNCTIONS
// ============================================================================

fn make_ok_void() -> *mut DooResult {
    unsafe {
        let ptr = libc::malloc(std::mem::size_of::<DooResult>()) as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        (*ptr).tag = 0;
        (*ptr).value = std::ptr::null_mut();
        (*ptr).owner = owner::FFI;
        ptr
    }
}

fn make_err_http(status: i32, message: &str) -> *mut DooResult {
    set_last_error(status, message.to_string());
    unsafe {
        let ptr = libc::malloc(std::mem::size_of::<DooResult>()) as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        (*ptr).tag = 1;
        (*ptr).value = string_to_c(message) as *mut c_void;
        (*ptr).owner = owner::FFI;
        ptr
    }
}
