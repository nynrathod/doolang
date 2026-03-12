//! Runtime Error Codes
//!
//! Centralized error codes for Database and Authentication operations.
//! Follows PostgreSQL and OAuth2/JWT standards.

use serde::{Deserialize, Serialize};

// ============================================================================
// DATABASE ERROR CODES (PostgreSQL Standard)
// ============================================================================

/// Database error codes following PostgreSQL conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

// ============================================================================
// AUTH ERROR CODES (OAuth 2.0 / JWT RFC Standards)
// ============================================================================

/// Authentication error codes following OAuth 2.0 and JWT RFC standards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
            Self::JwtSignatureInvalid => "Invalid token signature",
            Self::InvalidCredentials => "Invalid email or password",
            Self::EmailAlreadyExists => "Email already registered",
            Self::UserNotFound => "User not found",
            Self::PasswordTooWeak => "Password does not meet requirements",
            Self::TokenMissing => "Authentication token missing",
            Self::TokenRevoked => "Token has been revoked",
            Self::InsufficientPermissions => "You do not have permission to perform this action",
            Self::SecretNotConfigured => "Authentication not configured correctly",
            Self::InternalError => "Internal authentication error",
        }
    }
}
