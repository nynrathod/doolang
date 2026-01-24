//! # Error Codes
//!
//! Centralized error codes for the entire compiler.
//! See `codes.rs` for the complete error code definitions.

pub mod codes;

pub use codes::{ErrorCode, ErrorSeverity, CompilerError};
