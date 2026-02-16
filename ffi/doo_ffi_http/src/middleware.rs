//! Middleware Handlers
//! JWT authentication, CORS, and rate limiting middleware implementations.

use crate::error::*;
use crate::helpers::*;
use crate::types::*;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

// Global configurations
static CORS_CONFIG: OnceLock<Mutex<Option<CorsConfig>>> = OnceLock::new();
static RATELIMIT_CONFIG: OnceLock<Mutex<Option<RateLimitConfig>>> = OnceLock::new();
static RATELIMIT_STATE: OnceLock<Mutex<HashMap<String, RateLimitEntry>>> = OnceLock::new();
static JWT_SECRET: OnceLock<String> = OnceLock::new();

// ============================================================================
// FROZEN CORS — Pre-computed header values for zero-cost request-time access
// ============================================================================

/// Pre-computed CORS header values (immutable after freeze)
pub struct FrozenCorsHeaders {
    pub origin: String,
    pub methods: String,
    pub headers: String,
    pub credentials: bool,
    pub max_age: Option<String>,
}

/// Frozen CORS headers — lock-free access during request handling
static FROZEN_CORS: OnceLock<Option<FrozenCorsHeaders>> = OnceLock::new();

/// Freeze CORS config into pre-computed header values.
/// Called once before the accept loop starts.
pub fn freeze_cors() {
    let cors = get_cors_config().lock().ok().and_then(|guard| {
        guard.as_ref().map(|config| FrozenCorsHeaders {
            origin: config.origins.join(", "),
            methods: config.methods.join(", "),
            headers: config.headers.join(", "),
            credentials: config.credentials,
            max_age: config.max_age.map(|ma| ma.to_string()),
        })
    });
    let _ = FROZEN_CORS.set(cors);
}

/// Get frozen CORS headers (zero-cost, no lock)
#[inline]
pub fn get_frozen_cors() -> Option<&'static FrozenCorsHeaders> {
    FROZEN_CORS.get().and_then(|opt| opt.as_ref())
}

/// Check if CORS is configured (lock-free after freeze)
#[inline]
pub fn has_frozen_cors() -> bool {
    FROZEN_CORS.get().map(|opt| opt.is_some()).unwrap_or(false)
}

pub fn get_cors_config() -> &'static Mutex<Option<CorsConfig>> {
    CORS_CONFIG.get_or_init(|| Mutex::new(None))
}

pub fn get_ratelimit_config() -> &'static Mutex<Option<RateLimitConfig>> {
    RATELIMIT_CONFIG.get_or_init(|| Mutex::new(None))
}

pub fn get_ratelimit_state() -> &'static Mutex<HashMap<String, RateLimitEntry>> {
    RATELIMIT_STATE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Get JWT secret — reads from env once, caches in OnceLock.
/// Returns empty string if JWT_SECRET is not set.
pub fn get_jwt_secret() -> &'static str {
    JWT_SECRET
        .get_or_init(|| std::env::var(doo_ffi_core::constants::ENV_JWT_SECRET).unwrap_or_default())
        .as_str()
}

/// JWT middleware handler
/// Validates the JWT token signature + expiration, extracts the user ID,
/// and sets it on the request for handlers that need authenticated user info.
/// SAFETY: Wrapped in catch_unwind to prevent panics across FFI boundary.
pub extern "C" fn jwt_middleware_handler(
    req: *const DooRequest,
    next: DooNextFn,
) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        jwt_middleware_inner(req, next)
    })) {
        Ok(result) => result,
        Err(payload) => doo_ffi_core::helpers::make_panic_err("HTTP.jwt_middleware", payload),
    }
}

