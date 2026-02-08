//! Type Checker
//!
//! Validates types and infers missing types.

use std::collections::HashSet;
use std::sync::Arc;

use super::scope::{ScopeError, ScopeManager, Symbol, SymbolKind};
use doo_core::{
    constants::ffi_names,
    errors::codes::{CompilerError, ErrorCode, ErrorSeverity},
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
    /// Undefined function call.
    UndefinedFunction(String),
    /// Undefined type name.
    UndefinedType(String),
    /// Undefined field access.
    UndefinedField { type_name: String, field: String },
    /// Undefined method call.
    UndefinedMethod { type_name: String, method: String },
    /// Undefined enum variant.
    UndefinedVariant { enum_name: String, variant: String },
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
    /// Unknown type reference.
    UnknownType(String),
    /// Type cannot be inferred.
    CannotInfer(String),
    /// Incompatible types used together.
    Incompatible {
        left: TypeId,
        right: TypeId,
        operation: String,
    },
    /// Cannot convert between types.
    CannotConvert { from: TypeId, to: TypeId },
    /// Tuple length mismatch.
    TupleLengthMismatch { expected: usize, found: usize },
    /// Type parameter count wrong.
    TypeParamCount { expected: usize, found: usize },
    /// Array element has wrong type.
    InvalidArrayElement {
        expected: TypeId,
        found: TypeId,
        index: usize,
    },
    /// Map key type invalid.
    InvalidMapKey { found: TypeId },
    /// If/else branches have different types.
    IfElseMismatch {
        then_type: TypeId,
        else_type: TypeId,
    },
    /// Nil used with non-optional type.
    NilNonOptional { expected: TypeId },
    /// Missing struct field in construction.
    MissingStructField { struct_name: String, field: String },
    /// Unknown struct field in construction.
    UnknownStructField { struct_name: String, field: String },
    /// Invalid function signature.
    InvalidSignature(String),
}

/// The type checker.
pub struct TypeChecker {
    /// Type registry for type operations (tuple construction, compatibility checking).
    registry: Arc<TypeRegistry>,
    /// Scope manager for symbol tracking.
    scopes: ScopeManager,
    /// Collected errors.
    errors: Vec<TypeError>,
    /// Collected scope errors (redeclarations etc.).
    scope_errors: Vec<ScopeError>,
    /// Direct compiler errors (MissingReturn, UnreachableCode, etc.).
    direct_errors: Vec<doo_core::errors::codes::CompilerError>,
    /// Current function return type (for validating return statements).
    current_return_type: Option<TypeId>,
    /// Current function name (for error messages).
    current_function: String,
    /// Routes seen for DuplicateRoute detection: (method, path).
    routes_seen: HashSet<(String, String)>,
}

impl TypeChecker {
    /// Create a new type checker with access to the type registry.
    pub fn new(registry: Arc<TypeRegistry>) -> Self {
        Self {
            registry,
            scopes: ScopeManager::new(),
            errors: Vec::new(),
            scope_errors: Vec::new(),
            direct_errors: Vec::new(),
            current_return_type: None,
            current_function: String::new(),
            routes_seen: HashSet::new(),
        }
    }

    /// Define a symbol and collect any scope errors (e.g., redeclaration).
    /// Skips `_` (discard/wildcard variable) — never register it in scope.
    fn define_symbol(&mut self, symbol: Symbol) {
        if symbol.name == "_" {
            return; // `_` is a discard variable, never define it
        }
        if let Err(e) = self.scopes.define(symbol) {
            self.scope_errors.push(e);
        }
    }

    /// Get collected scope errors.
    pub fn scope_errors(&self) -> &[ScopeError] {
        &self.scope_errors
    }

    /// Take the collected scope errors.
    pub fn take_scope_errors(&mut self) -> Vec<ScopeError> {
        std::mem::take(&mut self.scope_errors)
    }

