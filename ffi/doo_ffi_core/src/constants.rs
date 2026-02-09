//! # FFI Constants - SINGLE SOURCE OF TRUTH
//!
//! All middleware names, FFI function names, and identifiers used across
//! compiler and FFI runtime MUST be defined here.
//!
//! ## Usage
//! - Compiler crates: `use doo_ffi_core::constants::*;`
//! - FFI crates: `use doo_ffi_core::constants::*;`

// ============================================================================
// MIDDLEWARE NAMES
// ============================================================================

/// JWT middleware identifier - matches Doo's `Jwt()` function (PascalCase = public)
pub const MIDDLEWARE_JWT: &str = "Jwt";

/// CORS middleware identifier
pub const MIDDLEWARE_CORS: &str = "cors";

/// Rate limit middleware identifier
pub const MIDDLEWARE_RATELIMIT: &str = "ratelimit";

// ============================================================================
// DOO FUNCTION NAMES (as they appear in Doo source code)
// ============================================================================

/// The Doo function name for JWT middleware (PascalCase = public per naming convention)
pub const DOO_JWT_FUNC_NAME: &str = "Jwt";

// ============================================================================
// BUILTIN MIDDLEWARE LIST
// ============================================================================

/// All built-in middleware names that the compiler and FFI recognize
pub const BUILTIN_MIDDLEWARES: &[&str] = &[MIDDLEWARE_JWT, MIDDLEWARE_CORS, MIDDLEWARE_RATELIMIT];

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Check if a middleware name is a built-in middleware
#[inline]
pub fn is_builtin_middleware(name: &str) -> bool {
    BUILTIN_MIDDLEWARES.contains(&name)
}

/// Check if a function name is the auth/JWT middleware function
#[inline]
pub fn is_auth_middleware(name: &str) -> bool {
    name == DOO_JWT_FUNC_NAME
}

/// Check if a middleware should skip normal FFI call generation
/// (because it returns a string identifier, not a callable)
#[inline]
pub fn is_middleware_identifier_func(name: &str) -> bool {
    name == DOO_JWT_FUNC_NAME
}
