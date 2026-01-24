//! # Doo Diagnostics
//!
//! Error diagnostics and formatting for the Doo compiler.
//!
//! ## Error Format
//!
//! ```text
//! ❌ file:line  ERROR_TYPE
//!    source line
//!    ~~~~~~~~ description → fix
//! ```

pub mod codes;
pub mod emitter;

pub use codes::{ErrorCode, ErrorCategory};
pub use emitter::DiagnosticEmitter;
