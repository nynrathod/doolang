//! AST to HIR Lowering
//!
//! Converts AST nodes to HIR with desugaring of complex constructs.
//!
//! ## Desugaring
//!
//! - `x += 1` → `x = x + 1`
//! - `x++` / `x--` → `x = x + 1` / `x = x - 1`
//! - `for x in iter` → while loop (basic structure)
//! - Range expressions → Range construction

mod expr;
mod for_loops;
mod helpers;
mod items;
mod route;
mod stmt;
mod type_infer;

use doo_core::{
    infer::{BinOpKind, UnaryOpKind},
    types::{TypeId, TypeRegistry},
    Span,
};
use doo_frontend::ast::{Item, Program};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::types::*;

/// AST to HIR lowering context.
pub struct Lower {
    /// Collected errors during lowering.
    errors: Vec<LowerError>,
    /// Variable type tracking for typed lowering (name -> TypeId)
    var_types: FxHashMap<String, TypeId>,
    /// Counter for generating unique internal variable names
    unique_counter: u64,
    /// Track JSON stringify sources: variable name -> type of the stringified value
    /// Used to infer JSON.parse return type when parsing a variable
    json_stringify_sources: FxHashMap<String, TypeId>,
    /// Items hoisted from inside function bodies (local struct/enum declarations).
    hoisted_items: Vec<HirItem>,
    /// Known standalone function names in the program (for disambiguating Namespace::Func from EnumVariant).
    /// Only contains functions WITHOUT an associated type (i.e., not methods like Server.get).
    known_functions: FxHashSet<String>,
    /// Known qualified methods: maps (TypeName, MethodName) pairs for associated functions.
    /// e.g., Server.get -> ("Server", "get"), Database.Postgres -> ("Database", "Postgres")
    known_qualified_methods: FxHashMap<String, FxHashSet<String>>,
}

/// Lowering error.
#[derive(Debug, Clone)]
pub struct LowerError {
    pub message: String,
    pub span: Span,
}

impl LowerError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

