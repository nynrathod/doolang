//! OAuth HTTP Route Handlers + Runtime Route Registration
//!
//! This module provides the HTTP route handlers for OAuth flows and registers
//! them with the HTTP server at runtime via `dlsym`/`GetProcAddress`. This keeps
//! OAuth as a pure package in doo_ffi_auth — zero OAuth code in doo_ffi_http.
//!
//! ## Architecture
//!
//! ```text
//! Doo program → @extern("doo_auth", "doo_auth_oauth_setup")
//!                        ↓
//!               doo_ffi_auth → init providers
//!                        ↓
//!               dlsym(RTLD_DEFAULT) → doo_http_register_package_route
//!                        ↓
//!               routes registered: GET /auth/google, GET /auth/google/callback, etc.
//! ```
//!
//! ## HTTP Request Struct Layout (matches doo_ffi_http's DooRequest)
//!
//! All fields are at C ABI-stable offsets:
//! ```text
//! offset  0: *const c_char  method
//! offset  8: *const c_char  path
//! offset 16: *const c_char  body
//! offset 24: *mut c_void    headers (HashMap<String,String>)
//! offset 32: *mut c_void    params  (JSON c_char*)
//! offset 40: *mut c_void    query   (HashMap<String,String>)
//! offset 48: *const c_char  user_id
//! ```

use std::collections::HashMap;
use std::ffi::c_void;
use std::os::raw::c_char;
use std::sync::OnceLock;

use doo_ffi_core::helpers::{c_to_string_lossy, string_to_c};
use doo_ffi_core::{ffi_debug, DooResult};

use super::{exchange_code, get_auth_url, init as oauth_init, list_providers};

// ============================================================================
// HTTP REQUEST — compatible layout with doo_ffi_http::DooRequest
// ============================================================================

/// C-compatible HTTP request struct matching doo_ffi_http's DooRequest layout.
/// Used to read request data from the HTTP server without depending on doo_ffi_http.
#[repr(C)]
struct HttpRequest {
    method: *const c_char,
    path: *const c_char,
    body: *const c_char,
    headers: *mut c_void,
    params: *mut c_void,
    query: *mut c_void,
    user_id: *const c_char,
}

// ============================================================================
// OAUTH CONFIG — stored at init, read by handlers
// ============================================================================

/// OAuth configuration stored at setup time
struct OAuthConfig {
    /// Registered provider names (lowercase): ["google", "github"]
    providers: Vec<String>,
    /// Base path for OAuth routes (default: "/auth")
    base_path: String,
}

static OAUTH_CONFIG: OnceLock<OAuthConfig> = OnceLock::new();

// ============================================================================
// RESPONSE HELPERS — construct DooResult without depending on doo_ffi_http
// ============================================================================

/// Create an OK JSON response (tag=0, data=json c_string)
fn make_ok_json(json: &str) -> *mut DooResult {
    DooResult::ok_string(json).into_raw()
}

/// Create an OK empty response (tag=0, data=null)
fn make_ok_void() -> *mut DooResult {
    DooResult::ok_empty().into_raw()
}

/// Create a 302 redirect response.
///
/// Returns a DooResult with error tag and an error struct containing:
/// - status = 302 (or given status)
/// - body = redirect URL (raw, not RFC 7807)
///
/// The HTTP server detects 3xx status and builds a proper 302 + Location header.
fn make_redirect(url: &str) -> *mut DooResult {
    unsafe {
        // Error response layout: { i32 status, i32 padding, *const c_char body, *const c_char ct }
        let struct_size = 4 + 4 + std::mem::size_of::<*const c_char>() * 2; // 24 bytes on 64-bit
        let ptr = libc::malloc(struct_size) as *mut u8;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        // status = 302
        *(ptr as *mut i32) = 302;
        // padding = 0
        *(ptr.add(4) as *mut i32) = 0;
        // body = URL string (allocated via doo_alloc_string)
        *(ptr.add(8) as *mut *const c_char) = string_to_c(url);
        // content_type = null (not needed for redirects)
        *(ptr.add(8 + std::mem::size_of::<*const c_char>()) as *mut *const c_char) =
            std::ptr::null();

        // Wrap in DooResult(tag=err, data=error_struct)
        let result = DooResult::err(302, ptr as *mut c_void, 0);
        result.into_raw()
    }
}

