//! # Doo Diagnostics
//!
//! Production-grade error diagnostics and formatting for the Doo compiler.
//! Single source of truth for all error rendering.
//!
//! Error codes and `CompilerError` live in `doo_core::errors::codes`.
//! This crate provides:
//! - `SourceMap` — maps file IDs to source text for span resolution
//! - `DiagnosticEmitter` — renders errors in compact, summary, and detailed formats
//!
//! ## Compact Format (default)
//!
//! ```text
//! ❌ handlers.doo:12  TYPE MISMATCH  [E0100]
//!    let age: Int = "twenty"
//!                   ~~~~~~~~ Str, expected Int → use: 20
//! ```
//!
//! ## Summary Format (multiple errors)
//!
//! ```text
//! ━━━ DOO COMPILE ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//! ❌ 3 errors  ⚠️ 1 warning  in 2 files
//!
//! handlers.doo
//!   ❌ :12   TYPE MISMATCH        let age: Int = "twenty"     → use: 20
//!   ❌ :15   UNKNOWN NAME         userName                    → user_name?
//! ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//! ```
//!
//! ## Detailed Format (`--explain`)
//!
//! Shows line numbers, context lines, and full explanation.

pub mod emitter;
pub mod source_map;

pub use emitter::DiagnosticEmitter;
pub use source_map::SourceMap;
