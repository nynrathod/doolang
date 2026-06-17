//! doo_ffi_auth — Production-Grade Authentication FFI Library
//!
//! ## Architecture
//!
//! ```text
//! Doo program → FFI (this crate) → AuthStrategy trait → strategies::jwt / strategies::oauth / ...
//! ```
//!
//! ## Generic (always available):
//! - Password hashing with bcrypt (cost 12, production-grade)
//! - JWT session token signing/verification (via session module — single source of truth)
//! - Result inspection and memory management
//!
//! ## Strategy-dispatched:
//! - Token signing → dispatched through registered AuthStrategy
//! - Token verification → dispatched through registered AuthStrategy
//!
//! ## OAuth-specific:
//! - OAuth provider initialization (Google, GitHub)
//! - Authorization URL generation (with PKCE + CSRF)
//! - Code exchange for tokens + user info
//! - Token refresh and revocation
//!
//! ## Adding a new auth strategy
//!
//! 1. Create `src/strategies/<name>/mod.rs` implementing `AuthStrategy`
//! 2. Register in `src/strategies/mod.rs`
//! 3. Done — zero compiler/codegen changes
//!
//! ## Security:
//! - JWT_SECRET must be set (no fallback)
//! - JWT_SECRET must be >= 32 bytes (HMAC-SHA256 requirement)
//! - bcrypt cost = 12 (DEFAULT_COST)
//! - All FFI functions wrapped in catch_unwind
//! - Password zeroization after use
//! - OAuth uses PKCE (S256) and CSRF state tokens

mod error;
pub mod session;
pub mod strategies;
pub mod strategy;

use std::os::raw::c_char;

use bcrypt::{hash, verify, DEFAULT_COST};
use doo_ffi_core::helpers::c_to_string;
use doo_ffi_core::{AuthErrorCode, DooResult};

pub use error::AuthError;

// ============================================================================
// RESULT HELPERS — Consistent libc::malloc allocation (no Box mismatch)
// ============================================================================

/// Create an Ok result with a string value.
/// Uses doo_ffi_core::helpers (single source of truth).
fn make_ok_string(s: &str) -> *mut DooResult {
    doo_ffi_core::helpers::make_ok_string(s)
}

/// Create an Ok result with a boolean value.
/// Uses doo_ffi_core::helpers (single source of truth).
fn make_ok_bool(b: bool) -> *mut DooResult {
    doo_ffi_core::helpers::make_ok_bool(b)
}

/// Create an Err result.
/// Uses doo_ffi_core::helpers (single source of truth).
fn make_err(code: AuthErrorCode, message: &str) -> *mut DooResult {
    doo_ffi_core::helpers::make_err_rfc7807(code as u16, message)
}

// ============================================================================
// PASSWORD HASHING — bcrypt cost 12 (DEFAULT_COST), with zeroization
// ============================================================================

/// Hash a password using bcrypt at production cost (12).
/// Zeroizes plaintext password after hashing.
/// Wrapped in catch_unwind for panic safety at FFI boundary.
///
/// Returns: DooResult with hashed password string on success
#[no_mangle]
pub extern "C" fn doo_auth_hash_password(password: *const c_char) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut pwd = match c_to_string(password) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        if pwd.is_empty() {
            return make_err(AuthErrorCode::PasswordTooWeak, "Password cannot be empty");
        }

        // Hash with DEFAULT_COST (12) — production-grade security
        let result = match hash(&pwd, DEFAULT_COST) {
            Ok(hashed) => make_ok_string(&hashed),
            Err(e) => make_err(AuthErrorCode::InternalError, &format!("Hash failed: {}", e)),
        };

        // Zeroize plaintext password from memory
        unsafe {
            let bytes = pwd.as_bytes_mut();
            for b in bytes.iter_mut() {
                std::ptr::write_volatile(b, 0);
            }
        }
        drop(pwd);

        result
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "Internal error"),
    }
}

