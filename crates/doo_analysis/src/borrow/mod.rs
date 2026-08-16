//! Borrow Checking
//!
//! Ensures safe memory access patterns:
//! - Only ONE mutable borrow OR multiple immutable borrows at a time
//! - The ONLY error users can get: concurrent mutable borrow
//!
//! ## Doo's Borrow Model
//!
//! Unlike Rust, users don't write `&` or `&mut`. The compiler:
//! - Auto-borrows for function arguments (immutable by default)
//! - Auto-borrows for method receivers
//! - Detects concurrent mutable access and errors
//!
//! ## Implementation
//!
//! Track active borrows per variable:
//! - When variable is read: add immutable borrow
//! - When variable is written: check no other borrows, add mutable borrow
//! - At scope end: release all borrows from that scope

use doo_core::Span;
use doo_hir::{HirExpr, HirExprKind, HirFunction, HirItem, HirProgram, HirStmt, HirStmtKind};
use rustc_hash::FxHashMap;

/// Borrow checking error.
#[derive(Debug, Clone)]
pub struct BorrowError {
    pub kind: BorrowErrorKind,
    pub span: Span,
}

/// Types of borrow errors.
#[derive(Debug, Clone)]
pub enum BorrowErrorKind {
    /// Trying to mutably borrow while already borrowed.
    ConcurrentMutableBorrow {
        variable: String,
        existing_borrow_span: Span,
    },
    /// Trying to borrow while mutably borrowed.
    BorrowWhileMutablyBorrowed {
        variable: String,
        mutable_borrow_span: Span,
    },
    /// Modifying a borrowed variable.
    ModifyWhileBorrowed { variable: String, borrow_span: Span },
}

impl BorrowError {
    pub fn concurrent_mut(variable: String, existing: Span, at: Span) -> Self {
        Self {
            kind: BorrowErrorKind::ConcurrentMutableBorrow {
                variable,
                existing_borrow_span: existing,
            },
            span: at,
        }
    }

    pub fn borrow_while_mut(variable: String, mut_span: Span, at: Span) -> Self {
        Self {
            kind: BorrowErrorKind::BorrowWhileMutablyBorrowed {
                variable,
                mutable_borrow_span: mut_span,
            },
            span: at,
        }
    }

    pub fn message(&self) -> String {
        match &self.kind {
            BorrowErrorKind::ConcurrentMutableBorrow { variable, .. } => {
                format!("Cannot mutably borrow '{}' - already borrowed", variable)
            }
            BorrowErrorKind::BorrowWhileMutablyBorrowed { variable, .. } => {
                format!("Cannot borrow '{}' - already mutably borrowed", variable)
            }
            BorrowErrorKind::ModifyWhileBorrowed { variable, .. } => {
                format!("Cannot modify '{}' - currently borrowed", variable)
            }
        }
    }
}

/// Information about an active borrow.
#[derive(Debug, Clone)]
pub struct BorrowInfo {
    /// Who/what is borrowing (expression or statement).
    pub borrower: String,
    /// Is this a mutable borrow?
    pub mutable: bool,
    /// Where the borrow started.
    pub span: Span,
    /// Scope depth when borrow was created.
    pub scope_depth: usize,
}

/// Borrow checker.
///
/// Tracks active borrows and detects concurrent mutable access.
pub struct BorrowChecker {
    /// Active borrows for each variable: variable name -> list of borrows.
    active_borrows: FxHashMap<String, Vec<BorrowInfo>>,
    /// Current scope depth.
    scope_depth: usize,
    /// Collected errors.
    errors: Vec<BorrowError>,
}

impl BorrowChecker {
    /// Create a new borrow checker.
    pub fn new() -> Self {
        Self {
            active_borrows: FxHashMap::default(),
            scope_depth: 0,
            errors: Vec::new(),
        }
    }

    /// Check a program for borrow violations.
    pub fn check(&mut self, program: &HirProgram) -> Result<(), Vec<BorrowError>> {
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
        // Clear state for new function
        self.active_borrows.clear();
        self.scope_depth = 0;

        // Check all statements
        for stmt in &func.body {
            self.check_stmt(stmt);
        }
    }

