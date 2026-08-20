//! JSON Writer — serialization functions for Doo's JSON FFI.
//!
//! Provides a streaming JSON writer with zero-allocation number formatting
//! via itoa/ryu. NaN/Infinity floats are serialized as null (JSON spec compliant).
//!
//! PERFORMANCE: Internal buffer is allocated via doo_alloc (libc::malloc), so
//! finish() can return the buffer directly without a secondary allocation.
//! This eliminates malloc+memcpy+free per JSON serialization (~40ns saved).

use doo_ffi_core::memory::{doo_alloc, doo_alloc_string, doo_free, doo_realloc};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};

// ============================================================================
// JSON Writer — libc-allocated buffer for zero-copy finish
// ============================================================================

/// Internal JSON writer with raw libc-allocated buffer.
/// Buffer allocated via doo_alloc (libc::malloc), compatible with doo_free.
/// This enables zero-copy finish: null-terminate in-place, return pointer directly.
pub struct JsonWriter {
    buf: *mut u8,
    len: usize,
    cap: usize,
}

impl JsonWriter {
    /// Create a new writer with default capacity (64 bytes).
    /// +1 byte is reserved internally for null terminator.
    pub(crate) fn new() -> Self {
        Self::with_capacity(64)
    }

    /// Create a writer with specified capacity hint.
    /// Minimum 64 bytes. +1 byte reserved for null terminator.
    pub(crate) fn with_capacity(cap: usize) -> Self {
        let cap = cap.max(64);
        let buf = doo_alloc(cap + 1); // +1 for null terminator
        Self { buf, len: 0, cap }
    }

    /// Write raw bytes into the buffer, growing if needed.
    #[inline]
    pub(crate) fn write_raw(&mut self, s: &[u8]) {
        if self.buf.is_null() {
            return; // OOM on initial alloc — silently skip
        }
        let needed = self.len + s.len();
        if needed > self.cap {
            let new_cap = needed.next_power_of_two().max(self.cap * 2);
            let new_buf = doo_realloc(self.buf, new_cap + 1);
            if new_buf.is_null() {
                return; // OOM on realloc — silently skip
            }
            self.buf = new_buf;
            self.cap = new_cap;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(s.as_ptr(), self.buf.add(self.len), s.len());
        }
        self.len += s.len();
    }
}

impl Drop for JsonWriter {
    fn drop(&mut self) {
        if !self.buf.is_null() {
            doo_free(self.buf);
            self.buf = std::ptr::null_mut();
        }
    }
}

/// Create a new JSON writer (default capacity: 64 bytes)
/// Struct allocated via doo_alloc (libc::malloc) — same allocator as buffer.
/// Eliminates Box overhead and allocator mismatch.
#[no_mangle]
pub extern "C" fn doo_json_writer_new() -> *mut JsonWriter {
    let writer = JsonWriter::new();
    let ptr = doo_alloc(std::mem::size_of::<JsonWriter>()) as *mut JsonWriter;
    if ptr.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        std::ptr::write(ptr, writer);
    }
    ptr
}

/// Create a new JSON writer with a capacity hint (avoids reallocations)
/// Used by codegen when struct field count is known at compile time.
#[no_mangle]
pub extern "C" fn doo_json_writer_new_with_cap(cap: usize) -> *mut JsonWriter {
    catch_unwind(|| {
        let writer = JsonWriter::with_capacity(cap);
        let ptr = doo_alloc(std::mem::size_of::<JsonWriter>()) as *mut JsonWriter;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        unsafe {
            std::ptr::write(ptr, writer);
        }
        ptr
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Free a JSON writer (without finishing)
/// Frees both the buffer (via Drop) and the struct (via doo_free).
#[no_mangle]
pub extern "C" fn doo_json_writer_free(writer: *mut JsonWriter) {
    if !writer.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
            std::ptr::drop_in_place(writer); // Drop impl frees the buffer
            doo_free(writer as *mut u8); // Free the struct itself
        }));
    }
}

/// Finish writing and return the JSON string (consumes writer).
/// ZERO-COPY: Null-terminates the buffer in-place and returns it directly.
/// No secondary allocation — saves ~40ns per serialization.
/// OWNERSHIP: Caller owns the returned string and must call doo_free.
/// Returns "null" JSON if writer is null (never returns null ptr).
#[no_mangle]
pub extern "C" fn doo_json_writer_finish(writer: *mut JsonWriter) -> *mut c_char {
    if writer.is_null() {
        return doo_alloc_string("null");
    }
    catch_unwind(AssertUnwindSafe(|| unsafe {
        let w = &mut *writer;

        if w.buf.is_null() {
            // OOM case — writer never had a valid buffer
            doo_free(writer as *mut u8);
            return doo_alloc_string("null");
        }

        // Null-terminate in-place — we always have room (+1 in allocation)
        *w.buf.add(w.len) = 0;
        let ptr = w.buf as *mut c_char;

        // Transfer ownership to caller — prevent Drop from freeing buffer
        w.buf = std::ptr::null_mut();
        // Now drop the struct: Drop runs (no-op since buf is null), then free struct
        std::ptr::drop_in_place(writer);
        doo_free(writer as *mut u8);

        ptr
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
        let needs_escape = bytes.iter().any(|&b| b == b'"' || b == b'\\' || b < 0x20);

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
                        w.write_raw(&[
                            b'\\',
                            b'u',
                            b'0',
                            b'0',
                            hex[(b >> 4) as usize],
                            hex[(b & 0xf) as usize],
                        ]);
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
