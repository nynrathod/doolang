//! GitHub OAuth 2.0 Provider
//!
//! Implements `OAuthProvider` for GitHub's OAuth flow.
//!
//! ## Endpoints
//! - Authorization: https://github.com/login/oauth/authorize
//! - Token: https://github.com/login/oauth/access_token
//! - UserInfo: https://api.github.com/user
//! - User Emails: https://api.github.com/user/emails (for verified email)
//!
//! ## GitHub-specific notes
//! - GitHub does NOT issue refresh tokens by default (tokens are long-lived)
//! - GitHub does NOT have a standard revocation endpoint
//! - GitHub requires User-Agent header on all API requests
//! - GitHub may not return email in /user — must also check /user/emails
//! - PKCE: supported but not required for confidential clients
//!
//! ## Environment Variables
//! - OAUTH_GITHUB_CLIENT_ID (required)
//! - OAUTH_GITHUB_CLIENT_SECRET (required)
//! - OAUTH_GITHUB_REDIRECT_URI (required)
//!
//! ## Scopes
//! Default: read:user, user:email

use serde::Deserialize;

use super::http_client;
use super::provider::{OAuthProvider, ProviderConfig};
use super::tokens::{OAuthError, TokenResponse, UserInfo};

// ============================================================================
// GITHUB ENDPOINTS
// ============================================================================

const GITHUB_AUTH_URL: &str = "https://github.com/login/oauth/authorize";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const GITHUB_USER_URL: &str = "https://api.github.com/user";
const GITHUB_EMAILS_URL: &str = "https://api.github.com/user/emails";

/// Default scopes: read user profile + email access
const GITHUB_DEFAULT_SCOPES: &[&str] = &["read:user", "user:email"];

// ============================================================================
// ENV VAR NAMES — Single source of truth
// ============================================================================

const ENV_GITHUB_CLIENT_ID: &str = "OAUTH_GITHUB_CLIENT_ID";
const ENV_GITHUB_CLIENT_SECRET: &str = "OAUTH_GITHUB_CLIENT_SECRET";
const ENV_GITHUB_REDIRECT_URI: &str = "OAUTH_GITHUB_REDIRECT_URI";

// ============================================================================
// GITHUB PROVIDER
// ============================================================================

/// GitHub OAuth 2.0 provider.
pub struct GitHubProvider {
    config: ProviderConfig,
}

/// GitHub's user API response format.
#[derive(Debug, Deserialize)]
struct GitHubUser {
    id: i64,
    #[serde(default)]
    login: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    avatar_url: Option<String>,
}

/// GitHub's email API response format.
#[derive(Debug, Deserialize)]
struct GitHubEmail {
    email: String,
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    verified: bool,
}

/// GitHub's token response format (different from standard OAuth 2.0).
#[derive(Debug, Deserialize)]
struct GitHubTokenResponse {
    access_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
    // GitHub error fields (returned in same response on failure)
    error: Option<String>,
    error_description: Option<String>,
}

impl GitHubProvider {
    /// Create a GitHubProvider from environment variables.
    ///
    /// All three env vars must be set:
    /// - OAUTH_GITHUB_CLIENT_ID
    /// - OAUTH_GITHUB_CLIENT_SECRET
    /// - OAUTH_GITHUB_REDIRECT_URI
    pub fn from_env() -> Result<Self, String> {
        let config = ProviderConfig {
            client_id: std::env::var(ENV_GITHUB_CLIENT_ID)
                .map_err(|_| format!("{} must be set", ENV_GITHUB_CLIENT_ID))?,
            client_secret: std::env::var(ENV_GITHUB_CLIENT_SECRET)
                .map_err(|_| format!("{} must be set", ENV_GITHUB_CLIENT_SECRET))?,
            redirect_uri: std::env::var(ENV_GITHUB_REDIRECT_URI)
                .map_err(|_| format!("{} must be set", ENV_GITHUB_REDIRECT_URI))?,
            scopes: GITHUB_DEFAULT_SCOPES
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };

        if config.client_id.is_empty() {
            return Err(format!("{} cannot be empty", ENV_GITHUB_CLIENT_ID));
        }
        if config.client_secret.is_empty() {
            return Err(format!("{} cannot be empty", ENV_GITHUB_CLIENT_SECRET));
        }

        Ok(Self { config })
    }

    /// Create a GitHubProvider with explicit configuration (for testing).
    pub fn with_config(config: ProviderConfig) -> Self {
        Self { config }
    }

