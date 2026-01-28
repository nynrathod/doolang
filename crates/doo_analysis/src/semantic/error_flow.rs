//! Error Flow Analysis
//!
//! Tracks Result type flows through expressions and ensures all errors are handled.
//!
//! ## Responsibilities
//!
//! - Track Result-returning function calls
//! - Verify that Result values are properly handled:
//!   - Auto-propagation with `?` operator
//!   - Manual extraction with `let result, err = ...`
//! - Report unhandled Result errors
//!
//! ## Rules
//!
//! 1. A function call returning `T ! E` (Result type) must be handled:
//!    - Using `?` operator for auto-propagation
//!    - Using `let ok, err = call()` for manual extraction
//! 2. Unhandled Result values generate `UnhandledResult` errors

use doo_core::{
    types::{TypeId, TypeKind, TypeRegistry},
    Span,
};
use doo_hir::{HirExpr, HirExprKind, HirFunction, HirItem, HirProgram, HirStmt, HirStmtKind};

// ============================================================================
// Error Types
// ============================================================================

/// Error flow analysis error.
#[derive(Debug, Clone)]
pub struct ErrorFlowError {
    pub kind: ErrorFlowErrorKind,
    pub span: Span,
}

impl ErrorFlowError {
    pub fn new(kind: ErrorFlowErrorKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// Kinds of error flow errors.
#[derive(Debug, Clone)]
pub enum ErrorFlowErrorKind {
    /// Result type not handled (no `?` or manual extraction).
    UnhandledResult {
        /// The Ok type of the Result.
        ok_type: TypeId,
        /// The Error type of the Result.
        err_type: TypeId,
    },
    /// Using `?` in a function that doesn't return a Result.
    TryInNonResultFunction {
        /// Name of the function.
        func_name: String,
    },
    /// Using `Err` in a function that doesn't have an error type.
    ErrInNonResultFunction {
        /// Name of the function.
        func_name: String,
    },
    /// Missing Ok on some paths when function returns Result.
    MissingOkPath {
        /// Name of the function.
        func_name: String,
    },
    /// Using `??` (panic) without a message.
    PanicWithoutMessage,
}

// ============================================================================
// Error Flow Checker
// ============================================================================

/// Checks that Result types are properly handled throughout the program.
pub struct ErrorFlowChecker<'a> {
    /// Type registry for looking up types.
    registry: &'a TypeRegistry,
    /// Collected errors.
    errors: Vec<ErrorFlowError>,
    /// Current function's error type (if any).
    current_func_error_type: Option<TypeId>,
    /// Current function name (for error messages).
    current_func_name: String,
}

impl<'a> ErrorFlowChecker<'a> {
    /// Create a new error flow checker.
    pub fn new(registry: &'a TypeRegistry) -> Self {
        Self {
            registry,
            errors: Vec::new(),
            current_func_error_type: None,
            current_func_name: String::new(),
        }
    }

    /// Check an entire program for error flow issues.
    pub fn check(&mut self, program: &HirProgram) -> Result<(), Vec<ErrorFlowError>> {
        for item in &program.items {
            if let HirItem::Function(func) = item {
                self.check_function(func);
            }
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.clone())
        }
    }

    /// Check a single function for error flow issues.
    pub fn check_function(&mut self, func: &HirFunction) {
        // Store current function context
        self.current_func_name = func.name.clone();
        self.current_func_error_type = func.error_type;

        // Check body statements
        for stmt in &func.body {
            self.check_stmt(stmt);
        }

        // Clear context
        self.current_func_error_type = None;
        self.current_func_name.clear();
    }

