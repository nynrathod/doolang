//! RFC 7807 Problem Details for HTTP APIs - Error Response Builder
//!
//! This module provides a centralized, dynamic error response builder
//! that follows RFC 7807 specification for all HTTP error responses.

use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Error type identifiers following RFC 7807
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorType {
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

impl ErrorType {
    /// Get the error type string identifier
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorType::BadRequest => "bad_request",
            ErrorType::Unauthorized => "unauthorized",
            ErrorType::Forbidden => "forbidden",
            ErrorType::NotFound => "not_found",
            ErrorType::MethodNotAllowed => "method_not_allowed",
            ErrorType::Conflict => "conflict",
            ErrorType::UnprocessableEntity => "validation_error",
            ErrorType::TooManyRequests => "rate_limit_exceeded",
            ErrorType::InternalError => "internal_error",
            ErrorType::NotImplemented => "not_implemented",
            ErrorType::BadGateway => "bad_gateway",
            ErrorType::ServiceUnavailable => "service_unavailable",
        }
    }

    /// Get the human-readable title
    pub fn title(&self) -> &'static str {
        match self {
            ErrorType::BadRequest => "Bad Request",
            ErrorType::Unauthorized => "Unauthorized",
            ErrorType::Forbidden => "Forbidden",
            ErrorType::NotFound => "Not Found",
            ErrorType::MethodNotAllowed => "Method Not Allowed",
            ErrorType::Conflict => "Conflict",
            ErrorType::UnprocessableEntity => "Validation Failed",
            ErrorType::TooManyRequests => "Too Many Requests",
            ErrorType::InternalError => "Internal Server Error",
            ErrorType::NotImplemented => "Not Implemented",
            ErrorType::BadGateway => "Bad Gateway",
            ErrorType::ServiceUnavailable => "Service Unavailable",
        }
    }

    /// Get the HTTP status code
    pub fn status_code(&self) -> u16 {
        match self {
            ErrorType::BadRequest => 400,
            ErrorType::Unauthorized => 401,
            ErrorType::Forbidden => 403,
            ErrorType::NotFound => 404,
            ErrorType::MethodNotAllowed => 405,
            ErrorType::Conflict => 409,
            ErrorType::UnprocessableEntity => 422,
            ErrorType::TooManyRequests => 429,
            ErrorType::InternalError => 500,
            ErrorType::NotImplemented => 501,
            ErrorType::BadGateway => 502,
            ErrorType::ServiceUnavailable => 503,
        }
    }

    /// Create from HTTP status code
    pub fn from_status_code(status: u16) -> Self {
        match status {
            400 => ErrorType::BadRequest,
            401 => ErrorType::Unauthorized,
            403 => ErrorType::Forbidden,
            404 => ErrorType::NotFound,
            405 => ErrorType::MethodNotAllowed,
            409 => ErrorType::Conflict,
            422 => ErrorType::UnprocessableEntity,
            429 => ErrorType::TooManyRequests,
            500 => ErrorType::InternalError,
            501 => ErrorType::NotImplemented,
            502 => ErrorType::BadGateway,
            503 => ErrorType::ServiceUnavailable,
            _ => ErrorType::InternalError,
        }
    }
}

/// Field error for validation errors
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
    pub fn new(message: String) -> Self {
        Self {
            rule: None,
            message,
            value: None,
            expected: None,
            received: None,
            error: None,
        }
    }

    pub fn with_rule(mut self, rule: String) -> Self {
        self.rule = Some(rule);
        self
    }

    pub fn with_value(mut self, value: String) -> Self {
        self.value = Some(value);
        self
    }

    pub fn with_expected(mut self, expected: String) -> Self {
        self.expected = Some(expected);
        self
    }

    pub fn with_received(mut self, received: String) -> Self {
        self.received = Some(received);
        self
    }

    pub fn with_error(mut self, error: String) -> Self {
        self.error = Some(error);
        self
    }

    pub fn to_json_value(&self) -> Value {
        let mut obj = Map::new();

        if let Some(ref rule) = self.rule {
            obj.insert("rule".to_string(), json!(rule));
        }
        obj.insert("message".to_string(), json!(self.message));

        if let Some(ref value) = self.value {
            obj.insert("value".to_string(), json!(value));
        }
        if let Some(ref expected) = self.expected {
            obj.insert("expected".to_string(), json!(expected));
        }
        if let Some(ref received) = self.received {
            obj.insert("received".to_string(), json!(received));
        }
        if let Some(ref error) = self.error {
            obj.insert("error".to_string(), json!(error));
        }

        Value::Object(obj)
    }
}

