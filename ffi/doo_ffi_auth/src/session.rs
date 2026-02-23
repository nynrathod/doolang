//! JWT Session Token Core — Single Source of Truth
//!
//! Shared JWT signing/verification logic used by ALL auth strategies
//! that need session tokens (JWT, OAuth, etc.).
//!
//! ## Why this exists
//!
//! Both JwtStrategy and OAuthStrategy need to sign/verify JWT session tokens.
//! This module ensures the logic is written ONCE, not duplicated.
//!
//! ## Security
//! - Algorithm: HS256 (explicit, pinned)
//! - JWT_SECRET must be set (no fallback)
//! - JWT_SECRET must be >= 32 bytes (HMAC-SHA256 requirement)
//! - All tokens include iss, sub, iat, exp
//! - Token size limit: 8KB max
//! - 30s clock skew tolerance

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

// ============================================================================
// CONSTANTS — Single Source of Truth
// ============================================================================

/// Maximum JWT token size in bytes (8KB) — prevents DoS via oversized tokens
const MAX_TOKEN_SIZE: usize = 8192;

/// JWT issuer claim — identifies tokens as Doo-issued
const JWT_ISSUER: &str = "doo";

/// Minimum secret length for HMAC-SHA256 (256 bits = 32 bytes)
const MIN_SECRET_LENGTH: usize = 32;

/// Default access token expiry: 1 hour (industry standard for SaaS)
/// Override via env var ACCESS_TOKEN_EXPIRY or code config.
const DEFAULT_ACCESS_EXPIRY_SECS: i64 = 3600;

/// Default refresh token expiry: 7 days
const DEFAULT_REFRESH_EXPIRY_SECS: i64 = 604800;

/// Token type claim value for access tokens
const TOKEN_TYPE_ACCESS: &str = "access";

/// Token type claim value for refresh tokens
const TOKEN_TYPE_REFRESH: &str = "refresh";

// ============================================================================
// UNIFIED JWT Claims — Single Source of Truth
// ============================================================================

/// JWT Claims structure — used for ALL token operations across all strategies.
///
/// This is the canonical claims format for Doo session tokens, whether created
/// by JwtStrategy (direct JWT auth) or OAuthStrategy (session after OAuth login).
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// Subject (email or user identifier)
    pub sub: String,
    /// User ID (database primary key)
    #[serde(default)]
    pub user_id: i64,
    /// Expiration time (Unix timestamp)
    pub exp: usize,
    /// Issued at (Unix timestamp)
    pub iat: usize,
    /// Issuer
    #[serde(default = "default_issuer")]
    pub iss: String,
    /// Token type: "access" or "refresh" — prevents token substitution attacks.
    /// Defaults to "access" for backward compatibility with existing tokens.
    #[serde(default = "default_token_type")]
    pub token_type: String,
    /// Optional embedded JSON data for custom claims
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

fn default_issuer() -> String {
    JWT_ISSUER.to_string()
}

fn default_token_type() -> String {
    TOKEN_TYPE_ACCESS.to_string()
}

// ============================================================================
// EXPIRY CONFIGURATION — Env var override → Code override → Default
// ============================================================================

/// Parse a duration string into seconds.
///
/// Supported formats: "15m", "1h", "7d", "30d", "900" (raw seconds)
fn parse_duration_str(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Try raw seconds first
    if let Ok(secs) = s.parse::<i64>() {
        return Some(secs);
    }
    // Try suffixed formats
    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('s') {
        (n, 1i64)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3600)
    } else if let Some(n) = s.strip_suffix('d') {
        (n, 86400)
    } else {
        return None;
    };
    num_str.trim().parse::<i64>().ok().map(|n| n * multiplier)
}

/// Get the access token expiry in seconds.
///
/// Priority: env var `ACCESS_TOKEN_EXPIRY` → default (15 minutes).
/// The caller (OAuth setup or app.auth config) can override via code config.
pub fn get_access_expiry() -> i64 {
    if let Ok(val) = std::env::var(doo_ffi_core::constants::ENV_ACCESS_TOKEN_EXPIRY) {
        if let Some(secs) = parse_duration_str(&val) {
            return secs;
        }
    }
    DEFAULT_ACCESS_EXPIRY_SECS
}

