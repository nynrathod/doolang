//! doo_ffi_auth — Production-Grade Authentication FFI Library
//!
//! ## Architecture
//!
//! ```text
//! Doo program → FFI (this crate) → AuthStrategy trait → strategies::jwt / strategies::oauth / ...
//! ```
//!
//! ## Generic (always available):
//! - Password hashing with bcrypt (cost 12, production-grade)
//! - Result inspection and memory management
//!
//! ## Strategy-dispatched:
//! - Token signing → dispatched through registered AuthStrategy
//! - Token verification → dispatched through registered AuthStrategy
//!
//! ## Adding a new auth strategy
//!
//! 1. Create `src/strategies/<name>/mod.rs` implementing `AuthStrategy`
//! 2. Register in `src/strategies/mod.rs`
//! 3. Done — zero compiler/codegen changes
//!
//! ## Security:
//! - JWT_SECRET must be set (no fallback)
//! - JWT_SECRET must be >= 32 bytes (HMAC-SHA256 requirement)
//! - bcrypt cost = 12 (DEFAULT_COST)
//! - All FFI functions wrapped in catch_unwind
//! - Password zeroization after use

mod error;
pub mod strategy;
pub mod strategies;

use std::os::raw::c_char;

use bcrypt::{hash, verify, DEFAULT_COST};
use doo_ffi_core::helpers::c_to_string;
use doo_ffi_core::{AuthErrorCode, DooResult};

pub use error::AuthError;

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
    doo_ffi_core::helpers::make_err_rfc7807(code as u16, message)
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
// JWT OPERATIONS — dispatched through AuthStrategy
// ============================================================================

/// Auto-initialize JWT strategy if not already registered.
/// This ensures backward compatibility — first sign/verify call auto-inits JWT.
fn ensure_strategy_initialized() {
    if !strategy::is_strategy_registered() {
        #[cfg(feature = "jwt")]
        {
            let _ = strategies::jwt::init();
        }
    }
}

/// Sign a JWT token via the registered auth strategy.
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
        ensure_strategy_initialized();

        let strat = match strategy::get_strategy() {
            Some(s) => s,
            None => return make_err(AuthErrorCode::SecretNotConfigured, "No auth strategy registered"),
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

        match strat.sign(&sub_str, data.as_deref(), expires_seconds) {
            Ok(token) => make_ok_string(&token),
            Err(e) => make_err(AuthErrorCode::InternalError, &e),
        }
    })) {
        Ok(result) => result,
        Err(_) => make_err(AuthErrorCode::InternalError, "Internal error"),
    }
}

/// Verify a JWT token via the registered auth strategy.
///
/// Returns: DooResult with claims JSON string on success
#[no_mangle]
pub extern "C" fn doo_auth_verify(token: *const c_char) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ensure_strategy_initialized();

        let strat = match strategy::get_strategy() {
            Some(s) => s,
            None => return make_err(AuthErrorCode::SecretNotConfigured, "No auth strategy registered"),
        };

        let token_str = match c_to_string(token) {
            Ok(s) => s,
            Err(e) => return make_err(AuthErrorCode::InvalidRequest, &e),
        };

        match strat.verify(&token_str) {
            Ok(claims_json) => make_ok_string(&claims_json),
            Err(e) => {
                let err_lower = e.to_lowercase();
                if err_lower.contains("expired") {
                    make_err(AuthErrorCode::JwtExpired, &e)
                } else if err_lower.contains("signature") {
                    make_err(AuthErrorCode::JwtSignatureInvalid, &e)
                } else {
                    make_err(AuthErrorCode::JwtInvalid, &e)
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
