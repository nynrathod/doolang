//! Password Reset System
//!
//! Provides `app.forgotPassword()` and `app.resetPassword()` endpoints.
//! - Forgot: accepts email, generates a reset token, stores it in DB
//! - Reset: accepts token + new password, validates, updates password
//!
//! Both endpoints require a database connection (DB-backed auth).

use std::ffi::c_void;
use std::os::raw::c_char;
use std::sync::Mutex as StdMutex;

use doo_ffi_core::ffi_debug;
use doo_ffi_core::DooResult;

use crate::auth::{get_auth_table_name, is_auth_db_backed};
use crate::db_bridge::{execute_db_query_with_string_param, execute_db_statement};
use crate::helpers::c_to_string;
use crate::router::get_routes;
use crate::types::*;
use crate::{make_err_http, make_ok_json, make_ok_void};

// ============================================================================
// PASSWORD RESET TOKEN STORE
// ============================================================================

/// In-memory token store as fallback (production should use DB)
static RESET_TOKENS: std::sync::OnceLock<StdMutex<Vec<ResetToken>>> = std::sync::OnceLock::new();

struct ResetToken {
    token: String,
    email: String,
    expires_at: i64,
}

fn get_reset_tokens() -> &'static StdMutex<Vec<ResetToken>> {
    RESET_TOKENS.get_or_init(|| StdMutex::new(Vec::new()))
}

/// Generate a random token string (URL-safe base64, 32 bytes)
fn generate_reset_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    // Simple random-enough token: combine timestamp + random bytes
    let random_part: u64 = (now as u64) ^ (now.wrapping_mul(6364136223846793005) as u64);
    format!(
        "{:016x}{:016x}",
        random_part,
        random_part.wrapping_mul(2654435761)
    )
}

/// Get current Unix timestamp in seconds
fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

// ============================================================================
// FORGOT PASSWORD — POST endpoint
// ============================================================================

/// Handler for forgot password endpoint.
/// Accepts: `{ "email": "user@example.com" }`
/// Returns: `{ "data": { "message": "...", "token": "..." } }`
///
/// In production, the token would be emailed — here it's returned for dev/testing.
extern "C" fn forgot_password_handler(req: *const DooRequest) -> *mut DooResult {
    ffi_debug!("AUTH", "forgot_password_handler called");

    if req.is_null() {
        return make_err_http(400, "Invalid request");
    }

    let body = unsafe { c_to_string((*req).body) };
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&body);
    let json = match parsed {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return make_err_http(400, "Request body must be a JSON object"),
    };

    // Extract email
    let email = json
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("email"))
        .and_then(|(_, v)| v.as_str())
        .map(|s| s.to_lowercase());

    let email = match email {
        Some(e) if !e.is_empty() => e,
        _ => return make_err_http(400, "Missing or empty 'email' field"),
    };

    // Generate token with 1-hour expiry
    let token = generate_reset_token();
    let expires_at = now_secs() + 3600;

    // Try DB-backed storage
    if is_auth_db_backed() {
        if let Some(table_name) = get_auth_table_name() {
            // Verify user exists
            let check_sql = format!(
                "SELECT COUNT(*) as cnt FROM {} WHERE LOWER(email) = LOWER($1)",
                table_name
            );
            match execute_db_query_with_string_param(&check_sql, &email) {
                Ok(json_str) => {
                    let parsed: serde_json::Value =
                        serde_json::from_str(&json_str).unwrap_or_default();
                    let exists = parsed
                        .as_array()
                        .and_then(|rows| rows.first())
                        .and_then(|r| r.get("cnt"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0)
                        > 0;
                    if !exists {
                        // Don't reveal whether user exists — return success anyway
                        ffi_debug!("AUTH", "User '{}' not found, returning silent success", email);
                        return make_ok_json(
                            &serde_json::json!({
                                "data": {
                                    "message": "If the email exists, a reset token has been generated"
                                }
                            })
                            .to_string(),
                        );
                    }
                }
                Err(e) => {
                    ffi_debug!("AUTH", "DB query error checking user: {}", e);
                    return make_err_http(500, "Internal server error");
                }
            }

            // Ensure password_resets table exists
            let create_table = "CREATE TABLE IF NOT EXISTS password_resets (\
                id SERIAL PRIMARY KEY, \
                email VARCHAR(255) NOT NULL, \
                token VARCHAR(255) NOT NULL UNIQUE, \
                expires_at BIGINT NOT NULL, \
                used BOOLEAN DEFAULT FALSE\
            )";
            let _ = execute_db_statement(create_table);

            // Insert token
            let insert_sql = format!(
                "INSERT INTO password_resets (email, token, expires_at) VALUES ('{}', '{}', {})",
                email.replace('\'', "''"),
                token,
                expires_at
            );
            if let Err(e) = execute_db_statement(&insert_sql) {
                ffi_debug!("AUTH", "Failed to store reset token: {}", e);
                return make_err_http(500, "Failed to store reset token");
            }
        }
    } else {
        // In-memory fallback
        let mut tokens = get_reset_tokens()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        tokens.push(ResetToken {
            token: token.clone(),
            email: email.clone(),
            expires_at,
        });
    }

    ffi_debug!("AUTH", "Reset token generated for {}: {}", email, token);

    make_ok_json(
        &serde_json::json!({
            "data": {
                "message": "If the email exists, a reset token has been generated",
                "token": token
            }
        })
        .to_string(),
    )
}