impl Lower {
    /// Create a new lowering context.
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            var_types: FxHashMap::default(),
            unique_counter: 0,
            json_stringify_sources: FxHashMap::default(),
            hoisted_items: Vec::new(),
            known_functions: FxHashSet::default(),
            known_qualified_methods: FxHashMap::default(),
        }
    }

    /// Generate a unique suffix for internal variable names.
    fn unique_suffix(&mut self) -> u64 {
        let id = self.unique_counter;
        self.unique_counter += 1;
        id
    }

    /// Recursively substitute a local variable name in a statement.
    fn substitute_local_in_stmt(&self, stmt: &mut HirStmt, old_name: &str, new_name: &str) {
        self.substitute_local_in_stmt_kind(&mut stmt.kind, old_name, new_name);
    }

    fn substitute_local_in_stmt_kind(
        &self,
        kind: &mut HirStmtKind,
        old_name: &str,
        new_name: &str,
    ) {
        match kind {
            HirStmtKind::Let { value, .. } => {
                self.substitute_local_in_expr(value, old_name, new_name);
            }
            HirStmtKind::TupleLet { value, .. } => {
                self.substitute_local_in_expr(value, old_name, new_name);
            }
            HirStmtKind::Expr(expr) => {
                self.substitute_local_in_expr(expr, old_name, new_name);
            }
            HirStmtKind::Assign { target, value, .. } => {
                self.substitute_local_in_expr(target, old_name, new_name);
                self.substitute_local_in_expr(value, old_name, new_name);
            }
            HirStmtKind::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                self.substitute_local_in_expr(condition, old_name, new_name);
                for s in then_block {
                    self.substitute_local_in_stmt(s, old_name, new_name);
                }
                if let Some(else_stmts) = else_block {
                    for s in else_stmts {
                        self.substitute_local_in_stmt(s, old_name, new_name);
                    }
                }
            }
            HirStmtKind::While {
                condition,
                body,
                increment,
            } => {
                self.substitute_local_in_expr(condition, old_name, new_name);
                for s in body {
                    self.substitute_local_in_stmt(s, old_name, new_name);
                }
                for s in increment {
                    self.substitute_local_in_stmt(s, old_name, new_name);
                }
            }
            HirStmtKind::Return(exprs) => {
                for expr in exprs {
                    self.substitute_local_in_expr(expr, old_name, new_name);
                }
            }
            HirStmtKind::ManualErrorExtract { expr, .. } => {
                self.substitute_local_in_expr(expr, old_name, new_name);
            }
            HirStmtKind::Break | HirStmtKind::Continue | HirStmtKind::Drop { .. } => {}
        }
    }

    fn substitute_local_in_expr(&self, expr: &mut HirExpr, old_name: &str, new_name: &str) {
        match &mut expr.kind {
            HirExprKind::Local { name, .. } => {
                if name == old_name {
                    *name = new_name.to_string();
                }
            }
            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.substitute_local_in_expr(lhs, old_name, new_name);
                self.substitute_local_in_expr(rhs, old_name, new_name);
            }
            HirExprKind::UnaryOp { operand, .. } => {
                self.substitute_local_in_expr(operand, old_name, new_name);
            }
            HirExprKind::Call { func, args, .. } => {
                self.substitute_local_in_expr(func, old_name, new_name);
                for arg in args {
                    self.substitute_local_in_expr(arg, old_name, new_name);
                }
            }
            HirExprKind::MethodCall { receiver, args, .. } => {
                self.substitute_local_in_expr(receiver, old_name, new_name);
                for arg in args {
                    self.substitute_local_in_expr(arg, old_name, new_name);
                }
            }
            HirExprKind::Field { object, .. } => {
                self.substitute_local_in_expr(object, old_name, new_name);
            }
            HirExprKind::Index { object, index, .. } => {
                self.substitute_local_in_expr(object, old_name, new_name);
                self.substitute_local_in_expr(index, old_name, new_name);
            }
            HirExprKind::Array(elements) => {
                for el in elements {
                    self.substitute_local_in_expr(el, old_name, new_name);
                }
            }
            HirExprKind::Map(entries) => {
                for (k, v) in entries {
                    self.substitute_local_in_expr(k, old_name, new_name);
                    self.substitute_local_in_expr(v, old_name, new_name);
                }
            }
            HirExprKind::Tuple(elements) => {
                for el in elements {
                    self.substitute_local_in_expr(el, old_name, new_name);
                }
            }
            HirExprKind::Struct { fields, .. } => {
                for (_, val) in fields {
                    self.substitute_local_in_expr(val, old_name, new_name);
                }
            }
            HirExprKind::EnumVariant { payload, .. } => {
                for p in payload {
                    self.substitute_local_in_expr(p, old_name, new_name);
                }
            }
            HirExprKind::Spread(inner) => {
                self.substitute_local_in_expr(inner, old_name, new_name);
            }
            HirExprKind::RouteBlock { routes } => {
                for route in routes {
                    self.substitute_local_in_expr(route, old_name, new_name);
                }
            }
            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                self.substitute_local_in_expr(condition, old_name, new_name);
                self.substitute_local_in_expr(then_expr, old_name, new_name);
                if let Some(el) = else_expr {
                    self.substitute_local_in_expr(el, old_name, new_name);
                }
            }
            HirExprKind::Block { stmts, expr, .. } => {
                for s in stmts {
                    self.substitute_local_in_stmt(s, old_name, new_name);
                }
                if let Some(e) = expr {
                    self.substitute_local_in_expr(e, old_name, new_name);
                }
            }
            HirExprKind::Match { values, arms, .. } => {
                for v in values {
                    self.substitute_local_in_expr(v, old_name, new_name);
                }
                for arm in arms {
                    if let Some(guard) = &mut arm.guard {
                        self.substitute_local_in_expr(guard, old_name, new_name);
                    }
                    self.substitute_local_in_expr(&mut arm.body, old_name, new_name);
                }
            }
            HirExprKind::Range { start, end, .. } => {
                self.substitute_local_in_expr(start, old_name, new_name);
                self.substitute_local_in_expr(end, old_name, new_name);
            }
            HirExprKind::Closure { body, .. } => {
                self.substitute_local_in_expr(body, old_name, new_name);
            }
            HirExprKind::Ok(inner)
            | HirExprKind::Err(inner)
            | HirExprKind::Try(inner)
            | HirExprKind::Move(inner)
            | HirExprKind::Clone(inner) => {
                self.substitute_local_in_expr(inner, old_name, new_name);
            }
            HirExprKind::UnwrapOrPanic {
                expr: inner,
                message,
            } => {
                self.substitute_local_in_expr(inner, old_name, new_name);
                self.substitute_local_in_expr(message, old_name, new_name);
            }
            HirExprKind::Borrow { expr: inner, .. } => {
                self.substitute_local_in_expr(inner, old_name, new_name);
            }
            HirExprKind::Cast { value, .. } => {
                self.substitute_local_in_expr(value, old_name, new_name);
            }
            HirExprKind::Await(inner) | HirExprKind::Spawn { body: inner } => {
                self.substitute_local_in_expr(inner, old_name, new_name);
            }
            HirExprKind::ScopeBlock { stmts } => {
                for s in stmts {
                    self.substitute_local_in_stmt(s, old_name, new_name);
                }
            }
            // Literals and constants don't have local references
            HirExprKind::Const(_) | HirExprKind::Global { .. } => {}
        }
    }

    /// Lower an entire program.
    pub fn lower_program(&mut self, program: &Program) -> HirProgram {
        // Pre-collect function names for namespace-qualified call disambiguation
        for item in &program.items {
            if let Item::Function(f) = item {
                if let Some(ref assoc_type) = f.associated_type {
                    // Associated method: track under its type namespace
                    self.known_qualified_methods
                        .entry(assoc_type.clone())
                        .or_default()
                        .insert(f.name.clone());
                } else {
                    // Standalone function only
                    self.known_functions.insert(f.name.clone());
                }
            }
        }

        let mut items: Vec<HirItem> = program
            .items
            .iter()
            .filter_map(|item| self.lower_item(item))
            .collect();

        // Append any struct/enum declarations hoisted from inside function bodies
        items.append(&mut self.hoisted_items);

        HirProgram {
            items,
            span: program.span,
        }
    }

    pub fn lower_program_typed(
        &mut self,
        program: &Program,
        registry: &mut TypeRegistry,
    ) -> HirProgram {
        // Pre-collect all function names so we can disambiguate
        // Namespace::Func(args) from EnumVariant during expression lowering.
        for item in &program.items {
            if let Item::Function(f) = item {
                if let Some(ref assoc_type) = f.associated_type {
                    // Associated method: track under its type namespace
                    self.known_qualified_methods
                        .entry(assoc_type.clone())
                        .or_default()
                        .insert(f.name.clone());
                } else {
                    // Standalone function only
                    self.known_functions.insert(f.name.clone());
                }
            }
        }

        for item in &program.items {
            match item {
                Item::Struct(s) => {
                    registry.declare_named(&s.name);
                }
                Item::Enum(e) => {
                    registry.declare_named(&e.name);
                }
                _ => {}
            }
        }

        let mut items: Vec<HirItem> = program
            .items
            .iter()
            .filter_map(|item| self.lower_item_typed(item, registry))
            .collect();

        // Register hoisted items (local struct/enum) in type registry
        for hoisted in &self.hoisted_items {
            match hoisted {
                HirItem::Struct(s) => {
                    registry.declare_named(&s.name);
                }
                HirItem::Enum(e) => {
                    registry.declare_named(&e.name);
                }
                _ => {}
            }
        }

        // Append hoisted items from inside function bodies
        items.append(&mut self.hoisted_items);

        HirProgram {
            items,
            span: program.span,
        }
    }

    /// Get collected errors.
    pub fn errors(&self) -> &[LowerError] {
        &self.errors
    }

    /// Check if lowering had errors.
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

