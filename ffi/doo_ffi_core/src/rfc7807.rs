//! RFC 7807 Error Format
//!
//! Standard error format for all HTTP errors.
//! SINGLE SOURCE OF TRUTH for all FFI error responses.
//! Used by: doo_ffi_http, doo_ffi_auth, doo_ffi_db

use serde::{Deserialize, Serialize};

/// RFC 7807 Problem Details for HTTP APIs.
/// This is the ONLY error format for all FFI modules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rfc7807Error {
    /// A URI reference that identifies the problem type.
    #[serde(rename = "type")]
    pub error_type: String,

    /// A short, human-readable summary of the problem.
    pub title: String,

    /// The HTTP status code.
    pub status: u16,

    /// A human-readable explanation specific to this occurrence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,

    /// A URI reference that identifies the specific occurrence (e.g., request path).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,

    /// Field-level validation errors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<FieldError>>,
}

/// Field-level validation error with full context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldError {
    /// Field name (e.g., "email", "tags[1]")
    pub field: String,
    /// Human-readable error message
    pub message: String,
    /// Validation rule that failed (e.g., "required", "type_mismatch", "email")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    /// Expected value/type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    /// Received value/type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub received: Option<String>,
}

impl FieldError {
    /// Create a simple field error.
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
            rule: None,
            expected: None,
            received: None,
        }
    }

    /// Create a type mismatch error.
    pub fn type_mismatch(
        field: impl Into<String>,
        expected: impl Into<String>,
        received: impl Into<String>,
    ) -> Self {
        let expected_str = expected.into();
        let received_str = received.into();
        Self {
            field: field.into(),
            message: format!("expected {}, got {}", expected_str, received_str),
            rule: Some("type_mismatch".to_string()),
            expected: Some(expected_str),
            received: Some(received_str),
        }
    }

    /// Create a required field error.
    pub fn required(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: "This field is required".to_string(),
            rule: Some("required".to_string()),
            expected: None,
            received: None,
        }
    }

    /// Add rule
    pub fn with_rule(mut self, rule: impl Into<String>) -> Self {
        self.rule = Some(rule.into());
        self
    }

    /// Add expected value
    pub fn with_expected(mut self, expected: impl Into<String>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    /// Add received value
    pub fn with_received(mut self, received: impl Into<String>) -> Self {
        self.received = Some(received.into());
        self
    }
}

impl Rfc7807Error {
    /// Create a new RFC 7807 error.
    pub fn new(status: u16, title: impl Into<String>) -> Self {
        Self {
            error_type: format!("https://doo.dev/errors/{}", status),
            title: title.into(),
            status,
            detail: None,
            instance: None,
            errors: None,
        }
    }

    /// Add detail.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Add instance (request path).
    pub fn with_instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    /// Add field errors.
    pub fn with_errors(mut self, errors: Vec<FieldError>) -> Self {
        self.errors = Some(errors);
        self
    }

    /// Add a single field error.
    pub fn with_field_error(mut self, error: FieldError) -> Self {
        match &mut self.errors {
            Some(errors) => errors.push(error),
            None => self.errors = Some(vec![error]),
        }
        self
    }

    /// Convert to JSON string.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            r#"{"type":"https://doo.dev/errors/500","title":"Internal Error","status":500}"#
                .to_string()
        })
    }

    // ========================================================================
    // Common Errors (convenience constructors)
    // ========================================================================

    /// 400 Bad Request
    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self::new(400, "Bad Request").with_detail(detail)
    }

    /// 401 Unauthorized
    pub fn unauthorized() -> Self {
        Self::new(401, "Unauthorized")
    }

    /// 403 Forbidden
    pub fn forbidden() -> Self {
        Self::new(403, "Forbidden")
    }

    /// 404 Not Found
    pub fn not_found(resource: impl Into<String>) -> Self {
        Self::new(404, "Not Found").with_detail(format!("{} not found", resource.into()))
    }

    /// 405 Method Not Allowed
    pub fn method_not_allowed(method: impl Into<String>, path: impl Into<String>) -> Self {
        Self::new(405, "Method Not Allowed").with_detail(format!(
            "{} not allowed for {}",
            method.into(),
            path.into()
        ))
    }

    /// 409 Conflict
    pub fn conflict(detail: impl Into<String>) -> Self {
        Self::new(409, "Conflict").with_detail(detail)
    }

    /// 422 Unprocessable Entity (validation failed)
    pub fn validation_error(errors: Vec<FieldError>) -> Self {
        Self::new(422, "Validation Failed")
            .with_detail("One or more fields failed validation")
            .with_errors(errors)
    }

    /// 429 Too Many Requests
    pub fn rate_limited() -> Self {
        Self::new(429, "Too Many Requests")
    }

    /// 500 Internal Server Error
    pub fn internal(detail: impl Into<String>) -> Self {
        Self::new(500, "Internal Server Error").with_detail(detail)
    }

    /// 503 Service Unavailable
    pub fn service_unavailable(detail: impl Into<String>) -> Self {
        Self::new(503, "Service Unavailable").with_detail(detail)
    }
}
