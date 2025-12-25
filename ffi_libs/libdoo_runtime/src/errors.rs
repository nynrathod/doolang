//! Centralized error handling for all Doo FFI modules
//! Provides RFC 7807 compliant HTTP errors, PostgreSQL database errors, and OAuth 2.0/JWT auth errors

use serde_json::json;
use std::ffi::CString;
use std::os::raw::c_char;

// ============================================================================
// HTTP Errors (RFC 7807 - Problem Details for HTTP APIs)
// ============================================================================

/// HTTP error types following RFC 7807
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpErrorType {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    Conflict,
    UnprocessableEntity,
    TooManyRequests,
    InternalError,
    NotImplemented,
    BadGateway,
    ServiceUnavailable,
}

impl HttpErrorType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::MethodNotAllowed => "method_not_allowed",
            Self::Conflict => "conflict",
            Self::UnprocessableEntity => "validation_error",
            Self::TooManyRequests => "rate_limit_exceeded",
            Self::InternalError => "internal_error",
            Self::NotImplemented => "not_implemented",
            Self::BadGateway => "bad_gateway",
            Self::ServiceUnavailable => "service_unavailable",
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::BadRequest => "Bad Request",
            Self::Unauthorized => "Unauthorized",
            Self::Forbidden => "Forbidden",
            Self::NotFound => "Not Found",
            Self::MethodNotAllowed => "Method Not Allowed",
            Self::Conflict => "Conflict",
            Self::UnprocessableEntity => "Validation Failed",
            Self::TooManyRequests => "Too Many Requests",
            Self::InternalError => "Internal Server Error",
            Self::NotImplemented => "Not Implemented",
            Self::BadGateway => "Bad Gateway",
            Self::ServiceUnavailable => "Service Unavailable",
        }
    }

    pub fn status_code(&self) -> u16 {
        match self {
            Self::BadRequest => 400,
            Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::MethodNotAllowed => 405,
            Self::Conflict => 409,
            Self::UnprocessableEntity => 422,
            Self::TooManyRequests => 429,
            Self::InternalError => 500,
            Self::NotImplemented => 501,
            Self::BadGateway => 502,
            Self::ServiceUnavailable => 503,
        }
    }
}

/// RFC 7807 compliant HTTP error
#[derive(Debug, Clone)]
pub struct HttpError {
    pub error_type: HttpErrorType,
    pub detail: String,
    pub instance: Option<String>,
    pub fields: Option<serde_json::Map<String, serde_json::Value>>,
}

impl HttpError {
    pub fn new(error_type: HttpErrorType, detail: impl Into<String>) -> Self {
        Self {
            error_type,
            detail: detail.into(),
            instance: None,
            fields: None,
        }
    }

    pub fn with_instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    pub fn with_fields(mut self, fields: serde_json::Map<String, serde_json::Value>) -> Self {
        self.fields = Some(fields);
        self
    }

    /// Convert to RFC 7807 JSON string
    pub fn to_json_string(&self) -> String {
        let mut obj = serde_json::Map::new();
        obj.insert("type".to_string(), json!(self.error_type.as_str()));
        obj.insert("title".to_string(), json!(self.error_type.title()));
        obj.insert("status".to_string(), json!(self.error_type.status_code()));
        obj.insert("detail".to_string(), json!(self.detail));
        
        if let Some(ref instance) = self.instance {
            obj.insert("instance".to_string(), json!(instance));
        }
        
        if let Some(ref fields) = self.fields {
            obj.insert("fields".to_string(), json!(fields));
        }
        
        json!(obj).to_string()
    }

    pub fn status_code(&self) -> u16 {
        self.error_type.status_code()
    }
}

// Helper functions for common HTTP errors
pub fn bad_request(detail: impl Into<String>) -> HttpError {
    HttpError::new(HttpErrorType::BadRequest, detail)
}

pub fn unauthorized(detail: impl Into<String>) -> HttpError {
    HttpError::new(HttpErrorType::Unauthorized, detail)
}

