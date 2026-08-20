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
            HirItem::Const(c) => self.visit_const(c),
            HirItem::Static(s) => self.visit_static(s),
            HirItem::Function(f) => self.visit_function(f),
            HirItem::Struct(s) => self.visit_struct(s),
            HirItem::Enum(e) => self.visit_enum(e),
            HirItem::Interface(i) => self.visit_interface(i),
            HirItem::Import(_) => {}
        }
    }

    fn visit_const(&mut self, c: &HirConst) {
        self.visit_expr(&c.value_expr);
    }

    fn visit_static(&mut self, _s: &HirStatic) {}

    fn visit_function(&mut self, f: &HirFunction) {
        for param in &f.params {
            self.visit_param(param);
        }
        for stmt in &f.body {
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

    fn visit_interface(&mut self, i: &HirInterface) {
        for method in &i.methods {
            self.visit_interface_method(method);
        }
    }

    fn visit_param(&mut self, _param: &HirParam) {}
    fn visit_field(&mut self, field: &HirField) {
        if let Some(default) = &field.default {
            self.visit_expr(default);
        }
    }
    fn visit_variant(&mut self, _variant: &HirVariant) {}
    fn visit_interface_method(&mut self, method: &HirInterfaceMethod) {
        for param in &method.params {
            self.visit_param(param);
        }
    }

    fn visit_stmt(&mut self, stmt: &HirStmt) {
        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &HirExpr) {
        walk_expr(self, expr);
    }

    fn visit_pattern(&mut self, pattern: &HirMatchPattern) {
        walk_pattern(self, pattern);
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
            HirItem::Const(c) => self.visit_const_mut(c),
            HirItem::Static(_) => {}
            HirItem::Function(f) => self.visit_function_mut(f),
            HirItem::Struct(s) => self.visit_struct_mut(s),
            HirItem::Enum(_) => {}
            HirItem::Interface(_) => {}
            HirItem::Import(_) => {}
        }
    }

    fn visit_const_mut(&mut self, c: &mut HirConst) {
        self.visit_expr_mut(&mut c.value_expr);
    }

    fn visit_function_mut(&mut self, f: &mut HirFunction) {
        for stmt in &mut f.body {
            self.visit_stmt_mut(stmt);
        }
    }

    fn visit_struct_mut(&mut self, s: &mut HirStruct) {
        for field in &mut s.fields {
            self.visit_field_mut(field);
        }
    }

    fn visit_field_mut(&mut self, field: &mut HirField) {
        if let Some(default) = &mut field.default {
            self.visit_expr_mut(default);
        }
    }

    fn visit_stmt_mut(&mut self, stmt: &mut HirStmt) {
        walk_stmt_mut(self, stmt);
    }

    fn visit_expr_mut(&mut self, expr: &mut HirExpr) {
        walk_expr_mut(self, expr);
    }

    fn visit_pattern_mut(&mut self, pattern: &mut HirMatchPattern) {
        walk_pattern_mut(self, pattern);
    }
}

// ============================================================================
// Walk Functions (Immutable)
// ============================================================================

pub fn walk_expr<V: HirVisitor + ?Sized>(visitor: &mut V, expr: &HirExpr) {
    match &expr.kind {
        HirExprKind::Const(_) | HirExprKind::Local { .. } | HirExprKind::Global { .. } => {}

        HirExprKind::BinOp { lhs, rhs, .. } => {
            visitor.visit_expr(lhs);
            visitor.visit_expr(rhs);
        }
        HirExprKind::UnaryOp { operand, .. } => visitor.visit_expr(operand),

        HirExprKind::Call { func, args } => {
            visitor.visit_expr(func);
            for arg in args {
                visitor.visit_expr(arg);
            }
        }
        HirExprKind::MethodCall { receiver, args, .. } => {
            visitor.visit_expr(receiver);
            for arg in args {
                visitor.visit_expr(arg);
            }
        }

        HirExprKind::Field { object, .. } => visitor.visit_expr(object),
        HirExprKind::Index { object, index } => {
            visitor.visit_expr(object);
            visitor.visit_expr(index);
        }

        HirExprKind::Array(elements) | HirExprKind::Tuple(elements) => {
            for elem in elements {
                visitor.visit_expr(elem);
            }
        }
        HirExprKind::Map(entries) => {
            for (k, v) in entries {
                visitor.visit_expr(k);
                visitor.visit_expr(v);
            }
        }
        HirExprKind::Struct { fields, .. } => {
            for (_, val) in fields {
                visitor.visit_expr(val);
            }
        }
        HirExprKind::EnumVariant { payload, .. } => {
            for e in payload {
                visitor.visit_expr(e);
            }
        }

        HirExprKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            visitor.visit_expr(condition);
            visitor.visit_expr(then_expr);
            if let Some(e) = else_expr {
                visitor.visit_expr(e);
            }
        }
        HirExprKind::Block { stmts, expr } => {
            for stmt in stmts {
                visitor.visit_stmt(stmt);
            }
            if let Some(e) = expr {
                visitor.visit_expr(e);
            }
        }
        HirExprKind::Match { values, arms } => {
            for v in values {
                visitor.visit_expr(v);
            }
            for arm in arms {
                visitor.visit_pattern(&arm.pattern);
                if let Some(g) = &arm.guard {
                    visitor.visit_expr(g);
                }
                visitor.visit_expr(&arm.body);
            }
        }
        HirExprKind::Range { start, end, .. } => {
            visitor.visit_expr(start);
            visitor.visit_expr(end);
        }

        HirExprKind::Ok(inner)
        | HirExprKind::Err(inner)
        | HirExprKind::Try(inner)
        | HirExprKind::Move(inner)
        | HirExprKind::Clone(inner)
        | HirExprKind::Await(inner)
        | HirExprKind::Spread(inner)
        | HirExprKind::Cast { value: inner, .. } => {
            visitor.visit_expr(inner);
        }
        HirExprKind::UnwrapOrPanic {
            expr: inner,
            message,
        } => {
            visitor.visit_expr(inner);
            visitor.visit_expr(message);
        }
        HirExprKind::Borrow { expr: inner, .. } => visitor.visit_expr(inner),

        HirExprKind::Closure { body, .. } => visitor.visit_expr(body),
        HirExprKind::Spawn { body } => visitor.visit_expr(body),
        HirExprKind::ScopeBlock { stmts } => {
            for stmt in stmts {
                visitor.visit_stmt(stmt);
            }
        }
    }
}

