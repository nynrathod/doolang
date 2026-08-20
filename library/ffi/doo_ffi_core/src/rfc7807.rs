//! RFC 7807 Error Format
//!
//! Standard error format for all HTTP errors.
//! SINGLE SOURCE OF TRUTH for all FFI error responses.
//! Used by: doo_ffi_http, doo_ffi_auth, doo_ffi_db
//!
//! All error types, fields, and JSON format are defined here.
//! No other module should define error response structures.

use serde_json::{json, Map, Value};
use std::collections::HashMap;

// ============================================================================
// Error Type Identifiers (SINGLE SOURCE OF TRUTH)
// Maps HTTP status codes to human-readable type strings.
// ============================================================================

/// Get the RFC 7807 `type` string for a given HTTP status code.
/// These are human-readable identifiers, NOT URLs.
pub fn error_type_for_status(status: u16) -> &'static str {
    match status {
        400 => "bad_request",
        401 => "unauthorized",
        403 => "forbidden",
        404 => "not_found",
        405 => "method_not_allowed",
        409 => "conflict",
        422 => "validation_error",
        429 => "rate_limit_exceeded",
        500 => "internal_error",
        501 => "not_implemented",
        502 => "bad_gateway",
        503 => "service_unavailable",
        _ => "error",
    }
}

