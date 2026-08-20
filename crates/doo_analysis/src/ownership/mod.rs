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
    types::{builtin, TypeId, TypeKind, TypeRegistry},
    Span,
};
use doo_hir::{
    HirExpr, HirExprKind, HirFunction, HirItem, HirProgram, HirStmt, HirStmtKind, Ownership,
};
use doo_thir::{ThirExpr, ThirExprKind, ThirFunction, ThirStmt, ThirStmtKind};
use rustc_hash::{FxHashMap, FxHashSet};

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
    /// Drop the value (owner becomes unreachable).
    Drop,
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
                    // Auto-ownership: `self` is always mutable in methods
                    mutable: param.name == "self",
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
            HirStmtKind::While {
                condition,
                body,
                increment,
            } => {
                self.count_uses_in_expr(condition);
                for s in body {
                    self.count_uses_in_stmt(s);
                }
                for s in increment {
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
            HirExprKind::Cast { value, .. } => {
                self.count_uses_in_expr(value);
            }

            // Async & concurrency
            HirExprKind::Await(inner) | HirExprKind::Spawn { body: inner } => {
                self.count_uses_in_expr(inner);
            }
            HirExprKind::ScopeBlock { stmts } => {
                for s in stmts {
                    self.count_uses_in_stmt(s);
                }
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
            | doo_hir::HirMatchPattern::EnumVariantPayload { .. }
            | doo_hir::HirMatchPattern::Rest(_) => {}
            doo_hir::HirMatchPattern::Tuple(parts) | doo_hir::HirMatchPattern::Array(parts) => {
                for x in parts {
                    self.count_uses_in_match_pattern(x);
                }
            }
            doo_hir::HirMatchPattern::Struct { fields, .. } => {
                for (_, p) in fields {
                    self.count_uses_in_match_pattern(p);
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
            HirStmtKind::While {
                condition,
                body,
                increment,
            } => {
                self.analyze_expr(condition);
                for s in body {
                    self.analyze_stmt(s);
                }
                for s in increment {
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
                method: _,
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
                // Field access is a read, not a move — borrow the object
                if let HirExprKind::Local { name } = &object.kind {
                    self.results
                        .record(name, object.span, Decision::Borrow { mutable: false });
                } else {
                    self.analyze_expr(object);
                }
            }
            HirExprKind::Index { object, index } => {
                // Index access is a read, not a move — borrow the object
                if let HirExprKind::Local { name } = &object.kind {
                    self.results
                        .record(name, object.span, Decision::Borrow { mutable: false });
                } else {
                    self.analyze_expr(object);
                }
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
            HirExprKind::Cast { value, .. } => {
                self.analyze_expr(value);
            }

            // Async & concurrency
            HirExprKind::Await(inner) | HirExprKind::Spawn { body: inner } => {
                self.analyze_expr(inner);
            }
            HirExprKind::ScopeBlock { stmts } => {
                for s in stmts {
                    self.analyze_stmt(s);
                }
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
            | doo_hir::HirMatchPattern::EnumVariantPayload { .. }
            | doo_hir::HirMatchPattern::Rest(_) => {}
            doo_hir::HirMatchPattern::Tuple(parts) | doo_hir::HirMatchPattern::Array(parts) => {
                for x in parts {
                    self.analyze_match_pattern(x);
                }
            }
            doo_hir::HirMatchPattern::Struct { fields, .. } => {
                for (_, p) in fields {
                    self.analyze_match_pattern(p);
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
// Ownership Decision Algorithm
// ============================================================================

/// The four ownership decisions evaluated in exact priority order.
///
/// Algorithm (per value use):
/// 1. Is it Copy? → Copy
/// 2. Does it need ownership? No → Borrow
/// 3. Else → Move
/// 4. When owner becomes unreachable → Drop
///
/// Clone is never inferred — it is always explicit via `.clone()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OwnershipDecision {
    /// Bitwise copy (primitives and Copy types only).
    Copy,
    /// Temporary read access without ownership transfer.
    Borrow,
    /// Transfer ownership to a new owner.
    Move,
    /// Owner becomes unreachable — free resources.
    Drop,
}

impl OwnershipDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Borrow => "borrow",
            Self::Move => "move",
            Self::Drop => "drop",
        }
    }
}

/// Check if a type implements Copy (bitwise copyable).
///
/// Rules
/// - Primitives (Int, Float, Bool, Char, all sizes) → Copy
/// - Struct/Enum: Copy ONLY if ALL fields are Copy
/// - Str, Array, Map, Set, Box, Closure → NOT Copy
/// - Tuple: Copy if all elements Copy
/// - Optional: Copy if inner is Copy
/// - Result: Copy if both ok and err are Copy
///
/// Uses a visited set to handle recursive types (which require Box).
pub fn is_copy(ty: TypeId, registry: &TypeRegistry) -> bool {
    let mut visited = FxHashSet::default();
    is_copy_recursive(ty, registry, &mut visited)
}

fn is_copy_recursive(ty: TypeId, registry: &TypeRegistry, visited: &mut FxHashSet<TypeId>) -> bool {
    if visited.contains(&ty) {
        return false;
    }

    let info = match registry.get(ty) {
        Some(i) => i,
        None => return false,
    };

    match &info.kind {
        // Primitives are always Copy
        TypeKind::Bool
        | TypeKind::Char
        | TypeKind::Int8
        | TypeKind::Int16
        | TypeKind::Int32
        | TypeKind::Int64
        | TypeKind::Int
        | TypeKind::UInt8
        | TypeKind::UInt16
        | TypeKind::UInt32
        | TypeKind::UInt64
        | TypeKind::UInt
        | TypeKind::Float32
        | TypeKind::Float64 => true,

        // Heap-backed types are never Copy
        TypeKind::Str => false,
        TypeKind::Array { .. } => false,
        TypeKind::Map { .. } => false,
        TypeKind::Set { .. } => false,
        TypeKind::Box { .. } => false,
        TypeKind::Function { .. } => false,
        TypeKind::Interface { .. } => false,

        // Void and Never have no data
        TypeKind::Void | TypeKind::Never => false,

        // Optional: Copy if inner is Copy
        TypeKind::Optional { inner } => is_copy_recursive(*inner, registry, visited),

        // Result: Copy if both ok and err are Copy
        TypeKind::Result { ok, err } => {
            is_copy_recursive(*ok, registry, visited) && is_copy_recursive(*err, registry, visited)
        }

        // Tuple: Copy if all elements are Copy
        TypeKind::Tuple { elements } => elements
            .iter()
            .all(|e| is_copy_recursive(*e, registry, visited)),

        // Struct: Copy ONLY if ALL fields are Copy (MMM Part II §4)
        TypeKind::Struct { def } => {
            visited.insert(ty);
            let result = def
                .fields
                .iter()
                .all(|f| is_copy_recursive(f.type_id, registry, visited));
            visited.remove(&ty);
            result
        }

        // Enum: Copy if ALL variant payloads are Copy
        TypeKind::Enum { def } => {
            visited.insert(ty);
            let result = def.variants.iter().all(|v| match v.payload {
                Some(payload_ty) => is_copy_recursive(payload_ty, registry, visited),
                None => true,
            });
            visited.remove(&ty);
            result
        }

        // Type parameters and unresolved types are not Copy
        TypeKind::TypeRef { .. }
        | TypeKind::TypeParam { .. }
        | TypeKind::SelfType
        | TypeKind::Any
        | TypeKind::Error => false,
    }
}

/// Check if a type needs Drop cleanup (MMM Part IV).
///
/// A type needs Drop if it is not Copy — it owns resources that must be freed.
pub fn needs_drop(ty: TypeId, registry: &TypeRegistry) -> bool {
    !is_copy(ty, registry)
}

// ============================================================================
// Signature Stability
// ============================================================================

/// Tracks locked parameter conventions for functions.
///
/// The first compile of a function locks its parameter convention per parameter.
/// Later body edits requiring a different convention produce an error at the
/// function's own declaration.
///
/// Stored in the query system as: `fn_convention_of(func, param_idx) -> OwnershipDecision`
#[derive(Debug, Clone, Default)]
pub struct SignatureStability {
    /// Maps function name → locked conventions per parameter index.
    locked: FxHashMap<String, Vec<OwnershipDecision>>,
}

impl SignatureStability {
    pub fn new() -> Self {
        Self {
            locked: FxHashMap::default(),
        }
    }

    /// Lock conventions for a function. Called on first compile.
    /// If already locked, validates that conventions match.
    pub fn lock_or_validate(
        &mut self,
        func_name: &str,
        conventions: &[OwnershipDecision],
    ) -> Result<(), SignatureDriftError> {
        match self.locked.get(func_name) {
            Some(existing) => {
                for (i, (existing_conv, new_conv)) in
                    existing.iter().zip(conventions.iter()).enumerate()
                {
                    if existing_conv != new_conv {
                        return Err(SignatureDriftError {
                            func_name: func_name.to_string(),
                            param_idx: i,
                            was: *existing_conv,
                            now: *new_conv,
                        });
                    }
                }
                Ok(())
            }
            None => {
                self.locked
                    .insert(func_name.to_string(), conventions.to_vec());
                Ok(())
            }
        }
    }

    /// Get the locked convention for a specific parameter.
    pub fn convention_of(&self, func_name: &str, param_idx: usize) -> Option<OwnershipDecision> {
        self.locked.get(func_name)?.get(param_idx).copied()
    }

    /// Get all locked conventions for a function.
    pub fn conventions_for(&self, func_name: &str) -> Option<&[OwnershipDecision]> {
        self.locked.get(func_name).map(|v| v.as_slice())
    }

    /// Check if a function's conventions are locked.
    pub fn is_locked(&self, func_name: &str) -> bool {
        self.locked.contains_key(func_name)
    }
}

/// Error when a function's body edit requires a different parameter convention
/// than what was locked on first compile (Decision 6.1).
#[derive(Debug, Clone)]
pub struct SignatureDriftError {
    pub func_name: String,
    pub param_idx: usize,
    pub was: OwnershipDecision,
    pub now: OwnershipDecision,
}

impl std::fmt::Display for SignatureDriftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "signature drift in '{}': parameter {} was locked as {}, now requires {}",
            self.func_name,
            self.param_idx,
            self.was.as_str(),
            self.now.as_str()
        )
    }
}

// ============================================================================
// Closure Capture Inference
// ============================================================================

/// Capture mode for a variable captured by a closure.
///
/// A closure is treated as an implicit struct whose fields are its captures.
/// The standard Copy → Borrow → Move algorithm runs per capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    /// Captured by bitwise copy (Copy types only).
    Copy,
    /// Captured by borrow (read access, still usable after).
    Borrow,
    /// Captured by move (consumed, invalid after).
    Move,
}

/// Information about a single closure capture.
#[derive(Debug, Clone)]
pub struct CaptureInfo {
    /// Name of the captured variable.
    pub name: String,
    /// Type of the captured variable.
    pub ty: TypeId,
    /// How the variable was captured.
    pub mode: CaptureMode,
    /// Whether the closure escapes its defining scope.
    pub escapes: bool,
}

/// Analyzes closure captures using the Copy → Borrow → Move algorithm.
///
/// A closure is an implicit struct with capture fields. For each captured
/// variable, the algorithm determines:
/// - Copy: if the type is Copy and the variable is used after the closure
/// - Borrow: if the variable is read but not consumed
/// - Move: if the variable is consumed (stored, returned, passed to consuming fn)
///
/// If the closure escapes (returned, stored, moved to heap, passed to go/async),
/// it becomes heap-allocated
pub struct ClosureCaptureAnalyzer {
    /// Captures for each closure, keyed by closure span start.
    captures: FxHashMap<u32, Vec<CaptureInfo>>,
}

impl ClosureCaptureAnalyzer {
    pub fn new() -> Self {
        Self {
            captures: FxHashMap::default(),
        }
    }

    /// Record a capture for a closure.
    pub fn record_capture(&mut self, closure_span_start: u32, info: CaptureInfo) {
        self.captures
            .entry(closure_span_start)
            .or_default()
            .push(info);
    }

    /// Get all captures for a closure.
    pub fn captures_for(&self, closure_span_start: u32) -> Option<&[CaptureInfo]> {
        self.captures.get(&closure_span_start).map(|v| v.as_slice())
    }

    /// Determine capture mode for a variable based on type and usage.
    ///
    /// Algorithm:
    /// 1. Is the type Copy? → Copy
    /// 2. Is the variable consumed (moved/stored/returned)? → Move
    /// 3. Otherwise → Borrow
    pub fn determine_mode(
        is_copy_type: bool,
        is_consumed: bool,
        _is_mutable_access: bool,
    ) -> CaptureMode {
        if is_copy_type {
            CaptureMode::Copy
        } else if is_consumed {
            CaptureMode::Move
        } else {
            CaptureMode::Borrow
        }
    }

    /// Check if a closure escapes its defining scope.
    ///
    /// A closure escapes if it is:
    /// - Returned from the function
    /// - Stored in a heap container (Array, Map, Box)
    /// - Passed to a `go` block (MMM Part III §11)
    /// - Passed to an async block
    /// - Assigned to a variable that outlives the closure's scope
    pub fn closure_escapes(
        is_returned: bool,
        is_stored_in_heap: bool,
        is_passed_to_go: bool,
        is_passed_to_async: bool,
    ) -> bool {
        is_returned || is_stored_in_heap || is_passed_to_go || is_passed_to_async
    }

    /// Get all analyzed closures.
    pub fn all_captures(&self) -> &FxHashMap<u32, Vec<CaptureInfo>> {
        &self.captures
    }
}

impl Default for ClosureCaptureAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// THIR-Based Ownership Analysis
// ============================================================================

/// Ownership analysis results for THIR.
///
/// Maps each variable use (identified by name + span) to its ownership decision.
/// Also tracks which variables were moved and which need Drop.
#[derive(Debug, Clone, Default)]
pub struct ThirOwnershipResults {
    /// Decision for each variable use.
    decisions: FxHashMap<UseLocation, OwnershipDecision>,
    /// Variables that have been moved (and their move span).
    moved: FxHashMap<String, Span>,
    /// Variables that need Drop (non-Copy types).
    needs_drop_set: FxHashSet<String>,
    /// Variable types for is_copy checking.
    var_types: FxHashMap<String, TypeId>,
}

impl ThirOwnershipResults {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, name: &str, span: Span, decision: OwnershipDecision) {
        self.decisions
            .insert(UseLocation::new(name, span), decision);
        if decision == OwnershipDecision::Move {
            self.moved.insert(name.to_string(), span);
        }
    }

    pub fn get_decision(&self, name: &str, span: Span) -> Option<OwnershipDecision> {
        self.decisions.get(&UseLocation::new(name, span)).copied()
    }

    pub fn is_moved(&self, name: &str) -> bool {
        self.moved.contains_key(name)
    }

    pub fn move_span(&self, name: &str) -> Option<Span> {
        self.moved.get(name).copied()
    }

    pub fn needs_drop(&self, name: &str) -> bool {
        self.needs_drop_set.contains(name)
    }

    pub fn set_var_type(&mut self, name: &str, ty: TypeId) {
        self.var_types.insert(name.to_string(), ty);
        if needs_drop(ty, &TypeRegistry::new()) {
            self.needs_drop_set.insert(name.to_string());
        }
    }

    pub fn var_type(&self, name: &str) -> Option<TypeId> {
        self.var_types.get(name).copied()
    }

    pub fn all_decisions(&self) -> &FxHashMap<UseLocation, OwnershipDecision> {
        &self.decisions
    }

    pub fn moved_vars(&self) -> impl Iterator<Item = (&String, &Span)> {
        self.moved.iter()
    }
}

/// THIR-based ownership analyzer.
///
/// Runs the Copy → Borrow → Move → Drop algorithm over THIR.
/// This is the spec-compliant analyzer that operates on fully typed IR.
pub struct ThirOwnershipAnalyzer<'a> {
    registry: &'a TypeRegistry,
    results: ThirOwnershipResults,
    /// Use counts per variable (for determining last use → Move).
    use_counts: FxHashMap<String, Vec<Span>>,
    /// Current use index per variable.
    use_index: FxHashMap<String, usize>,
    errors: Vec<OwnershipError>,
}

