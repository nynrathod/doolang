//! Route Registration
//!
//! All HTTP method route registration functions (GET, POST, PUT, DELETE, PATCH)
//! with optional middleware support. Uses centralized helpers for consistent behavior.
//!
//! ## Package Route Registration
//!
//! External FFI packages (e.g., doo_ffi_auth for OAuth) can register routes at runtime
//! via `doo_http_register_package_route`. This generic API allows any package to add
//! HTTP routes without coupling to the HTTP FFI crate.

use std::ffi::c_void;
use std::os::raw::c_char;

use doo_ffi_core::constants::{MIDDLEWARE_CORS, MIDDLEWARE_JWT, MIDDLEWARE_RATELIMIT};
use doo_ffi_core::DooResult;

use crate::helpers::c_to_string;
use crate::make_ok_void;
use crate::middleware::{cors_middleware_handler, jwt_middleware_handler, ratelimit_middleware_handler};
use crate::router::get_routes;
use crate::types::DooHandlerFn;

// ============================================================================
// ROUTE REGISTRATION
// ============================================================================

#[no_mangle]
pub extern "C" fn doo_http_get(
    _server: *const c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    ffi_safe_result!({ register_route("GET", path, handler_name) })
}

#[no_mangle]
pub extern "C" fn doo_http_get_fn(
    _server: *const c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    ffi_safe_result!({ register_route_fn("GET", path, handler) })
}

#[no_mangle]
pub extern "C" fn doo_http_post(
    _server: *const c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    ffi_safe_result!({ register_route("POST", path, handler_name) })
}

#[no_mangle]
pub extern "C" fn doo_http_post_fn(
    _server: *const c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    ffi_safe_result!({ register_route_fn("POST", path, handler) })
}

#[no_mangle]
pub extern "C" fn doo_http_put(
    _server: *const c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    ffi_safe_result!({ register_route("PUT", path, handler_name) })
}

#[no_mangle]
pub extern "C" fn doo_http_put_fn(
    _server: *const c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    ffi_safe_result!({ register_route_fn("PUT", path, handler) })
}

#[no_mangle]
pub extern "C" fn doo_http_delete(
    _server: *const c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    ffi_safe_result!({ register_route("DELETE", path, handler_name) })
}

#[no_mangle]
pub extern "C" fn doo_http_delete_fn(
    _server: *const c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    ffi_safe_result!({ register_route_fn("DELETE", path, handler) })
}

#[no_mangle]
pub extern "C" fn doo_http_patch(
    _server: *const c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    ffi_safe_result!({ register_route("PATCH", path, handler_name) })
}

#[no_mangle]
pub extern "C" fn doo_http_patch_fn(
    _server: *const c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    ffi_safe_result!({ register_route_fn("PATCH", path, handler) })
}

fn register_route(
    method: &str,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    let handler_str = c_to_string(handler_name);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());
    registry.register_by_name(method, &path_str, &handler_str);
    make_ok_void()
}

fn register_route_fn(method: &str, path: *const c_char, handler: DooHandlerFn) -> *mut DooResult {
    let path_str = c_to_string(path);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());
    registry.register(method, &path_str, handler);
    make_ok_void()
}

// ============================================================================
// ROUTE REGISTRATION WITH MIDDLEWARE - Single Source of Truth
// ============================================================================

