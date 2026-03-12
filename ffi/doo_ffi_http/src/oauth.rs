//! OAuth HTTP Integration
//!
//! Wires `app.oauth(config)` into the HTTP server:
//! - Parses config JSON: `{Providers: ["Google", "GitHub"]}`
//! - Initializes OAuth providers via `doo_ffi_auth`
//! - Auto-registers redirect + callback routes per provider
//!
//! ## Auto-generated routes
//!
//! For `app.oauth({Providers: ["Google", "GitHub"]})`:
//! - `GET /auth/google`          → 302 redirect to Google OAuth
//! - `GET /auth/google/callback` → exchange code → return JWT session JSON
//! - `GET /auth/github`          → 302 redirect to GitHub OAuth
//! - `GET /auth/github/callback` → exchange code → return JWT session JSON
//!
//! ## Coexistence with JWT
//!
//! OAuth handles **login/signup** (external provider authentication).
//! JWT handles **API protection** via `Jwt()` middleware.
//! After OAuth login, a JWT session token is issued — `Jwt()` verifies both.

use std::collections::HashMap;
use std::ffi::c_void;
use std::os::raw::c_char;
use std::sync::OnceLock;

use doo_ffi_core::ffi_debug;
use doo_ffi_core::DooResult;

use crate::helpers::{c_to_string, make_redirect, string_to_c};
use crate::router::get_routes;
use crate::types::*;
use crate::{make_err_http, make_ok_json, make_ok_void};

// ============================================================================
// OAUTH CONFIG — stored at init, read by handlers
// ============================================================================

/// OAuth configuration (set once by doo_http_oauth)
struct OAuthConfig {
    /// Registered provider names (lowercase): ["google", "github"]
    providers: Vec<String>,
    /// Base path for OAuth routes (default: "/auth")
    base_path: String,
}

/// Global OAuth config — written once at init, read by handlers
static OAUTH_CONFIG: OnceLock<OAuthConfig> = OnceLock::new();

// ============================================================================
// ROUTE HANDLERS — registered as DooHandlerFn
// ============================================================================

/// OAuth redirect handler — redirects browser to provider's authorization page.
///
/// Route: `GET /auth/<provider>`
/// Extracts provider name from request path, generates auth URL with PKCE + CSRF,
/// returns 302 redirect.
extern "C" fn oauth_redirect_handler(req: *const DooRequest) -> *mut DooResult {
    ffi_debug!("OAUTH", "oauth_redirect_handler called");

    if req.is_null() {
        return make_err_http(400, "Invalid request");
    }

    let path = unsafe { c_to_string((*req).path) };
    let provider = match extract_provider_from_path(&path) {
        Some(p) => p,
        None => {
            ffi_debug!("OAUTH", "Could not extract provider from path: {}", path);
            return make_err_http(400, "Invalid OAuth provider path");
        }
    };

    ffi_debug!("OAUTH", "Redirect handler for provider: {}", provider);

    // Get authorization URL from auth FFI (includes PKCE + CSRF state)
    match doo_ffi_auth::strategies::oauth::get_auth_url(&provider) {
        Ok(json) => {
            // json is: {"url":"https://...","state":"...","provider":"..."}
            // Extract the URL for the redirect
            match serde_json::from_str::<serde_json::Value>(&json) {
                Ok(val) => {
                    if let Some(url) = val.get("url").and_then(|u| u.as_str()) {
                        ffi_debug!("OAUTH", "Redirecting to: {}", url);
                        make_redirect(url)
                    } else {
                        make_err_http(500, "OAuth provider returned no authorization URL")
                    }
                }
                Err(e) => {
                    ffi_debug!("OAUTH", "Failed to parse auth URL response: {}", e);
                    make_err_http(500, "Failed to parse OAuth authorization URL")
                }
            }
        }
        Err(e) => {
            ffi_debug!("OAUTH", "Failed to get auth URL: {}", e);
            make_err_http(500, &format!("OAuth error: {}", e))
        }
    }
}

