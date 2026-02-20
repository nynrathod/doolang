//! JSON Writer — serialization functions for Doo's JSON FFI.
//!
//! Provides a streaming JSON writer with zero-allocation number formatting
//! via itoa/ryu. NaN/Infinity floats are serialized as null (JSON spec compliant).

use doo_ffi_core::memory::doo_alloc_string;
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};

// ============================================================================
// JSON Writer
// ============================================================================

/// Internal JSON writer buffer
pub struct JsonWriter {
    buffer: Vec<u8>,
}

impl JsonWriter {
    pub(crate) fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(1024),
        }
    }
    pub(crate) fn write_raw(&mut self, s: &[u8]) {
        self.buffer.extend_from_slice(s);
    }
}

/// Create a new JSON writer
#[no_mangle]
pub extern "C" fn doo_json_writer_new() -> *mut JsonWriter {
    Box::into_raw(Box::new(JsonWriter::new()))
}

/// Create a new JSON writer with a capacity hint (avoids reallocations)
#[no_mangle]
pub extern "C" fn doo_json_writer_new_with_cap(cap: usize) -> *mut JsonWriter {
    catch_unwind(|| {
        Box::into_raw(Box::new(JsonWriter {
            buffer: Vec::with_capacity(cap.max(64)),
        }))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Free a JSON writer (without finishing)
#[no_mangle]
pub extern "C" fn doo_json_writer_free(writer: *mut JsonWriter) {
    if !writer.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
            let _ = Box::from_raw(writer);
        }));
    }
}

/// Finish writing and return the JSON string (consumes writer)
/// OWNERSHIP: Caller owns the returned string.
/// Returns "null" JSON if writer is null (never returns null ptr)
#[no_mangle]
pub extern "C" fn doo_json_writer_finish(writer: *mut JsonWriter) -> *mut c_char {
    if writer.is_null() {
        return doo_alloc_string("null");
    }
    catch_unwind(AssertUnwindSafe(|| unsafe {
        let writer_box = Box::from_raw(writer);
        // SAFETY: Buffer only contains valid UTF-8 — we write ASCII JSON syntax and
        // strings that came from valid Doo strings (via CStr). No lossy conversion needed.
        let s = std::str::from_utf8_unchecked(&writer_box.buffer);
        doo_alloc_string(s)
    }))
    .unwrap_or_else(|_| doo_alloc_string("null"))
}

/// Write object start '{'
#[no_mangle]
pub extern "C" fn doo_json_write_start_object(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b"{");
    }
}

/// Write object end '}'
#[no_mangle]
pub extern "C" fn doo_json_write_end_object(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b"}");
    }
}

/// Write array start '['
#[no_mangle]
pub extern "C" fn doo_json_write_start_array(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b"[");
    }
}

/// Write array end ']'
#[no_mangle]
pub extern "C" fn doo_json_write_end_array(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b"]");
    }
}

/// Write comma ','
#[no_mangle]
pub extern "C" fn doo_json_write_comma(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b",");
    }
}

/// Write colon ':'
#[no_mangle]
pub extern "C" fn doo_json_write_colon(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b":");
    }
}

/// Write null literal
#[no_mangle]
pub extern "C" fn doo_json_write_null(writer: *mut JsonWriter) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b"null");
    }
}

/// Write integer value (zero-allocation via itoa)
#[no_mangle]
pub extern "C" fn doo_json_write_int(writer: *mut JsonWriter, val: i64) {
    if let Some(w) = unsafe { writer.as_mut() } {
        let mut buf = itoa::Buffer::new();
        w.write_raw(buf.format(val).as_bytes());
    }
}

/// Write float value (zero-allocation via ryu, NaN/Infinity → null per JSON spec)
#[no_mangle]
pub extern "C" fn doo_json_write_float(writer: *mut JsonWriter, val: f64) {
    if let Some(w) = unsafe { writer.as_mut() } {
        if val.is_nan() || val.is_infinite() {
            w.write_raw(b"null");
        } else {
            let mut buf = ryu::Buffer::new();
            w.write_raw(buf.format(val).as_bytes());
        }
    }
}

