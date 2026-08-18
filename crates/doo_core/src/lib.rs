//! # Doo Core
//!
//! Core types, traits, and utilities for the Doo compiler.
//!
//! ## Architecture
//!
//! - **Arena**: Bump-allocated memory for AST/HIR/MIR nodes
//! - **Interning**: String interning for O(1) symbol comparison
//! - **Span**: Source locations for error reporting
//! - **Types**: Centralized type registry

pub mod arena;
pub mod constants;
pub mod debug;
pub mod errors;
pub mod infer;
pub mod intern;
pub mod logging;
pub mod query;
pub mod scope;
pub mod span;
pub mod string;
pub mod symbol;
pub mod types;

// Re-export key types for convenience
pub use arena::{Arena, CompilerArena};
pub use constants::ffi_names;
pub use errors::{CompilerError, ErrorCode, ErrorSeverity};
pub use intern::Interner;
pub use query::TyCtxt;
pub use scope::{Scope, ScopeError, ScopeKind, ScopeManager, Symbol as ScopeSymbol, SymbolKind};
pub use span::{FileId, FileSpan, Span, Spanned};
pub use string::DooStr;
pub use symbol::Symbol;
pub use types::{TypeId, TypeInfo, TypeKind, TypeRegistry};
