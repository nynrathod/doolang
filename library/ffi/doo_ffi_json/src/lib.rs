//! JSON FFI Functions — Production-Grade, DooCloud-Safe
//!
//! Provides JSON serialization/parsing functions for the Doo compiler.
//! Uses serde_json for parsing but returns values in Doo's native memory layout.
//!
//! MEMORY MODEL: Pure Ownership/Borrow - No RC, No GC
//! All allocations use doo_alloc_* from memory.rs (single source of truth).
//! CRITICAL: Never use CString::into_raw() - it uses Rust allocator causing heap corruption.
//! CRITICAL: Never return null_mut() for collection types - always return valid empty collection.
//!
//! SAFETY:
//! - All extern "C" fn boundaries are wrapped with catch_unwind to prevent UB on panic
//! - JSON input size is limited to MAX_JSON_SIZE to prevent OOM attacks
//! - Recursive parsing has MAX_NESTING_DEPTH to prevent stack overflow
//! - NaN/Infinity floats are serialized as null (JSON spec compliant)
//! - Integer/float formatting uses itoa/ryu for zero-allocation performance

// Sub-modules
mod convert;
mod parser;
mod writer;

// Re-export all public FFI symbols so they remain visible at crate root
pub use convert::*;
pub use parser::*;
pub use writer::*;

use doo_ffi_core::ffi_debug;
use std::cell::RefCell;
use std::os::raw::c_char;

use doo_ffi_core::memory::{
    doo_alloc, doo_alloc_empty_string, doo_alloc_string, MIN_ALLOCATION_SIZE,
};
use doo_ffi_core::rfc7807::{FieldError, Rfc7807Error};

// ============================================================================
// Security Constants — Single Source of Truth
// ============================================================================

/// Maximum JSON input size in bytes (1 MB). Prevents OOM from JSON bombs.
/// Configurable via DOO_MAX_JSON_SIZE env var at runtime.
const MAX_JSON_SIZE: usize = 1_048_576;

/// Maximum JSON nesting depth. Prevents stack overflow from deeply nested JSON.
pub(crate) const MAX_NESTING_DEPTH: usize = 64;

/// Helper: check JSON size limit before parsing
#[inline]
pub(crate) fn check_json_size(s: &str) -> Result<(), String> {
    if s.len() > MAX_JSON_SIZE {
        Err(format!(
            "JSON too large: {} bytes (max {})",
            s.len(),
            MAX_JSON_SIZE
        ))
    } else {
        Ok(())
    }
}

/// Helper: safe JSON parse with size limit
#[inline]
pub(crate) fn safe_json_parse(s: &str) -> Result<serde_json::Value, String> {
    check_json_size(s)?;
    serde_json::from_str(s).map_err(|e| format!("Invalid JSON: {}", e))
}

// ============================================================================
// Parse Error State (Thread-Local) - Uses RFC 7807 format
// ============================================================================

// Thread-local storage for JSON parse errors.
// Uses RFC 7807 format for consistency across all FFI modules.
thread_local! {
    static PARSE_ERROR: RefCell<Option<Rfc7807Error>> = const { RefCell::new(None) };
}

/// Set a parse error (RFC 7807 format)
pub(crate) fn set_parse_error_rfc7807(field: &str, expected: &str, received: &str) {
    let error = Rfc7807Error::new(400, "Bad Request")
        .with_detail(format!(
            "Type mismatch at '{}': expected {}, got {}",
            field, expected, received
        ))
        .with_errors(vec![FieldError::type_mismatch(field, expected, received)]);
    PARSE_ERROR.with(|e: &RefCell<Option<Rfc7807Error>>| {
        *e.borrow_mut() = Some(error);
    });
}

/// Clear the parse error
#[no_mangle]
pub extern "C" fn doo_json_clear_parse_error() {
    PARSE_ERROR.with(|e: &RefCell<Option<Rfc7807Error>>| {
        *e.borrow_mut() = None;
    });
}

/// Check if there's a parse error
#[no_mangle]
pub extern "C" fn doo_json_has_parse_error() -> i32 {
    PARSE_ERROR.with(|e: &RefCell<Option<Rfc7807Error>>| e.borrow().is_some() as i32)
}

/// Get the parse error status (0 if no error)
#[no_mangle]
pub extern "C" fn doo_json_get_parse_error_status() -> i32 {
    PARSE_ERROR.with(|e: &RefCell<Option<Rfc7807Error>>| {
        e.borrow()
            .as_ref()
            .map(|err| err.status as i32)
            .unwrap_or(0)
    })
}

/// Get the parse error as RFC 7807 JSON string (empty string if no error)
/// OWNERSHIP: Caller owns the returned string
#[no_mangle]
pub extern "C" fn doo_json_get_parse_error_json() -> *mut c_char {
    PARSE_ERROR.with(|e: &RefCell<Option<Rfc7807Error>>| {
        e.borrow()
            .as_ref()
            .map(|err: &Rfc7807Error| doo_alloc_string(&err.to_json()))
            .unwrap_or_else(doo_alloc_empty_string)
    })
}

/// Helper to get JSON value type as string
pub(crate) fn json_value_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "Bool",
        serde_json::Value::Number(n) => {
            if n.is_i64() {
                "Int"
            } else {
                "Float"
            }
        }
        serde_json::Value::String(_) => "Str",
        serde_json::Value::Array(_) => "Array",
        serde_json::Value::Object(_) => "Object",
    }
}

/// Allocate an empty array with proper header [len=0, cap=0][data...]
/// Returns pointer to data section (header is at ptr-16)
/// CRITICAL: Never returns null - crashes calling code
#[inline]
pub(crate) fn alloc_empty_array() -> *mut u8 {
    ffi_debug!(
        "FFI",
        "alloc_empty_array: allocating {} bytes",
        MIN_ALLOCATION_SIZE
    );
    let ptr = doo_alloc(MIN_ALLOCATION_SIZE);
    if ptr.is_null() {
        ffi_debug!(
            "FFI",
            "CRITICAL: alloc_empty_array got null from doo_alloc!"
        );
        std::process::abort();
    }
    ffi_debug!("FFI", "alloc_empty_array: got ptr={:p}", ptr);
    unsafe {
        *(ptr as *mut i64) = 0; // length
        *(ptr.add(8) as *mut i64) = 0; // capacity
        let data_ptr = ptr.add(16);
        ffi_debug!(
            "FFI",
            "alloc_empty_array: header={:p}, data={:p}, len=0, cap=0",
            ptr,
            data_ptr
        );
        data_ptr
    }
}

/// Allocate an empty map with proper header [len=0, cap=0][data...]
/// Returns pointer to data section (header is at ptr-16)
/// CRITICAL: Never returns null - crashes calling code
#[inline]
pub(crate) fn alloc_empty_map() -> *mut u8 {
    alloc_empty_array() // Same layout as array
}
