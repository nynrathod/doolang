//! Doo Language Server — LSP implementation using the actual compiler.
//!
//! Provides IDE features backed by the real Doo compiler frontend:
//! - **Diagnostics**: Real compiler errors + warnings on every keystroke
//! - **Go-to-Definition**: Precise, scope-aware symbol resolution
//! - **Hover**: Type information from the actual type checker
//! - **Completions**: Context-aware suggestions from scope + type registry
//! - **Document Symbols**: AST-based outline of functions, structs, enums
//!
//! ## Architecture
//!
//! ```text
//! VS Code Extension ─── LSP Protocol (stdio) ─── doo-lsp binary
//!                                                     │
//!                                          ┌──────────┼──────────┐
//!                                     doo_frontend  doo_hir  doo_analysis
//!                                      (parser)    (lowering) (type check)
//! ```
//!
//! The LSP server maintains an in-memory cache of parsed files and their
//! analysis results. On each edit, only the changed file is re-parsed and
//! re-analyzed.

pub mod analysis;
pub mod capabilities;
pub mod diagnostics;
pub mod handler;
pub mod state;
