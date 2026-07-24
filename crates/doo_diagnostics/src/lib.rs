//! # Doo Diagnostics
//!
//! Production-grade error diagnostics and formatting for the Doo compiler.
//! Single source of truth for all error rendering.
//!
//! Error codes and `CompilerError` live in `doo_core::errors::codes`.
//! This crate provides:
//! - `SourceMap` — maps file IDs to source text for span resolution
//! - `DiagnosticEmitter` — renders errors in compact, summary, and detailed formats

pub mod emitter;
pub mod source_map;

pub use emitter::DiagnosticEmitter;
pub use source_map::{SourceMap, SpanContext};
