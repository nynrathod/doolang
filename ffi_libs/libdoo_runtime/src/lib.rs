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

/// Thread-local storage for JSON type mismatch errors during struct deserialization
thread_local! {
    static JSON_TYPE_MISMATCH: RefCell<Option<(String, String, String)>> = RefCell::new(None);
}

fn set_json_type_mismatch(field_name: String, expected_type: String, actual_type: String) {
    JSON_TYPE_MISMATCH.with(|cell| {
        *cell.borrow_mut() = Some((field_name, expected_type, actual_type));
    });
}

fn clear_json_type_mismatch() {
    JSON_TYPE_MISMATCH.with(|cell| {
        *cell.borrow_mut() = None;
    });
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

/// Allocate memory using the runtime's allocator (libc::malloc)
/// This ensures compatibility between FFI libraries and JIT-compiled code
#[no_mangle]
pub extern "C" fn dooruntime_malloc(size: libc::size_t) -> *mut u8 {
    unsafe { libc::malloc(size) as *mut u8 }
}

/// Free memory using the runtime's allocator (libc::free)
#[no_mangle]
pub extern "C" fn dooruntime_free(ptr: *mut u8) {
    if !ptr.is_null() {
        unsafe { libc::free(ptr as *mut libc::c_void) }
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
            "default" => {
                // @default is handled during value extraction, not validation
                // If value is empty, the default will be applied
                // This is a no-op here; default extraction happens separately
            }
            _ => {
                // Unknown decorator - ignore
            }
        }
    }
    Ok(())
}

/// Extract default value for a field based on @default decorator
/// Returns empty string if no default is found
pub extern "C" fn dooruntime_get_default_value(
    decorators_json: *const std::os::raw::c_char,
    field_type: *const std::os::raw::c_char,
) -> *mut std::os::raw::c_char {
    if decorators_json.is_null() || field_type.is_null() {
        return std::ptr::null_mut();
    }

    let decorators_str = unsafe {
        match std::ffi::CStr::from_ptr(decorators_json).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        }
    };

    let field_type_str = unsafe {
        match std::ffi::CStr::from_ptr(field_type).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        }
    };

    let decorators: Vec<FieldDecorator> = match serde_json::from_str(decorators_str) {
        Ok(d) => d,
        Err(_) => return std::ptr::null_mut(),
    };

    for decorator in decorators {
        if decorator.name == "default" {
            if let Some(default_value) = decorator.args.first() {
                // Validate that default value matches the field type
                let validated_value = match field_type_str {
                    "Str" => default_value.clone(),
                    "Int" => {
                        // Verify it's a valid integer
                        if default_value.parse::<i64>().is_ok() {
                            default_value.clone()
                        } else {
                            return std::ptr::null_mut();
                        }
                    }
                    "Bool" => {
                        // Verify it's a valid boolean
                        let lower = default_value.to_lowercase();
                        if lower == "true" || lower == "false" {
                            lower
                        } else {
                            return std::ptr::null_mut();
                        }
                    }
                    "Float" => {
                        // Verify it's a valid float
                        if default_value.parse::<f64>().is_ok() {
                            default_value.clone()
                        } else {
                            return std::ptr::null_mut();
                        }
                    }
                    _ => return std::ptr::null_mut(),
                };

                return std::ffi::CString::new(validated_value)
                    .ok()
                    .map(|s| s.into_raw())
                    .unwrap_or(std::ptr::null_mut());
            }
        }
    }

    std::ptr::null_mut()
}