    /// Take direct compiler errors (MissingReturn, UnreachableCode, etc.).
    pub fn take_direct_errors(&mut self) -> Vec<doo_core::errors::codes::CompilerError> {
        std::mem::take(&mut self.direct_errors)
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
                // Skip if already defined (can happen with imported functions merged into program)
                if self.scopes.lookup(&func.name).is_none() {
                    self.define_symbol(Symbol {
                        name: func.name.clone(),
                        kind: SymbolKind::Function,
                        type_id: Some(return_type),
                        mutable: false,
                        span: func.span,
                        used: false,
                    });
                }
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
            self.define_symbol(Symbol {
                name: param.name.clone(),
                kind: SymbolKind::Parameter,
                type_id: param.type_id.or(Some(builtin::ANY)),
                mutable: false,
                span: param.span,
                used: false,
            });
        }

        // Check body statements
        let mut found_return = false;
        let mut return_span = None;
        for (i, stmt) in func.body.iter().enumerate() {
            // UnreachableCode: after a return, subsequent statements are unreachable
            if found_return {
                self.direct_errors.push(
                    doo_core::errors::codes::CompilerError::new(
                        ErrorCode::UnreachableCode,
                        "unreachable code after return statement",
                        stmt.span,
                    )
                    .with_severity(ErrorSeverity::Warning)
                    .with_suggestion("remove this code or move it before the return"),
                );
                break; // Only report the first unreachable statement
            }
            self.check_stmt(stmt);

            // Track if this statement is a return (includes Ok/Err which act as returns)
            if Self::stmt_is_return(stmt) {
                found_return = true;
                return_span = Some(stmt.span);
            }
        }

        // MissingReturn: function has a return type but body doesn't end with return
        if let Some(ret_type) = func.return_type {
            if ret_type != builtin::VOID && !found_return && func.name != "main" {
                // Check if the last statement is a return (basic check)
                let last_returns = func.body.last().map_or(false, |s| Self::stmt_is_return(s));
                if !last_returns && !func.body.is_empty() {
                    self.direct_errors.push(
                        doo_core::errors::codes::CompilerError::new(
                            ErrorCode::MissingReturn,
                            format!(
                                "function '{}' may not return a value on all paths",
                                func.name
                            ),
                            func.span,
                        )
                        .with_suggestion("add a `return` statement"),
                    );
                }
            }
        }

        self.scopes.exit_scope();