impl<'a> ThirOwnershipAnalyzer<'a> {
    pub fn new(registry: &'a TypeRegistry) -> Self {
        Self {
            registry,
            results: ThirOwnershipResults::new(),
            use_counts: FxHashMap::default(),
            use_index: FxHashMap::default(),
            errors: Vec::new(),
        }
    }

    /// Analyze a THIR function body for ownership.
    pub fn analyze_function(&mut self, func: &ThirFunction) -> ThirOwnershipResults {
        self.use_counts.clear();
        self.use_index.clear();

        // Register parameters
        for param in &func.params {
            if let Some(ty) = param.ty {
                self.results.set_var_type(&param.name, ty);
            }
        }

        // Pass 1: Count all uses
        for stmt in &func.body {
            self.count_uses_stmt(stmt);
        }

        // Pass 2: Analyze ownership decisions
        self.use_index.clear();
        for stmt in &func.body {
            self.analyze_stmt(stmt);
        }

        std::mem::take(&mut self.results)
    }

    fn count_uses_stmt(&mut self, stmt: &ThirStmt) {
        match &stmt.kind {
            ThirStmtKind::Let {
                name, ty, value, ..
            } => {
                self.results.set_var_type(name, *ty);
                self.count_uses_expr(value);
            }
            ThirStmtKind::Const { name, ty, value } => {
                self.results.set_var_type(name, *ty);
                self.count_uses_expr(value);
            }
            ThirStmtKind::Assign { target, value } => {
                self.count_uses_expr(target);
                self.count_uses_expr(value);
            }
            ThirStmtKind::Expr(expr) => self.count_uses_expr(expr),
            ThirStmtKind::Return(opt_expr) => {
                if let Some(e) = opt_expr {
                    self.count_uses_expr(e);
                }
            }
            ThirStmtKind::While {
                cond,
                body,
                increment,
            } => {
                self.count_uses_expr(cond);
                for s in body {
                    self.count_uses_stmt(s);
                }
                for s in increment {
                    self.count_uses_stmt(s);
                }
            }
            ThirStmtKind::Loop { body } => {
                for s in body {
                    self.count_uses_stmt(s);
                }
            }
            ThirStmtKind::Go { expr } => self.count_uses_expr(expr),
            ThirStmtKind::Scope { stmts } => {
                for s in stmts {
                    self.count_uses_stmt(s);
                }
            }
            ThirStmtKind::TupleLet {
                names,
                type_ids,
                value,
                ..
            } => {
                for (n, t) in names.iter().zip(type_ids.iter()) {
                    self.results.set_var_type(n, *t);
                }
                self.count_uses_expr(value);
            }
            ThirStmtKind::ManualErrorExtract { expr, .. } => {
                self.count_uses_expr(expr);
            }
            ThirStmtKind::Break(opt_expr) => {
                if let Some(e) = opt_expr {
                    self.count_uses_expr(e);
                }
            }
            ThirStmtKind::Continue | ThirStmtKind::Drop { .. } => {}
        }
    }

