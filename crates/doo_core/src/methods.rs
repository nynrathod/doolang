//! # Method Registry
//!
//! **SINGLE SOURCE OF TRUTH** for all builtin methods on types.
//! This module defines what methods exist on each type (String, Array, Map, Int, etc.)
//!
//! ## Architecture
//!
//! - MIR lowering checks this registry to validate method calls
//! - Codegen uses this registry to dispatch method implementations
//! - NO hardcoded method names elsewhere in the codebase

use crate::types::TypeId;

/// Method signature definition
#[derive(Debug, Clone)]
pub struct MethodDef {
    /// Method name
    pub name: &'static str,
    /// Parameter types (excluding receiver)
    pub params: &'static [&'static str],
    /// Return type name
    pub return_type: &'static str,
    /// Whether this mutates the receiver
    pub mutates: bool,
}

/// Get all methods for a type by name
pub fn get_methods_for_type(type_name: &str) -> &'static [MethodDef] {
    match type_name {
        "Str" | "String" => STRING_METHODS,
        "Int" => INT_METHODS,
        "Float" => FLOAT_METHODS,
        "Bool" => BOOL_METHODS,
        _ if type_name.starts_with("[") || type_name.starts_with("Array") => ARRAY_METHODS,
        _ if type_name.starts_with("{") || type_name.starts_with("Map") => MAP_METHODS,
        _ => &[],
    }
}

/// Check if a method exists for a type
pub fn has_method(type_name: &str, method_name: &str) -> bool {
    get_methods_for_type(type_name)
        .iter()
        .any(|m| m.name == method_name)
}

/// Check if a method mutates its receiver
pub fn method_mutates(type_name: &str, method_name: &str) -> bool {
    get_methods_for_type(type_name)
        .iter()
        .find(|m| m.name == method_name)
        .map(|m| m.mutates)
        .unwrap_or(false)
}

/// Get method definition by name
pub fn get_method(type_name: &str, method_name: &str) -> Option<&'static MethodDef> {
    get_methods_for_type(type_name)
        .iter()
        .find(|m| m.name == method_name)
}

// =============================================================================
// String Methods (16 methods)
// =============================================================================
pub static STRING_METHODS: &[MethodDef] = &[
    MethodDef { name: "len", params: &[], return_type: "Int", mutates: false },
    MethodDef { name: "charAt", params: &["Int"], return_type: "Str", mutates: false },
    MethodDef { name: "substring", params: &["Int", "Int"], return_type: "Str", mutates: false },
    MethodDef { name: "concat", params: &["Str"], return_type: "Str", mutates: false },
    MethodDef { name: "indexOf", params: &["Str"], return_type: "Int", mutates: false },
    MethodDef { name: "toUpper", params: &[], return_type: "Str", mutates: false },
    MethodDef { name: "toLower", params: &[], return_type: "Str", mutates: false },
    MethodDef { name: "replace", params: &["Str", "Str"], return_type: "Str", mutates: false },
    MethodDef { name: "trim", params: &[], return_type: "Str", mutates: false },
    MethodDef { name: "reverse", params: &[], return_type: "Str", mutates: false },
    MethodDef { name: "contains", params: &["Str"], return_type: "Bool", mutates: false },
    MethodDef { name: "startsWith", params: &["Str"], return_type: "Bool", mutates: false },
    MethodDef { name: "endsWith", params: &["Str"], return_type: "Bool", mutates: false },
    MethodDef { name: "repeat", params: &["Int"], return_type: "Str", mutates: false },
    MethodDef { name: "charCode", params: &[], return_type: "Int", mutates: false },
    MethodDef { name: "countSubstr", params: &["Str"], return_type: "Int", mutates: false },
];

// =============================================================================
// Array Methods (16 methods including lambda methods)
// =============================================================================
pub static ARRAY_METHODS: &[MethodDef] = &[
    MethodDef { name: "len", params: &[], return_type: "Int", mutates: false },
    MethodDef { name: "first", params: &[], return_type: "T", mutates: false },
    MethodDef { name: "last", params: &[], return_type: "T", mutates: false },
    MethodDef { name: "isEmpty", params: &[], return_type: "Bool", mutates: false },
    MethodDef { name: "push", params: &["T"], return_type: "Void", mutates: true },
    MethodDef { name: "pop", params: &[], return_type: "T", mutates: true },
    MethodDef { name: "contains", params: &["T"], return_type: "Bool", mutates: false },
    MethodDef { name: "indexOf", params: &["T"], return_type: "Int", mutates: false },
    MethodDef { name: "sort", params: &[], return_type: "Void", mutates: true },
    MethodDef { name: "reverse", params: &[], return_type: "Void", mutates: true },
    MethodDef { name: "slice", params: &["Int", "Int"], return_type: "[T]", mutates: false },
    MethodDef { name: "clear", params: &[], return_type: "Void", mutates: true },
    MethodDef { name: "join", params: &["Str"], return_type: "Str", mutates: false },
    // Lambda methods
    MethodDef { name: "map", params: &["(T) -> U"], return_type: "[U]", mutates: false },
    MethodDef { name: "filter", params: &["(T) -> Bool"], return_type: "[T]", mutates: false },
    MethodDef { name: "reduce", params: &["U", "(U, T) -> U"], return_type: "U", mutates: false },
];

// =============================================================================
// Map Methods (7 methods)
// =============================================================================
pub static MAP_METHODS: &[MethodDef] = &[
    MethodDef { name: "has", params: &["K"], return_type: "Bool", mutates: false },
    MethodDef { name: "size", params: &[], return_type: "Int", mutates: false },
    MethodDef { name: "isEmpty", params: &[], return_type: "Bool", mutates: false },
    MethodDef { name: "keys", params: &[], return_type: "[K]", mutates: false },
    MethodDef { name: "values", params: &[], return_type: "[V]", mutates: false },
    MethodDef { name: "remove", params: &["K"], return_type: "Void", mutates: true },
    MethodDef { name: "clear", params: &[], return_type: "Void", mutates: true },
];

// =============================================================================
// Int Methods (1 method)
// =============================================================================
pub static INT_METHODS: &[MethodDef] = &[
    MethodDef { name: "toChar", params: &[], return_type: "Str", mutates: false },
];

// =============================================================================
// Float Methods (0 methods currently)
// =============================================================================
pub static FLOAT_METHODS: &[MethodDef] = &[];

// =============================================================================
// Bool Methods (0 methods currently)
// =============================================================================
pub static BOOL_METHODS: &[MethodDef] = &[];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_methods() {
        assert!(has_method("Str", "len"));
        assert!(has_method("Str", "charAt"));
        assert!(has_method("Str", "contains"));
        assert!(!has_method("Str", "nonexistent"));
    }

    #[test]
    fn test_array_methods() {
        assert!(has_method("[Int]", "len"));
        assert!(has_method("Array(Int)", "push"));
        assert!(has_method("[Str]", "join"));
    }

    #[test]
    fn test_map_methods() {
        assert!(has_method("{Str: Int}", "has"));
        assert!(has_method("Map(Str,Int)", "size"));
    }
}
