//! Error Flow Analysis
//!
/// Validates that `Result` values are properly handled and that the `?`
/// operator is used only in functions that return `Result`.
///
/// ## Desugaring
///
/// The `?` operator desugars to:
/// ```text
/// match expr {
///     Ok(v) => v,
///     Err(e) => return Err(e)
/// }
/// ```
///
/// ## Checks
///
/// - `?` used in a function that doesn't return `Result` → error
/// - `Ok(v)` or `Err(e)` used in a non-Result function → error
/// - `Result` value used as a statement without `?` or handling → warning
/// - Error type mismatch: `E` must match the function's error return type
use doo_core::types::{builtin, TypeId, TypeKind, TypeRegistry};
use doo_core::Span;
use doo_hir::{HirExpr, HirExprKind, HirFunction, HirItem, HirProgram, HirStmt, HirStmtKind};
use doo_thir::{
    ThirExpr, ThirExprKind, ThirFunction, ThirItem, ThirProgram, ThirStmt, ThirStmtKind,
};

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

    pub fn message(&self) -> String {
        self.kind.message()
    }
}

/// Categories of error flow errors.
#[derive(Debug, Clone)]
pub enum ErrorFlowErrorKind {
    /// A `Result` value was produced but never handled.
    UnhandledResult { ok_type: TypeId, err_type: TypeId },
    /// `?` operator used in a function that doesn't return `Result`.
    TryInNonResultFunction { func_name: String },
    /// `Err(e)` used in a function without an error type.
    ErrInNonResultFunction { func_name: String },
    /// `Ok(v)` used in a function without an error type.
    OkInNonResultFunction { func_name: String },
    /// Function returning `Result` is missing an `Ok` return on some path.
    MissingOkPath { func_name: String },
    /// `??` (panic) used without a message.
    PanicWithoutMessage,
}

impl ErrorFlowErrorKind {
    pub fn message(&self) -> String {
        match self {
            Self::UnhandledResult { .. } => {
                "Result value not handled — use `?` to propagate or `let val ?? err = ...` to handle".to_string()
            }
            Self::TryInNonResultFunction { func_name } => {
                format!("`?` used in '{}' which doesn't return Result", func_name)
            }
            Self::ErrInNonResultFunction { func_name } => {
                format!("`Err` used in '{}' without error type", func_name)
            }
            Self::OkInNonResultFunction { func_name } => {
                format!("`Ok` used in '{}' without error type", func_name)
            }
            Self::MissingOkPath { func_name } => {
                format!("'{}' missing Ok return on some paths", func_name)
            }
            Self::PanicWithoutMessage => {
                "`??` (panic) used without message".to_string()
            }
        }
    }
}

/// Error flow checker.
///
/// Walks both HIR and THIR to validate error handling consistency.
/// THIR checking is preferred since types are fully resolved there.
pub struct ErrorFlowChecker<'a> {
    registry: &'a TypeRegistry,
    /// Collected errors.
    errors: Vec<ErrorFlowError>,
    /// Current function name being analyzed.
    current_func_name: String,
    /// Current function's error type (None if function doesn't return Result).
    current_func_error_type: Option<TypeId>,
}

impl<'a> ErrorFlowChecker<'a> {
    pub fn new(registry: &'a TypeRegistry) -> Self {
        Self {
            registry,
            errors: Vec::new(),
            current_func_name: String::new(),
            current_func_error_type: None,
        }
    }

    // ========================================================================
    // THIR-based checking (preferred — types are fully resolved)
    // ========================================================================

