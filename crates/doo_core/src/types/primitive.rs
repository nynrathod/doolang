//! Primitive types in Doo.
//!
//! Primitives are the basic building blocks. They are value types (Copy) and
//! map directly to LLVM IR types.

use serde::{Deserialize, Serialize};

/// All primitive types supported by Doo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrimitiveType {
    Bool,
    Char,
    // Signed Integers
    Int8,
    Int16,
    Int32,
    Int64,
    Int, // Int is pointer-sized (default)
    // Unsigned Integers
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    UInt,
    // Floating Point
    Float32,
    Float64, // Float is alias for Float64 (default)
    // Strings & Void
    Str,   // Fat pointer (ptr + len), UTF-8
    Void,  // No value ()
    Never, // Bottom type (!), for functions that never return
}

impl PrimitiveType {
    /// Get the size in bytes for ABI and LLVM codegen.
    pub fn size_in_bytes(&self) -> usize {
        match self {
            Self::Bool | Self::Char | Self::Int8 | Self::UInt8 => 1,
            Self::Int16 | Self::UInt16 => 2,
            Self::Int32 | Self::UInt32 | Self::Float32 => 4,
            Self::Int64 | Self::UInt64 | Self::Float64 => 8,
            Self::Int | Self::UInt => std::mem::size_of::<usize>(),
            Self::Str => 16, // Fat pointer: ptr (8) + len (8) on 64-bit
            Self::Void | Self::Never => 0,
        }
    }

    /// Get the alignment requirement in bytes.
    pub fn align_in_bytes(&self) -> usize {
        match self {
            Self::Bool | Self::Char | Self::Int8 | Self::UInt8 => 1,
            Self::Int16 | Self::UInt16 => 2,
            Self::Int32 | Self::UInt32 | Self::Float32 => 4,
            Self::Int64 | Self::UInt64 | Self::Float64 | Self::Int | Self::UInt | Self::Str => 8,
            Self::Void | Self::Never => 0,
        }
    }

    /// Whether this type is Copy (bitwise copyable, no heap ownership).
    pub fn is_copy(&self) -> bool {
        !matches!(self, Self::Str | Self::Void | Self::Never)
    }

    /// Get the LLVM IR type string representation.
    pub fn llvm_type(&self) -> &'static str {
        match self {
            Self::Bool => "i1",
            Self::Char | Self::Int8 | Self::UInt8 => "i8",
            Self::Int16 | Self::UInt16 => "i16",
            Self::Int32 | Self::UInt32 | Self::Float32 => "f32",
            Self::Int64 | Self::UInt64 | Self::Float64 => "f64",
            Self::Int | Self::UInt => "i64",
            Self::Str => "{ i8*, i64 }",
            Self::Void => "void",
            Self::Never => "void",
        }
    }

    /// Get the Doo source code name for this primitive.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Bool => "Bool",
            Self::Char => "Char",
            Self::Int8 => "Int8",
            Self::Int16 => "Int16",
            Self::Int32 => "Int32",
            Self::Int64 => "Int64",
            Self::Int => "Int",
            Self::UInt8 => "UInt8",
            Self::UInt16 => "UInt16",
            Self::UInt32 => "UInt32",
            Self::UInt64 => "UInt64",
            Self::UInt => "UInt",
            Self::Float32 => "Float32",
            Self::Float64 => "Float64",
            Self::Str => "Str",
            Self::Void => "Void",
            Self::Never => "Never",
        }
    }

    /// Parse a type name string into a PrimitiveType.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Bool" => Some(Self::Bool),
            "Char" => Some(Self::Char),
            "Int8" => Some(Self::Int8),
            "Int16" => Some(Self::Int16),
            "Int32" => Some(Self::Int32),
            "Int64" => Some(Self::Int64),
            "Int" => Some(Self::Int),
            "UInt8" => Some(Self::UInt8),
            "UInt16" => Some(Self::UInt16),
            "UInt32" => Some(Self::UInt32),
            "UInt64" => Some(Self::UInt64),
            "UInt" => Some(Self::UInt),
            "Float32" => Some(Self::Float32),
            "Float64" | "Float" => Some(Self::Float64),
            "Str" | "String" => Some(Self::Str),
            "Void" => Some(Self::Void),
            "Never" => Some(Self::Never),
            _ => None,
        }
    }
}

impl std::fmt::Display for PrimitiveType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}
