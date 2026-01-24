//! doo_ffi_auth - Complete Authentication FFI Library
//!
//! Provides:
//! - Password hashing with bcrypt
//! - JWT signing with expiration
//! - JWT verification with claims extraction
//! - Error handling with RFC 7807 compatible errors

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bcrypt::{hash, verify, DEFAULT_COST};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use doo_ffi_core::{DooResult, AuthErrorCode};

// ============================================================================
// JWT Claims Structure
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,          // Subject (user ID)
    exp: usize,           // Expiration time
    iat: usize,           // Issued at
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<String>, // Optional embedded JSON data
}

// ============================================================================
// Key Management
// ============================================================================

static ENCODING_KEY: OnceLock<EncodingKey> = OnceLock::new();
static DECODING_KEY: OnceLock<DecodingKey> = OnceLock::new();

fn ensure_keys() -> Result<(&'static EncodingKey, &'static DecodingKey), &'static str> {
    let secret = std::env::var("JWT_SECRET").map_err(|_| "JWT_SECRET not set")?;
    
    let enc = ENCODING_KEY.get_or_init(|| EncodingKey::from_secret(secret.as_bytes()));
    let dec = DECODING_KEY.get_or_init(|| DecodingKey::from_secret(secret.as_bytes()));
    
    Ok((enc, dec))
}

// ============================================================================
// String Helpers
// ============================================================================

fn c_to_string(s: *const c_char) -> Result<String, String> {
    if s.is_null() {
        return Err("Null pointer".to_string());
    }
    unsafe {
        CStr::from_ptr(s)
            .to_str()
            .map(|s| s.to_string())
            .map_err(|e| format!("Invalid UTF-8: {}", e))
    }
}

fn string_to_c(s: &str) -> *const c_char {
    unsafe {
        let len = s.len();
        let ptr = libc::malloc(len + 1) as *mut u8;
        if ptr.is_null() {
            return std::ptr::null();
        }
        std::ptr::copy_nonoverlapping(s.as_ptr(), ptr, len);
        *ptr.add(len) = 0;
        ptr as *const c_char
    }
}

// ============================================================================
// Result Helpers
// ============================================================================

fn make_ok_string(s: &str) -> *mut DooResult {
    DooResult::ok_string(s).into_raw()
}

fn make_ok_bool(b: bool) -> *mut DooResult {
    // Return boolean as integer in the data pointer
    unsafe {
        let ptr = libc::malloc(std::mem::size_of::<DooResult>()) as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        (*ptr) = DooResult::ok((b as i32) as *mut std::ffi::c_void, 0);
        ptr
    }
}

fn make_err(code: AuthErrorCode, message: &str) -> *mut DooResult {
    DooResult::err_str(code as u16, message).into_raw()
}

// ============================================================================
// PASSWORD HASHING
// ============================================================================

/// Hash a password using bcrypt
/// Returns: DooResult with hashed password string on success
#[no_mangle]
pub extern "C" fn doo_auth_hash_password(password: *const c_char) -> *mut DooResult {
    let pwd = match c_to_string(password) {
        Ok(s) => s,
        Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
    };
    
    // Use slightly lower cost for better performance (DEFAULT_COST is 12, we use 8)
    match hash(&pwd, DEFAULT_COST - 4) {
        Ok(hashed) => make_ok_string(&hashed),
        Err(e) => make_err(AuthErrorCode::InternalError, &format!("Hash failed: {}", e)),
    }
}

/// Verify a password against a hash
/// Returns: DooResult with boolean value (0/1) indicating match
#[no_mangle]
pub extern "C" fn doo_auth_verify_password(password: *const c_char, hashed: *const c_char) -> *mut DooResult {
    let pwd = match c_to_string(password) {
        Ok(s) => s,
        Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
    };
    
    let hash_str = match c_to_string(hashed) {
        Ok(s) => s,
        Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
    };
    
    match verify(&pwd, &hash_str) {
        Ok(valid) => make_ok_bool(valid),
        Err(e) => make_err(AuthErrorCode::InternalError, &format!("Verify failed: {}", e)),
    }
}

