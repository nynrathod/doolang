//! # Doo Core Types
//!
//! Central type system for the Doo compiler.
//!
//! ## Design (from implementation_plan.md Phase 1)
//!
//! - **Single Source of Truth**: All types flow through TypeRegistry
//! - **Primitive, Collection, Composite**: Organized by category
//! - **TypeId-based**: Fast lookup, cheap copies
//!
//! ## Structure
//!
//! - `primitive`: Int, Float, Bool, Str, Void, etc.
//! - `collection`: Array, Map, Tuple, Optional, Result
//! - `composite`: Struct, Enum, Function
//! - `registry`: TypeRegistry - the central type store

pub mod collection;
pub mod composite;
pub mod display;
pub mod primitive;
pub mod registry;

// Re-export key types from each module
pub use collection::CollectionType;
pub use composite::{
    CompositeType, DecoratorDef, EnumDef, FieldDef, FunctionType, StructDef, VariantDef,
};
pub use primitive::PrimitiveType;
pub use registry::{builtin, TargetDataLayout, TypeId, TypeKind, TypeRegistry};