pub fn walk_stmt<V: HirVisitor + ?Sized>(visitor: &mut V, stmt: &HirStmt) {
    match &stmt.kind {
        HirStmtKind::Let { value, .. } | HirStmtKind::TupleLet { value, .. } => {
            visitor.visit_expr(value)
        }
        HirStmtKind::Expr(expr) => visitor.visit_expr(expr),
        HirStmtKind::Assign { target, value } => {
            visitor.visit_expr(target);
            visitor.visit_expr(value);
        }
        HirStmtKind::Return(values) => {
            for v in values {
                visitor.visit_expr(v);
            }
        }
        HirStmtKind::ManualErrorExtract { expr, .. } => visitor.visit_expr(expr),
        HirStmtKind::If {
            condition,
            then_block,
            else_block,
        } => {
            visitor.visit_expr(condition);
            for s in then_block {
                visitor.visit_stmt(s);
            }
            if let Some(else_stmts) = else_block {
                for s in else_stmts {
                    visitor.visit_stmt(s);
                }
            }
        }
        HirStmtKind::While {
            condition,
            body,
            increment,
        } => {
            visitor.visit_expr(condition);
            for s in body {
                visitor.visit_stmt(s);
            }
            for s in increment {
                visitor.visit_stmt(s);
            }
        }
        HirStmtKind::Break | HirStmtKind::Continue | HirStmtKind::Drop { .. } => {}
    }
}

pub fn walk_pattern<V: HirVisitor + ?Sized>(visitor: &mut V, pattern: &HirMatchPattern) {
    match pattern {
        HirMatchPattern::Literal(e) | HirMatchPattern::Condition(e) => visitor.visit_expr(e),
        HirMatchPattern::Wildcard
        | HirMatchPattern::EnumVariant { .. }
        | HirMatchPattern::EnumVariantPayload { .. }
        | HirMatchPattern::Rest(_) => {}
        HirMatchPattern::Tuple(parts) | HirMatchPattern::Array(parts) => {
            for p in parts {
                visitor.visit_pattern(p);
            }
        }
        HirMatchPattern::Struct { fields, .. } => {
            for (_, p) in fields {
                visitor.visit_pattern(p);
            }
        }
    }
}

// ============================================================================
// Walk Functions (Mutable)
// ============================================================================

pub fn walk_pattern_mut<V: HirVisitorMut + ?Sized>(visitor: &mut V, pattern: &mut HirMatchPattern) {
    match pattern {
        HirMatchPattern::Literal(e) | HirMatchPattern::Condition(e) => visitor.visit_expr_mut(e),
        HirMatchPattern::Wildcard
        | HirMatchPattern::EnumVariant { .. }
        | HirMatchPattern::EnumVariantPayload { .. }
        | HirMatchPattern::Rest(_) => {}
        HirMatchPattern::Tuple(parts) | HirMatchPattern::Array(parts) => {
            for p in parts {
                visitor.visit_pattern_mut(p);
            }
        }
        HirMatchPattern::Struct { fields, .. } => {
            for (_, p) in fields {
                visitor.visit_pattern_mut(p);
            }
        }
    }
}

// ============================================================================
// Walk Functions (Mutable)
// ============================================================================

