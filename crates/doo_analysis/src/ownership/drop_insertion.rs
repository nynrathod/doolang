//! Drop Insertion
//!
//! Automatically inserts Drop statements at optimal points to free memory.
//!
//! ## Doo's Drop Model
//!
//! Users don't call `drop()` or `free()`. The compiler:
//! - Tracks last use of each variable
//! - Inserts Drop immediately after last use
//! - Handles control flow (if/loop) correctly
//! - **Skips dropping variables that were moved** (ownership transferred)
//!
//! ## Algorithm
//!
//! 1. Find last use of each variable in the function
//! 2. Check if the last use was a Move (ownership transferred)
//! 3. If not moved, insert `Drop { name }` after the last use statement
//! 4. For control flow, drop at earliest exit point

use crate::{Decision, OwnershipResults};
use doo_core::Span;
use doo_hir::{
    HirExpr, HirExprKind, HirFunction, HirItem, HirProgram, HirStmt, HirStmtKind, HirVisitor,
    HirVisitorMut,
};
use rustc_hash::{FxHashMap, FxHashSet};

/// Drop insertion pass.
pub struct DropInserter<'a> {
    /// Last use index for each variable.
    last_use: FxHashMap<String, usize>,
    /// Last use span for each variable (for checking ownership decision).
    last_use_span: FxHashMap<String, Span>,
    /// Variables that need dropping (non-Copy types).
    needs_drop: FxHashSet<String>,
    /// Current statement index.
    current_idx: usize,
    /// Ownership results to check for Move decisions.
    ownership_results: Option<&'a OwnershipResults>,
}

impl<'a> DropInserter<'a> {
    /// Create a new drop inserter.
    pub fn new() -> Self {
        Self {
            last_use: FxHashMap::default(),
            last_use_span: FxHashMap::default(),
            needs_drop: FxHashSet::default(),
            current_idx: 0,
            ownership_results: None,
        }
    }

    /// Create a new drop inserter with ownership results.
    pub fn with_ownership_results(ownership_results: &'a OwnershipResults) -> Self {
        Self {
            last_use: FxHashMap::default(),
            last_use_span: FxHashMap::default(),
            needs_drop: FxHashSet::default(),
            current_idx: 0,
            ownership_results: Some(ownership_results),
        }
    }

    /// Insert drops into a program.
    pub fn insert_drops_program(&mut self, program: &mut HirProgram) {
        for item in &mut program.items {
            if let HirItem::Function(func) = item {
                self.insert_drops_function(func);
            }
        }
    }

    /// Insert drops into a function.
    pub fn insert_drops_function(&mut self, func: &mut HirFunction) {
        // Clear state
        self.last_use.clear();
        self.last_use_span.clear();
        self.needs_drop.clear();
        self.current_idx = 0;

        // Pass 1: Find all variables and their last uses
        self.find_last_uses(&func.body);

        // Pass 2: Insert drops
        let drops = self.compute_drops();
        self.apply_drops(&mut func.body, drops);
    }

    /// Find last use of each variable.
    fn find_last_uses(&mut self, stmts: &[HirStmt]) {
        for stmt in stmts {
            self.scan_stmt_for_uses(stmt);
            self.current_idx += 1;
        }
    }