/// Verify a password against a bcrypt hash.
/// Zeroizes plaintext password after verification.
/// Wrapped in catch_unwind for panic safety at FFI boundary.
///
/// Returns: DooResult with boolean value (0/1) indicating match
#[no_mangle]
pub extern "C" fn doo_auth_verify_password(
    password: *const c_char,
    hashed: *const c_char,
) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut pwd = match c_to_string(password) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        let hash_str = match c_to_string(hashed) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        let result = match verify(&pwd, &hash_str) {
            Ok(valid) => make_ok_bool(valid),
            Err(e) => make_err(
                AuthErrorCode::InternalError,
                &format!("Verify failed: {}", e),
            ),
        };

        // Zeroize plaintext password
        unsafe {
            let bytes = pwd.as_bytes_mut();
            for b in bytes.iter_mut() {
                std::ptr::write_volatile(b, 0);
            }
        }
        drop(pwd);

        result
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "Internal error"),
    }
}

// ============================================================================
// JWT OPERATIONS — dispatched through AuthStrategy
// ============================================================================

/// Auto-initialize JWT strategy if not already registered.
/// This ensures backward compatibility — first sign/verify call auto-inits JWT.
fn ensure_strategy_initialized() {
    if !strategy::is_strategy_registered() {
        #[cfg(feature = "jwt")]
        {
            let _ = strategies::jwt::init();
        }
    }
}

/// Sign a JWT token via the registered auth strategy.
///
/// Parameters:
/// - sub: Subject (typically email)
/// - data_json: Optional JSON string for additional claims
/// - expires_seconds: Token lifetime in seconds
/// Returns: DooResult with JWT token string on success
#[no_mangle]
pub extern "C" fn doo_auth_sign(
    sub: *const c_char,
    data_json: *const c_char,
    expires_seconds: i64,
) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ensure_strategy_initialized();

        let strat = match strategy::get_strategy() {
            Some(s) => s,
            None => {
                return make_err(
                    AuthErrorCode::SecretNotConfigured,
                    "No auth strategy registered",
                )
            }
        };

        let sub_str = match c_to_string(sub) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        let data = if data_json.is_null() {
            None
        } else {
            match c_to_string(data_json) {
                Ok(s) if !s.is_empty() => Some(s),
                _ => None,
            }
        };

        match strat.sign(&sub_str, data.as_deref(), expires_seconds) {
            Ok(token) => make_ok_string(&token),
            Err(e) => make_err(AuthErrorCode::InternalError, &e),
        }
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "Internal error"),
    }
}

/// Verify a JWT token via the registered auth strategy.
///
/// Returns: DooResult with claims JSON string on success
#[no_mangle]
pub extern "C" fn doo_auth_verify(token: *const c_char) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ensure_strategy_initialized();

        let strat = match strategy::get_strategy() {
            Some(s) => s,
            None => {
                return make_err(
                    AuthErrorCode::SecretNotConfigured,
                    "No auth strategy registered",
                )
            }
        };

        let token_str = match c_to_string(token) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        match strat.verify(&token_str) {
            Ok(claims_json) => make_ok_string(&claims_json),
            Err(e) => {
                let err_lower = e.to_lowercase();
                if err_lower.contains("expired") {
                    make_err(AuthErrorCode::JwtExpired, &e)
                } else if err_lower.contains("signature") {
                    make_err(AuthErrorCode::JwtSignatureInvalid, &e)
                } else {
                    make_err(AuthErrorCode::JwtInvalid, &e)
                }
            }
        }
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "Internal error"),
    }
}

// ============================================================================
// RESULT INSPECTION
// ============================================================================

/// Check if a result is an error
/// Returns: 0 for success, 1 for error
#[no_mangle]
pub extern "C" fn doo_auth_is_error(result: *mut DooResult) -> i32 {
    if result.is_null() {
        return 1;
    }
    unsafe {
        if (*result).is_err() {
            1
        } else {
            0
        }
    }
}

