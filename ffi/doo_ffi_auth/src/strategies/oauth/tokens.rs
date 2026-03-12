//! OAuth Token Types — Shared data structures for all OAuth providers.
//!
//! These types are the canonical representation of OAuth responses,
//! normalized across providers (Google, GitHub, etc.).
//!
//! ## Single Source of Truth
//! All provider implementations serialize to/from these types.

use serde::{Deserialize, Serialize};

// ============================================================================
// TOKEN RESPONSE — OAuth 2.0 Token Endpoint Response (RFC 6749 §5.1)
// ============================================================================

/// OAuth 2.0 token response — normalized across all providers.
///
/// Follows RFC 6749 §5.1 with additional fields for OIDC support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    /// The access token issued by the authorization server
    pub access_token: String,

    /// The type of the token issued (typically "Bearer")
    pub token_type: String,

    /// Lifetime in seconds of the access token (None if token doesn't expire)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,

    /// Refresh token for obtaining new access tokens (not all providers issue these)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,

    /// Space-delimited list of scopes granted
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,

    /// OpenID Connect ID Token (only for OIDC providers like Google)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
}

// ============================================================================
// USER INFO — Normalized user profile across providers
// ============================================================================

/// Normalized user info — same structure regardless of provider.
///
/// Each provider maps their specific API response to this canonical format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    /// Provider-specific user ID (string for cross-provider compatibility)
    pub id: String,

    /// User's email address
    pub email: String,

    /// User's display name (may not be available from all providers)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// URL to user's avatar/profile picture
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,

    /// Which provider this user info came from ("google", "github")
    pub provider: String,

    /// Whether the email is verified by the provider
    #[serde(default)]
    pub email_verified: bool,
}

// ============================================================================
// OAUTH ERROR — Provider error response (RFC 6749 §5.2)
// ============================================================================

/// OAuth 2.0 error response from providers.
///
/// All providers follow this format for error responses.
#[derive(Debug, Clone, Deserialize)]
pub struct OAuthError {
    /// Error code (e.g., "invalid_grant", "invalid_client")
    pub error: String,

    /// Human-readable error description
    #[serde(default)]
    pub error_description: Option<String>,

    /// URI for more error info (rarely used)
    #[serde(default)]
    pub error_uri: Option<String>,
}

impl std::fmt::Display for OAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error)?;
        if let Some(desc) = &self.error_description {
            write!(f, ": {}", desc)?;
        }
        Ok(())
    }
}

// ============================================================================
// COMBINED EXCHANGE RESULT — Tokens + User Info
// ============================================================================

/// Combined result from a full OAuth exchange flow.
///
/// Contains YOUR session tokens AND the provider's tokens for reference.
/// The frontend primarily uses `access_token` and `refresh_token` (YOUR JWTs).
///
/// ## Frontend usage:
/// - `access_token`: YOUR short-lived JWT for API authentication (use with `Jwt()` middleware)
/// - `refresh_token`: YOUR long-lived JWT for obtaining new access tokens via `/auth/refresh`
/// - `expires_in`: Access token lifetime in seconds
/// - `user`: Normalized user profile from the OAuth provider
/// - `provider_access_token`: Google/GitHub's access token (optional, for calling provider APIs)
/// - `provider_refresh_token`: Google's refresh token (optional, for calling provider APIs)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthExchangeResult {
    /// YOUR access JWT — short-lived, used for API calls (carries user data)
    pub access_token: String,

    /// YOUR refresh JWT — long-lived, used to get new access tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,

    /// Access token expiry in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,

    /// Normalized user info from the provider
    pub user: UserInfo,

    /// Provider's access token (for calling Google/GitHub APIs directly, if needed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_access_token: Option<String>,

    /// Provider's refresh token (Google only — for refreshing provider's access token)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_refresh_token: Option<String>,
}
