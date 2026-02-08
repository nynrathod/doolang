//! Ownership Analysis
//!
//! Tracks ownership of every variable and decides when to move, copy, or clone.
//!
//! ## Doo's Ownership Model
//!
//! Doo uses automatic ownership management:
//! - Variables own their data by default
//! - When a variable is used multiple times, the compiler auto-clones
//! - Only error: concurrent mutable access (detected in borrow checking)
//!
//! ## Analysis Passes
//!
//! 1. **Use Counting**: Count how many times each variable is used after each point
//! 2. **Decision**: For each use, decide Move (last use), Copy (primitives), or Clone

pub mod drop_insertion;

pub use drop_insertion::DropInserter;

use doo_core::{
    types::{builtin, TypeId},
    Span,
};
use doo_hir::{
    HirExpr, HirExprKind, HirFunction, HirItem, HirProgram, HirStmt, HirStmtKind, Ownership,
};
use rustc_hash::FxHashMap;

/// Key for identifying a specific variable use location.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UseLocation {
    /// Variable name
    pub name: String,
    /// Span of the use site
    pub span: Span,
}

impl UseLocation {
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Self {
            name: name.into(),
            span,
        }
    }
}

/// Results from ownership analysis.
/// Maps each variable use to its ownership decision.
#[derive(Debug, Clone, Default)]
pub struct OwnershipResults {
    /// Decision for each variable use, keyed by (variable_name, use_span)
    decisions: FxHashMap<UseLocation, Decision>,
}

impl OwnershipResults {
    /// Create empty ownership results.
    pub fn new() -> Self {
        Self {
            decisions: FxHashMap::default(),
        }
    }

    /// Record a decision for a variable use.
    pub fn record(&mut self, name: impl Into<String>, span: Span, decision: Decision) {
        let loc = UseLocation::new(name, span);
        self.decisions.insert(loc, decision);
    }

    /// Get the decision for a variable use at a specific span.
    pub fn get_decision(&self, name: &str, span: Span) -> Option<Decision> {
        let loc = UseLocation {
            name: name.to_string(),
            span,
        };
        self.decisions.get(&loc).copied()
    }

    /// Get the decision for a variable, searching by name only (returns first match).
    /// This is useful when you don't have the exact span.
    pub fn get_decision_by_name(&self, name: &str) -> Option<Decision> {
        for (loc, decision) in &self.decisions {
            if loc.name == name {
                return Some(*decision);
            }
        }
        None
    }

    /// Get all decisions.
    pub fn all_decisions(&self) -> &FxHashMap<UseLocation, Decision> {
        &self.decisions
    }

    /// Check if results are empty.
    pub fn is_empty(&self) -> bool {
        self.decisions.is_empty()
    }

    /// Number of recorded decisions.
    pub fn len(&self) -> usize {
        self.decisions.len()
    }
}

/// Ownership decision for a variable use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Move the value (zero-cost, last use).
    Move,
    /// Copy the value (bitwise copy for primitives).
    Copy,
    /// Clone the value (deep copy for non-primitives).
    Clone,
    /// Borrow the value (for function arguments).
    Borrow { mutable: bool },
}

/// Ownership analysis error.
#[derive(Debug, Clone)]
pub struct OwnershipError {
    pub message: String,
    pub span: Span,
}

impl OwnershipError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

/// Variable usage information.
#[derive(Debug, Clone, Default)]
struct VarInfo {
    /// All locations where this variable is used.
    uses: Vec<Span>,
    /// Type of the variable (for copy/clone decision).
    type_id: Option<TypeId>,
    /// Is this variable mutable?
    mutable: bool,
    /// Current ownership state.
    ownership: Ownership,
}

/// Ownership analyzer.
///
/// Analyzes HIR to track variable ownership and insert clone operations.
pub struct OwnershipAnalyzer {
    /// Variable information.
    vars: FxHashMap<String, VarInfo>,
    /// Current statement index (for use counting).
    current_idx: usize,
    /// Collected errors.
    errors: Vec<OwnershipError>,
    /// Collected ownership decisions for each variable use.
    results: OwnershipResults,
}

impl OwnershipAnalyzer {
    /// Create a new ownership analyzer.
    pub fn new() -> Self {
        Self {
            vars: FxHashMap::default(),
            current_idx: 0,
            errors: Vec::new(),
            results: OwnershipResults::new(),
        }
    }

