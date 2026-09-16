//! # FFI Bridge — Generic Cross-Crate Symbol Registry
//!
//! When FFI crates are statically linked into a single binary, normal OS-level
//! symbol resolution (`GetProcAddress` on Windows, `dlsym` on Unix) may not
//! find symbols because static executables don't have export tables.
//!
//! This module provides a **process-wide registry** where FFI crates can:
//! - **Register** their public functions by name (producer)
//! - **Resolve** functions from other crates by name (consumer)
//!
//! ## Usage
//!
//! **Producer** (e.g., `doo_ffi_http`):
//! ```ignore
//! doo_ffi_core::ffi_bridge::register(
//!     "doo_http_register_package_route",
//!     doo_http_register_package_route as *const c_void,
//! );
//! ```
//!
//! **Consumer** (e.g., `doo_ffi_auth`):
//! ```ignore
//! let fn_ptr: *const c_void = doo_ffi_core::ffi_bridge::resolve(
//!     "doo_http_register_package_route"
//! )?;
//! ```
//!
//! ## Design
//!
//! - Zero compiler involvement — packages manage their own inter-crate discovery
//! - Thread-safe via `Mutex` — producers write during init, consumers read after
//! - Works identically on Windows, Linux, and macOS
//! - Works for both static and dynamic linking

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;

/// Wrapper around a raw function pointer that is safe to share across threads.
/// Function pointers reference immutable code in the text segment — always safe.
#[derive(Clone, Copy)]
struct FnPtr(*const c_void);

// SAFETY: Function pointers reference immutable code — safe to send/share.
unsafe impl Send for FnPtr {}
unsafe impl Sync for FnPtr {}

/// Global registry of inter-FFI bridge symbols.
static BRIDGE: Mutex<Option<HashMap<&'static str, FnPtr>>> = Mutex::new(None);

/// Register a function pointer by name in the global bridge registry.
///
/// Called by FFI crates during their initialization to make their functions
/// discoverable by other FFI crates without a Cargo dependency.
///
/// # Arguments
/// - `name`: A `&'static str` symbol name (e.g., `"doo_http_register_package_route"`)
/// - `ptr`: The function pointer cast to `*const c_void`
pub fn register(name: &'static str, ptr: *const c_void) {
    if let Ok(mut guard) = BRIDGE.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        map.insert(name, FnPtr(ptr));
        crate::ffi_debug!("BRIDGE", "Registered: {}", name);
    }
}

/// Resolve a function pointer by name from the global bridge registry.
///
/// Returns `Some(ptr)` if the symbol was registered, `None` otherwise.
pub fn resolve(name: &str) -> Option<*const c_void> {
    if let Ok(guard) = BRIDGE.lock() {
        if let Some(map) = guard.as_ref() {
            return map.get(name).map(|fp| fp.0);
        }
    }
    None
}

/// Convenience: resolve and transmute to a specific function type.
///
/// # Safety
/// The caller must ensure `T` matches the actual function signature.
pub unsafe fn resolve_as<T: Copy>(name: &str) -> Option<T> {
    resolve(name).map(|ptr| std::mem::transmute_copy(&ptr))
}
