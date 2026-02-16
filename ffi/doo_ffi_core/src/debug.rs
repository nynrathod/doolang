//! Centralized debug logging for Doo FFI libraries.
//!
//! Single source of truth for all debug output across all FFI crates.
//!
//! ## Behavior
//!
//! Debug is enabled when the `DOO_DEBUG` environment variable is set.
//! This is automatically set by `doo run --debug` and inherited by child processes.
//!
//! The env var check is performed once (lazily on first use) and cached.
//!
//! ## Usage
//!
//! ```rust
//! use doo_ffi_core::ffi_debug;
//! ffi_debug!("HTTP", "Request received: {} {}", method, path);
//! ffi_debug!("DB", "Query executed in {}ms", elapsed);
//! ```

use std::sync::OnceLock;

/// Cached debug flag. Lazily initialized from environment on first access.
static DEBUG_ENABLED: OnceLock<bool> = OnceLock::new();

/// Check if debug mode is enabled. Result is cached after first call.
///
/// Returns `true` if:
/// - This is a debug build (`cfg!(debug_assertions)`), OR
/// - The `DOO_DEBUG` environment variable is set
#[inline]
pub fn is_enabled() -> bool {
    *DEBUG_ENABLED.get_or_init(|| {
        cfg!(debug_assertions) || std::env::var(crate::constants::ENV_DOO_DEBUG).is_ok()
    })
}

/// Debug logging macro for FFI internals.
///
/// Only produces output when debug mode is enabled.
/// When disabled, format arguments are not evaluated (zero cost).
///
/// # Examples
///
/// ```ignore
/// use doo_ffi_core::ffi_debug;
/// ffi_debug!("HTTP", "Request received: {} {}", "GET", "/api/users");
/// ffi_debug!("DB", "Query executed in {}ms", 42);
/// ```
#[macro_export]
macro_rules! ffi_debug {
    ($component:expr, $($arg:tt)*) => {
        if $crate::debug::is_enabled() {
            eprintln!("[{}] ({}:{}) {}", $component, file!(), line!(), format_args!($($arg)*));
        }
    };
}

/// Fatal error macro — always prints regardless of debug mode.
///
/// Use for unrecoverable errors that must always be visible.
/// Consistent format: `[FATAL] (file:line) message`
///
/// # Examples
///
/// ```ignore
/// use doo_ffi_core::ffi_fatal;
/// ffi_fatal!("Failed to create Tokio runtime: {}", err);
/// ```
#[macro_export]
macro_rules! ffi_fatal {
    ($($arg:tt)*) => {
        eprintln!("[FATAL] ({}:{}) {}", file!(), line!(), format_args!($($arg)*));
    };
}