    /// Analyze a program for ownership.
    pub fn analyze(
        &mut self,
        program: &HirProgram,
    ) -> Result<OwnershipResults, Vec<OwnershipError>> {
        for item in &program.items {
            if let HirItem::Function(func) = item {
                self.analyze_function(func);
            }
        }

        if self.errors.is_empty() {
            Ok(self.results.clone())
        } else {
            Err(self.errors.clone())
        }
    }

    /// Get the collected ownership results (call after analyze).
    pub fn results(&self) -> &OwnershipResults {
        &self.results
    }

    /// Take ownership of the results.
    pub fn take_results(&mut self) -> OwnershipResults {
        std::mem::take(&mut self.results)
    }

    /// Analyze a function.
    fn analyze_function(&mut self, func: &HirFunction) {
        // Clear state for new function
        self.vars.clear();
        self.current_idx = 0;

        // Register parameters
        for param in &func.params {
            self.vars.insert(
                param.name.clone(),
                VarInfo {
                    uses: Vec::new(),
                    type_id: param.type_id,
                    mutable: false, // Params immutable by default
                    ownership: Ownership::Owned,
                },
            );
        }

        // Pass 1: Count uses
        for stmt in &func.body {
            self.count_uses_in_stmt(stmt);
            self.current_idx += 1;
        }

        // Pass 2: Analyze ownership
        self.current_idx = 0;
        for stmt in &func.body {
            self.analyze_stmt(stmt);
            self.current_idx += 1;
        }
    }

    // ========================================================================
    // Pass 1: Use Counting
    // ========================================================================