/// Get the refresh token expiry in seconds.
///
/// Priority: env var `REFRESH_TOKEN_EXPIRY` → default (7 days).
pub fn get_refresh_expiry() -> i64 {
    if let Ok(val) = std::env::var(doo_ffi_core::constants::ENV_REFRESH_TOKEN_EXPIRY) {
        if let Some(secs) = parse_duration_str(&val) {
            return secs;
        }
    }
    DEFAULT_REFRESH_EXPIRY_SECS
}

// ============================================================================
// KEY MANAGEMENT — Single OnceLock, Read Once, No Race
// ============================================================================

/// Encoding + Decoding keys stored together in a single OnceLock.
static KEYS: OnceLock<(EncodingKey, DecodingKey)> = OnceLock::new();

/// Initialize JWT keys from JWT_SECRET env var.
/// Thread-safe: OnceLock ensures exactly one initialization.
///
/// # Security
/// - Rejects missing or short secrets
/// - Rejects known insecure secrets in release builds
pub fn ensure_keys() -> Result<&'static (EncodingKey, DecodingKey), &'static str> {
    // Fast path: keys already initialized
    if let Some(keys) = KEYS.get() {
        return Ok(keys);
    }

    // Slow path: initialize keys (first call only)
    let secret = std::env::var(doo_ffi_core::constants::ENV_JWT_SECRET)
        .map_err(|_| "JWT_SECRET environment variable must be set")?;

    if secret.len() < MIN_SECRET_LENGTH {
        return Err("JWT_SECRET must be at least 32 bytes for HMAC-SHA256 security");
    }

    // Reject known insecure secrets in release builds
    #[cfg(not(debug_assertions))]
    {
        if secret == "test-secret" || secret == "secret" || secret == "password" {
            return Err("JWT_SECRET is a known insecure value — use a strong random secret");
        }
    }

    Ok(KEYS.get_or_init(|| {
        (
            EncodingKey::from_secret(secret.as_bytes()),
            DecodingKey::from_secret(secret.as_bytes()),
        )
    }))
}

// ============================================================================
// TOKEN OPERATIONS — Used by all strategies
// ============================================================================

/// Sign a JWT access token.
///
/// Used by both JwtStrategy and OAuthStrategy for session management.
/// The token format is identical regardless of the auth strategy.
///
/// # Parameters
/// - `sub`: Subject (email or user identifier)
/// - `data_json`: Optional JSON string for additional claims
/// - `expires_seconds`: Token lifetime in seconds (minimum 1).
///   Pass 0 to use the default from env/config.
pub fn sign_token(
    sub: &str,
    data_json: Option<&str>,
    expires_seconds: i64,
) -> Result<String, String> {
    let (enc, _) = ensure_keys().map_err(|e| e.to_string())?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as usize)
        .unwrap_or(0);

    // Use provided expiry, or fall back to configured default
    let expires_secs = if expires_seconds > 0 {
        expires_seconds as usize
    } else {
        get_access_expiry() as usize
    };

    let claims = Claims {
        sub: sub.to_string(),
        user_id: 0,
        exp: now.saturating_add(expires_secs),
        iat: now,
        iss: JWT_ISSUER.to_string(),
        token_type: TOKEN_TYPE_ACCESS.to_string(),
        data: data_json.map(|s| s.to_string()),
    };

    encode(&Header::new(Algorithm::HS256), &claims, enc)
        .map_err(|e| format!("JWT sign failed: {}", e))
}

/// Sign a JWT refresh token.
///
/// Refresh tokens have a longer expiry and `token_type: "refresh"`.
/// They carry minimal claims (sub only) — no user data embedded.
/// Used to obtain new access tokens without re-authentication.
///
/// # Parameters
/// - `sub`: Subject (email or user identifier)
/// - `expires_seconds`: Token lifetime in seconds. Pass 0 for default (7 days).
pub fn sign_refresh_token(sub: &str, expires_seconds: i64) -> Result<String, String> {
    let (enc, _) = ensure_keys().map_err(|e| e.to_string())?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as usize)
        .unwrap_or(0);

    let expires_secs = if expires_seconds > 0 {
        expires_seconds as usize
    } else {
        get_refresh_expiry() as usize
    };

    let claims = Claims {
        sub: sub.to_string(),
        user_id: 0,
        exp: now.saturating_add(expires_secs),
        iat: now,
        iss: JWT_ISSUER.to_string(),
        token_type: TOKEN_TYPE_REFRESH.to_string(),
        data: None, // Refresh tokens carry no user data
    };

    encode(&Header::new(Algorithm::HS256), &claims, enc)
        .map_err(|e| format!("JWT sign failed: {}", e))
}

