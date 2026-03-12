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

/// Logger middleware identifier
pub const MIDDLEWARE_LOGGER: &str = "logger";

// ============================================================================
// DOO FUNCTION NAMES (as they appear in Doo source code)
// ============================================================================

/// The Doo function name for JWT middleware (PascalCase = public per naming convention)
pub const DOO_JWT_FUNC_NAME: &str = "Jwt";

// ============================================================================
// BUILTIN MIDDLEWARE LIST
// ============================================================================

/// All built-in middleware names that the compiler and FFI recognize
pub const BUILTIN_MIDDLEWARES: &[&str] = &[
    MIDDLEWARE_JWT,
    MIDDLEWARE_CORS,
    MIDDLEWARE_RATELIMIT,
    MIDDLEWARE_LOGGER,
];

// ============================================================================
// ENVIRONMENT VARIABLE NAMES — SINGLE SOURCE OF TRUTH
// ============================================================================

/// Master debug flag — enables debug output across all FFI crates.
pub const ENV_DOO_DEBUG: &str = "DOO_DEBUG";

/// Suppresses the HTTP server startup banner.
pub const ENV_DOO_NO_BANNER: &str = "DOO_NO_BANNER";

/// Verbose mode — shows detailed startup info (routes, timings, etc.).
pub const ENV_DOO_VERBOSE: &str = "DOO_VERBOSE";

/// JWT secret key for token signing/verification.
pub const ENV_JWT_SECRET: &str = "JWT_SECRET";

/// Access token expiry override (e.g., "15m", "1h", "900").
/// Default: 15 minutes. Used by both JWT and OAuth session tokens.
pub const ENV_ACCESS_TOKEN_EXPIRY: &str = "ACCESS_TOKEN_EXPIRY";

/// Refresh token expiry override (e.g., "7d", "30d", "604800").
/// Default: 7 days.
pub const ENV_REFRESH_TOKEN_EXPIRY: &str = "REFRESH_TOKEN_EXPIRY";

/// Auth base path (for cookie path scoping and route registration).
/// Default: "/auth". Set by OAuth setup or manually.
pub const ENV_AUTH_BASE_PATH: &str = "DOO_AUTH_BASE_PATH";

/// Dev mode flag — disables Secure flag on cookies for HTTP (not HTTPS).
pub const ENV_DOO_DEV: &str = "DOO_DEV";

/// PostgreSQL connection URL.
pub const ENV_DATABASE_URL: &str = "DATABASE_URL";

/// Maximum concurrent database queries (semaphore permits).
pub const ENV_DATABASE_MAX_QUERIES: &str = "DATABASE_MAX_QUERIES";

/// Database query timeout in seconds.
pub const ENV_DATABASE_QUERY_TIMEOUT_SECS: &str = "DATABASE_QUERY_TIMEOUT_SECS";

/// Database semaphore acquisition timeout in milliseconds.
pub const ENV_DATABASE_SEMAPHORE_WAIT_MS: &str = "DATABASE_SEMAPHORE_WAIT_MS";

/// Database connection pool size.
pub const ENV_DATABASE_POOL_SIZE: &str = "DATABASE_POOL_SIZE";

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
