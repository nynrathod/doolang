//! FFI Safety Macros — Single Source of Truth
//!
//! Every `extern "C"` fn MUST NOT unwind into C/LLVM code — UB on all platforms.
//! These macros provide zero-overhead panic safety wrappers for all return types.
//!
//! ## Usage
//!
//! ```ignore
//! use doo_ffi_core::{ffi_safe_result, ffi_safe_ptr, ffi_safe_void};
//!
//! #[no_mangle]
//! pub extern "C" fn my_fn() -> *mut DooResult {
//!     ffi_safe_result!({ do_work() })
//! }
//! ```
//!
//! All FFI crates should use these macros instead of rolling their own
//! `catch_unwind` wrappers.

/// Wrap an extern "C" fn that returns `*mut DooResult`.
/// On panic → returns a 500 error DooResult with panic message.
#[macro_export]
macro_rules! ffi_safe_result {
    ($body:expr) => {{
        match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| $body)) {
            Ok(r) => r,
            Err(payload) => $crate::make_panic_err("FFI", payload),
        }
    }};
    ($component:expr, $body:expr) => {{
        match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| $body)) {
            Ok(r) => r,
            Err(payload) => $crate::make_panic_err($component, payload),
        }
    }};
}

/// Wrap an extern "C" fn that returns `*mut c_void` or `*const c_void`.
/// On panic → returns null.
#[macro_export]
macro_rules! ffi_safe_ptr {
    ($body:expr) => {{
        match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| $body)) {
            Ok(r) => r,
            Err(_) => ::std::ptr::null_mut(),
        }
    }};
}

/// Wrap an extern "C" fn that returns `*const c_char`.
/// On panic → returns null.
#[macro_export]
macro_rules! ffi_safe_cstr {
    ($body:expr) => {{
        match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| $body)) {
            Ok(r) => r,
            Err(_) => ::std::ptr::null(),
        }
    }};
}

/// Wrap an extern "C" fn that returns `i32`.
/// On panic → returns -1 (error sentinel).
#[macro_export]
macro_rules! ffi_safe_i32 {
    ($body:expr) => {{
        match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| $body)) {
            Ok(r) => r,
            Err(_) => -1i32,
        }
    }};
}

/// Wrap an extern "C" fn that returns `i64`.
/// On panic → returns 0 (safe default).
#[macro_export]
macro_rules! ffi_safe_i64 {
    ($body:expr) => {{
        match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| $body)) {
            Ok(r) => r,
            Err(_) => 0i64,
        }
    }};
}

/// Wrap an extern "C" fn that returns `f64`.
/// On panic → returns 0.0.
#[macro_export]
macro_rules! ffi_safe_f64 {
    ($body:expr) => {{
        match ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| $body)) {
            Ok(r) => r,
            Err(_) => 0.0f64,
        }
    }};
}

/// Wrap an extern "C" fn that returns void.
/// On panic → silently absorb.
#[macro_export]
macro_rules! ffi_safe_void {
    ($body:expr) => {{
        let _ = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| $body));
    }};
}
