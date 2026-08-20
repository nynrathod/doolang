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
use doo_hir::{HirExpr, HirExprKind, HirFunction, HirItem, HirProgram, HirStmt, HirStmtKind};
use rustc_hash::{FxHashMap, FxHashSet};

// ============================================================================
// THIR-Based Drop Insertion (MMM Part IV)
// ============================================================================

use crate::ownership::{is_copy, needs_drop, ThirOwnershipResults};
use doo_core::types::{TypeKind, TypeRegistry};
use doo_thir::{ThirExpr, ThirExprKind, ThirFunction, ThirStmt, ThirStmtKind};

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

        // Pass 2: Insert drops at last-use points
        let drops = self.compute_drops();
        // Keep track of which variables get dropped (not moved)
        let dropped_vars: FxHashSet<String> = drops.iter().map(|(_, name)| name.clone()).collect();
        self.apply_drops(&mut func.body, drops);

        // Pass 3: Insert cleanup drops before early returns in nested blocks
        // This fixes Leak 3 (early return skipping drops scheduled later)
        Self::insert_early_return_drops(&mut func.body, &dropped_vars);
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
            HirExprKind::Closure { params, body } => {
                // Closures are built as separate MIR functions with their own scope.
                // Do NOT add closure params/locals to needs_drop in the outer function.
                // BUT we MUST scan for outer-scope variable references so that
                // captured variables aren't dropped before the closure is created.
                // This is the same pattern used for Spawn bodies (Leak 5 fix).
                let param_names: FxHashSet<String> =
                    params.iter().map(|(name, _)| name.clone()).collect();
                self.scan_closure_body_for_outer_uses(body, &param_names);
            }
            HirExprKind::Spread(inner) => {
                self.scan_expr_for_uses(inner);
            }
            HirExprKind::Cast { value, .. } => {
                self.scan_expr_for_uses(value);
            }

            // Async & concurrency
            HirExprKind::Await(inner) => {
                self.scan_expr_for_uses(inner);
            }
            HirExprKind::Spawn { body } => {
                // Do NOT fully recurse into spawn bodies — they have their own scope.
                // BUT we MUST scan for outer-scope variable references so that
                // captured variables aren't dropped before the Spawn instruction.
                // Without this, the drop inserter frees captured structs (e.g., db)
                // before the go block starts, causing use-after-free.
                self.scan_spawn_body_for_outer_uses(body);
            }
            HirExprKind::ScopeBlock { stmts } => {
                for s in stmts {
                    self.scan_stmt_for_uses(s);
                }
            }

            HirExprKind::Const(_) | HirExprKind::Global { .. } => {}
        }
    }

    /// Scan a Spawn body for outer-scope Local references to extend last_use.
    /// Only updates `last_use` — does NOT add to `needs_drop` (spawn-local
    /// variables have their own scope and should not be dropped in the outer function).
    fn scan_spawn_body_for_outer_uses(&mut self, expr: &HirExpr) {
        match &expr.kind {
            HirExprKind::Local { name } => {
                // Only update if already known (outer-scope variable).
                // This avoids adding spawn-internal variables to last_use.
                if self.needs_drop.contains(name) || self.last_use.contains_key(name) {
                    self.last_use.insert(name.clone(), self.current_idx);
                    self.last_use_span.insert(name.clone(), expr.span);
                }
            }
            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.scan_spawn_body_for_outer_uses(lhs);
                self.scan_spawn_body_for_outer_uses(rhs);
            }
            HirExprKind::UnaryOp { operand, .. } => {
                self.scan_spawn_body_for_outer_uses(operand);
            }
            HirExprKind::Call { func, args } => {
                self.scan_spawn_body_for_outer_uses(func);
                for a in args {
                    self.scan_spawn_body_for_outer_uses(a);
                }
            }
            HirExprKind::MethodCall { receiver, args, .. } => {
                self.scan_spawn_body_for_outer_uses(receiver);
                for a in args {
                    self.scan_spawn_body_for_outer_uses(a);
                }
            }
            HirExprKind::Field { object, .. } => {
                self.scan_spawn_body_for_outer_uses(object);
            }
            HirExprKind::Index { object, index } => {
                self.scan_spawn_body_for_outer_uses(object);
                self.scan_spawn_body_for_outer_uses(index);
            }
            HirExprKind::Array(elems) | HirExprKind::Tuple(elems) => {
                for e in elems {
                    self.scan_spawn_body_for_outer_uses(e);
                }
            }
            HirExprKind::Map(entries) => {
                for (k, v) in entries {
                    self.scan_spawn_body_for_outer_uses(k);
                    self.scan_spawn_body_for_outer_uses(v);
                }
            }
            HirExprKind::Struct { fields, .. } => {
                for (_, v) in fields {
                    self.scan_spawn_body_for_outer_uses(v);
                }
            }
            HirExprKind::EnumVariant { payload, .. } => {
                for p in payload {
                    self.scan_spawn_body_for_outer_uses(p);
                }
            }
            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.scan_spawn_body_for_outer_uses(condition);
                self.scan_spawn_body_for_outer_uses(then_expr);
                if let Some(e) = else_expr {
                    self.scan_spawn_body_for_outer_uses(e);
                }
            }
            HirExprKind::Block { stmts, expr } => {
                for s in stmts {
                    self.scan_spawn_stmt_for_outer_uses(s);
                }
                if let Some(e) = expr {
                    self.scan_spawn_body_for_outer_uses(e);
                }
            }
            HirExprKind::Match { values, arms } => {
                for v in values {
                    self.scan_spawn_body_for_outer_uses(v);
                }
                for arm in arms {
                    if let Some(g) = &arm.guard {
                        self.scan_spawn_body_for_outer_uses(g);
                    }
                    self.scan_spawn_body_for_outer_uses(&arm.body);
                }
            }
            HirExprKind::Range { start, end, .. } => {
                self.scan_spawn_body_for_outer_uses(start);
                self.scan_spawn_body_for_outer_uses(end);
            }
            HirExprKind::Ok(inner)
            | HirExprKind::Err(inner)
            | HirExprKind::Try(inner)
            | HirExprKind::Move(inner)
            | HirExprKind::Clone(inner)
            | HirExprKind::Borrow { expr: inner, .. }
            | HirExprKind::Await(inner)
            | HirExprKind::Spread(inner) => {
                self.scan_spawn_body_for_outer_uses(inner);
            }
            HirExprKind::UnwrapOrPanic {
                expr: inner,
                message,
            } => {
                self.scan_spawn_body_for_outer_uses(inner);
                self.scan_spawn_body_for_outer_uses(message);
            }
            HirExprKind::Cast { value, .. } => {
                self.scan_spawn_body_for_outer_uses(value);
            }
            HirExprKind::ScopeBlock { stmts } => {
                for s in stmts {
                    self.scan_spawn_stmt_for_outer_uses(s);
                }
            }
            // Don't recurse into nested spawns/closures
            HirExprKind::Spawn { .. } | HirExprKind::Closure { .. } => {}
            HirExprKind::Const(_) | HirExprKind::Global { .. } => {}
        }
    }

    /// Scan a statement inside a Spawn body for outer-scope uses.
    /// Does NOT add to needs_drop — spawn-local variables are the spawn's responsibility.
    fn scan_spawn_stmt_for_outer_uses(&mut self, stmt: &HirStmt) {
        match &stmt.kind {
            HirStmtKind::Let { value, .. } => {
                self.scan_spawn_body_for_outer_uses(value);
            }
            HirStmtKind::TupleLet { value, .. } => {
                self.scan_spawn_body_for_outer_uses(value);
            }
            HirStmtKind::Assign { target, value } => {
                self.scan_spawn_body_for_outer_uses(target);
                self.scan_spawn_body_for_outer_uses(value);
            }
            HirStmtKind::Expr(e) => {
                self.scan_spawn_body_for_outer_uses(e);
            }
            HirStmtKind::Return(exprs) => {
                for e in exprs {
                    self.scan_spawn_body_for_outer_uses(e);
                }
            }
            HirStmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.scan_spawn_body_for_outer_uses(condition);
                for s in then_block {
                    self.scan_spawn_stmt_for_outer_uses(s);
                }
                if let Some(stmts) = else_block {
                    for s in stmts {
                        self.scan_spawn_stmt_for_outer_uses(s);
                    }
                }
            }
            HirStmtKind::While {
                condition,
                body,
                increment,
            } => {
                self.scan_spawn_body_for_outer_uses(condition);
                for s in body {
                    self.scan_spawn_stmt_for_outer_uses(s);
                }
                for s in increment {
                    self.scan_spawn_stmt_for_outer_uses(s);
                }
            }
            HirStmtKind::ManualErrorExtract { expr, .. } => {
                self.scan_spawn_body_for_outer_uses(expr);
            }
            HirStmtKind::Break | HirStmtKind::Continue | HirStmtKind::Drop { .. } => {}
        }
    }

    /// Scan a closure body for outer-scope variable references.
    /// Extends `last_use` for captured variables so they aren't dropped
    /// before the closure is created (Leak 5 fix).
    /// `closure_params` are the closure's own parameters — should NOT be treated as captures.
    fn scan_closure_body_for_outer_uses(
        &mut self,
        expr: &HirExpr,
        closure_params: &FxHashSet<String>,
    ) {
        match &expr.kind {
            HirExprKind::Local { name } => {
                // Skip closure parameters — they're local to the closure
                if closure_params.contains(name) {
                    return;
                }
                // Only update if this variable is known from the outer scope
                if self.needs_drop.contains(name) || self.last_use.contains_key(name) {
                    self.last_use.insert(name.clone(), self.current_idx);
                    self.last_use_span.insert(name.clone(), expr.span);
                }
            }
            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.scan_closure_body_for_outer_uses(lhs, closure_params);
                self.scan_closure_body_for_outer_uses(rhs, closure_params);
            }
            HirExprKind::UnaryOp { operand, .. } => {
                self.scan_closure_body_for_outer_uses(operand, closure_params);
            }
            HirExprKind::Call { func, args } => {
                self.scan_closure_body_for_outer_uses(func, closure_params);
                for a in args {
                    self.scan_closure_body_for_outer_uses(a, closure_params);
                }
            }
            HirExprKind::MethodCall { receiver, args, .. } => {
                self.scan_closure_body_for_outer_uses(receiver, closure_params);
                for a in args {
                    self.scan_closure_body_for_outer_uses(a, closure_params);
                }
            }
            HirExprKind::Field { object, .. } => {
                self.scan_closure_body_for_outer_uses(object, closure_params);
            }
            HirExprKind::Index { object, index } => {
                self.scan_closure_body_for_outer_uses(object, closure_params);
                self.scan_closure_body_for_outer_uses(index, closure_params);
            }
            HirExprKind::Array(elems) | HirExprKind::Tuple(elems) => {
                for e in elems {
                    self.scan_closure_body_for_outer_uses(e, closure_params);
                }
            }
            HirExprKind::Map(entries) => {
                for (k, v) in entries {
                    self.scan_closure_body_for_outer_uses(k, closure_params);
                    self.scan_closure_body_for_outer_uses(v, closure_params);
                }
            }
            HirExprKind::Struct { fields, .. } => {
                for (_, v) in fields {
                    self.scan_closure_body_for_outer_uses(v, closure_params);
                }
            }
            HirExprKind::EnumVariant { payload, .. } => {
                for p in payload {
                    self.scan_closure_body_for_outer_uses(p, closure_params);
                }
            }
            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.scan_closure_body_for_outer_uses(condition, closure_params);
                self.scan_closure_body_for_outer_uses(then_expr, closure_params);
                if let Some(e) = else_expr {
                    self.scan_closure_body_for_outer_uses(e, closure_params);
                }
            }
            HirExprKind::Block { stmts, expr } => {
                for s in stmts {
                    self.scan_closure_stmt_for_outer_uses(s, closure_params);
                }
                if let Some(e) = expr {
                    self.scan_closure_body_for_outer_uses(e, closure_params);
                }
            }
            HirExprKind::Match { values, arms } => {
                for v in values {
                    self.scan_closure_body_for_outer_uses(v, closure_params);
                }
                for arm in arms {
                    if let Some(g) = &arm.guard {
                        self.scan_closure_body_for_outer_uses(g, closure_params);
                    }
                    self.scan_closure_body_for_outer_uses(&arm.body, closure_params);
                }
            }
            HirExprKind::Range { start, end, .. } => {
                self.scan_closure_body_for_outer_uses(start, closure_params);
                self.scan_closure_body_for_outer_uses(end, closure_params);
            }
            HirExprKind::Ok(inner)
            | HirExprKind::Err(inner)
            | HirExprKind::Try(inner)
            | HirExprKind::Move(inner)
            | HirExprKind::Clone(inner)
            | HirExprKind::Borrow { expr: inner, .. }
            | HirExprKind::Await(inner)
            | HirExprKind::Spread(inner) => {
                self.scan_closure_body_for_outer_uses(inner, closure_params);
            }
            HirExprKind::UnwrapOrPanic {
                expr: inner,
                message,
            } => {
                self.scan_closure_body_for_outer_uses(inner, closure_params);
                self.scan_closure_body_for_outer_uses(message, closure_params);
            }
            HirExprKind::Cast { value, .. } => {
                self.scan_closure_body_for_outer_uses(value, closure_params);
            }
            HirExprKind::ScopeBlock { stmts } => {
                for s in stmts {
                    self.scan_closure_stmt_for_outer_uses(s, closure_params);
                }
            }
            // Don't recurse into nested closures/spawns
            HirExprKind::Spawn { .. } | HirExprKind::Closure { .. } => {}
            HirExprKind::Const(_) | HirExprKind::Global { .. } => {}
        }
    }

    /// Scan a statement inside a closure body for outer-scope uses.
    fn scan_closure_stmt_for_outer_uses(
        &mut self,
        stmt: &HirStmt,
        closure_params: &FxHashSet<String>,
    ) {
        match &stmt.kind {
            HirStmtKind::Let { value, .. } => {
                self.scan_closure_body_for_outer_uses(value, closure_params);
            }
            HirStmtKind::TupleLet { value, .. } => {
                self.scan_closure_body_for_outer_uses(value, closure_params);
            }
            HirStmtKind::Assign { target, value } => {
                self.scan_closure_body_for_outer_uses(target, closure_params);
                self.scan_closure_body_for_outer_uses(value, closure_params);
            }
            HirStmtKind::Expr(e) => {
                self.scan_closure_body_for_outer_uses(e, closure_params);
            }
            HirStmtKind::Return(exprs) => {
                for e in exprs {
                    self.scan_closure_body_for_outer_uses(e, closure_params);
                }
            }
            HirStmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.scan_closure_body_for_outer_uses(condition, closure_params);
                for s in then_block {
                    self.scan_closure_stmt_for_outer_uses(s, closure_params);
                }
                if let Some(stmts) = else_block {
                    for s in stmts {
                        self.scan_closure_stmt_for_outer_uses(s, closure_params);
                    }
                }
            }
            HirStmtKind::While {
                condition,
                body,
                increment,
            } => {
                self.scan_closure_body_for_outer_uses(condition, closure_params);
                for s in body {
                    self.scan_closure_stmt_for_outer_uses(s, closure_params);
                }
                for s in increment {
                    self.scan_closure_stmt_for_outer_uses(s, closure_params);
                }
            }
            HirStmtKind::ManualErrorExtract { expr, .. } => {
                self.scan_closure_body_for_outer_uses(expr, closure_params);
            }
            HirStmtKind::Break | HirStmtKind::Continue | HirStmtKind::Drop { .. } => {}
        }
    }

    fn scan_match_pattern_for_uses(&mut self, p: &doo_hir::HirMatchPattern) {
        match p {
            doo_hir::HirMatchPattern::Literal(e) | doo_hir::HirMatchPattern::Condition(e) => {
                self.scan_expr_for_uses(e)
            }
            doo_hir::HirMatchPattern::Wildcard
            | doo_hir::HirMatchPattern::EnumVariant { .. }
            | doo_hir::HirMatchPattern::EnumVariantPayload { .. }
            | doo_hir::HirMatchPattern::Rest(_) => {}
            doo_hir::HirMatchPattern::Tuple(parts) | doo_hir::HirMatchPattern::Array(parts) => {
                for x in parts {
                    self.scan_match_pattern_for_uses(x);
                }
            }
            doo_hir::HirMatchPattern::Struct { fields, .. } => {
                for (_, p) in fields {
                    self.scan_match_pattern_for_uses(p);
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

    /// Insert cleanup drops before return statements in nested blocks.
    ///
    /// When a function has:
    /// ```text
    /// let arr = [1, 2, 3]
    /// if condition {
    ///     return 0   // arr not dropped here!
    /// }
    /// // drop(arr) is here, but early return skips it
    /// ```
    ///
    /// This pass scans the entire statement tree and inserts drops for all
    /// `dropped_vars` before each Return statement, skipping variables that
    /// appear in the return expression or have already been dropped on this path.
    fn insert_early_return_drops(stmts: &mut Vec<HirStmt>, dropped_vars: &FxHashSet<String>) {
        // Collect which variables have already been dropped at the top level
        // (from the Pass 2 `apply_drops`). We only need to insert extra drops
        // for returns that appear INSIDE nested blocks (if/while), because
        // top-level returns are already after their scheduled drops.
        let mut already_dropped_at_top: FxHashSet<String> = FxHashSet::default();

        let mut i = 0;
        while i < stmts.len() {
            match &stmts[i].kind {
                HirStmtKind::Drop { name } => {
                    already_dropped_at_top.insert(name.clone());
                    i += 1;
                }
                HirStmtKind::If {
                    then_block: _,
                    else_block: _,
                    ..
                } => {
                    // Variables NOT yet dropped at this point in the top-level flow
                    let still_alive: FxHashSet<String> = dropped_vars
                        .difference(&already_dropped_at_top)
                        .cloned()
                        .collect();

                    // Recurse into both branches to insert drops before returns
                    if let HirStmtKind::If {
                        then_block,
                        else_block,
                        ..
                    } = &mut stmts[i].kind
                    {
                        Self::insert_drops_before_returns_recursive(then_block, &still_alive);
                        if let Some(else_stmts) = else_block {
                            Self::insert_drops_before_returns_recursive(else_stmts, &still_alive);
                        }
                    }
                    i += 1;
                }
                HirStmtKind::While { .. } => {
                    let still_alive: FxHashSet<String> = dropped_vars
                        .difference(&already_dropped_at_top)
                        .cloned()
                        .collect();

                    if let HirStmtKind::While { body, .. } = &mut stmts[i].kind {
                        Self::insert_drops_before_returns_recursive(body, &still_alive);
                    }
                    i += 1;
                }
                _ => {
                    i += 1;
                }
            }
        }
    }

    /// Recursively scan nested blocks for Return statements and insert drops before them.
    fn insert_drops_before_returns_recursive(
        stmts: &mut Vec<HirStmt>,
        live_vars: &FxHashSet<String>,
    ) {
        let mut i = 0;
        while i < stmts.len() {
            match &stmts[i].kind {
                HirStmtKind::Return(values) => {
                    // Collect variable names used in the return expression
                    let mut returned_names = FxHashSet::default();
                    for v in values {
                        Self::collect_local_names(v, &mut returned_names);
                    }

                    // Insert drops for all live variables NOT in the return expression
                    let mut drop_count = 0;
                    for name in live_vars {
                        if !returned_names.contains(name) {
                            let drop_stmt = HirStmt::new(
                                HirStmtKind::Drop { name: name.clone() },
                                Span::dummy(),
                            );
                            stmts.insert(i + drop_count, drop_stmt);
                            drop_count += 1;
                        }
                    }
                    i += drop_count + 1; // Skip past inserted drops + the return
                }
                HirStmtKind::If {
                    then_block: _,
                    else_block: _,
                    ..
                } => {
                    // Recurse deeper
                    if let HirStmtKind::If {
                        then_block,
                        else_block,
                        ..
                    } = &mut stmts[i].kind
                    {
                        Self::insert_drops_before_returns_recursive(then_block, live_vars);
                        if let Some(else_stmts) = else_block {
                            Self::insert_drops_before_returns_recursive(else_stmts, live_vars);
                        }
                    }
                    i += 1;
                }
                HirStmtKind::While { .. } => {
                    if let HirStmtKind::While { body, .. } = &mut stmts[i].kind {
                        Self::insert_drops_before_returns_recursive(body, live_vars);
                    }
                    i += 1;
                }
                _ => {
                    i += 1;
                }
            }
        }
    }

    /// Collect all `Local` variable names referenced in an expression.
    fn collect_local_names(expr: &HirExpr, names: &mut FxHashSet<String>) {
        match &expr.kind {
            HirExprKind::Local { name } => {
                names.insert(name.clone());
            }
            HirExprKind::BinOp { lhs, rhs, .. } => {
                Self::collect_local_names(lhs, names);
                Self::collect_local_names(rhs, names);
            }
            HirExprKind::UnaryOp { operand, .. } => {
                Self::collect_local_names(operand, names);
            }
            HirExprKind::Call { func, args } => {
                Self::collect_local_names(func, names);
                for a in args {
                    Self::collect_local_names(a, names);
                }
            }
            HirExprKind::MethodCall { receiver, args, .. } => {
                Self::collect_local_names(receiver, names);
                for a in args {
                    Self::collect_local_names(a, names);
                }
            }
            HirExprKind::Field { object, .. } => {
                Self::collect_local_names(object, names);
            }
            HirExprKind::Index { object, index } => {
                Self::collect_local_names(object, names);
                Self::collect_local_names(index, names);
            }
            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::collect_local_names(condition, names);
                Self::collect_local_names(then_expr, names);
                if let Some(e) = else_expr {
                    Self::collect_local_names(e, names);
                }
            }
            HirExprKind::Ok(inner)
            | HirExprKind::Err(inner)
            | HirExprKind::Try(inner)
            | HirExprKind::Move(inner)
            | HirExprKind::Clone(inner)
            | HirExprKind::Await(inner)
            | HirExprKind::Spread(inner) => {
                Self::collect_local_names(inner, names);
            }
            HirExprKind::Borrow { expr: inner, .. } => {
                Self::collect_local_names(inner, names);
            }
            HirExprKind::Cast { value, .. } => {
                Self::collect_local_names(value, names);
            }
            HirExprKind::Array(elems) | HirExprKind::Tuple(elems) => {
                for e in elems {
                    Self::collect_local_names(e, names);
                }
            }
            HirExprKind::Struct { fields, .. } => {
                for (_, v) in fields {
                    Self::collect_local_names(v, names);
                }
            }
            _ => {}
        }
    }
}

impl<'a> Default for DropInserter<'a> {
    fn default() -> Self {
        Self::new()
    }
}

/// Drop insertion for THIR functions.
///
/// Inserts explicit `Drop { name, ty }` statements at ownership end points.
///
/// 1. Find last use of each variable (liveness analysis)
/// 2. Check if last use was a Move (ownership transferred)
/// 3. If NOT moved AND type needs Drop → insert Drop after last use
/// 4. For control flow, drop at earliest exit point on each path
/// 5. At scope end, drop all remaining owned variables in reverse declaration order
pub struct ThirDropInserter<'a> {
    registry: &'a TypeRegistry,
    /// Last use index for each variable.
    last_use: rustc_hash::FxHashMap<String, usize>,
    /// Declaration order (for reverse-order drops).
    declaration_order: Vec<(String, doo_core::types::TypeId)>,
    /// Variables that need Drop (non-Copy types).
    needs_drop_set: rustc_hash::FxHashSet<String>,
    /// Current statement index.
    current_idx: usize,
    /// Ownership results to check for Move decisions.
    ownership_results: &'a ThirOwnershipResults,
    /// Whether a type has a custom drop() implementation.
    custom_drop_types: rustc_hash::FxHashSet<String>,
}

impl<'a> ThirDropInserter<'a> {
    /// Create a new THIR drop inserter.
    pub fn new(registry: &'a TypeRegistry, ownership_results: &'a ThirOwnershipResults) -> Self {
        Self {
            registry,
            last_use: rustc_hash::FxHashMap::default(),
            declaration_order: Vec::new(),
            needs_drop_set: rustc_hash::FxHashSet::default(),
            current_idx: 0,
            ownership_results,
            custom_drop_types: rustc_hash::FxHashSet::default(),
        }
    }

    /// Insert drops into a THIR function.
    pub fn insert_drops(&mut self, func: &mut ThirFunction) {
        self.last_use.clear();
        self.declaration_order.clear();
        self.needs_drop_set.clear();
        self.current_idx = 0;

        // Pass 1: Find last uses and declaration order
        for stmt in &func.body {
            self.scan_stmt(stmt);
            self.current_idx += 1;
        }

        // Pass 2: Compute which variables need drops
        let drops = self.compute_drops();

        // Pass 3: Insert drops
        self.apply_drops(&mut func.body, drops);

        // Pass 4: Insert scope-end drops in reverse declaration order
        self.insert_scope_end_drops(&mut func.body);

        // Pass 5: Insert drops before early returns in nested blocks
        let dropped: rustc_hash::FxHashSet<String> = self.needs_drop_set.clone();
        self.insert_early_return_drops_thir(&mut func.body, &dropped);
    }

    fn scan_stmt(&mut self, stmt: &ThirStmt) {
        match &stmt.kind {
            ThirStmtKind::Let {
                name, ty, value, ..
            } => {
                if needs_drop(*ty, self.registry) {
                    self.needs_drop_set.insert(name.clone());
                }
                self.declaration_order.push((name.clone(), *ty));
                self.scan_expr(value);
            }
            ThirStmtKind::Const { name, ty, value } => {
                self.declaration_order.push((name.clone(), *ty));
                self.scan_expr(value);
            }
            ThirStmtKind::Expr(expr) => self.scan_expr(expr),
            ThirStmtKind::Assign { target, value } => {
                self.scan_expr(target);
                self.scan_expr(value);
            }
            ThirStmtKind::Return(opt_expr) => {
                if let Some(e) = opt_expr {
                    self.scan_expr(e);
                }
            }
            ThirStmtKind::While {
                cond,
                body,
                increment,
            } => {
                self.scan_expr(cond);
                for s in body {
                    self.scan_stmt(s);
                }
                for s in increment {
                    self.scan_stmt(s);
                }
            }
            ThirStmtKind::Loop { body } => {
                for s in body {
                    self.scan_stmt(s);
                }
            }
            ThirStmtKind::Go { expr } => self.scan_expr(expr),
            ThirStmtKind::Scope { stmts } => {
                for s in stmts {
                    self.scan_stmt(s);
                }
            }
            ThirStmtKind::TupleLet {
                names,
                type_ids,
                value,
                ..
            } => {
                for (n, t) in names.iter().zip(type_ids.iter()) {
                    if needs_drop(*t, self.registry) {
                        self.needs_drop_set.insert(n.clone());
                    }
                    self.declaration_order.push((n.clone(), *t));
                }
                self.scan_expr(value);
            }
            ThirStmtKind::ManualErrorExtract { expr, .. } => {
                self.scan_expr(expr);
            }
            ThirStmtKind::Break(opt_expr) => {
                if let Some(e) = opt_expr {
                    self.scan_expr(e);
                }
            }
            ThirStmtKind::Continue | ThirStmtKind::Drop { .. } => {}
        }
    }

    fn scan_expr(&mut self, expr: &ThirExpr) {
        match &expr.kind {
            ThirExprKind::Var(name) => {
                self.last_use.insert(name.clone(), self.current_idx);
            }
            ThirExprKind::Binary { lhs, rhs, .. } => {
                self.scan_expr(lhs);
                self.scan_expr(rhs);
            }
            ThirExprKind::Unary { expr, .. } => self.scan_expr(expr),
            ThirExprKind::Call { func, args } => {
                self.scan_expr(func);
                for a in args {
                    self.scan_expr(a);
                }
            }
            ThirExprKind::MethodCall { receiver, args, .. } => {
                self.scan_expr(receiver);
                for a in args {
                    self.scan_expr(a);
                }
            }
            ThirExprKind::FieldAccess { object, .. } => self.scan_expr(object),
            ThirExprKind::Index { object, index } => {
                self.scan_expr(object);
                self.scan_expr(index);
            }
            ThirExprKind::ArrayLiteral(elements) => {
                for e in elements {
                    self.scan_expr(e);
                }
            }
            ThirExprKind::MapLiteral(entries) => {
                for (k, v) in entries {
                    self.scan_expr(k);
                    self.scan_expr(v);
                }
            }
            ThirExprKind::StructLiteral { fields, .. } => {
                for (_, v) in fields {
                    self.scan_expr(v);
                }
            }
            ThirExprKind::EnumVariant { payload, .. } => {
                for p in payload {
                    self.scan_expr(p);
                }
            }
            ThirExprKind::Tuple(elements) => {
                for e in elements {
                    self.scan_expr(e);
                }
            }
            ThirExprKind::Spread(inner) => self.scan_expr(inner),
            ThirExprKind::If { cond, then, else_ } => {
                self.scan_expr(cond);
                self.scan_expr(then);
                if let Some(e) = else_ {
                    self.scan_expr(e);
                }
            }
            ThirExprKind::Match { expr, arms } => {
                self.scan_expr(expr);
                for arm in arms {
                    if let Some(g) = &arm.guard {
                        self.scan_expr(g);
                    }
                    self.scan_expr(&arm.body);
                }
            }
            ThirExprKind::Block(stmts, tail) => {
                for s in stmts {
                    self.scan_stmt(s);
                }
                if let Some(e) = tail {
                    self.scan_expr(e);
                }
            }
            ThirExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.scan_expr(s);
                }
                if let Some(e) = end {
                    self.scan_expr(e);
                }
            }
            ThirExprKind::Ok(inner)
            | ThirExprKind::Err(inner)
            | ThirExprKind::Try(inner)
            | ThirExprKind::Move(inner)
            | ThirExprKind::Clone(inner)
            | ThirExprKind::Async(inner)
            | ThirExprKind::Await(inner)
            | ThirExprKind::Spawn(inner) => {
                self.scan_expr(inner);
            }
            ThirExprKind::Borrow { expr, .. } => self.scan_expr(expr),
            ThirExprKind::Closure { body, .. } => self.scan_expr(body),
            ThirExprKind::Cast { value, .. } => self.scan_expr(value),
            ThirExprKind::UnwrapOrPanic { expr, message } => {
                self.scan_expr(expr);
                self.scan_expr(message);
            }
            ThirExprKind::ScopeBlock { stmts } => {
                for s in stmts {
                    self.scan_stmt(s);
                }
            }
            ThirExprKind::Literal(_) => {}
        }
    }

    /// Compute which drops to insert and where.
    ///
    /// Rules:
    /// - Skip variables whose last use was a Move (ownership transferred)
    /// - Skip Copy types (no Drop needed)
    /// - Insert Drop after last use for non-Copy, non-moved variables
    fn compute_drops(&self) -> Vec<(usize, String, doo_core::types::TypeId)> {
        let mut drops = Vec::new();

        for (name, ty) in &self.declaration_order {
            if !self.needs_drop_set.contains(name) {
                continue;
            }

            // Skip moved values — old owner doesn't drop (MMM Part IV §4)
            if self.ownership_results.is_moved(name) {
                continue;
            }

            if let Some(&last_idx) = self.last_use.get(name) {
                drops.push((last_idx + 1, name.clone(), *ty));
            }
        }

        // Sort by position (descending) for safe insertion
        drops.sort_by(|a, b| b.0.cmp(&a.0));
        drops
    }

    fn apply_drops(
        &self,
        stmts: &mut Vec<ThirStmt>,
        drops: Vec<(usize, String, doo_core::types::TypeId)>,
    ) {
        for (idx, name, ty) in drops {
            if idx <= stmts.len() {
                let drop_stmt = ThirStmt {
                    kind: ThirStmtKind::Drop { name, ty },
                    span: Span::dummy(),
                };
                stmts.insert(idx, drop_stmt);
            }
        }
    }

    /// Insert drops for all remaining owned variables at scope end,
    /// in REVERSE declaration order (MMM Part IV §3).
    fn insert_scope_end_drops(&self, stmts: &mut Vec<ThirStmt>) {
        let mut end_drops: Vec<(String, doo_core::types::TypeId)> = self
            .declaration_order
            .iter()
            .filter(|(name, ty)| {
                self.needs_drop_set.contains(name)
                    && !self.ownership_results.is_moved(name)
                    && !self.last_use.contains_key(name)
            })
            .map(|(n, t)| (n.clone(), *t))
            .collect();

        // Reverse declaration order: drop b, then a (MMM Part IV §3)
        end_drops.reverse();

        for (name, ty) in end_drops {
            let drop_stmt = ThirStmt {
                kind: ThirStmtKind::Drop { name, ty },
                span: Span::dummy(),
            };
            stmts.push(drop_stmt);
        }
    }

    /// Insert cleanup drops before Return statements in nested blocks.
    ///
    /// When a function has an early return inside a nested block (if/while),
    /// all owned locals must be dropped before the return executes.
    fn insert_early_return_drops_thir(
        &self,
        stmts: &mut Vec<ThirStmt>,
        live_vars: &rustc_hash::FxHashSet<String>,
    ) {
        let mut i = 0;
        while i < stmts.len() {
            match &stmts[i].kind {
                ThirStmtKind::Return(values) => {
                    // Collect variable names used in the return expression
                    let mut returned = rustc_hash::FxHashSet::default();
                    for v in values {
                        Self::collect_var_names_thir(v, &mut returned);
                    }

                    // Insert drops for all live variables not in the return
                    let mut drop_count = 0;
                    for (name, ty) in self.declaration_order.iter().rev() {
                        if live_vars.contains(name)
                            && !returned.contains(name)
                            && self.needs_drop_set.contains(name)
                            && !self.ownership_results.is_moved(name)
                        {
                            let drop_stmt = ThirStmt {
                                kind: ThirStmtKind::Drop {
                                    name: name.clone(),
                                    ty: *ty,
                                },
                                span: Span::dummy(),
                            };
                            stmts.insert(i + drop_count, drop_stmt);
                            drop_count += 1;
                        }
                    }
                    i += drop_count + 1;
                }
                ThirStmtKind::While { .. } => {
                    if let ThirStmtKind::While { body, .. } = &mut stmts[i].kind {
                        self.insert_early_return_drops_thir(body, live_vars);
                    }
                    i += 1;
                }
                ThirStmtKind::Scope { .. } => {
                    if let ThirStmtKind::Scope { stmts: inner } = &mut stmts[i].kind {
                        self.insert_early_return_drops_thir(inner, live_vars);
                    }
                    i += 1;
                }
                _ => {
                    i += 1;
                }
            }
        }
    }

    /// Collect all variable names referenced in a THIR expression.
    fn collect_var_names_thir(expr: &ThirExpr, names: &mut rustc_hash::FxHashSet<String>) {
        match &expr.kind {
            ThirExprKind::Var(name) => {
                names.insert(name.clone());
            }
            ThirExprKind::Binary { lhs, rhs, .. } => {
                Self::collect_var_names_thir(lhs, names);
                Self::collect_var_names_thir(rhs, names);
            }
            ThirExprKind::Unary { expr, .. } => {
                Self::collect_var_names_thir(expr, names);
            }
            ThirExprKind::Call { func, args } => {
                Self::collect_var_names_thir(func, names);
                for a in args {
                    Self::collect_var_names_thir(a, names);
                }
            }
            ThirExprKind::MethodCall { receiver, args, .. } => {
                Self::collect_var_names_thir(receiver, names);
                for a in args {
                    Self::collect_var_names_thir(a, names);
                }
            }
            ThirExprKind::FieldAccess { object, .. } => {
                Self::collect_var_names_thir(object, names);
            }
            ThirExprKind::Index { object, index } => {
                Self::collect_var_names_thir(object, names);
                Self::collect_var_names_thir(index, names);
            }
            ThirExprKind::If { cond, then, else_ } => {
                Self::collect_var_names_thir(cond, names);
                Self::collect_var_names_thir(then, names);
                if let Some(e) = else_ {
                    Self::collect_var_names_thir(e, names);
                }
            }
            ThirExprKind::Match { expr, arms } => {
                Self::collect_var_names_thir(expr, names);
                for arm in arms {
                    if let Some(g) = &arm.guard {
                        Self::collect_var_names_thir(g, names);
                    }
                    Self::collect_var_names_thir(&arm.body, names);
                }
            }
            ThirExprKind::Block(stmts, tail) => {
                for _s in stmts {
                    // Blocks can contain Let statements that introduce new vars
                }
                if let Some(e) = tail {
                    Self::collect_var_names_thir(e, names);
                }
            }
            ThirExprKind::Ok(inner)
            | ThirExprKind::Err(inner)
            | ThirExprKind::Try(inner)
            | ThirExprKind::Move(inner)
            | ThirExprKind::Clone(inner)
            | ThirExprKind::Async(inner)
            | ThirExprKind::Await(inner)
            | ThirExprKind::Spawn(inner)
            | ThirExprKind::Spread(inner) => {
                Self::collect_var_names_thir(inner, names);
            }
            ThirExprKind::Borrow { expr, .. } => {
                Self::collect_var_names_thir(expr, names);
            }
            ThirExprKind::Cast { value, .. } => {
                Self::collect_var_names_thir(value, names);
            }
            ThirExprKind::ArrayLiteral(elements) => {
                for e in elements {
                    Self::collect_var_names_thir(e, names);
                }
            }
            ThirExprKind::StructLiteral { fields, .. } => {
                for (_, v) in fields {
                    Self::collect_var_names_thir(v, names);
                }
            }
            ThirExprKind::Tuple(elements) => {
                for e in elements {
                    Self::collect_var_names_thir(e, names);
                }
            }
            ThirExprKind::UnwrapOrPanic { expr, message } => {
                Self::collect_var_names_thir(expr, names);
                Self::collect_var_names_thir(message, names);
            }
            _ => {}
        }
    }

    /// Register a type as having a custom drop() implementation.
    ///
    /// Custom drop is called automatically, exactly once, and cannot be
    /// called manually (MMM Part IV §11).
    pub fn register_custom_drop(&mut self, type_name: &str) {
        self.custom_drop_types.insert(type_name.to_string());
    }

    /// Verify that no explicit drop calls exist in the code.
    ///
    /// Custom drop() is called automatically by the compiler — manual
    /// calls are an error (MMM Part IV §11).
    pub fn verify_no_explicit_drops(&self, stmts: &[ThirStmt]) -> Vec<String> {
        let mut errors = Vec::new();
        for stmt in stmts {
            self.check_explicit_drop_stmt(stmt, &mut errors);
        }
        errors
    }

    fn check_explicit_drop_stmt(&self, stmt: &ThirStmt, errors: &mut Vec<String>) {
        match &stmt.kind {
            ThirStmtKind::Expr(expr) => {
                self.check_explicit_drop_expr(expr, errors);
            }
            ThirStmtKind::While { body, .. } => {
                for s in body {
                    self.check_explicit_drop_stmt(s, errors);
                }
            }
            ThirStmtKind::Scope { stmts } => {
                for s in stmts {
                    self.check_explicit_drop_stmt(s, errors);
                }
            }
            _ => {}
        }
    }

    fn check_explicit_drop_expr(&self, expr: &ThirExpr, errors: &mut Vec<String>) {
        match &expr.kind {
            ThirExprKind::Call { func, .. } => {
                if let ThirExprKind::Var(name) = &func.kind {
                    if name == "drop" || name == "free" {
                        errors.push(format!(
                            "explicit drop call is not allowed — drop is automatic"
                        ));
                    }
                }
            }
            ThirExprKind::MethodCall { method, .. } => {
                if method == "drop" || method == "free" {
                    errors.push(format!(
                        "explicit drop call is not allowed — drop is automatic"
                    ));
                }
            }
            _ => {}
        }
    }

    /// Check if dropping an owner should recursively drop everything it owns.
    ///
    /// Heap objects: dropping owner drops everything it owns (recursive).
    /// - struct User { name: Str } → dropping User drops name
    /// - Collections: drop all elements
    /// - Box: drop inner value, then free heap memory
    pub fn needs_recursive_drop(&self, ty: doo_core::types::TypeId) -> bool {
        if let Some(info) = self.registry.get(ty) {
            match &info.kind {
                TypeKind::Struct { def } => {
                    // Struct: check if any field needs Drop
                    def.fields
                        .iter()
                        .any(|f| needs_drop(f.type_id, self.registry))
                }
                TypeKind::Enum { def } => {
                    // Enum: check if any variant payload needs Drop
                    def.variants
                        .iter()
                        .any(|v| v.payload.map_or(false, |p| needs_drop(p, self.registry)))
                }
                TypeKind::Array { .. } | TypeKind::Map { .. } | TypeKind::Set { .. } => {
                    // Collections: elements may need Drop
                    true
                }
                TypeKind::Box { .. } => {
                    // Box: inner value needs Drop, then free heap
                    true
                }
                _ => false,
            }
        } else {
            false
        }
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

#[cfg(test)]
mod thir_drop_tests {
    use super::*;

    #[test]
    fn test_thir_drop_inserter_creation() {
        let registry = TypeRegistry::new();
        let results = ThirOwnershipResults::new();
        let inserter = ThirDropInserter::new(&registry, &results);
        assert!(inserter.last_use.is_empty());
        assert!(inserter.needs_drop_set.is_empty());
    }

    #[test]
    fn test_needs_recursive_drop_for_struct() {
        let mut registry = TypeRegistry::new();
        let str_id = registry.lookup("Str").unwrap();
        let _int_id = registry.lookup("Int").unwrap();

        // A struct with a Str field should need recursive drop
        let def = doo_core::types::composite::StructDef {
            name: doo_core::symbol::Symbol::intern("User"),
            fields: vec![doo_core::types::composite::FieldDef {
                name: doo_core::symbol::Symbol::intern("name"),
                type_id: str_id,
                is_public: true,
                is_optional: false,
                default_value: None,
                decorators: vec![],
            }],
            is_public: true,
            decorators: vec![],
        };
        let struct_id = registry.define_struct(def);

        let results = ThirOwnershipResults::new();
        let inserter = ThirDropInserter::new(&registry, &results);
        assert!(inserter.needs_recursive_drop(struct_id));
    }

    #[test]
    fn test_no_recursive_drop_for_primitive() {
        let registry = TypeRegistry::new();
        let int_id = registry.lookup("Int").unwrap();

        let results = ThirOwnershipResults::new();
        let inserter = ThirDropInserter::new(&registry, &results);
        assert!(!inserter.needs_recursive_drop(int_id));
    }
}
