//! # Type Inference Utilities
//!
//! **SINGLE SOURCE OF TRUTH** for basic type inference rules.
//! This module provides shared type inference logic used by HIR and MIR builders.
//!
//! ## Design Philosophy
//!
//! - Centralized inference rules, no duplication
//! - Stateless functions, no context required
//! - Used by both HIR lowering and MIR building

use crate::types::builtin;
use crate::types::registry::{TypeId, TypeKind, TypeRegistry};
use rustc_hash::FxHashMap;

// ============================================================================
// Operator Types
// ============================================================================

/// Binary operator types (matches HIR and frontend definitions).
/// This is the SINGLE SOURCE OF TRUTH for binary operator classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOpKind {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    // Comparison
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    // Logical
    And,
    Or,
    // Bitwise
    BitAnd,
    BitOr,
    // Membership
    In,
    /// Nil coalescing: a ?? b — returns a if a != nil, else b.
    /// Result type is the common type of both operands (both must match).
    NullCoalesce,
}

/// Unary operator types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOpKind {
    Neg,    // -x
    Not,    // !x (logical NOT)
    BitNot, // ~x (bitwise NOT)
}

// ============================================================================
// Type Inference Rules
// ============================================================================

/// Infer the result type of a binary operation.
/// This is the SINGLE SOURCE OF TRUTH for binary operation type rules.
pub fn infer_binop_result_type(op: BinOpKind, lhs: TypeId, rhs: TypeId) -> TypeId {
    // Comparison operators always return Bool
    if matches!(
        op,
        BinOpKind::Eq
            | BinOpKind::NotEq
            | BinOpKind::Lt
            | BinOpKind::LtEq
            | BinOpKind::Gt
            | BinOpKind::GtEq
            | BinOpKind::In
    ) {
        return builtin::BOOL;
    }

    // Logical operators return Bool
    if matches!(op, BinOpKind::And | BinOpKind::Or) {
        return builtin::BOOL;
    }

    // Bitwise operators return Int (or Bool if both operands are Bool)
    if matches!(op, BinOpKind::BitAnd | BinOpKind::BitOr) {
        if lhs == builtin::BOOL || rhs == builtin::BOOL {
            return builtin::BOOL;
        }
        return builtin::INT;
    }

    // Null coalescing: a ?? b returns the non-nil type.
    // If lhs is nil (VOID), use rhs type. Otherwise use lhs type.
    if op == BinOpKind::NullCoalesce {
        if lhs == builtin::VOID || lhs == builtin::ANY {
            return rhs;
        }
        return lhs;
    }

    // Arithmetic operators (Add, Sub, Mul, Div, Mod)
    // Int + Int = Int
    if lhs == builtin::INT && rhs == builtin::INT {
        return builtin::INT;
    }

    // Any Float operand promotes to Float
    if lhs == builtin::FLOAT || rhs == builtin::FLOAT {
        return builtin::FLOAT;
    }

    // String concatenation: Str + anything → Str (auto-coercion)
    // Doo allows "hello" + 42 → "hello42"
    if op == BinOpKind::Add && (lhs == builtin::STR || rhs == builtin::STR) {
        return builtin::STR;
    }

    // Prefer non-ANY type
    if lhs != builtin::ANY {
        lhs
    } else {
        rhs
    }
}

/// Infer the result type of a unary operation.
/// This is the SINGLE SOURCE OF TRUTH for unary operation type rules.
pub fn infer_unaryop_result_type(op: UnaryOpKind, operand: TypeId) -> TypeId {
    match op {
        UnaryOpKind::Not => builtin::BOOL,   // !x is always Bool
        UnaryOpKind::BitNot => builtin::INT, // ~x is always Int
        UnaryOpKind::Neg => operand,         // -Int = Int, -Float = Float
    }
}

// ============================================================================
// Type Variable Table (For Generic Inference)
// ============================================================================

/// A table for managing type variables during generic inference.
/// Maps TypeParams (TypeVars) to concrete TypeIds.
///
/// Used during monomorphization (Phase 20) to substitute generic type
/// parameters with concrete types inferred from call sites.
#[derive(Debug, Clone, Default)]
pub struct TypeVarTable {
    /// Maps TypeParam TypeId -> concrete TypeId
    bindings: FxHashMap<TypeId, TypeId>,
}

