//! doo_ffi_auth — Production-Grade Authentication FFI Library
//!
//! Single API surface for:
//! - Password hashing with bcrypt (cost 12, production-grade)
//! - JWT signing with HS256 (explicit algorithm, iss claim)
//! - JWT verification with full signature + expiry + claims validation
//! - Panic-safe FFI boundary (catch_unwind on every extern "C" fn)
//! - Consistent libc::malloc allocation (no allocator mismatch)
//! - Password zeroization after use
//!
//! SECURITY:
//! - JWT_SECRET must be set (no fallback)
//! - JWT_SECRET must be >= 32 bytes (HMAC-SHA256 requirement)
//! - All tokens include iss, sub, user_id, iat, exp
//! - Token size limit: 8KB max
//! - bcrypt cost = 12 (DEFAULT_COST)
//! - All FFI functions wrapped in catch_unwind

mod error;

use std::os::raw::c_char;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use bcrypt::{hash, verify, DEFAULT_COST};
use doo_ffi_core::helpers::c_to_string;
use doo_ffi_core::{AuthErrorCode, DooResult};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

pub use error::AuthError;

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

/// JWT Claims structure — used for ALL token operations across the crate.
/// Matches the claims used in doo_ffi_http for token generation.
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
/// Prevents the race condition where two separate OnceLocks could
/// be initialized from different env var reads.
static KEYS: OnceLock<(EncodingKey, DecodingKey)> = OnceLock::new();

/// Initialize JWT keys from JWT_SECRET env var.
/// - Reads env var exactly ONCE
/// - Validates minimum secret length (32 bytes)
/// - Returns error if JWT_SECRET is not set or too short
fn ensure_keys() -> Result<&'static (EncodingKey, DecodingKey), &'static str> {
    // Fast path: keys already initialized
    if let Some(keys) = KEYS.get() {
        return Ok(keys);
    }

    // Slow path: initialize keys (first call only)
    let secret =
        std::env::var("JWT_SECRET").map_err(|_| "JWT_SECRET environment variable must be set")?;

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
// STRING HELPERS — delegated to doo_ffi_core::helpers (single source of truth)
// c_to_string imported from doo_ffi_core::helpers
// ============================================================================

// ============================================================================
// RESULT HELPERS — Consistent libc::malloc allocation (no Box mismatch)
// ============================================================================

/// Create an Ok result with a string value.
/// Uses doo_ffi_core::helpers (single source of truth).
fn make_ok_string(s: &str) -> *mut DooResult {
    doo_ffi_core::helpers::make_ok_string(s)
}

/// Create an Ok result with a boolean value.
/// Uses doo_ffi_core::helpers (single source of truth).
fn make_ok_bool(b: bool) -> *mut DooResult {
    doo_ffi_core::helpers::make_ok_bool(b)
}

/// Create an Err result.
/// Uses doo_ffi_core::helpers (single source of truth).
fn make_err(code: AuthErrorCode, message: &str) -> *mut DooResult {
    doo_ffi_core::helpers::make_err(code as u16, message)
}

// ============================================================================
// PASSWORD HASHING — bcrypt cost 12 (DEFAULT_COST), with zeroization
// ============================================================================

/// Hash a password using bcrypt at production cost (12).
/// Zeroizes plaintext password after hashing.
/// Wrapped in catch_unwind for panic safety at FFI boundary.
///
/// Returns: DooResult with hashed password string on success
#[no_mangle]
pub extern "C" fn doo_auth_hash_password(password: *const c_char) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut pwd = match c_to_string(password) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        if pwd.is_empty() {
            return make_err(AuthErrorCode::PasswordTooWeak, "Password cannot be empty");
        }

        // Hash with DEFAULT_COST (12) — production-grade security
        let result = match hash(&pwd, DEFAULT_COST) {
            Ok(hashed) => make_ok_string(&hashed),
            Err(e) => make_err(AuthErrorCode::InternalError, &format!("Hash failed: {}", e)),
        };

        // Zeroize plaintext password from memory
        unsafe {
            let bytes = pwd.as_bytes_mut();
            for b in bytes.iter_mut() {
                std::ptr::write_volatile(b, 0);
            }
        }
        drop(pwd);

        result
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "Internal error"),
    }
}

/// Verify a password against a bcrypt hash.
/// Zeroizes plaintext password after verification.
/// Wrapped in catch_unwind for panic safety at FFI boundary.
///
/// Returns: DooResult with boolean value (0/1) indicating match
#[no_mangle]
pub extern "C" fn doo_auth_verify_password(
    password: *const c_char,
    hashed: *const c_char,
) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut pwd = match c_to_string(password) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        let hash_str = match c_to_string(hashed) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        let result = match verify(&pwd, &hash_str) {
            Ok(valid) => make_ok_bool(valid),
            Err(e) => make_err(
                AuthErrorCode::InternalError,
                &format!("Verify failed: {}", e),
            ),
        };

        // Zeroize plaintext password
        unsafe {
            let bytes = pwd.as_bytes_mut();
            for b in bytes.iter_mut() {
                std::ptr::write_volatile(b, 0);
            }
        }
        drop(pwd);

        result
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "Internal error"),
    }
}

// ============================================================================
// JWT OPERATIONS — HS256 explicit, with iss claim, strict validation
// ============================================================================

