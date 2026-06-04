//! Centralized debug logging for the Doo compiler pipeline.
//!
//! Single source of truth for all debug output across all compiler crates.
//!
//! ## Behavior
//!
//! - **Debug builds** (`cargo build`): Debug output is always enabled.
//! - **Release builds** (`cargo build --release`): Debug output is disabled by default.
//! - **Release + `--debug`** (`doo run --debug`): Debug output is enabled.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use doo_core::doo_debug;
//!
//!
//! ```

use std::sync::atomic::{AtomicBool, Ordering};

/// Global debug flag.
/// Initialized to `true` in debug builds, `false` in release builds.
static DEBUG_ENABLED: AtomicBool = AtomicBool::new(cfg!(debug_assertions));

/// Initialize debug mode. Call once at program startup.
///
/// In debug builds, debug is always enabled regardless of this flag.
/// In release builds, debug is only enabled if `enabled` is `true`
/// or the `DOO_DEBUG` environment variable is set.
pub fn init(enabled: bool) {
    let should_enable = cfg!(debug_assertions)
        || enabled
        || std::env::var(crate::constants::env_vars::DOO_DEBUG).is_ok();
    DEBUG_ENABLED.store(should_enable, Ordering::Relaxed);
}

/// Check if debug mode is currently enabled.
///
/// This is an atomic load (~1 CPU cycle) — safe to call in hot paths.
#[inline(always)]
pub fn is_enabled() -> bool {
    DEBUG_ENABLED.load(Ordering::Relaxed)
}

/// Debug logging macro for compiler internals.
///
/// Only produces output when debug mode is enabled.
/// When disabled, format arguments are not evaluated (zero cost).
///
/// # Examples
///
/// ```ignore
/// use doo_core::doo_debug;
///
///
///
/// ```
#[macro_export]
macro_rules! doo_debug {
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
/// use doo_core::doo_fatal;
/// doo_fatal!("Failed to create runtime: {}", err);
/// ```
#[macro_export]
macro_rules! doo_fatal {
    ($($arg:tt)*) => {
        eprintln!("[FATAL] ({}:{}) {}", file!(), line!(), format_args!($($arg)*));
    };
}
