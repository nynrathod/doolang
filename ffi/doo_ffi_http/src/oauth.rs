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
use std::sync::{Mutex, OnceLock};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use doo_ffi_core::ffi_debug;
use doo_ffi_core::DooResult;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::db_bridge::{
    execute_db_insert, execute_db_query_with_string_param, is_pool_initialized, to_snake_case,
};
use crate::helpers::{c_to_string, make_redirect};
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

/// PKCE code_verifier store — maps CSRF state token → code_verifier.
///
/// When generating an authorization URL, we store the PKCE code_verifier
/// keyed by the CSRF state token. When the callback arrives, we look up
/// the verifier to include it in the token exchange.
static PKCE_STORE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn get_pkce_store() -> &'static Mutex<HashMap<String, String>> {
    PKCE_STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

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

    // Get authorization URL — build using env vars for provider
    match get_oauth_auth_url(&provider) {
        Ok(url) => {
            ffi_debug!("OAUTH", "Redirecting to: {}", url);
            make_redirect(&url)
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
            ffi_debug!(
                "OAUTH",
                "Could not extract provider from callback path: {}",
                path
            );
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
                    return make_err_http(401, &format!("OAuth authorization denied: {}", desc));
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
    match exchange_oauth_code(&provider, &code, &state) {
        Ok(json) => {
            ffi_debug!("OAUTH", "OAuth exchange successful");

            // === WEBHOOK: fire "oauth_login" event (no-op if no webhooks registered) ===
            // Parse the JSON to get user data for the webhook payload
            if let Ok(user_data) = serde_json::from_str::<serde_json::Value>(&json) {
                crate::webhook_engine::fire(
                    &format!("oauth:{}", provider),
                    "oauth_login",
                    &user_data,
                );
            }

            make_ok_json(&json)
        }
        Err(e) => {
            ffi_debug!("OAUTH", "OAuth exchange failed: {}", e);
            let err_lower: String = e.to_lowercase();
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

        // Validate OAuth providers (non-fatal — logs warnings if env vars missing)
        for provider in &providers_lower {
            validate_oauth_provider_env(provider);
        }
        ffi_debug!("OAUTH", "OAuth providers validated");

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
// OAUTH ROUTE REGISTRATION WITH WEBHOOKS
// ============================================================================

/// Initialize OAuth with webhook support.
///
/// Uses the GENERIC webhook_engine — parses webhooks JSON, registers configs
/// with the engine for each provider, then delegates to the same route
/// registration as doo_http_oauth.
///
/// Webhook keys used:
/// - `"oauth:<provider>"` — fired on successful OAuth login (e.g., `"oauth:google"`)
/// - Event: `"oauth_login"`
#[no_mangle]
pub extern "C" fn doo_http_oauth_with_webhooks(
    server: *const c_void,
    options: *mut c_void,
    webhooks_json: *const c_char,
) -> *mut DooResult {
    ffi_safe_result!({
        if options.is_null() {
            return make_err_http(400, "OAuth config is required");
        }

        // Read map as HashMap<String, String> (same layout as Doo map objects)
        let map = unsafe { &*(options as *const std::collections::HashMap<String, String>) };

        // Read "Providers" key — comma-separated provider names
        let providers_str = match map.get("Providers").or_else(|| map.get("providers")) {
            Some(s) if !s.is_empty() => s.clone(),
            _ => {
                return make_err_http(400, "'Providers' key is required in OAuth config");
            }
        };

        let providers: Vec<String> = providers_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if providers.is_empty() {
            return make_err_http(400, "At least one OAuth provider must be specified");
        }

        // Read optional "BasePath" key
        let base_path = map
            .get("BasePath")
            .or_else(|| map.get("basePath"))
            .or_else(|| map.get("base_path"))
            .cloned()
            .unwrap_or_else(|| "/auth".to_string());

        // Build JSON config string for doo_http_oauth
        let config_json = serde_json::json!({
            "Providers": providers,
            "BasePath": base_path,
        })
        .to_string();

        // Parse and register webhook configs with the generic engine
        let wh_json = c_to_string(webhooks_json);
        if !wh_json.is_empty() && wh_json != "[]" {
            match crate::webhook_engine::parse_configs(&wh_json) {
                Ok(configs) if !configs.is_empty() => {
                    for provider in &providers {
                        let provider_lower = provider.to_lowercase();
                        let engine_key = format!("oauth:{}", provider_lower);
                        crate::webhook_engine::register(&engine_key, configs.clone());
                        ffi_debug!(
                            "HTTP",
                            "Registered webhooks for OAuth provider '{}'",
                            engine_key
                        );
                    }
                }
                Ok(_) => {
                    ffi_debug!("HTTP", "Empty webhook configs for OAuth");
                }
                Err(e) => {
                    ffi_debug!("HTTP", "Failed to parse OAuth webhooks JSON: {}", e);
                    // Non-fatal: OAuth still works without webhooks
                }
            }
        }

        // Delegate to the standard OAuth route registration
        let config_c = crate::helpers::string_to_c(&config_json);
        let result = doo_http_oauth(server, config_c);
        doo_ffi_core::doo_free(config_c as *mut u8);
        result
    })
}

// ============================================================================
// OAUTH HELPER FUNCTIONS — self-contained, no doo_ffi_auth dependency
// ============================================================================

/// Build the OAuth authorization URL for a given provider.
///
/// Generates PKCE (S256) challenge and CSRF state token.
/// Stores the code_verifier in PKCE_STORE for later retrieval during token exchange.
fn get_oauth_auth_url(provider: &str) -> Result<String, String> {
    let (client_id, redirect_uri, auth_url) = get_provider_config(provider)?;
    let state = uuid::Uuid::new_v4().to_string();
    let scope = match provider {
        "google" => "openid email profile",
        "github" => "user:email read:user",
        _ => "email profile",
    };

    // Generate PKCE code_verifier (32 random bytes → 43-char base64url)
    let mut random_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut random_bytes);
    let code_verifier = URL_SAFE_NO_PAD.encode(random_bytes);

    // Compute PKCE code_challenge = SHA256(code_verifier) → base64url
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let hash = hasher.finalize();
    let code_challenge = URL_SAFE_NO_PAD.encode(hash);

    // Store code_verifier keyed by state for later token exchange
    {
        let store = get_pkce_store();
        let mut map = store
            .lock()
            .map_err(|e| format!("PKCE store lock error: {}", e))?;
        map.insert(state.clone(), code_verifier);
    }

    let url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        auth_url,
        urlencoding::encode(&client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(scope),
        urlencoding::encode(&state),
        urlencoding::encode(&code_challenge),
    );
    Ok(url)
}

/// Exchange an OAuth authorization code for tokens using reqwest.
///
/// Retrieves the PKCE code_verifier from PKCE_STORE (stored during auth URL generation)
/// and includes it in the token exchange request. The verifier is consumed (removed) after use.
///
/// After successful exchange, upserts the user into the `users` table and returns
/// OUR JWT tokens (not the provider's) along with created_at/updated_at timestamps.
fn exchange_oauth_code(provider: &str, code: &str, state: &str) -> Result<String, String> {
    let (client_id, redirect_uri, _auth_url) = get_provider_config(provider)?;
    let client_secret = get_env_var(&format!("OAUTH_{}_CLIENT_SECRET", provider.to_uppercase()))?;
    let token_url = get_token_url(provider);

    // Retrieve and consume the PKCE code_verifier (single-use)
    let code_verifier = {
        let store = get_pkce_store();
        let mut map = store
            .lock()
            .map_err(|e| format!("PKCE store lock error: {}", e))?;
        map.remove(state)
    };

    let mut params = vec![
        ("client_id", client_id.as_str()),
        ("client_secret", client_secret.as_str()),
        ("code", code),
        ("redirect_uri", redirect_uri.as_str()),
        ("grant_type", "authorization_code"),
    ];

    // Include PKCE code_verifier if one was stored for this state
    if let Some(ref verifier) = code_verifier {
        params.push(("code_verifier", verifier.as_str()));
    }

    let client = reqwest::blocking::Client::new();
    let response = client
        .post(token_url)
        .form(&params)
        .header("Accept", "application/json")
        .send()
        .map_err(|e| format!("Token request failed: {}", e))?;

    let status = response.status();
    let body: String = response
        .text()
        .map_err(|e| format!("Failed to read token response: {}", e))?;

    if !status.is_success() {
        return Err(format!(
            "OAuth token exchange failed (HTTP {}): {}",
            status.as_u16(),
            body
        ));
    }

    let token_data: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Invalid token response: {}", e))?;

    let provider_access_token = token_data
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "No access_token in response".to_string())?;

    // Get normalized user info from the provider
    let user_info = get_oauth_user_info(provider, provider_access_token)?;

    // Upsert user into database (auto-sets created_at/updated_at via DB defaults + trigger)
    let (user_id, created_at, updated_at) = if is_pool_initialized() {
        upsert_oauth_user(&user_info)?
    } else {
        ffi_debug!("OAUTH", "DB not available — skipping user upsert for OAuth");
        (0i64, String::new(), String::new())
    };

    // Generate OUR JWT access token (not the provider's)
    let email = user_info
        .get("Email")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let access_token = crate::auth::generate_jwt_token(email, user_id, None);

    // Build result — returns OUR JWT token + user info + timestamps
    let result = serde_json::json!({
        "access_token": access_token,
        "refresh_token": token_data.get("refresh_token").and_then(|v| v.as_str()).unwrap_or(""),
        "expires_in": token_data.get("expires_in").and_then(|v| v.as_i64()).unwrap_or(3600),
        "user": user_info,
        "created_at": created_at,
        "updated_at": updated_at,
        "provider_access_token": provider_access_token,
        "provider_refresh_token": token_data.get("refresh_token").and_then(|v| v.as_str()).unwrap_or(""),
    });

    Ok(result.to_string())
}

/// Upsert an OAuth user into the `users` table.
///
/// - If user with this email doesn't exist: INSERT with provider info, created_at = NOW()
/// - If user exists: UPDATE name/avatar/provider, updated_at auto-set by trigger
///
/// Returns (user_id, created_at, updated_at) from the database record.
fn upsert_oauth_user(user_info: &serde_json::Value) -> Result<(i64, String, String), String> {
    let email = user_info
        .get("Email")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let name = user_info.get("Name").and_then(|v| v.as_str()).unwrap_or("");
    let avatar = user_info
        .get("Avatar")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let provider = user_info
        .get("Provider")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if email.is_empty() {
        return Err("OAuth user info missing email".to_string());
    }

    // Check if user already exists
    let check_sql = "SELECT id, created_at, updated_at FROM users WHERE email = $1";
    let existing = execute_db_query_with_string_param(check_sql, email)?;
    let existing_rows: Vec<serde_json::Value> = serde_json::from_str(&existing).unwrap_or_default();

    if let Some(row) = existing_rows.first() {
        // User exists — UPDATE provider/name/avatar, updated_at auto-set by trigger
        let user_id = row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        let created_at = row
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Update user profile with latest provider info
        let update_sql = "UPDATE users SET name = $1, avatar = $2, provider = $3 WHERE email = $4";
        let update_values: Vec<serde_json::Value> = vec![
            serde_json::json!(name),
            serde_json::json!(avatar),
            serde_json::json!(provider),
            serde_json::json!(email),
        ];
        execute_db_insert(update_sql, &update_values)?;

        // Fetch updated row to get current updated_at
        let refetch = execute_db_query_with_string_param(
            "SELECT updated_at FROM users WHERE email = $1",
            email,
        )?;
        let refetch_rows: Vec<serde_json::Value> =
            serde_json::from_str(&refetch).unwrap_or_default();
        let updated_at = refetch_rows
            .first()
            .and_then(|r| r.get("updated_at"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        ffi_debug!(
            "OAUTH",
            "Updated existing user: email={}, id={}",
            email,
            user_id
        );
        Ok((user_id, created_at, updated_at))
    } else {
        // New user — INSERT with provider info, created_at/updated_at auto-set by DB defaults
        let insert_sql = "INSERT INTO users (email, password, name, role, provider, avatar) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id, created_at, updated_at";
        let insert_values: Vec<serde_json::Value> = vec![
            serde_json::json!(email),
            serde_json::json!(""), // OAuth users have no password
            serde_json::json!(name),
            serde_json::json!(""), // default role
            serde_json::json!(provider),
            serde_json::json!(avatar),
        ];
        let result_json = execute_db_insert(insert_sql, &insert_values)?;
        let rows: Vec<serde_json::Value> = serde_json::from_str(&result_json).unwrap_or_default();

        if let Some(row) = rows.first() {
            let user_id = row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
            let created_at = row
                .get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let updated_at = row
                .get("updated_at")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            ffi_debug!(
                "OAUTH",
                "Created new OAuth user: email={}, id={}, provider={}",
                email,
                user_id,
                provider
            );
            Ok((user_id, created_at, updated_at))
        } else {
            Err("Failed to create OAuth user: no rows returned".to_string())
        }
    }
}

/// Get user info from provider using access token.
fn get_oauth_user_info(provider: &str, access_token: &str) -> Result<serde_json::Value, String> {
    let userinfo_url = match provider {
        "google" => "https://openidconnect.googleapis.com/v1/userinfo",
        "github" => "https://api.github.com/user",
        _ => return Err(format!("Unknown provider: {}", provider)),
    };

    let client = reqwest::blocking::Client::new();
    let mut req = client
        .get(userinfo_url)
        .header("Accept", "application/json");

    if provider == "github" {
        req = req
            .header("Authorization", format!("Bearer {}", access_token))
            .header("User-Agent", "Doo-OAuth/1.0");
    } else {
        req = req.header("Authorization", format!("Bearer {}", access_token));
    }

    let response = req
        .send()
        .map_err(|e| format!("User info request failed: {}", e))?;
    let body: String = response
        .text()
        .map_err(|e| format!("Failed to read user info: {}", e))?;
    let info: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Invalid user info response: {}", e))?;

    let normalized = match provider {
        "google" => serde_json::json!({
            "Id": info["sub"].as_str().unwrap_or(""),
            "Email": info["email"].as_str().unwrap_or(""),
            "Name": info["name"].as_str().unwrap_or(""),
            "Avatar": info["picture"].as_str().unwrap_or(""),
            "Provider": provider,
            "EmailVerified": info["email_verified"].as_bool().unwrap_or(false),
        }),
        "github" => {
            let email = info["email"].as_str().unwrap_or("").to_string();
            let name = info["login"].as_str().unwrap_or("");
            serde_json::json!({
                "Id": info["id"].to_string(),
                "Email": email,
                "Name": name,
                "Avatar": info["avatar_url"].as_str().unwrap_or(""),
                "Provider": provider,
                "EmailVerified": true,
            })
        }
        _ => info,
    };

    Ok(normalized)
}

/// Validate env vars exist for provider (non-fatal, logs warnings).
fn validate_oauth_provider_env(provider: &str) {
    let pu = provider.to_uppercase();
    if std::env::var(format!("OAUTH_{}_CLIENT_ID", pu)).is_err() {
        ffi_debug!("OAUTH", "Warning: OAUTH_{}_CLIENT_ID not set", pu);
    }
    if std::env::var(format!("OAUTH_{}_CLIENT_SECRET", pu)).is_err() {
        ffi_debug!("OAUTH", "Warning: OAUTH_{}_CLIENT_SECRET not set", pu);
    }
}

fn get_provider_config(provider: &str) -> Result<(String, String, String), String> {
    let pu = provider.to_uppercase();
    let client_id = get_env_var(&format!("OAUTH_{}_CLIENT_ID", pu))?;
    let redirect_uri = std::env::var(format!("OAUTH_{}_REDIRECT_URI", pu))
        .unwrap_or_else(|_| format!("http://localhost:3000/auth/{}/callback", provider));
    let auth_url = get_auth_endpoint(provider);
    Ok((client_id, redirect_uri, auth_url.to_string()))
}

fn get_auth_endpoint(provider: &str) -> &str {
    match provider {
        "google" => "https://accounts.google.com/o/oauth2/v2/auth",
        "github" => "https://github.com/login/oauth/authorize",
        _ => "https://example.com/oauth/authorize",
    }
}

fn get_token_url(provider: &str) -> &str {
    match provider {
        "google" => "https://oauth2.googleapis.com/token",
        "github" => "https://github.com/login/oauth/access_token",
        _ => "https://example.com/oauth/token",
    }
}

fn get_env_var(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("Environment variable {} not set", name))
}

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
