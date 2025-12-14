//! Authentication error handling using centralized libdoo_runtime errors
//! This module re-exports and provides convenience wrappers

use serde_json::json;
use std::ffi::CString;
use std::os::raw::c_char;

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

    /// Convert to C string for FFI
    pub fn to_c_string(&self) -> *mut c_char {
        let json = self.to_json_string();
        CString::new(json)
            .map(|c| c.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }
}

/// Helper functions for common auth errors
pub fn jwt_expired() -> AuthError {
    AuthError::new(AuthErrorCode::JwtExpired)
}

pub fn jwt_invalid() -> AuthError {
    AuthError::new(AuthErrorCode::JwtInvalid)
}

pub fn jwt_malformed() -> AuthError {
    AuthError::new(AuthErrorCode::JwtMalformed)
}

pub fn jwt_signature_invalid() -> AuthError {
    AuthError::new(AuthErrorCode::JwtSignatureInvalid)
}

pub fn invalid_credentials() -> AuthError {
    AuthError::new(AuthErrorCode::InvalidCredentials)
}

pub fn email_already_exists() -> AuthError {
    AuthError::new(AuthErrorCode::EmailAlreadyExists)
}

pub fn user_not_found() -> AuthError {
    AuthError::new(AuthErrorCode::UserNotFound)
}

pub fn password_too_weak() -> AuthError {
    AuthError::new(AuthErrorCode::PasswordTooWeak)
}

pub fn token_missing() -> AuthError {
    AuthError::new(AuthErrorCode::TokenMissing)
}

pub fn jwt_secret_missing() -> AuthError {
    AuthError::new(AuthErrorCode::SecretNotConfigured)
}

pub fn insufficient_permissions() -> AuthError {
    AuthError::new(AuthErrorCode::InsufficientPermissions)
}

pub fn internal_error(msg: &str) -> AuthError {
    AuthError::with_message(AuthErrorCode::InternalError, msg)
}
