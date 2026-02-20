//! JWT Authentication Strategy
//!
//! Implements `crate::strategy::AuthStrategy` for JSON Web Tokens.
//!
//! ## Security
//! - Algorithm: HS256 (explicit, pinned)
//! - JWT_SECRET must be set (no fallback)
//! - JWT_SECRET must be >= 32 bytes (HMAC-SHA256 requirement)
//! - All tokens include iss, sub, user_id, iat, exp
//! - Token size limit: 8KB max
//! - 30s clock skew tolerance

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::strategy::AuthStrategy;

// ============================================================================
// CONSTANTS
// ============================================================================

/// Maximum JWT token size in bytes (8KB) — prevents DoS via oversized tokens
const MAX_TOKEN_SIZE: usize = 8192;

/// JWT issuer claim — identifies tokens as Doo-issued
const JWT_ISSUER: &str = "doo";

/// Minimum secret length for HMAC-SHA256 (256 bits = 32 bytes)
const MIN_SECRET_LENGTH: usize = 32;

// ============================================================================
// UNIFIED JWT Claims — Single Source of Truth
// ============================================================================

/// JWT Claims structure — used for ALL token operations.
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
    /// Optional embedded JSON data for custom claims
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

fn default_issuer() -> String {
    JWT_ISSUER.to_string()
}

// ============================================================================
// KEY MANAGEMENT — Single OnceLock, Read Once, No Race
// ============================================================================

/// Encoding + Decoding keys stored together in a single OnceLock.
static KEYS: OnceLock<(EncodingKey, DecodingKey)> = OnceLock::new();

/// Initialize JWT keys from JWT_SECRET env var.
fn ensure_keys() -> Result<&'static (EncodingKey, DecodingKey), &'static str> {
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
// JWT Strategy
// ============================================================================

/// JWT authentication strategy.
///
/// Stateless — all state lives in the `KEYS` OnceLock.
pub struct JwtStrategy;

impl AuthStrategy for JwtStrategy {
    fn name(&self) -> &'static str {
        "jwt"
    }

    fn sign(
        &self,
        sub: &str,
        data_json: Option<&str>,
        expires_seconds: i64,
    ) -> Result<String, String> {
        let (enc, _) = ensure_keys().map_err(|e| e.to_string())?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as usize)
            .unwrap_or(0);

        let expires_secs = expires_seconds.max(1) as usize;

        let claims = Claims {
            sub: sub.to_string(),
            user_id: 0,
            exp: now.saturating_add(expires_secs),
            iat: now,
            iss: JWT_ISSUER.to_string(),
            data: data_json.map(|s| s.to_string()),
        };

        encode(&Header::new(Algorithm::HS256), &claims, enc)
            .map_err(|e| format!("JWT sign failed: {}", e))
    }

    fn verify(&self, token: &str) -> Result<String, String> {
        let (_, dec) = ensure_keys().map_err(|e| e.to_string())?;

        // Token size limit
        if token.len() > MAX_TOKEN_SIZE {
            return Err("Token too large".to_string());
        }

        // Strict validation: HS256 only, validate exp, 30s clock skew
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_required_spec_claims(&["exp", "sub", "iat"]);
        validation.leeway = 30;

        match decode::<Claims>(token, dec, &validation) {
            Ok(token_data) => serde_json::to_string(&token_data.claims)
                .map_err(|e| format!("Claims serialization failed: {}", e)),
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
}

/// Initialize the JWT strategy — registers it as the active auth strategy.
///
/// Call this once during startup. Returns error if another strategy was already registered.
pub fn init() -> Result<(), &'static str> {
    crate::strategy::register_strategy(Box::new(JwtStrategy))
}