/// Parameter error for path/query parameter errors
#[derive(Debug, Clone)]
pub struct ParameterError {
    pub name: String,
    pub expected: Option<String>,
    pub received: Option<String>,
    pub message: Option<String>,
}

impl ParameterError {
    pub fn new(name: String) -> Self {
        Self {
            name,
            expected: None,
            received: None,
            message: None,
        }
    }

    pub fn with_expected(mut self, expected: String) -> Self {
        self.expected = Some(expected);
        self
    }

    pub fn with_received(mut self, received: String) -> Self {
        self.received = Some(received);
        self
    }

    pub fn with_message(mut self, message: String) -> Self {
        self.message = Some(message);
        self
    }

    pub fn to_json_value(&self) -> Value {
        let mut obj = Map::new();
        obj.insert("name".to_string(), json!(self.name));

        if let Some(ref expected) = self.expected {
            obj.insert("expected".to_string(), json!(expected));
        }
        if let Some(ref received) = self.received {
            obj.insert("received".to_string(), json!(received));
        }
        if let Some(ref message) = self.message {
            obj.insert("message".to_string(), json!(message));
        }

        Value::Object(obj)
    }
}

/// RFC 7807 Error Response Builder
#[derive(Debug, Clone)]
pub struct ErrorResponse {
    error_type: ErrorType,
    detail: String,
    instance: String,
    fields: Option<HashMap<String, FieldError>>,
    parameter: Option<ParameterError>,
    allowed_methods: Option<Vec<String>>,
    unknown_fields: Option<Vec<String>>,
    expected: Option<String>,
    received: Option<String>,
    retry_after: Option<u64>,
    trace_id: Option<String>,
    handler: Option<String>,
    method: Option<String>,
    message: Option<String>,
}

impl ErrorResponse {
    /// Create a new error response
    pub fn new(error_type: ErrorType, detail: String, instance: String) -> Self {
        Self {
            error_type,
            detail,
            instance,
            fields: None,
            parameter: None,
            allowed_methods: None,
            unknown_fields: None,
            expected: None,
            received: None,
            retry_after: None,
            trace_id: None,
            handler: None,
            method: None,
            message: None,
        }
    }

    /// Add validation field errors
    pub fn with_fields(mut self, fields: HashMap<String, FieldError>) -> Self {
        self.fields = Some(fields);
        self
    }

    /// Add a single field error
    pub fn with_field(mut self, field_name: String, field_error: FieldError) -> Self {
        if self.fields.is_none() {
            self.fields = Some(HashMap::new());
        }
        self.fields
            .as_mut()
            .unwrap()
            .insert(field_name, field_error);
        self
    }

    /// Add parameter error
    pub fn with_parameter(mut self, parameter: ParameterError) -> Self {
        self.parameter = Some(parameter);
        self
    }

    /// Add allowed methods (for 405 Method Not Allowed)
    pub fn with_allowed_methods(mut self, methods: Vec<String>) -> Self {
        self.allowed_methods = Some(methods);
        self
    }

    /// Add unknown fields
    pub fn with_unknown_fields(mut self, fields: Vec<String>) -> Self {
        self.unknown_fields = Some(fields);
        self
    }

    /// Add expected value
    pub fn with_expected(mut self, expected: String) -> Self {
        self.expected = Some(expected);
        self
    }

    /// Add received value
    pub fn with_received(mut self, received: String) -> Self {
        self.received = Some(received);
        self
    }

