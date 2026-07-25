//! Display implementations for types.
//!
//! Provides human-readable formatting for all type kinds.

use super::registry::{TypeInfo, TypeKind};
use super::TypeId;
use std::fmt;

impl fmt::Display for TypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "T{}", self.0)
    }
}

impl fmt::Display for TypeInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl TypeKind {
    /// Get a short string representation of the type kind.
    pub fn kind_name(&self) -> &'static str {
        match self {
            TypeKind::Bool => "Bool",
            TypeKind::Char => "Char",
            TypeKind::Int8 => "Int8",
            TypeKind::Int16 => "Int16",
            TypeKind::Int32 => "Int32",
            TypeKind::Int64 => "Int64",
            TypeKind::Int => "Int",
            TypeKind::UInt8 => "UInt8",
            TypeKind::UInt16 => "UInt16",
            TypeKind::UInt32 => "UInt32",
            TypeKind::UInt64 => "UInt64",
            TypeKind::UInt => "UInt",
            TypeKind::Float32 => "Float32",
            TypeKind::Float64 => "Float64",
            TypeKind::Str => "Str",
            TypeKind::Void => "Void",
            TypeKind::Never => "Never",
            TypeKind::Array { .. } => "Array",
            TypeKind::Map { .. } => "Map",
            TypeKind::Set { .. } => "Set",
            TypeKind::Tuple { .. } => "Tuple",
            TypeKind::Struct { .. } => "Struct",
            TypeKind::Enum { .. } => "Enum",
            TypeKind::Interface { .. } => "Interface",
            TypeKind::Function { .. } => "Function",
            TypeKind::Optional { .. } => "Optional",
            TypeKind::Result { .. } => "Result",
            TypeKind::Box { .. } => "Box",
            TypeKind::TypeRef { .. } => "TypeRef",
            TypeKind::TypeParam { .. } => "TypeParam",
            TypeKind::SelfType => "Self",
            TypeKind::Any => "Any",
            TypeKind::Error => "Error",
        }
    }
}