/// Create an HTTP error response using the centralized RFC 7807 format.
///
/// Uses `doo_ffi_core::Rfc7807Error` (single source of truth) so all error
/// responses — JWT, OAuth, validation, etc. — share the same structure.
fn make_err_http(status: i32, detail: &str, instance: &str) -> *mut DooResult {
    let json = doo_ffi_core::Rfc7807Error::new(status as u16, detail)
        .with_instance(instance)
        .to_json();

    unsafe {
        let struct_size = 4 + 4 + std::mem::size_of::<*const c_char>() * 2;
        let ptr = libc::malloc(struct_size) as *mut u8;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = status;
        *(ptr.add(4) as *mut i32) = 0;
        *(ptr.add(8) as *mut *const c_char) = string_to_c(&json);
        *(ptr.add(8 + std::mem::size_of::<*const c_char>()) as *mut *const c_char) =
            string_to_c("application/problem+json");

        let result = DooResult::err(status as u16, ptr as *mut c_void, 0);
        result.into_raw()
    }
}

// ============================================================================
// ROUTE HANDLERS — extern "C" functions registered with HTTP server
// ============================================================================

/// OAuth redirect handler — redirects browser to provider's authorization page.
///
/// Route: `GET /auth/<provider>`
/// Reads provider name from request path, generates auth URL with PKCE + CSRF,
/// returns 302 redirect via DooResult error struct.
extern "C" fn oauth_redirect_handler(req: *const c_void) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ffi_debug!("OAUTH", "oauth_redirect_handler called");

        if req.is_null() {
            return make_err_http(400, "Invalid request", "/auth");
        }

        let request = req as *const HttpRequest;
        let path = unsafe { c_to_string_lossy((*request).path) };
        let provider = match extract_provider_from_path(&path) {
            Some(p) => p,
            None => {
                ffi_debug!("OAUTH", "Could not extract provider from path: {}", path);
                return make_err_http(400, "Invalid OAuth provider path", &path);
            }
        };

        ffi_debug!("OAUTH", "Redirect handler for provider: {}", provider);

        // Get authorization URL from auth FFI (includes PKCE + CSRF state)
        match get_auth_url(&provider) {
            Ok(json) => {
                // json is: {"url":"https://...","state":"...","provider":"..."}
                match serde_json::from_str::<serde_json::Value>(&json) {
                    Ok(val) => {
                        if let Some(url) = val.get("url").and_then(|u| u.as_str()) {
                            ffi_debug!("OAUTH", "Redirecting to: {}", url);
                            make_redirect(url)
                        } else {
                            make_err_http(
                                500,
                                "OAuth provider returned no authorization URL",
                                &path,
                            )
                        }
                    }
                    Err(e) => {
                        ffi_debug!("OAUTH", "Failed to parse auth URL response: {}", e);
                        make_err_http(500, "Failed to parse OAuth authorization URL", &path)
                    }
                }
            }
            Err(e) => {
                ffi_debug!("OAUTH", "Failed to get auth URL: {}", e);
                make_err_http(500, &format!("OAuth error: {}", e), &path)
            }
        }
    })) {
        Ok(result) => result,
        Err(_) => make_err_http(
            500,
            "Internal server error (panic in OAuth redirect handler)",
            "/auth",
        ),
    }
}