/// Check if a field has @default decorator
pub extern "C" fn dooruntime_has_default_decorator(
    decorators_json: *const std::os::raw::c_char,
) -> bool {
    if decorators_json.is_null() {
        return false;
    }

    let decorators_str = unsafe {
        match std::ffi::CStr::from_ptr(decorators_json).to_str() {
            Ok(s) => s,
            Err(_) => return false,
        }
    };

    let decorators: Vec<FieldDecorator> = match serde_json::from_str(decorators_str) {
        Ok(d) => d,
        Err(_) => return false,
    };

    decorators.iter().any(|d| d.name == "default")
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

/// Validate that a JSON string value matches the expected type
/// Returns 1 if valid, 0 if invalid
/// For Int: checks if value is a number in JSON (not a string)
/// For Float: checks if value is a number in JSON
/// For Bool: checks if value is true/false in JSON
/// For Str: checks if value is a string in JSON
#[no_mangle]
pub extern "C" fn dooruntime_validate_json_type(
    json_value: *const libc::c_char,
    expected_type: *const libc::c_char,
) -> i32 {
    if json_value.is_null() || expected_type.is_null() {
        return 0;
    }

    let json_str = unsafe {
        match CStr::from_ptr(json_value).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        }
    };

    let type_str = unsafe {
        match CStr::from_ptr(expected_type).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        }
    };

    // Parse the JSON value
    let json_val: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return 0,
    };

    // Check if the JSON value matches the expected type
    match type_str {
        "Int" => {
            if json_val.is_i64() || json_val.is_u64() {
                1
            } else {
                0
            }
        }
        "Float" => {
            if json_val.is_f64() || json_val.is_i64() || json_val.is_u64() {
                1
            } else {
                0
            }
        }
        "Bool" => {
            if json_val.is_boolean() {
                1
            } else {
                0
            }
        }
        "Str" => {
            if json_val.is_string() {
                1
            } else {
                0
            }
        }
        _ => 1, // Unknown type, allow it
    }
}

/// Get the actual JSON type of a value as a string
/// Returns: "number", "string", "boolean", "null", "array", "object"
#[no_mangle]
pub extern "C" fn dooruntime_get_json_type(json_value: *const libc::c_char) -> *mut libc::c_char {
    if json_value.is_null() {
        return std::ptr::null_mut();
    }

    let json_str = unsafe {
        match CStr::from_ptr(json_value).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        }
    };

    let json_val: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return std::ptr::null_mut(),
    };

    let type_name = match json_val {
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Null => "null",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    };

    CString::new(type_name)
        .map(|c| c.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// Extract a field value from JSON object and return as Int
/// Returns the integer value, or 0 if field not found or type mismatch
/// This validates that the JSON value is actually a number, not a string
#[no_mangle]
pub extern "C" fn json_get_int(json: *const libc::c_char, field_name: *const libc::c_char) -> i32 {
    if json.is_null() || field_name.is_null() {
        return 0;
    }

    let json_str = unsafe {
        match CStr::from_ptr(json).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        }
    };

    let field_str = unsafe {
        match CStr::from_ptr(field_name).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        }
    };

    // Parse JSON
    let json_obj: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return 0,
    };

    // Extract field
    if let Some(field_value) = json_obj.get(field_str) {
        // Validate it's a number
        if let Some(num) = field_value.as_i64() {
            return num as i32;
        } else if let Some(num) = field_value.as_u64() {
            return num as i32;
        } else {
            // Type mismatch - set error
            let actual_type = if field_value.is_string() {
                "string"
            } else if field_value.is_boolean() {
                "boolean"
            } else if field_value.is_null() {
                "null"
            } else if field_value.is_array() {
                "array"
            } else if field_value.is_object() {
                "object"
            } else {
                "unknown"
            };
            set_json_type_mismatch(
                field_str.to_string(),
                "Int".to_string(),
                actual_type.to_string(),
            );
        }
    }

    0 // Field not found or wrong type
}

/// Extract a field value from JSON object and return as Float
/// Returns the float value, or 0.0 if field not found or type mismatch
#[no_mangle]
pub extern "C" fn json_get_float(
    json: *const libc::c_char,
    field_name: *const libc::c_char,
) -> f64 {
    if json.is_null() || field_name.is_null() {
        return 0.0;
    }

    let json_str = unsafe {
        match CStr::from_ptr(json).to_str() {
            Ok(s) => s,
            Err(_) => return 0.0,
        }
    };

    let field_str = unsafe {
        match CStr::from_ptr(field_name).to_str() {
            Ok(s) => s,
            Err(_) => return 0.0,
        }
    };

    // Parse JSON
    let json_obj: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return 0.0,
    };

    // Extract field
    if let Some(field_value) = json_obj.get(field_str) {
        // Validate it's a number
        if let Some(num) = field_value.as_f64() {
            return num;
        } else if let Some(num) = field_value.as_i64() {
            return num as f64;
        } else if let Some(num) = field_value.as_u64() {
            return num as f64;
        } else {
            // Type mismatch - set error
            let actual_type = if field_value.is_string() {
                "string"
            } else if field_value.is_boolean() {
                "boolean"
            } else if field_value.is_null() {
                "null"
            } else if field_value.is_array() {
                "array"
            } else if field_value.is_object() {
                "object"
            } else {
                "unknown"
            };
            set_json_type_mismatch(
                field_str.to_string(),
                "Float".to_string(),
                actual_type.to_string(),
            );
        }
    }

    0.0 // Field not found or wrong type
}