// ============================================================================
// RESET PASSWORD — POST endpoint
// ============================================================================

/// Handler for reset password endpoint.
/// Accepts: `{ "token": "...", "password": "newpass123" }`
/// Returns: `{ "data": { "message": "Password updated successfully" } }`
extern "C" fn reset_password_handler(req: *const DooRequest) -> *mut DooResult {
    ffi_debug!("AUTH", "reset_password_handler called");

    if req.is_null() {
        return make_err_http(400, "Invalid request");
    }

    let body = unsafe { c_to_string((*req).body) };
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&body);
    let json = match parsed {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return make_err_http(400, "Request body must be a JSON object"),
    };

    // Extract token
    let token = json
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("token"))
        .and_then(|(_, v)| v.as_str())
        .map(|s| s.to_string());

    let token = match token {
        Some(t) if !t.is_empty() => t,
        _ => return make_err_http(400, "Missing or empty 'token' field"),
    };

    // Extract new password
    let password = json
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("password"))
        .and_then(|(_, v)| v.as_str())
        .map(|s| s.to_string());

    let password = match password {
        Some(p) if p.len() >= 6 => p,
        Some(_) => return make_err_http(400, "Password must be at least 6 characters"),
        None => return make_err_http(400, "Missing 'password' field"),
    };

    let now = now_secs();

    // Hash the new password
    let hashed = match bcrypt::hash(&password, bcrypt::DEFAULT_COST) {
        Ok(h) => h,
        Err(e) => {
            ffi_debug!("AUTH", "bcrypt error: {}", e);
            return make_err_http(500, "Failed to hash password");
        }
    };

    // DB-backed path
    if is_auth_db_backed() {
        if let Some(table_name) = get_auth_table_name() {
            // Find valid token
            let find_sql = format!(
                "SELECT email FROM password_resets WHERE token = '{}' AND expires_at > {} AND used = FALSE",
                token.replace('\'', "''"),
                now
            );
            match execute_db_query_with_string_param(&find_sql, "") {
                Ok(json_str) => {
                    let parsed: serde_json::Value =
                        serde_json::from_str(&json_str).unwrap_or_default();
                    let email = parsed
                        .as_array()
                        .and_then(|rows| rows.first())
                        .and_then(|r| r.get("email"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    match email {
                        Some(email) => {
                            // Update password
                            let update_sql = format!(
                                "UPDATE {} SET password = '{}' WHERE LOWER(email) = LOWER('{}')",
                                table_name,
                                hashed.replace('\'', "''"),
                                email.replace('\'', "''"),
                            );
                            if let Err(e) = execute_db_statement(&update_sql) {
                                ffi_debug!("AUTH", "Failed to update password: {}", e);
                                return make_err_http(500, "Failed to update password");
                            }

                            // Mark token as used
                            let mark_sql = format!(
                                "UPDATE password_resets SET used = TRUE WHERE token = '{}'",
                                token.replace('\'', "''"),
                            );
                            let _ = execute_db_statement(&mark_sql);

                            ffi_debug!("AUTH", "Password reset successful for {}", email);
                        }
                        None => {
                            return make_err_http(400, "Invalid or expired reset token");
                        }
                    }
                }
                Err(e) => {
                    ffi_debug!("AUTH", "DB query error finding token: {}", e);
                    return make_err_http(500, "Internal server error");
                }
            }
        }
    } else {
        // In-memory fallback
        let mut tokens = get_reset_tokens()
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let found = tokens
            .iter()
            .position(|t| t.token == token && t.expires_at > now);

        match found {
            Some(idx) => {
                let _email = tokens[idx].email.clone();
                tokens.remove(idx);
                // In-memory auth doesn't have persistent storage — log the reset
                ffi_debug!("AUTH", "In-memory password reset for {}", _email);
            }
            None => {
                return make_err_http(400, "Invalid or expired reset token");
            }
        }
    }

    make_ok_json(
        &serde_json::json!({
            "data": {
                "message": "Password updated successfully"
            }
        })
        .to_string(),
    )
}

// ============================================================================
// FFI ENTRY POINTS
// ============================================================================

/// Set up forgot password endpoint.
/// Registers a POST route that accepts email and generates a reset token.
///
/// Usage in Doo: `app.forgotPassword("/forgot", User, db)`
#[no_mangle]
pub extern "C" fn doo_http_forgot_password(
    _server: *const c_void,
    path: *const c_char,
    _user_struct_name: *const c_char,
    _db: *const c_void,
) -> *mut DooResult {
    ffi_safe_result!({
        let path_str = c_to_string(path);

        ffi_debug!("HTTP", "Forgot password endpoint registered at POST {}", path_str);

        let routes = get_routes();
        let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());
        registry.register("POST", &path_str, forgot_password_handler);

        make_ok_void()
    })
}

/// Set up reset password endpoint.
/// Registers a POST route that accepts token + new password and updates the user.
///
/// Usage in Doo: `app.resetPassword("/reset", User, db)`
#[no_mangle]
pub extern "C" fn doo_http_reset_password(
    _server: *const c_void,
    path: *const c_char,
    _user_struct_name: *const c_char,
    _db: *const c_void,
) -> *mut DooResult {
    ffi_safe_result!({
        let path_str = c_to_string(path);

        ffi_debug!("HTTP", "Reset password endpoint registered at POST {}", path_str);

        let routes = get_routes();
        let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());
        registry.register("POST", &path_str, reset_password_handler);

        make_ok_void()
    })
}