    /// Scan a statement for variable uses.
    fn scan_stmt_for_uses(&mut self, stmt: &HirStmt) {
        match &stmt.kind {
            HirStmtKind::Let { name, value, .. } => {
                // Register variable as needing drop (simplified: all non-primitives)
                self.needs_drop.insert(name.clone());
                self.scan_expr_for_uses(value);
            }
            HirStmtKind::TupleLet { names, value, .. } => {
                // Register all variables from tuple unpacking as needing drop
                for name in names {
                    self.needs_drop.insert(name.clone());
                }
                self.scan_expr_for_uses(value);
            }
            HirStmtKind::ManualErrorExtract {
                ok_names,
                error_name,
                expr,
            } => {
                for name in ok_names {
                    if name != "_" {
                        self.needs_drop.insert(name.clone());
                    }
                }
                if error_name != "_" {
                    self.needs_drop.insert(error_name.clone());
                }
                self.scan_expr_for_uses(expr);
            }
            HirStmtKind::Assign { target, value } => {
                self.scan_expr_for_uses(target);
                self.scan_expr_for_uses(value);
            }
            HirStmtKind::Expr(expr) => {
                self.scan_expr_for_uses(expr);
            }
            HirStmtKind::Return(values) => {
                for v in values {
                    self.scan_expr_for_uses(v);
                }
            }
            HirStmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.scan_expr_for_uses(condition);
                // Recurse into blocks
                let saved_idx = self.current_idx;
                self.find_last_uses(then_block);
                self.current_idx = saved_idx;
                if let Some(else_stmts) = else_block {
                    self.find_last_uses(else_stmts);
                }
            }
            HirStmtKind::While {
                condition,
                body,
                increment,
            } => {
                self.scan_expr_for_uses(condition);
                let saved_idx = self.current_idx;
                self.find_last_uses(body);
                self.find_last_uses(increment);
                self.current_idx = saved_idx;
            }
            HirStmtKind::Break | HirStmtKind::Continue | HirStmtKind::Drop { .. } => {}
        }
    }

    /// Scan an expression for variable uses.
    fn scan_expr_for_uses(&mut self, expr: &HirExpr) {
        match &expr.kind {
            HirExprKind::Local { name } => {
                // Record this as a use, along with its span for ownership decision lookup
                self.last_use.insert(name.clone(), self.current_idx);
                self.last_use_span.insert(name.clone(), expr.span);
            }
            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.scan_expr_for_uses(lhs);
                self.scan_expr_for_uses(rhs);
            }
            HirExprKind::UnaryOp { operand, .. } => {
                self.scan_expr_for_uses(operand);
            }
            HirExprKind::Call { func, args } => {
                self.scan_expr_for_uses(func);
                for arg in args {
                    self.scan_expr_for_uses(arg);
                }
            }
            HirExprKind::MethodCall { receiver, args, .. } => {
                self.scan_expr_for_uses(receiver);
                for arg in args {
                    self.scan_expr_for_uses(arg);
                }
            }
            HirExprKind::Field { object, .. } => {
                self.scan_expr_for_uses(object);
            }
            HirExprKind::Index { object, index } => {
                self.scan_expr_for_uses(object);
                self.scan_expr_for_uses(index);
            }
            HirExprKind::Array(elements) | HirExprKind::Tuple(elements) => {
                for elem in elements {
                    self.scan_expr_for_uses(elem);
                }
            }
            HirExprKind::Map(entries) => {
                for (k, v) in entries {
                    self.scan_expr_for_uses(k);
                    self.scan_expr_for_uses(v);
                }
            }
            HirExprKind::Struct { fields, .. } => {
                for (_, value) in fields {
                    self.scan_expr_for_uses(value);
                }
            }
            HirExprKind::EnumVariant { payload, .. } => {
                for e in payload {
                    self.scan_expr_for_uses(e);
                }
            }
            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.scan_expr_for_uses(condition);
                self.scan_expr_for_uses(then_expr);
                if let Some(e) = else_expr {
                    self.scan_expr_for_uses(e);
                }
            }

            HirExprKind::Match { values, arms } => {
                for v in values {
                    self.scan_expr_for_uses(v);
                }
                for arm in arms {
                    self.scan_match_pattern_for_uses(&arm.pattern);
                    if let Some(g) = &arm.guard {
                        self.scan_expr_for_uses(g);
                    }
                    self.scan_expr_for_uses(&arm.body);
                }
            }
            HirExprKind::Block { stmts, expr } => {
                for s in stmts {
                    self.scan_stmt_for_uses(s);
                }
                if let Some(e) = expr {
                    self.scan_expr_for_uses(e);
                }
            }
            HirExprKind::Range { start, end, .. } => {
                self.scan_expr_for_uses(start);
                self.scan_expr_for_uses(end);
            }
            HirExprKind::Ok(inner)
            | HirExprKind::Err(inner)
            | HirExprKind::Try(inner)
            | HirExprKind::Move(inner)
            | HirExprKind::Clone(inner) => {
                self.scan_expr_for_uses(inner);
            }
            HirExprKind::UnwrapOrPanic {
                expr: inner,
                message,
            } => {
                self.scan_expr_for_uses(inner);
                self.scan_expr_for_uses(message);
            }
            HirExprKind::Borrow { expr: inner, .. } => {
                self.scan_expr_for_uses(inner);
            }
            HirExprKind::Closure { .. } => {
                // Do NOT recurse into closure bodies.
                // Closures are built as separate MIR functions with their own scope.
                // Traversing into them would treat closure params/locals as outer variables,
                // causing invalid Drop insertions in the outer function.
            }
            HirExprKind::Spread(inner) => {
                self.scan_expr_for_uses(inner);
            }
            HirExprKind::RouteBlock { routes } => {
                for route in routes {
                    self.scan_expr_for_uses(route);
                }
            }
            HirExprKind::Cast { value, .. } => {
                self.scan_expr_for_uses(value);
            }

            // Async & concurrency
            HirExprKind::Await(inner) | HirExprKind::Spawn { body: inner } => {
                self.scan_expr_for_uses(inner);
            }
            HirExprKind::ScopeBlock { stmts } => {
                for s in stmts {
                    self.scan_stmt_for_uses(s);
                }
            }

            HirExprKind::Const(_) | HirExprKind::Global { .. } => {}
        }
    }

    fn scan_match_pattern_for_uses(&mut self, p: &doo_hir::HirMatchPattern) {
        match p {
            doo_hir::HirMatchPattern::Literal(e) | doo_hir::HirMatchPattern::Condition(e) => {
                self.scan_expr_for_uses(e)
            }
            doo_hir::HirMatchPattern::Wildcard
            | doo_hir::HirMatchPattern::EnumVariant { .. }
            | doo_hir::HirMatchPattern::EnumVariantPayload { .. } => {}
            doo_hir::HirMatchPattern::Tuple(parts) => {
                for x in parts {
                    self.scan_match_pattern_for_uses(x);
                }
            }
        }
    }

    /// Compute which drops to insert and where.
    fn compute_drops(&self) -> Vec<(usize, String)> {
        let mut drops = Vec::new();

        for (name, &last_idx) in &self.last_use {
            // Only insert drops for non-primitive variables
            if !self.needs_drop.contains(name) {
                continue;
            }

            // CRITICAL: Don't drop variables whose last use was a Move
            // When a variable is moved, ownership is transferred - no drop needed
            if let Some(ownership_results) = self.ownership_results {
                if let Some(span) = self.last_use_span.get(name) {
                    if let Some(decision) = ownership_results.get_decision(name, *span) {
                        if matches!(decision, Decision::Move) {
                            // Variable was moved at its last use - skip drop
                            continue;
                        }
                    }
                }
            }

            drops.push((last_idx + 1, name.clone()));
        }

        // Sort by position (descending) for safe insertion
        drops.sort_by(|a, b| b.0.cmp(&a.0));
        drops
    }

    /// Apply drop insertions to statement list.
    fn apply_drops(&self, stmts: &mut Vec<HirStmt>, drops: Vec<(usize, String)>) {
        for (idx, name) in drops {
            if idx <= stmts.len() {
                let drop_stmt = HirStmt::new(HirStmtKind::Drop { name }, Span::dummy());
                stmts.insert(idx, drop_stmt);
            }
        }
    }
}

impl<'a> Default for DropInserter<'a> {
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
    fn test_drop_inserter_creation() {
        let inserter = DropInserter::new();
        assert!(inserter.last_use.is_empty());
        assert!(inserter.needs_drop.is_empty());
    }

    #[test]
    fn test_compute_drops_ordering() {
        let mut inserter = DropInserter::new();
        inserter.last_use.insert("a".to_string(), 5);
        inserter.last_use.insert("b".to_string(), 2);
        inserter.last_use.insert("c".to_string(), 8);
        inserter.needs_drop.insert("a".to_string());
        inserter.needs_drop.insert("b".to_string());
        inserter.needs_drop.insert("c".to_string());

        let drops = inserter.compute_drops();

        // Should be sorted descending by index
        assert_eq!(drops.len(), 3);
        assert!(drops[0].0 > drops[1].0);
        assert!(drops[1].0 > drops[2].0);
    }
}