/// Extract a field value from JSON object and return as Bool
/// Returns 1 for true, 0 for false or not found
#[no_mangle]
pub extern "C" fn json_get_bool(json: *const libc::c_char, field_name: *const libc::c_char) -> i32 {
    if json.is_null() || field_name.is_null() {
        return 0;
    }

    let json_str = unsafe {
        match CStr::from_ptr(json).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        }
    };

    let field_str = unsafe {
        match CStr::from_ptr(field_name).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        }
    };

    // Parse JSON
    let json_obj: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return 0,
    };

    // Extract field
    if let Some(field_value) = json_obj.get(field_str) {
        // Validate it's a boolean
        if let Some(b) = field_value.as_bool() {
            return if b { 1 } else { 0 };
        } else {
            // Type mismatch - set error
            let actual_type = if field_value.is_string() {
                "string"
            } else if field_value.is_number() {
                "number"
            } else if field_value.is_null() {
                "null"
            } else if field_value.is_array() {
                "array"
            } else if field_value.is_object() {
                "object"
            } else {
                "unknown"
            };
            set_json_type_mismatch(
                field_str.to_string(),
                "Bool".to_string(),
                actual_type.to_string(),
            );
        }
    }

    0 // Field not found or wrong type
}

/// Check if there was a JSON type mismatch error
/// Returns JSON string with error details, or null if no error
#[no_mangle]
pub extern "C" fn dooruntime_get_json_type_mismatch() -> *mut libc::c_char {
    JSON_TYPE_MISMATCH.with(|cell| {
        if let Some((field_name, expected_type, actual_type)) = cell.borrow().as_ref() {
            let error = json!({
                "field": field_name,
                "expected": expected_type,
                "actual": actual_type,
                "message": format!("Field '{}' expected type '{}' but got '{}'", field_name, expected_type, actual_type)
            });

            if let Ok(json_str) = serde_json::to_string(&error) {
                return CString::new(json_str)
                    .map(|c| c.into_raw())
                    .unwrap_or(std::ptr::null_mut());
            }
        }
        std::ptr::null_mut()
    })
}

/// Clear JSON type mismatch error
#[no_mangle]
pub extern "C" fn dooruntime_clear_json_type_mismatch() {
    clear_json_type_mismatch();
}

/// Extract a field value from JSON object and return as String
/// Returns pointer to string (caller must free), or NULL if not found
#[no_mangle]
pub extern "C" fn json_get_str(
    json: *const libc::c_char,
    field_name: *const libc::c_char,
) -> *mut libc::c_char {
    if json.is_null() || field_name.is_null() {
        return std::ptr::null_mut();
    }

    let json_str = unsafe {
        match CStr::from_ptr(json).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        }
    };

    let field_str = unsafe {
        match CStr::from_ptr(field_name).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        }
    };

    // Parse JSON
    let json_obj: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return std::ptr::null_mut(),
    };

    // Extract field
    if let Some(field_value) = json_obj.get(field_str) {
        // Validate it's a string
        if let Some(s) = field_value.as_str() {
            return CString::new(s)
                .map(|c| c.into_raw())
                .unwrap_or(std::ptr::null_mut());
        } else {
            // Type mismatch - set error
            let actual_type = if field_value.is_number() {
                "number"
            } else if field_value.is_boolean() {
                "boolean"
            } else if field_value.is_null() {
                "null"
            } else if field_value.is_array() {
                "array"
            } else if field_value.is_object() {
                "object"
            } else {
                "unknown"
            };
            set_json_type_mismatch(
                field_str.to_string(),
                "Str".to_string(),
                actual_type.to_string(),
            );
        }
    }

    std::ptr::null_mut() // Field not found or wrong type
}