    /// Add retry_after (for 429 Too Many Requests)
    pub fn with_retry_after(mut self, seconds: u64) -> Self {
        self.retry_after = Some(seconds);
        self
    }

    /// Add trace_id (for 500 Internal Server Error)
    pub fn with_trace_id(mut self, trace_id: String) -> Self {
        self.trace_id = Some(trace_id);
        self
    }

    /// Add handler name (for 500 Internal Server Error)
    pub fn with_handler(mut self, handler: String) -> Self {
        self.handler = Some(handler);
        self
    }

    /// Add HTTP method
    pub fn with_method(mut self, method: String) -> Self {
        self.method = Some(method);
        self
    }

    /// Add additional message
    pub fn with_message(mut self, message: String) -> Self {
        self.message = Some(message);
        self
    }

    /// Build the JSON response string
    pub fn to_json_string(&self) -> String {
        let mut obj = Map::new();

        // Required RFC 7807 fields
        obj.insert("type".to_string(), json!(self.error_type.as_str()));
        obj.insert("title".to_string(), json!(self.error_type.title()));
        obj.insert("status".to_string(), json!(self.error_type.status_code()));
        obj.insert("detail".to_string(), json!(self.detail));
        obj.insert("instance".to_string(), json!(self.instance));

        // Optional fields based on error type
        if let Some(ref fields) = self.fields {
            let fields_obj: Map<String, Value> = fields
                .iter()
                .map(|(k, v)| (k.clone(), v.to_json_value()))
                .collect();
            obj.insert("fields".to_string(), Value::Object(fields_obj));
        }

        if let Some(ref param) = self.parameter {
            obj.insert("parameter".to_string(), param.to_json_value());
        }

        if let Some(ref methods) = self.allowed_methods {
            obj.insert("allowed_methods".to_string(), json!(methods));
        }

        if let Some(ref unknown) = self.unknown_fields {
            obj.insert("unknown_fields".to_string(), json!(unknown));
        }

        if let Some(ref expected) = self.expected {
            obj.insert("expected".to_string(), json!(expected));
        }

        if let Some(ref received) = self.received {
            obj.insert("received".to_string(), json!(received));
        }

        if let Some(retry) = self.retry_after {
            obj.insert("retry_after".to_string(), json!(retry));
        }

        if let Some(ref trace_id) = self.trace_id {
            obj.insert("trace_id".to_string(), json!(trace_id));
        }

        if let Some(ref handler) = self.handler {
            obj.insert("handler".to_string(), json!(handler));
        }

        if let Some(ref method) = self.method {
            obj.insert("method".to_string(), json!(method));
        }

        if let Some(ref message) = self.message {
            obj.insert("message".to_string(), json!(message));
        }

        serde_json::to_string(&Value::Object(obj)).unwrap_or_else(|_| {
            format!(
                r#"{{"type":"{}","title":"{}","status":{},"detail":"{}","instance":"{}"}}"#,
                self.error_type.as_str(),
                self.error_type.title(),
                self.error_type.status_code(),
                self.detail.replace('"', "\\\""),
                self.instance.replace('"', "\\\"")
            )
        })
    }

    /// Get the HTTP status code
    pub fn status_code(&self) -> u16 {
        self.error_type.status_code()
    }
}

/// Convenience functions for common error types

pub fn bad_request(detail: String, instance: String) -> ErrorResponse {
    ErrorResponse::new(ErrorType::BadRequest, detail, instance)
}

pub fn unauthorized(detail: String, instance: String) -> ErrorResponse {
    ErrorResponse::new(ErrorType::Unauthorized, detail, instance)
}

pub fn forbidden(detail: String, instance: String) -> ErrorResponse {
    ErrorResponse::new(ErrorType::Forbidden, detail, instance)
}

pub fn not_found(detail: String, instance: String) -> ErrorResponse {
    ErrorResponse::new(ErrorType::NotFound, detail, instance)
}

pub fn method_not_allowed(
    detail: String,
    instance: String,
    allowed_methods: Vec<String>,
) -> ErrorResponse {
    ErrorResponse::new(ErrorType::MethodNotAllowed, detail, instance)
        .with_allowed_methods(allowed_methods)
}

