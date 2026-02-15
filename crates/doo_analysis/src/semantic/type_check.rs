//! Type Checker
//!
//! Validates types and infers missing types.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::scope::{ScopeError, ScopeManager, Symbol, SymbolKind};
use doo_core::{
    constants::ffi_names,
    errors::codes::{CompilerError, ErrorCode, ErrorSeverity},
    types::{builtin, TypeId, TypeKind, TypeRegistry},
    Span,
};
use doo_hir::{
    HirBinOp, HirExpr, HirExprKind, HirFunction, HirItem, HirMatchPattern, HirProgram, HirStmt,
    HirStmtKind,
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
    /// Function signatures: name -> param types.
    functions: HashMap<String, Vec<Option<TypeId>>>,
    /// Struct fields that are optional or have defaults (not required in constructors).
    /// Maps struct name -> set of optional/defaulted field names.
    struct_optional_fields: HashMap<String, HashSet<String>>,
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
            functions: HashMap::new(),
            struct_optional_fields: HashMap::new(),
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
        // First pass: Register all functions, structs, enums in global scope
        // This allows forward references and detects duplicates
        self.scopes.enter_scope(super::scope::ScopeKind::Global);
        for item in &program.items {
            match item {
                HirItem::Function(func) => {
                    let return_type = func.return_type.unwrap_or(builtin::VOID);

                    // Check for duplicate function (error instead of silently skipping)
                    if self.scopes.lookup(&func.name).is_some() {
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::InvalidOp(format!(
                                "function '{}' is already defined",
                                func.name
                            )),
                            span: func.span,
                        });
                    } else {
                        self.define_symbol(Symbol {
                            name: func.name.clone(),
                            kind: SymbolKind::Function,
                            type_id: Some(return_type),
                            mutable: false,
                            span: func.span,
                            used: false,
                        });
                    }
                    // Store function parameter types for call-site validation
                    self.functions.insert(
                        func.name.clone(),
                        func.params.iter().map(|p| p.type_id).collect(),
                    );
                }
                HirItem::Struct(s) => {
                    // Detect duplicate struct definitions using scope tracking
                    let struct_key = format!("__struct_{}", s.name);
                    if self.scopes.lookup(&struct_key).is_some() {
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::InvalidOp(format!(
                                "struct '{}' is already defined",
                                s.name
                            )),
                            span: s.span,
                        });
                    } else {
                        self.define_symbol(Symbol {
                            name: struct_key,
                            kind: SymbolKind::Variable,
                            type_id: None,
                            mutable: false,
                            span: s.span,
                            used: false,
                        });
                    }
                    // Collect optional/defaulted fields for missing-field checks
                    let optional_fields: HashSet<String> = s
                        .fields
                        .iter()
                        .filter(|f| f.is_optional || f.default.is_some())
                        .map(|f| f.name.clone())
                        .collect();
                    if !optional_fields.is_empty() {
                        self.struct_optional_fields
                            .insert(s.name.clone(), optional_fields);
                    }
                }
                HirItem::Enum(e) => {
                    // Detect duplicate enum definitions
                    // Track enums we've already seen via a simple check
                    let enum_key = format!("__enum_{}", e.name);
                    if self.scopes.lookup(&enum_key).is_some() {
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::InvalidOp(format!(
                                "enum '{}' is already defined",
                                e.name
                            )),
                            span: e.span,
                        });
                    } else {
                        self.define_symbol(Symbol {
                            name: enum_key,
                            kind: SymbolKind::Variable,
                            type_id: None,
                            mutable: false,
                            span: e.span,
                            used: false,
                        });
                    }
                }
                _ => {}
            }
        }

        // Validate type annotations — catch unknown types before deep analysis
        self.validate_type_annotations(program);

        // Early exit if unknown types found (avoids stack-heavy phases)
        if !self.errors.is_empty() {
            self.scopes.exit_scope();
            return Err(self.errors.clone());
        }

        // Second pass: Type check function bodies
        for item in &program.items {
            if let HirItem::Function(func) = item {
                self.check_function(func);
            }
        }

        self.scopes.exit_scope();

        // Merge direct_errors (CompilerError) into TypeError list so callers
        // see *all* type errors through a single channel.
        for ce in &self.direct_errors {
            self.errors.push(TypeError {
                kind: TypeErrorKind::InvalidOp(ce.message.clone()),
                span: ce.span,
            });
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.clone())
        }
    }

    /// Validate all type annotations in the program to catch undefined types early.
    /// Scans Let statements and function params/returns for TypeRef (unresolved named types).
    #[inline(never)]
    fn validate_type_annotations(&mut self, program: &HirProgram) {
        for item in &program.items {
            if let HirItem::Function(func) = item {
                // Check parameter types
                for param in &func.params {
                    if let Some(tid) = param.type_id {
                        self.check_type_ref(tid, param.span);
                    }
                }
                // Check return type
                if let Some(ret_tid) = func.return_type {
                    self.check_type_ref(ret_tid, func.span);
                }
                // Check Let statements in body
                self.scan_stmts_for_type_refs(&func.body);
            }
        }
    }

    /// Check if a TypeId is an unresolved TypeRef (undefined type).
    fn check_type_ref(&mut self, tid: TypeId, span: Span) {
        if let Some(info) = self.registry.get(tid) {
            if let TypeKind::TypeRef { name } = &info.kind {
                // Skip common types that might be forward-declared by FFI or runtime
                let struct_key = format!("__struct_{}", name);
                let enum_key = format!("__enum_{}", name);
                // Skip built-in / FFI types that are always available
                let is_builtin = matches!(
                    name.as_str(),
                    "Fn" | "Function"
                        | "Callback"
                        | "Handler"
                        | "Request"
                        | "Response"
                        | "Range"
                        | "Json"
                        | "File"
                        | "Database"
                ) || name.starts_with("__");
                if !is_builtin
                    && self.scopes.lookup(&struct_key).is_none()
                    && self.scopes.lookup(&enum_key).is_none()
                {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::Undefined(name.clone()),
                        span,
                    });
                }
            }
        }
    }

    /// Scan statements for Let type annotations containing TypeRef.
    fn scan_stmts_for_type_refs(&mut self, stmts: &[HirStmt]) {
        for stmt in stmts {
            match &stmt.kind {
                HirStmtKind::Let { type_id, .. } => {
                    if let Some(tid) = type_id {
                        self.check_type_ref(*tid, stmt.span);
                    }
                }
                HirStmtKind::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    self.scan_stmts_for_type_refs(then_block);
                    if let Some(eb) = else_block {
                        self.scan_stmts_for_type_refs(eb);
                    }
                }
                HirStmtKind::While { body, .. } => {
                    self.scan_stmts_for_type_refs(body);
                }
                _ => {}
            }
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

            HirStmtKind::While {
                condition,
                body,
                increment,
            } => {
                // Condition must be Bool
                self.check_condition(condition);

                // Enter loop scope for the body
                self.scopes.enter_scope(super::scope::ScopeKind::Loop);
                for s in body {
                    self.check_stmt(s);
                }
                for s in increment {
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
                    if value_type != builtin::ANY
                        && *expected != builtin::ANY
                        && !self.registry.is_compatible(value_type, *expected)
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

                    // Deep element-level check for array literals with annotation
                    // e.g., let arr: [Int] = [1, "two", 3] — check each element against Int
                    if let HirExprKind::Array(elements) = &value.kind {
                        if let Some(info) = self.registry.get(*expected) {
                            if let TypeKind::Array {
                                element: expected_elem,
                            } = &info.kind
                            {
                                let expected_elem = *expected_elem;
                                if expected_elem != builtin::ANY {
                                    for (i, elem) in elements.iter().enumerate() {
                                        let elem_type =
                                            elem.type_id.unwrap_or_else(|| self.check_expr(elem));
                                        if elem_type != builtin::ANY
                                            && !self
                                                .registry
                                                .is_compatible(elem_type, expected_elem)
                                        {
                                            self.errors.push(TypeError {
                                                kind: TypeErrorKind::InvalidArrayElement {
                                                    expected: expected_elem,
                                                    found: elem_type,
                                                    index: i,
                                                },
                                                span: elem.span,
                                            });
                                        }
                                    }
                                }
                            }
                        }
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
                // Immutability check: plain variable assignment (x = val)
                if let HirExprKind::Local { name } = &target.kind {
                    if let Some(sym) = self.scopes.lookup(name) {
                        if !sym.mutable {
                            self.direct_errors.push(CompilerError::new(
                                ErrorCode::AssignToImmutable,
                                format!(
                                    "cannot assign to '{}': variable is immutable (use 'let mut')",
                                    name
                                ),
                                target.span,
                            ));
                        }
                    }
                }
                // Immutability check: index assignment (arr[i] = val, map[k] = val)
                else if let HirExprKind::Index { object, .. } = &target.kind {
                    if let HirExprKind::Local { name } = &object.kind {
                        if let Some(sym) = self.scopes.lookup(name) {
                            if !sym.mutable {
                                self.direct_errors.push(CompilerError::new(
                                    ErrorCode::AssignToImmutable,
                                    format!(
                                        "cannot modify '{}': variable is not mutable (use 'let mut')",
                                        name
                                    ),
                                    target.span,
                                ));
                            }
                        }
                    }
                }
                // Immutability check: field assignment (obj.field = val)
                else if let HirExprKind::Field { object, field } = &target.kind {
                    if let HirExprKind::Local { name } = &object.kind {
                        if let Some(sym) = self.scopes.lookup(name) {
                            if !sym.mutable {
                                self.direct_errors.push(CompilerError::new(
                                    ErrorCode::AssignToImmutable,
                                    format!(
                                        "cannot assign to '{}.{}': '{}' is not mutable",
                                        name, field, name
                                    ),
                                    target.span,
                                ));
                            }
                        }
                    }
                }

                let target_type = self.check_expr(target);
                let value_type = self.check_expr(value);

                // Type mismatch check: target type vs value type
                if target_type != builtin::ANY
                    && value_type != builtin::ANY
                    && target_type != builtin::VOID
                    && value_type != builtin::VOID
                    && !self.registry.is_compatible(value_type, target_type)
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

            // Expression statement — must type-check the expression
            HirStmtKind::Expr(expr) => {
                self.check_expr(expr);
            }

            // If statement — check condition and recurse into blocks
            HirStmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.check_condition(condition);

                self.scopes.enter_scope(super::scope::ScopeKind::Block);
                for s in then_block {
                    self.check_stmt(s);
                }
                self.scopes.exit_scope();

                if let Some(else_stmts) = else_block {
                    self.scopes.enter_scope(super::scope::ScopeKind::Block);
                    for s in else_stmts {
                        self.check_stmt(s);
                    }
                    self.scopes.exit_scope();
                }
            }

            // While loop — check condition and recurse into body
            HirStmtKind::While {
                condition,
                body,
                increment,
            } => {
                self.check_condition(condition);

                self.scopes.enter_scope(super::scope::ScopeKind::Block);
                for s in body {
                    self.check_stmt(s);
                }
                for s in increment {
                    self.check_stmt(s);
                }
                self.scopes.exit_scope();
            }

            // Break, Continue, Drop — no type checking needed
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

            HirExprKind::BinOp { lhs, rhs, op } => {
                let lhs_type = self.check_expr(lhs);
                let rhs_type = self.check_expr(rhs);
                self.validate_binop(op, lhs_type, rhs_type, expr.span);
                expr.type_id.unwrap_or(builtin::ANY)
            }

            HirExprKind::UnaryOp { operand, .. } => {
                self.check_expr(operand);
                expr.type_id.unwrap_or(builtin::ANY)
            }

            HirExprKind::Call { func, args } => {
                // Check all argument expressions
                let arg_types: Vec<TypeId> = args.iter().map(|a| self.check_expr(a)).collect();

                // Try to get the function return type
                // First, see if the func is a local reference (e.g., a function name)
                let func_return_type = if let HirExprKind::Local { name } = &func.kind {
                    // Built-in modules/functions don't need to be in scope
                    let is_builtin = ffi_names::is_builtin_module(name)
                        || name == "print"
                        || name == "panic"
                        || name == "toString"
                        || name == "sleep";

                    // Validate argument count and types against function signature
                    if let Some(param_types) = self.functions.get(name).cloned() {
                        // Check argument count
                        if arg_types.len() != param_types.len() {
                            self.errors.push(TypeError {
                                kind: TypeErrorKind::ArgMismatch {
                                    expected: param_types.len(),
                                    found: arg_types.len(),
                                },
                                span: expr.span,
                            });
                        } else {
                            // Check argument types
                            for (i, (arg_type, param_type)) in
                                arg_types.iter().zip(param_types.iter()).enumerate()
                            {
                                if let Some(expected) = param_type {
                                    if *arg_type != builtin::ANY
                                        && *expected != builtin::ANY
                                        && !self.registry.is_compatible(*arg_type, *expected)
                                    {
                                        self.errors.push(TypeError {
                                            kind: TypeErrorKind::Mismatch {
                                                expected: *expected,
                                                found: *arg_type,
                                            },
                                            span: args[i].span,
                                        });
                                    }
                                }
                            }
                        }
                    } else if !is_builtin && self.scopes.lookup(name).is_none() {
                        // Function not found in signatures or scope — undefined
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::Undefined(name.clone()),
                            span: func.span,
                        });
                    }

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
                let recv_type = self.check_expr(receiver);
                let mut first_arg_type = builtin::ANY;
                for (i, a) in args.iter().enumerate() {
                    let t = self.check_expr(a);
                    if i == 0 {
                        first_arg_type = t;
                    }
                }

                self.check_method_immutability(receiver, method, expr.span);

                // Type-check push arg for arrays
                if method == "push" && !args.is_empty() {
                    if let Some(info) = self.registry.get(recv_type) {
                        if let TypeKind::Array { element } = &info.kind {
                            let elem_type = *element;
                            if first_arg_type != builtin::ANY
                                && elem_type != builtin::ANY
                                && !self.registry.is_compatible(first_arg_type, elem_type)
                            {
                                self.errors.push(TypeError {
                                    kind: TypeErrorKind::Mismatch {
                                        expected: elem_type,
                                        found: first_arg_type,
                                    },
                                    span: args[0].span,
                                });
                            }
                        }
                    }
                }

                self.check_method_receiver_type(recv_type, method, expr.span);

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

                // Check all arms and track body types for consistency
                let mut first_arm_type: Option<TypeId> = None;

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

                    // Check body and track type
                    let body_type = self.check_expr(&arm.body);

                    // Check arm type consistency
                    if body_type != builtin::ANY && body_type != builtin::VOID {
                        if let Some(first) = first_arm_type {
                            if first != builtin::ANY
                                && !self.registry.is_compatible(body_type, first)
                            {
                                self.errors.push(TypeError {
                                    kind: TypeErrorKind::Mismatch {
                                        expected: first,
                                        found: body_type,
                                    },
                                    span: arm.body.span,
                                });
                            }
                        } else {
                            first_arm_type = Some(body_type);
                        }
                    }

                    // Exit arm scope
                    self.scopes.exit_scope();
                }

                first_arm_type.or(expr.type_id).unwrap_or(builtin::ANY)
            }

            // Error handling expressions - recurse into inner value
            HirExprKind::Ok(inner) | HirExprKind::Err(inner) | HirExprKind::Try(inner) => {
                self.check_expr(inner);
                expr.type_id.unwrap_or(builtin::ANY)
            }

            // Struct literal — delegate to helper
            HirExprKind::Struct { name, fields } => {
                self.check_struct_literal(name, fields, expr.span);
                expr.type_id
                    .unwrap_or_else(|| self.registry.lookup(name).unwrap_or(builtin::ANY))
            }

            // Array literal — check element type consistency
            HirExprKind::Array(elements) => {
                let mut elem_types: Vec<TypeId> = Vec::new();
                for elem in elements {
                    elem_types.push(self.check_expr(elem));
                }
                // Check that all elements have consistent types
                self.check_array_internal_consistency(&elem_types, elements);
                expr.type_id.unwrap_or(builtin::ANY)
            }

            // Map literal — delegate to helper
            HirExprKind::Map(entries) => {
                self.check_map_consistency(entries);
                expr.type_id.unwrap_or(builtin::ANY)
            }

            // Index access — validate index type for arrays/maps
            HirExprKind::Index { object, index } => {
                let obj_type = self.check_expr(object);
                let idx_type = self.check_expr(index);

                let mut resolved_elem_type = None;
                if let Some(info) = self.registry.get(obj_type) {
                    match &info.kind {
                        TypeKind::Array { element } => {
                            resolved_elem_type = Some(*element);
                            // Array index must be Int
                            if idx_type != builtin::ANY && idx_type != builtin::INT {
                                self.errors.push(TypeError {
                                    kind: TypeErrorKind::Mismatch {
                                        expected: builtin::INT,
                                        found: idx_type,
                                    },
                                    span: index.span,
                                });
                            }
                        }
                        TypeKind::Map { key, value } => {
                            resolved_elem_type = Some(*value);
                            // Map index must match key type
                            if idx_type != builtin::ANY
                                && *key != builtin::ANY
                                && !self.registry.is_compatible(idx_type, *key)
                            {
                                self.errors.push(TypeError {
                                    kind: TypeErrorKind::Mismatch {
                                        expected: *key,
                                        found: idx_type,
                                    },
                                    span: index.span,
                                });
                            }
                        }
                        _ => {}
                    }
                }
                // Return: explicit type_id > resolved element type > ANY
                expr.type_id.or(resolved_elem_type).unwrap_or(builtin::ANY)
            }

            // Field access — validate field existence on structs
            HirExprKind::Field { object, field } => {
                let obj_type = self.check_expr(object);

                let mut resolved_field_type = None;
                if obj_type != builtin::ANY {
                    if let Some(info) = self.registry.get(obj_type) {
                        if let TypeKind::Struct {
                            fields: declared, ..
                        } = &info.kind
                        {
                            if let Some((_, field_tid, _)) =
                                declared.iter().find(|(n, _, _)| n == field)
                            {
                                resolved_field_type = Some(*field_tid);
                            } else {
                                let type_name = if info.name.is_empty() {
                                    self.type_name(obj_type)
                                } else {
                                    info.name.clone()
                                };
                                self.errors.push(TypeError {
                                    kind: TypeErrorKind::UndefinedField {
                                        type_name,
                                        field: field.clone(),
                                    },
                                    span: expr.span,
                                });
                            }
                        }
                    }
                }
                // Return: explicit type_id > resolved field type > ANY
                expr.type_id.or(resolved_field_type).unwrap_or(builtin::ANY)
            }

            // Enum variant — delegate to helper
            HirExprKind::EnumVariant {
                enum_name,
                variant,
                payload,
            } => {
                self.check_enum_variant(enum_name, variant, payload, expr.span);
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

            HirExprKind::BinOp { lhs, rhs, op } => {
                let lhs_type = self.check_expr(lhs);
                let rhs_type = self.check_expr(rhs);
                self.validate_binop(op, lhs_type, rhs_type, expr.span);
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

            HirExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                let recv_type = self.check_expr(receiver);
                let mut first_arg_type = builtin::ANY;
                for (i, a) in args.iter().enumerate() {
                    let t = self.check_expr(a);
                    if i == 0 {
                        first_arg_type = t;
                    }
                }
                self.check_method_immutability(receiver, method, expr.span);
                self.check_method_receiver_type(recv_type, method, expr.span);
                // Type-check push arg for arrays
                if method == "push" && !args.is_empty() {
                    if let Some(info) = self.registry.get(recv_type) {
                        if let TypeKind::Array { element } = &info.kind {
                            let elem_type = *element;
                            if first_arg_type != builtin::ANY
                                && elem_type != builtin::ANY
                                && !self.registry.is_compatible(first_arg_type, elem_type)
                            {
                                self.errors.push(TypeError {
                                    kind: TypeErrorKind::Mismatch {
                                        expected: elem_type,
                                        found: first_arg_type,
                                    },
                                    span: args[0].span,
                                });
                            }
                        }
                    }
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

                // Check all arms with type consistency checking
                let mut first_arm_type: Option<TypeId> = None;

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

                    // Check body and track type
                    let body_type = self.check_expr(&arm.body);

                    // Check arm type consistency
                    if body_type != builtin::ANY && body_type != builtin::VOID {
                        if let Some(first) = first_arm_type {
                            if first != builtin::ANY
                                && !self.registry.is_compatible(body_type, first)
                            {
                                self.errors.push(TypeError {
                                    kind: TypeErrorKind::Mismatch {
                                        expected: first,
                                        found: body_type,
                                    },
                                    span: arm.body.span,
                                });
                            }
                        } else {
                            first_arm_type = Some(body_type);
                        }
                    }

                    // Exit arm scope
                    self.scopes.exit_scope();
                }
            }

            // Error handling expressions - recurse into inner value
            HirExprKind::Ok(inner) | HirExprKind::Err(inner) | HirExprKind::Try(inner) => {
                self.check_expr(inner);
            }

            // Struct literal — delegate to helper
            HirExprKind::Struct { name, fields } => {
                self.check_struct_literal(name, fields, expr.span);
            }

            // Array literal — check element type consistency even when type_id is pre-set
            HirExprKind::Array(elements) => {
                // Determine expected element type from the array's own type_id
                let expected_elem_type = expr.type_id.and_then(|arr_tid| {
                    self.registry.get(arr_tid).and_then(|info| {
                        if let TypeKind::Array { element } = &info.kind {
                            Some(*element)
                        } else {
                            None
                        }
                    })
                });

                let mut elem_types: Vec<TypeId> = Vec::new();
                for elem in elements {
                    elem_types.push(self.check_expr(elem));
                }

                if let Some(expected) = expected_elem_type {
                    if expected != builtin::ANY {
                        // Check each element against the declared element type
                        for (i, &et) in elem_types.iter().enumerate() {
                            if et != builtin::ANY && !self.registry.is_compatible(et, expected) {
                                self.errors.push(TypeError {
                                    kind: TypeErrorKind::InvalidArrayElement {
                                        expected,
                                        found: et,
                                        index: i,
                                    },
                                    span: elements[i].span,
                                });
                            }
                        }
                    } else {
                        // Element type is ANY — still check internal consistency
                        self.check_array_internal_consistency(&elem_types, elements);
                    }
                } else {
                    // No declared type — check internal consistency
                    self.check_array_internal_consistency(&elem_types, elements);
                }
            }

            // Enum variant — delegate to helper
            HirExprKind::EnumVariant {
                enum_name,
                variant,
                payload,
            } => {
                self.check_enum_variant(enum_name, variant, payload, expr.span);
            }

            // Index access — validate index type for arrays/maps even when type_id is pre-set
            HirExprKind::Index { object, index } => {
                self.check_expr(object);
                self.check_expr(index);
            }

            // Local variable reference — validate scope even when type_id is pre-set
            HirExprKind::Local { name } => {
                if !ffi_names::is_builtin_module(name)
                    && self.scopes.lookup(name).is_none()
                    && self.registry.lookup(name).is_none()
                    && name != "print"
                    && name != "panic"
                    && name != "toString"
                    && name != "sleep"
                {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::Undefined(name.clone()),
                        span: expr.span,
                    });
                }
            }

            // Map literal — delegate to helper
            HirExprKind::Map(entries) => {
                self.check_map_consistency(entries);
            }

            _ => {}
        }
    }

    /// Check that all array elements have consistent types (first element defines expected type).
    fn check_array_internal_consistency(&mut self, elem_types: &[TypeId], elements: &[HirExpr]) {
        if let Some(&first) = elem_types.first() {
            if first != builtin::ANY {
                for (i, &et) in elem_types.iter().enumerate().skip(1) {
                    if et != builtin::ANY && et != first && !self.registry.is_compatible(et, first)
                    {
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::InvalidArrayElement {
                                expected: first,
                                found: et,
                                index: i,
                            },
                            span: elements[i].span,
                        });
                    }
                }
            }
        }
    }

    /// Check if a method name is a known built-in method (not user-defined).
    fn is_known_builtin_method(method: &str) -> bool {
        matches!(
            method,
            "push"
                | "pop"
                | "remove"
                | "insert"
                | "clear"
                | "shift"
                | "unshift"
                | "len"
                | "contains"
                | "keys"
                | "values"
                | "entries"
                | "map"
                | "filter"
                | "reduce"
                | "forEach"
                | "find"
                | "sort"
                | "reverse"
                | "join"
                | "split"
                | "trim"
                | "toLowerCase"
                | "toUpperCase"
                | "startsWith"
                | "endsWith"
                | "includes"
                | "replace"
                | "slice"
                | "substring"
                | "charAt"
                | "indexOf"
                | "toString"
                | "toInt"
                | "toFloat"
                | "listen"
                | "get"
                | "post"
                | "put"
                | "delete"
                | "patch"
                | "options"
                | "head"
                | "use"
                | "auth"
                | "cors"
                | "static"
                | "middleware"
        )
    }

    /// Validate that a method call's method is defined on the receiver's type.
    #[inline(never)]
    fn check_method_receiver_type(&mut self, recv_type: TypeId, method: &str, span: Span) {
        if recv_type == builtin::ANY {
            return;
        }
        if let Some(info) = self.registry.get(recv_type) {
            if let TypeKind::Struct { .. } = &info.kind {
                let type_name = &info.name;
                if !type_name.is_empty() {
                    let mangled = format!("_method_{}_{}", type_name, method);
                    if !self.functions.contains_key(&mangled)
                        && !self.functions.contains_key(method)
                        && !Self::is_known_builtin_method(method)
                    {
                        let tn = type_name.clone();
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::UndefinedField {
                                type_name: tn,
                                field: method.to_string(),
                            },
                            span,
                        });
                    }
                }
            }
        }
    }

    /// Validate immutability for mutating collection methods.
    #[inline(never)]
    fn check_method_immutability(&mut self, receiver: &HirExpr, method: &str, span: Span) {
        let is_mutating = matches!(
            method,
            "push" | "pop" | "remove" | "insert" | "clear" | "shift" | "unshift"
        );
        if is_mutating {
            if let HirExprKind::Local { name } = &receiver.kind {
                if let Some(sym) = self.scopes.lookup(name) {
                    if !sym.mutable {
                        self.direct_errors.push(CompilerError::new(
                            ErrorCode::AssignToImmutable,
                            format!(
                                "cannot call '{}' on '{}': variable is immutable (use 'let mut')",
                                method, name
                            ),
                            span,
                        ));
                    }
                }
            }
        }
    }

    /// Validate map literal key/value type consistency.
    #[inline(never)]
    fn check_map_consistency(&mut self, entries: &[(HirExpr, HirExpr)]) {
        let mut key_types = Vec::new();
        let mut val_types = Vec::new();
        for (k, v) in entries {
            key_types.push(self.check_expr(k));
            val_types.push(self.check_expr(v));
        }
        if let Some(&first_key) = key_types.first() {
            if first_key != builtin::ANY {
                for (i, &kt) in key_types.iter().enumerate().skip(1) {
                    if kt != builtin::ANY && !self.registry.is_compatible(kt, first_key) {
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::InvalidOp(format!(
                                "map key type mismatch: expected {}, found {}",
                                self.type_name(first_key),
                                self.type_name(kt)
                            )),
                            span: entries[i].0.span,
                        });
                    }
                }
            }
        }
        if let Some(&first_val) = val_types.first() {
            if first_val != builtin::ANY {
                for (i, &vt) in val_types.iter().enumerate().skip(1) {
                    if vt != builtin::ANY && !self.registry.is_compatible(vt, first_val) {
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::InvalidOp(format!(
                                "map value type mismatch: expected {}, found {}",
                                self.type_name(first_val),
                                self.type_name(vt)
                            )),
                            span: entries[i].1.span,
                        });
                    }
                }
            }
        }
    }

    /// Validate struct literal fields and definition.
    #[inline(never)]
    fn check_struct_literal(&mut self, name: &str, fields: &[(String, HirExpr)], span: Span) {
        let struct_type_id = self.registry.lookup(name).unwrap_or(builtin::ANY);

        let is_defined_struct = self
            .registry
            .get(struct_type_id)
            .map(|info| matches!(info.kind, TypeKind::Struct { .. }))
            .unwrap_or(false);

        if is_defined_struct {
            let declared = self
                .registry
                .get(struct_type_id)
                .and_then(|info| {
                    if let TypeKind::Struct { fields: f, .. } = &info.kind {
                        Some(f.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            for (fname, fexpr) in fields {
                let value_type = self.check_expr(fexpr);
                if let Some((_, expected_type, _)) = declared.iter().find(|(n, _, _)| n == fname) {
                    if value_type != builtin::ANY
                        && *expected_type != builtin::ANY
                        && !self.registry.is_compatible(value_type, *expected_type)
                    {
                        self.direct_errors.push(
                            CompilerError::new(
                                ErrorCode::TypeMismatch,
                                format!(
                                    "{}.{}: expected {}, found {}",
                                    name,
                                    fname,
                                    self.type_name(*expected_type),
                                    self.type_name(value_type),
                                ),
                                fexpr.span,
                            )
                            .with_suggestion(format!(
                                "change value to type {}",
                                self.type_name(*expected_type)
                            )),
                        );
                    }
                } else {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::UnknownStructField {
                            struct_name: name.to_string(),
                            field: fname.clone(),
                        },
                        span: fexpr.span,
                    });
                }
            }

            let optional = self.struct_optional_fields.get(name);
            for (dname, _, _) in &declared {
                if optional.map_or(false, |s| s.contains(dname)) {
                    continue;
                }
                if !fields.iter().any(|(n, _)| n == dname) {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::MissingStructField {
                            struct_name: name.to_string(),
                            field: dname.clone(),
                        },
                        span,
                    });
                }
            }
        } else {
            let struct_key = format!("__struct_{}", name);
            if self.scopes.lookup(&struct_key).is_none() {
                self.errors.push(TypeError {
                    kind: TypeErrorKind::Undefined(name.to_string()),
                    span,
                });
            }
            for (_, fexpr) in fields {
                self.check_expr(fexpr);
            }
        }
    }

    /// Validate enum variant (existence, payload).
    #[inline(never)]
    fn check_enum_variant(
        &mut self,
        enum_name: &str,
        variant: &str,
        payload: &[HirExpr],
        span: Span,
    ) {
        for p in payload {
            self.check_expr(p);
        }

        let variant_info: Option<(bool, Option<TypeId>, bool)> = self
            .registry
            .lookup(enum_name)
            .and_then(|enum_type_id| self.registry.get(enum_type_id))
            .map(|info| {
                if let TypeKind::Enum { variants, .. } = &info.kind {
                    if let Some((_, declared_payload)) = variants.iter().find(|(v, _)| v == variant)
                    {
                        (true, declared_payload.clone(), true)
                    } else {
                        (false, None, true)
                    }
                } else {
                    (false, None, false)
                }
            });

        match variant_info {
            Some((true, declared_payload, true)) => match (&declared_payload, payload.len()) {
                (Some(_), 0) => {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::InvalidOp(format!(
                            "{}::{} requires a payload value",
                            enum_name, variant
                        )),
                        span,
                    });
                }
                (None, n) if n > 0 => {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::InvalidOp(format!(
                            "{}::{} does not take a payload",
                            enum_name, variant
                        )),
                        span,
                    });
                }
                (Some(expected_type), n) if n > 0 => {
                    // When the declared payload is a Tuple and multiple args are provided,
                    // check each arg against its corresponding tuple element type.
                    let tuple_elements: Option<Vec<TypeId>> =
                        self.registry.get(*expected_type).and_then(|info| {
                            if let TypeKind::Tuple { elements } = &info.kind {
                                Some(elements.clone())
                            } else {
                                None
                            }
                        });

                    if let Some(elements) = tuple_elements {
                        if n != elements.len() {
                            self.errors.push(TypeError {
                                kind: TypeErrorKind::InvalidOp(format!(
                                    "{}::{} expects {} payload values, got {}",
                                    enum_name,
                                    variant,
                                    elements.len(),
                                    n
                                )),
                                span,
                            });
                        } else {
                            for (i, elem_type) in elements.iter().enumerate() {
                                let payload_type = self.check_expr(&payload[i]);
                                if payload_type != builtin::ANY
                                    && *elem_type != builtin::ANY
                                    && !self.registry.is_compatible(payload_type, *elem_type)
                                {
                                    self.errors.push(TypeError {
                                        kind: TypeErrorKind::Mismatch {
                                            expected: *elem_type,
                                            found: payload_type,
                                        },
                                        span: payload[i].span,
                                    });
                                }
                            }
                        }
                    } else {
                        // Non-tuple payload: single value expected
                        if n > 1 {
                            self.errors.push(TypeError {
                                kind: TypeErrorKind::InvalidOp(format!(
                                    "{}::{} expects 1 payload value, got {}",
                                    enum_name, variant, n
                                )),
                                span,
                            });
                        } else {
                            let payload_type = self.check_expr(&payload[0]);
                            if payload_type != builtin::ANY
                                && *expected_type != builtin::ANY
                                && !self.registry.is_compatible(payload_type, *expected_type)
                            {
                                self.errors.push(TypeError {
                                    kind: TypeErrorKind::Mismatch {
                                        expected: *expected_type,
                                        found: payload_type,
                                    },
                                    span: payload[0].span,
                                });
                            }
                        }
                    }
                }
                _ => {}
            },
            Some((false, _, true)) => {
                self.errors.push(TypeError {
                    kind: TypeErrorKind::UndefinedVariant {
                        enum_name: enum_name.to_string(),
                        variant: variant.to_string(),
                    },
                    span,
                });
            }
            Some((_, _, false)) => {
                let enum_key = format!("__enum_{}", enum_name);
                if self.scopes.lookup(&enum_key).is_none() {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::Undefined(enum_name.to_string()),
                        span,
                    });
                }
            }
            None => {
                if self.registry.lookup(enum_name).is_some() {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::UndefinedVariant {
                            enum_name: enum_name.to_string(),
                            variant: variant.to_string(),
                        },
                        span,
                    });
                } else {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::Undefined(enum_name.to_string()),
                        span,
                    });
                }
            }
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
                        // If there are multiple bindings and the payload is a Tuple,
                        // decompose the tuple elements into individual binding types.
                        let element_types: Option<Vec<TypeId>> = if bindings.len() > 1 {
                            self.registry.get(payload_type_id).and_then(|info| {
                                if let TypeKind::Tuple { elements } = &info.kind {
                                    Some(elements.clone())
                                } else {
                                    None
                                }
                            })
                        } else {
                            None
                        };

                        if let Some(elems) = element_types {
                            // Multi-binding: assign each binding its tuple element type
                            for (i, binding) in bindings.iter().enumerate() {
                                let bind_type = elems.get(i).copied().unwrap_or(builtin::ANY);
                                self.define_symbol(Symbol {
                                    name: binding.clone(),
                                    kind: SymbolKind::Variable,
                                    type_id: Some(bind_type),
                                    mutable: false,
                                    span,
                                    used: false,
                                });
                            }
                        } else {
                            // Single binding: assign the payload type directly
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

    /// Validate binary operation type compatibility.
    fn validate_binop(&mut self, op: &HirBinOp, lhs_type: TypeId, rhs_type: TypeId, span: Span) {
        // Skip if either side is ANY (dynamic typing)
        if lhs_type == builtin::ANY || rhs_type == builtin::ANY {
            return;
        }

        match op {
            // Arithmetic: operands must be same type AND numeric (or Str for Add)
            HirBinOp::Add | HirBinOp::Sub | HirBinOp::Mul | HirBinOp::Div | HirBinOp::Mod => {
                let lhs_numeric = lhs_type == builtin::INT || lhs_type == builtin::FLOAT;
                let lhs_str = lhs_type == builtin::STR;
                let rhs_str = rhs_type == builtin::STR;

                // For Add, allow any type if either side is Str (auto-coerce to string concat)
                if !lhs_numeric && !(matches!(op, HirBinOp::Add) && (lhs_str || rhs_str)) {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::InvalidOp(format!(
                            "cannot apply arithmetic to {}",
                            self.type_name(lhs_type)
                        )),
                        span,
                    });
                    return;
                }

                if lhs_type != rhs_type {
                    // For Add, allow Str + non-Str (auto-coerce to string concatenation)
                    if matches!(op, HirBinOp::Add) && (lhs_type == builtin::STR || rhs_type == builtin::STR) {
                        // String concatenation with auto-coercion — allowed
                    } else {
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::Incompatible {
                                left: lhs_type,
                                right: rhs_type,
                                operation: format!("{:?}", op),
                            },
                            span,
                        });
                    }
                }
            }
            // Comparison: operands must be same type
            HirBinOp::Eq
            | HirBinOp::NotEq
            | HirBinOp::Lt
            | HirBinOp::Gt
            | HirBinOp::LtEq
            | HirBinOp::GtEq => {
                if lhs_type != rhs_type {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::Incompatible {
                            left: lhs_type,
                            right: rhs_type,
                            operation: format!("{:?}", op),
                        },
                        span,
                    });
                }
            }
            // Logical, bitwise, In — no additional checks here
            _ => {}
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
