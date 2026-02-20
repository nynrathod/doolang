//! Authentication Strategy Trait — Extensible auth architecture.
//!
//! Any auth backend (JWT, OAuth, API Key, etc.) implements `AuthStrategy`.
//! The base `doo_ffi_auth` crate dispatches sign/verify through the registered strategy.
//!
//! ## Adding a new auth strategy
//!
//! 1. Create a new module under `src/strategies/<name>/` (e.g., `src/strategies/oauth/`)
//! 2. Implement `AuthStrategy` for your strategy struct
//! 3. Add deps to `Cargo.toml` (can use feature gates)
//! 4. Register from `src/strategies/mod.rs`
//!
//! **Zero compiler changes required. Zero codegen changes required.**

use std::sync::OnceLock;

/// Authentication strategy trait — each backend implements this.
///
/// Strategies handle token/credential generation and verification.
/// Password hashing (bcrypt) is NOT part of strategies — it's always available
/// as a generic utility regardless of the auth strategy.
pub trait AuthStrategy: Send + Sync + 'static {
    /// Strategy name for logging (e.g., "jwt", "oauth", "api_key").
    fn name(&self) -> &'static str;

    /// Sign/generate a token or credential.
    ///
    /// - `sub`: Subject (typically email or user identifier)
    /// - `data_json`: Optional JSON string for additional claims/metadata
    /// - `expires_seconds`: Token/credential lifetime in seconds
    ///
    /// Returns: Token/credential string on success, error message on failure.
    fn sign(
        &self,
        sub: &str,
        data_json: Option<&str>,
        expires_seconds: i64,
    ) -> Result<String, String>;

    /// Verify a token or credential.
    ///
    /// Returns: Claims/payload JSON string on success, error message on failure.
    fn verify(&self, token: &str) -> Result<String, String>;
}

// ============================================================================
// Strategy Registry — OnceLock for zero-overhead after initialization
// ============================================================================

static STRATEGY: OnceLock<Box<dyn AuthStrategy>> = OnceLock::new();

/// Register the active auth strategy.
///
/// Called by strategy implementations during initialization.
/// Can only be called once — first strategy wins.
pub fn register_strategy(strategy: Box<dyn AuthStrategy>) -> Result<(), &'static str> {
    STRATEGY
        .set(strategy)
        .map_err(|_| "Auth strategy already registered")
}

/// Get the active auth strategy.
///
/// Returns `None` if no strategy has been registered.
pub fn get_strategy() -> Option<&'static dyn AuthStrategy> {
    STRATEGY.get().map(|b| b.as_ref())
}

/// Check if a strategy has been registered.
pub fn is_strategy_registered() -> bool {
    STRATEGY.get().is_some()
}
