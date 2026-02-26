//! OAuth 2.0 Login Module — Google & GitHub Providers
//!
//! ## Architecture
//!
//! ```text
//! Doo program → app.oauth(config) → HTTP FFI → Auth FFI (this module)
//!                                       ↓              ↓
//!                               auto-registers    OAuthProvider (Google, GitHub)
//!                               routes             + PKCE + state + HTTP client
//!                                       ↓
//!                               callback handler → exchange code → issue JWT session
//! ```
//!
//! ## How it works (OAuth + JWT coexistence)
//!
//! 1. `app.oauth({Providers: ["Google", "GitHub"], Callback: "/auth/callback"})` → init
//! 2. Auto-generated routes:
//!    - GET /auth/google → redirect to Google authorization
//!    - GET /auth/google/callback → exchange code, issue JWT session token
//!    - GET /auth/github → redirect to GitHub authorization
//!    - GET /auth/github/callback → exchange code, issue JWT session token
//! 3. After OAuth login, a JWT session token is issued (via `session` module)
//! 4. `Jwt()` middleware protects API routes (works with both JWT and OAuth sessions)
//!
//! ## Key design: OAuth does NOT replace JWT
//!
//! - OAuth is for **login/signup** (external provider authentication)
//! - JWT is for **API protection** (session tokens after any login method)
//! - `app.auth()` always uses JWT (password-based auth)
//! - `app.oauth()` adds OAuth login (issues JWT sessions after OAuth login)
//! - `Jwt()` middleware verifies tokens from BOTH systems (same JWT format)
//!
//! ## Adding a new provider
//!
//! 1. Create `strategies/oauth/<name>.rs` implementing `OAuthProvider`
//! 2. Add `pub mod <name>;` below
//! 3. Add provider matching in `init()`
//! 4. Done — zero compiler changes

pub mod github;
pub mod google;
pub mod http_handlers;
pub mod pkce;
pub mod provider;
pub mod state;
pub mod tokens;

// HTTP client utilities — shared across all providers
pub(crate) mod http_client;

use std::collections::HashMap;
use std::sync::OnceLock;

use provider::OAuthProvider;
use state::StateManager;

use crate::session;

// ============================================================================
// GLOBAL STATE — OnceLock for zero-overhead after initialization
// ============================================================================

/// Registered OAuth providers (provider_name → provider_impl)
static PROVIDERS: OnceLock<HashMap<String, Box<dyn OAuthProvider>>> = OnceLock::new();

/// CSRF state manager for OAuth flows
static STATE_MANAGER: OnceLock<StateManager> = OnceLock::new();

/// Default session token expiry (24 hours) — fallback when env not set.
/// Prefer `session::get_access_expiry()` which reads env var first.
const DEFAULT_SESSION_EXPIRY_SECS: i64 = 86400;

// ============================================================================
// INITIALIZATION — Provider registration from config
// ============================================================================

