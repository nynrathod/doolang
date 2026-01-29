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

pub mod types;
pub mod span;
pub mod symbol;
pub mod errors;
pub mod intern;
pub mod methods;
pub mod constants;
pub mod infer;

// Re-exports for convenience
pub use types::{
    TypeRegistry, TypeId, TypeKind, builtin,
    PrimitiveType, CollectionType, CompositeType,
    StructDef, EnumDef, FunctionType, FieldDef, VariantDef, DecoratorDef,
};
pub use span::{Span, Spanned};
pub use symbol::{SymbolTable, SymbolInfo, SymbolKind};
pub use errors::{ErrorCode, CompilerError};
pub use infer::{BinOpKind, UnaryOpKind, infer_binop_result_type, infer_unaryop_result_type};
