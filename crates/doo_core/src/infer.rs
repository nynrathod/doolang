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

use crate::types::{builtin, TypeId};

/// Binary operator types (matches HIR and frontend definitions)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOpKind {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    /// Nil coalescing: a ?? b — returns a if a != nil, else b.
    /// Result type is the common type of both operands (both must match).
    NullCoalesce,
}

/// Unary operator types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOpKind {
    Neg,
    Not,
}

/// Infer the result type of a binary operation.
/// This is the SINGLE SOURCE OF TRUTH for binary operation type rules.
pub fn infer_binop_result_type(op: BinOpKind, lhs: TypeId, rhs: TypeId) -> TypeId {
    use BinOpKind::*;
    match op {
        // Comparison operators always return Bool
        Eq | Ne | Lt | Le | Gt | Ge => builtin::BOOL,
        // Logical operators return Bool
        And | Or => builtin::BOOL,
        // Nil coalescing: a ?? b returns the non-nil type.
        // If lhs is nil (VOID), use rhs type. Otherwise use lhs type.
        NullCoalesce => {
            if lhs == builtin::VOID || lhs == builtin::ANY {
                rhs
            } else {
                lhs
            }
        }
        // Arithmetic operators return the operand type
        Add | Sub | Mul | Div | Mod => {
            // Int + Int = Int
            if lhs == builtin::INT && rhs == builtin::INT {
                builtin::INT
            // Any Float operand promotes to Float
            } else if lhs == builtin::FLOAT || rhs == builtin::FLOAT {
                builtin::FLOAT
            // String concatenation
            } else if lhs == builtin::STR && op == Add {
                builtin::STR
            // Prefer non-ANY type
            } else if lhs != builtin::ANY {
                lhs
            } else if rhs != builtin::ANY {
                rhs
            } else {
                builtin::ANY
            }
        }
    }
}

/// Infer the result type of a unary operation.
/// This is the SINGLE SOURCE OF TRUTH for unary operation type rules.
pub fn infer_unaryop_result_type(op: UnaryOpKind, operand: TypeId) -> TypeId {
    match op {
        UnaryOpKind::Not => builtin::BOOL,
        UnaryOpKind::Neg => operand, // -Int = Int, -Float = Float
    }
}

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
    }

    #[test]
    fn test_logical_types() {
        assert_eq!(
            infer_binop_result_type(BinOpKind::And, builtin::BOOL, builtin::BOOL),
            builtin::BOOL
        );
    }

    #[test]
    fn test_string_concat() {
        assert_eq!(
            infer_binop_result_type(BinOpKind::Add, builtin::STR, builtin::STR),
            builtin::STR
        );
    }
}