/// Validate that a JSON field exists and has the correct type
/// Returns 1 if valid, 0 if invalid or missing
#[no_mangle]
pub extern "C" fn json_validate_field_type(
    json: *const libc::c_char,
    field_name: *const libc::c_char,
    expected_type: *const libc::c_char,
) -> i32 {
    if json.is_null() || field_name.is_null() || expected_type.is_null() {
        return 0;
    }

    let json_str = unsafe {
        match CStr::from_ptr(json).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        }
    };

    let field_str = unsafe {
        match CStr::from_ptr(field_name).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        }
    };

    let type_str = unsafe {
        match CStr::from_ptr(expected_type).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        }
    };

    // Parse JSON
    let json_obj: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return 0,
    };

    // Extract field
    if let Some(field_value) = json_obj.get(field_str) {
        // Check type
        match type_str {
            "Int" | "number" => {
                if field_value.is_i64() || field_value.is_u64() {
                    return 1;
                }
            }
            "Float" => {
                if field_value.is_f64() || field_value.is_i64() || field_value.is_u64() {
                    return 1;
                }
            }
            "Bool" | "boolean" => {
                if field_value.is_boolean() {
                    return 1;
                }
            }
            "Str" | "string" => {
                if field_value.is_string() {
                    return 1;
                }
            }
            _ => return 1, // Unknown type, allow it
        }
    }

    0 // Field not found or wrong type
}

// ============================================================================
// JSON VALIDATION FOR HTTP REQUEST BODIES (RFC 7807 Enhanced)
// ============================================================================

/// Check for missing required fields in JSON body
/// Returns JSON string with missing field errors, or null if all fields present
/// Format: {"fields": {"field_name": {"error": "required", "message": "This field is required"}}}
#[no_mangle]
pub extern "C" fn dooruntime_check_missing_fields(
    json: *const c_char,
    required_fields_json: *const c_char, // JSON array of required field names
) -> *mut c_char {
    if json.is_null() || required_fields_json.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let json_str = match CStr::from_ptr(json).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        let required_str = match CStr::from_ptr(required_fields_json).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        // Parse JSON body
        let json_obj: serde_json::Value = match serde_json::from_str(json_str) {
            Ok(v) => v,
            Err(_) => return std::ptr::null_mut(), // Malformed JSON - will be caught elsewhere
        };

        // Parse required fields
        let required_fields: Vec<String> = match serde_json::from_str(required_str) {
            Ok(v) => v,
            Err(_) => return std::ptr::null_mut(),
        };

        let mut missing_fields = serde_json::Map::new();

        // Check each required field
        for field_name in required_fields {
            if let Some(obj) = json_obj.as_object() {
                if !obj.contains_key(&field_name) {
                    let mut field_error = serde_json::Map::new();
                    field_error.insert("error".to_string(), json!("required"));
                    field_error.insert(
                        "message".to_string(),
                        json!("This field is required"),
                    );
                    missing_fields.insert(field_name, json!(field_error));
                }
            }
        }

        // If any fields missing, return error JSON
        if !missing_fields.is_empty() {
            let error_obj = json!({
                "fields": missing_fields
            });

            CString::new(error_obj.to_string())
                .map(|c| c.into_raw())
                .unwrap_or(std::ptr::null_mut())
        } else {
            std::ptr::null_mut() // All fields present
        }
    }
}

/// Helper function to validate a JSON value against an expected type
/// Returns (is_valid, error_detail) where error_detail describes the mismatch
fn validate_type(expected_type: &str, value: &serde_json::Value) -> (bool, Option<String>) {
    match expected_type {
        "Int" => {
            if value.is_i64() || value.is_u64() {
                (true, None)
            } else {
                (false, Some(format!("expected Int, got {}", json_type_name(value))))
            }
        }
        "Float" => {
            if value.is_f64() || value.is_i64() || value.is_u64() {
                (true, None)
            } else {
                (false, Some(format!("expected Float, got {}", json_type_name(value))))
            }
        }
        "Bool" => {
            if value.is_boolean() {
                (true, None)
            } else {
                (false, Some(format!("expected Bool, got {}", json_type_name(value))))
            }
        }
        "Str" => {
            if value.is_string() {
                (true, None)
            } else {
                (false, Some(format!("expected Str, got {}", json_type_name(value))))
            }
        }
        // Array types: [Str], [Int], [Float], [Bool]
        ty if ty.starts_with('[') && ty.ends_with(']') => {
            let inner_type = &ty[1..ty.len()-1];
            if let Some(arr) = value.as_array() {
                for (idx, elem) in arr.iter().enumerate() {
                    let (elem_valid, elem_error) = validate_type(inner_type, elem);
                    if !elem_valid {
                        return (false, Some(format!(
                            "array element at index {} has wrong type: {}",
                            idx, elem_error.unwrap_or_default()
                        )));
                    }
                }
                (true, None)
            } else {
                (false, Some(format!("expected array, got {}", json_type_name(value))))
            }
        }
        // Map types: {Str: Int}, {Str: Str}, etc.
        ty if ty.starts_with('{') && ty.ends_with('}') && ty.contains(':') => {
            let inner = &ty[1..ty.len()-1];
            if let Some(colon_pos) = inner.find(':') {
                let key_type = inner[..colon_pos].trim();
                let value_type = inner[colon_pos+1..].trim();
                
                if let Some(obj) = value.as_object() {
                    // Validate keys (should always be strings in JSON)
                    if key_type != "Str" {
                        // JSON only supports string keys, so non-Str key types can't be validated
                        return (true, None);
                    }
                    
                    // Validate values
                    for (k, v) in obj {
                        let (val_valid, val_error) = validate_type(value_type, v);
                        if !val_valid {
                            return (false, Some(format!(
                                "map value for key '{}' has wrong type: {}",
                                k, val_error.unwrap_or_default()
                            )));
                        }
                    }
                    (true, None)
                } else {
                    (false, Some(format!("expected object, got {}", json_type_name(value))))
                }
            } else {
                (true, None) // Can't parse map type, skip validation
            }
        }
        // Enum types and unknown - allow them through (enums are validated elsewhere)
        _ => (true, None),
    }
}