/// OAuth callback handler — exchanges authorization code for tokens + user info.
///
/// Route: `GET /auth/<provider>/callback?code=...&state=...`
/// Reads provider from path, reads `code` and `state` from query params,
/// calls auth FFI to exchange, returns JSON with session token.
extern "C" fn oauth_callback_handler(req: *const c_void) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ffi_debug!("OAUTH", "oauth_callback_handler called");

        if req.is_null() {
            return make_err_http(400, "Invalid request", "/auth/callback");
        }

        let request = req as *const HttpRequest;
        let path = unsafe { c_to_string_lossy((*request).path) };
        let provider = match extract_provider_from_callback_path(&path) {
            Some(p) => p,
            None => {
                ffi_debug!(
                    "OAUTH",
                    "Could not extract provider from callback path: {}",
                    path
                );
                return make_err_http(400, "Invalid OAuth callback path", &path);
            }
        };

        ffi_debug!("OAUTH", "Callback handler for provider: {}", provider);

        // Read query parameters (code and state)
        let (code, state) = unsafe {
            let query_ptr = (*request).query as *const HashMap<String, String>;
            if query_ptr.is_null() {
                ffi_debug!("OAUTH", "No query parameters in callback request");
                return make_err_http(
                    400,
                    "Missing OAuth callback parameters (code, state)",
                    &path,
                );
            }

            let query_map = &*query_ptr;

            let code = match query_map.get("code") {
                Some(c) => c.clone(),
                None => {
                    // Check for error parameter (OAuth provider denied access)
                    if let Some(error) = query_map.get("error") {
                        let desc = query_map
                            .get("error_description")
                            .map(|d| d.as_str())
                            .unwrap_or("Access denied");
                        ffi_debug!("OAUTH", "OAuth error from provider: {} - {}", error, desc);
                        return make_err_http(
                            401,
                            &format!("OAuth authorization denied: {}", desc),
                            &path,
                        );
                    }
                    ffi_debug!("OAUTH", "Missing 'code' query parameter");
                    return make_err_http(400, "Missing 'code' parameter in OAuth callback", &path);
                }
            };

            let state = match query_map.get("state") {
                Some(s) => s.clone(),
                None => {
                    ffi_debug!("OAUTH", "Missing 'state' query parameter");
                    return make_err_http(
                        400,
                        "Missing 'state' parameter in OAuth callback",
                        &path,
                    );
                }
            };

            (code, state)
        };

        ffi_debug!(
            "OAUTH",
            "Exchanging code for provider={}, code_len={}, state_len={}",
            provider,
            code.len(),
            state.len()
        );

        // Exchange authorization code for tokens + user info + session JWT
        match exchange_code(&provider, &code, &state) {
            Ok(json) => {
                ffi_debug!("OAUTH", "OAuth exchange successful");
                make_ok_json(&json)
            }
            Err(e) => {
                ffi_debug!("OAUTH", "OAuth exchange failed: {}", e);
                let err_lower = e.to_lowercase();
                if err_lower.contains("state") || err_lower.contains("csrf") {
                    make_err_http(403, &format!("OAuth security error: {}", e), &path)
                } else if err_lower.contains("invalid_grant") || err_lower.contains("expired") {
                    make_err_http(401, &format!("OAuth grant error: {}", e), &path)
                } else {
                    make_err_http(500, &format!("OAuth exchange error: {}", e), &path)
                }
            }
        }
    })) {
        Ok(result) => result,
        Err(_) => make_err_http(
            500,
            "Internal server error (panic in OAuth callback handler)",
            "/auth/callback",
        ),
    }
}

