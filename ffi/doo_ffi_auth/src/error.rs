//! Auth Errors
//!
//! Centralized authentication error codes.

use serde::{Deserialize, Serialize};
use doo_ffi_core::Rfc7807Error;

/// Authentication error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthError {
    /// Invalid credentials
    InvalidCredentials,
    /// Token expired
    TokenExpired,
    /// Token invalid
    TokenInvalid,
    /// Token missing
    TokenMissing,
    /// Hash failed
    HashFailed,
    /// Verify failed
    VerifyFailed,
    /// Sign failed
    SignFailed,
    /// User not found
    UserNotFound,
    /// User already exists
    UserExists,
    /// Permission denied
    PermissionDenied,
}

impl AuthError {
    /// Get error code.
    pub fn code(&self) -> u16 {
        match self {
            Self::InvalidCredentials => 4001,
            Self::TokenExpired => 4002,
            Self::TokenInvalid => 4003,
            Self::TokenMissing => 4004,
            Self::HashFailed => 4005,
            Self::VerifyFailed => 4006,
            Self::SignFailed => 4007,
            Self::UserNotFound => 4008,
            Self::UserExists => 4009,
            Self::PermissionDenied => 4010,
        }
    }

    /// Get error message.
    pub fn message(&self) -> &'static str {
        match self {
            Self::InvalidCredentials => "Invalid credentials",
            Self::TokenExpired => "Token expired",
            Self::TokenInvalid => "Token is invalid",
            Self::TokenMissing => "Token missing",
            Self::HashFailed => "Password hash failed",
            Self::VerifyFailed => "Password verification failed",
            Self::SignFailed => "Token signing failed",
            Self::UserNotFound => "User not found",
            Self::UserExists => "User already exists",
            Self::PermissionDenied => "Permission denied",
        }
    }

    /// Convert to RFC 7807 error.
    pub fn to_rfc7807(&self) -> Rfc7807Error {
        match self {
            Self::InvalidCredentials | Self::TokenExpired | Self::TokenInvalid | Self::TokenMissing => {
                Rfc7807Error::unauthorized()
            }
            Self::PermissionDenied => Rfc7807Error::forbidden(),
            Self::UserNotFound => Rfc7807Error::not_found("User"),
            Self::UserExists => Rfc7807Error::conflict("User already exists"),
            _ => Rfc7807Error::internal(self.message()),
        }
    }
}
