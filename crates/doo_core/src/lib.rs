//! # Doo Core
//!
//! Core types, traits, and infrastructure for the Doo compiler.
//!
//! ## Design Philosophy
//!
//! - **Single Source of Truth**: TypeRegistry is the ONLY place types are defined
//! - **Centralized**: Symbols, spans, errors all in one place
//! - **Minimal**: Only essential types, no domain-specific logic
//!
//! ## Modules
//!
//! - `types`: TypeRegistry and type definitions
//! - `span`: Source location tracking
//! - `symbol`: Symbol table with scopes
//! - `errors`: Centralized error codes
//! - `infer`: Type inference utilities (single source of truth)

pub mod constants;
pub mod debug;
pub mod errors;
pub mod infer;
pub mod intern;
pub mod methods;
pub mod span;
pub mod symbol;
pub mod types;

// Re-exports for convenience
pub use errors::{CompilerError, ErrorCode};
pub use infer::{infer_binop_result_type, infer_unaryop_result_type, BinOpKind, UnaryOpKind};
pub use span::{Span, Spanned};
pub use symbol::{SymbolInfo, SymbolKind, SymbolTable};
pub use types::{
    builtin, CollectionType, CompositeType, DecoratorDef, EnumDef, FieldDef, FunctionType,
    PrimitiveType, StructDef, TypeId, TypeKind, TypeRegistry, VariantDef,
};
