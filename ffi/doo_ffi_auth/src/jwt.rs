//! JWT Token Management
//!
//! Sign and verify JSON Web Tokens.
//! MEMORY MODEL: Pure Ownership/Borrow - No RC, No GC
//! All string data uses doo_alloc_string from doo_ffi_core::memory (single source of truth).

use serde::{Deserialize, Serialize};
use doo_ffi_core::{DooResult, doo_alloc_string};
use crate::error::AuthError;

/// JWT claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    /// Subject (user ID)
    pub sub: String,
    /// Expiration time (Unix timestamp)
    pub exp: u64,
    /// Issued at (Unix timestamp)
    pub iat: u64,
    /// Issuer
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
}

impl JwtClaims {
    /// Create new claims.
    pub fn new(sub: &str, exp_seconds: u64) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        Self {
            sub: sub.to_string(),
            exp: now + exp_seconds,
            iat: now,
            iss: None,
        }
    }

    /// Check if expired.
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.exp < now
    }
}

/// Sign a JWT token.
pub fn sign_token(claims: &JwtClaims, secret: &str) -> Result<String, AuthError> {
    use jsonwebtoken::{encode, Header, EncodingKey};
    
    encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|_| AuthError::SignFailed)
}

/// Verify a JWT token.
pub fn verify_token(token: &str, secret: &str) -> Result<JwtClaims, AuthError> {
    use jsonwebtoken::{decode, DecodingKey, Validation};
    
    let token_data = decode::<JwtClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|e| match e.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => AuthError::TokenExpired,
        _ => AuthError::TokenInvalid,
    })?;
    
    Ok(token_data.claims)
}

// ============================================================================
// FFI Functions
// ============================================================================

/// Sign a JWT token.
/// OWNERSHIP: Returns DooResult with string allocated using libc.
#[no_mangle]
pub extern "C" fn doo_jwt_sign(
    sub: *const i8,
    exp_seconds: u64,
    secret: *const i8,
) -> DooResult {
    if sub.is_null() || secret.is_null() {
        return DooResult::err_str(400, "Null parameters");
    }
    
    unsafe {
        let sub_str = std::ffi::CStr::from_ptr(sub).to_str().unwrap_or("");
        let secret_str = std::ffi::CStr::from_ptr(secret).to_str().unwrap_or("");
        
        let claims = JwtClaims::new(sub_str, exp_seconds);
        match sign_token(&claims, secret_str) {
            Ok(token) => {
                let len = token.len() as u32;
                // Use centralized string allocation - NOT std::mem::forget!
                let ptr = doo_alloc_string(&token) as *mut std::ffi::c_void;
                DooResult::ok(ptr, len)
            }
            Err(e) => DooResult::err_str(500, &format!("{:?}", e)),
        }
    }
}

/// Verify a JWT token.
/// OWNERSHIP: Returns DooResult with string allocated using libc.
#[no_mangle]
pub extern "C" fn doo_jwt_verify(token: *const i8, secret: *const i8) -> DooResult {
    if token.is_null() || secret.is_null() {
        return DooResult::err_str(400, "Null parameters");
    }
    
    unsafe {
        let token_str = std::ffi::CStr::from_ptr(token).to_str().unwrap_or("");
        let secret_str = std::ffi::CStr::from_ptr(secret).to_str().unwrap_or("");
        
        match verify_token(token_str, secret_str) {
            Ok(claims) => {
                let json = serde_json::to_string(&claims).unwrap_or_default();
                let len = json.len() as u32;
                // Use centralized string allocation - NOT std::mem::forget!
                let ptr = doo_alloc_string(&json) as *mut std::ffi::c_void;
                DooResult::ok(ptr, len)
            }
            Err(e) => DooResult::err_str(401, &format!("{:?}", e)),
        }
    }
}
