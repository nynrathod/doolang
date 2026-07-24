//! # Doo Core
//!
//! Core types, traits, and infrastructure for the Doo compiler.
//!
//! ## Design Philosophy (Memory Model Master Part I)
//!
//! - **Single Source of Truth**: Each concept defined in exactly one place.
//! - **Centralized**: Symbols, spans, types, errors all in one crate.
//! - **Minimal**: Only essential types — no domain-specific (HTTP/DB/Auth) logic.
//! - **Zero Runtime Cost**: All types are compile-time constructs.
//!
//! ## Modules
//!
//! - [`arena`]: Arena allocation (typed + type-erased bump allocators)
//! - [`span`]: Source span tracking (`Span`, `FileId`, `FileSpan`)
//! - [`symbol`]: Interned string symbols (`Symbol`, pre-interned keywords)
//! - [`intern`]: Thread-safe string interner (`Interner`, global interner)
//! - [`string`]: Fat-pointer string type for FFI (`DooStr`)
//! - [`scope`]: Scope-based symbol table for name resolution
//! - [`types`]: Type registry and type definitions
//! - [`errors`]: Centralized error codes
//! - [`infer`]: Type inference utilities
//! - [`query`]: Query-based incremental compilation
//! - [`constants`]: Centralized constants (env vars, FFI names)
//! - [`debug`]: Debug logging
//! - [`logging`]: Structured tracing
//! - [`methods`]: Builtin method registry

pub mod arena;
pub mod constants;
pub mod debug;
pub mod errors;
pub mod infer;
pub mod intern;
pub mod logging;
pub mod methods;
pub mod query;
pub mod scope;
pub mod span;
pub mod string;
pub mod symbol;
pub mod types;

// ============================================================================
// Re-exports (primary API surface)
// ============================================================================

// Arena
pub use arena::{Arena, ArenaStats, CompilerArena};

// Span
pub use span::{FileId, FileSpan, LineIndex, Span, Spanned};

// Symbol + Intern
pub use intern::{get as get_symbol, intern, intern_static, kw, resolve, Interner};
pub use symbol::Symbol;

// String
pub use string::DooStr;

// Scope
pub use scope::{Scope, SymbolError, SymbolInfo, SymbolKind, SymbolTable};

// Types
pub use types::registry::{TypeId, TypeInfo, TypeKind, TypeRegistry};

// Errors
pub use errors::codes::{CompilerError, ErrorCategory, ErrorCode, ErrorSeverity};

// Query
pub use query::{QueryDatabase, QueryKey, Revision};