/// Token refresh handler — exchanges a refresh token for a new access token.
///
/// Route: `POST /auth/refresh`
/// Reads `refresh_token` from JSON body, verifies it, issues new access token.
extern "C" fn oauth_refresh_handler(req: *const c_void) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ffi_debug!("OAUTH", "oauth_refresh_handler called");

        if req.is_null() {
            return make_err_http(400, "Invalid request", "/auth/refresh");
        }

        let request = req as *const HttpRequest;
        let body = unsafe { c_to_string_lossy((*request).body) };

        if body.is_empty() {
            return make_err_http(400, "Request body is required", "/auth/refresh");
        }

        // Parse body JSON: {"refresh_token": "eyJ..."} or read from cookie
        let refresh_token = {
            // Strategy 1: Read from JSON body
            let from_body = if !body.is_empty() {
                serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|val| {
                        val.get("refresh_token")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    })
            } else {
                None
            };

            // Strategy 2: Read from cookie (fallback for cookie-only clients)
            let from_cookie = if from_body.is_none() {
                let headers = unsafe { (*request).headers as *const HashMap<String, String> };
                if !headers.is_null() {
                    let cookie_header = unsafe {
                        (*headers)
                            .get("cookie")
                            .or_else(|| (*headers).get("Cookie"))
                    };
                    cookie_header.and_then(|h| {
                        doo_ffi_core::cookies::extract_cookie_value(
                            h,
                            doo_ffi_core::cookies::COOKIE_REFRESH_TOKEN,
                        )
                        .map(|s| s.to_string())
                    })
                } else {
                    None
                }
            } else {
                None
            };

            match from_body.or(from_cookie) {
                Some(t) => t,
                None => {
                    return make_err_http(
                        400,
                        "Missing refresh token (provide in body or cookie)",
                        "/auth/refresh",
                    );
                }
            }
        };

        // Verify refresh token and get subject (email)
        let sub = match crate::session::verify_refresh_token(&refresh_token) {
            Ok(s) => s,
            Err(e) => {
                ffi_debug!("OAUTH", "Refresh token verification failed: {}", e);
                return make_err_http(
                    401,
                    &format!("Invalid refresh token: {}", e),
                    "/auth/refresh",
                );
            }
        };

        // Issue new access token with default expiry
        let access_expiry = crate::session::get_access_expiry();
        let new_access = match crate::session::sign_token(&sub, None, access_expiry) {
            Ok(t) => t,
            Err(e) => {
                return make_err_http(
                    500,
                    &format!("Failed to issue new token: {}", e),
                    "/auth/refresh",
                );
            }
        };

        // Refresh token rotation: issue new refresh token too (extends session)
        let refresh_expiry = crate::session::get_refresh_expiry();
        let new_refresh = match crate::session::sign_refresh_token(&sub, refresh_expiry) {
            Ok(t) => t,
            Err(e) => {
                return make_err_http(
                    500,
                    &format!("Failed to issue refresh token: {}", e),
                    "/auth/refresh",
                );
            }
        };

        let json = serde_json::json!({
            "access_token": new_access,
            "refresh_token": new_refresh,
            "expires_in": access_expiry,
            "token_type": "Bearer"
        })
        .to_string();

        // Push rotated cookies — single centralized function
        doo_ffi_core::cookies::push_auth_cookies(
            &new_access,
            Some(&new_refresh),
            access_expiry as i64,
            refresh_expiry as i64,
        );

        ffi_debug!(
            "OAUTH",
            "Refresh successful (with rotation) for sub={}",
            sub
        );
        make_ok_json(&json)
    })) {
        Ok(result) => result,
        Err(_) => make_err_http(
            500,
            "Internal server error (panic in refresh handler)",
            "/auth/refresh",
        ),
    }
}

// ============================================================================
// AUTH ME HANDLER — Returns current user identity from JWT claims
// ============================================================================

