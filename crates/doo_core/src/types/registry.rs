//! # Type Registry - Single Source of Truth
//!
//! All types in the Doo compiler flow through this registry.
//! This is the ONLY place where types are defined.
//!
//! ## Design Philosophy
//!
//! - **Centralized**: One registry, all types
//! - **TypeId-based**: Fast lookup, cheap copies
//! - **Interned**: Types are stored once, referenced by ID
//! - **Extensible**: Easy to add new primitive or composite types

use crate::doo_debug;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================================
// Type IDs
// ============================================================================

/// Unique identifier for a type.
/// This is the ONLY way to reference types throughout the compiler.
/// Using a newtype for type safety instead of bare u32.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct TypeId(pub u32);

impl TypeId {
    /// Create a new TypeId from a u32
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    /// Get the raw u32 value
    pub const fn raw(&self) -> u32 {
        self.0
    }
}

impl From<u32> for TypeId {
    fn from(id: u32) -> Self {
        Self(id)
    }
}

impl From<TypeId> for u32 {
    fn from(id: TypeId) -> Self {
        id.0
    }
}

impl std::ops::AddAssign<u32> for TypeId {
    fn add_assign(&mut self, rhs: u32) {
        self.0 += rhs;
    }
}

impl std::ops::Add<u32> for TypeId {
    type Output = Self;
    fn add(self, rhs: u32) -> Self {
        Self(self.0 + rhs)
    }
}

/// Reserved TypeIds for built-in types
pub mod builtin {
    use super::TypeId;

    pub const VOID: TypeId = TypeId(0);
    pub const BOOL: TypeId = TypeId(1);
    pub const INT: TypeId = TypeId(2);
    pub const FLOAT: TypeId = TypeId(3);
    pub const STR: TypeId = TypeId(4);
    pub const ANY: TypeId = TypeId(5);
    pub const ERROR: TypeId = TypeId(6);

    // Reserved range for primitives: 0-99
    pub const PRIMITIVE_END: u32 = 99;
}

// ============================================================================
// Type Kind
// ============================================================================

/// The kind of a type - what category it belongs to.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    /// Void (no value)
    Void,
    /// Boolean
    Bool,
    /// 64-bit signed integer
    Int,
    /// 64-bit float
    Float,
    /// UTF-8 string
    Str,
    /// Array of elements
    Array { element: TypeId },
    /// Map from keys to values
    Map { key: TypeId, value: TypeId },
    /// Optional value (T?)
    Optional { inner: TypeId },
    /// Result type (T ! E)
    Result { ok: TypeId, err: TypeId },
    /// Tuple of types
    Tuple { elements: Vec<TypeId> },
    /// Struct with named fields
    /// Fields are (name, type_id, is_public)
    Struct {
        name: String,
        fields: Vec<(String, TypeId, bool)>,
    },
    /// Enum with variants
    Enum {
        name: String,
        variants: Vec<(String, Option<TypeId>)>,
    },
    /// Function type
    Function {
        params: Vec<TypeId>,
        returns: TypeId,
    },
    /// Reference to a named type (before resolution)
    TypeRef { name: String },
    /// Any type (for JSON.parse, etc.)
    Any,
    /// Error type
    Error,
}

// ============================================================================
// Type Info
// ============================================================================

/// Complete information about a type.
#[derive(Debug, Clone)]
pub struct TypeInfo {
    /// The type's unique ID
    pub id: TypeId,
    /// The kind of type
    pub kind: TypeKind,
    /// Display name (for error messages)
    pub name: String,
}

impl TypeInfo {
    /// Check if this type is Copy (can be copied without cloning)
    pub fn is_copy(&self) -> bool {
        matches!(
            self.kind,
            TypeKind::Void | TypeKind::Bool | TypeKind::Int | TypeKind::Float
        )
    }

    /// Check if this type needs Drop cleanup
    pub fn needs_drop(&self) -> bool {
        !self.is_copy()
    }

