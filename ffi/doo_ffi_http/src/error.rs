//! RFC 7807 Error System
//! Centralized error response generation following RFC 7807 Problem Details.

use serde::{Serialize, Deserialize};
use std::collections::HashMap;

/// Error type enum matching RFC 7807 type URIs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ErrorType {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    Conflict,
    UnprocessableEntity,
    TooManyRequests,
    InternalServerError,
    NotImplemented,
    BadGateway,
    ServiceUnavailable,
}

impl ErrorType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::MethodNotAllowed => "method_not_allowed",
            Self::Conflict => "conflict",
            Self::UnprocessableEntity => "unprocessable_entity",
            Self::TooManyRequests => "too_many_requests",
            Self::InternalServerError => "internal_server_error",
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
            Self::UnprocessableEntity => "Unprocessable Entity",
            Self::TooManyRequests => "Too Many Requests",
            Self::InternalServerError => "Internal Server Error",
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
            Self::InternalServerError => 500,
            Self::NotImplemented => 501,
            Self::BadGateway => 502,
            Self::ServiceUnavailable => 503,
        }
    }
}

/// Field-level validation error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldError {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub received: Option<String>,
}

impl FieldError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            rule: None,
            message: message.into(),
            value: None,
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
}

/// RFC 7807 Error Response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    #[serde(rename = "type")]
    pub error_type: String,
    pub title: String,
    pub status: u16,
    pub detail: String,
    pub instance: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub fields: HashMap<String, FieldError>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unknown_fields: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub allowed_methods: Vec<String>,
}

impl ErrorResponse {
    pub fn new(error_type: ErrorType, detail: impl Into<String>, instance: impl Into<String>) -> Self {
        Self {
            error_type: error_type.as_str().to_string(),
            title: error_type.title().to_string(),
            status: error_type.status_code(),
            detail: detail.into(),
            instance: instance.into(),
            method: None,
            fields: HashMap::new(),
            unknown_fields: Vec::new(),
            allowed_methods: Vec::new(),
        }
    }
    
    pub fn with_field(mut self, name: impl Into<String>, error: FieldError) -> Self {
        self.fields.insert(name.into(), error);
        self
    }
    
    pub fn with_method(mut self, method: impl Into<String>) -> Self {
        self.method = Some(method.into());
        self
    }
    
    pub fn with_allowed_methods(mut self, methods: Vec<String>) -> Self {
        self.allowed_methods = methods;
        self
    }
    
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

// Convenience constructors
pub fn bad_request(detail: impl Into<String>, instance: impl Into<String>) -> ErrorResponse {
    ErrorResponse::new(ErrorType::BadRequest, detail, instance)
}

pub fn unauthorized(detail: impl Into<String>, instance: impl Into<String>) -> ErrorResponse {
    ErrorResponse::new(ErrorType::Unauthorized, detail, instance)
}

pub fn forbidden(detail: impl Into<String>, instance: impl Into<String>) -> ErrorResponse {
    ErrorResponse::new(ErrorType::Forbidden, detail, instance)
}

pub fn not_found(detail: impl Into<String>, instance: impl Into<String>) -> ErrorResponse {
    ErrorResponse::new(ErrorType::NotFound, detail, instance)
}

pub fn conflict(detail: impl Into<String>, instance: impl Into<String>) -> ErrorResponse {
    ErrorResponse::new(ErrorType::Conflict, detail, instance)
}

pub fn internal_error(detail: impl Into<String>, instance: impl Into<String>) -> ErrorResponse {
    ErrorResponse::new(ErrorType::InternalServerError, detail, instance)
}

pub fn too_many_requests(detail: impl Into<String>, instance: impl Into<String>) -> ErrorResponse {
    ErrorResponse::new(ErrorType::TooManyRequests, detail, instance)
}

pub fn validation_error(detail: impl Into<String>, instance: impl Into<String>, fields: HashMap<String, FieldError>) -> ErrorResponse {
    let mut err = ErrorResponse::new(ErrorType::UnprocessableEntity, detail, instance);
    err.fields = fields;
    err
}

pub fn method_not_allowed(detail: impl Into<String>, instance: impl Into<String>, allowed: Vec<String>) -> ErrorResponse {
    ErrorResponse::new(ErrorType::MethodNotAllowed, detail, instance)
        .with_allowed_methods(allowed)
}
