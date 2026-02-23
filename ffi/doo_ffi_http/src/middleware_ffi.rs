//! Middleware FFI Entry Points
//!
//! Functions for registering and configuring middleware:
//! JWT, CORS, Rate Limiting, custom middleware, groups, and next-call chaining.

use std::ffi::c_void;
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};

use doo_ffi_core::constants::{MIDDLEWARE_CORS, MIDDLEWARE_JWT, MIDDLEWARE_RATELIMIT};
use doo_ffi_core::DooResult;
use doo_ffi_core::{ffi_safe_cstr, ffi_safe_ptr, ffi_safe_void};

use crate::helpers::{c_to_string, string_to_c};
use crate::make_ok_void;
use crate::map_ops::{doo_map_get_str, parse_json_bool_or_default, parse_json_i64_or_default, parse_json_string_or_default};
use crate::middleware::*;
use crate::router::get_routes;
use crate::types::*;

// ============================================================================
// MIDDLEWARE REGISTRATION
// ============================================================================

/// Register a middleware function pointer for global use
/// The middleware parameter is a function pointer (DooMiddlewareFn) despite the type signature
/// The compiler passes the wrapper function pointer directly
#[no_mangle]
pub extern "C" fn doo_http_use(
    server: *const c_void,
    middleware_fn: DooMiddlewareFn,
) -> *const c_void {
    match catch_unwind(AssertUnwindSafe(|| {
        let routes = get_routes();
        let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());

        // Add the middleware function directly to global middleware
        registry.add_middleware(middleware_fn);

        server
    })) {
        Ok(r) => r,
        Err(_) => std::ptr::null(),
    }
}

/// Register a user-defined middleware function by name
/// Called by the compiler to register middleware before route registration
#[no_mangle]
pub extern "C" fn doo_http_register_middleware(
    name: *const c_char,
    middleware_fn: DooMiddlewareFn,
) {
    ffi_safe_void!({
        let name_str = c_to_string(name);
        let routes = get_routes();
        let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());
        registry.middleware_handlers.insert(name_str, middleware_fn);
    });
}

#[no_mangle]
pub extern "C" fn doo_http_jwt() -> *const c_char {
    ffi_safe_cstr!({
        let routes = get_routes();
        let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());
        if !registry.middleware_handlers.contains_key(MIDDLEWARE_JWT) {
            registry
                .middleware_handlers
                .insert(MIDDLEWARE_JWT.to_string(), jwt_middleware_handler);
        }
        string_to_c(MIDDLEWARE_JWT)
    })
}

#[no_mangle]
pub extern "C" fn doo_http_cors(server: *mut c_void) -> *mut c_void {
    ffi_safe_ptr!({
        let config = CorsConfig::default();
        *get_cors_config().lock().unwrap_or_else(|e| e.into_inner()) = Some(config);

        let routes = get_routes();
        let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());
        if !registry.middleware_handlers.contains_key(MIDDLEWARE_CORS) {
            registry
                .middleware_handlers
                .insert(MIDDLEWARE_CORS.to_string(), cors_middleware_handler);
        }
        registry.add_middleware(cors_middleware_handler);
        server
    })
}

#[no_mangle]
pub extern "C" fn doo_http_cors_custom(server: *mut c_void, options: *mut c_void) -> *mut c_void {
    ffi_safe_ptr!({
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

        *get_cors_config().lock().unwrap_or_else(|e| e.into_inner()) = Some(config);

        let routes = get_routes();
        let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());
        if !registry.middleware_handlers.contains_key(MIDDLEWARE_CORS) {
            registry
                .middleware_handlers
                .insert(MIDDLEWARE_CORS.to_string(), cors_middleware_handler);
        }
        registry.add_middleware(cors_middleware_handler);
        server
    })
}

#[no_mangle]
pub extern "C" fn doo_http_ratelimit(server: *mut c_void) -> *mut c_void {
    ffi_safe_ptr!({
        let config = RateLimitConfig::default();
        *get_ratelimit_config()
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(config);

        // Clear state for fresh start
        get_ratelimit_state()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();

        let routes = get_routes();
        let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());
        if !registry
            .middleware_handlers
            .contains_key(MIDDLEWARE_RATELIMIT)
        {
            registry.middleware_handlers.insert(
                MIDDLEWARE_RATELIMIT.to_string(),
                ratelimit_middleware_handler,
            );
        }
        registry.add_middleware(ratelimit_middleware_handler);
        server
    })
}

#[no_mangle]
pub extern "C" fn doo_http_ratelimit_custom(
    server: *mut c_void,
    options: *mut c_void,
) -> *mut c_void {
    ffi_safe_ptr!({
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

        *get_ratelimit_config()
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(config);

        // Clear state for fresh start
        get_ratelimit_state()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();

        let routes = get_routes();
        let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());
        if !registry
            .middleware_handlers
            .contains_key(MIDDLEWARE_RATELIMIT)
        {
            registry.middleware_handlers.insert(
                MIDDLEWARE_RATELIMIT.to_string(),
                ratelimit_middleware_handler,
            );
        }
        registry.add_middleware(ratelimit_middleware_handler);
        server
    })
}

