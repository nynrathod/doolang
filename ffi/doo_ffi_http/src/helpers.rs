//! Helper Functions
//! String conversion, memory allocation, and utility functions.
//!
//! Memory Management Strategy:
//! - ALL string allocations use doo_alloc_string (libc::malloc) from doo_ffi_core
//! - NEVER use CString::into_raw() — it uses Rust's allocator, causing heap corruption
//! - For C-compatible structs, we use repr(C) and Box::into_raw for ownership transfer
//! - Caller receives ownership and is responsible for freeing via corresponding free functions

use std::ffi::c_void;
use std::os::raw::c_char;

// Re-export core helpers for use across this crate
pub use doo_ffi_core::helpers::{c_to_string_lossy, safe_ffi};

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
    /// All strings allocated via doo_alloc_string (libc::malloc) — NO CString!
    /// Body is automatically formatted as JSON if it's not already
    pub fn new(status: i32, body: &str) -> Self {
        // Format body as JSON if it's not already JSON
        let json_body = if body.starts_with('{') || body.starts_with('[') {
            body.to_string()
        } else {
            // Format plain message as RFC 7807-style JSON using centralized type/title
            let s = status as u16;
            serde_json::json!({
                "type": doo_ffi_core::error_type_for_status(s),
                "title": doo_ffi_core::title_for_status(s),
                "status": status,
                "detail": body
            })
            .to_string()
        };

        Self {
            status,
            _padding: 0,
            body: doo_ffi_core::helpers::string_to_c(&json_body),
            content_type: doo_ffi_core::helpers::string_to_c(CONTENT_TYPE_JSON),
        }
    }

    /// Convert to raw pointer, transferring ownership to caller
    /// Caller must free via `free_error_response`
    pub fn into_raw(self) -> *mut c_void {
        Box::into_raw(Box::new(self)) as *mut c_void
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
/// Strings freed via libc::free (matching doo_alloc_string allocation)
#[no_mangle]
pub unsafe extern "C" fn free_error_response(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    let response = Box::from_raw(ptr as *mut RawErrorResponse);
    // Free strings allocated with doo_alloc_string (libc::malloc)
    if !response.body.is_null() {
        doo_ffi_core::doo_free(response.body as *mut u8);
    }
    if !response.content_type.is_null() {
        doo_ffi_core::doo_free(response.content_type as *mut u8);
    }
    // Box drops here, freeing the struct
}

/// Convert C string to Rust String (borrows, does not take ownership).
/// Lossy: null → empty string, invalid UTF-8 → replacement chars.
/// Delegates to centralized core helper.
pub fn c_to_string(ptr: *const c_char) -> String {
    doo_ffi_core::helpers::c_to_string_lossy(ptr)
}

/// Convert Rust string to C string.
/// Uses doo_alloc_string (libc::malloc) — consistent allocator.
/// Caller owns the returned pointer and must free via doo_free/libc::free.
/// CRITICAL: NEVER use CString::into_raw() — allocator mismatch causes heap corruption.
#[inline]
pub fn string_to_c(s: &str) -> *const c_char {
    doo_ffi_core::helpers::string_to_c(s)
}

/// Free a string allocated by string_to_c (via doo_alloc_string → libc::malloc).
/// Uses libc::free — matching the allocator used by doo_alloc_string.
#[no_mangle]
pub unsafe extern "C" fn free_c_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        doo_ffi_core::doo_free(ptr as *mut u8);
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
