//! RFC 7807 Error System
//!
//! Re-exports centralized RFC 7807 errors from doo_ffi_core.
//! SINGLE SOURCE OF TRUTH: All error types come from doo_ffi_core::rfc7807
//!
//! This module provides HTTP-specific convenience constructors that wrap
//! the core Rfc7807Error with HTTP context (instance path).

use std::collections::HashMap;

// Re-export the centralized RFC 7807 types
pub use doo_ffi_core::FieldError as CoreFieldError;
pub use doo_ffi_core::Rfc7807Error;

// Convenience type alias for backward compatibility
pub type ErrorResponse = Rfc7807Error;

// ============================================================================
// HTTP-specific FieldError (for backward compatibility with existing code)
// ============================================================================

/// HTTP-specific field error (for validation_error function)
/// This provides a simpler API compatible with existing code.
#[derive(Debug, Clone)]
pub struct FieldError {
    pub rule: Option<String>,
    pub message: String,
    pub expected: Option<String>,
    pub received: Option<String>,
}

impl FieldError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            rule: None,
            message: message.into(),
            expected: None,
            received: None,
        }
    }

    pub fn with_rule(mut self, rule: impl Into<String>) -> Self {
        self.rule = Some(rule.into());
        self
    }

    pub fn with_expected(mut self, expected: impl Into<String>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    pub fn with_received(mut self, received: impl Into<String>) -> Self {
        self.received = Some(received.into());
        self
    }

    /// Convert to core FieldError
    pub fn to_core(&self, field_name: &str) -> CoreFieldError {
        let mut fe = CoreFieldError::new(field_name, &self.message);
        if let Some(ref rule) = self.rule {
            fe = fe.with_rule(rule);
        }
        if let Some(ref expected) = self.expected {
            fe = fe.with_expected(expected);
        }
        if let Some(ref received) = self.received {
            fe = fe.with_received(received);
        }
        fe
    }
}

// ============================================================================
// HTTP-specific convenience constructors
// These wrap the core Rfc7807Error with HTTP-specific context (instance path)
// ============================================================================

/// 400 Bad Request
pub fn bad_request(detail: impl Into<String>, instance: impl Into<String>) -> Rfc7807Error {
    Rfc7807Error::bad_request(detail).with_instance(instance)
}

/// 401 Unauthorized
pub fn unauthorized(detail: impl Into<String>, instance: impl Into<String>) -> Rfc7807Error {
    Rfc7807Error::unauthorized()
        .with_detail(detail)
        .with_instance(instance)
}

/// 403 Forbidden
pub fn forbidden(detail: impl Into<String>, instance: impl Into<String>) -> Rfc7807Error {
    Rfc7807Error::forbidden()
        .with_detail(detail)
        .with_instance(instance)
}

/// 404 Not Found
pub fn not_found(detail: impl Into<String>, instance: impl Into<String>) -> Rfc7807Error {
    Rfc7807Error::new(404, "Not Found")
        .with_detail(detail)
        .with_instance(instance)
}

/// 405 Method Not Allowed
pub fn method_not_allowed(
    detail: impl Into<String>,
    instance: impl Into<String>,
    _allowed: Vec<String>,
) -> Rfc7807Error {
    Rfc7807Error::new(405, "Method Not Allowed")
        .with_detail(detail)
        .with_instance(instance)
}

/// 409 Conflict
pub fn conflict(detail: impl Into<String>, instance: impl Into<String>) -> Rfc7807Error {
    Rfc7807Error::conflict(detail).with_instance(instance)
}

/// 422 Validation Error
pub fn validation_error(
    detail: impl Into<String>,
    instance: impl Into<String>,
    errors: HashMap<String, FieldError>,
) -> Rfc7807Error {
    // Convert HTTP FieldErrors to core FieldErrors
    let core_errors: Vec<CoreFieldError> = errors
        .into_iter()
        .map(|(field, err)| err.to_core(&field))
        .collect();

    Rfc7807Error::validation_error(core_errors)
        .with_detail(detail)
        .with_instance(instance)
}

/// 429 Too Many Requests
pub fn too_many_requests(detail: impl Into<String>, instance: impl Into<String>) -> Rfc7807Error {
    Rfc7807Error::rate_limited()
        .with_detail(detail)
        .with_instance(instance)
}

/// 500 Internal Server Error
pub fn internal_error(detail: impl Into<String>, instance: impl Into<String>) -> Rfc7807Error {
    Rfc7807Error::internal(detail).with_instance(instance)
}
