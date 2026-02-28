//! doo_ffi_http - Complete HTTP FFI Library
//!
//! Provides all HTTP functionality for Doo applications:
//! - Route registration (GET, POST, PUT, DELETE, PATCH)
//! - Middleware (JWT, CORS, Rate Limiting)
//! - RFC 7807 error responses
//! - Request helpers (params, query, headers)
//! - Server lifecycle

// ============================================================================
// GLOBAL ALLOCATOR — mimalloc for reduced lock contention at high RPS
// Since this is a cdylib, the allocator applies to all allocations in this DLL.
// ============================================================================
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod error;
mod helpers;
mod middleware;
mod router;
mod server;
mod types;
pub mod ws;

use std::ffi::c_void;
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};

use doo_ffi_core::ffi_debug;
use doo_ffi_core::ffi_safe_ptr;

pub use error::*;
pub use helpers::*;
pub use middleware::*;
pub use router::*;
pub use types::*;

// ============================================================================
// PANIC SAFETY MACROS — catch_unwind at every FFI boundary
// ============================================================================
// Every extern "C" fn MUST NOT unwind into C/LLVM code — UB on all platforms.
//
// Generic macros (ffi_safe_ptr, ffi_safe_cstr, ffi_safe_i32, ffi_safe_i64,
// ffi_safe_f64, ffi_safe_void) come from doo_ffi_core::macros via #[macro_use].
//
// HTTP-specific: ffi_safe_result! uses make_err_http for RFC 7807 formatting.

/// Wrap an extern "C" fn that returns *mut DooResult.
/// On panic → returns a 500 RFC 7807 error DooResult (HTTP-specific).
macro_rules! ffi_safe_result {
    ($body:expr) => {{
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $body)) {
            Ok(r) => r,
            Err(_) => $crate::make_err_http(500, "Internal server error (panic)"),
        }
    }};
}

// New submodules (extracted from lib.rs monolith)
mod auth;
mod crud;
mod db_bridge;
mod dispatch;
mod fetch;
mod map_ops;
mod metadata;
pub mod metrics;
mod middleware_ffi;
mod password_reset;
mod request;
mod response;
mod routes;
mod validation;
mod ws_ffi;

// ============================================================================
// SERVER LIFECYCLE
// ============================================================================

/// Global server instance pointer — accessible to handler wrappers that need `app: Server`.
static GLOBAL_SERVER_PTR: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

/// Get the global server instance pointer. Called by codegen-generated handler
/// wrappers when a user handler has `app: Server` parameter.
#[no_mangle]
pub extern "C" fn doo_http_get_server_instance() -> *const c_void {
    match catch_unwind(AssertUnwindSafe(|| {
        GLOBAL_SERVER_PTR
            .get()
            .map(|p| *p as *const c_void)
            .unwrap_or(std::ptr::null())
    })) {
        Ok(r) => r,
        Err(_) => std::ptr::null(),
    }
}

#[no_mangle]
pub extern "C" fn doo_http_server_new(host_port: *const c_char) -> *mut c_void {
    ffi_safe_ptr!({
        // Register bridge symbols so other FFI crates (e.g., doo_ffi_auth) can
        // discover them via doo_ffi_core::ffi_bridge — works for static linking
        // where OS-level symbol resolution (GetProcAddress/dlsym) may not find
        // symbols in the final executable.
        register_bridge_symbols();

        let host_port_str = if host_port.is_null() {
            ":3000".to_string()
        } else {
            c_to_string(host_port)
        };

        // Parse host:port
        let (host, port) = if let Some(colon) = host_port_str.rfind(':') {
            let h = &host_port_str[..colon];
            let p: i64 = host_port_str[colon + 1..].parse().unwrap_or(3000);
            (if h.is_empty() { "127.0.0.1" } else { h }.to_string(), p)
        } else {
            ("127.0.0.1".to_string(), 3000i64)
        };

        // Allocate server struct matching LLVM's %Server = type { i64, ptr }
        // Layout: Port (i64) at offset 0, Host (ptr) at offset 8
        unsafe {
            let ptr = libc::malloc(16) as *mut u8;
            if ptr.is_null() {
                return std::ptr::null_mut();
            }
            // Store Port as i64 at offset 0
            *(ptr as *mut i64) = port;
            // Store Host as ptr at offset 8
            *(ptr.add(8) as *mut *const c_char) = string_to_c(&host);
            ptr as *mut c_void
        }
    })
}

#[no_mangle]
pub extern "C" fn doo_http_listen(server_ptr: *const c_void) -> *mut DooResult {
    ffi_safe_result!({
        // Store global server pointer for handler wrappers with `app: Server` param
        let _ = GLOBAL_SERVER_PTR.set(server_ptr as usize);

        let (host, port) = if server_ptr.is_null() {
            ("0.0.0.0".to_string(), 3000u16)
        } else {
            unsafe {
                // Server struct: { i64 Port, ptr Host }
                let port = *(server_ptr as *const i64) as u16;
                let host_ptr = *((server_ptr as *const u8).add(8) as *const *const c_char);
                (c_to_string(host_ptr), port)
            }
        };

        match server::start_server(&host, port) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err_http(500, &e),
        }
    })
}

// ============================================================================
// SHARED UTILITY FUNCTIONS -- used by all submodules
// ============================================================================

pub(crate) fn make_ok_void() -> *mut DooResult {
    DooResult::ok_empty().into_raw()
}

pub(crate) fn make_ok_json(json: &str) -> *mut DooResult {
    ffi_debug!("FFI", "make_ok_json called with len={}", json.len());
    let ptr = DooResult::ok_string(json).into_raw();
    ffi_debug!("FFI", "make_ok_json: returning DooResult at {:?}", ptr);
    ptr
}

/// Create an error result using centralized error response builder
/// Error response struct layout: { i32 status, ptr body, ptr content_type }
pub(crate) fn make_err_http(status: i32, message: &str) -> *mut DooResult {
    // Ensure set_last_error always stores proper RFC 7807 JSON
    let json_body = if message.starts_with('{') || message.starts_with('[') {
        message.to_string()
    } else {
        let path = get_current_request_path();
        Rfc7807Error::new(status as u16, message)
            .with_instance(&path)
            .to_json()
    };
    set_last_error(status, json_body);
    unsafe {
        // Use centralized helper to build error response struct
        let error_response = alloc_error_response(status, message);
        if error_response.is_null() {
            return std::ptr::null_mut();
        }

        // Allocate DooResult using core layout { i64 tag, *mut c_void data }
        let ptr = libc::malloc(std::mem::size_of::<DooResult>()) as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        std::ptr::write(ptr, DooResult::err(status as u16, error_response, 0));
        ptr
    }
}

// ============================================================================
// FFI BRIDGE REGISTRATION — Cross-Crate Symbol Discovery
// ============================================================================

/// Register HTTP bridge symbols with doo_ffi_core's FFI bridge registry.
/// Called once during server init so other packages (e.g., doo_ffi_auth)
/// can discover these functions without OS-level symbol resolution.
fn register_bridge_symbols() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        doo_ffi_core::ffi_bridge::register(
            "doo_http_register_package_route",
            routes::doo_http_register_package_route as *const std::ffi::c_void,
        );
        doo_ffi_core::ffi_bridge::register(
            "doo_http_push_cookie",
            routes::doo_http_push_cookie as *const std::ffi::c_void,
        );
    });
}