/// Centralized helper: Register route with middleware names (comma-separated) and function pointer handler
fn register_route_with_middleware_fn(
    method: &str,
    path: *const c_char,
    middleware_names: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    let middleware_str = c_to_string(middleware_names);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());

    // Parse middleware names (comma-separated)
    let middleware_list: Vec<String> = if middleware_str.is_empty() {
        vec![]
    } else {
        middleware_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    // Lookup middleware functions and auto-register built-ins
    let mut middleware_fns = Vec::new();
    for mw_name in middleware_list {
        // Auto-register built-in middleware if referenced
        if mw_name == MIDDLEWARE_JWT && !registry.middleware_handlers.contains_key(MIDDLEWARE_JWT) {
            registry
                .middleware_handlers
                .insert(MIDDLEWARE_JWT.to_string(), jwt_middleware_handler);
        }
        if mw_name == MIDDLEWARE_CORS && !registry.middleware_handlers.contains_key(MIDDLEWARE_CORS)
        {
            registry
                .middleware_handlers
                .insert(MIDDLEWARE_CORS.to_string(), cors_middleware_handler);
        }
        if mw_name == MIDDLEWARE_RATELIMIT
            && !registry
                .middleware_handlers
                .contains_key(MIDDLEWARE_RATELIMIT)
        {
            registry.middleware_handlers.insert(
                MIDDLEWARE_RATELIMIT.to_string(),
                ratelimit_middleware_handler,
            );
        }

        if let Some(mw_fn) = registry.middleware_handlers.get(&mw_name).copied() {
            middleware_fns.push(mw_fn);
        }
    }

    registry.register_with_middleware(method, &path_str, handler, middleware_fns);
    make_ok_void()
}

/// Centralized helper: Register route with middleware names (comma-separated) and handler name
fn register_route_with_middleware(
    method: &str,
    path: *const c_char,
    middleware_names: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    let middleware_str = c_to_string(middleware_names);
    let handler_str = c_to_string(handler_name);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());

    // Parse middleware names (comma-separated)
    let middleware_list: Vec<String> = if middleware_str.is_empty() {
        vec![]
    } else {
        middleware_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    // Lookup middleware functions and auto-register built-ins
    let mut middleware_fns = Vec::new();
    for mw_name in middleware_list {
        // Auto-register built-in middleware if referenced
        if mw_name == MIDDLEWARE_JWT && !registry.middleware_handlers.contains_key(MIDDLEWARE_JWT) {
            registry
                .middleware_handlers
                .insert(MIDDLEWARE_JWT.to_string(), jwt_middleware_handler);
        }
        if mw_name == MIDDLEWARE_CORS && !registry.middleware_handlers.contains_key(MIDDLEWARE_CORS)
        {
            registry
                .middleware_handlers
                .insert(MIDDLEWARE_CORS.to_string(), cors_middleware_handler);
        }
        if mw_name == MIDDLEWARE_RATELIMIT
            && !registry
                .middleware_handlers
                .contains_key(MIDDLEWARE_RATELIMIT)
        {
            registry.middleware_handlers.insert(
                MIDDLEWARE_RATELIMIT.to_string(),
                ratelimit_middleware_handler,
            );
        }

        if let Some(mw_fn) = registry.middleware_handlers.get(&mw_name).copied() {
            middleware_fns.push(mw_fn);
        }
    }

    registry.register_by_name_with_middleware(method, &path_str, &handler_str, middleware_fns);
    make_ok_void()
}

#[no_mangle]
pub extern "C" fn doo_http_get_with_middleware(
    _server: *const c_void,
    path: *const c_char,
    middleware_names: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    ffi_safe_result!({ register_route_with_middleware_fn("GET", path, middleware_names, handler) })
}

#[no_mangle]
pub extern "C" fn doo_http_post_with_middleware(
    _server: *const c_void,
    path: *const c_char,
    middleware_names: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    ffi_safe_result!({ register_route_with_middleware_fn("POST", path, middleware_names, handler) })
}

#[no_mangle]
pub extern "C" fn doo_http_put_with_middleware(
    _server: *const c_void,
    path: *const c_char,
    middleware_names: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    ffi_safe_result!({ register_route_with_middleware_fn("PUT", path, middleware_names, handler) })
}

#[no_mangle]
pub extern "C" fn doo_http_delete_with_middleware(
    _server: *const c_void,
    path: *const c_char,
    middleware_names: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    ffi_safe_result!({
        register_route_with_middleware_fn("DELETE", path, middleware_names, handler)
    })
}