/// Handler for `GET /auth/me` — returns the authenticated user's identity data.
///
/// This is the SINGLE endpoint for getting current user info. Works for ALL
/// auth strategies (OAuth, JWT, future). Reads token from cookie or header,
/// decodes claims, returns identity JSON.
///
/// Response: `{ "sub": "user@email.com", "data": { email, name, ... }, "expires_at": ..., "issued_at": ... }`
extern "C" fn auth_me_handler(req: *const c_void) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ffi_debug!("AUTH", "auth_me_handler called");

        if req.is_null() {
            return make_err_http(401, "Invalid request", "/auth/me");
        }

        let request = req as *const HttpRequest;

        // Read token: Authorization header first, then cookie fallback
        let token = unsafe {
            let headers = (*request).headers as *const HashMap<String, String>;
            if headers.is_null() {
                return make_err_http(401, "No authentication provided", "/auth/me");
            }

            // Strategy 1: Authorization: Bearer <token>
            let from_header = (*headers)
                .get("authorization")
                .or_else(|| (*headers).get("Authorization"))
                .and_then(|h| h.strip_prefix("Bearer ").map(|s| s.to_string()));

            // Strategy 2: Cookie fallback
            let from_cookie = if from_header.is_none() {
                (*headers)
                    .get("cookie")
                    .or_else(|| (*headers).get("Cookie"))
                    .and_then(|h| {
                        doo_ffi_core::cookies::extract_cookie_value(
                            h,
                            doo_ffi_core::cookies::COOKIE_ACCESS_TOKEN,
                        )
                        .map(|s| s.to_string())
                    })
            } else {
                None
            };

            match from_header.or(from_cookie) {
                Some(t) => t,
                None => return make_err_http(401, "No authentication token provided", "/auth/me"),
            }
        };

        // Decode claims and return identity JSON
        match crate::session::get_token_claims_json(&token) {
            Ok(json) => {
                ffi_debug!("AUTH", "/auth/me returning claims for token");
                make_ok_json(&json)
            }
            Err(e) => {
                ffi_debug!("AUTH", "/auth/me token decode failed: {}", e);
                make_err_http(401, &format!("Invalid token: {}", e), "/auth/me")
            }
        }
    })) {
        Ok(result) => result,
        Err(_) => make_err_http(
            500,
            "Internal server error (panic in auth/me handler)",
            "/auth/me",
        ),
    }
}

// ============================================================================
// PATH HELPERS
// ============================================================================

/// Extract provider name from redirect path: `/auth/google` → `"google"`
fn extract_provider_from_path(path: &str) -> Option<String> {
    let config = OAUTH_CONFIG.get()?;
    let base = config.base_path.trim_end_matches('/');
    let suffix = path.strip_prefix(base)?;
    let provider = suffix.trim_start_matches('/');

    if !provider.contains('/') && config.providers.contains(&provider.to_string()) {
        Some(provider.to_string())
    } else {
        None
    }
}

/// Extract provider name from callback path: `/auth/google/callback` → `"google"`
fn extract_provider_from_callback_path(path: &str) -> Option<String> {
    let config = OAUTH_CONFIG.get()?;
    let base = config.base_path.trim_end_matches('/');
    let suffix = path.strip_prefix(base)?;
    let suffix = suffix.trim_start_matches('/');
    let provider = suffix.strip_suffix("/callback")?;

    if config.providers.contains(&provider.to_string()) {
        Some(provider.to_string())
    } else {
        None
    }
}

// ============================================================================
// RUNTIME ROUTE REGISTRATION — finds HTTP FFI symbol in process
// ============================================================================

/// Type alias for the generic route registration function in HTTP FFI
type RegisterPackageRouteFn = extern "C" fn(
    method: *const c_char,
    path: *const c_char,
    handler: extern "C" fn(*const c_void) -> *mut DooResult,
) -> *mut DooResult;

/// Cached function pointer to `doo_http_register_package_route`.
///
/// Resolution: Search the current process symbol table via dlsym/GetProcAddress.
/// Both doo_ffi_http and doo_ffi_auth are linked into the same final binary,
/// so the symbol is always available in the process image.
static REGISTER_FN: OnceLock<Option<RegisterPackageRouteFn>> = OnceLock::new();