    /// Check a statement for error flow issues.
    fn check_stmt(&mut self, stmt: &HirStmt) {
        match &stmt.kind {
            HirStmtKind::Let { value, .. } => {
                // Check the value expression - Result handling is done at expression level
                // The Let statement itself handles binding, not Result extraction
                // ManualErrorExtract is used for `let ok, err = expr`
                self.check_expr(value);
            }
            HirStmtKind::TupleLet { value, .. } => {
                // Check the value expression for tuple unpacking
                self.check_expr(value);
            }
            HirStmtKind::ManualErrorExtract { expr, .. } => {
                // This is valid manual error extraction - just check the inner expression
                self.check_expr(expr);
            }
            HirStmtKind::Expr(expr) => {
                // Check if this is an unhandled Result type
                if let Some(type_id) = expr.type_id {
                    if let Some(type_info) = self.registry.get(type_id) {
                        if let TypeKind::Result { ok, err } = &type_info.kind {
                            // Result used as expression statement without handling
                            // Unless it's a Try expression (handled by ?)
                            if !self.is_try_expr(expr) {
                                self.errors.push(ErrorFlowError::new(
                                    ErrorFlowErrorKind::UnhandledResult {
                                        ok_type: *ok,
                                        err_type: *err,
                                    },
                                    stmt.span,
                                ));
                            }
                        }
                    }
                }
                self.check_expr(expr);
            }
            HirStmtKind::Return(exprs) => {
                for expr in exprs {
                    self.check_expr(expr);
                }
            }
            HirStmtKind::While { condition, body } => {
                self.check_expr(condition);
                for stmt in body {
                    self.check_stmt(stmt);
                }
            }
            HirStmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.check_expr(condition);
                for stmt in then_block {
                    self.check_stmt(stmt);
                }
                if let Some(else_stmts) = else_block {
                    for stmt in else_stmts {
                        self.check_stmt(stmt);
                    }
                }
            }
            HirStmtKind::Assign { value, .. } => {
                self.check_expr(value);
            }
            HirStmtKind::Break | HirStmtKind::Continue | HirStmtKind::Drop { .. } => {}
        }
    }

    /// Check an expression for error flow issues.
    fn check_expr(&mut self, expr: &HirExpr) {
        match &expr.kind {
            // Try operator (`?`) - check that we're in a function that returns Result
            // Special case: `main` function can use `?` without declaring error type
            // (errors will panic at runtime)
            HirExprKind::Try(inner) => {
                if self.current_func_error_type.is_none() && self.current_func_name != "main" {
                    self.errors.push(ErrorFlowError::new(
                        ErrorFlowErrorKind::TryInNonResultFunction {
                            func_name: self.current_func_name.clone(),
                        },
                        expr.span,
                    ));
                }
                self.check_expr(inner);
            }

            // Err expression - check that we're in a function with error type
            HirExprKind::Err(inner) => {
                if self.current_func_error_type.is_none() {
                    self.errors.push(ErrorFlowError::new(
                        ErrorFlowErrorKind::ErrInNonResultFunction {
                            func_name: self.current_func_name.clone(),
                        },
                        expr.span,
                    ));
                }
                self.check_expr(inner);
            }

            // UnwrapOrPanic (`??`) - just recurse, but could check for message
            HirExprKind::UnwrapOrPanic {
                expr: inner,
                message,
            } => {
                self.check_expr(inner);
                self.check_expr(message);
            }

            // Ok expression - just recurse
            HirExprKind::Ok(inner) => {
                self.check_expr(inner);
            }

            // Call - check if result is being ignored
            HirExprKind::Call { args, .. } => {
                // Check if this call returns a Result and it's being used
                // as an expression statement (result ignored)
                if let Some(type_id) = expr.type_id {
                    if let Some(type_info) = self.registry.get(type_id) {
                        if let TypeKind::Result { ok, err } = &type_info.kind {
                            // Result type - this will be caught at Let statement level
                            // or if used as standalone expression
                            // For now, just note that call returns Result
                            let _ = (*ok, *err);
                        }
                    }
                }

                // Recurse into arguments
                for arg in args {
                    self.check_expr(arg);
                }
            }

            // Method call - same as regular call
            HirExprKind::MethodCall { receiver, args, .. } => {
                self.check_expr(receiver);
                for arg in args {
                    self.check_expr(arg);
                }
            }

            // Binary/Unary ops
            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.check_expr(lhs);
                self.check_expr(rhs);
            }
            HirExprKind::UnaryOp { operand, .. } => {
                self.check_expr(operand);
            }

            // Control flow
            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.check_expr(condition);
                self.check_expr(then_expr);
                if let Some(else_e) = else_expr {
                    self.check_expr(else_e);
                }
            }

            HirExprKind::Match { values, arms } => {
                for value in values {
                    self.check_expr(value);
                }
                for arm in arms {
                    self.check_expr(&arm.body);
                }
            }

            HirExprKind::Block {
                stmts,
                expr: tail_expr,
            } => {
                for stmt in stmts {
                    self.check_stmt(stmt);
                }
                if let Some(tail) = tail_expr {
                    self.check_expr(tail);
                }
            }

            // Compound expressions
            HirExprKind::Array(elements) => {
                for elem in elements {
                    self.check_expr(elem);
                }
            }
            HirExprKind::Tuple(elements) => {
                for elem in elements {
                    self.check_expr(elem);
                }
            }
            HirExprKind::Map(entries) => {
                for (key, value) in entries {
                    self.check_expr(key);
                    self.check_expr(value);
                }
            }
            HirExprKind::Struct { fields, .. } => {
                for (_, value) in fields {
                    self.check_expr(value);
                }
            }
            HirExprKind::Index { object, index } => {
                self.check_expr(object);
                self.check_expr(index);
            }
            HirExprKind::Field { object, .. } => {
                self.check_expr(object);
            }
            HirExprKind::Range { start, end, .. } => {
                self.check_expr(start);
                self.check_expr(end);
            }
            HirExprKind::Closure { body, .. } => {
                self.check_expr(body);
            }
            HirExprKind::Cast { value, .. } => {
                self.check_expr(value);
            }
            HirExprKind::Move(inner) | HirExprKind::Clone(inner) | HirExprKind::Spread(inner) => {
                self.check_expr(inner);
            }
            HirExprKind::Borrow { expr: inner, .. } => {
                self.check_expr(inner);
            }
            HirExprKind::EnumVariant { payload, .. } => {
                for p in payload {
                    self.check_expr(p);
                }
            }

            // Terminal expressions - no recursion needed
            HirExprKind::Const(_) | HirExprKind::Local { .. } | HirExprKind::Global { .. } => {}
        }
    }

    /// Count the number of ok values in a type (handles tuples).
    fn count_ok_values(&self, type_id: TypeId) -> usize {
        if let Some(type_info) = self.registry.get(type_id) {
            match &type_info.kind {
                TypeKind::Tuple { elements } => elements.len(),
                TypeKind::Void => 0,
                _ => 1,
            }
        } else {
            1 // Default to 1 if type not found
        }
    }

    /// Check if an expression is a Try expression (using `?`).
    fn is_try_expr(&self, expr: &HirExpr) -> bool {
        matches!(&expr.kind, HirExprKind::Try(_))
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Check if a type is a Result type.
pub fn is_result_type(registry: &TypeRegistry, type_id: TypeId) -> bool {
    if let Some(type_info) = registry.get(type_id) {
        matches!(type_info.kind, TypeKind::Result { .. })
    } else {
        false
    }
}

/// Extract the ok and error types from a Result type.
pub fn extract_result_types(registry: &TypeRegistry, type_id: TypeId) -> Option<(TypeId, TypeId)> {
    if let Some(type_info) = registry.get(type_id) {
        if let TypeKind::Result { ok, err } = type_info.kind {
            return Some((ok, err));
        }
    }
    None
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_flow_error_creation() {
        use doo_core::types::builtin;

        let error = ErrorFlowError::new(
            ErrorFlowErrorKind::UnhandledResult {
                ok_type: builtin::INT,
                err_type: builtin::STR,
            },
            Span::default(),
        );

        match error.kind {
            ErrorFlowErrorKind::UnhandledResult { ok_type, err_type } => {
                assert_eq!(ok_type, builtin::INT);
                assert_eq!(err_type, builtin::STR);
            }
            _ => panic!("Expected UnhandledResult error"),
        }
    }

    #[test]
    fn test_try_in_non_result_function_error() {
        let error = ErrorFlowError::new(
            ErrorFlowErrorKind::TryInNonResultFunction {
                func_name: "test_func".to_string(),
            },
            Span::default(),
        );

        match error.kind {
            ErrorFlowErrorKind::TryInNonResultFunction { func_name } => {
                assert_eq!(func_name, "test_func");
            }
            _ => panic!("Expected TryInNonResultFunction error"),
        }
    }

    #[test]
    fn test_err_in_non_result_function_error() {
        let error = ErrorFlowError::new(
            ErrorFlowErrorKind::ErrInNonResultFunction {
                func_name: "another_func".to_string(),
            },
            Span::default(),
        );

        match error.kind {
            ErrorFlowErrorKind::ErrInNonResultFunction { func_name } => {
                assert_eq!(func_name, "another_func");
            }
            _ => panic!("Expected ErrInNonResultFunction error"),
        }
    }
}
