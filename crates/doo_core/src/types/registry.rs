//! # Type Registry
//!
//! The central registry for all types in the Doo compiler.
//! Types are identified by a 4-byte `TypeId`.

use crate::symbol::Symbol;
use crate::types::composite::{EnumDef, FunctionSig, InterfaceDef, StructDef};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================================
// Target Data Layout
// ============================================================================

/// Platform-specific type sizes and alignment info.
#[derive(Debug, Clone, Copy)]
pub struct TargetDataLayout {
    pub pointer_size: usize,
    pub i64_size: usize,
    pub f64_size: usize,
    pub alignment: usize,
}

impl TargetDataLayout {
    pub fn x86_64() -> Self {
        Self {
            pointer_size: 8,
            i64_size: 8,
            f64_size: 8,
            alignment: 8,
        }
    }

    pub fn wasm32() -> Self {
        Self {
            pointer_size: 4,
            i64_size: 8,
            f64_size: 8,
            alignment: 4,
        }
    }
}

// ============================================================================
// Type IDs
// ============================================================================

/// Unique identifier for a type (4 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TypeId(pub u32);

impl TypeId {
    pub const fn new(id: u32) -> Self {
        Self(id)
    }
    pub const fn raw(self) -> u32 {
        self.0
    }
}

// ============================================================================
// Type Kind
// ============================================================================

/// The kind of a type - what category it belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TypeKind {
    // Primitives
    Bool,
    Char,
    Int8,
    Int16,
    Int32,
    Int64,
    Int,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    UInt,
    Float32,
    Float64,
    Str,
    Void,
    Never,

    // Collections
    Array { element: TypeId },
    Map { key: TypeId, value: TypeId },
    Set { element: TypeId },
    Tuple { elements: Vec<TypeId> },

    // Composites
    Struct { def: StructDef },
    Enum { def: EnumDef },
    Interface { def: InterfaceDef },
    Function { sig: FunctionSig },
    Optional { inner: TypeId },
    Result { ok: TypeId, err: TypeId },
    Box { inner: TypeId },

    // Compiler Internal
    TypeRef { name: Symbol },   // Unresolved named type
    TypeParam { name: Symbol }, // Generic placeholder <T>
    SelfType,                   // Self in impl blocks
    Any,                        // Dynamic type
    Error,                      // Error recovery type
}

// ============================================================================
// Type Info
// ============================================================================

/// Complete information about a type.
#[derive(Clone, Debug)]
pub struct TypeInfo {
    pub id: TypeId,
    pub kind: TypeKind,
    pub name: String,
}

impl TypeInfo {
    /// Check if this type is Copy (bitwise copyable).
    pub fn is_copy(&self) -> bool {
        match &self.kind {
            TypeKind::Str | TypeKind::Void | TypeKind::Never => false,
            TypeKind::Array { .. } | TypeKind::Map { .. } | TypeKind::Set { .. } => false,
            TypeKind::Struct { .. } | TypeKind::Enum { .. } | TypeKind::Box { .. } => false,
            TypeKind::Function { .. } | TypeKind::Interface { .. } => false,
            _ => true,
        }
    }

    /// Check if this type needs Drop cleanup.
    pub fn needs_drop(&self) -> bool {
        !self.is_copy()
    }

    /// Names under which inherent `impl` methods are registered.
    ///
    /// Collection types use the std type name (`Array`, `Map`, …) rather than
    /// the display name (`[Int]`), matching `library/std/*.doo` impl blocks.
    pub fn inherent_impl_names(&self) -> Vec<String> {
        match &self.kind {
            TypeKind::Array { .. } => vec!["Array".to_string()],
            TypeKind::Map { .. } => vec!["Map".to_string()],
            TypeKind::Set { .. } => vec!["Set".to_string()],
            TypeKind::Optional { .. } => vec!["Option".to_string(), "Optional".to_string()],
            TypeKind::Result { .. } => vec!["Result".to_string()],
            TypeKind::Struct { def } => vec![def.name.resolve().to_string()],
            TypeKind::Enum { def } => vec![def.name.resolve().to_string()],
            _ => {
                if self.name.is_empty() {
                    vec![self.kind.kind_name().to_string()]
                } else {
                    vec![self.name.clone()]
                }
            }
        }
    }
}

// ============================================================================
// Type Registry
// ============================================================================

/// The central type registry - SINGLE SOURCE OF TRUTH for all types.
#[derive(Clone)]
pub struct TypeRegistry {
    types: FxHashMap<TypeId, TypeInfo>,
    name_to_id: HashMap<String, TypeId>,
    next_id: TypeId,
}

