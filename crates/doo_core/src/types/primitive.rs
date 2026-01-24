//! Primitive types in Doo.
//!
//! Primitives are the basic building blocks: Int, Float, Bool, Str, Void, Nil.
//! These are value types that can be efficiently copied.

use serde::{Deserialize, Serialize};

/// All primitive types supported by Doo.
///
/// Primitives are value types - they can be copied without allocation.
/// Str is special: it's a fat pointer (ptr + len) but the data is heap-allocated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrimitiveType {
    /// 64-bit signed integer (default integer type)
    Int,
    /// 32-bit signed integer (for FFI compatibility)
    Int32,
    /// 64-bit signed integer (explicit)
    Int64,
    /// 64-bit unsigned integer (future)
    UInt,
    /// 64-bit floating point (default float type)
    Float,
    /// 32-bit floating point (for FFI compatibility)
    Float32,
    /// Boolean (true/false)
    Bool,
    /// UTF-8 string (fat pointer: ptr + len)
    Str,
    /// No value (function returns nothing)
    Void,
    /// Null/nil value (compatible with any Optional or pointer type)
    Nil,
    /// Dynamic type - compatible with any type (used for JSON.parse)
    Any,
    /// Generic error type - can hold any error value
    Error,
}

impl PrimitiveType {
    /// Size in bytes for memory layout and LLVM codegen.
    ///
    /// Str is 16 bytes because it's a fat pointer (ptr: 8 bytes + len: 8 bytes),
    /// not the actual string content size.
    pub fn size_bytes(&self) -> usize {
        match self {
            Self::Int | Self::Int64 | Self::Float | Self::UInt => 8,
            Self::Int32 | Self::Float32 => 4,
            Self::Bool => 1,
            Self::Str => 16, // Fat pointer: ptr (8) + len (8)
            Self::Void | Self::Nil => 0,
            Self::Any => 16, // Tagged union: tag (8) + ptr (8)
            Self::Error => 16, // Error code + message ptr
        }
    }

    /// Alignment requirement in bytes.
    pub fn alignment(&self) -> usize {
        match self {
            Self::Int | Self::Int64 | Self::Float | Self::UInt => 8,
            Self::Int32 | Self::Float32 => 4,
            Self::Bool => 1,
            Self::Str | Self::Any | Self::Error => 8,
            Self::Void | Self::Nil => 1,
        }
    }

    /// Whether this type supports numeric operations.
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            Self::Int | Self::Int32 | Self::Int64 | Self::UInt | Self::Float | Self::Float32
        )
    }

    /// Whether this type supports integer operations (bitwise, modulo).
    pub fn is_integer(&self) -> bool {
        matches!(self, Self::Int | Self::Int32 | Self::Int64 | Self::UInt)
    }

    /// Whether this type supports floating-point operations.
    pub fn is_float(&self) -> bool {
        matches!(self, Self::Float | Self::Float32)
    }

    /// Whether this type can be copied without allocation (true for all primitives except Str content).
    pub fn is_copy(&self) -> bool {
        !matches!(self, Self::Str | Self::Any | Self::Error)
    }

    /// LLVM type representation.
    pub fn llvm_type_name(&self) -> &'static str {
        match self {
            Self::Int | Self::Int64 | Self::UInt => "i64",
            Self::Int32 => "i32",
            Self::Float => "double",
            Self::Float32 => "float",
            Self::Bool => "i1",
            Self::Str | Self::Any | Self::Error => "ptr",
            Self::Void | Self::Nil => "void",
        }
    }

    /// Human-readable Doo type name.
    pub fn doo_type_name(&self) -> &'static str {
        match self {
            Self::Int | Self::Int64 => "Int",
            Self::Int32 => "Int32",
            Self::UInt => "UInt",
            Self::Float => "Float",
            Self::Float32 => "Float32",
            Self::Bool => "Bool",
            Self::Str => "Str",
            Self::Void => "Void",
            Self::Nil => "Nil",
            Self::Any => "Any",
            Self::Error => "Error",
        }
    }

    /// Parse a type name string into a PrimitiveType.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Int" | "Int64" => Some(Self::Int),
            "Int32" => Some(Self::Int32),
            "UInt" => Some(Self::UInt),
            "Float" | "Float64" => Some(Self::Float),
            "Float32" => Some(Self::Float32),
            "Bool" => Some(Self::Bool),
            "Str" | "String" => Some(Self::Str),
            "Void" => Some(Self::Void),
            "Nil" => Some(Self::Nil),
            "Any" => Some(Self::Any),
            "Error" => Some(Self::Error),
            _ => None,
        }
    }

    /// Default value for this primitive type.
    pub fn default_value(&self) -> &'static str {
        match self {
            Self::Int | Self::Int32 | Self::Int64 | Self::UInt => "0",
            Self::Float | Self::Float32 => "0.0",
            Self::Bool => "false",
            Self::Str => "\"\"",
            Self::Void | Self::Nil => "nil",
            Self::Any | Self::Error => "nil",
        }
    }
}

impl std::fmt::Display for PrimitiveType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.doo_type_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_primitive_sizes() {
        assert_eq!(PrimitiveType::Int.size_bytes(), 8);
        assert_eq!(PrimitiveType::Int32.size_bytes(), 4);
        assert_eq!(PrimitiveType::Float.size_bytes(), 8);
        assert_eq!(PrimitiveType::Bool.size_bytes(), 1);
        assert_eq!(PrimitiveType::Str.size_bytes(), 16);
        assert_eq!(PrimitiveType::Void.size_bytes(), 0);
    }

    #[test]
    fn test_numeric_checks() {
        assert!(PrimitiveType::Int.is_numeric());
        assert!(PrimitiveType::Float.is_numeric());
        assert!(!PrimitiveType::Bool.is_numeric());
        assert!(!PrimitiveType::Str.is_numeric());
    }

    #[test]
    fn test_from_name() {
        assert_eq!(PrimitiveType::from_name("Int"), Some(PrimitiveType::Int));
        assert_eq!(PrimitiveType::from_name("Str"), Some(PrimitiveType::Str));
        assert_eq!(PrimitiveType::from_name("String"), Some(PrimitiveType::Str));
        assert_eq!(PrimitiveType::from_name("Unknown"), None);
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", PrimitiveType::Int), "Int");
        assert_eq!(format!("{}", PrimitiveType::Str), "Str");
    }
}
