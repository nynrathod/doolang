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
/// Validates the JWT token and extracts the user ID, setting it on the request
/// for handlers that need authenticated user information.
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

        // JWT format: header.payload.signature (base64 encoded)
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return make_err_response(401, "Invalid token format");
        }

        // Decode the payload (middle part) to extract user ID
        // JWT payload is base64url encoded JSON
        let payload_b64 = parts[1];
        let payload_json = match base64_url_decode(payload_b64) {
            Ok(json) => json,
            Err(_) => return make_err_response(401, "Invalid token payload encoding"),
        };

        // Parse the payload JSON to extract user ID
        // JWT payload typically has: { "sub": "email@example.com", "iat": ..., "exp": ... }
        // We need to look up the user ID from the database based on the subject (email)
        let user_id = match extract_user_id_from_jwt_payload(&payload_json) {
            Some(id) => id,
            None => return make_err_response(401, "Could not extract user ID from token"),
        };

        // Set the user_id on the request (as a JSON integer string for consistent parsing)
        // The codegen will use doo_json_parse_int to read this
        let user_id_str = user_id.to_string();
        let req_mut = req as *mut DooRequest;
        (*req_mut).user_id = string_to_c(&user_id_str);

        // Call next handler with the modified request
        next(req)
    }
}

/// Decode base64url encoded string (used in JWT)
fn base64_url_decode(input: &str) -> Result<String, String> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

    // Add padding if needed
    let padded = match input.len() % 4 {
        2 => format!("{}==", input),
        3 => format!("{}=", input),
        _ => input.to_string(),
    };

    URL_SAFE_NO_PAD
        .decode(&padded)
        .or_else(|_| URL_SAFE_NO_PAD.decode(input))
        .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
        .map_err(|e| format!("Base64 decode error: {}", e))
}

/// Extract user ID from JWT payload JSON
/// First tries to get user_id directly from claims, then falls back to database lookup
fn extract_user_id_from_jwt_payload(payload_json: &str) -> Option<i64> {
    let payload: serde_json::Value = serde_json::from_str(payload_json).ok()?;

    // First try to get user_id directly from JWT claims
    if let Some(user_id) = payload.get("user_id").and_then(|v| v.as_i64()) {
        return Some(user_id);
    }

    // Fall back to database lookup by email (for backwards compatibility)
    let email = payload.get("sub")?.as_str()?;
    lookup_user_id_by_email(email)
}

/// Look up user ID by email from the database
fn lookup_user_id_by_email(email: &str) -> Option<i64> {
    use libloading::{Library, Symbol};
    use std::ffi::CString;
    use std::os::raw::c_char;

    // Try to load the database library
    #[cfg(target_os = "windows")]
    let lib_names = ["doo_ffi_db.dll", "libdoo_ffi_db.dll"];
    #[cfg(target_os = "linux")]
    let lib_names = ["libdoo_ffi_db.so", "doo_ffi_db.so"];
    #[cfg(target_os = "macos")]
    let lib_names = ["libdoo_ffi_db.dylib", "doo_ffi_db.dylib"];

    let lib = lib_names
        .iter()
        .find_map(|name| unsafe { Library::new(name).ok() })?;

    // Call doo_db_query_one to find user by email
    type QueryFn = unsafe extern "C" fn(*const c_char, *const c_char) -> *mut std::ffi::c_void;
    let query_fn: Symbol<QueryFn> = unsafe { lib.get(b"doo_db_query_with_params").ok()? };

    let sql = CString::new("SELECT id FROM users WHERE email = $1 LIMIT 1").ok()?;
    let params_json = serde_json::json!([email]).to_string();
    let params = CString::new(params_json).ok()?;

    let result_ptr = unsafe { query_fn(sql.as_ptr(), params.as_ptr()) };
    if result_ptr.is_null() {
        return None;
    }

    // Parse the result - it's a JSON array with rows
    // Each row is an object with column values
    let result_str = c_to_string(result_ptr as *const c_char);
    let result: serde_json::Value = serde_json::from_str(&result_str).ok()?;

    // Get the first row's id field
    result.as_array()?.first()?.get("id")?.as_i64()
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
    // Build a proper RFC 7807 error response using centralized Rfc7807Error
    let path = crate::helpers::get_current_request_path();
    let err = match status {
        401 => Rfc7807Error::unauthorized_with_message(&path, message),
        403 => Rfc7807Error::forbidden_with_message(&path, message),
        429 => Rfc7807Error::rate_limited().with_instance(&path).with_message(message),
        _ => Rfc7807Error::new(status as u16, message).with_instance(&path),
    };
    let json = err.to_json();
    set_last_error(status, json.clone());
    unsafe {
        let error_response = alloc_error_response(status, &json);
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
