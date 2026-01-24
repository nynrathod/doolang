//! HIR Visitor Pattern
//!
//! Trait-based visitors for traversing HIR nodes.
//! Used by analysis passes to walk the HIR tree.

use crate::types::*;

/// Immutable HIR visitor.
///
/// Implement this trait to walk the HIR tree without modifying it.
/// Default implementations recursively visit child nodes.
pub trait HirVisitor {
    // === Program ===
    fn visit_program(&mut self, program: &HirProgram) {
        for item in &program.items {
            self.visit_item(item);
        }
    }

    // === Items ===
    fn visit_item(&mut self, item: &HirItem) {
        match item {
            HirItem::Function(f) => self.visit_function(f),
            HirItem::Struct(s) => self.visit_struct(s),
            HirItem::Enum(e) => self.visit_enum(e),
            HirItem::Import(i) => self.visit_import(i),
        }
    }

    fn visit_function(&mut self, func: &HirFunction) {
        for param in &func.params {
            self.visit_param(param);
        }
        for stmt in &func.body {
            self.visit_stmt(stmt);
        }
    }

    fn visit_struct(&mut self, s: &HirStruct) {
        for field in &s.fields {
            self.visit_field(field);
        }
    }

    fn visit_enum(&mut self, e: &HirEnum) {
        for variant in &e.variants {
            self.visit_variant(variant);
        }
    }

    fn visit_import(&mut self, _import: &HirImport) {}

    fn visit_param(&mut self, _param: &HirParam) {}
    fn visit_field(&mut self, field: &HirField) {
        if let Some(default) = &field.default {
            self.visit_expr(default);
        }
    }
    fn visit_variant(&mut self, _variant: &HirVariant) {}

    // === Statements ===
    fn visit_stmt(&mut self, stmt: &HirStmt) {
        match &stmt.kind {
            HirStmtKind::Let { value, .. } => {
                self.visit_expr(value);
            }
            HirStmtKind::ManualErrorExtract { expr, .. } => {
                self.visit_expr(expr);
            }
            HirStmtKind::Assign { target, value } => {
                self.visit_expr(target);
                self.visit_expr(value);
            }
            HirStmtKind::Expr(expr) => {
                self.visit_expr(expr);
            }
            HirStmtKind::Return(values) => {
                for v in values {
                    self.visit_expr(v);
                }
            }
            HirStmtKind::Break | HirStmtKind::Continue | HirStmtKind::Drop { .. } => {}
            HirStmtKind::If { condition, then_block, else_block } => {
                self.visit_expr(condition);
                for stmt in then_block {
                    self.visit_stmt(stmt);
                }
                if let Some(else_stmts) = else_block {
                    for stmt in else_stmts {
                        self.visit_stmt(stmt);
                    }
                }
            }
            HirStmtKind::While { condition, body } => {
                self.visit_expr(condition);
                for stmt in body {
                    self.visit_stmt(stmt);
                }
            }
        }
    }