    fn count_uses_expr(&mut self, expr: &ThirExpr) {
        match &expr.kind {
            ThirExprKind::Var(name) => {
                self.use_counts
                    .entry(name.clone())
                    .or_default()
                    .push(expr.span);
            }
            ThirExprKind::Binary { lhs, rhs, .. } => {
                self.count_uses_expr(lhs);
                self.count_uses_expr(rhs);
            }
            ThirExprKind::Unary { expr, .. } => self.count_uses_expr(expr),
            ThirExprKind::Call { func, args } => {
                self.count_uses_expr(func);
                for a in args {
                    self.count_uses_expr(a);
                }
            }
            ThirExprKind::MethodCall { receiver, args, .. } => {
                self.count_uses_expr(receiver);
                for a in args {
                    self.count_uses_expr(a);
                }
            }
            ThirExprKind::FieldAccess { object, .. } => self.count_uses_expr(object),
            ThirExprKind::Index { object, index } => {
                self.count_uses_expr(object);
                self.count_uses_expr(index);
            }
            ThirExprKind::ArrayLiteral(elements) => {
                for e in elements {
                    self.count_uses_expr(e);
                }
            }
            ThirExprKind::MapLiteral(entries) => {
                for (k, v) in entries {
                    self.count_uses_expr(k);
                    self.count_uses_expr(v);
                }
            }
            ThirExprKind::StructLiteral { fields, .. } => {
                for (_, v) in fields {
                    self.count_uses_expr(v);
                }
            }
            ThirExprKind::EnumVariant { payload, .. } => {
                for p in payload {
                    self.count_uses_expr(p);
                }
            }
            ThirExprKind::Tuple(elements) => {
                for e in elements {
                    self.count_uses_expr(e);
                }
            }
            ThirExprKind::Spread(inner) => self.count_uses_expr(inner),
            ThirExprKind::If { cond, then, else_ } => {
                self.count_uses_expr(cond);
                self.count_uses_expr(then);
                if let Some(e) = else_ {
                    self.count_uses_expr(e);
                }
            }
            ThirExprKind::Match { expr, arms } => {
                self.count_uses_expr(expr);
                for arm in arms {
                    if let Some(g) = &arm.guard {
                        self.count_uses_expr(g);
                    }
                    self.count_uses_expr(&arm.body);
                }
            }
            ThirExprKind::Block(stmts, tail) => {
                for s in stmts {
                    self.count_uses_stmt(s);
                }
                if let Some(e) = tail {
                    self.count_uses_expr(e);
                }
            }
            ThirExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.count_uses_expr(s);
                }
                if let Some(e) = end {
                    self.count_uses_expr(e);
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
                self.count_uses_expr(inner);
            }
            ThirExprKind::Borrow { expr, .. } => self.count_uses_expr(expr),
            ThirExprKind::Closure { body, .. } => {
                self.count_uses_expr(body);
            }
            ThirExprKind::Cast { value, .. } => self.count_uses_expr(value),
            ThirExprKind::UnwrapOrPanic { expr, message } => {
                self.count_uses_expr(expr);
                self.count_uses_expr(message);
            }
            ThirExprKind::ScopeBlock { stmts } => {
                for s in stmts {
                    self.count_uses_stmt(s);
                }
            }
            ThirExprKind::Literal(_) => {}
        }
    }

    fn analyze_stmt(&mut self, stmt: &ThirStmt) {
        match &stmt.kind {
            ThirStmtKind::Let { value, .. } | ThirStmtKind::Const { value, .. } => {
                self.analyze_expr(value);
            }
            ThirStmtKind::Assign { target, value } => {
                self.analyze_expr(target);
                self.analyze_expr(value);
            }
            ThirStmtKind::Expr(expr) => self.analyze_expr(expr),
            ThirStmtKind::Return(opt_expr) => {
                if let Some(e) = opt_expr {
                    self.analyze_expr(e);
                }
            }
            ThirStmtKind::While {
                cond,
                body,
                increment,
            } => {
                self.analyze_expr(cond);
                for s in body {
                    self.analyze_stmt(s);
                }
                for s in increment {
                    self.analyze_stmt(s);
                }
            }
            ThirStmtKind::Loop { body } => {
                for s in body {
                    self.analyze_stmt(s);
                }
            }
            ThirStmtKind::Go { expr } => {
                self.analyze_expr(expr);
            }
            ThirStmtKind::Scope { stmts } => {
                for s in stmts {
                    self.analyze_stmt(s);
                }
            }
            ThirStmtKind::TupleLet { value, .. } => {
                self.analyze_expr(value);
            }
            ThirStmtKind::ManualErrorExtract { expr, .. } => {
                self.analyze_expr(expr);
            }
            ThirStmtKind::Break(opt_expr) => {
                if let Some(e) = opt_expr {
                    self.analyze_expr(e);
                }
            }
            ThirStmtKind::Continue | ThirStmtKind::Drop { .. } => {}
        }
    }

    fn analyze_expr(&mut self, expr: &ThirExpr) {
        match &expr.kind {
            ThirExprKind::Var(name) => {
                let decision = self.decide_use(name, expr.span);
                self.results.record(name, expr.span, decision);
            }
            ThirExprKind::Binary { lhs, rhs, .. } => {
                self.analyze_expr(lhs);
                self.analyze_expr(rhs);
            }
            ThirExprKind::Unary { expr: inner, .. } => self.analyze_expr(inner),
            ThirExprKind::Call { func, args } => {
                self.analyze_expr(func);
                for a in args {
                    self.analyze_expr(a);
                }
            }
            ThirExprKind::MethodCall { receiver, args, .. } => {
                // Method receiver: borrow by default (Decision 6.2)
                if let ThirExprKind::Var(name) = &receiver.kind {
                    self.results
                        .record(name, receiver.span, OwnershipDecision::Borrow);
                } else {
                    self.analyze_expr(receiver);
                }
                for a in args {
                    self.analyze_expr(a);
                }
            }
            ThirExprKind::FieldAccess { object, .. } => {
                // Field access is a read — borrow the object
                if let ThirExprKind::Var(name) = &object.kind {
                    self.results
                        .record(name, object.span, OwnershipDecision::Borrow);
                } else {
                    self.analyze_expr(object);
                }
            }
            ThirExprKind::Index { object, index } => {
                if let ThirExprKind::Var(name) = &object.kind {
                    self.results
                        .record(name, object.span, OwnershipDecision::Borrow);
                } else {
                    self.analyze_expr(object);
                }
                self.analyze_expr(index);
            }
            ThirExprKind::ArrayLiteral(elements) => {
                for e in elements {
                    self.analyze_expr(e);
                }
            }
            ThirExprKind::MapLiteral(entries) => {
                for (k, v) in entries {
                    self.analyze_expr(k);
                    self.analyze_expr(v);
                }
            }
            ThirExprKind::StructLiteral { fields, .. } => {
                for (_, v) in fields {
                    self.analyze_expr(v);
                }
            }
            ThirExprKind::EnumVariant { payload, .. } => {
                for p in payload {
                    self.analyze_expr(p);
                }
            }
            ThirExprKind::Tuple(elements) => {
                for e in elements {
                    self.analyze_expr(e);
                }
            }
            ThirExprKind::Spread(inner) => self.analyze_expr(inner),
            ThirExprKind::If { cond, then, else_ } => {
                self.analyze_expr(cond);
                self.analyze_expr(then);
                if let Some(e) = else_ {
                    self.analyze_expr(e);
                }
            }
            ThirExprKind::Match { expr, arms } => {
                self.analyze_expr(expr);
                for arm in arms {
                    if let Some(g) = &arm.guard {
                        self.analyze_expr(g);
                    }
                    self.analyze_expr(&arm.body);
                }
            }
            ThirExprKind::Block(stmts, tail) => {
                for s in stmts {
                    self.analyze_stmt(s);
                }
                if let Some(e) = tail {
                    self.analyze_expr(e);
                }
            }
            ThirExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.analyze_expr(s);
                }
                if let Some(e) = end {
                    self.analyze_expr(e);
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
                self.analyze_expr(inner);
            }
            ThirExprKind::Borrow { expr: inner, .. } => self.analyze_expr(inner),
            ThirExprKind::Closure { body, .. } => {
                // Closure body analyzed separately
                self.analyze_expr(body);
            }
            ThirExprKind::Cast { value, .. } => self.analyze_expr(value),
            ThirExprKind::UnwrapOrPanic { expr, message } => {
                self.analyze_expr(expr);
                self.analyze_expr(message);
            }
            ThirExprKind::ScopeBlock { stmts } => {
                for s in stmts {
                    self.analyze_stmt(s);
                }
            }
            ThirExprKind::Literal(_) => {}
        }
    }

    /// Core decision:
    /// 1. Is it Copy? → Copy
    /// 2. Does it need ownership? No → Borrow
    /// 3. Else → Move
    fn decide_use(&mut self, name: &str, span: Span) -> OwnershipDecision {
        let uses = self.use_counts.get(name).cloned().unwrap_or_default();
        let idx = self.use_index.entry(name.to_string()).or_insert(0);
        *idx += 1;

        let is_last_use = *idx >= uses.len();

        // Check if already moved
        if self.results.is_moved(name) {
            self.errors.push(OwnershipError::new(
                format!("use of moved value '{}'", name),
                span,
            ));
            return OwnershipDecision::Move;
        }

        // Step 1: Is it Copy?
        let ty = self.results.var_type(name);
        let copy_type = ty.map_or(false, |t| is_copy(t, self.registry));

        if copy_type {
            return OwnershipDecision::Copy;
        }

        // Step 2: Does it need ownership? (consumed/stored/returned/passed to consuming fn)
        // For simplicity: last use → Move (ownership required), earlier use → Borrow
        if is_last_use {
            OwnershipDecision::Move
        } else {
            OwnershipDecision::Borrow
        }
    }

    pub fn errors(&self) -> &[OwnershipError] {
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