impl Default for Lower {
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
    use doo_frontend::Parser;

    fn parse_and_lower(source: &str) -> HirProgram {
        let mut parser = Parser::new(source, 0);
        let program = parser.parse_program().unwrap();
        let mut lower = Lower::new();
        lower.lower_program(&program)
    }

    #[test]
    fn test_lower_int_literal() {
        let hir = parse_and_lower("let x = 42");
        assert_eq!(hir.items.len(), 0); // Top-level statements not supported in HIR items
    }

    #[test]
    fn test_lower_function() {
        let hir = parse_and_lower("fn add(a: Int, b: Int) -> Int { return a + b }");
        assert_eq!(hir.items.len(), 1);
        if let HirItem::Function(f) = &hir.items[0] {
            assert_eq!(f.name, "add");
            assert_eq!(f.params.len(), 2);
        } else {
            panic!("Expected function");
        }
    }

    #[test]
    fn test_lower_struct() {
        let hir = parse_and_lower("struct User { name: Str, age: Int }");
        assert_eq!(hir.items.len(), 1);
        if let HirItem::Struct(s) = &hir.items[0] {
            assert_eq!(s.name, "User");
            assert_eq!(s.fields.len(), 2);
        } else {
            panic!("Expected struct");
        }
    }

    #[test]
    fn test_desugar_compound_assign() {
        let hir = parse_and_lower("fn test() { let mut x = 1\n x += 2 }");
        if let HirItem::Function(f) = &hir.items[0] {
            // Second statement should be desugared assignment
            assert_eq!(f.body.len(), 2);
            if let HirStmtKind::Assign { value, .. } = &f.body[1].kind {
                assert!(matches!(
                    value.kind,
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        ..
                    }
                ));
            } else {
                panic!("Expected assign");
            }
        }
    }

    #[test]
    fn test_desugar_increment() {
        let hir = parse_and_lower("fn test() { let mut x = 0\n x++ }");
        if let HirItem::Function(f) = &hir.items[0] {
            assert_eq!(f.body.len(), 2);
            if let HirStmtKind::Assign { value, .. } = &f.body[1].kind {
                assert!(matches!(
                    value.kind,
                    HirExprKind::BinOp {
                        op: HirBinOp::Add,
                        ..
                    }
                ));
            } else {
                panic!("Expected assign from increment");
            }
        }
    }

    #[test]
    fn test_desugar_infinite_for_loop() {
        // Note: New parser requires pattern; infinite loop uses wildcard or ident + no iterable
        let hir = parse_and_lower("fn test() { for _ { break } }");
        if let HirItem::Function(f) = &hir.items[0] {
            assert_eq!(f.body.len(), 1);
            if let HirStmtKind::While {
                condition, body, ..
            } = &f.body[0].kind
            {
                // Infinite loop has condition = true
                assert!(matches!(
                    condition.kind,
                    HirExprKind::Const(ConstValue::Bool(true))
                ));
                // Body contains break
                assert_eq!(body.len(), 1);
                assert!(matches!(body[0].kind, HirStmtKind::Break));
            } else {
                panic!("Expected while loop for desugared for loop");
            }
        }
    }

    #[test]
    fn test_desugar_range_for_loop() {
        let hir = parse_and_lower("fn test() { for i in 0..10 { print(i) } }");
        if let HirItem::Function(f) = &hir.items[0] {
            assert_eq!(f.body.len(), 1);
            // Desugared to block expression containing: let __idx = 0; while __idx < 10 { ... }
            if let HirStmtKind::Expr(expr) = &f.body[0].kind {
                if let HirExprKind::Block { stmts, .. } = &expr.kind {
                    // First stmt: let __i_idx = 0
                    assert_eq!(stmts.len(), 2);
                    if let HirStmtKind::Let { name, mutable, .. } = &stmts[0].kind {
                        assert!(name.contains("_idx"));
                        assert!(*mutable); // Index is mutable
                    } else {
                        panic!("Expected let statement for index initialization");
                    }
                    // Second stmt: while loop
                    assert!(matches!(stmts[1].kind, HirStmtKind::While { .. }));
                } else {
                    panic!("Expected block expression");
                }
            } else {
                panic!("Expected expression statement");
            }
        }
    }

    #[test]
    fn test_desugar_inclusive_range_for_loop() {
        let hir = parse_and_lower("fn test() { for i in 0..=5 { print(i) } }");
        if let HirItem::Function(f) = &hir.items[0] {
            if let HirStmtKind::Expr(expr) = &f.body[0].kind {
                if let HirExprKind::Block { stmts, .. } = &expr.kind {
                    // Check while condition uses LtEq for inclusive range
                    if let HirStmtKind::While { condition, .. } = &stmts[1].kind {
                        if let HirExprKind::BinOp { op, .. } = &condition.kind {
                            assert_eq!(*op, HirBinOp::LtEq);
                        } else {
                            panic!("Expected BinOp condition");
                        }
                    } else {
                        panic!("Expected while loop");
                    }
                }
            }
        }
    }

    #[test]
    fn test_desugar_array_for_loop() {
        let hir = parse_and_lower("fn test() { let arr = [1, 2, 3]\n for x in arr { print(x) } }");
        if let HirItem::Function(f) = &hir.items[0] {
            assert_eq!(f.body.len(), 2); // let arr; for x in arr
                                         // Second statement is desugared for loop
            if let HirStmtKind::Expr(expr) = &f.body[1].kind {
                if let HirExprKind::Block { stmts, .. } = &expr.kind {
                    // Should have: let __arr = arr; let __len = __arr.len(); let __idx = 0; while __idx < __len { ... }
                    assert_eq!(stmts.len(), 4);
                    // First: let __arr = arr
                    if let HirStmtKind::Let { name, mutable, .. } = &stmts[0].kind {
                        assert!(name.contains("_arr"));
                        assert!(!*mutable);
                    }
                    // Second: let __len = __arr.len()
                    if let HirStmtKind::Let { name, mutable, .. } = &stmts[1].kind {
                        assert!(name.contains("_len"));
                        assert!(!*mutable);
                    }
                    // Third: let __idx = 0
                    if let HirStmtKind::Let { name, mutable, .. } = &stmts[2].kind {
                        assert!(name.contains("_idx"));
                        assert!(*mutable);
                    }
                    // Fourth: while loop
                    if let HirStmtKind::While {
                        condition, body, ..
                    } = &stmts[3].kind
                    {
                        // Condition should be __idx < __len
                        if let HirExprKind::BinOp { op, .. } = &condition.kind {
                            assert_eq!(*op, HirBinOp::Lt);
                        }
                        // Body should have: let x = __arr[__idx]; print(x); __idx++
                        assert!(body.len() >= 2);
                    }
                }
            }
        }
    }

    #[test]
    fn test_desugar_indexed_array_for_loop() {
        let hir = parse_and_lower(
            "fn test() { let arr = [10, 20, 30]\n for i, x in arr { print(i, x) } }",
        );
        if let HirItem::Function(f) = &hir.items[0] {
            if let HirStmtKind::Expr(expr) = &f.body[1].kind {
                if let HirExprKind::Block { stmts, .. } = &expr.kind {
                    if let HirStmtKind::While { body, .. } = &stmts[2].kind {
                        // Body should have:
                        // 1. let i = __idx
                        // 2. let x = __arr[__idx]
                        // 3. print(i, x)
                        // 4. __idx++
                        assert!(body.len() >= 3);

                        // First should be index assignment
                        if let HirStmtKind::Let { name, .. } = &body[0].kind {
                            assert_eq!(name, "i");
                        } else {
                            panic!("Expected let statement for index");
                        }

                        // Second should be element extraction
                        if let HirStmtKind::Let { name, value, .. } = &body[1].kind {
                            assert_eq!(name, "x");
                            assert!(matches!(value.kind, HirExprKind::Index { .. }));
                        } else {
                            panic!("Expected let statement for element");
                        }
                    }
                }
            }
        }
    }
}