    /// Get size in bytes (for ABI)
    pub fn size_bytes(&self) -> usize {
        match &self.kind {
            TypeKind::Void => 0,
            TypeKind::Bool => 1,
            TypeKind::Int => 8,
            TypeKind::Float => 8,
            TypeKind::Str => 16,                                 // ptr + len
            TypeKind::Array { .. } => 24,                        // ptr + len + cap
            TypeKind::Map { .. } => 8,                           // ptr to hashmap
            TypeKind::Optional { .. } => 16,                     // tag + value
            TypeKind::Result { .. } => 24,                       // tag + ok + err
            TypeKind::Tuple { elements } => elements.len() * 8,  // Simplified
            TypeKind::Struct { fields, .. } => fields.len() * 8, // Simplified
            TypeKind::Enum { .. } => 16,                         // tag + max payload
            TypeKind::Function { .. } => 8,                      // function pointer
            TypeKind::TypeRef { .. } => 8,                       // Will be resolved
            TypeKind::Any => 16,                                 // tag + ptr
            TypeKind::Error => 16,                               // ptr + len
        }
    }

    /// Get LLVM type name
    pub fn llvm_type(&self) -> &'static str {
        match &self.kind {
            TypeKind::Void => "void",
            TypeKind::Bool => "i1",
            TypeKind::Int => "i64",
            TypeKind::Float => "double",
            TypeKind::Str => "ptr",
            TypeKind::Array { .. } => "ptr",
            TypeKind::Map { .. } => "ptr",
            TypeKind::Optional { .. } => "ptr",
            TypeKind::Result { .. } => "ptr",
            TypeKind::Tuple { .. } => "ptr",
            TypeKind::Struct { .. } => "ptr",
            TypeKind::Enum { .. } => "ptr",
            TypeKind::Function { .. } => "ptr",
            TypeKind::TypeRef { .. } => "ptr",
            TypeKind::Any => "ptr",
            TypeKind::Error => "ptr",
        }
    }
}

// ============================================================================
// Type Registry
// ============================================================================

/// The central type registry - SINGLE SOURCE OF TRUTH for all types.
#[derive(Debug)]
pub struct TypeRegistry {
    /// All registered types
    types: HashMap<TypeId, TypeInfo>,
    /// Name to ID lookup for named types
    name_to_id: HashMap<String, TypeId>,
    /// Next available type ID
    next_id: TypeId,
}

impl TypeRegistry {
    /// Create a new registry with built-in types
    pub fn new() -> Self {
        let mut registry = Self {
            types: HashMap::new(),
            name_to_id: HashMap::new(),
            next_id: TypeId(builtin::PRIMITIVE_END + 1),
        };

        // Register built-in primitives
        registry.register_builtin(builtin::VOID, "Void", TypeKind::Void);
        registry.register_builtin(builtin::BOOL, "Bool", TypeKind::Bool);
        registry.register_builtin(builtin::INT, "Int", TypeKind::Int);
        registry.register_builtin(builtin::FLOAT, "Float", TypeKind::Float);
        registry.register_builtin(builtin::STR, "Str", TypeKind::Str);
        registry.register_builtin(builtin::ANY, "Any", TypeKind::Any);
        registry.register_builtin(builtin::ERROR, "Error", TypeKind::Error);

        registry
    }

    /// Register a built-in type with a specific ID
    fn register_builtin(&mut self, id: TypeId, name: &str, kind: TypeKind) {
        let info = TypeInfo {
            id,
            kind,
            name: name.to_string(),
        };
        self.types.insert(id, info);
        self.name_to_id.insert(name.to_string(), id);
    }

    /// Register a new type and return its ID
    pub fn register(&mut self, name: &str, kind: TypeKind) -> TypeId {
        // Check if already registered
        if let Some(&id) = self.name_to_id.get(name) {
            return id;
        }

        let id = self.next_id;
        self.next_id += 1;

        let info = TypeInfo {
            id,
            kind,
            name: name.to_string(),
        };

        self.types.insert(id, info);
        self.name_to_id.insert(name.to_string(), id);

        id
    }

    pub fn declare_named(&mut self, name: &str) -> TypeId {
        if let Some(&id) = self.name_to_id.get(name) {
            return id;
        }
        self.register(
            name,
            TypeKind::TypeRef {
                name: name.to_string(),
            },
        )
    }

    pub fn define_struct(&mut self, name: &str, fields: Vec<(String, TypeId, bool)>) -> TypeId {
        let id = self.declare_named(name);
        if std::env::var(crate::constants::env_vars::DOO_DEBUG_TYPES).is_ok() {
            doo_debug!(
                "TYPES",
                "define_struct '{}' with id={:?}, fields={:?}",
                name,
                id,
                fields
            );
        }
        if let Some(info) = self.types.get_mut(&id) {
            info.kind = TypeKind::Struct {
                name: name.to_string(),
                fields,
            };
            info.name = name.to_string();
        }
        id
    }

