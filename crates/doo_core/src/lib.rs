//! # Doo Core
//!
//! Core types, traits, and infrastructure for the Doo compiler.

pub mod arena;
pub mod constants;
pub mod debug;
pub mod errors;
pub mod infer;
pub mod intern;
pub mod logging;
pub mod methods;
pub mod query; // Now contains TyCtxt
pub mod scope;
pub mod span;
pub mod string;
pub mod symbol;
pub mod types;

// Re-exports
pub use arena::{Arena, ArenaStats, CompilerArena};
pub use errors::codes::{CompilerError, ErrorCategory, ErrorCode, ErrorSeverity};
pub use intern::{get as get_symbol, intern, intern_static, kw, resolve, Interner};
pub use query::TyCtxt;
pub use scope::{Scope, SymbolError, SymbolInfo, SymbolKind, SymbolTable};
pub use span::{FileId, FileSpan, LineIndex, Span, Spanned};
pub use string::DooStr;
pub use symbol::Symbol;
pub use types::registry::{TypeId, TypeInfo, TypeKind, TypeRegistry}; // Export TyCtxt instead of QueryDatabase