pub fn conflict(detail: String, instance: String) -> ErrorResponse {
    ErrorResponse::new(ErrorType::Conflict, detail, instance)
}

pub fn validation_error(
    detail: String,
    instance: String,
    fields: HashMap<String, FieldError>,
) -> ErrorResponse {
    ErrorResponse::new(ErrorType::UnprocessableEntity, detail, instance).with_fields(fields)
}

pub fn too_many_requests(detail: String, instance: String, retry_after: u64) -> ErrorResponse {
    ErrorResponse::new(ErrorType::TooManyRequests, detail, instance).with_retry_after(retry_after)
}

pub fn internal_error(detail: String, instance: String) -> ErrorResponse {
    ErrorResponse::new(ErrorType::InternalError, detail, instance)
}

pub fn not_implemented(detail: String, instance: String) -> ErrorResponse {
    ErrorResponse::new(ErrorType::NotImplemented, detail, instance)
}

pub fn bad_gateway(detail: String, instance: String) -> ErrorResponse {
    ErrorResponse::new(ErrorType::BadGateway, detail, instance)
}

pub fn service_unavailable(detail: String, instance: String) -> ErrorResponse {
    ErrorResponse::new(ErrorType::ServiceUnavailable, detail, instance)
}

/// Build a path/query parameter error (400 Bad Request) with parameter details
pub fn parameter_error(
    detail: String,
    instance: String,
    parameter: ParameterError,
) -> ErrorResponse {
    ErrorResponse::new(ErrorType::BadRequest, detail, instance).with_parameter(parameter)
}

/// Build a bad request for missing/invalid content type
pub fn content_type_error(
    detail: String,
    instance: String,
    expected: Option<String>,
    received: Option<String>,
) -> ErrorResponse {
    let mut resp = ErrorResponse::new(ErrorType::BadRequest, detail, instance);
    if let Some(exp) = expected {
        resp = resp.with_expected(exp);
    }
    if let Some(rec) = received {
        resp = resp.with_received(rec);
    }
    resp
}

/// Build a bad request for unknown fields during deserialization
pub fn unknown_fields_error(
    detail: String,
    instance: String,
    unknown_fields: Vec<String>,
) -> ErrorResponse {
    ErrorResponse::new(ErrorType::BadRequest, detail, instance).with_unknown_fields(unknown_fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_basic_error_response() {
        let err = bad_request(
            "Invalid JSON: unexpected character at line 1, column 5".to_string(),
            "/api/users/signup".to_string(),
        );

        let json = err.to_json_string();
        assert!(json.contains(r#""type":"bad_request""#));
        assert!(json.contains(r#""title":"Bad Request""#));
        assert!(json.contains(r#""status":400"#));
        assert!(json.contains(r#""detail":"Invalid JSON"#));
        assert!(json.contains(r#""instance":"/api/users/signup""#));
    }

    #[test]
    fn test_validation_error_with_fields() {
        let mut fields = HashMap::new();
        fields.insert(
            "Email".to_string(),
            FieldError::new("Invalid email format".to_string())
                .with_rule("email".to_string())
                .with_value("not-an-email".to_string()),
        );

        let err = validation_error(
            "One or more fields failed validation".to_string(),
            "/api/users/signup".to_string(),
            fields,
        );

        let json = err.to_json_string();
        assert!(json.contains(r#""type":"validation_error""#));
        assert!(json.contains(r#""status":422"#));
        assert!(json.contains(r#""fields""#));
    }

    #[test]
    fn test_method_not_allowed() {
        let err = method_not_allowed(
            "The requested method is not allowed for this route".to_string(),
            "/users".to_string(),
            vec!["GET".to_string(), "POST".to_string()],
        );

        let json = err.to_json_string();
        assert!(json.contains(r#""type":"method_not_allowed""#));
        assert!(json.contains(r#""status":405"#));
        assert!(json.contains(r#""allowed_methods""#));
    }
}