impl TypeRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            types: FxHashMap::default(),
            name_to_id: HashMap::new(),
            next_id: TypeId(100), // 0-99 reserved for builtins
        };

        registry.register_builtins();
        registry
    }

    fn register_builtins(&mut self) {
        let builtins = [
            (TypeId(1), "Bool", TypeKind::Bool),
            (TypeId(2), "Char", TypeKind::Char),
            (TypeId(3), "Int8", TypeKind::Int8),
            (TypeId(4), "Int16", TypeKind::Int16),
            (TypeId(5), "Int32", TypeKind::Int32),
            (TypeId(6), "Int64", TypeKind::Int64),
            (TypeId(7), "Int", TypeKind::Int),
            (TypeId(8), "UInt8", TypeKind::UInt8),
            (TypeId(9), "UInt16", TypeKind::UInt16),
            (TypeId(10), "UInt32", TypeKind::UInt32),
            (TypeId(11), "UInt64", TypeKind::UInt64),
            (TypeId(12), "UInt", TypeKind::UInt),
            (TypeId(13), "Float32", TypeKind::Float32),
            (TypeId(14), "Float64", TypeKind::Float64),
            (TypeId(15), "Str", TypeKind::Str),
            (TypeId(16), "Void", TypeKind::Void),
            (TypeId(17), "Never", TypeKind::Never),
            (TypeId(18), "Any", TypeKind::Any),
            (TypeId(19), "Error", TypeKind::Error),
        ];

        for (id, name, kind) in builtins {
            let info = TypeInfo {
                id,
                kind,
                name: name.to_string(),
            };
            self.types.insert(id, info);
            self.name_to_id.insert(name.to_string(), id);
        }

        // Add type aliases so the registry returns the correct TypeId
        self.name_to_id
            .insert("Float".to_string(), builtin::FLOAT64);
        self.name_to_id.insert("String".to_string(), builtin::STR);
    }

    /// Register a new type and return its ID.
    pub fn register(&mut self, name: &str, kind: TypeKind) -> TypeId {
        if let Some(&id) = self.name_to_id.get(name) {
            return id;
        }
        let id = self.next_id;
        self.next_id.0 += 1;

        let info = TypeInfo {
            id,
            kind,
            name: name.to_string(),
        };
        self.types.insert(id, info);
        self.name_to_id.insert(name.to_string(), id);
        id
    }

    /// Declare a named type placeholder (for recursive types).
    pub fn declare_named(&mut self, name: &str) -> TypeId {
        self.register(
            name,
            TypeKind::TypeRef {
                name: Symbol::intern(name),
            },
        )
    }

    /// Define a struct type.
    ///
    /// If `name` already identifies a language primitive (e.g. `Str`), keep
    /// that type. Methods attach through `impl Str` — same as Rust's `impl str`.
    pub fn define_struct(&mut self, def: StructDef) -> TypeId {
        let name = def.name.resolve().to_string();
        if let Some(id) = self.existing_language_type(&name) {
            return id;
        }
        let id = self.declare_named(&name);
        if let Some(info) = self.types.get_mut(&id) {
            info.kind = TypeKind::Struct { def };
        }
        id
    }

    /// Define an enum type.
    pub fn define_enum(&mut self, def: EnumDef) -> TypeId {
        let name = def.name.resolve().to_string();
        if let Some(id) = self.existing_language_type(&name) {
            return id;
        }
        let id = self.declare_named(&name);
        if let Some(info) = self.types.get_mut(&id) {
            info.kind = TypeKind::Enum { def };
        }
        id
    }

    /// True when `name` already maps to a primitive / collection kind that
    /// std must not replace with a user struct or enum.
    fn existing_language_type(&self, name: &str) -> Option<TypeId> {
        let id = self.lookup(name)?;
        let info = self.types.get(&id)?;
        match &info.kind {
            TypeKind::TypeRef { .. } | TypeKind::Struct { .. } | TypeKind::Enum { .. } => None,
            _ => Some(id),
        }
    }

    /// Define an interface type.
    pub fn define_interface(&mut self, def: InterfaceDef) -> TypeId {
        let name = def.name.resolve().to_string();
        let id = self.declare_named(&name);
        if let Some(info) = self.types.get_mut(&id) {
            info.kind = TypeKind::Interface { def };
        }
        id
    }

    /// Register an array type `[T]`.
    pub fn register_array(&mut self, element: TypeId) -> TypeId {
        let name = format!("[{}]", self.display_name(element));
        self.register(&name, TypeKind::Array { element })
    }

    /// Register a map type `{K: V}`.
    pub fn register_map(&mut self, key: TypeId, value: TypeId) -> TypeId {
        let name = format!(
            "Map<{}, {}>",
            self.display_name(key),
            self.display_name(value)
        );
        self.register(&name, TypeKind::Map { key, value })
    }

    /// Register a set type `{T}`.
    pub fn register_set(&mut self, element: TypeId) -> TypeId {
        let name = format!("Set<{}>", self.display_name(element));
        self.register(&name, TypeKind::Set { element })
    }

    /// Register a tuple type `(T1, T2, ...)`.
    pub fn register_tuple(&mut self, elements: Vec<TypeId>) -> TypeId {
        let parts: Vec<String> = elements.iter().map(|e| self.display_name(*e)).collect();
        let name = format!("({})", parts.join(", "));
        self.register(&name, TypeKind::Tuple { elements })
    }

    /// Register a function type `(Params) -> Ret`.
    pub fn register_function(&mut self, sig: FunctionSig) -> TypeId {
        let params: Vec<String> = sig.params.iter().map(|p| self.display_name(*p)).collect();
        let name = format!(
            "fn({}) -> {}",
            params.join(", "),
            self.display_name(sig.return_type)
        );
        self.register(&name, TypeKind::Function { sig })
    }

    /// Register an optional type `T?`.
    pub fn register_optional(&mut self, inner: TypeId) -> TypeId {
        let name = format!("{}?", self.display_name(inner));
        self.register(&name, TypeKind::Optional { inner })
    }

    /// Register a result type `T ! E`.
    pub fn register_result(&mut self, ok: TypeId, err: TypeId) -> TypeId {
        let name = format!("{} ! {}", self.display_name(ok), self.display_name(err));
        self.register(&name, TypeKind::Result { ok, err })
    }

    /// Register a box type `Box<T>`.
    pub fn register_box(&mut self, inner: TypeId) -> TypeId {
        let name = format!("Box<{}>", self.display_name(inner));
        self.register(&name, TypeKind::Box { inner })
    }

    /// Register a generic type parameter `<T>`.
    pub fn register_type_param(&mut self, name: &str) -> TypeId {
        let sym = Symbol::intern(name);
        self.register(name, TypeKind::TypeParam { name: sym })
    }

    /// Get type info by ID.
    pub fn get(&self, id: TypeId) -> Option<&TypeInfo> {
        self.types.get(&id)
    }

    /// Lookup type by name.
    pub fn lookup(&self, name: &str) -> Option<TypeId> {
        self.name_to_id.get(name).copied()
    }

    /// Get a human-readable display name for a TypeId.
    pub fn display_name(&self, id: TypeId) -> String {
        self.types
            .get(&id)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| format!("T{}", id.0))
    }

    /// Check if a type is Copy.
    pub fn is_copy(&self, id: TypeId) -> bool {
        self.types.get(&id).map(|t| t.is_copy()).unwrap_or(false)
    }

    /// Check if a type needs Drop.
    pub fn needs_drop(&self, id: TypeId) -> bool {
        self.types.get(&id).map(|t| t.needs_drop()).unwrap_or(true)
    }

    /// Iterate over all registered type IDs.
    pub fn all_type_ids(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.types.keys().copied()
    }

    /// Check type compatibility (structural).
    pub fn is_compatible(&self, actual: TypeId, expected: TypeId) -> bool {
        if actual == expected {
            return true;
        }

        // Any accepts everything
        if actual == TypeId(18) || expected == TypeId(18) {
            return true;
        }

        // TypeRef resolution
        if let Some(info) = self.get(actual) {
            if let TypeKind::TypeRef { name } = &info.kind {
                if let Some(&resolved_id) = self.name_to_id.get(name.resolve()) {
                    if resolved_id != actual {
                        return self.is_compatible(resolved_id, expected);
                    }
                }
            }
        }

        // Optional compatibility: T is assignable to T?
        if let Some(info) = self.get(expected) {
            if let TypeKind::Optional { inner } = &info.kind {
                return self.is_compatible(actual, *inner);
            }
        }

        false
    }
}

impl Default for TypeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Reserved TypeIds for built-in types
pub mod builtin {
    use super::TypeId;

    pub const VOID: TypeId = TypeId(16);
    pub const BOOL: TypeId = TypeId(1);
    pub const CHAR: TypeId = TypeId(2);
    pub const INT8: TypeId = TypeId(3);
    pub const INT16: TypeId = TypeId(4);
    pub const INT32: TypeId = TypeId(5);
    pub const INT64: TypeId = TypeId(6);
    pub const INT: TypeId = TypeId(7);
    pub const UINT8: TypeId = TypeId(8);
    pub const UINT16: TypeId = TypeId(9);
    pub const UINT32: TypeId = TypeId(10);
    pub const UINT64: TypeId = TypeId(11);
    pub const UINT: TypeId = TypeId(12);
    pub const FLOAT32: TypeId = TypeId(13);
    pub const FLOAT64: TypeId = TypeId(14);
    pub const FLOAT: TypeId = TypeId(14);
    pub const STR: TypeId = TypeId(15);
    pub const ANY: TypeId = TypeId(18);
    pub const ERROR: TypeId = TypeId(19);
    pub const PRIMITIVE_END: TypeId = TypeId(19);
}
