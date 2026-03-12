//! OAuth State Manager — CSRF Protection with TTL
//!
//! Manages OAuth state tokens for CSRF (Cross-Site Request Forgery) protection.
//! Each authorization request generates a unique state token that must be
//! presented during the callback to prevent CSRF attacks.
//!
//! ## Security
//! - Cryptographically random state tokens (32 bytes of entropy)
//! - Time-to-live: 10 minutes (expired states automatically cleaned up)
//! - Single-use: state is consumed (removed) on validation
//! - Thread-safe: Mutex<HashMap> for concurrent access

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;

/// State token time-to-live — 10 minutes.
///
/// This is generous enough for slow connections but short enough
/// to limit the window for attacks.
const STATE_TTL: Duration = Duration::from_secs(600);

/// Maximum number of pending states before cleanup is forced.
///
/// Prevents memory exhaustion from abandoned OAuth flows.
const MAX_PENDING_STATES: usize = 10_000;

// ============================================================================
// STATE DATA — Associated with each state token
// ============================================================================

/// Data associated with a pending OAuth state token.
pub struct StateData {
    /// PKCE code_verifier (needed for token exchange)
    pub code_verifier: Option<String>,

    /// Which provider this state is for ("google", "github")
    pub provider: String,

    /// When this state was created (for TTL enforcement)
    pub created_at: Instant,
}

// ============================================================================
// STATE MANAGER — Thread-safe state storage with TTL
// ============================================================================

/// Manages OAuth CSRF state tokens with automatic expiry.
///
/// Thread-safe via Mutex. States are single-use (consumed on validation)
/// and expire after STATE_TTL (10 minutes).
pub struct StateManager {
    states: Mutex<HashMap<String, StateData>>,
}

impl StateManager {
    /// Create a new empty state manager.
    pub fn new() -> Self {
        StateManager {
            states: Mutex::new(HashMap::new()),
        }
    }

    /// Create a new state token for an OAuth authorization request.
    ///
    /// # Parameters
    /// - `provider`: Which provider this is for ("google", "github")
    /// - `code_verifier`: Optional PKCE code_verifier to associate
    ///
    /// # Returns
    /// The state token string (to be included in the authorization URL)
    pub fn create_state(&self, provider: &str, code_verifier: Option<String>) -> String {
        // Generate cryptographically random state token
        let mut random_bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut random_bytes);
        let state = URL_SAFE_NO_PAD.encode(random_bytes);

        let data = StateData {
            code_verifier,
            provider: provider.to_string(),
            created_at: Instant::now(),
        };

        let mut map = self.states.lock().expect("State lock poisoned");

        // Cleanup expired states if we're getting too many
        if map.len() >= MAX_PENDING_STATES {
            Self::cleanup_expired(&mut map);
        }

        map.insert(state.clone(), data);
        state
    }

    /// Validate and consume a state token from an OAuth callback.
    ///
    /// # Security
    /// - State is removed after validation (single-use)
    /// - Expired states are rejected
    /// - Missing states are rejected (prevents replay/CSRF)
    ///
    /// # Returns
    /// The associated StateData on success, error message on failure.
    pub fn validate_and_consume(&self, state: &str) -> Result<StateData, String> {
        let mut map = self.states.lock().expect("State lock poisoned");

        // Clean up expired states on every validation
        Self::cleanup_expired(&mut map);

        // Remove and return the state data (single-use)
        let data = map
            .remove(state)
            .ok_or_else(|| "Invalid or expired OAuth state token".to_string())?;

        // Check TTL
        if data.created_at.elapsed() > STATE_TTL {
            return Err("OAuth state token has expired (>10 minutes)".to_string());
        }

        Ok(data)
    }

    /// Remove all expired states from the map.
    fn cleanup_expired(map: &mut HashMap<String, StateData>) {
        map.retain(|_, data| data.created_at.elapsed() <= STATE_TTL);
    }
}

impl Default for StateManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_validate_state() {
        let manager = StateManager::new();

        let state = manager.create_state("google", Some("verifier123".to_string()));
        assert!(!state.is_empty());

        let data = manager.validate_and_consume(&state).unwrap();
        assert_eq!(data.provider, "google");
        assert_eq!(data.code_verifier.as_deref(), Some("verifier123"));
    }

    #[test]
    fn test_state_single_use() {
        let manager = StateManager::new();

        let state = manager.create_state("github", None);

        // First validation should succeed
        assert!(manager.validate_and_consume(&state).is_ok());

        // Second validation should fail (consumed)
        assert!(manager.validate_and_consume(&state).is_err());
    }

    #[test]
    fn test_invalid_state() {
        let manager = StateManager::new();

        assert!(manager.validate_and_consume("nonexistent").is_err());
    }
}