/// Initialize OAuth with specified providers.
///
/// Reads credentials from environment variables for each provider.
/// Does NOT change the active auth strategy — JWT remains for API auth.
/// OAuth is a separate login mechanism that coexists with JWT.
///
/// # Config format
/// - JSON array: `["google", "github"]` — providers with env var config
/// - JSON object: `{"providers": ["google", "github"]}` — explicit config
///
/// # Environment variables (per provider)
/// - `OAUTH_<PROVIDER>_CLIENT_ID`
/// - `OAUTH_<PROVIDER>_CLIENT_SECRET`
/// - `OAUTH_<PROVIDER>_REDIRECT_URI`
pub fn init(config_json: Option<&str>) -> Result<(), String> {
    let provider_names = parse_provider_names(config_json)?;

    if provider_names.is_empty() {
        return Err("At least one OAuth provider must be specified".to_string());
    }

    let mut providers: HashMap<String, Box<dyn OAuthProvider>> = HashMap::new();
    let mut skipped: Vec<(String, String)> = Vec::new();

    for name in &provider_names {
        let result: Result<Box<dyn OAuthProvider>, String> = match name.as_str() {
            "google" => google::GoogleProvider::from_env().map(|p| Box::new(p) as Box<dyn OAuthProvider>),
            "github" => github::GitHubProvider::from_env().map(|p| Box::new(p) as Box<dyn OAuthProvider>),
            _ => {
                skipped.push((name.clone(), format!("Unknown provider: '{}'", name)));
                continue;
            }
        };

        match result {
            Ok(provider) => {
                doo_ffi_core::ffi_debug!("OAuth", "Registered provider: {}", name);
                providers.insert(name.clone(), provider);
            }
            Err(reason) => {
                doo_ffi_core::ffi_debug!(
                    "OAuth",
                    "Skipping provider '{}': {} (credentials not configured)",
                    name,
                    reason
                );
                skipped.push((name.clone(), reason));
            }
        }
    }

    if providers.is_empty() {
        let reasons: Vec<String> = skipped
            .iter()
            .map(|(name, reason)| format!("{}: {}", name, reason))
            .collect();
        return Err(format!(
            "No OAuth providers could be initialized. All failed:\n  {}",
            reasons.join("\n  ")
        ));
    }

    if !skipped.is_empty() {
        for (name, reason) in &skipped {
            eprintln!(
                "[OAuth] Warning: Provider '{}' skipped — {}. Routes will not be registered for this provider.",
                name, reason
            );
        }
    }

    PROVIDERS
        .set(providers)
        .map_err(|_| "OAuth providers already initialized".to_string())?;

    STATE_MANAGER
        .set(StateManager::new())
        .map_err(|_| "OAuth state manager already initialized".to_string())?;

    doo_ffi_core::ffi_debug!(
        "OAuth",
        "Initialized with {} providers ({} skipped)",
        provider_names.len() - skipped.len(),
        skipped.len()
    );
    Ok(())
}

/// Parse provider names from config JSON.
fn parse_provider_names(config_json: Option<&str>) -> Result<Vec<String>, String> {
    let json = match config_json {
        Some(s) if !s.is_empty() => s,
        _ => return Err("OAuth config must specify at least one provider".to_string()),
    };

    // Try as JSON array first: ["google", "github"]
    if let Ok(names) = serde_json::from_str::<Vec<String>>(json) {
        return Ok(names);
    }

    // Try as JSON object: {"providers": ["google", "github"]}
    if let Ok(obj) = serde_json::from_str::<serde_json::Value>(json) {
        if let Some(providers) = obj.get("providers").and_then(|v| v.as_array()) {
            let names: Vec<String> = providers
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            if !names.is_empty() {
                return Ok(names);
            }
        }
    }

    // Try as single provider name: "google"
    let trimmed = json.trim().trim_matches('"');
    if !trimmed.is_empty() && !trimmed.contains('{') && !trimmed.contains('[') {
        return Ok(vec![trimmed.to_string()]);
    }

    Err(format!("Invalid OAuth config: {}", json))
}

// ============================================================================
// PROVIDER ACCESS — Used by FFI functions
// ============================================================================

/// Get a registered provider by name.
fn get_provider(name: &str) -> Result<&'static dyn OAuthProvider, String> {
    let providers = PROVIDERS
        .get()
        .ok_or("OAuth not initialized. Call OAuth.init() first")?;

    providers.get(name).map(|p| p.as_ref()).ok_or_else(|| {
        let available: Vec<&str> = providers.keys().map(|k| k.as_str()).collect();
        format!(
            "OAuth provider '{}' not registered. Available: {:?}",
            name, available
        )
    })
}

/// Get the state manager.
fn get_state_manager() -> Result<&'static StateManager, String> {
    STATE_MANAGER
        .get()
        .ok_or_else(|| "OAuth not initialized. Call OAuth.init() first".to_string())
}

// ============================================================================
// PUBLIC API — Called from FFI functions in lib.rs
// ============================================================================

/// Get an authorization URL for redirecting the user to the OAuth provider.
///
/// Generates PKCE challenge and CSRF state automatically.
///
/// # Returns
/// JSON: `{"url": "https://...", "state": "abc123"}`
pub fn get_auth_url(provider_name: &str) -> Result<String, String> {
    let provider = get_provider(provider_name)?;
    let state_mgr = get_state_manager()?;

    // Generate PKCE challenge
    let pkce = if provider.supports_pkce() {
        Some(pkce::PkceChallenge::generate())
    } else {
        None
    };

    // Create CSRF state with associated code_verifier
    let state = state_mgr.create_state(
        provider_name,
        pkce.as_ref().map(|p| p.code_verifier.clone()),
    );

    // Build authorization URL
    let url = provider.build_auth_url(&state, pkce.as_ref().map(|p| p.code_challenge.as_str()));

    // Return JSON with URL and state
    let result = serde_json::json!({
        "url": url,
        "state": state,
        "provider": provider_name,
    });

    serde_json::to_string(&result)
        .map_err(|e| format!("Failed to serialize auth URL response: {}", e))
}