    pub fn define_enum(&mut self, name: &str, variants: Vec<(String, Option<TypeId>)>) -> TypeId {
        let id = self.declare_named(name);
        if std::env::var(crate::constants::env_vars::DOO_DEBUG_TYPES).is_ok() {
            doo_debug!(
                "TYPES",
                "define_enum '{}' with id={:?}, variants={:?}",
                name,
                id,
                variants.iter().map(|(n, _)| n).collect::<Vec<_>>()
            );
        }
        if let Some(info) = self.types.get_mut(&id) {
            info.kind = TypeKind::Enum {
                name: name.to_string(),
                variants,
            };
            info.name = name.to_string();
        }
        id
    }

    /// Register an array type
    pub fn register_array(&mut self, element: TypeId) -> TypeId {
        let name = format!(
            "[{}]",
            self.get(element)
                .map(|t| &t.name)
                .unwrap_or(&"?".to_string())
        );
        self.register(&name, TypeKind::Array { element })
    }

    /// Register a map type
    pub fn register_map(&mut self, key: TypeId, value: TypeId) -> TypeId {
        let key_name = self
            .get(key)
            .map(|t| &t.name)
            .unwrap_or(&"?".to_string())
            .clone();
        let val_name = self
            .get(value)
            .map(|t| &t.name)
            .unwrap_or(&"?".to_string())
            .clone();
        let name = format!("Map<{}, {}>", key_name, val_name);
        self.register(&name, TypeKind::Map { key, value })
    }

    pub fn register_tuple(&mut self, elements: Vec<TypeId>) -> TypeId {
        let parts: Vec<String> = elements
            .iter()
            .map(|id| {
                self.get(*id)
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| "?".to_string())
            })
            .collect();
        let name = format!("({})", parts.join(", "));
        self.register(&name, TypeKind::Tuple { elements })
    }

    pub fn register_function(&mut self, params: Vec<TypeId>, returns: TypeId) -> TypeId {
        let params_s: Vec<String> = params
            .iter()
            .map(|id| {
                self.get(*id)
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| "?".to_string())
            })
            .collect();
        let returns_s = self
            .get(returns)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| "?".to_string());
        let name = format!("({}) -> {}", params_s.join(", "), returns_s);
        self.register(&name, TypeKind::Function { params, returns })
    }

    /// Register an optional type
    pub fn register_optional(&mut self, inner: TypeId) -> TypeId {
        let name = format!(
            "{}?",
            self.get(inner).map(|t| &t.name).unwrap_or(&"?".to_string())
        );
        self.register(&name, TypeKind::Optional { inner })
    }

    /// Register a result type
    pub fn register_result(&mut self, ok: TypeId, err: TypeId) -> TypeId {
        let ok_name = self
            .get(ok)
            .map(|t| &t.name)
            .unwrap_or(&"?".to_string())
            .clone();
        let err_name = self
            .get(err)
            .map(|t| &t.name)
            .unwrap_or(&"?".to_string())
            .clone();
        let name = format!("{} ! {}", ok_name, err_name);
        self.register(&name, TypeKind::Result { ok, err })
    }

    /// Register a struct type
    pub fn register_struct(&mut self, name: &str, fields: Vec<(String, TypeId, bool)>) -> TypeId {
        self.register(
            name,
            TypeKind::Struct {
                name: name.to_string(),
                fields,
            },
        )
    }

    /// Register an enum type
    pub fn register_enum(&mut self, name: &str, variants: Vec<(String, Option<TypeId>)>) -> TypeId {
        self.register(
            name,
            TypeKind::Enum {
                name: name.to_string(),
                variants,
            },
        )
    }

    /// Get type info by ID
    pub fn get(&self, id: TypeId) -> Option<&TypeInfo> {
        self.types.get(&id)
    }

    /// Lookup type by name
    pub fn lookup(&self, name: &str) -> Option<TypeId> {
        self.name_to_id.get(name).copied()
    }

    /// Get all registered type IDs
    pub fn all_type_ids(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.types.keys().copied()
    }

    /// Check if a type is Copy (bitwise copyable)
    pub fn is_copy(&self, id: TypeId) -> bool {
        self.get(id).map(|t| t.is_copy()).unwrap_or(false)
    }

    /// Check if a type needs Drop
    pub fn needs_drop(&self, id: TypeId) -> bool {
        self.get(id).map(|t| t.needs_drop()).unwrap_or(true)
    }

    /// Check type compatibility
    pub fn is_compatible(&self, actual: TypeId, expected: TypeId) -> bool {
        if actual == expected {
            return true;
        }

        // Any is compatible with everything
        if actual == builtin::ANY || expected == builtin::ANY {
            return true;
        }

        // nil (VOID) is compatible with any Optional type
        if actual == builtin::VOID {
            if let Some(e) = self.get(expected) {
                if matches!(e.kind, TypeKind::Optional { .. }) {
                    return true;
                }
            }
        }

        // Check structural compatibility
        match (self.get(actual), self.get(expected)) {
            (Some(a), Some(e)) => {
                match (&a.kind, &e.kind) {
                    // Array covariance
                    (TypeKind::Array { element: a_elem }, TypeKind::Array { element: e_elem }) => {
                        self.is_compatible(*a_elem, *e_elem)
                    }
                    // Map covariance
                    (
                        TypeKind::Map {
                            key: a_key,
                            value: a_val,
                        },
                        TypeKind::Map {
                            key: e_key,
                            value: e_val,
                        },
                    ) => self.is_compatible(*a_key, *e_key) && self.is_compatible(*a_val, *e_val),
                    // Optional covariance
                    (
                        TypeKind::Optional { inner: a_inner },
                        TypeKind::Optional { inner: e_inner },
                    ) => self.is_compatible(*a_inner, *e_inner),
                    // T is assignable to Optional<T>
                    (_, TypeKind::Optional { inner: e_inner }) => {
                        self.is_compatible(actual, *e_inner)
                    }
                    // TypeRef resolves to actual type (guard against self-referential TypeRefs)
                    (TypeKind::TypeRef { name }, _) => self
                        .lookup(name)
                        .filter(|&id| id != actual)
                        .map(|id| self.is_compatible(id, expected))
                        .unwrap_or(false),
                    (_, TypeKind::TypeRef { name }) => self
                        .lookup(name)
                        .filter(|&id| id != expected)
                        .map(|id| self.is_compatible(actual, id))
                        .unwrap_or(false),
                    _ => false,
                }
            }
            _ => false,
        }
    }
}

