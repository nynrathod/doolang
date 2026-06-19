//! Google OAuth 2.0 Provider
//!
//! Implements `OAuthProvider` for Google's OAuth 2.0 + OpenID Connect flow.
//!
//! ## Endpoints (production, well-known)
//! - Authorization: https://accounts.google.com/o/oauth2/v2/auth
//! - Token: https://oauth2.googleapis.com/token
//! - UserInfo: https://www.googleapis.com/oauth2/v2/userinfo
//! - Revocation: https://oauth2.googleapis.com/revoke
//!
//! ## Features
//! - Full PKCE support (S256)
//! - Refresh tokens (with access_type=offline&prompt=consent)
//! - Token revocation
//! - OpenID Connect (returns id_token)
//!
//! ## Environment Variables
//! - OAUTH_GOOGLE_CLIENT_ID (required)
//! - OAUTH_GOOGLE_CLIENT_SECRET (required)
//! - OAUTH_GOOGLE_REDIRECT_URI (required)
//!
//! ## Scopes
//! Default: openid, email, profile

use serde::Deserialize;

use super::http_client;
use super::provider::{OAuthProvider, ProviderConfig};
use super::tokens::{OAuthError, TokenResponse, UserInfo};

// ============================================================================
// GOOGLE ENDPOINTS — Well-known, stable OAuth 2.0 endpoints
// ============================================================================

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
const GOOGLE_REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";

/// Default scopes: OpenID Connect + email + profile
const GOOGLE_DEFAULT_SCOPES: &[&str] = &["openid", "email", "profile"];

// ============================================================================
// ENV VAR NAMES — Single source of truth
// ============================================================================

const ENV_GOOGLE_CLIENT_ID: &str = "OAUTH_GOOGLE_CLIENT_ID";
const ENV_GOOGLE_CLIENT_SECRET: &str = "OAUTH_GOOGLE_CLIENT_SECRET";
const ENV_GOOGLE_REDIRECT_URI: &str = "OAUTH_GOOGLE_REDIRECT_URI";

// ============================================================================
// GOOGLE PROVIDER
// ============================================================================

/// Google OAuth 2.0 provider with OIDC support.
pub struct GoogleProvider {
    config: ProviderConfig,
}

/// Google's userinfo API response format.
#[derive(Debug, Deserialize)]
struct GoogleUserInfo {
    id: String,
    email: String,
    #[serde(default)]
    verified_email: bool,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    picture: Option<String>,
}

impl GoogleProvider {
    /// Create a GoogleProvider from environment variables.
    ///
    /// All three env vars must be set:
    /// - OAUTH_GOOGLE_CLIENT_ID
    /// - OAUTH_GOOGLE_CLIENT_SECRET
    /// - OAUTH_GOOGLE_REDIRECT_URI
    pub fn from_env() -> Result<Self, String> {
        let config = ProviderConfig {
            client_id: std::env::var(ENV_GOOGLE_CLIENT_ID)
                .map_err(|_| format!("{} must be set", ENV_GOOGLE_CLIENT_ID))?,
            client_secret: std::env::var(ENV_GOOGLE_CLIENT_SECRET)
                .map_err(|_| format!("{} must be set", ENV_GOOGLE_CLIENT_SECRET))?,
            redirect_uri: std::env::var(ENV_GOOGLE_REDIRECT_URI)
                .map_err(|_| format!("{} must be set", ENV_GOOGLE_REDIRECT_URI))?,
            scopes: GOOGLE_DEFAULT_SCOPES
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };

        // Validate client_id format (Google client IDs end with .apps.googleusercontent.com)
        if config.client_id.is_empty() {
            return Err(format!("{} cannot be empty", ENV_GOOGLE_CLIENT_ID));
        }
        if config.client_secret.is_empty() {
            return Err(format!("{} cannot be empty", ENV_GOOGLE_CLIENT_SECRET));
        }

        Ok(Self { config })
    }

    /// Create a GoogleProvider with explicit configuration (for testing).
    pub fn with_config(config: ProviderConfig) -> Self {
        Self { config }
    }
}

impl OAuthProvider for GoogleProvider {
    fn name(&self) -> &'static str {
        "google"
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    fn supports_pkce(&self) -> bool {
        true
    }

    fn build_auth_url(&self, state: &str, code_challenge: Option<&str>) -> String {
        let scopes = self.config.scopes.join(" ");

        let mut params = vec![
            ("client_id", self.config.client_id.as_str()),
            ("redirect_uri", self.config.redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", &scopes),
            ("state", state),
            // access_type=offline ensures refresh_token is returned
            ("access_type", "offline"),
            // prompt=consent forces consent screen (needed for refresh_token on re-auth)
            ("prompt", "consent"),
        ];

        // Add PKCE parameters if supported
        let challenge_method = "S256";
        if let Some(challenge) = code_challenge {
            params.push(("code_challenge", challenge));
            params.push(("code_challenge_method", challenge_method));
        }

        http_client::build_url(GOOGLE_AUTH_URL, &params)
    }

    fn exchange_code(
        &self,
        code: &str,
        code_verifier: Option<&str>,
    ) -> Result<TokenResponse, String> {
        let mut params = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", self.config.redirect_uri.as_str()),
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.as_str()),
        ];

        if let Some(verifier) = code_verifier {
            params.push(("code_verifier", verifier));
        }

        let (status, body) = http_client::post_form(GOOGLE_TOKEN_URL, &params)?;

        if status >= 400 {
            if let Ok(err) = serde_json::from_str::<OAuthError>(&body) {
                return Err(format!("Google token exchange failed: {}", err));
            }
            return Err(format!(
                "Google token exchange failed (HTTP {}): {}",
                status, body
            ));
        }

        serde_json::from_str::<TokenResponse>(&body)
            .map_err(|e| format!("Failed to parse Google token response: {}", e))
    }

    fn refresh_token(&self, refresh_token: &str) -> Result<TokenResponse, String> {
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.as_str()),
        ];

        let (status, body) = http_client::post_form(GOOGLE_TOKEN_URL, &params)?;

        if status >= 400 {
            if let Ok(err) = serde_json::from_str::<OAuthError>(&body) {
                return Err(format!("Google token refresh failed: {}", err));
            }
            return Err(format!(
                "Google token refresh failed (HTTP {}): {}",
                status, body
            ));
        }

        serde_json::from_str::<TokenResponse>(&body)
            .map_err(|e| format!("Failed to parse Google refresh response: {}", e))
    }

    fn get_user_info(&self, access_token: &str) -> Result<UserInfo, String> {
        let (status, body) = http_client::get_with_bearer(GOOGLE_USERINFO_URL, access_token)?;

        if status >= 400 {
            return Err(format!(
                "Google userinfo request failed (HTTP {}): {}",
                status, body
            ));
        }

        let google_user: GoogleUserInfo = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse Google user info: {}", e))?;

        Ok(UserInfo {
            id: google_user.id,
            email: google_user.email,
            name: google_user.name,
            avatar: google_user.picture,
            provider: "google".to_string(),
            email_verified: google_user.verified_email,
            created_at: None, // Set by auth layer on user creation
            updated_at: None, // Set by auth layer on user creation
        })
    }

    fn revoke_token(&self, token: &str) -> Result<(), String> {
        let params = [("token", token)];

        let (status, body) = http_client::post_form(GOOGLE_REVOKE_URL, &params)?;

        if status >= 400 {
            return Err(format!(
                "Google token revocation failed (HTTP {}): {}",
                status, body
            ));
        }

        Ok(())
    }
}