/// Exchange an authorization code for tokens and user info.
///
/// Validates CSRF state, exchanges code via PKCE, fetches user info,
/// and creates a Doo session JWT.
///
/// # Returns
/// JSON with tokens, user info, and session token.
pub fn exchange_code(provider_name: &str, code: &str, state: &str) -> Result<String, String> {
    let provider = get_provider(provider_name)?;
    let state_mgr = get_state_manager()?;

    // Validate and consume CSRF state (single-use)
    let state_data = state_mgr.validate_and_consume(state)?;

    // Verify the state was created for the same provider
    if state_data.provider != provider_name {
        return Err(format!(
            "State mismatch: expected provider '{}', got '{}'",
            state_data.provider, provider_name
        ));
    }

    // Exchange authorization code for tokens (with PKCE verifier)
    let token_response = provider.exchange_code(code, state_data.code_verifier.as_deref())?;

    // Fetch user info using the provider's access token (used once, then discarded)
    let user_info = provider.get_user_info(&token_response.access_token)?;

    // Create YOUR session tokens (not the provider's) — these are what the frontend uses
    let user_data_json = serde_json::to_string(&user_info)
        .map_err(|e| format!("Failed to serialize user info: {}", e))?;

    // Access token: short-lived, carries user data, used for API auth
    let access_expiry = session::get_access_expiry();
    let access_token = session::sign_token(&user_info.email, Some(&user_data_json), access_expiry)?;

    // Refresh token: long-lived, minimal claims, used to get new access tokens
    let refresh_expiry = session::get_refresh_expiry();
    let refresh_token = session::sign_refresh_token(&user_info.email, refresh_expiry)?;

    // Build result — returns YOUR tokens + provider tokens for reference
    let result = tokens::OAuthExchangeResult {
        access_token: access_token.clone(),
        refresh_token: Some(refresh_token.clone()),
        expires_in: Some(access_expiry),
        user: user_info,
        provider_access_token: Some(token_response.access_token),
        provider_refresh_token: token_response.refresh_token,
    };

    // Push httpOnly cookies via the cross-DLL bridge — ensures cookies land in the
    // HTTP DLL's thread-local (not auth DLL's), where the server can find them.
    http_handlers::push_cookies_via_http_bridge(
        &access_token,
        Some(&refresh_token),
        access_expiry as i64,
        refresh_expiry as i64,
    );

    serde_json::to_string(&result)
        .map_err(|e| format!("Failed to serialize exchange result: {}", e))
}

/// Refresh an access token using a refresh token.
///
/// # Returns
/// JSON with new token response.
pub fn refresh(provider_name: &str, refresh_token: &str) -> Result<String, String> {
    let provider = get_provider(provider_name)?;

    let token_response = provider.refresh_token(refresh_token)?;

    serde_json::to_string(&token_response)
        .map_err(|e| format!("Failed to serialize refresh response: {}", e))
}

/// Get user info from a provider using an access token.
///
/// # Returns
/// JSON with normalized user info.
pub fn get_user_info(provider_name: &str, access_token: &str) -> Result<String, String> {
    let provider = get_provider(provider_name)?;

    let user_info = provider.get_user_info(access_token)?;

    serde_json::to_string(&user_info).map_err(|e| format!("Failed to serialize user info: {}", e))
}

/// Revoke a token with a provider.
pub fn revoke(provider_name: &str, token: &str) -> Result<(), String> {
    let provider = get_provider(provider_name)?;
    provider.revoke_token(token)
}

/// Get list of registered provider names.
pub fn list_providers() -> Vec<String> {
    PROVIDERS
        .get()
        .map(|p| p.keys().cloned().collect())
        .unwrap_or_default()
}