impl TypeVarTable {
    /// Create a new empty type variable table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind a type variable to a concrete type.
    pub fn bind(&mut self, var: TypeId, ty: TypeId) {
        self.bindings.insert(var, ty);
    }

    /// Lookup the concrete type for a type variable.
    pub fn lookup(&self, var: TypeId) -> Option<TypeId> {
        self.bindings.get(&var).copied()
    }

    /// Check if a type variable has been bound.
    pub fn is_bound(&self, var: TypeId) -> bool {
        self.bindings.contains_key(&var)
    }

    /// Substitute all type variables in a type with their bindings.
    /// This recursively walks composite types (Arrays, Maps, Structs, etc.)
    /// and replaces any TypeParam TypeIds with their bound concrete types.
    ///
    /// Returns a new TypeId registered in the TypeRegistry.
    pub fn substitute(&self, ty: TypeId, registry: &mut TypeRegistry) -> TypeId {
        // Clone the TypeKind to release the immutable borrow on `registry`
        // before we make recursive calls or mutable `register_*` calls.
        let kind = registry.get(ty).map(|info| info.kind.clone());

        match kind {
            // If this type is directly a TypeParam, return its binding (or itself if unbound)
            Some(TypeKind::TypeParam { .. }) => self.bindings.get(&ty).copied().unwrap_or(ty),

            // For composite types, recursively substitute inner types
            Some(TypeKind::Array { element }) => {
                let new_elem = self.substitute(element, registry);
                registry.register_array(new_elem)
            }
            Some(TypeKind::Map { key, value }) => {
                let new_key = self.substitute(key, registry);
                let new_val = self.substitute(value, registry);
                registry.register_map(new_key, new_val)
            }
            Some(TypeKind::Set { element }) => {
                let new_elem = self.substitute(element, registry);
                registry.register_set(new_elem)
            }
            Some(TypeKind::Optional { inner }) => {
                let new_inner = self.substitute(inner, registry);
                registry.register_optional(new_inner)
            }
            Some(TypeKind::Result { ok, err }) => {
                let new_ok = self.substitute(ok, registry);
                let new_err = self.substitute(err, registry);
                registry.register_result(new_ok, new_err)
            }
            Some(TypeKind::Box { inner }) => {
                let new_inner = self.substitute(inner, registry);
                registry.register_box(new_inner)
            }
            Some(TypeKind::Tuple { elements }) => {
                let new_elements: Vec<TypeId> = elements
                    .iter()
                    .map(|&e| self.substitute(e, registry))
                    .collect();
                registry.register_tuple(new_elements)
            }
            Some(TypeKind::Function { sig }) => {
                let new_params: Vec<TypeId> = sig
                    .params
                    .iter()
                    .map(|&p| self.substitute(p, registry))
                    .collect();
                let new_ret = self.substitute(sig.return_type, registry);
                let new_err = sig.error_type.map(|e| self.substitute(e, registry));
                registry.register_function(crate::types::composite::FunctionSig {
                    params: new_params,
                    return_type: new_ret,
                    error_type: new_err,
                    is_closure: sig.is_closure,
                })
            }

            // Struct, Enum, Interface: keep as-is (their fields are resolved separately)
            // TypeRef, TypeParam, SelfType: return as-is
            _ => ty,
        }
    }
    /// Get all bindings as a slice of (var, concrete) pairs.
    pub fn bindings(&self) -> &FxHashMap<TypeId, TypeId> {
        &self.bindings
    }

