//! Type Checker
//!
//! Validates types and infers missing types.

use super::scope::{ScopeManager, Symbol, SymbolKind};
use doo_core::{
    types::{builtin, TypeId},
    Span,
};
use doo_hir::{
    HirExpr, HirExprKind, HirFunction, HirItem, HirMatchPattern, HirProgram, HirStmt, HirStmtKind,
};

/// Type checking error.
#[derive(Debug, Clone)]
pub struct TypeError {
    pub kind: TypeErrorKind,
    pub span: Span,
}

/// Kinds of type errors.
#[derive(Debug, Clone)]
pub enum TypeErrorKind {
    /// Type mismatch (expected X, found Y).
    Mismatch { expected: TypeId, found: TypeId },
    /// Undefined variable.
    Undefined(String),
    /// Invalid operation for type.
    InvalidOp(String),
    /// Function argument mismatch.
    ArgMismatch { expected: usize, found: usize },
    /// Invalid condition type (must be Bool).
    InvalidCondition { found: TypeId },
    /// Invalid cast (from X to Y).
    InvalidCast { from: TypeId, to: TypeId },
    /// Return type mismatch (function expects X, found Y).
    ReturnTypeMismatch {
        function: String,
        expected: TypeId,
        found: TypeId,
    },
}

/// The type checker.
pub struct TypeChecker {
    /// Scope manager for symbol tracking.
    scopes: ScopeManager,
    /// Collected errors.
    errors: Vec<TypeError>,
    /// Current function return type (for validating return statements).
    current_return_type: Option<TypeId>,
    /// Current function name (for error messages).
    current_function: String,
}

impl TypeChecker {
    /// Create a new type checker.
    pub fn new() -> Self {
        Self {
            scopes: ScopeManager::new(),
            errors: Vec::new(),
            current_return_type: None,
            current_function: String::new(),
        }
    }

    /// Check an entire program.
    pub fn check(&mut self, program: &HirProgram) -> Result<(), Vec<TypeError>> {
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

    /// Check a function.
    fn check_function(&mut self, func: &HirFunction) {
        // Save previous function context
        let prev_return_type = self.current_return_type;
        let prev_function = self.current_function.clone();

        // Set current function context
        self.current_return_type = func.return_type;
        self.current_function = func.name.clone();

        // Register function parameters in scope
        self.scopes.enter_scope(super::scope::ScopeKind::Function);

        for param in &func.params {
            let _ = self.scopes.define(Symbol {
                name: param.name.clone(),
                kind: SymbolKind::Parameter,
                type_id: param.type_id.or(Some(builtin::ANY)), // Default to Any if unknown
                mutable: false,
                span: param.span,
                used: false,
            });
        }

        // Check body statements
        for stmt in &func.body {
            self.check_stmt(stmt);
        }

        self.scopes.exit_scope();

        // Restore previous function context
        self.current_return_type = prev_return_type;
        self.current_function = prev_function;
    }

    /// Check a statement for type correctness.
    fn check_stmt(&mut self, stmt: &HirStmt) {
        match &stmt.kind {
            HirStmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                // Condition must be Bool
                self.check_condition(condition);

                // Check then block in its own scope
                self.scopes.enter_scope(super::scope::ScopeKind::Block);
                for s in then_block {
                    self.check_stmt(s);
                }
                self.scopes.exit_scope();

                // Check else block if present
                if let Some(else_stmts) = else_block {
                    self.scopes.enter_scope(super::scope::ScopeKind::Block);
                    for s in else_stmts {
                        self.check_stmt(s);
                    }
                    self.scopes.exit_scope();
                }
            }

            HirStmtKind::While { condition, body } => {
                // Condition must be Bool
                self.check_condition(condition);

                // Enter loop scope for the body
                self.scopes.enter_scope(super::scope::ScopeKind::Loop);
                for s in body {
                    self.check_stmt(s);
                }
                self.scopes.exit_scope();
            }

            HirStmtKind::Expr(expr) => {
                self.check_expr(expr);
            }

            HirStmtKind::Let {
                name,
                type_id,
                value,
                mutable,
                ..
            } => {
                // First check the value expression
                let value_type = self.check_expr(value);

                // Determine the variable's type: explicit type_id, or inferred from value
                let var_type = type_id.or(Some(value_type));

                // Register the variable in the current scope
                let _ = self.scopes.define(Symbol {
                    name: name.clone(),
                    kind: SymbolKind::Variable,
                    type_id: var_type,
                    mutable: *mutable,
                    span: stmt.span,
                    used: false,
                });
            }

            HirStmtKind::Assign { target, value } => {
                self.check_expr(target);
                self.check_expr(value);
            }

            HirStmtKind::Return(exprs) => {
                self.check_return(exprs, stmt.span);
            }

            _ => {}
        }
    }

