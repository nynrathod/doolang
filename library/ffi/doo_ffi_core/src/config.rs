//! Config Module — Environment Variable Access (Single Source of Truth)
//!
//! Provides user-facing functions for reading environment variables
//! from both `.env` files (loaded by doo_driver) and system env vars.
//!
//! ## FFI Functions
//!
//! - `doo_config_get(key)` → Returns value or panics if missing
//! - `doo_config_get_or(key, default)` → Returns value or default if missing
//! - `doo_config_has(key)` → Returns 1 if key exists, 0 otherwise
//! - `doo_config_get_int(key, default)` → Returns parsed int or default
//! - `doo_config_get_bool(key, default)` → Returns parsed bool or default
//!
//! ## Memory Model
//!
//! All returned strings are allocated via `doo_alloc_string` (libc::malloc).
//! Caller owns the returned pointer and must free via `doo_free`.

use crate::helpers::{c_to_string, make_err, make_ok_string, string_to_c};
use crate::result::DooResult;
use crate::ffi_debug;
use std::os::raw::c_char;

// ============================================================================
// Config::get(key) — panics if missing
// ============================================================================

/// Get an environment variable value by key.
/// Returns the value as a C string, or an error DooResult if the key is not set.
///
/// Doo syntax: `Config::get("API_KEY")`
#[no_mangle]
pub extern "C" fn doo_config_get(key: *const c_char) -> *mut DooResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let key_str = match c_to_string(key) {
            Ok(s) => s,
            Err(e) => return make_err(400, &format!("Config::get: invalid key — {}", e)),
        };

        ffi_debug!("CONFIG", "Config::get({:?})", key_str);

        match std::env::var(&key_str) {
            Ok(val) => {
                ffi_debug!("CONFIG", "Config::get({:?}) = {:?}", key_str, val);
                make_ok_string(&val)
            }
            Err(_) => make_err(
                404,
                &format!(
                    "Config::get: environment variable '{}' is not set. \
                     Set it in your .env file or system environment.",
                    key_str
                ),
            ),
        }
    })) {
        Ok(result) => result,
        Err(_) => make_err(500, "Config::get: internal error (panic)"),
    }
}

// ============================================================================
// Config::get(key, default) — returns default if missing
// ============================================================================

/// Get an environment variable value by key, with a default fallback.
/// Returns the value if set, or the default value if not.
/// Always succeeds (returns default on missing key), so returns a raw string.
///
/// Doo syntax: `Config::getOr("PORT", "3100")`
#[no_mangle]
pub extern "C" fn doo_config_get_or(key: *const c_char, default: *const c_char) -> *const c_char {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let key_str = match c_to_string(key) {
            Ok(s) => s,
            Err(_) => {
                // On invalid key, fall back to default
                let default_str = c_to_string(default).unwrap_or_default();
                return string_to_c(&default_str);
            }
        };

        let default_str = match c_to_string(default) {
            Ok(s) => s,
            Err(_) => String::new(),
        };

        ffi_debug!(
            "CONFIG",
            "Config::getOr({:?}, default={:?})",
            key_str,
            default_str
        );

        match std::env::var(&key_str) {
            Ok(val) => {
                ffi_debug!(
                    "CONFIG",
                    "Config::getOr({:?}) = {:?} (from env)",
                    key_str,
                    val
                );
                string_to_c(&val)
            }
            Err(_) => {
                ffi_debug!(
                    "CONFIG",
                    "Config::getOr({:?}) = {:?} (default)",
                    key_str,
                    default_str
                );
                string_to_c(&default_str)
            }
        }
    })) {
        Ok(result) => result,
        Err(_) => string_to_c(""),
    }
}

// ============================================================================
// Config::has(key) — check if variable exists
// ============================================================================

/// Check if an environment variable exists.
/// Returns a C string "true" or "false".
///
/// Doo syntax: `Config::has("API_KEY")` → Bool
#[no_mangle]
pub extern "C" fn doo_config_has(key: *const c_char) -> *const c_char {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let key_str = match c_to_string(key) {
            Ok(s) => s,
            Err(_) => return string_to_c("false"),
        };

        ffi_debug!("CONFIG", "Config::has({:?})", key_str);

        if std::env::var(&key_str).is_ok() {
            string_to_c("true")
        } else {
            string_to_c("false")
        }
    })) {
        Ok(result) => result,
        Err(_) => string_to_c("false"),
    }
}

// ============================================================================
// Config::getInt(key, default) — parse as integer
// ============================================================================

/// Get an environment variable as an integer, with default fallback.
/// Parses the env var value as i64.
///
/// Doo syntax: `Config::getInt("PORT", 3100)` → Int
#[no_mangle]
pub extern "C" fn doo_config_get_int(key: *const c_char, default: i64) -> i64 {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let key_str = match c_to_string(key) {
            Ok(s) => s,
            Err(_) => return default,
        };

        ffi_debug!(
            "CONFIG",
            "Config::getInt({:?}, default={})",
            key_str,
            default
        );

        match std::env::var(&key_str) {
            Ok(val) => val.trim().parse::<i64>().unwrap_or(default),
            Err(_) => default,
        }
    })) {
        Ok(result) => result,
        Err(_) => default,
    }
}

// ============================================================================
// Config::getBool(key, default) — parse as boolean
// ============================================================================

/// Get an environment variable as a boolean, with default fallback.
/// Recognizes "true", "1", "yes", "on" as true; everything else as false.
///
/// Doo syntax: `Config::getBool("FEATURE_FLAG", false)` → Bool
#[no_mangle]
pub extern "C" fn doo_config_get_bool(key: *const c_char, default: i32) -> i32 {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let key_str = match c_to_string(key) {
            Ok(s) => s,
            Err(_) => return default,
        };

        ffi_debug!(
            "CONFIG",
            "Config::getBool({:?}, default={})",
            key_str,
            default != 0
        );

        match std::env::var(&key_str) {
            Ok(val) => {
                let val_lower = val.trim().to_lowercase();
                match val_lower.as_str() {
                    "true" | "1" | "yes" | "on" => 1,
                    "false" | "0" | "no" | "off" => 0,
                    _ => default,
                }
            }
            Err(_) => default,
        }
    })) {
        Ok(result) => result,
        Err(_) => default,
    }
}

// ============================================================================
// Config::set(key, value) — set environment variable at runtime
// ============================================================================

/// Set an environment variable at runtime.
/// Useful for tests and dynamic configuration.
///
/// Doo syntax: `Config::set("KEY", "value")`
#[no_mangle]
pub extern "C" fn doo_config_set(key: *const c_char, value: *const c_char) -> *const c_char {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let key_str = match c_to_string(key) {
            Ok(s) => s,
            Err(_) => return string_to_c(""),
        };
        let value_str = match c_to_string(value) {
            Ok(s) => s,
            Err(_) => return string_to_c(""),
        };

        ffi_debug!("CONFIG", "Config::set({:?}, {:?})", key_str, value_str);

        // SAFETY: We are in a single-threaded init context.
        // In production, env vars are set before the server starts.
        unsafe {
            std::env::set_var(&key_str, &value_str);
        }

        string_to_c(&value_str)
    })) {
        Ok(result) => result,
        Err(_) => string_to_c(""),
    }
}