    /// Check a THIR program for error flow issues.
    pub fn check_thir(&mut self, program: &ThirProgram) -> Result<(), Vec<ErrorFlowError>> {
        for item in &program.items {
            if let ThirItem::Function(func) = item {
                self.check_thir_function(func);
            }
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.clone())
        }
    }

    fn check_thir_function(&mut self, func: &ThirFunction) {
        self.current_func_name = func.name.clone();
        self.current_func_error_type = func.error_type;

        for stmt in &func.body {
            self.check_thir_stmt(stmt);
        }

        self.current_func_error_type = None;
        self.current_func_name.clear();
    }

    fn check_thir_stmt(&mut self, stmt: &ThirStmt) {
        match &stmt.kind {
            ThirStmtKind::Let { value, .. } | ThirStmtKind::Const { value, .. } => {
                self.check_thir_expr(value);
            }
            ThirStmtKind::Assign { value, .. } => {
                self.check_thir_expr(value);
            }
            ThirStmtKind::Expr(expr) => {
                // Check if this is an unhandled Result expression
                if let Some(type_info) = self.registry.get(expr.ty) {
                    if let TypeKind::Result { ok, err } = &type_info.kind {
                        if !self.is_thir_try_expr(expr) {
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
                self.check_thir_expr(expr);
            }
            ThirStmtKind::Return(val) => {
                if let Some(e) = val {
                    self.check_thir_expr(e);
                }
            }
            ThirStmtKind::While {
                cond,
                body,
                increment,
            } => {
                self.check_thir_expr(cond);
                for s in body {
                    self.check_thir_stmt(s);
                }
                for s in increment {
                    self.check_thir_stmt(s);
                }
            }
            ThirStmtKind::Loop { body } => {
                for s in body {
                    self.check_thir_stmt(s);
                }
            }
            ThirStmtKind::Go { expr } => {
                self.check_thir_expr(expr);
            }
            ThirStmtKind::Scope { stmts } => {
                for s in stmts {
                    self.check_thir_stmt(s);
                }
            }
            ThirStmtKind::TupleLet { value, .. } => {
                self.check_thir_expr(value);
            }
            ThirStmtKind::ManualErrorExtract { expr, .. } => {
                self.check_thir_expr(expr);
            }
            ThirStmtKind::Drop { .. } | ThirStmtKind::Break(_) | ThirStmtKind::Continue => {}
        }
    }

    fn check_thir_expr(&mut self, expr: &ThirExpr) {
        match &expr.kind {
            ThirExprKind::Try(inner) => {
                // ? operator must be in a function returning Result
                if self.current_func_error_type.is_none() && self.current_func_name != "main" {
                    self.errors.push(ErrorFlowError::new(
                        ErrorFlowErrorKind::TryInNonResultFunction {
                            func_name: self.current_func_name.clone(),
                        },
                        expr.span,
                    ));
                }

                // Verify error type propagation — E must match function's error type
                if let Some(func_err) = self.current_func_error_type {
                    if let Some(inner_info) = self.registry.get(inner.ty) {
                        if let TypeKind::Result { err: inner_err, .. } = &inner_info.kind {
                            if *inner_err != func_err {
                                self.errors.push(ErrorFlowError::new(
                                    ErrorFlowErrorKind::UnhandledResult {
                                        ok_type: inner.ty,
                                        err_type: *inner_err,
                                    },
                                    expr.span,
                                ));
                            }
                        }
                    }
                }

                self.check_thir_expr(inner);
            }

            ThirExprKind::Ok(inner) => {
                if self.current_func_error_type.is_none() {
                    self.errors.push(ErrorFlowError::new(
                        ErrorFlowErrorKind::OkInNonResultFunction {
                            func_name: self.current_func_name.clone(),
                        },
                        expr.span,
                    ));
                }
                self.check_thir_expr(inner);
            }

            ThirExprKind::Err(inner) => {
                if self.current_func_error_type.is_none() {
                    self.errors.push(ErrorFlowError::new(
                        ErrorFlowErrorKind::ErrInNonResultFunction {
                            func_name: self.current_func_name.clone(),
                        },
                        expr.span,
                    ));
                }
                self.check_thir_expr(inner);
            }

            ThirExprKind::UnwrapOrPanic {
                expr: inner,
                message,
            } => {
                // Check for panic without meaningful message
                if let ThirExprKind::Literal(doo_thir::ThirLiteral::String(s)) = &message.kind {
                    if s.is_empty() {
                        self.errors.push(ErrorFlowError::new(
                            ErrorFlowErrorKind::PanicWithoutMessage,
                            expr.span,
                        ));
                    }
                }
                self.check_thir_expr(inner);
                self.check_thir_expr(message);
            }

            ThirExprKind::Binary { lhs, rhs, .. } => {
                self.check_thir_expr(lhs);
                self.check_thir_expr(rhs);
            }
            ThirExprKind::Unary { expr: inner, .. } => {
                self.check_thir_expr(inner);
            }
            ThirExprKind::Call { func, args } => {
                self.check_thir_expr(func);
                for a in args {
                    self.check_thir_expr(a);
                }
            }
            ThirExprKind::MethodCall { receiver, args, .. } => {
                self.check_thir_expr(receiver);
                for a in args {
                    self.check_thir_expr(a);
                }
            }
            ThirExprKind::FieldAccess { object, .. } => {
                self.check_thir_expr(object);
            }
            ThirExprKind::Index { object, index } => {
                self.check_thir_expr(object);
                self.check_thir_expr(index);
            }
            ThirExprKind::If { cond, then, else_ } => {
                self.check_thir_expr(cond);
                self.check_thir_expr(then);
                if let Some(e) = else_ {
                    self.check_thir_expr(e);
                }
            }
            ThirExprKind::Match {
                expr: scrutinee,
                arms,
            } => {
                self.check_thir_expr(scrutinee);
                for arm in arms {
                    if let Some(g) = &arm.guard {
                        self.check_thir_expr(g);
                    }
                    self.check_thir_expr(&arm.body);
                }
            }
            ThirExprKind::Block(stmts, tail) => {
                for s in stmts {
                    self.check_thir_stmt(s);
                }
                if let Some(e) = tail {
                    self.check_thir_expr(e);
                }
            }
            ThirExprKind::ArrayLiteral(elements) => {
                for e in elements {
                    self.check_thir_expr(e);
                }
            }
            ThirExprKind::MapLiteral(entries) => {
                for (k, v) in entries {
                    self.check_thir_expr(k);
                    self.check_thir_expr(v);
                }
            }
            ThirExprKind::StructLiteral { fields, .. } => {
                for (_, v) in fields {
                    self.check_thir_expr(v);
                }
            }
            ThirExprKind::EnumVariant { payload, .. } => {
                for p in payload {
                    self.check_thir_expr(p);
                }
            }
            ThirExprKind::Tuple(elements) => {
                for e in elements {
                    self.check_thir_expr(e);
                }
            }
            ThirExprKind::Spread(inner) => {
                self.check_thir_expr(inner);
            }
            ThirExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.check_thir_expr(s);
                }
                if let Some(e) = end {
                    self.check_thir_expr(e);
                }
            }
            ThirExprKind::Move(inner)
            | ThirExprKind::Clone(inner)
            | ThirExprKind::Async(inner)
            | ThirExprKind::Await(inner)
            | ThirExprKind::Spawn(inner) => {
                self.check_thir_expr(inner);
            }
            ThirExprKind::Borrow { expr: inner, .. } => {
                self.check_thir_expr(inner);
            }
            ThirExprKind::Closure { body, .. } => {
                self.check_thir_expr(body);
            }
            ThirExprKind::Cast { value, .. } => {
                self.check_thir_expr(value);
            }
            ThirExprKind::ScopeBlock { stmts } => {
                for s in stmts {
                    self.check_thir_stmt(s);
                }
            }
            ThirExprKind::Literal(_) | ThirExprKind::Var(_) => {}
        }
    }

    /// Check if an expression is a `?` (Try) operation.
    fn is_thir_try_expr(&self, expr: &ThirExpr) -> bool {
        matches!(expr.kind, ThirExprKind::Try(_))
    }

    // ========================================================================
    // HIR-based checking (fallback — used when THIR is not yet built)
    // ========================================================================

    /// Check an HIR program for error flow issues.
    pub fn check_hir(&mut self, program: &HirProgram) -> Result<(), Vec<ErrorFlowError>> {
        for item in &program.items {
            if let HirItem::Function(func) = item {
                self.check_hir_function(func);
            }
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.clone())
        }
    }

    fn check_hir_function(&mut self, func: &HirFunction) {
        self.current_func_name = func.name.clone();
        // Determine error type from return type
        self.current_func_error_type = func.return_type.and_then(|ret_ty| {
            self.registry.get(ret_ty).and_then(|info| {
                if let TypeKind::Result { err, .. } = &info.kind {
                    Some(*err)
                } else {
                    None
                }
            })
        });

        for stmt in &func.body {
            self.check_hir_stmt(stmt);
        }

        self.current_func_error_type = None;
        self.current_func_name.clear();
    }

    fn check_hir_stmt(&mut self, stmt: &HirStmt) {
        match &stmt.kind {
            HirStmtKind::Let { value, .. } => {
                self.check_hir_expr(value);
            }
            HirStmtKind::Assign { value, .. } => {
                self.check_hir_expr(value);
            }
            HirStmtKind::Expr(expr) => {
                self.check_hir_expr(expr);
            }
            HirStmtKind::Return(values) => {
                for v in values {
                    self.check_hir_expr(v);
                }
            }
            HirStmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.check_hir_expr(condition);
                for s in then_block {
                    self.check_hir_stmt(s);
                }
                if let Some(else_stmts) = else_block {
                    for s in else_stmts {
                        self.check_hir_stmt(s);
                    }
                }
            }
            HirStmtKind::While {
                condition,
                body,
                increment,
            } => {
                self.check_hir_expr(condition);
                for s in body {
                    self.check_hir_stmt(s);
                }
                for s in increment {
                    self.check_hir_stmt(s);
                }
            }
            _ => {}
        }
    }

    fn check_hir_expr(&mut self, expr: &HirExpr) {
        match &expr.kind {
            HirExprKind::Try(inner) => {
                if self.current_func_error_type.is_none() && self.current_func_name != "main" {
                    self.errors.push(ErrorFlowError::new(
                        ErrorFlowErrorKind::TryInNonResultFunction {
                            func_name: self.current_func_name.clone(),
                        },
                        expr.span,
                    ));
                }
                self.check_hir_expr(inner);
            }
            HirExprKind::Ok(inner) => {
                if self.current_func_error_type.is_none() {
                    self.errors.push(ErrorFlowError::new(
                        ErrorFlowErrorKind::OkInNonResultFunction {
                            func_name: self.current_func_name.clone(),
                        },
                        expr.span,
                    ));
                }
                self.check_hir_expr(inner);
            }
            HirExprKind::Err(inner) => {
                if self.current_func_error_type.is_none() {
                    self.errors.push(ErrorFlowError::new(
                        ErrorFlowErrorKind::ErrInNonResultFunction {
                            func_name: self.current_func_name.clone(),
                        },
                        expr.span,
                    ));
                }
                self.check_hir_expr(inner);
            }
            HirExprKind::UnwrapOrPanic {
                expr: inner,
                message,
            } => {
                self.check_hir_expr(inner);
                self.check_hir_expr(message);
            }
            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.check_hir_expr(lhs);
                self.check_hir_expr(rhs);
            }
            HirExprKind::UnaryOp { operand, .. } => {
                self.check_hir_expr(operand);
            }
            HirExprKind::Call { func, args } => {
                self.check_hir_expr(func);
                for a in args {
                    self.check_hir_expr(a);
                }
            }
            HirExprKind::MethodCall { receiver, args, .. } => {
                self.check_hir_expr(receiver);
                for a in args {
                    self.check_hir_expr(a);
                }
            }
            HirExprKind::Field { object, .. } => {
                self.check_hir_expr(object);
            }
            HirExprKind::Index { object, index } => {
                self.check_hir_expr(object);
                self.check_hir_expr(index);
            }
            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.check_hir_expr(condition);
                self.check_hir_expr(then_expr);
                if let Some(e) = else_expr {
                    self.check_hir_expr(e);
                }
            }
            HirExprKind::Match { values, arms } => {
                for v in values {
                    self.check_hir_expr(v);
                }
                for arm in arms {
                    if let Some(g) = &arm.guard {
                        self.check_hir_expr(g);
                    }
                    self.check_hir_expr(&arm.body);
                }
            }
            HirExprKind::Block { stmts, expr } => {
                for s in stmts {
                    self.check_hir_stmt(s);
                }
                if let Some(e) = expr {
                    self.check_hir_expr(e);
                }
            }
            _ => {}
        }
    }

    /// Get collected errors.
    pub fn errors(&self) -> &[ErrorFlowError] {
        &self.errors
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_flow_checker_creation() {
        let registry = TypeRegistry::new();
        let checker = ErrorFlowChecker::new(&registry);
        assert!(checker.errors.is_empty());
    }

    #[test]
    fn test_unhandled_result_message() {
        let err = ErrorFlowError::new(
            ErrorFlowErrorKind::UnhandledResult {
                ok_type: builtin::INT,
                err_type: builtin::STR,
            },
            Span::dummy(),
        );
        assert!(err.message().contains("Result"));
    }

    #[test]
    fn test_try_in_non_result_message() {
        let err = ErrorFlowError::new(
            ErrorFlowErrorKind::TryInNonResultFunction {
                func_name: "foo".into(),
            },
            Span::dummy(),
        );
        assert!(err.message().contains("foo"));
    }
}