/// Convert HirBinOp to BinOpKind for centralized type inference.
fn hir_binop_to_kind(op: HirBinOp) -> BinOpKind {
    match op {
        HirBinOp::Add => BinOpKind::Add,
        HirBinOp::Sub => BinOpKind::Sub,
        HirBinOp::Mul => BinOpKind::Mul,
        HirBinOp::Div => BinOpKind::Div,
        HirBinOp::Mod => BinOpKind::Mod,
        HirBinOp::Eq => BinOpKind::Eq,
        HirBinOp::NotEq => BinOpKind::Ne,
        HirBinOp::Lt => BinOpKind::Lt,
        HirBinOp::Gt => BinOpKind::Gt,
        HirBinOp::LtEq => BinOpKind::Le,
        HirBinOp::GtEq => BinOpKind::Ge,
        HirBinOp::And => BinOpKind::And,
        HirBinOp::Or => BinOpKind::Or,
        // In and BitAnd/BitOr don't have direct equivalents, default to appropriate
        HirBinOp::In => BinOpKind::Eq, // Comparison semantics
        HirBinOp::BitAnd | HirBinOp::BitOr => BinOpKind::And, // Logical semantics for type inference
    }
}

/// Convert HirUnaryOp to UnaryOpKind for centralized type inference.
fn hir_unaryop_to_kind(op: HirUnaryOp) -> UnaryOpKind {
    match op {
        HirUnaryOp::Neg => UnaryOpKind::Neg,
        HirUnaryOp::Not => UnaryOpKind::Not,
    }
}
