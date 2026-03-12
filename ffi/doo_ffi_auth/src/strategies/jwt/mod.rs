//! JWT Authentication Strategy
//!
//! Implements `crate::strategy::AuthStrategy` for JSON Web Tokens.
//! Delegates all token operations to `crate::session` (single source of truth).
//!
//! ## Security
//! - Algorithm: HS256 (explicit, pinned)
//! - JWT_SECRET must be set (no fallback)
//! - JWT_SECRET must be >= 32 bytes (HMAC-SHA256 requirement)
//! - All tokens include iss, sub, user_id, iat, exp
//! - Token size limit: 8KB max
//! - 30s clock skew tolerance

use crate::session;
use crate::strategy::AuthStrategy;

// Re-export Claims for backward compatibility
pub use crate::session::Claims;

// ============================================================================
// JWT Strategy — Thin wrapper over session module
// ============================================================================

/// JWT authentication strategy.
///
/// Stateless — all state lives in `session` module static OnceLocks.
/// This is a thin wrapper that delegates to the shared session module.
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
        session::sign_token(sub, data_json, expires_seconds)
    }

    fn verify(&self, token: &str) -> Result<String, String> {
        session::verify_token(token)
    }
}

/// Initialize the JWT strategy — registers it as the active auth strategy.
///
/// Call this once during startup. Returns error if another strategy was already registered.
pub fn init() -> Result<(), &'static str> {
    crate::strategy::register_strategy(Box::new(JwtStrategy))
}
