//! Type Checker
//!
//! Validates types and infers missing types.

use std::sync::Arc;

use super::scope::{ScopeManager, Symbol, SymbolKind};
use doo_core::{
    constants::ffi_names,
    types::{builtin, TypeId, TypeKind, TypeRegistry},
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
    /// Type registry for type operations (tuple construction, compatibility checking).
    registry: Arc<TypeRegistry>,
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
    /// Create a new type checker with access to the type registry.
    pub fn new(registry: Arc<TypeRegistry>) -> Self {
        Self {
            registry,
            scopes: ScopeManager::new(),
            errors: Vec::new(),
            current_return_type: None,
            current_function: String::new(),
        }
    }

    /// Check an entire program.
    pub fn check(&mut self, program: &HirProgram) -> Result<(), Vec<TypeError>> {
        // First pass: Register all functions in global scope
        // This allows function calls to resolve even before the function is defined
        self.scopes.enter_scope(super::scope::ScopeKind::Global);
        for item in &program.items {
            if let HirItem::Function(func) = item {
                // For now, store the return type directly as the function's type_id
                // This is simplified - a full implementation would build a function type
                let return_type = func.return_type.unwrap_or(builtin::VOID);

                // Register function in global scope
                let _ = self.scopes.define(Symbol {
                    name: func.name.clone(),
                    kind: SymbolKind::Function,
                    type_id: Some(return_type),
                    mutable: false,
                    span: func.span,
                    used: false,
                });
            }
        }

        // Second pass: Type check function bodies
        for item in &program.items {
            if let HirItem::Function(func) = item {
                self.check_function(func);
            }
        }

        self.scopes.exit_scope();

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

            HirStmtKind::TupleLet {
                names,
                type_ids,
                value,
                mutable,
            } => {
                // Check the value expression (should be a tuple or function returning tuple)
                let value_type = self.check_expr(value);

                // Try to get element types from the tuple type
                let element_types: Vec<TypeId> = if let Some(info) = self.registry.get(value_type) {
                    if let TypeKind::Tuple { elements } = &info.kind {
                        elements.clone()
                    } else {
                        vec![builtin::ANY; names.len()]
                    }
                } else {
                    vec![builtin::ANY; names.len()]
                };

                // Register each variable in the current scope
                for (i, name) in names.iter().enumerate() {
                    let var_type = type_ids
                        .get(i)
                        .and_then(|t| *t)
                        .or_else(|| element_types.get(i).copied())
                        .unwrap_or(builtin::ANY);

                    let _ = self.scopes.define(Symbol {
                        name: name.clone(),
                        kind: SymbolKind::Variable,
                        type_id: Some(var_type),
                        mutable: *mutable,
                        span: stmt.span,
                        used: false,
                    });
                }
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
                // Built-in modules (JSON, Math, File, etc.) don't need to be in scope
                if ffi_names::is_builtin_module(name) {
                    return builtin::ANY; // Module type - resolved at codegen
                }

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
                // Check all argument expressions
                for arg in args {
                    self.check_expr(arg);
                }

                // Try to get the function return type
                // First, see if the func is a local reference (e.g., a function name)
                let func_return_type = if let HirExprKind::Local { name } = &func.kind {
                    // Look up the function in scope
                    if let Some(sym) = self.scopes.lookup(name) {
                        if sym.kind == SymbolKind::Function {
                            sym.type_id
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    // For more complex call targets, just check the expression
                    self.check_expr(func);
                    None
                };

                // Return the function's return type, or fall back to expr.type_id, or ANY
                func_return_type.or(expr.type_id).unwrap_or(builtin::ANY)
            }

            HirExprKind::MethodCall { receiver, args, .. } => {
                // Built-in modules are checked via is_builtin_module in Local case
                // This call to check_expr will properly skip the undefined error for them
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
            // Multiple return values form a tuple type.
            // Look up or match the expected tuple type to verify compatibility.
            self.match_tuple_return_type(&return_types, expected_type, span)
        };

        // Skip validation if types use ANY (dynamic typing)
        if actual_type == builtin::ANY || expected_type == builtin::ANY {
            return;
        }

        // Check type compatibility using registry
        if !self.registry.is_compatible(actual_type, expected_type) {
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

    /// Match a multi-value return against an expected tuple type.
    /// Returns the expected type if the elements match, or reports errors.
    fn match_tuple_return_type(
        &mut self,
        return_types: &[TypeId],
        expected_type: TypeId,
        span: Span,
    ) -> TypeId {
        // Get the expected type info from registry
        let Some(expected_info) = self.registry.get(expected_type) else {
            // Expected type not in registry, can't validate - return ANY to skip
            return builtin::ANY;
        };

        // Expected type must be a Tuple for multi-value returns
        let TypeKind::Tuple {
            elements: expected_elements,
        } = &expected_info.kind
        else {
            // Expected type is not a tuple but we have multiple return values
            // This is a type mismatch - report with first element type
            return return_types[0];
        };

        // Check element count matches
        if return_types.len() != expected_elements.len() {
            // Different number of elements - report mismatch
            // Return first type to trigger error reporting
            return return_types[0];
        }

        // Check each element for compatibility
        for (i, (actual, expected)) in return_types
            .iter()
            .zip(expected_elements.iter())
            .enumerate()
        {
            // Skip ANY types (dynamic)
            if *actual == builtin::ANY || *expected == builtin::ANY {
                continue;
            }

            if !self.registry.is_compatible(*actual, *expected) {
                // Element type mismatch - report specific error
                self.errors.push(TypeError {
                    kind: TypeErrorKind::Mismatch {
                        expected: *expected,
                        found: *actual,
                    },
                    span,
                });
            }
        }

        // All elements matched - return the expected tuple type
        expected_type
    }
}