/// Get the JSON type name for error messages
fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "Bool",
        serde_json::Value::Number(n) => {
            if n.is_f64() && n.as_i64().is_none() { "Float" } else { "Int" }
        },
        serde_json::Value::String(_) => "Str",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Validate all field types in JSON body against expected types
/// Returns JSON string with type mismatch errors, or null if all types correct

/// field_types_json format: {"field_name": "Int", "email": "Str", ...}
/// Returns: {"fields": {"field": {"expected": "Int", "received": "String", "value": "invalid"}}}
#[no_mangle]
pub extern "C" fn dooruntime_validate_field_types(
    json: *const c_char,
    field_types_json: *const c_char,
) -> *mut c_char {
    if json.is_null() || field_types_json.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let json_str = match CStr::from_ptr(json).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        let types_str = match CStr::from_ptr(field_types_json).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        // Parse JSON body
        let json_obj: serde_json::Value = match serde_json::from_str(json_str) {
            Ok(v) => v,
            Err(_) => return std::ptr::null_mut(),
        };

        // Parse expected types
        let field_types: serde_json::Map<String, serde_json::Value> =
            match serde_json::from_str(types_str) {
                Ok(v) => v,
                Err(_) => return std::ptr::null_mut(),
            };

        let mut type_errors = serde_json::Map::new();

        // Check each field type
        if let Some(obj) = json_obj.as_object() {
            for (field_name, expected_type_val) in &field_types {
                if let Some(expected_type) = expected_type_val.as_str() {
                    if let Some(field_value) = obj.get(field_name) {
                        let (is_valid, error_detail) = validate_type(expected_type, field_value);

                        if !is_valid {
                            let value_str = field_value.to_string();
                            let mut field_error = serde_json::Map::new();
                            field_error.insert("expected".to_string(), json!(expected_type));
                            field_error.insert("received".to_string(), json!(json_type_name(field_value)));
                            field_error.insert("value".to_string(), json!(value_str));
                            if let Some(detail) = error_detail {
                                field_error.insert("detail".to_string(), json!(detail));
                            }
                            type_errors.insert(field_name.clone(), json!(field_error));
                        }

                    }
                }
            }
        }

        // If any type errors, return error JSON
        if !type_errors.is_empty() {
            let error_obj = json!({
                "fields": type_errors
            });

            CString::new(error_obj.to_string())
                .map(|c| c.into_raw())
                .unwrap_or(std::ptr::null_mut())
        } else {
            std::ptr::null_mut() // All types correct
        }
    }
}

