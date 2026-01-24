//! doo_ffi_http - Complete HTTP FFI Library
//! 
//! Provides all HTTP functionality for Doo applications:
//! - Route registration (GET, POST, PUT, DELETE, PATCH)
//! - Middleware (JWT, CORS, Rate Limiting)
//! - RFC 7807 error responses
//! - Request helpers (params, query, headers)
//! - Server lifecycle

mod types;
mod router;
mod error;
mod middleware;
mod helpers;
mod server;

use std::ffi::c_void;
use std::os::raw::c_char;
use std::collections::HashMap;

pub use types::*;
pub use router::*;
pub use error::*;
pub use middleware::*;
pub use helpers::*;

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
        let p = host_port_str[colon+1..].parse().unwrap_or(3000);
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
pub extern "C" fn doo_http_get(_server: *const c_void, path: *const c_char, handler_name: *const c_char) -> *mut DooResult {
    register_route("GET", path, handler_name)
}

#[no_mangle]
pub extern "C" fn doo_http_get_fn(_server: *const c_void, path: *const c_char, handler: DooHandlerFn) -> *mut DooResult {
    register_route_fn("GET", path, handler)
}

#[no_mangle]
pub extern "C" fn doo_http_post(_server: *const c_void, path: *const c_char, handler_name: *const c_char) -> *mut DooResult {
    register_route("POST", path, handler_name)
}

#[no_mangle]
pub extern "C" fn doo_http_post_fn(_server: *const c_void, path: *const c_char, handler: DooHandlerFn) -> *mut DooResult {
    register_route_fn("POST", path, handler)
}

#[no_mangle]
pub extern "C" fn doo_http_put(_server: *const c_void, path: *const c_char, handler_name: *const c_char) -> *mut DooResult {
    register_route("PUT", path, handler_name)
}

#[no_mangle]
pub extern "C" fn doo_http_put_fn(_server: *const c_void, path: *const c_char, handler: DooHandlerFn) -> *mut DooResult {
    register_route_fn("PUT", path, handler)
}

#[no_mangle]
pub extern "C" fn doo_http_delete(_server: *const c_void, path: *const c_char, handler_name: *const c_char) -> *mut DooResult {
    register_route("DELETE", path, handler_name)
}

#[no_mangle]
pub extern "C" fn doo_http_delete_fn(_server: *const c_void, path: *const c_char, handler: DooHandlerFn) -> *mut DooResult {
    register_route_fn("DELETE", path, handler)
}

#[no_mangle]
pub extern "C" fn doo_http_patch(_server: *const c_void, path: *const c_char, handler_name: *const c_char) -> *mut DooResult {
    register_route("PATCH", path, handler_name)
}

#[no_mangle]
pub extern "C" fn doo_http_patch_fn(_server: *const c_void, path: *const c_char, handler: DooHandlerFn) -> *mut DooResult {
    register_route_fn("PATCH", path, handler)
}

fn register_route(method: &str, path: *const c_char, handler_name: *const c_char) -> *mut DooResult {
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
pub extern "C" fn doo_http_use(server: *const c_void, middleware_name: *const c_char) -> *const c_void {
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
        registry.middleware_handlers.insert("jwt".to_string(), jwt_middleware_handler);
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
        registry.middleware_handlers.insert("cors".to_string(), cors_middleware_handler);
    }
    registry.add_middleware(cors_middleware_handler);
    server
}

#[no_mangle]
pub extern "C" fn doo_http_cors_custom(server: *mut c_void, options: *mut c_void) -> *mut c_void {
    // Parse options map and create CorsConfig
    let config = CorsConfig::default(); // TODO: parse from options
    *get_cors_config().lock().unwrap() = Some(config);
    
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.add_middleware(cors_middleware_handler);
    server
}

#[no_mangle]
pub extern "C" fn doo_http_ratelimit(server: *mut c_void) -> *mut c_void {
    let config = RateLimitConfig::default();
    *get_ratelimit_config().lock().unwrap() = Some(config);
    
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.add_middleware(ratelimit_middleware_handler);
    server
}