    /// Fetch the user's primary verified email from GitHub's /user/emails API.
    ///
    /// GitHub may not return email in the /user endpoint if the user has
    /// a private email. This function fetches from /user/emails instead.
    fn fetch_primary_email(&self, access_token: &str) -> Result<(String, bool), String> {
        let (status, body) =
            http_client::get_with_bearer_and_ua(GITHUB_EMAILS_URL, access_token, "Doo-OAuth/1.0")?;

        if status >= 400 {
            return Err(format!(
                "GitHub emails request failed (HTTP {}): {}",
                status, body
            ));
        }

        let emails: Vec<GitHubEmail> = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse GitHub emails: {}", e))?;

        // Find primary verified email, fallback to first verified, then first email
        if let Some(email) = emails.iter().find(|e| e.primary && e.verified) {
            return Ok((email.email.clone(), true));
        }
        if let Some(email) = emails.iter().find(|e| e.verified) {
            return Ok((email.email.clone(), true));
        }
        if let Some(email) = emails.first() {
            return Ok((email.email.clone(), email.verified));
        }

        Err("No email found in GitHub account".to_string())
    }
}

impl OAuthProvider for GitHubProvider {
    fn name(&self) -> &'static str {
        "github"
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    fn supports_pkce(&self) -> bool {
        // GitHub supports PKCE but doesn't require it for confidential clients
        true
    }

    fn build_auth_url(&self, state: &str, code_challenge: Option<&str>) -> String {
        let scopes = self.config.scopes.join(" ");

        let mut params = vec![
            ("client_id", self.config.client_id.as_str()),
            ("redirect_uri", self.config.redirect_uri.as_str()),
            ("scope", &scopes),
            ("state", state),
        ];

        // Add PKCE if supported
        let challenge_method = "S256";
        if let Some(challenge) = code_challenge {
            params.push(("code_challenge", challenge));
            params.push(("code_challenge_method", challenge_method));
        }

        http_client::build_url(GITHUB_AUTH_URL, &params)
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

        // GitHub requires Accept: application/json to return JSON
        let (status, body) =
            http_client::post_form_with_accept(GITHUB_TOKEN_URL, &params, "application/json")?;

        if status >= 400 {
            if let Ok(err) = serde_json::from_str::<OAuthError>(&body) {
                return Err(format!("GitHub token exchange failed: {}", err));
            }
            return Err(format!(
                "GitHub token exchange failed (HTTP {}): {}",
                status, body
            ));
        }

        // GitHub has a non-standard response format — may include error in 200 response
        let github_resp: GitHubTokenResponse = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse GitHub token response: {}", e))?;

        if let Some(error) = &github_resp.error {
            let desc = github_resp
                .error_description
                .as_deref()
                .unwrap_or("Unknown error");
            return Err(format!("GitHub token exchange failed: {}: {}", error, desc));
        }

        let access_token = github_resp
            .access_token
            .ok_or("GitHub response missing access_token")?;

        Ok(TokenResponse {
            access_token,
            token_type: github_resp
                .token_type
                .unwrap_or_else(|| "bearer".to_string()),
            expires_in: None,    // GitHub tokens don't expire by default
            refresh_token: None, // GitHub doesn't issue refresh tokens by default
            scope: github_resp.scope,
            id_token: None, // GitHub doesn't support OIDC
        })
    }

    fn refresh_token(&self, _refresh_token: &str) -> Result<TokenResponse, String> {
        // GitHub doesn't issue refresh tokens by default.
        // GitHub access tokens are long-lived until manually revoked.
        Err("GitHub does not support token refresh. GitHub tokens are long-lived and do not expire unless revoked.".to_string())
    }

    fn get_user_info(&self, access_token: &str) -> Result<UserInfo, String> {
        let (status, body) =
            http_client::get_with_bearer_and_ua(GITHUB_USER_URL, access_token, "Doo-OAuth/1.0")?;

        if status >= 400 {
            return Err(format!(
                "GitHub userinfo request failed (HTTP {}): {}",
                status, body
            ));
        }

        let github_user: GitHubUser = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse GitHub user info: {}", e))?;

        // GitHub may not return email in /user — fetch from /user/emails
        let (email, email_verified) = if let Some(ref email) = github_user.email {
            if !email.is_empty() {
                (email.clone(), true)
            } else {
                self.fetch_primary_email(access_token)?
            }
        } else {
            self.fetch_primary_email(access_token)?
        };

        Ok(UserInfo {
            id: github_user.id.to_string(),
            email,
            name: github_user.name.or(Some(github_user.login)),
            avatar: github_user.avatar_url,
            provider: "github".to_string(),
            email_verified,
            created_at: None, // Set by auth layer on user creation
            updated_at: None, // Set by auth layer on user creation
        })
    }

    // GitHub does not have a standard token revocation endpoint.
    // Token deletion requires the GitHub API with the OAuth app's client_id/secret.
    // We intentionally leave the default (unsupported) implementation.
}
