//! OAuth Provider Trait — Extensible provider architecture.
//!
//! Each OAuth provider (Google, GitHub, etc.) implements this trait.
//! New providers can be added by:
//! 1. Creating `strategies/oauth/<name>.rs`
//! 2. Implementing `OAuthProvider`
//! 3. Registering in `strategies/oauth/mod.rs`
//!
//! ## Design
//! - Each provider owns its configuration (client_id, client_secret, redirect_uri)
//! - Configuration comes from environment variables (secure by default)
//! - All HTTP communication is handled by each provider (they know their API specifics)
//! - Results are normalized to `TokenResponse` and `UserInfo` types

use super::tokens::{TokenResponse, UserInfo};

// ============================================================================
// PROVIDER CONFIGURATION — Shared across all providers
// ============================================================================

/// OAuth provider configuration — loaded from environment variables.
///
/// Each provider constructs this from its specific env vars.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// OAuth client ID (from provider's developer console)
    pub client_id: String,

    /// OAuth client secret (from provider's developer console)
    pub client_secret: String,

    /// Redirect URI for OAuth callback (must match provider's console)
    pub redirect_uri: String,

    /// OAuth scopes to request
    pub scopes: Vec<String>,
}

// ============================================================================
// OAUTH PROVIDER TRAIT — Each provider implements this
// ============================================================================

/// OAuth provider trait — each backend (Google, GitHub, etc.) implements this.
///
/// Providers handle the specifics of their OAuth flow:
/// - Building authorization URLs with provider-specific parameters
/// - Exchanging codes for tokens using their token endpoint
/// - Fetching user info from their userinfo endpoint
/// - Token refresh and revocation
///
/// ## Adding a new provider
///
/// 1. Create `strategies/oauth/<name>.rs`
/// 2. Implement this trait
/// 3. Register in the provider match in `strategies/oauth/mod.rs::init()`
///
/// **Zero compiler changes required. Zero codegen changes required.**
pub trait OAuthProvider: Send + Sync + 'static {
    /// Provider name for logging and identification (e.g., "google", "github")
    fn name(&self) -> &'static str;

    /// Get the provider's configuration
    fn config(&self) -> &ProviderConfig;

    /// Whether this provider supports PKCE (RFC 7636)
    ///
    /// Default: true (most modern providers support PKCE)
    fn supports_pkce(&self) -> bool {
        true
    }

    /// Build the authorization URL for redirecting users to the provider.
    ///
    /// # Parameters
    /// - `state`: CSRF protection token (managed by StateManager)
    /// - `code_challenge`: PKCE code challenge (None if provider doesn't support PKCE)
    ///
    /// # Returns
    /// Full authorization URL with all required query parameters
    fn build_auth_url(&self, state: &str, code_challenge: Option<&str>) -> String;

    /// Exchange an authorization code for tokens.
    ///
    /// # Parameters
    /// - `code`: Authorization code from the callback
    /// - `code_verifier`: PKCE code verifier (None if PKCE not used)
    ///
    /// # Returns
    /// Token response with access_token, optional refresh_token, etc.
    fn exchange_code(
        &self,
        code: &str,
        code_verifier: Option<&str>,
    ) -> Result<TokenResponse, String>;

    /// Refresh an access token using a refresh token.
    ///
    /// # Returns
    /// New token response with fresh access_token
    fn refresh_token(&self, refresh_token: &str) -> Result<TokenResponse, String>;

    /// Get normalized user info using an access token.
    ///
    /// Each provider maps their API response to the canonical `UserInfo` format.
    fn get_user_info(&self, access_token: &str) -> Result<UserInfo, String>;

    /// Revoke a token (access or refresh).
    ///
    /// Default: not supported (returns error). Override for providers that support it.
    fn revoke_token(&self, _token: &str) -> Result<(), String> {
        Err(format!(
            "Token revocation not supported by {} provider",
            self.name()
        ))
    }
}
