//! Environment Variable Names — Single Source of Truth
//!
//! All environment variable names used across the compiler crates.
//! NEVER hardcode env var strings — import from here.

// ============================================================================
// Compiler Debug & Config
// ============================================================================

/// Master debug flag — enables debug output across all crates.
pub const DOO_DEBUG: &str = "DOO_DEBUG";

/// Enables verbose type registry debug output.
pub const DOO_DEBUG_TYPES: &str = "DOO_DEBUG_TYPES";

/// Controls whether compiler warnings are shown.
pub const DOO_SHOW_WARNINGS: &str = "DOO_SHOW_WARNINGS";

/// Overrides the entry point file name.
pub const DOO_ENTRY: &str = "DOO_ENTRY";

/// Overrides the build root directory.
pub const DOO_BUILD_ROOT: &str = "DOO_BUILD_ROOT";

/// Overrides the standard library search path.
pub const DOO_STDLIB_PATH: &str = "DOO_STDLIB_PATH";

/// Overrides the output binary name.
pub const DOO_OUTPUT_NAME: &str = "DOO_OUTPUT_NAME";

/// Verbose output — shows detailed startup/runtime info.
pub const DOO_VERBOSE: &str = "DOO_VERBOSE";

/// When set, only check for errors without compiling.
pub const DOO_CHECK_ONLY: &str = "DOO_CHECK_ONLY";

/// Disables the compiler banner on startup.
pub const DOO_NO_BANNER: &str = "DOO_NO_BANNER";
