//! Middleware Handlers
//! JWT authentication, CORS, and rate limiting middleware implementations.

use crate::error::*;
use crate::helpers::*;
use crate::types::*;
use hyper::header::HeaderValue;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

// Global configurations
static CORS_CONFIG: OnceLock<Mutex<Option<CorsConfig>>> = OnceLock::new();
static RATELIMIT_CONFIG: OnceLock<Mutex<Option<RateLimitConfig>>> = OnceLock::new();
static RATELIMIT_STATE: OnceLock<Mutex<HashMap<String, RateLimitEntry>>> = OnceLock::new();
static JWT_SECRET: OnceLock<String> = OnceLock::new();
static LOGGER_CONFIG: OnceLock<Mutex<Option<LoggerConfig>>> = OnceLock::new();

// ============================================================================
// FROZEN CORS — Pre-computed header values for zero-cost request-time access
// ============================================================================

/// Pre-computed CORS header values as `HeaderValue` (immutable after freeze).
/// Uses dynamic origin reflection — the industry-standard approach for CORS with credentials.
/// Instead of a static origin header, the request's Origin is matched against allowed origins.
pub struct FrozenCorsHeaders {
    /// Allowed origins — matched against the request's Origin header at request time.
    pub allowed_origins: Vec<String>,
    /// True if origins includes "*" (allow all).
    pub allow_all: bool,
    pub methods: HeaderValue,
    pub headers: HeaderValue,
    pub credentials: bool,
    pub max_age: Option<HeaderValue>,
}

/// Frozen CORS headers — lock-free access during request handling
static FROZEN_CORS: OnceLock<Option<FrozenCorsHeaders>> = OnceLock::new();

/// Freeze CORS config into pre-computed `HeaderValue`s.
/// Called once before the accept loop starts.
/// Uses exactly what was configured via `.cors()` or `.cors({...})` — no env var reading.
pub fn freeze_cors() {
    let cors = get_cors_config().lock().ok().and_then(|guard| {
        guard.as_ref().map(|config| {
            // Strip paths from origin URLs (CORS origins must be scheme://host[:port] only)
            let origins: Vec<String> = config
                .origins
                .iter()
                .map(|o| {
                    if o.starts_with("http://") || o.starts_with("https://") {
                        if let Some(scheme_end) = o.find("://") {
                            let after_scheme = &o[scheme_end + 3..];
                            if let Some(path_start) = after_scheme.find('/') {
                                return o[..scheme_end + 3 + path_start].to_string();
                            }
                        }
                    }
                    o.clone()
                })
                .collect();

            // If any CORS origin uses http:// (not https://), inform the cookie
            // system that Secure flag should be omitted. This is a runtime signal
            // that doesn't depend on DOO_DEV env var propagation.
            let has_http_origin = origins.iter().any(|o| o.starts_with("http://"));
            if has_http_origin && config.credentials {
                doo_ffi_core::cookies::set_insecure_cookies(true);
            }

            let allow_all = origins.iter().any(|o| o == "*");
            let methods_str = config.methods.join(", ");
            let headers_str = config.headers.join(", ");

            FrozenCorsHeaders {
                allowed_origins: origins,
                allow_all,
                methods: HeaderValue::from_str(&methods_str)
                    .unwrap_or_else(|_| HeaderValue::from_static("GET, POST, OPTIONS")),
                headers: HeaderValue::from_str(&headers_str)
                    .unwrap_or_else(|_| HeaderValue::from_static("Content-Type")),
                credentials: config.credentials,
                max_age: config
                    .max_age
                    .and_then(|ma| HeaderValue::from_str(&ma.to_string()).ok()),
            }
        })
    });
    let _ = FROZEN_CORS.set(cors);
}

/// Get frozen CORS headers (zero-cost, no lock)
#[inline]
pub fn get_frozen_cors() -> Option<&'static FrozenCorsHeaders> {
    FROZEN_CORS.get().and_then(|opt| opt.as_ref())
}

impl FrozenCorsHeaders {
    /// Get the correct `Access-Control-Allow-Origin` header value for a given request origin.
    /// This implements dynamic origin reflection — the industry standard for CORS with credentials.
    ///
    /// Rules:
    /// - If allow_all and no credentials: returns "*"
    /// - If allow_all and credentials: reflects the request origin (wildcard + credentials is invalid per spec)
    /// - If specific origins: reflects the request origin only if it matches the allowed list
    /// - If no match: returns None (CORS headers should not be set)
    pub fn get_origin_for_request(&self, request_origin: &str) -> Option<HeaderValue> {
        if self.allow_all {
            if self.credentials && !request_origin.is_empty() {
                // Wildcard + credentials: reflect the request origin
                HeaderValue::from_str(request_origin).ok()
            } else {
                Some(HeaderValue::from_static("*"))
            }
        } else if request_origin.is_empty() {
            // No Origin header (same-origin request) — use first allowed origin
            self.allowed_origins
                .first()
                .and_then(|o| HeaderValue::from_str(o).ok())
        } else if self.allowed_origins.iter().any(|o| o == request_origin) {
            // Origin matches allowed list — reflect it
            HeaderValue::from_str(request_origin).ok()
        } else {
            // Origin not in allowed list — no CORS headers
            None
        }
    }
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

        // Strategy 1: Authorization: Bearer <token> header
        let auth_header = (*headers)
            .get("authorization")
            .or_else(|| (*headers).get("Authorization"));