/// OAuth callback handler — exchanges authorization code for tokens + user info.
///
/// Route: `GET /auth/<provider>/callback?code=...&state=...`
/// Extracts provider from path, reads `code` and `state` from query params,
/// calls auth FFI to exchange, returns JSON with session token.
extern "C" fn oauth_callback_handler(req: *const DooRequest) -> *mut DooResult {
    ffi_debug!("OAUTH", "oauth_callback_handler called");

    if req.is_null() {
        return make_err_http(400, "Invalid request");
    }

    let path = unsafe { c_to_string((*req).path) };
    let provider = match extract_provider_from_callback_path(&path) {
        Some(p) => p,
        None => {
            ffi_debug!("OAUTH", "Could not extract provider from callback path: {}", path);
            return make_err_http(400, "Invalid OAuth callback path");
        }
    };

    ffi_debug!("OAUTH", "Callback handler for provider: {}", provider);

    // Extract query parameters (code and state)
    let (code, state) = unsafe {
        let query_ptr = (*req).query as *const HashMap<String, String>;
        if query_ptr.is_null() {
            ffi_debug!("OAUTH", "No query parameters in callback request");
            return make_err_http(400, "Missing OAuth callback parameters (code, state)");
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
                    );
                }
                ffi_debug!("OAUTH", "Missing 'code' query parameter");
                return make_err_http(400, "Missing 'code' parameter in OAuth callback");
            }
        };

        let state = match query_map.get("state") {
            Some(s) => s.clone(),
            None => {
                ffi_debug!("OAUTH", "Missing 'state' query parameter");
                return make_err_http(400, "Missing 'state' parameter in OAuth callback");
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
    match doo_ffi_auth::strategies::oauth::exchange_code(&provider, &code, &state) {
        Ok(json) => {
            ffi_debug!("OAUTH", "OAuth exchange successful");
            make_ok_json(&json)
        }
        Err(e) => {
            ffi_debug!("OAUTH", "OAuth exchange failed: {}", e);
            let err_lower = e.to_lowercase();
            if err_lower.contains("state") || err_lower.contains("csrf") {
                make_err_http(403, &format!("OAuth security error: {}", e))
            } else if err_lower.contains("invalid_grant") || err_lower.contains("expired") {
                make_err_http(401, &format!("OAuth grant error: {}", e))
            } else {
                make_err_http(500, &format!("OAuth exchange error: {}", e))
            }
        }
    }
}

// ============================================================================
// PATH HELPERS — extract provider name from route paths
// ============================================================================

/// Extract provider name from redirect path: `/auth/google` → `"google"`
fn extract_provider_from_path(path: &str) -> Option<String> {
    let config = OAUTH_CONFIG.get()?;
    let base = config.base_path.trim_end_matches('/');

    // path should be "{base}/{provider}"
    let suffix = path.strip_prefix(base)?;
    let provider = suffix.trim_start_matches('/');

    // Validate it's a known provider (not a sub-path like "google/callback")
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

    // path should be "{base}/{provider}/callback"
    let suffix = path.strip_prefix(base)?;
    let suffix = suffix.trim_start_matches('/');

    // Strip "/callback" suffix
    let provider = suffix.strip_suffix("/callback")?;

    if config.providers.contains(&provider.to_string()) {
        Some(provider.to_string())
    } else {
        None
    }
}

// ============================================================================
// FFI ENTRY POINT — called by codegen for `app.oauth(config)`
// ============================================================================

/// Initialize OAuth and register auto-generated routes.
///
/// Config JSON format (from Doo): `{"Providers": ["Google", "GitHub"]}`
///
/// Steps:
/// 1. Parse config → extract provider list
/// 2. Initialize auth FFI OAuth providers (reads env vars for credentials)
/// 3. Register routes per provider:
///    - `GET /auth/<provider>` → redirect to OAuth provider
///    - `GET /auth/<provider>/callback` → exchange code, return JWT session
///
/// Returns: DooResult with "ok" on success
#[no_mangle]
pub extern "C" fn doo_http_oauth(
    _server: *const c_void,
    config_json: *const c_char,
) -> *mut DooResult {
    ffi_safe_result!({
        let config_str = c_to_string(config_json);
        ffi_debug!("OAUTH", "doo_http_oauth called with config: {}", config_str);

        // Parse configuration
        let (providers, base_path) = match parse_oauth_config(&config_str) {
            Ok(cfg) => cfg,
            Err(e) => {
                ffi_debug!("OAUTH", "Invalid OAuth config: {}", e);
                return make_err_http(400, &format!("Invalid OAuth config: {}", e));
            }
        };

        ffi_debug!(
            "OAUTH",
            "Parsed config: providers={:?}, base_path={}",
            providers,
            base_path
        );

        // Normalize provider names to lowercase
        let providers_lower: Vec<String> = providers.iter().map(|p| p.to_lowercase()).collect();

        // Initialize auth FFI OAuth providers (reads credentials from env vars)
        let providers_json = serde_json::to_string(&providers_lower)
            .unwrap_or_else(|_| "[]".to_string());

        match doo_ffi_auth::strategies::oauth::init(Some(&providers_json)) {
            Ok(()) => {
                ffi_debug!("OAUTH", "Auth FFI OAuth providers initialized");
            }
            Err(e) => {
                ffi_debug!("OAUTH", "Failed to initialize OAuth providers: {}", e);
                return make_err_http(500, &format!("OAuth init failed: {}", e));
            }
        }

        // Store config for handler use
        let config = OAuthConfig {
            providers: providers_lower.clone(),
            base_path: base_path.clone(),
        };

        if OAUTH_CONFIG.set(config).is_err() {
            return make_err_http(400, "OAuth already initialized");
        }

        // Register routes for each provider
        let routes = get_routes();
        let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());
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

            registry.register("GET", &redirect_path, oauth_redirect_handler);
            registry.register("GET", &callback_path, oauth_callback_handler);
        }

        ffi_debug!(
            "OAUTH",
            "OAuth configured: {} providers, {} routes registered",
            providers_lower.len(),
            providers_lower.len() * 2
        );

        make_ok_void()
    })
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
///
/// Returns: (provider_names, base_path)
fn parse_oauth_config(json: &str) -> Result<(Vec<String>, String), String> {
    let default_base = "/auth".to_string();

    // Try as JSON array first: ["Google", "GitHub"]
    if let Ok(names) = serde_json::from_str::<Vec<String>>(json) {
        if names.is_empty() {
            return Err("At least one OAuth provider must be specified".to_string());
        }
        return Ok((names, default_base));
    }

    // Try as JSON object: {"Providers": ["Google", "GitHub"], "BasePath": "/auth"}
    if let Ok(obj) = serde_json::from_str::<serde_json::Value>(json) {
        // Case-insensitive key lookup for "Providers"
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
                // Optional base path
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