/// Get the RFC 7807 `title` string for a given HTTP status code.
pub fn title_for_status(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

// ============================================================================
// Field-Level Error (used in `fields` map)
// ============================================================================

/// Field-level error for validation / deserialization errors.
/// Serialized as a value in the `"fields"` map, keyed by field name.
#[derive(Debug, Clone)]
pub struct FieldError {
    /// Validation rule that failed (e.g., "required", "type_mismatch", "email", "min:8")
    pub rule: Option<String>,
    /// Human-readable error message (only for validation/required errors, NOT type mismatch)
    pub message: Option<String>,
    /// The value that was received/failed validation
    pub value: Option<String>,
    /// Expected type or value
    pub expected: Option<String>,
    /// Received type or value
    pub received: Option<String>,
    /// Error category (e.g., "required" for missing field errors)
    pub error: Option<String>,
}

impl FieldError {
    /// Create a simple field error with a message.
    pub fn new(_field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            rule: None,
            message: Some(message.into()),
            value: None,
            expected: None,
            received: None,
            error: None,
        }
    }

    /// Create a type mismatch error (no message, only expected/received/value per spec).
    pub fn type_mismatch(
        _field: impl Into<String>,
        expected: impl Into<String>,
        received: impl Into<String>,
    ) -> Self {
        let expected_str = expected.into();
        let received_str = received.into();
        Self {
            rule: None,
            message: None,
            value: None,
            expected: Some(expected_str),
            received: Some(received_str),
            error: None,
        }
    }

    /// Create a required field error.
    pub fn required(_field: impl Into<String>) -> Self {
        Self {
            rule: None,
            message: Some("This field is required".to_string()),
            value: None,
            expected: None,
            received: None,
            error: Some("required".to_string()),
        }
    }

    /// Set the validation rule
    pub fn with_rule(mut self, rule: impl Into<String>) -> Self {
        self.rule = Some(rule.into());
        self
    }

    /// Set the expected type/value
    pub fn with_expected(mut self, expected: impl Into<String>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    /// Set the received type/value
    pub fn with_received(mut self, received: impl Into<String>) -> Self {
        self.received = Some(received.into());
        self
    }

    /// Set the value that failed validation
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Set the error category
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    /// Convert to a serde_json::Value for embedding in JSON response.
    pub fn to_json_value(&self) -> Value {
        let mut obj = Map::new();

        if let Some(ref rule) = self.rule {
            obj.insert("rule".to_string(), json!(rule));
        }
        if let Some(ref message) = self.message {
            obj.insert("message".to_string(), json!(message));
        }

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

// ============================================================================
// Parameter Error (for path/query parameter errors)
// ============================================================================

/// Parameter error for path/query parameter validation.
#[derive(Debug, Clone)]
pub struct ParameterError {
    /// Parameter name
    pub name: String,
    /// Expected type
    pub expected: Option<String>,
    /// Received value
    pub received: Option<String>,
    /// Error message
    pub message: Option<String>,
}

impl ParameterError {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            expected: None,
            received: None,
            message: None,
        }
    }

    pub fn with_expected(mut self, expected: impl Into<String>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    pub fn with_received(mut self, received: impl Into<String>) -> Self {
        self.received = Some(received.into());
        self
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Convert to a serde_json::Value
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

// ============================================================================
// RFC 7807 Error Response (SINGLE SOURCE OF TRUTH)
// ============================================================================

/// RFC 7807 Problem Details for HTTP APIs.
/// This is the ONLY error format for all FFI modules.
#[derive(Debug, Clone)]
pub struct Rfc7807Error {
    /// HTTP status code (drives `type` and `title` automatically)
    pub status: u16,
    /// Human-readable explanation specific to this occurrence
    pub detail: String,
    /// Request path (RFC 7807 `instance`)
    pub instance: String,
    /// Field-level errors: field_name -> FieldError
    pub fields: Option<HashMap<String, FieldError>>,
    /// Parameter error (path/query)
    pub parameter: Option<ParameterError>,
    /// HTTP method (for routing errors)
    pub method: Option<String>,
    /// Allowed methods (for 405)
    pub allowed_methods: Option<Vec<String>>,
    /// Unknown fields in request body
    pub unknown_fields: Option<Vec<String>>,
    /// Expected value/type (e.g., Content-Type)
    pub expected: Option<String>,
    /// Received value/type
    pub received: Option<String>,
    /// Additional message (e.g., middleware context)
    pub message: Option<String>,
    /// Trace ID for server errors
    pub trace_id: Option<String>,
    /// Override the `type` field (default derives from status code)
    pub type_override: Option<String>,
}

impl Rfc7807Error {
    /// Create a new RFC 7807 error.
    pub fn new(status: u16, detail: impl Into<String>) -> Self {
        Self {
            status,
            detail: detail.into(),
            instance: String::new(),
            fields: None,
            parameter: None,
            method: None,
            allowed_methods: None,
            unknown_fields: None,
            expected: None,
            received: None,
            message: None,
            trace_id: None,
            type_override: None,
        }
    }

    // ========================================================================
    // Builder methods
    // ========================================================================

    pub fn with_instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = instance.into();
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    pub fn with_fields(mut self, fields: HashMap<String, FieldError>) -> Self {
        self.fields = Some(fields);
        self
    }

    pub fn with_field_error(mut self, field_name: impl Into<String>, error: FieldError) -> Self {
        let fields = self.fields.get_or_insert_with(HashMap::new);
        fields.insert(field_name.into(), error);
        self
    }

    pub fn with_parameter(mut self, parameter: ParameterError) -> Self {
        self.parameter = Some(parameter);
        self
    }

    pub fn with_method(mut self, method: impl Into<String>) -> Self {
        self.method = Some(method.into());
        self
    }

    pub fn with_allowed_methods(mut self, methods: Vec<String>) -> Self {
        self.allowed_methods = Some(methods);
        self
    }

    pub fn with_unknown_fields(mut self, fields: Vec<String>) -> Self {
        self.unknown_fields = Some(fields);
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

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }

    pub fn with_type(mut self, error_type: impl Into<String>) -> Self {
        self.type_override = Some(error_type.into());
        self
    }

    // kept for backward compat — wraps errors into empty field_name entries
    pub fn with_errors(mut self, errors: Vec<FieldError>) -> Self {
        let mut fields = HashMap::new();
        for (i, err) in errors.into_iter().enumerate() {
            fields.insert(format!("field_{}", i), err);
        }
        self.fields = Some(fields);
        self
    }

    // ========================================================================
    // Serialization
    // ========================================================================

    /// Convert to JSON string matching the router-error.md spec exactly.
    pub fn to_json(&self) -> String {
        let mut obj = Map::new();

        // Required fields (always present)
        let error_type = self
            .type_override
            .as_deref()
            .unwrap_or_else(|| error_type_for_status(self.status));
        obj.insert("type".to_string(), json!(error_type));
        obj.insert("title".to_string(), json!(title_for_status(self.status)));
        obj.insert("status".to_string(), json!(self.status));
        obj.insert("detail".to_string(), json!(self.detail));
        obj.insert("instance".to_string(), json!(self.instance));

        // Optional fields — only included when set
        if let Some(ref method) = self.method {
            obj.insert("method".to_string(), json!(method));
        }

        if let Some(ref allowed_methods) = self.allowed_methods {
            obj.insert("allowed_methods".to_string(), json!(allowed_methods));
        }

        if let Some(ref fields) = self.fields {
            let fields_obj: Map<String, Value> = fields
                .iter()
                .map(|(k, v)| (k.clone(), v.to_json_value()))
                .collect();
            obj.insert("fields".to_string(), Value::Object(fields_obj));
        }

        if let Some(ref parameter) = self.parameter {
            obj.insert("parameter".to_string(), parameter.to_json_value());
        }

        if let Some(ref unknown_fields) = self.unknown_fields {
            obj.insert("unknown_fields".to_string(), json!(unknown_fields));
        }

        if let Some(ref expected) = self.expected {
            obj.insert("expected".to_string(), json!(expected));
        }

        if let Some(ref received) = self.received {
            obj.insert("received".to_string(), json!(received));
        }

        if let Some(ref message) = self.message {
            obj.insert("message".to_string(), json!(message));
        }

        if let Some(ref trace_id) = self.trace_id {
            obj.insert("trace_id".to_string(), json!(trace_id));
        }

        serde_json::to_string(&Value::Object(obj)).unwrap_or_else(|_| {
            let et = self
                .type_override
                .as_deref()
                .unwrap_or_else(|| error_type_for_status(self.status));
            format!(
                r#"{{"type":"{}","title":"{}","status":{},"detail":"{}","instance":"{}"}}"#,
                et,
                title_for_status(self.status),
                self.status,
                self.detail.replace('"', "\\\""),
                self.instance.replace('"', "\\\"")
            )
        })
    }

    pub fn status_code(&self) -> u16 {
        self.status
    }

    // ========================================================================
    // Convenience constructors
    // ========================================================================

    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self::new(400, detail)
    }

    pub fn unauthorized() -> Self {
        Self::new(401, "Authentication credentials are missing or invalid")
    }

    pub fn forbidden() -> Self {
        Self::new(403, "You do not have permission to access this resource")
    }

    pub fn not_found(detail: impl Into<String>) -> Self {
        Self::new(404, detail)
    }

    pub fn method_not_allowed(
        method: impl Into<String>,
        path: impl Into<String>,
        allowed: Vec<String>,
    ) -> Self {
        let m = method.into();
        let p = path.into();
        Self::new(405, "The requested method is not allowed for this route")
            .with_instance(p)
            .with_method(m)
            .with_allowed_methods(allowed)
    }

    pub fn conflict(detail: impl Into<String>) -> Self {
        Self::new(409, detail)
    }

    pub fn validation_error(fields: HashMap<String, FieldError>) -> Self {
        let detail = if fields.len() > 1 {
            "Multiple fields failed validation"
        } else {
            "One or more fields failed validation"
        };
        Self::new(422, detail).with_fields(fields)
    }

    pub fn rate_limited() -> Self {
        Self::new(429, "Too many requests")
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        Self::new(500, detail)
    }

    pub fn service_unavailable(detail: impl Into<String>) -> Self {
        Self::new(503, detail)
    }

    // ========================================================================
    // Specialized constructors matching router-error.md
    // ========================================================================

    pub fn route_not_found(method: &str, path: &str) -> Self {
        Self::not_found("The requested route does not exist")
            .with_instance(path)
            .with_method(method)
    }

    pub fn malformed_json(instance: &str) -> Self {
        Self::bad_request("Invalid JSON").with_instance(instance)
    }

    pub fn missing_content_type(instance: &str) -> Self {
        Self::bad_request("Content-Type header required for POST/PUT requests")
            .with_instance(instance)
            .with_expected("application/json")
    }

    pub fn wrong_content_type(instance: &str, received: &str) -> Self {
        Self::bad_request("Invalid Content-Type header")
            .with_instance(instance)
            .with_expected("application/json")
            .with_received(received)
    }

    pub fn body_type_mismatch(instance: &str, fields: HashMap<String, FieldError>) -> Self {
        Self::bad_request("Type mismatch in request body")
            .with_instance(instance)
            .with_fields(fields)
            .with_type("validation_error")
    }

    pub fn missing_required_field(instance: &str, fields: HashMap<String, FieldError>) -> Self {
        Self::bad_request("Required field missing in request body")
            .with_instance(instance)
            .with_fields(fields)
            .with_type("validation_error")
    }

    pub fn unknown_fields_error(instance: &str, unknown: Vec<String>) -> Self {
        Self::bad_request("Unknown fields in request body")
            .with_instance(instance)
            .with_unknown_fields(unknown)
    }

    pub fn invalid_path_param(instance: &str, param: ParameterError) -> Self {
        Self::bad_request("Invalid path parameter type")
            .with_instance(instance)
            .with_parameter(param)
            .with_type("validation_error")
    }

    pub fn missing_path_param(instance: &str, param: ParameterError) -> Self {
        Self::bad_request("Path parameter not found")
            .with_instance(instance)
            .with_parameter(param)
            .with_type("validation_error")
    }

    pub fn invalid_query_param(instance: &str, param: ParameterError) -> Self {
        Self::bad_request("Invalid query parameter type")
            .with_instance(instance)
            .with_parameter(param)
            .with_type("validation_error")
    }

    pub fn missing_query_param(instance: &str, param: ParameterError) -> Self {
        Self::bad_request("Required query parameter missing")
            .with_instance(instance)
            .with_parameter(param)
            .with_type("validation_error")
    }

    pub fn unauthorized_with_message(instance: &str, message: &str) -> Self {
        Self::unauthorized()
            .with_instance(instance)
            .with_message(message)
    }

    pub fn forbidden_with_message(instance: &str, message: &str) -> Self {
        Self::forbidden()
            .with_instance(instance)
            .with_message(message)
    }

    pub fn internal_with_trace(instance: &str, trace_id: &str) -> Self {
        Self::internal("An unexpected error occurred")
            .with_instance(instance)
            .with_trace_id(trace_id)
    }
}
