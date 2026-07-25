//! # Method Registry
//!
//! **SINGLE SOURCE OF TRUTH** for all builtin methods on types.
//! This module defines what methods exist on each type (String, Array, Map, Int, etc.)

use crate::symbol::Symbol;
use crate::types::registry::TypeId;
use rustc_hash::FxHashMap;

/// Check if a method name is a known built-in method across ALL types.
/// This is the single source of truth for "is this a builtin method?".
/// Used by the type checker and MIR builder to avoid false "undefined method" errors.
pub fn is_builtin_method(method: &str) -> bool {
    matches!(
        method,
        // Array methods
        "len" | "isEmpty" | "contains" | "indexOf" | "push" | "pop" | "sort" |
        "reverse" | "slice" | "clear" | "join" | "map" | "filter" | "reduce" |
        "first" | "last" | "insert" | "set" |
        // String methods
        "charAt" | "substring" | "concat" | "toUpper" | "toLower" | "replace" |
        "trim" | "startsWith" | "endsWith" | "repeat" | "charCode" | "countSubstr" |
        "split" |
        // Map methods
        "keys" | "values" | "has" | "remove" | "get" |
        // Int/Float/Bool methods
        "toStr" | "toChar"
    )
}

/// Method signature definition.
#[derive(Debug, Clone)]
pub struct MethodSig {
    /// Method name (interned).
    pub name: Symbol,
    /// Parameter types (excluding receiver).
    pub params: Vec<TypeId>,
    /// Return type.
    pub ret: TypeId,
    /// Whether this mutates the receiver.
    pub mutates: bool,
    /// Whether this is a static method (no receiver required).
    pub is_static: bool,
}

/// Registry of methods for all types.
/// Maps TypeId -> list of methods available on that type.
#[derive(Debug, Default)]
pub struct MethodRegistry {
    methods: FxHashMap<TypeId, Vec<MethodSig>>,
}

impl MethodRegistry {
    /// Create a new empty method registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a method for a type.
    pub fn register(&mut self, type_id: TypeId, method: MethodSig) {
        self.methods.entry(type_id).or_default().push(method);
    }

    /// Lookup a method by type and name.
    pub fn lookup(&self, type_id: TypeId, method_name: Symbol) -> Option<&MethodSig> {
        self.methods
            .get(&type_id)?
            .iter()
            .find(|m| m.name == method_name)
    }

    /// Check if a method exists for a type.
    pub fn has_method(&self, type_id: TypeId, method_name: Symbol) -> bool {
        self.lookup(type_id, method_name).is_some()
    }

    /// Get all methods for a type.
    pub fn methods_for_type(&self, type_id: TypeId) -> &[MethodSig] {
        self.methods
            .get(&type_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}
