//! Constants Module - Single Source of Truth
//!
//! All constants used across the compiler crates are centralized here.

pub mod env_vars;
pub mod ffi_names;
pub mod names;

// Re-export for convenience
pub use ffi_names::*;
pub use names::{demangle_method, mangle_method, METHOD_NAME_PREFIX};
