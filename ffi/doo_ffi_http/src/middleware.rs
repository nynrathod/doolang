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
            data: Option<String>,
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
        // First try direct user_id claim (>0 — 0 is sentinel for "not set", e.g. OAuth tokens),
        // then fallback to DB lookup by sub (email), then auto-create from JWT data claim.
        let user_id = match token_data.claims.user_id.filter(|&uid| uid > 0) {
            Some(uid) => uid,
            None => {
                if let Some(ref email) = token_data.claims.sub {
                    match lookup_user_id_by_email(email) {
                        Some(id) => id,
                        None => {
                            // User not in DB — auto-create from JWT data (OAuth users)
                            match ensure_user_in_db(email, token_data.claims.data.as_deref()) {
                                Some(id) => id,
                                None => return make_err_response(401, "User not found"),
                            }
                        }
                    }
                } else {
                    return make_err_response(401, "Could not extract user ID from token");
                }
            }
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

/// Look up user ID by email from the database.
/// Uses the centralized db_bridge for cached DB library access.
/// Gets auth table name from configuration (set by app.auth()).
fn lookup_user_id_by_email(email: &str) -> Option<i64> {
    // Get auth table name dynamically from auth config
    let table_name = crate::auth::get_auth_table_name().unwrap_or_else(|| "users".to_string());

    let sql = format!("SELECT id FROM {} WHERE email = $1 LIMIT 1", table_name);
    let result_json = crate::db_bridge::execute_db_query_with_string_param(&sql, email).ok()?;

    let result: serde_json::Value = serde_json::from_str(&result_json).ok()?;
    result.as_array()?.first()?.get("id")?.as_i64()
}

/// Auto-create a user in the DB from JWT data claim (for OAuth users).
///
/// OAuth tokens carry user info in the `data` JSON string (name, email, avatar, provider)
/// but don't insert users into the local DB table. This function upserts the user
/// by discovering the table schema at runtime — no hardcoded column names.
///
/// Uses centralized db_bridge for cached DB library access.
fn ensure_user_in_db(email: &str, data_json: Option<&str>) -> Option<i64> {
    // Get auth table name from config
    let table_name = crate::auth::get_auth_table_name().unwrap_or_else(|| "users".to_string());

    // Parse JWT data claim into a key-value map
    let data_map: HashMap<String, serde_json::Value> = data_json
        .and_then(|d| serde_json::from_str(d).ok())
        .unwrap_or_default();

    // Discover which columns actually exist in the table at runtime
    // This is truly generic — works for ANY table schema
    let schema_sql = format!(
        "SELECT column_name, column_default, is_nullable \
         FROM information_schema.columns \
         WHERE table_name = $1 \
         ORDER BY ordinal_position"
    );
    let schema_json =
        crate::db_bridge::execute_db_query_with_string_param(&schema_sql, &table_name).ok()?;

    let column_rows: Vec<serde_json::Value> = serde_json::from_str(&schema_json).ok()?;
    if column_rows.is_empty() {
        return None;
    }

    // Build dynamic column list and values by matching table columns to JWT data
    let mut insert_columns = Vec::new();
    let mut placeholders = Vec::new();
    let mut values: Vec<serde_json::Value> = Vec::new();
    let mut email_col = String::new();
    let mut idx = 1usize;

    for col_row in &column_rows {
        // DB FFI returns PascalCase keys: column_name→ColumnName, column_default→ColumnDefault, is_nullable→IsNullable
        let col_name = col_row
            .get("ColumnName")
            .or_else(|| col_row.get("column_name"))
            .and_then(|v| v.as_str());
        let col_name = match col_name {
            Some(n) => n,
            None => continue, // skip unparseable rows
        };
        let col_default = col_row
            .get("ColumnDefault")
            .or_else(|| col_row.get("column_default"))
            .and_then(|v| v.as_str());
        let is_nullable = col_row
            .get("IsNullable")
            .or_else(|| col_row.get("is_nullable"))
            .and_then(|v| v.as_str())
            == Some("YES");

        // Skip auto-generated columns (serial/identity with nextval default)
        if let Some(def) = col_default {
            if def.contains("nextval") {
                continue;
            }
        }

        // Try to find a value for this column from email param or JWT data
        let value = if crate::metadata::field_names_match(col_name, "email") {
            email_col = col_name.to_string();
            Some(email.to_string())
        } else {
            // Search JWT data keys for a match (case-insensitive field name matching)
            data_map
                .iter()
                .find(|(k, _)| crate::metadata::field_names_match(k, col_name))
                .and_then(|(_, v)| match v {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Bool(b) => Some(b.to_string()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
        };

        match value {
            Some(val) => {
                insert_columns.push(col_name.to_string());
                placeholders.push(format!("${}", idx));
                values.push(serde_json::json!(val));
                idx += 1;
            }
            None => {
                // Column has no matching data — skip if nullable or has default
                if is_nullable || col_default.is_some() {
                    // Skip — DB will use default/null
                } else {
                    // NOT NULL without default — insert empty string to avoid constraint violation
                    insert_columns.push(col_name.to_string());
                    placeholders.push(format!("${}", idx));
                    values.push(serde_json::json!(""));
                    idx += 1;
                }
            }
        }
    }

    if insert_columns.is_empty() || email_col.is_empty() {
        return None;
    }

    // Build INSERT ... ON CONFLICT DO NOTHING
    let insert_sql = format!(
        "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT ({}) DO NOTHING",
        table_name,
        insert_columns.join(", "),
        placeholders.join(", "),
        email_col,
    );

    // Execute the INSERT via centralized db_bridge
    let _ = crate::db_bridge::execute_db_insert(&insert_sql, &values);

    // Now look up the user ID (the INSERT may have been a no-op if user already existed)
    let select_sql = format!(
        "SELECT id FROM {} WHERE {} = $1 LIMIT 1",
        table_name, email_col
    );
    let select_json =
        crate::db_bridge::execute_db_query_with_string_param(&select_sql, email).ok()?;

    let result: serde_json::Value = serde_json::from_str(&select_json).ok()?;
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
pub fn log_request(method: &str, path: &str, status: u16, duration_us: u64) {
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
    // < 1ms → µs, < 1s → ms, >= 1s → seconds
    let dur_str = if duration_us < 1_000 {
        format!("{:>4}µs", duration_us)
    } else if duration_us < 1_000_000 {
        format!("{:>5.1}ms", duration_us as f64 / 1_000.0)
    } else {
        format!("{:>5.1}s ", duration_us as f64 / 1_000_000.0)
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
