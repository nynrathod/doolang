//! RFC 7807 Error System
//!
//! Re-exports centralized RFC 7807 errors from doo_ffi_core.
//! SINGLE SOURCE OF TRUTH: All error types come from doo_ffi_core::rfc7807
//!
//! This module provides HTTP-specific convenience constructors that wrap
//! the core Rfc7807Error with HTTP context (instance path).

use std::collections::HashMap;

// Re-export the centralized RFC 7807 types
pub use doo_ffi_core::rfc7807::FieldError as CoreFieldError;
pub use doo_ffi_core::rfc7807::ParameterError;
pub use doo_ffi_core::Rfc7807Error;

// Convenience type alias for backward compatibility
pub type ErrorResponse = Rfc7807Error;

// ============================================================================
// HTTP-specific FieldError (for backward compatibility with existing code)
// ============================================================================

/// HTTP-specific field error wrapper.
/// Converts to core FieldError for use in Rfc7807Error.
#[derive(Debug, Clone)]
pub struct FieldError {
    pub rule: Option<String>,
    pub message: String,
    pub value: Option<String>,
    pub expected: Option<String>,
    pub received: Option<String>,
    pub error: Option<String>,
}

impl FieldError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            rule: None,
            message: message.into(),
            value: None,
            expected: None,
            received: None,
            error: None,
        }
    }

    /// Create a type mismatch error (no message per router-error.md spec)
    pub fn type_mismatch(expected: impl Into<String>, received: impl Into<String>) -> Self {
        Self {
            rule: None,
            message: String::new(),
            value: None,
            expected: Some(expected.into()),
            received: Some(received.into()),
            error: None,
        }
    }

    /// Create a required field error (sets error: "required" per router-error.md)
    pub fn required() -> Self {
        Self {
            rule: None,
            message: "This field is required".to_string(),
            value: None,
            expected: None,
            received: None,
            error: Some("required".to_string()),
        }
    }

    pub fn with_rule(mut self, rule: impl Into<String>) -> Self {
        self.rule = Some(rule.into());
        self
    }

    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
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

    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    /// Convert to core FieldError
    pub fn to_core(&self, _field_name: &str) -> CoreFieldError {
        let mut fe = if self.message.is_empty() {
            // Type mismatch: no message
            CoreFieldError {
                rule: None,
                message: None,
                value: None,
                expected: None,
                received: None,
                error: None,
            }
        } else {
            CoreFieldError::new(_field_name, &self.message)
        };
        if let Some(ref rule) = self.rule {
            fe = fe.with_rule(rule);
        }
        if let Some(ref value) = self.value {
            fe = fe.with_value(value);
        }
        if let Some(ref expected) = self.expected {
            fe = fe.with_expected(expected);
        }
        if let Some(ref received) = self.received {
            fe = fe.with_received(received);
        }
        if let Some(ref error) = self.error {
            fe = fe.with_error(error);
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
    Rfc7807Error::not_found(detail).with_instance(instance)
}

/// 405 Method Not Allowed
pub fn method_not_allowed(
    detail: impl Into<String>,
    instance: impl Into<String>,
    method: impl Into<String>,
    allowed: Vec<String>,
) -> Rfc7807Error {
    Rfc7807Error::new(405, detail)
        .with_instance(instance)
        .with_method(method)
        .with_allowed_methods(allowed)
}

/// 409 Conflict
pub fn conflict(detail: impl Into<String>, instance: impl Into<String>) -> Rfc7807Error {
    Rfc7807Error::conflict(detail).with_instance(instance)
}

/// 422 Validation Error
pub fn validation_error(
    instance: impl Into<String>,
    errors: HashMap<String, FieldError>,
) -> Rfc7807Error {
    // Convert HTTP FieldErrors to core FieldErrors in a HashMap
    let core_fields: HashMap<String, CoreFieldError> = errors
        .into_iter()
        .map(|(field, err)| (field.clone(), err.to_core(&field)))
        .collect();

    Rfc7807Error::validation_error(core_fields).with_instance(instance)
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