// ============================================================================
// JWT OPERATIONS
// ============================================================================

/// Sign a JWT token
/// Parameters:
/// - sub: Subject (typically user ID)
/// - data_json: Optional JSON string for additional claims
/// - expires_seconds: Token lifetime in seconds
/// Returns: DooResult with JWT token string on success
#[no_mangle]
pub extern "C" fn doo_auth_sign(sub: *const c_char, data_json: *const c_char, expires_seconds: i32) -> *mut DooResult {
    let (enc, _) = match ensure_keys() {
        Ok(keys) => keys,
        Err(e) => return make_err(AuthErrorCode::SecretNotConfigured, e),
    };
    
    let sub_str = match c_to_string(sub) {
        Ok(s) => s,
        Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
    };
    
    let data = if data_json.is_null() {
        None
    } else {
        match c_to_string(data_json) {
            Ok(s) if !s.is_empty() => Some(s),
            _ => None,
        }
    };
    
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as usize)
        .unwrap_or(0);
    
    let expires_secs = expires_seconds.max(1) as usize;
    
    let claims = Claims {
        sub: sub_str,
        exp: now.saturating_add(expires_secs),
        iat: now,
        data,
    };
    
    match encode(&Header::new(Algorithm::HS256), &claims, enc) {
        Ok(token) => make_ok_string(&token),
        Err(e) => make_err(AuthErrorCode::InternalError, &format!("JWT sign failed: {}", e)),
    }
}

/// Verify a JWT token and extract claims
/// Returns: DooResult with claims JSON string on success
#[no_mangle]
pub extern "C" fn doo_auth_verify(token: *const c_char) -> *mut DooResult {
    let (_, dec) = match ensure_keys() {
        Ok(keys) => keys,
        Err(e) => return make_err(AuthErrorCode::SecretNotConfigured, e),
    };
    
    let token_str = match c_to_string(token) {
        Ok(s) => s,
        Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
    };
    
    let validation = Validation::new(Algorithm::HS256);
    
    match decode::<Claims>(&token_str, dec, &validation) {
        Ok(token_data) => {
            let json = serde_json::to_string(&token_data.claims)
                .unwrap_or_else(|_| "{}".to_string());
            make_ok_string(&json)
        }
        Err(e) => {
            let err_str = e.to_string().to_lowercase();
            if err_str.contains("expired") {
                make_err(AuthErrorCode::JwtExpired, "Token has expired")
            } else {
                make_err(AuthErrorCode::JwtInvalid, "Invalid token")
            }
        }
    }
}

// ============================================================================
// RESULT INSPECTION
// ============================================================================

/// Check if a result is an error
/// Returns: 0 for success, 1 for error
#[no_mangle]
pub extern "C" fn doo_auth_is_error(result: *mut DooResult) -> i32 {
    if result.is_null() {
        return 1; // Null is treated as error
    }
    unsafe {
        if (*result).is_err() { 1 } else { 0 }
    }
}

/// Get error message from a result
/// Returns: Pointer to error message string, or null if not an error
#[no_mangle]
pub extern "C" fn doo_auth_get_error_message(result: *mut DooResult) -> *const c_char {
    if result.is_null() {
        return std::ptr::null();
    }
    unsafe {
        if (*result).is_err() {
            (*result).data as *const c_char
        } else {
            std::ptr::null()
        }
    }
}

// ============================================================================
// MEMORY MANAGEMENT
// ============================================================================

/// Free a string allocated by this library
#[no_mangle]
pub extern "C" fn doo_auth_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        libc::free(ptr as *mut std::ffi::c_void);
    }
}

/// Free a DooResult allocated by this library
#[no_mangle]
pub extern "C" fn doo_auth_free_result(result: *mut DooResult) {
    if result.is_null() {
        return;
    }
    unsafe {
        let res = Box::from_raw(result);
        if !res.data.is_null() {
            libc::free(res.data);
        }
    }
}