    /// Clear all bindings.
    pub fn clear(&mut self) {
        self.bindings.clear();
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arithmetic_types() {
        assert_eq!(
            infer_binop_result_type(BinOpKind::Add, builtin::INT, builtin::INT),
            builtin::INT
        );
        assert_eq!(
            infer_binop_result_type(BinOpKind::Mul, builtin::INT, builtin::INT),
            builtin::INT
        );
        assert_eq!(
            infer_binop_result_type(BinOpKind::Add, builtin::FLOAT, builtin::INT),
            builtin::FLOAT
        );
    }

    #[test]
    fn test_comparison_types() {
        assert_eq!(
            infer_binop_result_type(BinOpKind::Lt, builtin::INT, builtin::INT),
            builtin::BOOL
        );
        assert_eq!(
            infer_binop_result_type(BinOpKind::Eq, builtin::STR, builtin::STR),
            builtin::BOOL
        );
        assert_eq!(
            infer_binop_result_type(BinOpKind::NotEq, builtin::INT, builtin::FLOAT),
            builtin::BOOL
        );
    }

    #[test]
    fn test_logical_types() {
        assert_eq!(
            infer_binop_result_type(BinOpKind::And, builtin::BOOL, builtin::BOOL),
            builtin::BOOL
        );
    }

    #[test]
    fn test_bitwise_types() {
        assert_eq!(
            infer_binop_result_type(BinOpKind::BitAnd, builtin::INT, builtin::INT),
            builtin::INT
        );
        assert_eq!(
            infer_binop_result_type(BinOpKind::BitOr, builtin::BOOL, builtin::BOOL),
            builtin::BOOL
        );
    }

    #[test]
    fn test_string_concat() {
        // Str + Str = Str
        assert_eq!(
            infer_binop_result_type(BinOpKind::Add, builtin::STR, builtin::STR),
            builtin::STR
        );
        // Str + Int = Str (auto-coercion)
        assert_eq!(
            infer_binop_result_type(BinOpKind::Add, builtin::STR, builtin::INT),
            builtin::STR
        );
        // Int + Str = Str (auto-coercion)
        assert_eq!(
            infer_binop_result_type(BinOpKind::Add, builtin::INT, builtin::STR),
            builtin::STR
        );
    }

    #[test]
    fn test_null_coalesce() {
        // nil ?? Int = Int
        assert_eq!(
            infer_binop_result_type(BinOpKind::NullCoalesce, builtin::VOID, builtin::INT),
            builtin::INT
        );
        // Int ?? Int = Int
        assert_eq!(
            infer_binop_result_type(BinOpKind::NullCoalesce, builtin::INT, builtin::INT),
            builtin::INT
        );
    }

    #[test]
    fn test_in_operator() {
        assert_eq!(
            infer_binop_result_type(BinOpKind::In, builtin::INT, builtin::ANY),
            builtin::BOOL
        );
    }

    #[test]
    fn test_unary_neg() {
        assert_eq!(
            infer_unaryop_result_type(UnaryOpKind::Neg, builtin::INT),
            builtin::INT
        );
        assert_eq!(
            infer_unaryop_result_type(UnaryOpKind::Neg, builtin::FLOAT),
            builtin::FLOAT
        );
    }

    #[test]
    fn test_unary_not() {
        assert_eq!(
            infer_unaryop_result_type(UnaryOpKind::Not, builtin::BOOL),
            builtin::BOOL
        );
    }

    #[test]
    fn test_unary_bitnot() {
        assert_eq!(
            infer_unaryop_result_type(UnaryOpKind::BitNot, builtin::INT),
            builtin::INT
        );
    }

    #[test]
    fn test_type_var_table() {
        let mut table = TypeVarTable::new();
        let var = TypeId::new(999);
        let concrete = builtin::INT;

        // Before binding
        assert!(!table.is_bound(var));
        assert_eq!(table.lookup(var), None);

        // After binding
        table.bind(var, concrete);
        assert!(table.is_bound(var));
        assert_eq!(table.lookup(var), Some(concrete));

        // Clear
        table.clear();
        assert!(!table.is_bound(var));
    }

    #[test]
    fn test_type_var_substitute() {
        let mut registry = TypeRegistry::new();
        let mut table = TypeVarTable::new();

        // Create a type param
        let tp = registry.register_type_param("T");

        // Create an array of the type param: [T]
        let arr_of_tp = registry.register_array(tp);

        // Bind T -> Int
        table.bind(tp, builtin::INT);

        // Substitute: [T] should become [Int]
        let result = table.substitute(arr_of_tp, &mut registry);

        // Verify the result is Array<Int>
        let info = registry.get(result).unwrap();
        match &info.kind {
            TypeKind::Array { element } => {
                assert_eq!(*element, builtin::INT);
            }
            _ => panic!("Expected Array type, got {:?}", info.kind),
        }
    }
}
