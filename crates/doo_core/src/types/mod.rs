//! # Core Type System
//!
//! Defines the central type registry and all type representations for the Doo compiler.
//!
//! ## Design
//!
//! - **Single Source of Truth**: All types flow through the `TypeRegistry`.
//! - **TypeId-based**: Fast lookup, cheap copies (4 bytes).
//! - **Comprehensive**: Includes primitives, collections, composites, and compiler internals.

pub mod collection;
pub mod composite;
pub mod display;
pub mod primitive;
pub mod registry;

// Re-export key types from each module
pub use collection::CollectionType;
pub use composite::{
    CompositeDef, DecoratorDef, EnumDef, FieldDef, FunctionSig, InterfaceDef, MethodSig, StructDef,
    VariantDef,
};
pub use primitive::PrimitiveType;
pub use registry::{TargetDataLayout, TypeId, TypeInfo, TypeKind, TypeRegistry};

/// Re-export the built-in type IDs for convenience.
pub mod builtin {
    pub use crate::types::registry::builtin::*;
}