/// Comprehensive validation: checks missing fields, type mismatches, and decorators
/// Returns RFC 7807 formatted error JSON with all validation errors, or null if valid
/// This combines missing field check, type validation, and decorator validation in one call
#[no_mangle]
pub extern "C" fn dooruntime_validate_json_body(
    json: *const c_char,
    struct_metadata_json: *const c_char, // Contains required_fields, field_types, and decorators
) -> *mut c_char {
    if json.is_null() || struct_metadata_json.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let json_str = match CStr::from_ptr(json).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        let metadata_str = match CStr::from_ptr(struct_metadata_json).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        // Parse JSON body
        let json_obj: serde_json::Value = match serde_json::from_str(json_str) {
            Ok(v) => v,
            Err(_) => return std::ptr::null_mut(),
        };

        // Parse metadata
        let metadata: serde_json::Value = match serde_json::from_str(metadata_str) {
            Ok(v) => v,
            Err(_) => return std::ptr::null_mut(),
        };

        let mut all_errors = serde_json::Map::new();
        let mut error_detail = String::from("Validation failed");

        // 1. Check missing required fields
        if let Some(required_fields) = metadata.get("required_fields").and_then(|v| v.as_array()) {
            if let Some(obj) = json_obj.as_object() {
                for field_val in required_fields {
                    if let Some(field_name) = field_val.as_str() {
                        if !obj.contains_key(field_name) {
                            let mut field_error = serde_json::Map::new();
                            field_error.insert("error".to_string(), json!("required"));
                            field_error.insert(
                                "message".to_string(),
                                json!("This field is required"),
                            );
                            all_errors.insert(field_name.to_string(), json!(field_error));
                            error_detail = "Required field missing in request body".to_string();
                        }
                    }
                }
            }
        }

        // 2. Check type mismatches
        if let Some(field_types) = metadata.get("field_types").and_then(|v| v.as_object()) {
            if let Some(obj) = json_obj.as_object() {
                for (field_name, expected_type_val) in field_types {
                    if let Some(expected_type) = expected_type_val.as_str() {
                        if let Some(field_value) = obj.get(field_name) {
                            let is_valid = match expected_type {
                                "Int" => field_value.is_i64() || field_value.is_u64(),
                                "Float" => {
                                    field_value.is_f64()
                                        || field_value.is_i64()
                                        || field_value.is_u64()
                                }
                                "Bool" => field_value.is_boolean(),
                                "Str" => field_value.is_string(),
                                _ => true,
                            };

                            if !is_valid {
                                let actual_type = if field_value.is_string() {
                                    "String"
                                } else if field_value.is_i64() || field_value.is_u64() {
                                    "Int"
                                } else if field_value.is_f64() {
                                    "Float"
                                } else if field_value.is_boolean() {
                                    "Bool"
                                } else {
                                    "Unknown"
                                };

                                let mut field_error = serde_json::Map::new();
                                field_error.insert("expected".to_string(), json!(expected_type));
                                field_error.insert("received".to_string(), json!(actual_type));
                                field_error
                                    .insert("value".to_string(), json!(field_value.to_string()));
                                all_errors.insert(field_name.clone(), json!(field_error));
                                error_detail = "Type mismatch in request body".to_string();
                            }
                        }
                    }
                }
            }
        }

        // If any errors found, return RFC 7807 formatted error
        if !all_errors.is_empty() {
            let error_obj = json!({
                "fields": all_errors,
                "detail": error_detail
            });

            CString::new(error_obj.to_string())
                .map(|c| c.into_raw())
                .unwrap_or(std::ptr::null_mut())
        } else {
            std::ptr::null_mut() // All validation passed
        }
    }
}

// ============================================================================
// JSON SCALAR EXTRACTION FOR DB RESULTS
// ============================================================================
// These functions extract the first scalar value from SQL result sets
// COUNT queries return: [{"count": 5}] or [{"COUNT(*)": 5}] or just a number
// We extract the first value from the first row

/// Extract the first integer value from a JSON result set
/// Handles formats like: [{"count": 5}], [{"COUNT(*)": 5}], 5, "5"
#[no_mangle]
pub extern "C" fn json_extract_scalar_v2(json: *const libc::c_char) -> i32 {
    if json.is_null() {
        return 0;
    }

    let json_str = unsafe {
        match CStr::from_ptr(json).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        }
    };

    let json_val: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => {
            // Try parsing as simple integer string if JSON fails
            return json_str.trim().parse::<i32>().unwrap_or(0);
        }
    };

    extract_first_int_from_value(&json_val)
}