#[no_mangle]
pub extern "C" fn doo_http_group(
    _server: *const c_void,
    _prefix: *const c_char,
    _handler: extern "C" fn(),
) -> *mut DooResult {
    ffi_safe_result!({
        // Groups handled at compile-time, no-op at runtime
        make_ok_void()
    })
}

// ============================================================================
// LOGGER CONFIGURATION
// ============================================================================

/// Enable logger with default config (all levels: Info, Warn, Error).
/// Called when user writes `app.logger()` with no arguments.
/// Returns server pointer for chaining.
#[no_mangle]
pub extern "C" fn doo_http_logger_custom(
    server: *mut c_void,
    options: *mut c_void,
) -> *mut c_void {
    ffi_safe_ptr!({
        let config = if options.is_null() {
            // Default: all levels enabled
            LoggerConfig::default()
        } else {
            // Parse Level array from options map
            // Doo code: app.logger({Level: ["Error", "Warn"]})
            let level_ptr = doo_map_get_str(options, "Level");

            if level_ptr.is_null() {
                // No Level key → all levels enabled (default)
                LoggerConfig::default()
            } else {
                // Parse the Level value — it's a JSON array string like ["Error","Warn"]
                let level_str = parse_json_string_or_default(level_ptr, "");

                // Start with all disabled, enable only specified levels
                let mut info = false;
                let mut warn = false;
                let mut error = false;

                // Parse comma-separated or JSON array
                let cleaned = level_str
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .replace('"', "");
                for level in cleaned.split(',') {
                    match level.trim().to_lowercase().as_str() {
                        "info" => info = true,
                        "warn" | "warning" => warn = true,
                        "error" | "err" => error = true,
                        _ => {}
                    }
                }

                LoggerConfig { info, warn, error }
            }
        };

        *get_logger_config().lock().unwrap_or_else(|e| e.into_inner()) = Some(config);
        server
    })
}

// ============================================================================
// MIDDLEWARE NEXT CALL
// ============================================================================

/// Call the next middleware/handler in the chain
/// The `next` parameter is actually a function pointer (DooNextFn) passed from the wrapper
/// We need to get the current request from thread-local and call the next function
#[no_mangle]
pub extern "C" fn doo_http_next_call(next: *const std::ffi::c_void) -> *mut std::ffi::c_void {
    ffi_safe_ptr!({
        use crate::server::get_current_request;

        if next.is_null() {
            // No next function - return null result
            return std::ptr::null_mut();
        }

        // The `next` is actually a DooNextFn: fn(*const DooRequest) -> *mut DooResult
        // Get the current request from thread-local storage
        let request_ptr = get_current_request();
        if request_ptr.is_null() {
            return std::ptr::null_mut();
        }

        // Cast next to the function type and call it
        let next_fn: DooNextFn = unsafe { std::mem::transmute(next) };
        let result = next_fn(request_ptr);

        // The result is a DooResult containing a Response
        // We need to extract the Response pointer and return it
        if result.is_null() {
            return std::ptr::null_mut();
        }

        unsafe {
            let doo_result = &*result;

            // Build Response struct from DooResult
            // Response layout: { i64 Status, ptr Body, ptr ContentType }
            let response_size = std::mem::size_of::<i64>() + 2 * std::mem::size_of::<*const i8>();
            let response_ptr = libc::malloc(response_size) as *mut u8;
            if response_ptr.is_null() {
                return std::ptr::null_mut();
            }

            if doo_result.tag == 0 {
                // Ok result - build Response with status 200 and the JSON body
                *(response_ptr as *mut i64) = 200;
                *((response_ptr as *mut u8).add(8) as *mut *const i8) =
                    doo_result.data as *const i8;
                *((response_ptr as *mut u8).add(16) as *mut *const i8) =
                    string_to_c("application/json");
            } else {
                // Error result - value is error response struct { i32 status, ptr body, ptr content_type }
                // Extract fields from error struct and build Response
                let error_struct = doo_result.data as *const u8;
                let status = *(error_struct as *const i32) as i64;
                let body_ptr = *((error_struct as *const u8).add(8) as *const *const i8);
                let ct_ptr = *((error_struct as *const u8).add(16) as *const *const i8);

                *(response_ptr as *mut i64) = status;
                *((response_ptr as *mut u8).add(8) as *mut *const i8) = body_ptr;
                *((response_ptr as *mut u8).add(16) as *mut *const i8) = ct_ptr;
            }

            response_ptr as *mut std::ffi::c_void
        }
    })
}
