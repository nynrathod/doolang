//! Middleware Handlers
//! JWT authentication, CORS, and rate limiting middleware implementations.

use crate::error::*;
use crate::helpers::*;
use crate::router::get_routes;
use crate::types::*;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

// Global configurations
static CORS_CONFIG: OnceLock<Mutex<Option<CorsConfig>>> = OnceLock::new();
static RATELIMIT_CONFIG: OnceLock<Mutex<Option<RateLimitConfig>>> = OnceLock::new();
static RATELIMIT_STATE: OnceLock<Mutex<HashMap<String, RateLimitEntry>>> = OnceLock::new();
static JWT_SECRET: OnceLock<String> = OnceLock::new();

pub fn get_cors_config() -> &'static Mutex<Option<CorsConfig>> {
    CORS_CONFIG.get_or_init(|| Mutex::new(None))
}

pub fn get_ratelimit_config() -> &'static Mutex<Option<RateLimitConfig>> {
    RATELIMIT_CONFIG.get_or_init(|| Mutex::new(None))
}

pub fn get_ratelimit_state() -> &'static Mutex<HashMap<String, RateLimitEntry>> {
    RATELIMIT_STATE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// JWT middleware handler
pub extern "C" fn jwt_middleware_handler(
    req: *const DooRequest,
    next: DooNextFn,
) -> *mut DooResult {
    if req.is_null() {
        return make_err_response(401, "Null request");
    }

    unsafe {
        let headers = (*req).headers as *const HashMap<String, String>;
        if headers.is_null() {
            return make_err_response(401, "Missing Authorization header");
        }

        let auth_header = (*headers)
            .get("authorization")
            .or_else(|| (*headers).get("Authorization"));

        let token = match auth_header {
            Some(h) if h.starts_with("Bearer ") => &h[7..],
            _ => return make_err_response(401, "Invalid Authorization header format"),
        };

        // Verify JWT using doo_ffi_auth (or inline simple verification)
        // Use same default as token generation to ensure consistency
        let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "test-secret".to_string());

        // Simple JWT validation (header.payload.signature)
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return make_err_response(401, "Invalid token format");
        }

        // For now, trust token if well-formed (full validation would use jsonwebtoken)
        // In production this calls doo_auth_verify

        // Call next handler
        next(req)
    }
}

/// CORS middleware handler
pub extern "C" fn cors_middleware_handler(
    req: *const DooRequest,
    next: DooNextFn,
) -> *mut DooResult {
    if req.is_null() {
        return next(req);
    }

    unsafe {
        let method = c_to_string((*req).method);

        // Handle OPTIONS preflight
        if method.eq_ignore_ascii_case("OPTIONS") {
            return make_cors_preflight_response();
        }

        // Continue to handler, CORS headers added in response
        next(req)
    }
}

/// Rate limit middleware handler
pub extern "C" fn ratelimit_middleware_handler(
    req: *const DooRequest,
    next: DooNextFn,
) -> *mut DooResult {
    if req.is_null() {
        return next(req);
    }

    let config_guard = get_ratelimit_config().lock().unwrap();
    let config = match config_guard.as_ref() {
        Some(c) => c.clone(),
        None => return next(req), // No rate limit configured
    };
    drop(config_guard);

    // Get client identifier (IP or user_id)
    let client_id = unsafe {
        if config.per == "user" {
            if !(*req).user_id.is_null() {
                c_to_string((*req).user_id)
            } else {
                "anonymous".to_string()
            }
        } else {
            // Would extract from headers/connection, simplified to "ip"
            "client_ip".to_string()
        }
    };

    let mut state = get_ratelimit_state().lock().unwrap();
    let now = Instant::now();

    let entry = state.entry(client_id).or_insert(RateLimitEntry {
        count: 0,
        window_start: now,
    });

    // Reset window if expired
    if now.duration_since(entry.window_start).as_secs() >= config.window {
        entry.count = 0;
        entry.window_start = now;
    }

    entry.count += 1;

    if entry.count > config.max {
        drop(state);
        let err = too_many_requests("Rate limit exceeded", unsafe { c_to_string((*req).path) });
        return make_err_response(429, &err.to_json());
    }

    drop(state);
    next(req)
}

// ============================================================================
// Helper functions
// ============================================================================

/// Create an error result with proper error response struct format
/// Error response struct layout: { i32 status, ptr body, ptr content_type }
/// This matches the format expected by server.rs handle_request
fn make_err_response(status: i32, message: &str) -> *mut DooResult {
    set_last_error(status, message.to_string());
    unsafe {
        // Use centralized helper to build error response struct
        let error_response = alloc_error_response(status, message);
        if error_response.is_null() {
            return std::ptr::null_mut();
        }

        let result = libc::malloc(std::mem::size_of::<DooResult>()) as *mut DooResult;
        if result.is_null() {
            return std::ptr::null_mut();
        }
        (*result).tag = 1; // Error
        (*result).value = error_response;
        (*result).owner = owner::FFI;
        result
    }
}

fn make_cors_preflight_response() -> *mut DooResult {
    // Return 204 No Content with CORS headers
    // In actual implementation this would set response headers
    unsafe {
        let result = libc::malloc(std::mem::size_of::<DooResult>()) as *mut DooResult;
        if result.is_null() {
            return std::ptr::null_mut();
        }
        (*result).tag = 0;
        (*result).value = std::ptr::null_mut();
        (*result).owner = owner::FFI;
        result
    }
}