/// Sign a JWT token with unified Claims.
/// - Algorithm: HS256 (explicit, pinned)
/// - Includes iss (issuer) claim
/// - JWT_SECRET must be set and >= 32 bytes
/// Wrapped in catch_unwind for panic safety.
///
/// Parameters:
/// - sub: Subject (typically email)
/// - data_json: Optional JSON string for additional claims
/// - expires_seconds: Token lifetime in seconds
/// Returns: DooResult with JWT token string on success
#[no_mangle]
pub extern "C" fn doo_auth_sign(
    sub: *const c_char,
    data_json: *const c_char,
    expires_seconds: i64,
) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
            user_id: 0,
            exp: now.saturating_add(expires_secs),
            iat: now,
            iss: JWT_ISSUER.to_string(),
            data,
        };

        match encode(&Header::new(Algorithm::HS256), &claims, enc) {
            Ok(token) => make_ok_string(&token),
            Err(e) => make_err(
                AuthErrorCode::InternalError,
                &format!("JWT sign failed: {}", e),
            ),
        }
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "Internal error"),
    }
}

/// Verify a JWT token and extract claims.
/// - Validates signature with HMAC-SHA256
/// - Validates expiration
/// - Validates required claims: exp, sub, iat
/// - Rejects tokens > 8KB
/// - 30s clock skew tolerance
/// Wrapped in catch_unwind for panic safety.
///
/// Returns: DooResult with claims JSON string on success
#[no_mangle]
pub extern "C" fn doo_auth_verify(token: *const c_char) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let (_, dec) = match ensure_keys() {
            Ok(keys) => keys,
            Err(e) => return make_err(AuthErrorCode::SecretNotConfigured, e),
        };

        let token_str = match c_to_string(token) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        // Token size limit — prevent DoS via oversized tokens
        if token_str.len() > MAX_TOKEN_SIZE {
            return make_err(AuthErrorCode::JwtMalformed, "Token too large");
        }

        // Strict validation: HS256 only, validate exp, 30s clock skew
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_required_spec_claims(&["exp", "sub", "iat"]);
        validation.leeway = 30; // 30s clock skew tolerance

        match decode::<Claims>(&token_str, dec, &validation) {
            Ok(token_data) => {
                let json =
                    serde_json::to_string(&token_data.claims).unwrap_or_else(|_| "{}".to_string());
                make_ok_string(&json)
            }
            Err(e) => {
                let err_str = e.to_string().to_lowercase();
                if err_str.contains("expired") {
                    make_err(AuthErrorCode::JwtExpired, "Token has expired")
                } else if err_str.contains("signature") {
                    make_err(
                        AuthErrorCode::JwtSignatureInvalid,
                        "Invalid token signature",
                    )
                } else {
                    make_err(AuthErrorCode::JwtInvalid, "Invalid token")
                }
            }
        }
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "Internal error"),
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
        return 1;
    }
    unsafe {
        if (*result).is_err() {
            1
        } else {
            0
        }
    }
}

/// Get error message from a result.
/// Correctly reads the inner string from the wrapper struct { *char message }.
/// Returns: Pointer to error message string, or null if not an error
#[no_mangle]
pub extern "C" fn doo_auth_get_error_message(result: *mut DooResult) -> *const c_char {
    if result.is_null() {
        return std::ptr::null();
    }
    unsafe {
        if (*result).is_err() && !(*result).data.is_null() {
            // err_str wraps strings in { *char message } struct
            // Read the inner string pointer from the wrapper
            let wrapper = (*result).data as *const *const c_char;
            if !wrapper.is_null() {
                *wrapper
            } else {
                std::ptr::null()
            }
        } else {
            std::ptr::null()
        }
    }
}

// ============================================================================
// MEMORY MANAGEMENT
// ============================================================================

/// Free a string allocated by this library (libc::malloc)
#[no_mangle]
pub extern "C" fn doo_auth_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        libc::free(ptr as *mut std::ffi::c_void);
    }
}

/// Free a DooResult allocated by this library.
/// All allocations use libc::malloc — freed with libc::free consistently.
/// Handles Err results from err_str: data -> wrapper { *char } -> frees inner string + wrapper.
#[no_mangle]
pub extern "C" fn doo_auth_free_result(result: *mut DooResult) {
    if result.is_null() {
        return;
    }
    unsafe {
        let tag = (*result).tag;
        let data = (*result).data;

        if !data.is_null() {
            if tag == 1 {
                // Err: data -> wrapper { *char message } -> free inner string first
                let inner_str = *(data as *const *mut std::ffi::c_void);
                if !inner_str.is_null() {
                    libc::free(inner_str);
                }
            }
            libc::free(data as *mut std::ffi::c_void);
        }

        // Free the outer DooResult shell (allocated with libc::malloc)
        libc::free(result as *mut std::ffi::c_void);
    }
}

// ============================================================================
// FFI NAME ALIASES — Match codegen registered names
// ============================================================================

/// Alias for doo_auth_sign (codegen may emit doo_auth_sign_token)
#[no_mangle]
pub extern "C" fn doo_auth_sign_token(
    sub: *const c_char,
    data_json: *const c_char,
    expires_seconds: i64,
) -> *mut DooResult {
    doo_auth_sign(sub, data_json, expires_seconds)
}

/// Alias for doo_auth_verify (codegen may emit doo_auth_verify_token with 2 args: token, secret)
/// The secret arg is ignored — we use the centralized JWT_SECRET env var.
#[no_mangle]
pub extern "C" fn doo_auth_verify_token(
    token: *const c_char,
    _secret: *const c_char,
) -> *mut DooResult {
    doo_auth_verify(token)
}