#[no_mangle]
pub extern "C" fn doo_http_patch_with_middleware(
    _server: *const c_void,
    path: *const c_char,
    middleware_names: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    ffi_safe_result!({
        register_route_with_middleware_fn("PATCH", path, middleware_names, handler)
    })
}

// ============================================================================
// GENERIC PACKAGE ROUTE REGISTRATION
// ============================================================================

/// Register a route from an external FFI package at runtime.
///
/// This is the public API for package-based route registration. Any external
/// FFI library (e.g., doo_ffi_auth for OAuth) can call this function via
/// `libloading` to register HTTP routes without coupling to doo_ffi_http.
///
/// The handler receives a raw pointer to the HTTP request struct (`DooRequest`)
/// as `*const c_void` and must return `*mut DooResult`. The request struct layout
/// is `#[repr(C)]`: `{ method: *const c_char, path: *const c_char, body: *const c_char,
/// headers: *mut c_void, params: *mut c_void, query: *mut c_void, user_id: *const c_char }`.
///
/// The `query` field points to a `HashMap<String, String>` (boxed).
///
/// For redirect responses (e.g., OAuth), return a DooResult with error tag and
/// an error struct containing status 302 + URL as body. The server automatically
/// builds a proper HTTP 302 redirect with Location header for any 3xx status.
#[no_mangle]
pub extern "C" fn doo_http_register_package_route(
    method: *const c_char,
    path: *const c_char,
    handler: extern "C" fn(*const c_void) -> *mut DooResult,
) -> *mut DooResult {
    ffi_safe_result!({
        let method_str = c_to_string(method);
        let path_str = c_to_string(path);

        doo_ffi_core::ffi_debug!(
            "HTTP",
            "Package route registered: {} {}",
            method_str,
            path_str
        );

        let routes = get_routes();
        let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());

        // Cast void handler to DooHandlerFn — same C ABI layout
        // *const c_void and *const DooRequest are both raw pointers (identical ABI)
        let handler_fn: DooHandlerFn = unsafe { std::mem::transmute(handler) };

        // Package routes are standalone handlers that need headers (cookies, auth, etc.)
        // Use register_package() which sets needs_headers=true so the server extracts
        // headers even for GET requests (unlike Doo-defined routes that may skip this).
        registry.register_package(&method_str, &path_str, handler_fn);

        make_ok_void()
    })
}

/// Push a cookie from an external DLL (e.g., doo_ffi_auth) into the HTTP server's
/// thread-local pending cookies.
///
/// # Why this exists
///
/// Each DLL gets its own copy of doo_ffi_core's thread-local `PENDING_COOKIES`.
/// When doo_ffi_auth (in its DLL) pushes cookies via `doo_ffi_core::cookies::push_auth_cookies()`,
/// those cookies go into doo_ffi_auth's thread-local — NOT doo_ffi_http's.
/// The server post-processing reads from doo_ffi_http's thread-local → cookies lost.
///
/// This function bridges the gap: auth DLL calls this exported symbol to push cookies
/// into the HTTP DLL's doo_ffi_core thread-local, where the server will find them.
///
/// # Parameters
/// - `cookie_header_value`: C string containing the full Set-Cookie header value
///   (e.g., "doo_access_token=eyJ...; Max-Age=3600; Path=/; HttpOnly; SameSite=Lax")
#[no_mangle]
pub extern "C" fn doo_http_push_cookie(cookie_header_value: *const c_char) {
    if cookie_header_value.is_null() {
        return;
    }
    let header_val = c_to_string(cookie_header_value);
    if header_val.is_empty() {
        return;
    }

    // Parse the Set-Cookie header value and push as a ResponseCookie
    // into the HTTP DLL's thread-local pending cookies.
    doo_ffi_core::cookies::push_raw_cookie_header(header_val);
}