#[no_mangle]
pub extern "C" fn doo_http_ratelimit_custom(server: *mut c_void, options: *mut c_void) -> *mut c_void {
    let config = RateLimitConfig::default(); // TODO: parse from options
    *get_ratelimit_config().lock().unwrap() = Some(config);
    
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.add_middleware(ratelimit_middleware_handler);
    server
}

#[no_mangle]
pub extern "C" fn doo_http_group(_server: *const c_void, _prefix: *const c_char, _handler: extern "C" fn()) -> *mut DooResult {
    // Groups handled at compile-time, no-op at runtime
    make_ok_void()
}

// ============================================================================
// REQUEST HELPERS
// ============================================================================

#[no_mangle]
pub extern "C" fn doo_http_req_query(req: *const DooRequest, key: *const c_char) -> *const c_char {
    if req.is_null() { return std::ptr::null(); }
    unsafe {
        let query_map = (*req).query as *const HashMap<String, String>;
        if query_map.is_null() { return std::ptr::null(); }
        let key_str = c_to_string(key);
        (*query_map).get(&key_str).map(|v| string_to_c(v)).unwrap_or(std::ptr::null())
    }
}

#[no_mangle]
pub extern "C" fn doo_http_req_param(req: *const DooRequest, key: *const c_char) -> *const c_char {
    if req.is_null() { return std::ptr::null(); }
    unsafe {
        let params_map = (*req).params as *const HashMap<String, String>;
        if params_map.is_null() { return std::ptr::null(); }
        let key_str = c_to_string(key);
        (*params_map).get(&key_str).map(|v| string_to_c(v)).unwrap_or(std::ptr::null())
    }
}

#[no_mangle]
pub extern "C" fn doo_http_req_header(req: *const DooRequest, key: *const c_char) -> *const c_char {
    if req.is_null() { return std::ptr::null(); }
    unsafe {
        let headers_map = (*req).headers as *const HashMap<String, String>;
        if headers_map.is_null() { return std::ptr::null(); }
        let key_str = c_to_string(key).to_lowercase();
        (*headers_map).get(&key_str).map(|v| string_to_c(v)).unwrap_or(string_to_c(""))
    }
}

#[no_mangle]
pub extern "C" fn doohttp_extract_param_int(req: *const DooRequest, param_name: *const c_char) -> i64 {
    if req.is_null() || param_name.is_null() { return 0; }
    let value_ptr = doo_http_req_param(req, param_name);
    if value_ptr.is_null() { return 0; }
    c_to_string(value_ptr).parse().unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn doohttp_extract_param_float(req: *const DooRequest, param_name: *const c_char) -> f64 {
    if req.is_null() || param_name.is_null() { return 0.0; }
    let value_ptr = doo_http_req_param(req, param_name);
    if value_ptr.is_null() { return 0.0; }
    c_to_string(value_ptr).parse().unwrap_or(0.0)
}

// ============================================================================
// RFC 7807 ERROR FUNCTIONS
// ============================================================================

#[no_mangle]
pub extern "C" fn doohttp_error_rfc7807(status: i32, detail: *const c_char, instance: *const c_char) -> *const c_char {
    let detail_str = c_to_string(detail);
    let instance_str = if instance.is_null() { get_current_request_path() } else { c_to_string(instance) };
    
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
pub extern "C" fn doohttp_error_rfc7807_auto_instance(status: i32, detail: *const c_char) -> *const c_char {
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
pub extern "C" fn doohttp_error_to_status(_error_type: *const c_char, variant: *const c_char) -> i32 {
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
    metadata_json: *const c_char
) {
    let name_str = c_to_string(name);
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    
    // Parse metadata (simplified)
    let metadata = HandlerMetadata::default();
    registry.register_handler_with_metadata(&name_str, handler, metadata);
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
        if response.is_null() { return std::ptr::null_mut(); }
        
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
// UTILITY FUNCTIONS  
// ============================================================================

fn make_ok_void() -> *mut DooResult {
    unsafe {
        let ptr = libc::malloc(std::mem::size_of::<DooResult>()) as *mut DooResult;
        if ptr.is_null() { return std::ptr::null_mut(); }
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
        if ptr.is_null() { return std::ptr::null_mut(); }
        (*ptr).tag = 1;
        (*ptr).value = string_to_c(message) as *mut c_void;
        (*ptr).owner = owner::FFI;
        ptr
    }
}