    /// Check that a condition expression is Bool.
    fn check_condition(&mut self, condition: &HirExpr) {
        let cond_type = self.check_expr(condition);

        // Condition must be Bool (or Any for dynamic typing)
        if cond_type != builtin::BOOL && cond_type != builtin::ANY {
            self.errors.push(TypeError {
                kind: TypeErrorKind::InvalidCondition { found: cond_type },
                span: condition.span,
            });
        }
    }

    /// Check an expression and return its type.
    fn check_expr(&mut self, expr: &HirExpr) -> TypeId {
        // If the expression already has a type_id, use it
        if let Some(type_id) = expr.type_id {
            // Still need to recurse into sub-expressions for validation
            self.validate_expr_children(expr);
            return type_id;
        }

        match &expr.kind {
            HirExprKind::Const(c) => c.type_id(),

            HirExprKind::Local { name } => {
                if let Some(sym) = self.scopes.lookup(name) {
                    sym.type_id.unwrap_or(builtin::ANY)
                } else {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::Undefined(name.clone()),
                        span: expr.span,
                    });
                    builtin::ANY
                }
            }

            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                // Condition must be Bool
                self.check_condition(condition);

                // Check then/else branches
                let then_type = self.check_expr(then_expr);
                if let Some(else_e) = else_expr {
                    self.check_expr(else_e);
                }
                then_type
            }

            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.check_expr(lhs);
                self.check_expr(rhs);
                expr.type_id.unwrap_or(builtin::ANY)
            }

            HirExprKind::UnaryOp { operand, .. } => {
                self.check_expr(operand);
                expr.type_id.unwrap_or(builtin::ANY)
            }

            HirExprKind::Call { func, args } => {
                self.check_expr(func);
                for arg in args {
                    self.check_expr(arg);
                }
                expr.type_id.unwrap_or(builtin::ANY)
            }

            HirExprKind::MethodCall { receiver, args, .. } => {
                self.check_expr(receiver);
                for arg in args {
                    self.check_expr(arg);
                }
                expr.type_id.unwrap_or(builtin::ANY)
            }

            HirExprKind::Block {
                stmts,
                expr: final_expr,
            } => {
                // Enter block scope so let bindings are properly scoped
                self.scopes.enter_scope(super::scope::ScopeKind::Block);
                for s in stmts {
                    self.check_stmt(s);
                }
                let result = if let Some(e) = final_expr {
                    self.check_expr(e)
                } else {
                    builtin::VOID
                };
                self.scopes.exit_scope();
                result
            }

            HirExprKind::Cast { value, to_type } => {
                let from_type = self.check_expr(value);
                self.validate_cast(from_type, *to_type, expr.span);
                *to_type
            }

            HirExprKind::Match { values, arms } => {
                // Check all match values
                for value in values {
                    self.check_expr(value);
                }

                // Check all arms
                for arm in arms {
                    // Check pattern (guards must be Bool)
                    self.check_match_pattern(&arm.pattern);

                    // Check guard if present
                    if let Some(guard) = &arm.guard {
                        self.check_condition(guard);
                    }

                    // Check body
                    self.check_expr(&arm.body);
                }

                expr.type_id.unwrap_or(builtin::ANY)
            }

            _ => expr.type_id.unwrap_or(builtin::ANY),
        }
    }

    /// Validate children of an expression without changing its type.
    fn validate_expr_children(&mut self, expr: &HirExpr) {
        match &expr.kind {
            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.check_condition(condition);
                self.check_expr(then_expr);
                if let Some(else_e) = else_expr {
                    self.check_expr(else_e);
                }
            }

            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.check_expr(lhs);
                self.check_expr(rhs);
            }

            HirExprKind::UnaryOp { operand, .. } => {
                self.check_expr(operand);
            }

            HirExprKind::Call { func, args } => {
                self.check_expr(func);
                for arg in args {
                    self.check_expr(arg);
                }
            }

            HirExprKind::MethodCall { receiver, args, .. } => {
                self.check_expr(receiver);
                for arg in args {
                    self.check_expr(arg);
                }
            }

            HirExprKind::Block {
                stmts,
                expr: final_expr,
            } => {
                for s in stmts {
                    self.check_stmt(s);
                }
                if let Some(e) = final_expr {
                    self.check_expr(e);
                }
            }

            HirExprKind::Cast { value, to_type } => {
                let from_type = self.check_expr(value);
                self.validate_cast(from_type, *to_type, expr.span);
            }

            HirExprKind::Match { values, arms } => {
                // Check all match values
                for value in values {
                    self.check_expr(value);
                }

                // Check all arms
                for arm in arms {
                    // Check pattern (guards must be Bool)
                    self.check_match_pattern(&arm.pattern);

                    // Check guard if present
                    if let Some(guard) = &arm.guard {
                        self.check_condition(guard);
                    }

                    // Check body
                    self.check_expr(&arm.body);
                }
            }

            _ => {}
        }
    }

    /// Check a match pattern.
    fn check_match_pattern(&mut self, pattern: &HirMatchPattern) {
        match pattern {
            HirMatchPattern::Literal(expr) => {
                self.check_expr(expr);
            }
            HirMatchPattern::Condition(expr) => {
                // Conditions must be Bool type
                self.check_condition(expr);
            }
            HirMatchPattern::Tuple(patterns) => {
                for p in patterns {
                    self.check_match_pattern(p);
                }
            }
            HirMatchPattern::Wildcard
            | HirMatchPattern::EnumVariant { .. }
            | HirMatchPattern::EnumVariantPayload { .. } => {
                // No type checking needed for these patterns
            }
        }
    }

    /// Validate that a cast from one type to another is legal.
    /// Rules from legacy analyzer:
    /// - Int -> Int, Float, Str: allowed
    /// - Int -> Bool: rejected
    /// - Float -> Int, Float, Str: allowed
    /// - Float -> Bool: rejected
    /// - Bool -> Int, Str, Bool: allowed
    /// - Bool -> Float: rejected
    /// - Str -> Int, Float, Str: allowed
    /// - Str -> Bool: rejected
    fn validate_cast(&mut self, from: TypeId, to: TypeId, span: Span) {
        // Same type casts are always valid
        if from == to {
            return;
        }

        // ANY type can cast to anything
        if from == builtin::ANY || to == builtin::ANY {
            return;
        }

        let valid = match from {
            t if t == builtin::INT => {
                // Int -> Int, Float, Str allowed; Int -> Bool rejected
                to == builtin::INT || to == builtin::FLOAT || to == builtin::STR
            }
            t if t == builtin::FLOAT => {
                // Float -> Int, Float, Str allowed; Float -> Bool rejected
                to == builtin::INT || to == builtin::FLOAT || to == builtin::STR
            }
            t if t == builtin::BOOL => {
                // Bool -> Int, Str, Bool allowed; Bool -> Float rejected
                to == builtin::INT || to == builtin::STR || to == builtin::BOOL
            }
            t if t == builtin::STR => {
                // Str -> Int, Float, Str allowed; Str -> Bool rejected
                to == builtin::INT || to == builtin::FLOAT || to == builtin::STR
            }
            _ => false, // Unknown types: reject
        };

        if !valid {
            self.errors.push(TypeError {
                kind: TypeErrorKind::InvalidCast { from, to },
                span,
            });
        }
    }

    /// Check return statement types match function signature.
    fn check_return(&mut self, exprs: &[HirExpr], span: Span) {
        // Check all expression types
        let mut return_types: Vec<TypeId> = Vec::new();
        for expr in exprs {
            let expr_type = self.check_expr(expr);
            return_types.push(expr_type);
        }

        // If function has no declared return type, nothing to validate
        let Some(expected_type) = self.current_return_type else {
            return;
        };

        // Determine the actual return type
        let actual_type = if return_types.is_empty() {
            builtin::VOID
        } else if return_types.len() == 1 {
            return_types[0]
        } else {
            // Multiple return values would be a tuple, but we don't have tuple registry here
            // For now, just check each individual return against expected
            // This is a simplified check - full implementation would build tuple type
            return_types[0]
        };

        // Skip validation if types use ANY (dynamic typing)
        if actual_type == builtin::ANY || expected_type == builtin::ANY {
            return;
        }

        // Check type compatibility
        if actual_type != expected_type {
            self.errors.push(TypeError {
                kind: TypeErrorKind::ReturnTypeMismatch {
                    function: self.current_function.clone(),
                    expected: expected_type,
                    found: actual_type,
                },
                span,
            });
        }
    }
}