/// Get error message from a result.
/// Correctly reads the inner string from the wrapper struct { *char message }.
/// Returns: Pointer to error message string, or null if not an error
#[no_mangle]
pub extern "C" fn doo_auth_get_error_message(result: *mut DooResult) -> *const c_char {
    if result.is_null() {
        return std::ptr::null();
    }
    unsafe {
        if (*result).is_err() && !(*result).data.is_null() {
            // err_str wraps strings in { *char message } struct
            // Read the inner string pointer from the wrapper
            let wrapper = (*result).data as *const *const c_char;
            if !wrapper.is_null() {
                *wrapper
            } else {
                std::ptr::null()
            }
        } else {
            std::ptr::null()
        }
    }
}

// ============================================================================
// MEMORY MANAGEMENT
// ============================================================================

/// Free a string allocated by this library (libc::malloc)
#[no_mangle]
pub extern "C" fn doo_auth_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        libc::free(ptr as *mut std::ffi::c_void);
    }
}

/// Free a DooResult allocated by this library.
/// All allocations use libc::malloc — freed with libc::free consistently.
/// Handles Err results from err_str: data -> wrapper { *char } -> frees inner string + wrapper.
#[no_mangle]
pub extern "C" fn doo_auth_free_result(result: *mut DooResult) {
    if result.is_null() {
        return;
    }
    unsafe {
        let tag = (*result).tag;
        let data = (*result).data;

        if !data.is_null() {
            if tag == 1 {
                // Err: data -> wrapper { *char message } -> free inner string first
                let inner_str = *(data as *const *mut std::ffi::c_void);
                if !inner_str.is_null() {
                    libc::free(inner_str);
                }
            }
            libc::free(data as *mut std::ffi::c_void);
        }

        // Free the outer DooResult shell (allocated with libc::malloc)
        libc::free(result as *mut std::ffi::c_void);
    }
}

// ============================================================================
// FFI NAME ALIASES — Match codegen registered names
// ============================================================================

/// Alias for doo_auth_sign (codegen may emit doo_auth_sign_token)
#[no_mangle]
pub extern "C" fn doo_auth_sign_token(
    sub: *const c_char,
    data_json: *const c_char,
    expires_seconds: i64,
) -> *mut DooResult {
    doo_auth_sign(sub, data_json, expires_seconds)
}

/// Alias for doo_auth_verify (codegen may emit doo_auth_verify_token with 2 args: token, secret)
/// The secret arg is ignored — we use the centralized JWT_SECRET env var.
#[no_mangle]
pub extern "C" fn doo_auth_verify_token(
    token: *const c_char,
    _secret: *const c_char,
) -> *mut DooResult {
    doo_auth_verify(token)
}

// ============================================================================
// OAUTH FFI FUNCTIONS — OAuth 2.0 with Google, GitHub, etc.
// ============================================================================

/// Initialize OAuth with specified providers.
///
/// config_json: JSON string — array `["google","github"]`, object `{"providers":["google"]}`,
///              or single provider name `"google"`.
///
/// Credentials are read from environment variables:
/// - OAUTH_<PROVIDER>_CLIENT_ID
/// - OAUTH_<PROVIDER>_CLIENT_SECRET
/// - OAUTH_<PROVIDER>_REDIRECT_URI
///
/// Registers OAuthStrategy as the active auth strategy.
/// Returns: DooResult with "ok" on success.
#[cfg(feature = "oauth")]
#[no_mangle]
pub extern "C" fn doo_auth_oauth_init(config_json: *const c_char) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let config = if config_json.is_null() {
            None
        } else {
            match c_to_string(config_json) {
                Ok(s) if !s.is_empty() => Some(s),
                _ => None,
            }
        };

        match strategies::oauth::init(config.as_deref()) {
            Ok(()) => make_ok_string("ok"),
            Err(e) => make_err(AuthErrorCode::InvalidRequest, &e),
        }
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "OAuth init panicked"),
    }
}

/// Get OAuth authorization URL for a provider.
///
/// Generates PKCE challenge and CSRF state automatically.
///
/// provider: Provider name ("google" or "github")
/// Returns: DooResult with JSON `{"url":"https://...","state":"...","provider":"..."}`
#[cfg(feature = "oauth")]
#[no_mangle]
pub extern "C" fn doo_auth_oauth_get_auth_url(provider: *const c_char) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let provider_name = match c_to_string(provider) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        match strategies::oauth::get_auth_url(&provider_name) {
            Ok(json) => make_ok_string(&json),
            Err(e) => make_err(AuthErrorCode::InvalidRequest, &e),
        }
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "OAuth get_auth_url panicked"),
    }
}