/// Inner JWT middleware logic (separated for catch_unwind wrapping)
fn jwt_middleware_inner(req: *const DooRequest, next: DooNextFn) -> *mut DooResult {
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

        // Token size limit — prevent DoS via oversized tokens
        if token.len() > 8192 {
            return make_err_response(401, "Token too large");
        }

        // Get JWT secret — MUST be set, no fallback
        let secret = get_jwt_secret();
        if secret.is_empty() {
            return make_err_response(500, "JWT_SECRET not configured");
        }

        // VERIFY JWT SIGNATURE + EXPIRATION using jsonwebtoken crate
        // This is the critical fix — previously did ZERO verification
        use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};

        #[derive(serde::Deserialize)]
        struct JwtClaims {
            sub: Option<String>,
            user_id: Option<i64>,
            exp: Option<usize>,
        }

        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_required_spec_claims(&["exp", "sub"]);
        validation.leeway = 30; // 30s clock skew tolerance

        let decoding_key = DecodingKey::from_secret(secret.as_bytes());
        let token_data = match decode::<JwtClaims>(token, &decoding_key, &validation) {
            Ok(data) => data,
            Err(e) => {
                let err_str = e.to_string().to_lowercase();
                if err_str.contains("expired") {
                    return make_err_response(401, "Token has expired");
                } else if err_str.contains("signature") {
                    return make_err_response(401, "Invalid token signature");
                } else {
                    return make_err_response(401, "Invalid token");
                }
            }
        };

        // Extract user_id from verified claims
        // First try direct user_id claim, then fallback to DB lookup by sub (email)
        let user_id = if let Some(uid) = token_data.claims.user_id {
            uid
        } else if let Some(ref email) = token_data.claims.sub {
            match lookup_user_id_by_email(email) {
                Some(id) => id,
                None => return make_err_response(401, "User not found"),
            }
        } else {
            return make_err_response(401, "Could not extract user ID from token");
        };

        // Set the user_id on the request
        let user_id_str = user_id.to_string();
        let req_mut = req as *mut DooRequest;
        // CRITICAL: Free old user_id if it was previously set (e.g., by prior middleware)
        // to prevent memory leak per request in multi-middleware chains.
        if !(*req_mut).user_id.is_null() {
            doo_ffi_core::doo_free((*req_mut).user_id as *mut u8);
        }
        (*req_mut).user_id = string_to_c(&user_id_str);

        // Call next handler with the verified request
        next(req)
    }
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

    // CRITICAL: Free the DB result pointer to prevent leak on every JWT-authenticated request
    doo_ffi_core::doo_free(result_ptr as *mut u8);

    let result: serde_json::Value = serde_json::from_str(&result_str).ok()?;

    // Get the first row's id field
    result.as_array()?.first()?.get("id")?.as_i64()
}

/// CORS middleware handler
/// SAFETY: Wrapped in catch_unwind to prevent panics across FFI boundary.
pub extern "C" fn cors_middleware_handler(
    req: *const DooRequest,
    next: DooNextFn,
) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
    })) {
        Ok(result) => result,
        Err(payload) => doo_ffi_core::helpers::make_panic_err("HTTP.cors_middleware", payload),
    }
}

/// Rate limit middleware handler
/// SAFETY: Wrapped in catch_unwind to prevent panics across FFI boundary.
pub extern "C" fn ratelimit_middleware_handler(
    req: *const DooRequest,
    next: DooNextFn,
) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ratelimit_middleware_inner(req, next)
    })) {
        Ok(result) => result,
        Err(payload) => doo_ffi_core::helpers::make_panic_err("HTTP.ratelimit_middleware", payload),
    }
}

/// Inner rate limit logic (separated for catch_unwind wrapping)
fn ratelimit_middleware_inner(req: *const DooRequest, next: DooNextFn) -> *mut DooResult {
    if req.is_null() {
        return next(req);
    }

    let config_guard = get_ratelimit_config()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
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

    let mut state = get_ratelimit_state()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
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
        429 => Rfc7807Error::rate_limited()
            .with_instance(&path)
            .with_message(message),
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
        std::ptr::write(result, DooResult::err(status as u16, error_response, 0));
        result
    }
}

fn make_cors_preflight_response() -> *mut DooResult {
    // Return 204 No Content with CORS headers
    DooResult::ok_empty().into_raw()
}