/// Write boolean value
#[no_mangle]
pub extern "C" fn doo_json_write_bool(writer: *mut JsonWriter, val: bool) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(if val { b"true" } else { b"false" });
    }
}

/// Write string value with inline JSON escaping — zero allocation.
///
/// Instead of calling serde_json::to_string (which allocates a String),
/// this writes the quoted+escaped string directly into the writer buffer.
/// Most strings have no special chars, so the fast path is just memcpy.
#[no_mangle]
pub extern "C" fn doo_json_write_string(writer: *mut JsonWriter, val: *const c_char) {
    if let Some(w) = unsafe { writer.as_mut() } {
        if val.is_null() {
            w.write_raw(b"null");
            return;
        }
        // Read the C string directly — no intermediate String allocation.
        // SAFETY: val is a valid null-terminated C string from Doo's allocator.
        let cstr = unsafe { std::ffi::CStr::from_ptr(val) };
        let bytes = cstr.to_bytes();

        // Fast path: check if any escaping is needed (most strings don't need it)
        let needs_escape = bytes
            .iter()
            .any(|&b| b == b'"' || b == b'\\' || b < 0x20);

        w.write_raw(b"\"");
        if !needs_escape {
            // Fast path — no escaping needed, direct copy
            w.write_raw(bytes);
        } else {
            // Slow path — escape special characters inline
            for &b in bytes {
                match b {
                    b'"' => w.write_raw(b"\\\""),
                    b'\\' => w.write_raw(b"\\\\"),
                    b'\n' => w.write_raw(b"\\n"),
                    b'\r' => w.write_raw(b"\\r"),
                    b'\t' => w.write_raw(b"\\t"),
                    b if b < 0x20 => {
                        // Control character — \u00XX
                        let hex = b"0123456789abcdef";
                        w.write_raw(&[b'\\', b'u', b'0', b'0', hex[(b >> 4) as usize], hex[(b & 0xf) as usize]]);
                    }
                    _ => w.write_raw(&[b]),
                }
            }
        }
        w.write_raw(b"\"");
    }
}

/// Write string as object key (alias for doo_json_write_string)
#[no_mangle]
pub extern "C" fn doo_json_write_key(writer: *mut JsonWriter, key: *const c_char) {
    doo_json_write_string(writer, key);
}

/// Write integer as object key (quoted string, zero-alloc via itoa)
/// JSON standard requires all object keys to be strings
#[no_mangle]
pub extern "C" fn doo_json_write_key_int(writer: *mut JsonWriter, key: i64) {
    if let Some(w) = unsafe { writer.as_mut() } {
        let mut buf = itoa::Buffer::new();
        w.write_raw(b"\"");
        w.write_raw(buf.format(key).as_bytes());
        w.write_raw(b"\"");
    }
}

/// Write float as object key (quoted string, zero-alloc via ryu)
/// JSON standard requires all object keys to be strings
#[no_mangle]
pub extern "C" fn doo_json_write_key_float(writer: *mut JsonWriter, key: f64) {
    if let Some(w) = unsafe { writer.as_mut() } {
        w.write_raw(b"\"");
        if key.is_nan() || key.is_infinite() {
            w.write_raw(b"null");
        } else {
            let mut buf = ryu::Buffer::new();
            w.write_raw(buf.format(key).as_bytes());
        }
        w.write_raw(b"\"");
    }
}

/// Write bool as object key (quoted string)
/// JSON standard requires all object keys to be strings
#[no_mangle]
pub extern "C" fn doo_json_write_key_bool(writer: *mut JsonWriter, key: bool) {
    if let Some(w) = unsafe { writer.as_mut() } {
        if key {
            w.write_raw(b"\"true\"");
        } else {
            w.write_raw(b"\"false\"");
        }
    }
}
