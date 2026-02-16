//! Route Registration
//!
//! All HTTP method route registration functions (GET, POST, PUT, DELETE, PATCH)
//! with optional middleware support. Uses centralized helpers for consistent behavior.

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
