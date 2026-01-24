//! Password Hashing
//!
//! Argon2 password hashing and verification.

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use password_hash::SaltString;
use rand::rngs::OsRng;
use doo_ffi_core::DooResult;
use crate::error::AuthError;

/// Hash a password using Argon2.
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| AuthError::HashFailed)
}

/// Verify a password against a hash.
pub fn verify_password(password: &str, hash: &str) -> Result<bool, AuthError> {
    let parsed_hash = PasswordHash::new(hash)
        .map_err(|_| AuthError::VerifyFailed)?;
    
    let argon2 = Argon2::default();
    Ok(argon2.verify_password(password.as_bytes(), &parsed_hash).is_ok())
}

// ============================================================================
// FFI Functions
// ============================================================================

/// Hash a password.
#[no_mangle]
pub extern "C" fn doo_password_hash(password: *const i8) -> DooResult {
    if password.is_null() {
        return DooResult::err_str(400, "Null password");
    }
    
    unsafe {
        let password_str = std::ffi::CStr::from_ptr(password).to_str().unwrap_or("");
        
        match hash_password(password_str) {
            Ok(hash) => {
                let len = hash.len() as u32;
                let ptr = hash.as_ptr() as *mut std::ffi::c_void;
                std::mem::forget(hash);
                DooResult::ok(ptr, len)
            }
            Err(e) => DooResult::err_str(500, &format!("{:?}", e)),
        }
    }
}

/// Verify a password.
#[no_mangle]
pub extern "C" fn doo_password_verify(password: *const i8, hash: *const i8) -> DooResult {
    if password.is_null() || hash.is_null() {
        return DooResult::err_str(400, "Null parameters");
    }
    
    unsafe {
        let password_str = std::ffi::CStr::from_ptr(password).to_str().unwrap_or("");
        let hash_str = std::ffi::CStr::from_ptr(hash).to_str().unwrap_or("");
        
        match verify_password(password_str, hash_str) {
            Ok(true) => DooResult::ok_empty(),
            Ok(false) => DooResult::err_str(401, "Invalid password"),
            Err(e) => DooResult::err_str(500, &format!("{:?}", e)),
        }
    }
}
