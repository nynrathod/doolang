//! Print/debug runtime — Tier A functions for stdout output.
//!
//! These are called by the codegen as generic @extern calls.
//! The compiler only knows the symbol names (in ffi_names.rs),
//! not what they do — same as doo_alloc, doo_free.

use std::io::Write;

/// Print a null-terminated C string to stdout. No newline.
#[no_mangle]
pub extern "C" fn doo_print_str(s: *const u8) {
    if s.is_null() {
        return;
    }
    // SAFETY: caller guarantees s is a valid null-terminated C string
    unsafe {
        let cs = std::ffi::CStr::from_ptr(s as *const i8);
        if let Ok(str) = cs.to_str() {
            let mut stdout = std::io::stdout();
            let _ = stdout.write_all(str.as_bytes());
        }
    }
}

/// Print a newline to stdout and flush.
#[no_mangle]
pub extern "C" fn doo_println() {
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(b"\n");
    let _ = stdout.flush();
}

/// Flush stdout — called at program exit to ensure all output is written.
#[no_mangle]
pub extern "C" fn doo_flush() {
    let mut stdout = std::io::stdout();
    let _ = stdout.flush();
}

// ============================================================================
// Value-to-String Conversion Functions
//
// These convert Doo primitives to heap-allocated null-terminated strings.
// The caller owns the returned pointer and is responsible for freeing it.
// Used by the Cast-to-Str instruction in the codegen — same as how Rust's
// std::fmt converts values to strings via the Display trait.
// ============================================================================

/// Convert an integer to a heap-allocated string.
#[no_mangle]
pub extern "C" fn doo_int_to_str(n: i64) -> *mut u8 {
    let s = n.to_string();
    let cs = std::ffi::CString::new(s).unwrap_or_else(|_| std::ffi::CString::new("<int>").unwrap());
    cs.into_raw() as *mut u8
}

/// Convert a float to a heap-allocated string.
#[no_mangle]
pub extern "C" fn doo_float_to_str(f: f64) -> *mut u8 {
    let s = if f == f.floor() && f.is_finite() {
        // Whole number: show without trailing zeros (e.g. "3" not "3.0")
        // But still append .0 for type clarity
        format!("{:.1}", f)
    } else {
        format!("{}", f)
    };
    let cs =
        std::ffi::CString::new(s).unwrap_or_else(|_| std::ffi::CString::new("<float>").unwrap());
    cs.into_raw() as *mut u8
}

/// Convert a boolean to a heap-allocated string.
#[no_mangle]
pub extern "C" fn doo_bool_to_str(b: i32) -> *mut u8 {
    let s = if b != 0 { "true" } else { "false" };
    let cs = std::ffi::CString::new(s).unwrap();
    cs.into_raw() as *mut u8
}

/// Format a null pointer as "<null>".
#[no_mangle]
pub extern "C" fn doo_null_to_str() -> *mut u8 {
    let cs = std::ffi::CString::new("<null>").unwrap();
    cs.into_raw() as *mut u8
}

/// Free a string previously allocated by doo_int_to_str, doo_float_to_str, etc.
#[no_mangle]
pub extern "C" fn doo_str_free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: ptr was created by CString::into_raw in one of the doo_*_to_str functions
    unsafe {
        let _ = std::ffi::CString::from_raw(ptr as *mut i8);
    }
}