pub fn walk_expr_mut<V: HirVisitorMut + ?Sized>(visitor: &mut V, expr: &mut HirExpr) {
    match &mut expr.kind {
        HirExprKind::Const(_) | HirExprKind::Local { .. } | HirExprKind::Global { .. } => {}

        HirExprKind::BinOp { lhs, rhs, .. } => {
            visitor.visit_expr_mut(lhs);
            visitor.visit_expr_mut(rhs);
        }
        HirExprKind::UnaryOp { operand, .. } => visitor.visit_expr_mut(operand),

        HirExprKind::Call { func, args } => {
            visitor.visit_expr_mut(func);
            for arg in args {
                visitor.visit_expr_mut(arg);
            }
        }
        HirExprKind::MethodCall { receiver, args, .. } => {
            visitor.visit_expr_mut(receiver);
            for arg in args {
                visitor.visit_expr_mut(arg);
            }
        }

        HirExprKind::Field { object, .. } => visitor.visit_expr_mut(object),
        HirExprKind::Index { object, index } => {
            visitor.visit_expr_mut(object);
            visitor.visit_expr_mut(index);
        }

        HirExprKind::Array(elements) | HirExprKind::Tuple(elements) => {
            for elem in elements {
                visitor.visit_expr_mut(elem);
            }
        }
        HirExprKind::Map(entries) => {
            for (k, v) in entries {
                visitor.visit_expr_mut(k);
                visitor.visit_expr_mut(v);
            }
        }
        HirExprKind::Struct { fields, .. } => {
            for (_, val) in fields {
                visitor.visit_expr_mut(val);
            }
        }
        HirExprKind::EnumVariant { payload, .. } => {
            for e in payload {
                visitor.visit_expr_mut(e);
            }
        }

        HirExprKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            visitor.visit_expr_mut(condition);
            visitor.visit_expr_mut(then_expr);
            if let Some(e) = else_expr {
                visitor.visit_expr_mut(e);
            }
        }
        HirExprKind::Block { stmts, expr } => {
            for stmt in stmts {
                visitor.visit_stmt_mut(stmt);
            }
            if let Some(e) = expr {
                visitor.visit_expr_mut(e);
            }
        }
        HirExprKind::Match { values, arms } => {
            for v in values {
                visitor.visit_expr_mut(v);
            }
            for arm in arms {
                if let Some(g) = &mut arm.guard {
                    visitor.visit_expr_mut(g);
                }
                visitor.visit_expr_mut(&mut arm.body);
            }
        }
        HirExprKind::Range { start, end, .. } => {
            visitor.visit_expr_mut(start);
            visitor.visit_expr_mut(end);
        }

        HirExprKind::Ok(inner)
        | HirExprKind::Err(inner)
        | HirExprKind::Try(inner)
        | HirExprKind::Move(inner)
        | HirExprKind::Clone(inner)
        | HirExprKind::Await(inner)
        | HirExprKind::Spread(inner)
        | HirExprKind::Cast { value: inner, .. } => {
            visitor.visit_expr_mut(inner);
        }
        HirExprKind::UnwrapOrPanic {
            expr: inner,
            message,
        } => {
            visitor.visit_expr_mut(inner);
            visitor.visit_expr_mut(message);
        }
        HirExprKind::Borrow { expr: inner, .. } => visitor.visit_expr_mut(inner),

        HirExprKind::Closure { body, .. } => visitor.visit_expr_mut(body),
        HirExprKind::Spawn { body } => visitor.visit_expr_mut(body),
        HirExprKind::ScopeBlock { stmts } => {
            for stmt in stmts {
                visitor.visit_stmt_mut(stmt);
            }
        }
    }
}

pub fn walk_stmt_mut<V: HirVisitorMut + ?Sized>(visitor: &mut V, stmt: &mut HirStmt) {
    match &mut stmt.kind {
        HirStmtKind::Let { value, .. } | HirStmtKind::TupleLet { value, .. } => {
            visitor.visit_expr_mut(value)
        }
        HirStmtKind::Expr(expr) => visitor.visit_expr_mut(expr),
        HirStmtKind::Assign { target, value } => {
            visitor.visit_expr_mut(target);
            visitor.visit_expr_mut(value);
        }
        HirStmtKind::Return(values) => {
            for v in values {
                visitor.visit_expr_mut(v);
            }
        }
        HirStmtKind::ManualErrorExtract { expr, .. } => visitor.visit_expr_mut(expr),
        HirStmtKind::If {
            condition,
            then_block,
            else_block,
        } => {
            visitor.visit_expr_mut(condition);
            for s in then_block {
                visitor.visit_stmt_mut(s);
            }
            if let Some(else_stmts) = else_block {
                for s in else_stmts {
                    visitor.visit_stmt_mut(s);
                }
            }
        }
        HirStmtKind::While {
            condition,
            body,
            increment,
        } => {
            visitor.visit_expr_mut(condition);
            for s in body {
                visitor.visit_stmt_mut(s);
            }
            for s in increment {
                visitor.visit_stmt_mut(s);
            }
        }
        HirStmtKind::Break | HirStmtKind::Continue | HirStmtKind::Drop { .. } => {}
    }
}

/// Default walker implementation.
pub struct WalkHir;
impl HirVisitor for WalkHir {}
