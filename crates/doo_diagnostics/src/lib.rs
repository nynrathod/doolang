//! # Doo Diagnostics
//!
//! Production-grade error diagnostics and formatting for the Doo compiler.
//! Single source of truth for all error rendering.

pub mod emitter;
pub mod source_map;

pub use emitter::DiagnosticEmitter;
pub use source_map::{SourceMap, SpanContext};
