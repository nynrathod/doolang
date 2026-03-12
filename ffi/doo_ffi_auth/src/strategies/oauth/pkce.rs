//! PKCE (Proof Key for Code Exchange) — RFC 7636 Implementation
//!
//! PKCE protects the authorization code grant from interception attacks.
//! This is the industry standard for OAuth 2.0 public and confidential clients.
//!
//! ## Method: S256 (always)
//! - code_verifier: 43-128 chars of [A-Za-z0-9-._~]
//! - code_challenge: BASE64URL(SHA256(code_verifier))
//! - We NEVER use "plain" method — S256 only
//!
//! ## Security
//! - Uses cryptographic random bytes (OS entropy via rand)
//! - 32 bytes of entropy → 43-char base64url string
//! - SHA-256 challenge prevents verifier interception

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// PKCE challenge pair (code_verifier + code_challenge).
///
/// Generated once per OAuth authorization request.
/// The code_challenge is sent with the auth URL, the code_verifier
/// is sent with the token exchange to prove ownership.
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    /// Random secret kept by the client (sent during token exchange)
    pub code_verifier: String,

    /// SHA-256 hash of code_verifier, base64url-encoded (sent with auth URL)
    pub code_challenge: String,
}

impl PkceChallenge {
    /// Generate a new PKCE challenge pair using cryptographic randomness.
    ///
    /// # Security
    /// - 32 bytes of OS-level entropy
    /// - Base64url encoding produces a 43-char code_verifier
    /// - SHA-256 + base64url for code_challenge (S256 method)
    pub fn generate() -> Self {
        // Generate 32 bytes of cryptographic randomness
        let mut random_bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut random_bytes);

        // code_verifier: base64url-encode the random bytes (43 chars for 32 bytes)
        let code_verifier = URL_SAFE_NO_PAD.encode(random_bytes);

        // code_challenge: SHA-256(code_verifier), then base64url-encode
        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let hash = hasher.finalize();
        let code_challenge = URL_SAFE_NO_PAD.encode(hash);

        PkceChallenge {
            code_verifier,
            code_challenge,
        }
    }

    /// Get the PKCE method string for the authorization URL.
    ///
    /// Always returns "S256" — we never use "plain".
    pub fn method() -> &'static str {
        "S256"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pkce_generate() {
        let pkce = PkceChallenge::generate();

        // code_verifier should be 43 chars (32 bytes base64url-encoded without padding)
        assert_eq!(pkce.code_verifier.len(), 43);

        // code_challenge should be 43 chars (32 bytes SHA-256 hash base64url-encoded)
        assert_eq!(pkce.code_challenge.len(), 43);

        // They should be different
        assert_ne!(pkce.code_verifier, pkce.code_challenge);
    }

    #[test]
    fn test_pkce_unique() {
        let a = PkceChallenge::generate();
        let b = PkceChallenge::generate();

        // Two generations should produce different values
        assert_ne!(a.code_verifier, b.code_verifier);
        assert_ne!(a.code_challenge, b.code_challenge);
    }

    #[test]
    fn test_pkce_method() {
        assert_eq!(PkceChallenge::method(), "S256");
    }
}