    /// Check a statement.
    fn check_stmt(&mut self, stmt: &HirStmt) {
        match &stmt.kind {
            HirStmtKind::Let { name, value, .. } => {
                // Check value expression
                self.check_expr(value, false);
                // New variable - no borrows yet
                self.active_borrows.remove(name);
            }

            HirStmtKind::TupleLet { names, value, .. } => {
                // Check value expression
                self.check_expr(value, false);
                // New variables - no borrows yet
                for name in names {
                    self.active_borrows.remove(name);
                }
            }

            HirStmtKind::ManualErrorExtract {
                ok_names,
                error_name,
                expr,
            } => {
                self.check_expr(expr, false);
                for name in ok_names {
                    if name != "_" {
                        self.active_borrows.remove(name);
                    }
                }
                if error_name != "_" {
                    self.active_borrows.remove(error_name);
                }
            }

            HirStmtKind::Assign { target, value } => {
                // Check the value being assigned FIRST (before we clear borrows on target)
                self.check_expr(value, false);

                // Assignment replaces the value - clear any borrows on the target
                // This is safe because:
                // 1. We already checked the RHS which may read from the target
                // 2. The assignment creates a new value, invalidating old borrows
                if let HirExprKind::Local { name } = &target.kind {
                    self.active_borrows.remove(name);
                }
            }

            HirStmtKind::Expr(expr) => {
                self.check_expr(expr, false);
            }

            HirStmtKind::Return(values) => {
                for v in values {
                    self.check_expr(v, false);
                }
            }

            HirStmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                // Check condition in its own temporary scope
                // Borrows from condition don't persist into then/else blocks
                // This is a simplified NLL: condition borrows are temporary
                self.enter_scope();
                self.check_expr(condition, false);
                self.exit_scope();

                // Enter scope for then block
                self.enter_scope();
                for s in then_block {
                    self.check_stmt(s);
                }
                self.exit_scope();

                // Enter scope for else block
                if let Some(else_stmts) = else_block {
                    self.enter_scope();
                    for s in else_stmts {
                        self.check_stmt(s);
                    }
                    self.exit_scope();
                }
            }

            HirStmtKind::While {
                condition,
                body,
                increment,
            } => {
                // Check condition in its own temporary scope
                // Borrows from condition don't persist into loop body
                self.enter_scope();
                self.check_expr(condition, false);
                self.exit_scope();

                // Enter scope for loop body
                self.enter_scope();
                for s in body {
                    self.check_stmt(s);
                }
                for s in increment {
                    self.check_stmt(s);
                }
                self.exit_scope();
            }

            HirStmtKind::Break | HirStmtKind::Continue | HirStmtKind::Drop { .. } => {}
        }
    }

    /// Check an expression.
    /// `is_mutable_context` indicates if this expression is being mutated.
    fn check_expr(&mut self, expr: &HirExpr, is_mutable_context: bool) {
        match &expr.kind {
            HirExprKind::Local { name } => {
                if is_mutable_context {
                    // Mutable borrow - check no other borrows exist
                    self.try_mutable_borrow(name, expr.span);
                } else {
                    // Immutable borrow - check no mutable borrows exist
                    self.try_immutable_borrow(name, expr.span);
                }
            }

            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.check_expr(lhs, false);
                self.check_expr(rhs, false);
            }

            HirExprKind::UnaryOp { operand, .. } => {
                self.check_expr(operand, false);
            }

            HirExprKind::Call { func, args } => {
                self.check_expr(func, false);
                // Function arguments: auto-borrow (immutable by default)
                for arg in args {
                    self.check_expr(arg, false);
                }
            }

            HirExprKind::MethodCall { receiver, args, .. } => {
                // Method receiver: typically borrowed
                self.check_expr(receiver, false);
                for arg in args {
                    self.check_expr(arg, false);
                }
            }

            HirExprKind::Field { object, .. } => {
                self.check_expr(object, is_mutable_context);
            }

            HirExprKind::Index { object, index } => {
                self.check_expr(object, is_mutable_context);
                self.check_expr(index, false);
            }

            HirExprKind::Array(elements) | HirExprKind::Tuple(elements) => {
                for elem in elements {
                    self.check_expr(elem, false);
                }
            }

            HirExprKind::Map(entries) => {
                for (k, v) in entries {
                    self.check_expr(k, false);
                    self.check_expr(v, false);
                }
            }

            HirExprKind::Struct { fields, .. } => {
                for (_, value) in fields {
                    self.check_expr(value, false);
                }
            }

            HirExprKind::EnumVariant { payload, .. } => {
                for e in payload {
                    self.check_expr(e, false);
                }
            }

            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.check_expr(condition, false);
                self.check_expr(then_expr, false);
                if let Some(e) = else_expr {
                    self.check_expr(e, false);
                }
            }

            HirExprKind::Match { values, arms } => {
                for v in values {
                    self.check_expr(v, false);
                }
                for arm in arms {
                    self.check_match_pattern(&arm.pattern);
                    if let Some(g) = &arm.guard {
                        self.check_expr(g, false);
                    }
                    self.check_expr(&arm.body, false);
                }
            }

            HirExprKind::Block { stmts, expr } => {
                self.enter_scope();
                for s in stmts {
                    self.check_stmt(s);
                }
                if let Some(e) = expr {
                    self.check_expr(e, false);
                }
                self.exit_scope();
            }

            HirExprKind::Range { start, end, .. } => {
                self.check_expr(start, false);
                self.check_expr(end, false);
            }

            HirExprKind::Ok(inner)
            | HirExprKind::Err(inner)
            | HirExprKind::Try(inner)
            | HirExprKind::Move(inner)
            | HirExprKind::Clone(inner) => {
                self.check_expr(inner, false);
            }

            HirExprKind::UnwrapOrPanic {
                expr: inner,
                message,
            } => {
                self.enter_scope();
                self.check_expr(inner, false);
                self.check_expr(message, false);
                self.exit_scope();
            }

            HirExprKind::Borrow {
                expr: inner,
                mutable,
            } => {
                self.check_expr(inner, *mutable);
            }

            HirExprKind::Closure { body, .. } => {
                // Closures create new scope
                self.enter_scope();
                self.check_expr(body, false);
                self.exit_scope();
            }

            HirExprKind::Spread(inner) => {
                self.check_expr(inner, false);
            }

            HirExprKind::Cast { value, .. } => {
                self.check_expr(value, false);
            }

            // Async & concurrency
            HirExprKind::Await(inner) | HirExprKind::Spawn { body: inner } => {
                self.check_expr(inner, false);
            }
            HirExprKind::ScopeBlock { stmts } => {
                for s in stmts {
                    self.check_stmt(s);
                }
            }

            HirExprKind::Const(_) | HirExprKind::Global { .. } => {}
        }
    }

    fn check_match_pattern(&mut self, p: &doo_hir::HirMatchPattern) {
        match p {
            doo_hir::HirMatchPattern::Literal(e) | doo_hir::HirMatchPattern::Condition(e) => {
                self.check_expr(e, false)
            }
            doo_hir::HirMatchPattern::Wildcard
            | doo_hir::HirMatchPattern::EnumVariant { .. }
            | doo_hir::HirMatchPattern::EnumVariantPayload { .. } => {}
            doo_hir::HirMatchPattern::Tuple(parts) => {
                for x in parts {
                    self.check_match_pattern(x);
                }
            }
        }
    }

    // ========================================================================
    // Borrow Management
    // ========================================================================

    /// Try to take an immutable borrow.
    fn try_immutable_borrow(&mut self, name: &str, span: Span) {
        // Skip borrow tracking for internal/synthetic variables (e.g., __i_idx from for-loop desugaring)
        if name.starts_with("__") {
            return;
        }

        if let Some(borrows) = self.active_borrows.get(name) {
            // Check for existing mutable borrow
            for borrow in borrows {
                if borrow.mutable {
                    self.errors.push(BorrowError::borrow_while_mut(
                        name.to_string(),
                        borrow.span,
                        span,
                    ));
                    return;
                }
            }
        }

        // Add immutable borrow
        self.add_borrow(name, false, span);
    }

    /// Try to take a mutable borrow.
    fn try_mutable_borrow(&mut self, name: &str, span: Span) {
        // Skip borrow tracking for internal/synthetic variables (e.g., __i_idx from for-loop desugaring)
        if name.starts_with("__") {
            return;
        }

        if let Some(borrows) = self.active_borrows.get(name) {
            // Any existing borrow blocks mutable borrow
            if !borrows.is_empty() {
                let first = &borrows[0];
                self.errors.push(BorrowError::concurrent_mut(
                    name.to_string(),
                    first.span,
                    span,
                ));
                return;
            }
        }

        // Add mutable borrow
        self.add_borrow(name, true, span);
    }

    /// Add a borrow to the active set.
    fn add_borrow(&mut self, name: &str, mutable: bool, span: Span) {
        let info = BorrowInfo {
            borrower: format!("expr@{}", span.start),
            mutable,
            span,
            scope_depth: self.scope_depth,
        };

        self.active_borrows
            .entry(name.to_string())
            .or_default()
            .push(info);
    }

    /// Enter a new scope.
    fn enter_scope(&mut self) {
        self.scope_depth += 1;
    }

    /// Exit current scope, releasing all borrows from this scope.
    fn exit_scope(&mut self) {
        // Remove all borrows from this scope
        for borrows in self.active_borrows.values_mut() {
            borrows.retain(|b| b.scope_depth < self.scope_depth);
        }
        self.scope_depth = self.scope_depth.saturating_sub(1);
    }

    /// Get collected errors.
    pub fn errors(&self) -> &[BorrowError] {
        &self.errors
    }
}

impl Default for BorrowChecker {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_borrow_checker_creation() {
        let checker = BorrowChecker::new();
        assert!(checker.errors.is_empty());
        assert_eq!(checker.scope_depth, 0);
    }

    #[test]
    fn test_scope_enter_exit() {
        let mut checker = BorrowChecker::new();
        assert_eq!(checker.scope_depth, 0);

        checker.enter_scope();
        assert_eq!(checker.scope_depth, 1);

        checker.enter_scope();
        assert_eq!(checker.scope_depth, 2);

        checker.exit_scope();
        assert_eq!(checker.scope_depth, 1);

        checker.exit_scope();
        assert_eq!(checker.scope_depth, 0);
    }

    #[test]
    fn test_error_message() {
        let err = BorrowError::concurrent_mut("x".to_string(), Span::dummy(), Span::dummy());
        assert!(err.message().contains("mutably borrow"));
    }
}