/// Exchange an OAuth authorization code for tokens and user info.
///
/// Validates CSRF state, exchanges code via PKCE, fetches user info,
/// and creates a Doo session JWT.
///
/// provider: Provider name ("google" or "github")
/// code: Authorization code from the OAuth callback
/// state: CSRF state from the OAuth callback
/// Returns: DooResult with JSON containing tokens, user info, and session_token
#[cfg(feature = "oauth")]
#[no_mangle]
pub extern "C" fn doo_auth_oauth_exchange(
    provider: *const c_char,
    code: *const c_char,
    state: *const c_char,
) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let provider_name = match c_to_string(provider) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };
        let code_str = match c_to_string(code) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };
        let state_str = match c_to_string(state) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        match strategies::oauth::exchange_code(&provider_name, &code_str, &state_str) {
            Ok(json) => make_ok_string(&json),
            Err(e) => {
                let err_lower = e.to_lowercase();
                if err_lower.contains("invalid_grant") || err_lower.contains("invalid_code") {
                    make_err(AuthErrorCode::InvalidGrant, &e)
                } else if err_lower.contains("state") {
                    make_err(AuthErrorCode::InvalidRequest, &e)
                } else {
                    make_err(AuthErrorCode::InternalError, &e)
                }
            }
        }
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "OAuth exchange panicked"),
    }
}

/// Refresh an OAuth access token using a refresh token.
///
/// provider: Provider name ("google" or "github")
/// refresh_token: Refresh token from a previous token exchange
/// Returns: DooResult with JSON containing new token response
#[cfg(feature = "oauth")]
#[no_mangle]
pub extern "C" fn doo_auth_oauth_refresh(
    provider: *const c_char,
    refresh_token: *const c_char,
) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let provider_name = match c_to_string(provider) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };
        let token = match c_to_string(refresh_token) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        match strategies::oauth::refresh(&provider_name, &token) {
            Ok(json) => make_ok_string(&json),
            Err(e) => make_err(AuthErrorCode::InvalidGrant, &e),
        }
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "OAuth refresh panicked"),
    }
}

/// Get user info from an OAuth provider using an access token.
///
/// provider: Provider name ("google" or "github")
/// access_token: Access token from a previous token exchange
/// Returns: DooResult with JSON containing normalized user info
#[cfg(feature = "oauth")]
#[no_mangle]
pub extern "C" fn doo_auth_oauth_get_user(
    provider: *const c_char,
    access_token: *const c_char,
) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let provider_name = match c_to_string(provider) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };
        let token = match c_to_string(access_token) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        match strategies::oauth::get_user_info(&provider_name, &token) {
            Ok(json) => make_ok_string(&json),
            Err(e) => make_err(AuthErrorCode::InvalidRequest, &e),
        }
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "OAuth get_user panicked"),
    }
}

/// Revoke an OAuth token with a provider.
///
/// provider: Provider name ("google" or "github")
/// token: Token to revoke (access or refresh token)
/// Returns: DooResult with "ok" on success
#[cfg(feature = "oauth")]
#[no_mangle]
pub extern "C" fn doo_auth_oauth_revoke(
    provider: *const c_char,
    token: *const c_char,
) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let provider_name = match c_to_string(provider) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };
        let token_str = match c_to_string(token) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        match strategies::oauth::revoke(&provider_name, &token_str) {
            Ok(()) => make_ok_string("ok"),
            Err(e) => make_err(AuthErrorCode::InvalidRequest, &e),
        }
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "OAuth revoke panicked"),
    }
}

// ============================================================================
// OAUTH HTTP SETUP — app.oauth(config) entry point
// ============================================================================