/// Resolve `doo_http_register_package_route` from the running process.
///
/// Uses raw platform APIs (libc::dlsym on Unix, GetProcAddress on Windows)
/// to avoid third-party library indirection.
fn get_register_fn() -> Option<RegisterPackageRouteFn> {
    *REGISTER_FN.get_or_init(|| {
        const SYMBOL_NAME: &[u8] = b"doo_http_register_package_route\0";

        #[cfg(unix)]
        {
            // dlsym(RTLD_DEFAULT) searches: main binary → all loaded shared libraries
            // Both static and dynamic linking make the symbol visible here.
            let addr = unsafe {
                libc::dlsym(
                    libc::RTLD_DEFAULT,
                    SYMBOL_NAME.as_ptr() as *const libc::c_char,
                )
            };

            if !addr.is_null() {
                ffi_debug!("OAUTH", "Found HTTP register fn via dlsym(RTLD_DEFAULT)");
                let f: RegisterPackageRouteFn = unsafe { std::mem::transmute(addr) };
                return Some(f);
            }

            // Log the dlsym error for diagnostics (always visible, even in release)
            let err = unsafe { libc::dlerror() };
            let err_msg = if err.is_null() {
                "symbol not found (no dlerror detail)".to_string()
            } else {
                unsafe { std::ffi::CStr::from_ptr(err).to_string_lossy().to_string() }
            };
            eprintln!(
                "[doo_ffi_auth] ERROR: Could not find doo_http_register_package_route: {}",
                err_msg
            );
        }

        #[cfg(windows)]
        {
            let sym_name = SYMBOL_NAME.as_ptr() as *const i8;
            let module = unsafe { GetModuleHandleA(std::ptr::null()) };
            if !module.is_null() {
                let addr = unsafe { GetProcAddress(module, sym_name) };
                if !addr.is_null() {
                    ffi_debug!("OAUTH", "Found HTTP register fn via GetProcAddress");
                    let f: RegisterPackageRouteFn = unsafe { std::mem::transmute(addr) };
                    return Some(f);
                }
            }
            eprintln!(
                "[doo_ffi_auth] ERROR: Could not find doo_http_register_package_route via GetProcAddress"
            );
        }

        None
    })
}

#[cfg(windows)]
extern "system" {
    fn GetModuleHandleA(lpModuleName: *const i8) -> *mut std::ffi::c_void;
    fn GetProcAddress(
        hModule: *mut std::ffi::c_void,
        lpProcName: *const i8,
    ) -> *mut std::ffi::c_void;
}

/// Register a route with the HTTP server via runtime symbol resolution.
fn register_http_route(
    method: &str,
    path: &str,
    handler: extern "C" fn(*const c_void) -> *mut DooResult,
) -> Result<(), String> {
    let register_fn = get_register_fn().ok_or_else(|| {
        let msg =
            "Could not find doo_http_register_package_route — is the HTTP server initialized?";
        eprintln!("[doo_ffi_auth] ERROR: {}", msg);
        msg.to_string()
    })?;

    let method_c = string_to_c(method);
    let path_c = string_to_c(path);

    let result = register_fn(method_c, path_c, handler);

    // Free the C strings we allocated
    doo_ffi_core::doo_free(method_c as *mut u8);
    doo_ffi_core::doo_free(path_c as *mut u8);

    if result.is_null() {
        return Err("Route registration returned null".to_string());
    }

    // Check result tag (0 = ok)
    let res = unsafe { &*result };
    if res.is_ok() {
        Ok(())
    } else {
        Err("Route registration failed".to_string())
    }
}

// ============================================================================
// CONFIG PARSING
// ============================================================================