pub fn forbidden(detail: impl Into<String>) -> HttpError {
    HttpError::new(HttpErrorType::Forbidden, detail)
}

pub fn not_found(detail: impl Into<String>) -> HttpError {
    HttpError::new(HttpErrorType::NotFound, detail)
}

pub fn conflict(detail: impl Into<String>) -> HttpError {
    HttpError::new(HttpErrorType::Conflict, detail)
}

pub fn validation_error(detail: impl Into<String>) -> HttpError {
    HttpError::new(HttpErrorType::UnprocessableEntity, detail)
}

pub fn internal_error(detail: impl Into<String>) -> HttpError {
    HttpError::new(HttpErrorType::InternalError, detail)
}

// ============================================================================
// Database Errors (PostgreSQL standard)
// ============================================================================

/// Database error codes following PostgreSQL standards
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbErrorCode {
    // Connection errors
    ConnectionFailure,
    InvalidConnectionString,
    AuthenticationFailed,
    
    // Query errors
    SyntaxError,
    UndefinedTable,
    UndefinedColumn,
    InvalidDataType,
    
    // Constraint violations
    UniqueViolation,
    ForeignKeyViolation,
    NotNullViolation,
    CheckViolation,
    
    // Transaction errors
    DeadlockDetected,
    SerializationFailure,
    
    // Data errors
    StringDataTooLong,
    NumericValueOutOfRange,
    InvalidTextRepresentation,
    
    // General
    InternalError,
    Unknown,
}

impl DbErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ConnectionFailure => "08006",
            Self::InvalidConnectionString => "08P01",
            Self::AuthenticationFailed => "28P01",
            Self::SyntaxError => "42601",
            Self::UndefinedTable => "42P01",
            Self::UndefinedColumn => "42703",
            Self::InvalidDataType => "42804",
            Self::UniqueViolation => "23505",
            Self::ForeignKeyViolation => "23503",
            Self::NotNullViolation => "23502",
            Self::CheckViolation => "23514",
            Self::DeadlockDetected => "40P01",
            Self::SerializationFailure => "40001",
            Self::StringDataTooLong => "22001",
            Self::NumericValueOutOfRange => "22003",
            Self::InvalidTextRepresentation => "22P02",
            Self::InternalError => "XX000",
            Self::Unknown => "XXXXX",
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::ConnectionFailure => "Connection failed",
            Self::InvalidConnectionString => "Invalid connection string",
            Self::AuthenticationFailed => "Authentication failed",
            Self::SyntaxError => "Syntax error in SQL query",
            Self::UndefinedTable => "Table does not exist",
            Self::UndefinedColumn => "Column does not exist",
            Self::InvalidDataType => "Invalid data type",
            Self::UniqueViolation => "Unique constraint violation",
            Self::ForeignKeyViolation => "Foreign key constraint violation",
            Self::NotNullViolation => "NOT NULL constraint violation",
            Self::CheckViolation => "Check constraint violation",
            Self::DeadlockDetected => "Deadlock detected",
            Self::SerializationFailure => "Serialization failure",
            Self::StringDataTooLong => "String data too long",
            Self::NumericValueOutOfRange => "Numeric value out of range",
            Self::InvalidTextRepresentation => "Invalid text representation",
            Self::InternalError => "Internal database error",
            Self::Unknown => "Unknown database error",
        }
    }
}

/// Database error structure
#[derive(Debug, Clone)]
pub struct DbError {
    pub code: DbErrorCode,
    pub message: String,
    pub detail: Option<String>,
    pub hint: Option<String>,
}

impl DbError {
    pub fn new(code: DbErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
            hint: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Convert to JSON string for FFI
    pub fn to_json_string(&self) -> String {
        let mut obj = serde_json::Map::new();
        obj.insert("success".to_string(), json!(false));
        
        let mut error = serde_json::Map::new();
        error.insert("code".to_string(), json!(self.code.as_str()));
        error.insert("message".to_string(), json!(self.message));
        
        if let Some(ref detail) = self.detail