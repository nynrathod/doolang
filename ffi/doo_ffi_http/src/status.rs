//! HTTP Status Codes
//!
//! Centralized HTTP status codes for all responses.

use serde::{Deserialize, Serialize};

/// HTTP status code enum (centralized).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u16)]
pub enum HttpStatus {
    // Success 2xx
    Ok = 200,
    Created = 201,
    Accepted = 202,
    NoContent = 204,
    
    // Redirect 3xx
    MovedPermanently = 301,
    Found = 302,
    SeeOther = 303,
    NotModified = 304,
    TemporaryRedirect = 307,
    PermanentRedirect = 308,
    
    // Client Error 4xx
    BadRequest = 400,
    Unauthorized = 401,
    Forbidden = 403,
    NotFound = 404,
    MethodNotAllowed = 405,
    Conflict = 409,
    Gone = 410,
    UnprocessableEntity = 422,
    TooManyRequests = 429,
    
    // Server Error 5xx
    InternalServerError = 500,
    NotImplemented = 501,
    BadGateway = 502,
    ServiceUnavailable = 503,
}

impl HttpStatus {
    /// Get status code as u16.
    pub fn code(self) -> u16 {
        self as u16
    }

    /// Get reason phrase.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Created => "Created",
            Self::Accepted => "Accepted",
            Self::NoContent => "No Content",
            Self::MovedPermanently => "Moved Permanently",
            Self::Found => "Found",
            Self::SeeOther => "See Other",
            Self::NotModified => "Not Modified",
            Self::TemporaryRedirect => "Temporary Redirect",
            Self::PermanentRedirect => "Permanent Redirect",
            Self::BadRequest => "Bad Request",
            Self::Unauthorized => "Unauthorized",
            Self::Forbidden => "Forbidden",
            Self::NotFound => "Not Found",
            Self::MethodNotAllowed => "Method Not Allowed",
            Self::Conflict => "Conflict",
            Self::Gone => "Gone",
            Self::UnprocessableEntity => "Unprocessable Entity",
            Self::TooManyRequests => "Too Many Requests",
            Self::InternalServerError => "Internal Server Error",
            Self::NotImplemented => "Not Implemented",
            Self::BadGateway => "Bad Gateway",
            Self::ServiceUnavailable => "Service Unavailable",
        }
    }

    /// Check if status is success (2xx).
    pub fn is_success(self) -> bool {
        (200..300).contains(&self.code())
    }

    /// Check if status is client error (4xx).
    pub fn is_client_error(self) -> bool {
        (400..500).contains(&self.code())
    }

    /// Check if status is server error (5xx).
    pub fn is_server_error(self) -> bool {
        (500..600).contains(&self.code())
    }
}

/// HTTP method enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HttpMethod {
    GET,
    POST,
    PUT,
    PATCH,
    DELETE,
    OPTIONS,
    HEAD,
}

impl HttpMethod {
    /// Parse from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "GET" => Some(Self::GET),
            "POST" => Some(Self::POST),
            "PUT" => Some(Self::PUT),
            "PATCH" => Some(Self::PATCH),
            "DELETE" => Some(Self::DELETE),
            "OPTIONS" => Some(Self::OPTIONS),
            "HEAD" => Some(Self::HEAD),
            _ => None,
        }
    }

    /// Convert to string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GET => "GET",
            Self::POST => "POST",
            Self::PUT => "PUT",
            Self::PATCH => "PATCH",
            Self::DELETE => "DELETE",
            Self::OPTIONS => "OPTIONS",
            Self::HEAD => "HEAD",
        }
    }
}

/// Content type enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    Json,
    Html,
    Text,
    FormUrlEncoded,
    MultipartFormData,
}

impl ContentType {
    /// Get MIME type string.
    pub fn mime(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Html => "text/html",
            Self::Text => "text/plain",
            Self::FormUrlEncoded => "application/x-www-form-urlencoded",
            Self::MultipartFormData => "multipart/form-data",
        }
    }
}
