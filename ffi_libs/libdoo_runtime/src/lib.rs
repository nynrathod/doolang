//! Centralized runtime library for Doo FFI modules
//! Provides shared validation, error handling, and utilities

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FieldDecorator {
    name: String,
    args: Vec<String>,
}

/// Structured validation error for RFC 7807 responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub field_name: String,
    pub rule: String,
    pub message: String,
    pub value: String,
}

/// Thread-local storage for the last validation error (for HTTP context)
thread_local! {
    static LAST_VALIDATION_ERROR: RefCell<Option<ValidationError>> = RefCell::new(None);
}

fn set_validation_error(error: ValidationError) {
    LAST_VALIDATION_ERROR.with(|cell| {
        *cell.borrow_mut() = Some(error);
    });
}

fn clear_validation_error() {
    LAST_VALIDATION_ERROR.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

/// Get the last validation error as JSON string
/// Returns null if no error, or JSON string with error details
#[no_mangle]
pub extern "C" fn dooruntime_get_last_validation_error() -> *mut libc::c_char {
    LAST_VALIDATION_ERROR.with(|cell| {
        if let Some(error) = cell.borrow().as_ref() {
            if let Ok(json) = serde_json::to_string(error) {
                if let Ok(c_str) = CString::new(json) {
                    return c_str.into_raw();
                }
            }
        }
        std::ptr::null_mut()
    })
}

/// Clear the last validation error
#[no_mangle]
pub extern "C" fn dooruntime_clear_validation_error() {
    clear_validation_error();
}

/// Validate a single field with its decorators
/// Returns error message as C string, or null if validation passes
/// Also stores structured error in thread-local for HTTP RFC 7807 responses
#[no_mangle]
pub extern "C" fn dooruntime_validate_field(
    field_name: *const libc::c_char,
    field_type: *const libc::c_char,
    value: *const libc::c_char,
    decorators_json: *const libc::c_char,
) -> *const libc::c_char {
    if field_name.is_null() || field_type.is_null() || value.is_null() || decorators_json.is_null()
    {
        return std::ptr::null();
    }

    let field_name_str = unsafe { CStr::from_ptr(field_name).to_string_lossy().to_string() };
    let field_type_str = unsafe { CStr::from_ptr(field_type).to_string_lossy().to_string() };
    let value_str = unsafe { CStr::from_ptr(value).to_string_lossy().to_string() };
    let decorators_str = unsafe {
        CStr::from_ptr(decorators_json)
            .to_string_lossy()
            .to_string()
    };

    // Parse decorators JSON
    let decorators: Vec<FieldDecorator> = match serde_json::from_str(&decorators_str) {
        Ok(d) => d,
        Err(_) => return std::ptr::null(), // Invalid JSON, no validation
    };

    // Clear any previous validation error
    clear_validation_error();

    // Validate each decorator
    match validate_field_decorators(&field_name_str, &field_type_str, &value_str, &decorators) {
        Ok(_) => std::ptr::null(), // No error
        Err((err_msg, rule, message)) => {
            // Store structured error for HTTP context
            let validation_error = ValidationError {
                field_name: field_name_str.clone(),
                rule,
                message: message.clone(),
                value: value_str.clone(),
            };
            set_validation_error(validation_error);

            // Return simple error message as C string (backward compatibility)
            match CString::new(err_msg) {
                Ok(c_str) => c_str.into_raw(),
                Err(_) => std::ptr::null(),
            }
        }
    }
}

/// Free a string allocated by this library
#[no_mangle]
pub extern "C" fn dooruntime_free_string(ptr: *mut libc::c_char) {
    if !ptr.is_null() {
        unsafe {
            let _ = CString::from_raw(ptr);
        }
    }
}

fn validate_field_decorators(
    field_name: &str,
    field_type: &str,
    value: &str,
    decorators: &[FieldDecorator],
) -> Result<(), (String, String, String)> {
    for decorator in decorators {
        match decorator.name.as_str() {
            "email" => {
                if field_type != "Str" {
                    return Err((
                        format!(
                            "Field '{}' has @email decorator but type is {}, expected Str",
                            field_name, field_type
                        ),
                        "email".to_string(),
                        "Email decorator requires String type".to_string(),
                    ));
                }
                // Email validation: must contain @ and . with chars before/after @
                let parts: Vec<&str> = value.split('@').collect();
                if parts.len() != 2 || parts[0].is_empty() || !parts[1].contains('.') {
                    return Err((
                        format!(
                            "Field '{}': '{}' is not a valid email address",
                            field_name, value
                        ),
                        "email".to_string(),
                        "Invalid email format".to_string(),
                    ));
                }
            }
            "min" => {
                if let Some(min_arg) = decorator.args.first() {
                    if field_type == "Str" {
                        // For strings, min is length
                        if let Ok(min_len) = min_arg.parse::<usize>() {
                            if value.len() < min_len {
                                return Err((
                                    format!(
                                        "Field '{}': must have at least {} characters (got {})",
                                        field_name,
                                        min_len,
                                        value.len()
                                    ),
                                    format!("min:{}", min_len),
                                    format!("Must be at least {} characters", min_len),
                                ));
                            }
                        }
                    } else if field_type == "Int" {
                        // For Int, min is numeric value
                        if let Ok(min_val) = min_arg.parse::<i64>() {
                            if let Ok(val) = value.parse::<i64>() {
                                if val < min_val {
                                    return Err((
                                        format!(
                                            "Field '{}': value {} is below minimum {}",
                                            field_name, val, min_val
                                        ),
                                        format!("min:{}", min_val),
                                        format!("Must be at least {}", min_val),
                                    ));
                                }
                            }
                        }
                    } else if field_type == "Float" {
                        // For Float, min is numeric value
                        if let Ok(min_val) = min_arg.parse::<f64>() {
                            if let Ok(val) = value.parse::<f64>() {
                                if val < min_val {
                                    return Err((
                                        format!(
                                            "Field '{}': value {} is below minimum {}",
                                            field_name, val, min_val
                                        ),
                                        format!("min:{}", min_val),
                                        format!("Must be at least {}", min_val),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            "max" => {
                if let Some(max_arg) = decorator.args.first() {
                    if field_type == "Str" {
                        // For strings, max is length
                        if let Ok(max_len) = max_arg.parse::<usize>() {
                            if value.len() > max_len {
                                return Err((
                                    format!(
                                        "Field '{}': must have at most {} characters (got {})",
                                        field_name,
                                        max_len,
                                        value.len()
                                    ),
                                    format!("max:{}", max_len),
                                    format!("Maximum {} characters allowed", max_len),
                                ));
                            }
                        }
                    } else if field_type == "Int" {
                        // For Int, max is numeric value
                        if let Ok(max_val) = max_arg.parse::<i64>() {
                            if let Ok(val) = value.parse::<i64>() {
                                if val > max_val {
                                    return Err((
                                        format!(
                                            "Field '{}': value {} exceeds maximum {}",
                                            field_name, val, max_val
                                        ),
                                        format!("max:{}", max_val),
                                        format!("Maximum {} allowed", max_val),
                                    ));
                                }
                            }
                        }
                    } else if field_type == "Float" {
                        // For Float, max is numeric value
                        if let Ok(max_val) = max_arg.parse::<f64>() {
                            if let Ok(val) = value.parse::<f64>() {
                                if val > max_val {
                                    return Err((
                                        format!(
                                            "Field '{}': value {} exceeds maximum {}",
                                            field_name, val, max_val
                                        ),
                                        format!("max:{}", max_val),
                                        format!("Maximum {} allowed", max_val),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            "enum" => {
                if field_type != "Str" {
                    return Err((
                        format!(
                            "Field '{}' has @enum decorator but type is {}, expected Str",
                            field_name, field_type
                        ),
                        "enum".to_string(),
                        "Enum decorator requires String type".to_string(),
                    ));
                }
                // Check if value is in the allowed enum list
                if !decorator.args.contains(&value.to_string()) {
                    return Err((
                        format!(
                            "Field '{}': '{}' is not a valid option. Must be one of: {}",
                            field_name,
                            value,
                            decorator.args.join(", ")
                        ),
                        format!("enum({})", decorator.args.join("|")),
                        format!("Must be one of: {}", decorator.args.join(", ")),
                    ));
                }
            }
            "required" => {
                if value.is_empty() {
                    return Err((
                        format!("Field '{}' is required and cannot be empty", field_name),
                        "required".to_string(),
                        "This field is required".to_string(),
                    ));
                }
            }
            "optional" => {
                // Always valid - just marks field as optional
            }
            "unique" => {
                // @unique is DB-specific and must be validated at DB layer
                // Runtime can't check uniqueness without DB query
                // This is a no-op here; DB FFI will handle it
            }
            _ => {
                // Unknown decorator - ignore
            }
        }
    }
    Ok(())
}

// ============================================================================
// DATABASE ERROR FORMATTING (PostgreSQL Standard)
// ============================================================================

/// Database error codes following PostgreSQL conventions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbErrorCode {
    ConnectionFailed,
    QueryFailed,
    UniqueViolation,
    ForeignKeyViolation,
    NotNullViolation,
    CheckViolation,
    InvalidSql,
    TableNotFound,
    ColumnNotFound,
    DataTypeMismatch,
    InternalError,
}

impl DbErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ConnectionFailed => "CONNECTION_FAILED",
            Self::QueryFailed => "QUERY_FAILED",
            Self::UniqueViolation => "UNIQUE_VIOLATION",
            Self::ForeignKeyViolation => "FOREIGN_KEY_VIOLATION",
            Self::NotNullViolation => "NOT_NULL_VIOLATION",
            Self::CheckViolation => "CHECK_VIOLATION",
            Self::InvalidSql => "INVALID_SQL",
            Self::TableNotFound => "TABLE_NOT_FOUND",
            Self::ColumnNotFound => "COLUMN_NOT_FOUND",
            Self::DataTypeMismatch => "DATA_TYPE_MISMATCH",
            Self::InternalError => "INTERNAL_ERROR",
        }
    }

    pub fn pg_code(&self) -> &'static str {
        match self {
            Self::ConnectionFailed => "08006",
            Self::QueryFailed => "42601",
            Self::UniqueViolation => "23505",
            Self::ForeignKeyViolation => "23503",
            Self::NotNullViolation => "23502",
            Self::CheckViolation => "23514",
            Self::InvalidSql => "42601",
            Self::TableNotFound => "42P01",
            Self::ColumnNotFound => "42703",
            Self::DataTypeMismatch => "42804",
            Self::InternalError => "XX000",
        }
    }

    pub fn http_status(&self) -> u16 {
        match self {
            Self::ConnectionFailed | Self::InternalError => 500,
            Self::UniqueViolation => 409,
            Self::NotNullViolation | Self::CheckViolation | Self::DataTypeMismatch => 400,
            Self::TableNotFound | Self::ColumnNotFound => 404,
            _ => 500,
        }
    }

    pub fn user_message(&self) -> &'static str {
        match self {
            Self::ConnectionFailed => "Failed to connect to database",
            Self::QueryFailed => "Failed to execute query",
            Self::UniqueViolation => "A record with this value already exists",
            Self::ForeignKeyViolation => "Referenced record does not exist",
            Self::NotNullViolation => "Required field cannot be null",
            Self::CheckViolation => "Value does not meet constraints",
            Self::InvalidSql => "Invalid SQL syntax",
            Self::TableNotFound => "Table does not exist",
            Self::ColumnNotFound => "Column does not exist",
            Self::DataTypeMismatch => "Data type mismatch",
            Self::InternalError => "Internal database error",
        }
    }
}

/// Database error structure
#[derive(Debug, Clone)]
pub struct DbError {
    pub code: DbErrorCode,
    pub message: String,
    pub detail: Option<String>,
    pub constraint: Option<String>,
}

impl DbError {
    pub fn new(code: DbErrorCode) -> Self {
        Self {
            code,
            message: code.user_message().to_string(),
            detail: None,
            constraint: None,
        }
    }

    pub fn with_message(code: DbErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
            constraint: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_constraint(mut self, constraint: impl Into<String>) -> Self {
        self.constraint = Some(constraint.into());
        self
    }

    /// Convert to JSON string for FFI responses
    pub fn to_json_string(&self) -> String {
        let mut obj = json!({
            "success": false,
            "error": {
                "code": self.code.as_str(),
                "pg_code": self.code.pg_code(),
                "message": self.message,
                "status": self.code.http_status(),
            }
        });

        if let Some(detail) = &self.detail {
            obj["error"]["detail"] = json!(detail);
        }
        if let Some(constraint) = &self.constraint {
            obj["error"]["constraint"] = json!(constraint);
        }

        obj.to_string()
    }
}

/// Helper functions for common database errors
pub fn db_connection_failed(msg: &str) -> DbError {
    DbError::with_message(DbErrorCode::ConnectionFailed, msg)
}

pub fn db_query_failed(msg: &str) -> DbError {
    DbError::with_message(DbErrorCode::QueryFailed, msg)
}

pub fn db_unique_violation(field: &str) -> DbError {
    DbError::new(DbErrorCode::UniqueViolation)
        .with_detail(format!("Duplicate value for field: {}", field))
        .with_constraint(field.to_string())
}

pub fn db_not_null_violation(field: &str) -> DbError {
    DbError::new(DbErrorCode::NotNullViolation)
        .with_detail(format!("Field '{}' cannot be null", field))
}

pub fn db_internal_error(msg: &str) -> DbError {
    DbError::with_message(DbErrorCode::InternalError, msg)
}

// ============================================================================
// AUTH ERROR FORMATTING (OAuth 2.0 / JWT RFC Standards)
// ============================================================================

/// Authentication error codes following OAuth 2.0 and JWT RFC standards
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthErrorCode {
    // OAuth 2.0 standard errors (RFC 6749)
    InvalidRequest,
    InvalidClient,
    InvalidGrant,
    UnauthorizedClient,
    UnsupportedGrantType,
    InvalidScope,

    // JWT specific errors (RFC 7519)
    JwtExpired,
    JwtInvalid,
    JwtMalformed,
    JwtSignatureInvalid,

    // Application-level auth errors
    InvalidCredentials,
    EmailAlreadyExists,
    UserNotFound,
    PasswordTooWeak,
    TokenMissing,
    TokenRevoked,
    InsufficientPermissions,

    // Configuration errors
    SecretNotConfigured,
    InternalError,
}

impl AuthErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::InvalidClient => "invalid_client",
            Self::InvalidGrant => "invalid_grant",
            Self::UnauthorizedClient => "unauthorized_client",
            Self::UnsupportedGrantType => "unsupported_grant_type",
            Self::InvalidScope => "invalid_scope",
            Self::JwtExpired => "jwt_expired",
            Self::JwtInvalid => "jwt_invalid",
            Self::JwtMalformed => "jwt_malformed",
            Self::JwtSignatureInvalid => "jwt_signature_invalid",
            Self::InvalidCredentials => "invalid_credentials",
            Self::EmailAlreadyExists => "email_already_exists",
            Self::UserNotFound => "user_not_found",
            Self::PasswordTooWeak => "password_too_weak",
            Self::TokenMissing => "token_missing",
            Self::TokenRevoked => "token_revoked",
            Self::InsufficientPermissions => "insufficient_permissions",
            Self::SecretNotConfigured => "secret_not_configured",
            Self::InternalError => "internal_error",
        }
    }

    pub fn http_status(&self) -> u16 {
        match self {
            Self::InvalidRequest | Self::PasswordTooWeak => 400,
            Self::InvalidCredentials
            | Self::JwtExpired
            | Self::JwtInvalid
            | Self::JwtMalformed
            | Self::JwtSignatureInvalid
            | Self::TokenMissing
            | Self::TokenRevoked
            | Self::InvalidClient
            | Self::InvalidGrant
            | Self::UnauthorizedClient => 401,
            Self::InsufficientPermissions => 403,
            Self::UserNotFound => 404,
            Self::EmailAlreadyExists => 409,
            Self::SecretNotConfigured
            | Self::InternalError
            | Self::UnsupportedGrantType
            | Self::InvalidScope => 500,
        }
    }

    pub fn user_message(&self) -> &'static str {
        match self {
            Self::InvalidRequest => "Invalid request format",
            Self::InvalidClient => "Invalid client credentials",
            Self::InvalidGrant => "Invalid authorization grant",
            Self::UnauthorizedClient => "Client is not authorized",
            Self::UnsupportedGrantType => "Grant type is not supported",
            Self::InvalidScope => "Invalid scope requested",
            Self::JwtExpired => "Your session has expired. Please log in again",
            Self::JwtInvalid => "Invalid authentication token",
            Self::JwtMalformed => "Malformed authentication token",
            Self::JwtSignatureInvalid => "Token signature validation failed",
            Self::InvalidCredentials => "Invalid email or password",
            Self::EmailAlreadyExists => "Email already exists",
            Self::UserNotFound => "User not found",
            Self::PasswordTooWeak => "Password does not meet security requirements",
            Self::TokenMissing => "Authentication token is required",
            Self::TokenRevoked => "Token has been revoked",
            Self::InsufficientPermissions => "Insufficient permissions to access this resource",
            Self::SecretNotConfigured => "Authentication system is not configured",
            Self::InternalError => "Internal authentication error",
        }
    }
}

/// Authentication error structure following OAuth 2.0 and JWT standards
#[derive(Debug, Clone)]
pub struct AuthError {
    pub code: AuthErrorCode,
    pub message: String,
    pub error_description: Option<String>,
}

impl AuthError {
    pub fn new(code: AuthErrorCode) -> Self {
        Self {
            code,
            message: code.user_message().to_string(),
            error_description: None,
        }
    }

    pub fn with_message(code: AuthErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            error_description: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.error_description = Some(description.into());
        self
    }

    /// Convert to JSON string following OAuth 2.0 error response format
    pub fn to_json_string(&self) -> String {
        let mut obj = json!({
            "error": self.code.as_str(),
            "error_description": self.message,
            "status": self.code.http_status(),
        });

        if let Some(desc) = &self.error_description {
            obj["error_description"] = json!(desc);
        }

        obj.to_string()
    }

    /// Convert to RFC 7807 format for HTTP API responses
    pub fn to_rfc7807_string(&self, instance: &str) -> String {
        let error_type = match self.code {
            AuthErrorCode::InvalidCredentials
            | AuthErrorCode::JwtExpired
            | AuthErrorCode::JwtInvalid
            | AuthErrorCode::JwtMalformed
            | AuthErrorCode::JwtSignatureInvalid
            | AuthErrorCode::TokenMissing
            | AuthErrorCode::TokenRevoked => "unauthorized",
            AuthErrorCode::EmailAlreadyExists => "conflict",
            AuthErrorCode::InsufficientPermissions => "forbidden",
            AuthErrorCode::UserNotFound => "not_found",
            AuthErrorCode::InvalidRequest | AuthErrorCode::PasswordTooWeak => "bad_request",
            _ => "internal_error",
        };

        json!({
            "type": format!("https://doo.dev/errors/{}", error_type),
            "title": match self.code.http_status() {
                400 => "Bad Request",
                401 => "Unauthorized",
                403 => "Forbidden",
                404 => "Not Found",
                409 => "Conflict",
                _ => "Internal Server Error",
            },
            "status": self.code.http_status(),
            "detail": self.message,
            "instance": instance,
            "error_code": self.code.as_str(),
        })
        .to_string()
    }
}

/// Helper functions for common auth errors
pub fn auth_invalid_credentials() -> AuthError {
    AuthError::new(AuthErrorCode::InvalidCredentials)
}

pub fn auth_email_exists() -> AuthError {
    AuthError::new(AuthErrorCode::EmailAlreadyExists)
}

pub fn auth_jwt_expired() -> AuthError {
    AuthError::new(AuthErrorCode::JwtExpired)
}

pub fn auth_jwt_invalid() -> AuthError {
    AuthError::new(AuthErrorCode::JwtInvalid)
}

pub fn auth_jwt_malformed() -> AuthError {
    AuthError::new(AuthErrorCode::JwtMalformed)
}

pub fn auth_token_missing() -> AuthError {
    AuthError::new(AuthErrorCode::TokenMissing)
}

pub fn auth_secret_not_configured() -> AuthError {
    AuthError::new(AuthErrorCode::SecretNotConfigured)
}

pub fn auth_insufficient_permissions() -> AuthError {
    AuthError::new(AuthErrorCode::InsufficientPermissions)
}

pub fn auth_internal_error(msg: &str) -> AuthError {
    AuthError::with_message(AuthErrorCode::InternalError, msg)
}

// ============================================================================
// FFI EXPORTS FOR ERROR FORMATTING
// ============================================================================

/// Create a database error JSON string
#[no_mangle]
pub extern "C" fn dooruntime_db_error(
    code_str: *const c_char,
    message: *const c_char,
) -> *mut c_char {
    if code_str.is_null() || message.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let code_s = CStr::from_ptr(code_str).to_string_lossy();
        let msg = CStr::from_ptr(message).to_string_lossy();

        let code = match code_s.as_ref() {
            "CONNECTION_FAILED" => DbErrorCode::ConnectionFailed,
            "UNIQUE_VIOLATION" => DbErrorCode::UniqueViolation,
            "QUERY_FAILED" => DbErrorCode::QueryFailed,
            _ => DbErrorCode::InternalError,
        };

        let error = DbError::with_message(code, msg.to_string());
        let json = error.to_json_string();

        CString::new(json)
            .map(|c| c.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }
}

/// Create an auth error JSON string
#[no_mangle]
pub extern "C" fn dooruntime_auth_error(
    code_str: *const c_char,
    message: *const c_char,
) -> *mut c_char {
    if code_str.is_null() || message.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let code_s = CStr::from_ptr(code_str).to_string_lossy();
        let msg = CStr::from_ptr(message).to_string_lossy();

        let code = match code_s.as_ref() {
            "invalid_credentials" => AuthErrorCode::InvalidCredentials,
            "jwt_expired" => AuthErrorCode::JwtExpired,
            "jwt_invalid" => AuthErrorCode::JwtInvalid,
            "email_already_exists" => AuthErrorCode::EmailAlreadyExists,
            _ => AuthErrorCode::InternalError,
        };

        let error = AuthError::with_message(code, msg.to_string());
        let json = error.to_json_string();

        CString::new(json)
            .map(|c| c.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }
}

/// Create an auth error in RFC 7807 format
#[no_mangle]
pub extern "C" fn dooruntime_auth_error_rfc7807(
    code_str: *const c_char,
    message: *const c_char,
    instance: *const c_char,
) -> *mut c_char {
    if code_str.is_null() || message.is_null() || instance.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let code_s = CStr::from_ptr(code_str).to_string_lossy();
        let msg = CStr::from_ptr(message).to_string_lossy();
        let inst = CStr::from_ptr(instance).to_string_lossy();

        let code = match code_s.as_ref() {
            "invalid_credentials" => AuthErrorCode::InvalidCredentials,
            "jwt_expired" => AuthErrorCode::JwtExpired,
            "jwt_invalid" => AuthErrorCode::JwtInvalid,
            "email_already_exists" => AuthErrorCode::EmailAlreadyExists,
            _ => AuthErrorCode::InternalError,
        };

        let error = AuthError::with_message(code, msg.to_string());
        let json = error.to_rfc7807_string(&inst);

        CString::new(json)
            .map(|c| c.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }
}

// ============================================================================
// DB DECORATOR HELPERS
// ============================================================================

/// Check if a field has @unique decorator
#[no_mangle]
pub extern "C" fn dooruntime_has_unique_decorator(decorators_json: *const c_char) -> i32 {
    if decorators_json.is_null() {
        return 0;
    }

    unsafe {
        let json_str = match CStr::from_ptr(decorators_json).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        };

        let decorators: Vec<FieldDecorator> = match serde_json::from_str(json_str) {
            Ok(d) => d,
            Err(_) => return 0,
        };

        for dec in decorators {
            if dec.name == "unique" {
                return 1;
            }
        }
        0
    }
}

/// Check if a field has @primary decorator
#[no_mangle]
pub extern "C" fn dooruntime_has_primary_decorator(decorators_json: *const c_char) -> i32 {
    if decorators_json.is_null() {
        return 0;
    }

    unsafe {
        let json_str = match CStr::from_ptr(decorators_json).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        };

        let decorators: Vec<FieldDecorator> = match serde_json::from_str(json_str) {
            Ok(d) => d,
            Err(_) => return 0,
        };

        for dec in decorators {
            if dec.name == "primary" {
                return 1;
            }
        }
        0
    }
}

/// Check if a field has @auto decorator
#[no_mangle]
pub extern "C" fn dooruntime_has_auto_decorator(decorators_json: *const c_char) -> i32 {
    if decorators_json.is_null() {
        return 0;
    }

    unsafe {
        let json_str = match CStr::from_ptr(decorators_json).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        };

        let decorators: Vec<FieldDecorator> = match serde_json::from_str(json_str) {
            Ok(d) => d,
            Err(_) => return 0,
        };

        for dec in decorators {
            if dec.name == "auto" {
                return 1;
            }
        }
        0
    }
}

/// Check if a field has @hash decorator
#[no_mangle]
pub extern "C" fn dooruntime_has_hash_decorator(decorators_json: *const c_char) -> i32 {
    if decorators_json.is_null() {
        return 0;
    }

    unsafe {
        let json_str = match CStr::from_ptr(decorators_json).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        };

        let decorators: Vec<FieldDecorator> = match serde_json::from_str(json_str) {
            Ok(d) => d,
            Err(_) => return 0,
        };

        for dec in decorators {
            if dec.name == "hash" {
                return 1;
            }
        }
        0
    }
}

/// Extract all unique field names from metadata
#[no_mangle]
pub extern "C" fn dooruntime_extract_unique_fields(metadata_json: *const c_char) -> *mut c_char {
    if metadata_json.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let json_str = match CStr::from_ptr(metadata_json).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        let metadata: serde_json::Value = match serde_json::from_str(json_str) {
            Ok(m) => m,
            Err(_) => return std::ptr::null_mut(),
        };

        let mut unique_fields = Vec::new();

        if let Some(fields) = metadata.get("fields").and_then(|f| f.as_array()) {
            for field in fields {
                let field_name = field.get("name").and_then(|n| n.as_str()).unwrap_or("");
                if let Some(decorators) = field.get("decorators").and_then(|d| d.as_array()) {
                    for dec in decorators {
                        if let Some(dec_name) = dec.get("name").and_then(|n| n.as_str()) {
                            if dec_name == "unique" {
                                unique_fields.push(field_name);
                                break;
                            }
                        }
                    }
                }
            }
        }

        let result_json = json!(unique_fields).to_string();
        CString::new(result_json)
            .map(|c| c.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }
}

/// Create a DB error in RFC 7807 format for HTTP responses
#[no_mangle]
pub extern "C" fn dooruntime_db_error_rfc7807(
    code_str: *const c_char,
    message: *const c_char,
    instance: *const c_char,
    field: *const c_char,
) -> *mut c_char {
    if code_str.is_null() || message.is_null() || instance.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let code_s = CStr::from_ptr(code_str).to_string_lossy();
        let msg = CStr::from_ptr(message).to_string_lossy();
        let inst = CStr::from_ptr(instance).to_string_lossy();
        let field_name = if field.is_null() {
            None
        } else {
            Some(CStr::from_ptr(field).to_string_lossy().to_string())
        };

        let code = match code_s.as_ref() {
            "CONNECTION_FAILED" => DbErrorCode::ConnectionFailed,
            "UNIQUE_VIOLATION" => DbErrorCode::UniqueViolation,
            "QUERY_FAILED" => DbErrorCode::QueryFailed,
            "NOT_NULL_VIOLATION" => DbErrorCode::NotNullViolation,
            "TABLE_NOT_FOUND" => DbErrorCode::TableNotFound,
            _ => DbErrorCode::InternalError,
        };

        let mut error = DbError::with_message(code, msg.to_string());
        if let Some(f) = field_name {
            error = error.with_constraint(f);
        }

        let error_type = match code {
            DbErrorCode::UniqueViolation => "conflict",
            DbErrorCode::NotNullViolation => "bad_request",
            DbErrorCode::TableNotFound => "not_found",
            _ => "internal_error",
        };

        let json = json!({
            "type": format!("https://doo.dev/errors/db/{}", error_type),
            "title": match code.http_status() {
                400 => "Bad Request",
                404 => "Not Found",
                409 => "Conflict",
                _ => "Internal Server Error",
            },
            "status": code.http_status(),
            "detail": error.message,
            "instance": inst.to_string(),
            "error_code": code.as_str(),
            "pg_code": code.pg_code(),
        })
        .to_string();

        CString::new(json)
            .map(|c| c.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }
}