    // === Expressions ===
    fn visit_expr(&mut self, expr: &HirExpr) {
        match &expr.kind {
            HirExprKind::Const(_) | HirExprKind::Local { .. } | HirExprKind::Global { .. } => {}
            
            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.visit_expr(lhs);
                self.visit_expr(rhs);
            }
            HirExprKind::UnaryOp { operand, .. } => {
                self.visit_expr(operand);
            }
            HirExprKind::Call { func, args } => {
                self.visit_expr(func);
                for arg in args {
                    self.visit_expr(arg);
                }
            }
            HirExprKind::MethodCall { receiver, args, .. } => {
                self.visit_expr(receiver);
                for arg in args {
                    self.visit_expr(arg);
                }
            }
            HirExprKind::Field { object, .. } => {
                self.visit_expr(object);
            }
            HirExprKind::Index { object, index } => {
                self.visit_expr(object);
                self.visit_expr(index);
            }
            HirExprKind::Array(elements) | HirExprKind::Tuple(elements) => {
                for elem in elements {
                    self.visit_expr(elem);
                }
            }
            HirExprKind::Map(entries) => {
                for kv in entries {
                    self.visit_expr(&kv.0);
                    self.visit_expr(&kv.1);
                }
            }
            HirExprKind::Struct { fields, .. } => {
                for (_, value) in fields {
                    self.visit_expr(value);
                }
            }
            HirExprKind::EnumVariant { payload, .. } => {
                for e in payload {
                    self.visit_expr(e);
                }
            }
            HirExprKind::If { condition, then_expr, else_expr } => {
                self.visit_expr(condition);
                self.visit_expr(then_expr);
                if let Some(e) = else_expr {
                    self.visit_expr(e);
                }
            }
            HirExprKind::Match { values, arms } => {
                for v in values {
                    self.visit_expr(v);
                }
                for arm in arms {
                    self.visit_match_pattern(&arm.pattern);
                    if let Some(g) = &arm.guard {
                        self.visit_expr(g);
                    }
                    self.visit_expr(&arm.body);
                }
            }
            HirExprKind::Block { stmts, expr } => {
                for stmt in stmts {
                    self.visit_stmt(stmt);
                }
                if let Some(e) = expr {
                    self.visit_expr(e);
                }
            }
            HirExprKind::Range { start, end, .. } => {
                self.visit_expr(start);
                self.visit_expr(end);
            }
            HirExprKind::Ok(inner) | HirExprKind::Err(inner) | HirExprKind::Try(inner) => {
                self.visit_expr(inner);
            }
            HirExprKind::UnwrapOrPanic { expr: inner, message } => {
                self.visit_expr(inner);
                self.visit_expr(message);
            }
            HirExprKind::Move(inner) | HirExprKind::Clone(inner) => {
                self.visit_expr(inner);
            }
            HirExprKind::Borrow { expr, .. } => {
                self.visit_expr(expr);
            }
            HirExprKind::Closure { body, .. } => {
                self.visit_expr(body);
            }
        }
    }

    fn visit_match_pattern(&mut self, p: &HirMatchPattern) {
        match p {
            HirMatchPattern::Literal(e) | HirMatchPattern::Condition(e) => self.visit_expr(e),
            HirMatchPattern::Wildcard
            | HirMatchPattern::EnumVariant { .. }
            | HirMatchPattern::EnumVariantPayload { .. } => {}
            HirMatchPattern::Tuple(parts) => {
                for x in parts {
                    self.visit_match_pattern(x);
                }
            }
        }
    }
}

/// Mutable HIR visitor.
///
/// Like `HirVisitor` but takes mutable references to nodes.
/// Used by transformation passes.
pub trait HirVisitorMut {
    fn visit_program_mut(&mut self, program: &mut HirProgram) {
        for item in &mut program.items {
            self.visit_item_mut(item);
        }
    }

    fn visit_item_mut(&mut self, item: &mut HirItem) {
        match item {
            HirItem::Function(f) => self.visit_function_mut(f),
            HirItem::Struct(s) => self.visit_struct_mut(s),
            HirItem::Enum(e) => self.visit_enum_mut(e),
            HirItem::Import(i) => self.visit_import_mut(i),
        }
    }

    fn visit_function_mut(&mut self, func: &mut HirFunction) {
        for stmt in &mut func.body {
            self.visit_stmt_mut(stmt);
        }
    }

    fn visit_struct_mut(&mut self, s: &mut HirStruct) {
        for field in &mut s.fields {
            self.visit_field_mut(field);
        }
    }

    fn visit_enum_mut(&mut self, _e: &mut HirEnum) {}
    fn visit_import_mut(&mut self, _import: &mut HirImport) {}
    fn visit_field_mut(&mut self, field: &mut HirField) {
        if let Some(default) = &mut field.default {
            self.visit_expr_mut(default);
        }
    }

    fn visit_stmt_mut(&mut self, stmt: &mut HirStmt) {
        match &mut stmt.kind {
            HirStmtKind::Let { value, .. } => {
                self.visit_expr_mut(value);
            }
            HirStmtKind::ManualErrorExtract { expr, .. } => {
                self.visit_expr_mut(expr);
            }
            HirStmtKind::Assign { target, value } => {
                self.visit_expr_mut(target);
                self.visit_expr_mut(value);
            }
            HirStmtKind::Expr(expr) => {
                self.visit_expr_mut(expr);
            }
            HirStmtKind::Return(values) => {
                for v in values {
                    self.visit_expr_mut(v);
                }
            }
            HirStmtKind::Break | HirStmtKind::Continue | HirStmtKind::Drop { .. } => {}
            HirStmtKind::If { condition, then_block, else_block } => {
                self.visit_expr_mut(condition);
                for stmt in then_block {
                    self.visit_stmt_mut(stmt);
                }
                if let Some(else_stmts) = else_block {
                    for stmt in else_stmts {
                        self.visit_stmt_mut(stmt);
                    }
                }
            }
            HirStmtKind::While { condition, body } => {
                self.visit_expr_mut(condition);
                for stmt in body {
                    self.visit_stmt_mut(stmt);
                }
            }
        }
    }

