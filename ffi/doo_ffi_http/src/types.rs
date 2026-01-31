//! HTTP FFI Types
//! Centralized type definitions for FFI-compatible request/response handling.

use std::ffi::c_void;
use std::os::raw::c_char;
use std::collections::HashMap;

/// Handler function pointer type - universal signature
/// All handlers from Doo are compiled to this signature by the compiler
/// The compiler generates wrapper code to adapt any user handler to this
pub type DooHandlerFn = extern "C" fn(*const DooRequest) -> *mut DooResult;

/// Middleware function pointer type
pub type DooMiddlewareFn = extern "C" fn(*const DooRequest, DooNextFn) -> *mut DooResult;

/// Next function for middleware chaining
pub type DooNextFn = extern "C" fn(*const DooRequest) -> *mut DooResult;

/// HTTP Request - FFI compatible
#[repr(C)]
pub struct DooRequest {
    pub method: *const c_char,
    pub path: *const c_char,
    pub body: *const c_char,
    pub headers: *mut c_void,
    pub params: *mut c_void,
    pub query: *mut c_void,
    pub user_id: *const c_char,
}

/// HTTP Response - FFI compatible
#[repr(C)]
pub struct DooResponse {
    pub status: i32,
    pub body: *const c_char,
    pub content_type: *const c_char,
}

/// FFI Result type
#[repr(C)]
pub struct DooResult {
    pub tag: i32,       // 0 = Ok, 1 = Err
    pub value: *mut c_void,
    pub owner: u8,      // 0 = LLVM, 1 = FFI, 2 = Rust
}

/// Owner constants
pub mod owner {
    pub const LLVM: u8 = 0;
    pub const FFI: u8 = 1;
    pub const RUST: u8 = 2;
}

/// CORS configuration
#[derive(Clone, Default)]
pub struct CorsConfig {
    pub origins: Vec<String>,
    pub methods: Vec<String>,
    pub headers: Vec<String>,
    pub credentials: bool,
    pub max_age: Option<i32>,
}

impl CorsConfig {
    pub fn default() -> Self {
        Self {
            origins: vec!["*".to_string()],
            methods: vec!["GET", "POST", "PUT", "DELETE", "OPTIONS", "PATCH"]
                .into_iter().map(String::from).collect(),
            headers: vec!["Content-Type", "Authorization"]
                .into_iter().map(String::from).collect(),
            credentials: false,
            max_age: None,
        }
    }
}

/// Rate limit configuration
#[derive(Clone)]
pub struct RateLimitConfig {
    pub max: u32,      // Max requests
    pub window: u64,   // Window in seconds
    pub per: String,   // "ip" or "user"
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max: 100,
            window: 60,
            per: "ip".to_string(),
        }
    }
}

/// Rate limit state entry
pub struct RateLimitEntry {
    pub count: u32,
    pub window_start: std::time::Instant,
}

/// Handler metadata for serialization
#[derive(Clone, Default)]
pub struct HandlerMetadata {
    pub param_types: Vec<String>,
    pub return_type: String,
    pub struct_decorators: HashMap<String, HashMap<String, Vec<String>>>,
    pub struct_layouts: HashMap<String, serde_json::Value>,
    pub enum_variants: HashMap<String, Vec<String>>,
}
