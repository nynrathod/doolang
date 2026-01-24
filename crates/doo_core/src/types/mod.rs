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

pub mod primitive;
pub mod collection;
pub mod composite;
pub mod registry;
pub mod display;

// Re-export key types from each module
pub use primitive::PrimitiveType;
pub use collection::CollectionType;
pub use composite::{CompositeType, StructDef, EnumDef, FunctionType, FieldDef, VariantDef, DecoratorDef};
pub use registry::{TypeRegistry, TypeKind, TypeId, builtin};