        let token_from_header = match auth_header {
            Some(h) if h.starts_with("Bearer ") => Some(&h[7..]),
            _ => None,
        };

        // Strategy 2: Cookie fallback — read access token from httpOnly cookie
        // This is the CORE auth pattern: auth sets cookies, middleware reads them
        let cookie_token_owned: Option<String> = if token_from_header.is_none() {
            let cookie_header = (*headers)
                .get("cookie")
                .or_else(|| (*headers).get("Cookie"));
            cookie_header.and_then(|h| {
                doo_ffi_core::cookies::extract_cookie_value(
                    h,
                    doo_ffi_core::cookies::COOKIE_ACCESS_TOKEN,
                )
                .map(|s| s.to_string())
            })
        } else {
            None
        };

        // Use header token first, then cookie token
        let token = match token_from_header.or(cookie_token_owned.as_deref()) {
            Some(t) => t,
            None => return make_err_response(401, "No authentication token provided"),
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

        // Also inject user_id into request headers so handlers can use
        // req.header("X-User-Id") to retrieve the authenticated user's ID.
        // This enables multi-param handlers (e.g., needing both userId and body)
        // without requiring compiler changes for multi-param JWT injection.
        let headers_ptr = (*req_mut).headers as *mut HashMap<String, String>;
        if !headers_ptr.is_null() {
            (*headers_ptr).insert("x-user-id".to_string(), user_id_str.clone());
        } else {
            let mut map = HashMap::new();
            map.insert("x-user-id".to_string(), user_id_str.clone());
            (*req_mut).headers = Box::into_raw(Box::new(map)) as *mut std::ffi::c_void;
        }

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
// LOGGER — Server-level request/response logging
// ============================================================================

pub fn get_logger_config() -> &'static Mutex<Option<LoggerConfig>> {
    LOGGER_CONFIG.get_or_init(|| Mutex::new(None))
}

/// Frozen logger config — immutable after freeze, zero-lock access at request time.
/// `None` = logger disabled (no `app.logger()` call).
/// `Some(config)` = logger enabled with the specified level filters.
static FROZEN_LOGGER: OnceLock<Option<LoggerConfig>> = OnceLock::new();

/// Freeze logger config into lock-free static.
/// Called once before the accept loop starts, alongside freeze_cors()/freeze_routes().
pub fn freeze_logger() {
    let config = get_logger_config()
        .lock()
        .ok()
        .and_then(|guard| guard.clone());
    let _ = FROZEN_LOGGER.set(config);
}

/// Get frozen logger config (zero-cost, no lock).
/// Returns `None` if logger was never enabled.
#[inline]
pub fn get_frozen_logger() -> Option<&'static LoggerConfig> {
    FROZEN_LOGGER.get().and_then(|opt| opt.as_ref())
}

/// Check if logger is enabled (lock-free after freeze).
#[inline]
pub fn is_logger_enabled() -> bool {
    FROZEN_LOGGER
        .get()
        .map(|opt| opt.is_some())
        .unwrap_or(false)
}

/// Log a request/response in the Doo format.
/// Format: `[Doo] HH:MM:SS | STATUS | DURATIONms | METHOD /path`
/// Color-coded: green=2xx, yellow=4xx, red=5xx, cyan=3xx
///
/// Level classification:
/// - Info:  1xx, 2xx, 3xx (success / redirect)
/// - Warn:  4xx (client error)
/// - Error: 5xx (server error)
///
/// This is intentionally NOT a middleware — it runs at the server level
/// to avoid adding function pointer overhead to the middleware chain.
/// Cost: only the string formatting + eprintln when the level is enabled.
#[inline]
pub fn log_request(method: &str, path: &str, status: u16, duration_ms: u64) {
    let config = match get_frozen_logger() {
        Some(c) => c,
        None => return,
    };

    // Classify by status code — generic, no hardcoded status values
    let is_info = status < 400;
    let is_warn = (400..500).contains(&status);
    let is_error = status >= 500;

    // Check if this level is enabled
    if (is_info && !config.info) || (is_warn && !config.warn) || (is_error && !config.error) {
        return;
    }

    // Get current time HH:MM:SS
    // Use system time directly — no chrono dependency needed
    let now = std::time::SystemTime::now();
    let since_epoch = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = since_epoch.as_secs();
    let hours = (total_secs % 86400) / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    // ANSI color: follows NO_COLOR standard (https://no-color.org/)
    // Whole line is colored like Gin/Fiber/Express — not just the status code
    use std::io::IsTerminal;
    let use_color = std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal();

    let (color, reset) = if use_color {
        let c = if is_error {
            "\x1b[31m" // red   — 5xx
        } else if is_warn {
            "\x1b[33m" // yellow — 4xx
        } else if status >= 300 {
            "\x1b[36m" // cyan  — 3xx
        } else {
            "\x1b[32m" // green — 2xx
        };
        (c, "\x1b[0m")
    } else {
        ("", "")
    };

    // Right-align duration for clean columns
    let dur_str = if duration_ms < 1000 {
        format!("{:>4}ms", duration_ms)
    } else {
        format!("{:>4.1}s ", duration_ms as f64 / 1000.0)
    };

    // Format: [Doo] HH:MM:SS | STATUS | DURATION | METHOD /path
    // Entire line colored by status level (like Gin/Fiber/Express)
    eprintln!(
        "{}[Doo] {:02}:{:02}:{:02} | {:>3} | {} | {} {}{}",
        color, hours, minutes, seconds, status, dur_str, method, path, reset
    );
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