    fn count_uses_in_stmt(&mut self, stmt: &HirStmt) {
        match &stmt.kind {
            HirStmtKind::Let {
                name,
                value,
                mutable,
                type_id,
                ..
            } => {
                // Register new variable
                self.vars.insert(
                    name.clone(),
                    VarInfo {
                        uses: Vec::new(),
                        type_id: *type_id,
                        mutable: *mutable,
                        ownership: Ownership::Owned,
                    },
                );
                self.count_uses_in_expr(value);
            }
            HirStmtKind::TupleLet {
                names,
                type_ids,
                value,
                mutable,
            } => {
                // Register each variable from tuple unpacking
                for (i, name) in names.iter().enumerate() {
                    let type_id = type_ids.get(i).and_then(|t| *t);
                    self.vars.insert(
                        name.clone(),
                        VarInfo {
                            uses: Vec::new(),
                            type_id,
                            mutable: *mutable,
                            ownership: Ownership::Owned,
                        },
                    );
                }
                self.count_uses_in_expr(value);
            }
            HirStmtKind::ManualErrorExtract {
                ok_names,
                error_name,
                expr,
            } => {
                for name in ok_names {
                    if name != "_" {
                        self.vars.insert(
                            name.clone(),
                            VarInfo {
                                uses: Vec::new(),
                                type_id: None,
                                mutable: false,
                                ownership: Ownership::Owned,
                            },
                        );
                    }
                }
                if error_name != "_" {
                    self.vars.insert(
                        error_name.clone(),
                        VarInfo {
                            uses: Vec::new(),
                            type_id: None,
                            mutable: false,
                            ownership: Ownership::Owned,
                        },
                    );
                }
                self.count_uses_in_expr(expr);
            }
            HirStmtKind::Assign { target, value } => {
                self.count_uses_in_expr(target);
                self.count_uses_in_expr(value);
            }
            HirStmtKind::Expr(expr) => {
                self.count_uses_in_expr(expr);
            }
            HirStmtKind::Return(values) => {
                for v in values {
                    self.count_uses_in_expr(v);
                }
            }
            HirStmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.count_uses_in_expr(condition);
                for s in then_block {
                    self.count_uses_in_stmt(s);
                }
                if let Some(else_stmts) = else_block {
                    for s in else_stmts {
                        self.count_uses_in_stmt(s);
                    }
                }
            }
            HirStmtKind::While { condition, body } => {
                self.count_uses_in_expr(condition);
                for s in body {
                    self.count_uses_in_stmt(s);
                }
            }
            HirStmtKind::Break | HirStmtKind::Continue | HirStmtKind::Drop { .. } => {}
        }
    }

    fn count_uses_in_expr(&mut self, expr: &HirExpr) {
        match &expr.kind {
            HirExprKind::Local { name } => {
                if let Some(info) = self.vars.get_mut(name) {
                    info.uses.push(expr.span);
                }
            }
            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.count_uses_in_expr(lhs);
                self.count_uses_in_expr(rhs);
            }
            HirExprKind::UnaryOp { operand, .. } => {
                self.count_uses_in_expr(operand);
            }
            HirExprKind::Call { func, args } => {
                self.count_uses_in_expr(func);
                for arg in args {
                    self.count_uses_in_expr(arg);
                }
            }
            HirExprKind::MethodCall { receiver, args, .. } => {
                self.count_uses_in_expr(receiver);
                for arg in args {
                    self.count_uses_in_expr(arg);
                }
            }
            HirExprKind::Field { object, .. } => {
                self.count_uses_in_expr(object);
            }
            HirExprKind::Index { object, index } => {
                self.count_uses_in_expr(object);
                self.count_uses_in_expr(index);
            }
            HirExprKind::Array(elements) | HirExprKind::Tuple(elements) => {
                for elem in elements {
                    self.count_uses_in_expr(elem);
                }
            }
            HirExprKind::Map(entries) => {
                for (k, v) in entries {
                    self.count_uses_in_expr(k);
                    self.count_uses_in_expr(v);
                }
            }
            HirExprKind::Struct { fields, .. } => {
                for (_, value) in fields {
                    self.count_uses_in_expr(value);
                }
            }
            HirExprKind::EnumVariant { payload, .. } => {
                for e in payload {
                    self.count_uses_in_expr(e);
                }
            }
            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.count_uses_in_expr(condition);
                self.count_uses_in_expr(then_expr);
                if let Some(e) = else_expr {
                    self.count_uses_in_expr(e);
                }
            }
            HirExprKind::Match { values, arms } => {
                for v in values {
                    self.count_uses_in_expr(v);
                }
                for arm in arms {
                    self.count_uses_in_match_pattern(&arm.pattern);
                    if let Some(g) = &arm.guard {
                        self.count_uses_in_expr(g);
                    }
                    self.count_uses_in_expr(&arm.body);
                }
            }
            HirExprKind::Block { stmts, expr } => {
                for s in stmts {
                    self.count_uses_in_stmt(s);
                }
                if let Some(e) = expr {
                    self.count_uses_in_expr(e);
                }
            }
            HirExprKind::Range { start, end, .. } => {
                self.count_uses_in_expr(start);
                self.count_uses_in_expr(end);
            }
            HirExprKind::Ok(inner)
            | HirExprKind::Err(inner)
            | HirExprKind::Try(inner)
            | HirExprKind::Move(inner)
            | HirExprKind::Clone(inner) => {
                self.count_uses_in_expr(inner);
            }
            HirExprKind::UnwrapOrPanic {
                expr: inner,
                message,
            } => {
                self.count_uses_in_expr(inner);
                self.count_uses_in_expr(message);
            }
            HirExprKind::Borrow { expr: inner, .. } => {
                self.count_uses_in_expr(inner);
            }
            HirExprKind::Closure { .. } => {
                // Do NOT recurse into closure bodies for usage counting.
                // Closures are separate functions; their params/locals are not
                // part of the outer function's scope.
            }
            HirExprKind::Spread(inner) => {
                self.count_uses_in_expr(inner);
            }
            HirExprKind::RouteBlock { routes } => {
                for route in routes {
                    self.count_uses_in_expr(route);
                }
            }
            HirExprKind::Cast { value, .. } => {
                self.count_uses_in_expr(value);
            }
            HirExprKind::Const(_) | HirExprKind::Global { .. } => {}
        }
    }

    fn count_uses_in_match_pattern(&mut self, p: &doo_hir::HirMatchPattern) {
        match p {
            doo_hir::HirMatchPattern::Literal(e) | doo_hir::HirMatchPattern::Condition(e) => {
                self.count_uses_in_expr(e)
            }
            doo_hir::HirMatchPattern::Wildcard
            | doo_hir::HirMatchPattern::EnumVariant { .. }
            | doo_hir::HirMatchPattern::EnumVariantPayload { .. } => {}
            doo_hir::HirMatchPattern::Tuple(parts) => {
                for x in parts {
                    self.count_uses_in_match_pattern(x);
                }
            }
        }
    }

    // ========================================================================
    // Pass 2: Ownership Analysis
    // ========================================================================

    fn analyze_stmt(&mut self, stmt: &HirStmt) {
        match &stmt.kind {
            HirStmtKind::Let { value, .. } => {
                self.analyze_expr(value);
            }
            HirStmtKind::TupleLet { value, .. } => {
                self.analyze_expr(value);
            }
            HirStmtKind::ManualErrorExtract { expr, .. } => {
                self.analyze_expr(expr);
            }
            HirStmtKind::Assign { target, value } => {
                // Check target is mutable
                if let HirExprKind::Local { name } = &target.kind {
                    if let Some(info) = self.vars.get(name) {
                        if !info.mutable {
                            self.errors.push(OwnershipError::new(
                                format!("Cannot assign to immutable variable '{}'", name),
                                stmt.span,
                            ));
                        }
                    }
                }
                self.analyze_expr(value);
            }
            HirStmtKind::Expr(expr) => {
                self.analyze_expr(expr);
            }
            HirStmtKind::Return(values) => {
                for v in values {
                    self.analyze_expr(v);
                }
            }
            HirStmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.analyze_expr(condition);
                for s in then_block {
                    self.analyze_stmt(s);
                }
                if let Some(else_stmts) = else_block {
                    for s in else_stmts {
                        self.analyze_stmt(s);
                    }
                }
            }
            HirStmtKind::While { condition, body } => {
                self.analyze_expr(condition);
                for s in body {
                    self.analyze_stmt(s);
                }
            }
            HirStmtKind::Break | HirStmtKind::Continue | HirStmtKind::Drop { .. } => {}
        }
    }

    fn analyze_expr(&mut self, expr: &HirExpr) {
        match &expr.kind {
            HirExprKind::Local { name } => {
                // Decide what to do with this use
                let _decision = self.decide_use(name, expr.span);
                // In a full implementation, we'd annotate the HIR node
            }
            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.analyze_expr(lhs);
                self.analyze_expr(rhs);
            }
            HirExprKind::UnaryOp { operand, .. } => {
                self.analyze_expr(operand);
            }
            HirExprKind::Call { func, args } => {
                self.analyze_expr(func);
                for arg in args {
                    self.analyze_expr(arg);
                }
            }
            HirExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                // For method calls on locals, use Borrow instead of Clone
                // This ensures mutations from methods like Add, push, etc. affect the original
                // Note: We can't statically determine if a user-defined method mutates self,
                // so we conservatively use mutable borrow for ALL method calls on locals
                // This is safe because:
                // - Mutating methods: changes persist to the original (correct behavior)
                // - Non-mutating methods: just reads the value (still correct, no clone overhead)
                if let HirExprKind::Local { name } = &receiver.kind {
                    // All method calls on locals use mutable borrow to allow mutation
                    self.results
                        .record(name, receiver.span, Decision::Borrow { mutable: true });
                } else {
                    // For non-local receivers (fields, temporaries), analyze normally
                    self.analyze_expr(receiver);
                }
                // Analyze arguments normally
                for arg in args {
                    self.analyze_expr(arg);
                }
            }
            HirExprKind::Field { object, .. } => {
                self.analyze_expr(object);
            }
            HirExprKind::Index { object, index } => {
                self.analyze_expr(object);
                self.analyze_expr(index);
            }
            HirExprKind::Array(elements) | HirExprKind::Tuple(elements) => {
                for elem in elements {
                    self.analyze_expr(elem);
                }
            }
            HirExprKind::Map(entries) => {
                for (k, v) in entries {
                    self.analyze_expr(k);
                    self.analyze_expr(v);
                }
            }
            HirExprKind::Struct { fields, .. } => {
                for (_, value) in fields {
                    self.analyze_expr(value);
                }
            }
            HirExprKind::EnumVariant { payload, .. } => {
                for e in payload {
                    self.analyze_expr(e);
                }
            }
            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.analyze_expr(condition);
                self.analyze_expr(then_expr);
                if let Some(e) = else_expr {
                    self.analyze_expr(e);
                }
            }
            HirExprKind::Match { values, arms } => {
                for v in values {
                    self.analyze_expr(v);
                }
                for arm in arms {
                    self.analyze_match_pattern(&arm.pattern);
                    if let Some(g) = &arm.guard {
                        self.analyze_expr(g);
                    }
                    self.analyze_expr(&arm.body);
                }
            }
            HirExprKind::Block { stmts, expr } => {
                for s in stmts {
                    self.analyze_stmt(s);
                }
                if let Some(e) = expr {
                    self.analyze_expr(e);
                }
            }
            HirExprKind::Range { start, end, .. } => {
                self.analyze_expr(start);
                self.analyze_expr(end);
            }
            HirExprKind::Ok(inner)
            | HirExprKind::Err(inner)
            | HirExprKind::Try(inner)
            | HirExprKind::Move(inner)
            | HirExprKind::Clone(inner) => {
                self.analyze_expr(inner);
            }
            HirExprKind::UnwrapOrPanic {
                expr: inner,
                message,
            } => {
                self.analyze_expr(inner);
                self.analyze_expr(message);
            }
            HirExprKind::Borrow { expr: inner, .. } => {
                self.analyze_expr(inner);
            }
            HirExprKind::Closure { .. } => {
                // Do NOT recurse into closure bodies for ownership analysis.
                // Closures are built as separate MIR functions with their own
                // param/local scope. Analyzing them here would record ownership
                // decisions for closure params as if they were outer-scope variables.
            }
            HirExprKind::Spread(inner) => {
                self.analyze_expr(inner);
            }
            HirExprKind::RouteBlock { routes } => {
                for route in routes {
                    self.analyze_expr(route);
                }
            }
            HirExprKind::Cast { value, .. } => {
                self.analyze_expr(value);
            }
            HirExprKind::Const(_) | HirExprKind::Global { .. } => {}
        }
    }

    fn analyze_match_pattern(&mut self, p: &doo_hir::HirMatchPattern) {
        match p {
            doo_hir::HirMatchPattern::Literal(e) | doo_hir::HirMatchPattern::Condition(e) => {
                self.analyze_expr(e)
            }
            doo_hir::HirMatchPattern::Wildcard
            | doo_hir::HirMatchPattern::EnumVariant { .. }
            | doo_hir::HirMatchPattern::EnumVariantPayload { .. } => {}
            doo_hir::HirMatchPattern::Tuple(parts) => {
                for x in parts {
                    self.analyze_match_pattern(x);
                }
            }
        }
    }

    // ========================================================================
    // Decision Logic
    // ========================================================================

    /// Decide what to do when a variable is used.
    fn decide_use(&mut self, name: &str, use_span: Span) -> Decision {
        let info = match self.vars.get(name) {
            Some(info) => info.clone(),
            None => {
                let decision = Decision::Move; // Unknown var, assume move
                self.results.record(name, use_span, decision);
                return decision;
            }
        };

        // Count uses after this point
        let future_uses = info
            .uses
            .iter()
            .filter(|s| s.start > use_span.start)
            .count();

        let decision = if future_uses == 0 {
            // Last use - move is zero-cost
            Decision::Move
        } else if self.is_copy_type(info.type_id) {
            // Primitive - bitwise copy
            Decision::Copy
        } else {
            // Non-primitive with future uses - auto-clone
            Decision::Clone
        };

        // Record the decision for later use by MIR builder
        self.results.record(name, use_span, decision);

        decision
    }

    /// Check if a method is a known mutating method for arrays or maps.
    /// This is a fast check that doesn't require TypeRegistry access.
    fn is_mutating_method(&self, method: &str) -> bool {
        // Check against known mutating methods from doo_core::methods
        // Array mutating: push, pop, sort, reverse, clear
        // Map mutating: remove, clear
        matches!(
            method,
            "push" | "pop" | "sort" | "reverse" | "clear" | "remove"
        )
    }

    /// Check if a type is Copy (primitives).
    fn is_copy_type(&self, type_id: Option<TypeId>) -> bool {
        match type_id {
            Some(id) => {
                // Primitives are Copy - compare against builtin type IDs
                id == builtin::INT
                    || id == builtin::FLOAT
                    || id == builtin::BOOL
                    || id == builtin::VOID
            }
            None => false, // Unknown type, assume not Copy
        }
    }

    /// Get collected errors.
    pub fn errors(&self) -> &[OwnershipError] {
        &self.errors
    }
}

impl Default for OwnershipAnalyzer {
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
    fn test_decision_move_last_use() {
        let analyzer = OwnershipAnalyzer::new();
        // Creating a simple test - in a full implementation we'd parse HIR
        assert!(analyzer.is_copy_type(Some(builtin::INT)));
        assert!(!analyzer.is_copy_type(Some(builtin::STR)));
    }

    #[test]
    fn test_analyzer_creation() {
        let analyzer = OwnershipAnalyzer::new();
        assert!(analyzer.errors.is_empty());
    }

    #[test]
    fn test_copy_types() {
        let analyzer = OwnershipAnalyzer::new();
        assert!(analyzer.is_copy_type(Some(builtin::INT)));
        assert!(analyzer.is_copy_type(Some(builtin::FLOAT)));
        assert!(analyzer.is_copy_type(Some(builtin::BOOL)));
        assert!(!analyzer.is_copy_type(Some(builtin::STR)));
        assert!(!analyzer.is_copy_type(None));
    }
}
