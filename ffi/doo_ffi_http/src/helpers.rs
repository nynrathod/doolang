//! Helper Functions
//! String conversion, memory allocation, and utility functions.
//!
//! Memory Management Strategy:
//! - All allocations use Rust's Box when possible, converting to raw pointers at FFI boundary
//! - For C-compatible structs, we use repr(C) and Box::into_raw for ownership transfer
//! - Caller receives ownership and is responsible for freeing via corresponding free functions

use std::ffi::c_void;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

// ============================================================================
// CONSTANTS (Single Source of Truth)
// ============================================================================

/// Standard JSON content type for all API responses
pub const CONTENT_TYPE_JSON: &str = "application/json";

/// Size of the error response struct: { i32 status (4) + padding (4) + ptr body (8) + ptr content_type (8) }
pub const ERROR_RESPONSE_SIZE: usize = 24;

// ============================================================================
// ERROR RESPONSE STRUCT
// Layout matches codegen expectations: { i32 status, ptr body, ptr content_type }
// This is the single source of truth for error response structure
// ============================================================================

/// Represents the raw error response struct layout expected by codegen
/// Memory layout: { i32 status, padding[4], *const char body, *const char content_type }
#[repr(C)]
pub struct RawErrorResponse {
    pub status: i32,
    _padding: i32,
    pub body: *const c_char,
    pub content_type: *const c_char,
}

impl RawErrorResponse {
    /// Create a new RawErrorResponse with proper ownership transfer
    /// The strings are allocated with CString and leaked for C compatibility
    /// Ownership transfers to the returned struct
    /// Body is automatically formatted as JSON if it's not already
    pub fn new(status: i32, body: &str) -> Self {
        // Format body as JSON if it's not already JSON
        let json_body = if body.starts_with('{') || body.starts_with('[') {
            body.to_string()
        } else {
            // Format plain message as RFC 7807-style JSON
            serde_json::json!({
                "type": "about:blank",
                "title": status_to_title(status),
                "status": status,
                "detail": body
            })
            .to_string()
        };

        Self {
            status,
            _padding: 0,
            body: string_to_c(&json_body),
            content_type: string_to_c(CONTENT_TYPE_JSON),
        }
    }

    /// Convert to raw pointer, transferring ownership to caller
    /// Caller must free via `free_error_response`
    pub fn into_raw(self) -> *mut c_void {
        Box::into_raw(Box::new(self)) as *mut c_void
    }
}

/// Convert HTTP status code to standard title
fn status_to_title(status: i32) -> &'static str {
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
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

/// Allocate and populate an error response struct
/// Returns a pointer to: { i32 status, *const char body, *const char content_type }
/// Caller receives ownership and is responsible for freeing this memory
#[inline]
pub fn alloc_error_response(status: i32, body: &str) -> *mut c_void {
    RawErrorResponse::new(status, body).into_raw()
}

/// Free an error response allocated by alloc_error_response
/// Safety: ptr must have been allocated by alloc_error_response
#[no_mangle]
pub unsafe extern "C" fn free_error_response(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    let response = Box::from_raw(ptr as *mut RawErrorResponse);
    // Free the strings
    if !response.body.is_null() {
        let _ = CString::from_raw(response.body as *mut c_char);
    }
    if !response.content_type.is_null() {
        let _ = CString::from_raw(response.content_type as *mut c_char);
    }
    // Box drops here, freeing the struct
}

/// Convert C string to Rust String (borrows, does not take ownership)
pub fn c_to_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().to_string() }
}

/// Convert Rust string to C string
/// Uses CString for proper memory management, leaks for FFI transfer
/// Caller receives ownership and must free via CString::from_raw or libc::free
#[inline]
pub fn string_to_c(s: &str) -> *const c_char {
    match CString::new(s) {
        Ok(cstr) => cstr.into_raw() as *const c_char,
        Err(_) => {
            // String contains null bytes, allocate empty string
            CString::new("").unwrap().into_raw() as *const c_char
        }
    }
}

/// Free a string allocated by string_to_c
/// Safety: ptr must have been allocated by string_to_c
#[no_mangle]
pub unsafe extern "C" fn free_c_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        let _ = CString::from_raw(ptr);
    }
}

/// Parse query string into HashMap
pub fn parse_query(query: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            map.insert(
                urlencoding::decode(key).unwrap_or_default().to_string(),
                urlencoding::decode(value).unwrap_or_default().to_string(),
            );
        }
    }
    map
}

/// Thread-local storage for current request path (for RFC 7807 instance field)
thread_local! {
    static CURRENT_REQUEST_PATH: std::cell::RefCell<String> = std::cell::RefCell::new("/".to_string());
    static LAST_ERROR_STATUS: std::cell::Cell<i32> = std::cell::Cell::new(0);
    static LAST_ERROR_JSON: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
}

pub fn set_current_request_path(path: &str) {
    CURRENT_REQUEST_PATH.with(|p| *p.borrow_mut() = path.to_string());
}

pub fn get_current_request_path() -> String {
    CURRENT_REQUEST_PATH.with(|p| p.borrow().clone())
}

pub fn set_last_error(status: i32, json: String) {
    LAST_ERROR_STATUS.with(|s| s.set(status));
    LAST_ERROR_JSON.with(|j| *j.borrow_mut() = json);
}

pub fn get_last_error_status() -> i32 {
    LAST_ERROR_STATUS.with(|s| s.get())
}

pub fn get_last_error_json() -> String {
    LAST_ERROR_JSON.with(|j| j.borrow().clone())
}

pub fn clear_last_error() {
    set_last_error(0, String::new());
}
