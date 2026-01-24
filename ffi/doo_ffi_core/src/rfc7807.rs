//! RFC 7807 Error Format
//!
//! Standard error format for all HTTP errors.

use serde::{Deserialize, Serialize};

/// RFC 7807 Problem Details for HTTP APIs.
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
    
    /// A URI reference that identifies the specific occurrence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    
    /// Custom extension fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<FieldError>>,
}

/// Field-level validation error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldError {
    /// Field name
    pub field: String,
    /// Error message
    pub message: String,
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

    /// Add field errors.
    pub fn with_errors(mut self, errors: Vec<FieldError>) -> Self {
        self.errors = Some(errors);
        self
    }

    /// Convert to JSON string.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            r#"{"type":"https://doo.dev/errors/500","title":"Internal Error","status":500}"#.to_string()
        })
    }

    // ========================================================================
    // Common Errors
    // ========================================================================

    /// 400 Bad Request
    pub fn bad_request(detail: &str) -> Self {
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
    pub fn not_found(resource: &str) -> Self {
        Self::new(404, "Not Found").with_detail(format!("{} not found", resource))
    }

    /// 409 Conflict
    pub fn conflict(detail: &str) -> Self {
        Self::new(409, "Conflict").with_detail(detail)
    }

    /// 422 Unprocessable Entity
    pub fn unprocessable(errors: Vec<FieldError>) -> Self {
        Self::new(422, "Validation Failed").with_errors(errors)
    }

    /// 429 Too Many Requests
    pub fn rate_limited() -> Self {
        Self::new(429, "Too Many Requests")
    }

    /// 500 Internal Server Error
    pub fn internal(detail: &str) -> Self {
        Self::new(500, "Internal Server Error").with_detail(detail)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rfc7807_new() {
        let err = Rfc7807Error::new(404, "Not Found");
        assert_eq!(err.status, 404);
        assert_eq!(err.title, "Not Found");
    }

    #[test]
    fn test_rfc7807_json() {
        let err = Rfc7807Error::not_found("User");
        let json = err.to_json();
        assert!(json.contains("\"status\":404"));
        assert!(json.contains("\"title\":\"Not Found\""));
    }
}