/// Set up OAuth and register HTTP routes for login via external providers.
///
/// This is the FFI entry point for `app.oauth(config)` in Doo. It:
/// 1. Parses config JSON for provider list
/// 2. Initializes OAuth providers (reads env vars for credentials)
/// 3. Registers redirect + callback routes via HTTP FFI's generic registration
///
/// The routes are registered at runtime via `dlsym`/`GetProcAddress` — zero coupling with doo_ffi_http.
///
/// options: Doo map pointer (HashMap<String, String>) with keys:
///   - "Providers": comma-separated provider names, e.g. "Google,GitHub"
///   - "BasePath": optional base path, default "/auth"
///
/// This follows the same pattern as doo_http_cors_custom.
/// Returns: DooResult with "ok" on success
#[cfg(feature = "oauth")]
#[no_mangle]
pub extern "C" fn doo_auth_oauth_setup(
    _server: *const std::ffi::c_void,
    options: *mut std::ffi::c_void,
) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if options.is_null() {
            return make_err(AuthErrorCode::InvalidRequest, "OAuth config is required");
        }

        // Read map as HashMap<String, String> (same layout as Doo map objects)
        let map = unsafe { &*(options as *const std::collections::HashMap<String, String>) };

        // Read "Providers" key — comma-separated provider names
        let providers_str = match map.get("Providers").or_else(|| map.get("providers")) {
            Some(s) if !s.is_empty() => s.clone(),
            _ => {
                return make_err(
                    AuthErrorCode::InvalidRequest,
                    "'Providers' key is required in OAuth config",
                )
            }
        };

        let providers: Vec<String> = providers_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if providers.is_empty() {
            return make_err(
                AuthErrorCode::InvalidRequest,
                "At least one OAuth provider must be specified",
            );
        }

        // Read optional "BasePath" key
        let base_path = map
            .get("BasePath")
            .or_else(|| map.get("basePath"))
            .or_else(|| map.get("base_path"))
            .cloned()
            .unwrap_or_else(|| "/auth".to_string());

        // Read optional expiry overrides → set as env vars so session module picks them up.
        // These take priority over env vars set externally (code config > env > defaults).
        if let Some(v) = map.get("AccessExpiry").or_else(|| map.get("accessExpiry")) {
            std::env::set_var(doo_ffi_core::constants::ENV_ACCESS_TOKEN_EXPIRY, v);
        }
        if let Some(v) = map
            .get("RefreshExpiry")
            .or_else(|| map.get("refreshExpiry"))
        {
            std::env::set_var(doo_ffi_core::constants::ENV_REFRESH_TOKEN_EXPIRY, v);
        }

        // Store base path for cookie path scoping (used by refresh handler + FFI functions)
        std::env::set_var(doo_ffi_core::constants::ENV_AUTH_BASE_PATH, &base_path);

        // Read optional "CallbackUrl" key — where to redirect after successful OAuth login.
        // Falls back to OAUTH_CALLBACK_URL env var if not set in config.
        let callback_url = map
            .get("CallbackUrl")
            .or_else(|| map.get("callbackUrl"))
            .or_else(|| map.get("callback_url"))
            .cloned()
            .or_else(|| std::env::var("OAUTH_CALLBACK_URL").ok());

        // Read optional "WebhooksJson" key — JSON array of WebhookConfig objects.
        // Passed to the generic webhook engine via cross-DLL bridge (same pattern as cookies).
        let webhooks_json = map
            .get("WebhooksJson")
            .or_else(|| map.get("webhooksJson"))
            .or_else(|| map.get("webhooks_json"))
            .cloned();

        match strategies::oauth::http_handlers::setup_from_map(&providers, &base_path, callback_url.as_deref(), webhooks_json.as_deref()) {
            Ok(()) => make_ok_string("ok"),
            Err(e) => make_err(AuthErrorCode::InvalidRequest, &e),
        }
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "OAuth setup panicked"),
    }
}

// ============================================================================
// REFRESH TOKEN OPERATIONS — Available for both JWT and OAuth
// ============================================================================