fn extract_first_int_from_value(json_val: &serde_json::Value) -> i32 {
    // If it's a plain number, return it
    if let Some(n) = json_val.as_i64() {
        return n as i32;
    }
    if let Some(n) = json_val.as_u64() {
        return n as i32;
    }

    // If it's an array, get first element
    if let Some(arr) = json_val.as_array() {
        if let Some(first) = arr.first() {
            return extract_first_int_from_value(first);
        }
    }

    // If it's an object, get first field value
    if let Some(obj) = json_val.as_object() {
        // Try common COUNT field names first
        for key in ["count", "COUNT(*)", "COUNT", "sum", "SUM", "avg", "AVG"] {
            if let Some(val) = obj.get(key) {
                if let Some(n) = val.as_i64() {
                    return n as i32;
                }
                if let Some(n) = val.as_u64() {
                    return n as i32;
                }
            }
        }
        // Try first field
        if let Some((_, val)) = obj.iter().next() {
            if let Some(n) = val.as_i64() {
                return n as i32;
            }
            if let Some(n) = val.as_u64() {
                return n as i32;
            }
        }
    }

    0
}

/// Extract the first float value from a JSON result set
#[no_mangle]
pub extern "C" fn json_extract_first_float(json: *const libc::c_char) -> f64 {
    if json.is_null() {
        return 0.0;
    }

    let json_str = unsafe {
        match CStr::from_ptr(json).to_str() {
            Ok(s) => s,
            Err(_) => return 0.0,
        }
    };

    let json_val: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => {
            return json_str.trim().parse::<f64>().unwrap_or(0.0);
        }
    };

    extract_first_float_from_value(&json_val)
}

fn extract_first_float_from_value(json_val: &serde_json::Value) -> f64 {
    if let Some(n) = json_val.as_f64() {
        return n;
    }
    if let Some(n) = json_val.as_i64() {
        return n as f64;
    }
    if let Some(n) = json_val.as_u64() {
        return n as f64;
    }

    if let Some(arr) = json_val.as_array() {
        if let Some(first) = arr.first() {
            return extract_first_float_from_value(first);
        }
    }

    if let Some(obj) = json_val.as_object() {
        if let Some((_, val)) = obj.iter().next() {
            if let Some(n) = val.as_f64() {
                return n;
            }
            if let Some(n) = val.as_i64() {
                return n as f64;
            }
        }
    }

    0.0
}

/// Extract the first boolean value from a JSON result set
#[no_mangle]
pub extern "C" fn json_extract_first_bool(json: *const libc::c_char) -> i32 {
    if json.is_null() {
        return 0;
    }

    let json_str = unsafe {
        match CStr::from_ptr(json).to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        }
    };

    let json_val: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => {
            return if json_str.trim() == "true" { 1 } else { 0 };
        }
    };

    extract_first_bool_from_value(&json_val)
}

fn extract_first_bool_from_value(json_val: &serde_json::Value) -> i32 {
    if let Some(b) = json_val.as_bool() {
        return if b { 1 } else { 0 };
    }

    if let Some(arr) = json_val.as_array() {
        if let Some(first) = arr.first() {
            return extract_first_bool_from_value(first);
        }
    }

    if let Some(obj) = json_val.as_object() {
        if let Some((_, val)) = obj.iter().next() {
            if let Some(b) = val.as_bool() {
                return if b { 1 } else { 0 };
            }
        }
    }

    0
}

/// Extract the first string value from a JSON result set
#[no_mangle]
pub extern "C" fn json_extract_first_str(json: *const libc::c_char) -> *mut libc::c_char {
    if json.is_null() {
        return std::ptr::null_mut();
    }

    let json_str = unsafe {
        match CStr::from_ptr(json).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        }
    };

    let json_val: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => {
            // Return the raw string
            return CString::new(json_str)
                .map(|c| c.into_raw())
                .unwrap_or(std::ptr::null_mut());
        }
    };

    extract_first_str_from_value(&json_val)
}

fn extract_first_str_from_value(json_val: &serde_json::Value) -> *mut libc::c_char {
    if let Some(s) = json_val.as_str() {
        return CString::new(s)
            .map(|c| c.into_raw())
            .unwrap_or(std::ptr::null_mut());
    }

    if let Some(arr) = json_val.as_array() {
        if let Some(first) = arr.first() {
            return extract_first_str_from_value(first);
        }
    }

    if let Some(obj) = json_val.as_object() {
        if let Some((_, val)) = obj.iter().next() {
            if let Some(s) = val.as_str() {
                return CString::new(s)
                    .map(|c| c.into_raw())
                    .unwrap_or(std::ptr::null_mut());
            }
        }
    }

    std::ptr::null_mut()
}