    fn visit_expr_mut(&mut self, expr: &mut HirExpr) {
        match &mut expr.kind {
            HirExprKind::Const(_) | HirExprKind::Local { .. } | HirExprKind::Global { .. } => {}
            
            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.visit_expr_mut(lhs);
                self.visit_expr_mut(rhs);
            }
            HirExprKind::UnaryOp { operand, .. } => {
                self.visit_expr_mut(operand);
            }
            HirExprKind::Call { func, args } => {
                self.visit_expr_mut(func);
                for arg in args {
                    self.visit_expr_mut(arg);
                }
            }
            HirExprKind::MethodCall { receiver, args, .. } => {
                self.visit_expr_mut(receiver);
                for arg in args {
                    self.visit_expr_mut(arg);
                }
            }
            HirExprKind::Field { object, .. } => {
                self.visit_expr_mut(object);
            }
            HirExprKind::Index { object, index } => {
                self.visit_expr_mut(object);
                self.visit_expr_mut(index);
            }
            HirExprKind::Array(elements) | HirExprKind::Tuple(elements) => {
                for elem in elements {
                    self.visit_expr_mut(elem);
                }
            }
            HirExprKind::Map(entries) => {
                for kv in entries {
                    self.visit_expr_mut(&mut kv.0);
                    self.visit_expr_mut(&mut kv.1);
                }
            }
            HirExprKind::Struct { fields, .. } => {
                for (_, value) in fields {
                    self.visit_expr_mut(value);
                }
            }
            HirExprKind::EnumVariant { payload, .. } => {
                for e in payload {
                    self.visit_expr_mut(e);
                }
            }
            HirExprKind::If { condition, then_expr, else_expr } => {
                self.visit_expr_mut(condition);
                self.visit_expr_mut(then_expr);
                if let Some(e) = else_expr {
                    self.visit_expr_mut(e);
                }
            }
            HirExprKind::Match { values, arms } => {
                for v in values {
                    self.visit_expr_mut(v);
                }
                for arm in arms {
                    self.visit_match_pattern_mut(&mut arm.pattern);
                    if let Some(g) = &mut arm.guard {
                        self.visit_expr_mut(g);
                    }
                    self.visit_expr_mut(&mut arm.body);
                }
            }
            HirExprKind::Block { stmts, expr } => {
                for stmt in stmts {
                    self.visit_stmt_mut(stmt);
                }
                if let Some(e) = expr {
                    self.visit_expr_mut(e);
                }
            }
            HirExprKind::Range { start, end, .. } => {
                self.visit_expr_mut(start);
                self.visit_expr_mut(end);
            }
            HirExprKind::Ok(inner) | HirExprKind::Err(inner) | HirExprKind::Try(inner) => {
                self.visit_expr_mut(inner);
            }
            HirExprKind::UnwrapOrPanic { expr: inner, message } => {
                self.visit_expr_mut(inner);
                self.visit_expr_mut(message);
            }
            HirExprKind::Move(inner) | HirExprKind::Clone(inner) => {
                self.visit_expr_mut(inner);
            }
            HirExprKind::Borrow { expr, .. } => {
                self.visit_expr_mut(expr);
            }
            HirExprKind::Closure { body, .. } => {
                self.visit_expr_mut(body);
            }
        }
    }

    fn visit_match_pattern_mut(&mut self, p: &mut HirMatchPattern) {
        match p {
            HirMatchPattern::Literal(e) | HirMatchPattern::Condition(e) => self.visit_expr_mut(e),
            HirMatchPattern::Wildcard
            | HirMatchPattern::EnumVariant { .. }
            | HirMatchPattern::EnumVariantPayload { .. } => {}
            HirMatchPattern::Tuple(parts) => {
                for x in parts {
                    self.visit_match_pattern_mut(x);
                }
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use doo_frontend::Parser;
    use crate::Lower;

    /// Counter visitor for testing.
    struct ExprCounter {
        count: usize,
    }

    impl HirVisitor for ExprCounter {
        fn visit_expr(&mut self, expr: &HirExpr) {
            self.count += 1;
            // Call default implementation to recurse
            match &expr.kind {
                HirExprKind::BinOp { lhs, rhs, .. } => {
                    self.visit_expr(lhs);
                    self.visit_expr(rhs);
                }
                HirExprKind::Call { func, args } => {
                    self.visit_expr(func);
                    for arg in args {
                        self.visit_expr(arg);
                    }
                }
                _ => {}
            }
        }
    }

    #[test]
    fn test_visitor_count_exprs() {
        let mut parser = Parser::new("fn test() { let x = 1 + 2 }", 0);
        let program = parser.parse_program().unwrap();
        let mut lower = Lower::new();
        let hir = lower.lower_program(&program);

        let mut counter = ExprCounter { count: 0 };
        counter.visit_program(&hir);
        
        // Should count: 1, 2, and the binop expression = 3 expressions
        assert!(counter.count >= 3);
    }
}