/// Parse OAuth config JSON from Doo.
///
/// Supported formats:
/// - `{"Providers": ["Google", "GitHub"]}` — default base_path "/auth"
/// - `{"Providers": ["Google"], "BasePath": "/oauth"}` — custom base path
/// - `["Google", "GitHub"]` — shorthand, default base_path "/auth"
fn parse_oauth_config(json: &str) -> Result<(Vec<String>, String), String> {
    let default_base = "/auth".to_string();

    // Try as JSON array: ["Google", "GitHub"]
    if let Ok(names) = serde_json::from_str::<Vec<String>>(json) {
        if names.is_empty() {
            return Err("At least one OAuth provider must be specified".to_string());
        }
        return Ok((names, default_base));
    }

    // Try as JSON object: {"Providers": ["Google", "GitHub"], "BasePath": "/auth"}
    if let Ok(obj) = serde_json::from_str::<serde_json::Value>(json) {
        let providers = obj
            .get("Providers")
            .or_else(|| obj.get("providers"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<String>>()
            });

        match providers {
            Some(names) if !names.is_empty() => {
                let base = obj
                    .get("BasePath")
                    .or_else(|| obj.get("basePath"))
                    .or_else(|| obj.get("base_path"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or(default_base);

                Ok((names, base))
            }
            _ => Err("'Providers' array is required and must not be empty".to_string()),
        }
    } else {
        Err(format!("Invalid JSON config: {}", json))
    }
}

// ============================================================================
// PUBLIC API — called from lib.rs FFI entry point
// ============================================================================

/// Initialize OAuth and register HTTP routes from pre-parsed map data.
///
/// Called by `doo_auth_oauth_setup` after reading the Doo map (same pattern as CORS).
///
/// `providers`: Provider names (e.g., ["Google", "GitHub"])
/// `base_path`: Base path for routes (e.g., "/auth")
pub fn setup_from_map(providers: &[String], base_path: &str) -> Result<(), String> {
    if providers.is_empty() {
        return Err("At least one OAuth provider must be specified".to_string());
    }

    // Normalize provider names to lowercase
    let providers_lower: Vec<String> = providers.iter().map(|p| p.to_lowercase()).collect();

    ffi_debug!(
        "OAUTH",
        "Setting up OAuth: providers={:?}, base_path={}",
        providers_lower,
        base_path
    );

    // Initialize auth FFI OAuth providers (reads credentials from env vars)
    let providers_json =
        serde_json::to_string(&providers_lower).unwrap_or_else(|_| "[]".to_string());
    oauth_init(Some(&providers_json))?;

    // Store config for handler use
    let config = OAuthConfig {
        providers: providers_lower.clone(),
        base_path: base_path.to_string(),
    };

    OAUTH_CONFIG
        .set(config)
        .map_err(|_| "OAuth already initialized".to_string())?;

    // Register routes for each provider via runtime symbol resolution
    let base = base_path.trim_end_matches('/');

    for provider in &providers_lower {
        let redirect_path = format!("{}/{}", base, provider);
        let callback_path = format!("{}/{}/callback", base, provider);

        ffi_debug!(
            "OAUTH",
            "Registering routes: GET {} (redirect), GET {} (callback)",
            redirect_path,
            callback_path
        );

        register_http_route("GET", &redirect_path, oauth_redirect_handler)?;
        register_http_route("GET", &callback_path, oauth_callback_handler)?;
    }

    // Register the shared token refresh route (POST /auth/refresh)
    let refresh_path = format!("{}/refresh", base);
    ffi_debug!("OAUTH", "Registering route: POST {}", refresh_path);
    register_http_route("POST", &refresh_path, oauth_refresh_handler)?;

    // Register /auth/me — returns current user identity from JWT claims
    // Works for ALL auth strategies (OAuth, JWT, future). Auto-registered.
    let me_path = format!("{}/me", base);
    ffi_debug!("AUTH", "Registering route: GET {}", me_path);
    register_http_route("GET", &me_path, auth_me_handler)?;

    let registered = list_providers();
    ffi_debug!(
        "OAUTH",
        "OAuth setup complete: {} providers ({:?}), {} routes (includes /refresh, /me)",
        registered.len(),
        registered,
        registered.len() * 2 + 2
    );

    Ok(())
}

/// Initialize OAuth and register HTTP routes from a JSON config string.
///
/// 1. Parses config JSON for provider list and base path
/// 2. Initializes OAuth providers (reads env vars for credentials)
/// 3. Registers redirect + callback routes via HTTP FFI's generic registration
///
/// Returns Ok(()) on success, Err(message) on failure.
pub fn setup(config_json: &str) -> Result<(), String> {
    // Parse configuration
    let (providers, base_path) = parse_oauth_config(config_json)?;
    setup_from_map(&providers, &base_path)
}