/// Sign a refresh token for the given subject.
///
/// Used by custom login flows that need to issue refresh tokens alongside access tokens.
/// OAuth flows automatically issue refresh tokens — this is for `app.auth()` password flows.
///
/// - sub: Subject (email or user identifier)
/// - expires_seconds: Token lifetime in seconds (0 = use default from env/config)
/// Returns: DooResult with refresh JWT string on success
#[no_mangle]
pub extern "C" fn doo_auth_sign_refresh(
    sub: *const c_char,
    expires_seconds: i64,
) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let sub_str = match c_to_string(sub) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        match session::sign_refresh_token(&sub_str, expires_seconds) {
            Ok(token) => make_ok_string(&token),
            Err(e) => make_err(AuthErrorCode::InternalError, &e),
        }
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "Internal error"),
    }
}

/// Verify a refresh token and issue a new access token.
///
/// - refresh_token: The refresh JWT to verify
/// - data_json: Optional JSON data to embed in the new access token
/// Returns: DooResult with JSON `{"access_token":"...","expires_in":900,"token_type":"Bearer"}`
#[no_mangle]
pub extern "C" fn doo_auth_refresh(
    refresh_token: *const c_char,
    data_json: *const c_char,
) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let token_str = match c_to_string(refresh_token) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        let data = if data_json.is_null() {
            None
        } else {
            match c_to_string(data_json) {
                Ok(s) if !s.is_empty() => Some(s),
                _ => None,
            }
        };

        // Verify the refresh token
        let sub = match session::verify_refresh_token(&token_str) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::JwtInvalid, &e),
        };

        // Issue new access token
        let access_expiry = session::get_access_expiry();
        let new_access = match session::sign_token(&sub, data.as_deref(), access_expiry) {
            Ok(t) => t,
            Err(e) => return make_err(AuthErrorCode::InternalError, &e),
        };

        // Refresh token rotation: issue new refresh token (extends session)
        let refresh_expiry = session::get_refresh_expiry();
        let new_refresh = match session::sign_refresh_token(&sub, refresh_expiry) {
            Ok(t) => t,
            Err(e) => return make_err(AuthErrorCode::InternalError, &e),
        };

        let json = serde_json::json!({
            "access_token": new_access,
            "refresh_token": new_refresh,
            "expires_in": access_expiry,
            "token_type": "Bearer"
        })
        .to_string();

        // Push rotated cookies via cross-DLL bridge
        crate::strategies::oauth::http_handlers::push_cookies_via_http_bridge(
            &new_access,
            Some(&new_refresh),
            access_expiry as i64,
            refresh_expiry as i64,
        );

        make_ok_string(&json)
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "Internal error"),
    }
}

// ============================================================================
// COOKIE OPERATIONS — Core auth concept, works with any auth strategy
// ============================================================================

/// Set auth cookies (access + refresh) on the current response.
///
/// This is a CORE auth function — any auth strategy (JWT login, OAuth, future SSO)
/// can call this to set httpOnly cookies. The HTTP server reads pending cookies
/// and adds Set-Cookie headers automatically.
///
/// - access_token: The access JWT to set as cookie
/// - refresh_token: The refresh JWT to set as cookie (nullable = no refresh cookie)
/// Returns: DooResult with "ok" on success
#[no_mangle]
pub extern "C" fn doo_auth_set_cookies(
    access_token: *const c_char,
    refresh_token: *const c_char,
) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let access = match c_to_string(access_token) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        let access_expiry = session::get_access_expiry();
        let refresh_expiry = session::get_refresh_expiry();

        let refresh = if !refresh_token.is_null() {
            c_to_string(refresh_token).ok().filter(|s| !s.is_empty())
        } else {
            None
        };

        // Push cookies via cross-DLL bridge — ensures they reach the HTTP DLL's thread-local
        crate::strategies::oauth::http_handlers::push_cookies_via_http_bridge(
            &access,
            refresh.as_deref(),
            access_expiry as i64,
            refresh_expiry as i64,
        );

        make_ok_string("ok")
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "Failed to set cookies"),
    }
}

/// Clear auth cookies — logs the user out by setting Max-Age=0.
///
/// - Returns: DooResult with "ok" on success
#[no_mangle]
pub extern "C" fn doo_auth_clear_cookies() -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Single centralized clear function — no manual cookie building
        doo_ffi_core::cookies::push_clear_cookies();

        make_ok_string("ok")
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "Failed to clear cookies"),
    }
}