        // Restore previous function context
        self.current_return_type = prev_return_type;
        self.current_function = prev_function;
    }

    /// Check if a statement effectively returns a value.
    /// Returns true for `return`, `Ok(...)`, `Err(...)`, `if/else` where all branches return,
    /// and expression statements that implicitly return (match, block, etc.).
    fn stmt_is_return(stmt: &HirStmt) -> bool {
        match &stmt.kind {
            HirStmtKind::Return(_) => true,
            HirStmtKind::Expr(expr) => Self::expr_is_return(expr),
            HirStmtKind::If {
                then_block,
                else_block,
                ..
            } => {
                // If both branches end with a return/Ok/Err, the whole if is a return
                let then_returns = then_block.last().map_or(false, |s| Self::stmt_is_return(s));
                let else_returns = else_block.as_ref().map_or(false, |eb| {
                    eb.last().map_or(false, |s| Self::stmt_is_return(s))
                });
                then_returns && else_returns
            }
            _ => false,
        }
    }

    /// Resolve a TypeId to a human-readable type name.
    fn type_name(&self, id: TypeId) -> String {
        self.registry
            .get(id)
            .map(|t| t.kind.to_string())
            .unwrap_or_else(|| format!("{}", id))
    }

    /// Check if an expression implicitly returns a value (can serve as the last expression).
    fn expr_is_return(expr: &HirExpr) -> bool {
        match &expr.kind {
            HirExprKind::Ok(_) | HirExprKind::Err(_) => true,
            // Match expression as last statement = implicit return (each arm produces a value)
            HirExprKind::Match { .. } => true,
            // Block expression — check if its last expression/stmt returns
            HirExprKind::Block { stmts, expr } => {
                if let Some(tail_expr) = expr {
                    Self::expr_is_return(tail_expr)
                } else {
                    stmts.last().map_or(false, |s| Self::stmt_is_return(s))
                }
            }
            // If expression with else → both branches produce a value
            HirExprKind::If {
                then_expr,
                else_expr,
                ..
            } => {
                let then_returns = Self::expr_is_return(then_expr);
                let else_returns = else_expr
                    .as_ref()
                    .map_or(false, |e| Self::expr_is_return(e));
                then_returns && else_returns
            }
            // Regular expressions are NOT implicit returns — they are just expression statements
            _ => false,
        }
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

                // Type mismatch check: annotated type vs. actual value type
                if let Some(expected) = type_id {
                    if value_type != *expected
                        && value_type != builtin::ANY
                        && *expected != builtin::ANY
                        && value_type != builtin::VOID
                    {
                        self.direct_errors.push(CompilerError::new(
                            ErrorCode::TypeMismatch,
                            format!(
                                "expected {}, found {}",
                                self.type_name(*expected),
                                self.type_name(value_type)
                            ),
                            value.span,
                        ));
                    }
                }

                // Determine the variable's type: explicit type_id, or inferred from value
                let var_type = type_id.or(Some(value_type));

                // Register the variable in the current scope
                self.define_symbol(Symbol {
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

                    self.define_symbol(Symbol {
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
                let target_type = self.check_expr(target);
                let value_type = self.check_expr(value);

                // Type mismatch check: target type vs value type
                if target_type != value_type
                    && target_type != builtin::ANY
                    && value_type != builtin::ANY
                    && target_type != builtin::VOID
                    && value_type != builtin::VOID
                {
                    self.direct_errors.push(CompilerError::new(
                        ErrorCode::TypeMismatch,
                        format!(
                            "expected {}, found {}",
                            self.type_name(target_type),
                            self.type_name(value_type)
                        ),
                        value.span,
                    ));
                }
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
            self.direct_errors.push(CompilerError::new(
                ErrorCode::InvalidConditionType,
                format!("expected Bool, found {}", self.type_name(cond_type)),
                condition.span,
            ));
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
                } else if let Some(type_id) = self.registry.lookup(name) {
                    // Check if it's a registered type (struct or enum) used as a type reference
                    // This handles cases like `app.auth(..., User, db)` where User is a struct name
                    type_id
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

            HirExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                // Built-in modules are checked via is_builtin_module in Local case
                // This call to check_expr will properly skip the undefined error for them
                self.check_expr(receiver);
                for arg in args {
                    self.check_expr(arg);
                }

                // Compile-time route validation: detect duplicate routes
                let is_http_method = matches!(
                    method.as_str(),
                    "get" | "post" | "put" | "delete" | "patch" | "options" | "head"
                );
                if is_http_method && !args.is_empty() {
                    // First arg is typically the route path (a string literal)
                    if let HirExprKind::Const(doo_hir::ConstValue::Str(ref path)) = args[0].kind {
                        let route_key = (method.to_uppercase(), path.to_string());
                        if !self.routes_seen.insert(route_key) {
                            self.direct_errors.push(
                                doo_core::errors::codes::CompilerError::new(
                                    ErrorCode::DuplicateRoute,
                                    format!("duplicate route: {} {}", method.to_uppercase(), path),
                                    expr.span,
                                )
                                .with_suggestion("each route path+method must be unique"),
                            );
                        }
                    }
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
                    // Enter arm scope for pattern bindings
                    self.scopes.enter_scope(super::scope::ScopeKind::Block);

                    // Register pattern bindings in scope
                    self.register_pattern_bindings(&arm.pattern, arm.span);

                    // Check pattern (guards must be Bool)
                    self.check_match_pattern(&arm.pattern);

                    // Check guard if present
                    if let Some(guard) = &arm.guard {
                        self.check_condition(guard);
                    }

                    // Check body
                    self.check_expr(&arm.body);

                    // Exit arm scope
                    self.scopes.exit_scope();
                }

                expr.type_id.unwrap_or(builtin::ANY)
            }

            // Error handling expressions - recurse into inner value
            HirExprKind::Ok(inner) | HirExprKind::Err(inner) | HirExprKind::Try(inner) => {
                self.check_expr(inner);
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
                    // Enter arm scope for pattern bindings
                    self.scopes.enter_scope(super::scope::ScopeKind::Block);

                    // Register pattern bindings in scope
                    self.register_pattern_bindings(&arm.pattern, arm.span);

                    // Check pattern (guards must be Bool)
                    self.check_match_pattern(&arm.pattern);

                    // Check guard if present
                    if let Some(guard) = &arm.guard {
                        self.check_condition(guard);
                    }

                    // Check body
                    self.check_expr(&arm.body);

                    // Exit arm scope
                    self.scopes.exit_scope();
                }
            }

            // Error handling expressions - recurse into inner value
            HirExprKind::Ok(inner) | HirExprKind::Err(inner) | HirExprKind::Try(inner) => {
                self.check_expr(inner);
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

    /// Register pattern bindings in the current scope.
    /// This allows bound variables from match patterns to be used in arm bodies.
    fn register_pattern_bindings(&mut self, pattern: &HirMatchPattern, span: Span) {
        match pattern {
            HirMatchPattern::EnumVariantPayload {
                enum_name,
                variant,
                bindings,
            } => {
                // Look up the enum type to find the variant's payload type
                if let Some(enum_type_id) = self.registry.lookup(enum_name) {
                    // Extract payload type from registry before calling define_symbol
                    // to avoid borrow conflict (immutable borrow on registry vs mutable on self)
                    let payload_type = if let Some(type_info) = self.registry.get(enum_type_id) {
                        if let TypeKind::Enum { variants, .. } = &type_info.kind {
                            variants
                                .iter()
                                .find(|(v, _)| v == variant)
                                .and_then(|(_, payload)| *payload)
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some(payload_type_id) = payload_type {
                        for binding in bindings {
                            self.define_symbol(Symbol {
                                name: binding.clone(),
                                kind: SymbolKind::Variable,
                                type_id: Some(payload_type_id),
                                mutable: false,
                                span,
                                used: false,
                            });
                        }
                    }
                }
            }
            HirMatchPattern::Tuple(patterns) => {
                for p in patterns {
                    self.register_pattern_bindings(p, span);
                }
            }
            // These patterns don't introduce bindings
            HirMatchPattern::Literal(_)
            | HirMatchPattern::Condition(_)
            | HirMatchPattern::Wildcard
            | HirMatchPattern::EnumVariant { .. } => {}
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
            _ => {
                // Any type can be cast to Str (for string interpolation / print)
                to == builtin::STR
            }
        };

        if !valid {
            self.direct_errors.push(CompilerError::new(
                ErrorCode::InvalidCast,
                format!(
                    "cannot cast {} to {}",
                    self.type_name(from),
                    self.type_name(to)
                ),
                span,
            ));
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
            self.direct_errors.push(CompilerError::new(
                ErrorCode::ReturnTypeMismatch,
                format!(
                    "expected {}, found {} in '{}'",
                    self.type_name(expected_type),
                    self.type_name(actual_type),
                    self.current_function
                ),
                span,
            ));
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
                self.direct_errors.push(CompilerError::new(
                    ErrorCode::TypeMismatch,
                    format!(
                        "expected {}, found {} at tuple index {}",
                        self.type_name(*expected),
                        self.type_name(*actual),
                        i
                    ),
                    span,
                ));
            }
        }

        // All elements matched - return the expected tuple type
        expected_type
    }
}