/// Verify a JWT access token.
///
/// Used by both JwtStrategy and OAuthStrategy for session verification.
/// Validates expiry, required claims, signature, and ensures it's an access token.
/// Rejects refresh tokens — they cannot be used for API authentication.
///
/// # Returns
/// JSON string of the decoded claims on success.
pub fn verify_token(token: &str) -> Result<String, String> {
    let claims = decode_token(token)?;

    // Reject refresh tokens used as access tokens (token substitution attack)
    if claims.token_type == TOKEN_TYPE_REFRESH {
        return Err("Refresh tokens cannot be used for API authentication".to_string());
    }

    serde_json::to_string(&claims).map_err(|e| format!("Claims serialization failed: {}", e))
}

/// Verify a refresh token and return the subject.
///
/// Validates expiry, signature, and ensures `token_type == "refresh"`.
/// Returns the subject (email/user identifier) for issuing a new access token.
pub fn verify_refresh_token(token: &str) -> Result<String, String> {
    let claims = decode_token(token)?;

    if claims.token_type != TOKEN_TYPE_REFRESH {
        return Err("Invalid refresh token".to_string());
    }

    Ok(claims.sub)
}

/// Internal: decode and validate a JWT token (any type).
fn decode_token(token: &str) -> Result<Claims, String> {
    let (_, dec) = ensure_keys().map_err(|e| e.to_string())?;

    // Token size limit — prevents DoS
    if token.len() > MAX_TOKEN_SIZE {
        return Err("Token too large".to_string());
    }

    // Strict validation: HS256 only, validate exp, 30s clock skew
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_required_spec_claims(&["exp", "sub", "iat"]);
    validation.leeway = 30;

    match decode::<Claims>(token, dec, &validation) {
        Ok(token_data) => Ok(token_data.claims),
        Err(e) => {
            let err_str = e.to_string().to_lowercase();
            if err_str.contains("expired") {
                Err("Token has expired".to_string())
            } else if err_str.contains("signature") {
                Err("Invalid token signature".to_string())
            } else {
                Err("Invalid token".to_string())
            }
        }
    }
}

/// Decode an access token and return all claims as JSON.
///
/// Used by `/auth/me` endpoint to return the current user's identity data.
/// Only accepts access tokens (rejects refresh tokens).
///
/// Returns JSON: `{ "data": { "sub": "...", "email": "...", "name": "...", ... }, "expires_at": ..., "issued_at": ... }`
///
/// Consistent with app.auth /auth/me format: `{ "data": { ...user_info } }`
/// The `data` object flattens the embedded claims data alongside `sub`.
pub fn get_token_claims_json(token: &str) -> Result<String, String> {
    let claims = decode_token(token)?;

    // Reject refresh tokens — /auth/me is for access tokens only
    if claims.token_type == TOKEN_TYPE_REFRESH {
        return Err("Refresh tokens cannot be used for /auth/me".to_string());
    }

    // Start with sub as the base identity
    let mut user_data = serde_json::json!({
        "sub": claims.sub,
    });

    // Flatten embedded data into the user_data object (not nested under "data")
    if let Some(data_str) = claims.data.as_deref() {
        if let Ok(serde_json::Value::Object(data_obj)) = serde_json::from_str::<serde_json::Value>(data_str) {
            if let Some(obj) = user_data.as_object_mut() {
                for (k, v) in data_obj {
                    obj.insert(k, v);
                }
            }
        }
    }

    let result = serde_json::json!({
        "data": user_data,
        "expires_at": claims.exp,
        "issued_at": claims.iat,
    });

    serde_json::to_string(&result)
        .map_err(|e| format!("Failed to serialize claims: {}", e))
}