impl Default for TypeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_types() {
        let reg = TypeRegistry::new();

        assert!(reg.get(builtin::INT).is_some());
        assert!(reg.get(builtin::STR).is_some());
        assert!(reg.get(builtin::BOOL).is_some());

        assert_eq!(reg.lookup("Int"), Some(builtin::INT));
        assert_eq!(reg.lookup("Str"), Some(builtin::STR));
    }

    #[test]
    fn test_is_copy() {
        let reg = TypeRegistry::new();

        assert!(reg.is_copy(builtin::INT));
        assert!(reg.is_copy(builtin::FLOAT));
        assert!(reg.is_copy(builtin::BOOL));
        assert!(!reg.is_copy(builtin::STR)); // Strings are not Copy
    }

    #[test]
    fn test_register_array() {
        let mut reg = TypeRegistry::new();

        let int_array = reg.register_array(builtin::INT);
        assert!(reg.get(int_array).is_some());

        let info = reg.get(int_array).unwrap();
        assert!(matches!(info.kind, TypeKind::Array { element } if element == builtin::INT));
    }

    #[test]
    fn test_register_struct() {
        let mut reg = TypeRegistry::new();

        let user_id = reg.register_struct(
            "User",
            vec![
                ("id".to_string(), builtin::INT, false),
                ("name".to_string(), builtin::STR, false),
            ],
        );

        assert!(reg.get(user_id).is_some());
        assert_eq!(reg.lookup("User"), Some(user_id));
    }

    #[test]
    fn test_type_compatibility() {
        let reg = TypeRegistry::new();

        assert!(reg.is_compatible(builtin::INT, builtin::INT));
        assert!(!reg.is_compatible(builtin::INT, builtin::STR));
        assert!(reg.is_compatible(builtin::INT, builtin::ANY)); // Any accepts all
    }
}
