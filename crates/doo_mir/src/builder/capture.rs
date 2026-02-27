//! Free-variable (capture) analysis for go blocks and closures.
//!
//! Walks an HIR expression tree, collects all `Local` references,
//! subtracts variables defined within the tree, and returns the
//! list of free variables that must be captured from the outer scope.

use doo_hir::{HirExpr, HirExprKind, HirStmt, HirStmtKind};
use std::collections::HashSet;

use super::MirBuilder;

/// Builtin intrinsic names that should not be captured.
fn is_intrinsic_name(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "println"
            | "__print_interp"
            | "sleep"
            | "typeOf"
            | "len"
            | "toString"
            | "toInt"
            | "toFloat"
            | "input"
            | "panic"
            | "assert"
    )
}

/// Collect free variables from an HIR expression body.
/// Returns a sorted Vec of variable names that must be captured.
pub fn collect_free_vars(body: &HirExpr, builder: &MirBuilder) -> Vec<String> {
    let mut referenced = HashSet::new();
    let mut defined = HashSet::new();
    walk_expr(body, &mut referenced, &mut defined);

    let mut free_vars: Vec<String> = referenced
        .difference(&defined)
        .filter(|name| {
            !builder.is_module_name(name)
                && !builder.is_function_name(name)
                && !builder.is_type_name(name)
                && !is_intrinsic_name(name)
        })
        .cloned()
        .collect();
    free_vars.sort(); // deterministic order for reproducible codegen
    free_vars
}

/// Walk an HIR expression, collecting `Local` references and locally-defined names.
fn walk_expr(expr: &HirExpr, referenced: &mut HashSet<String>, defined: &mut HashSet<String>) {
    match &expr.kind {
        HirExprKind::Local { name } => {
            referenced.insert(name.clone());
        }
        HirExprKind::Global { name } => {
            // Globals don't need capture
            let _ = name;
        }
        HirExprKind::Const(_) => {}

        HirExprKind::BinOp { lhs, rhs, .. } => {
            walk_expr(lhs, referenced, defined);
            walk_expr(rhs, referenced, defined);
        }
        HirExprKind::UnaryOp { operand, .. } => {
            walk_expr(operand, referenced, defined);
        }
        HirExprKind::Call { func, args } => {
            walk_expr(func, referenced, defined);
            for a in args {
                walk_expr(a, referenced, defined);
            }
        }
        HirExprKind::MethodCall { receiver, args, .. } => {
            walk_expr(receiver, referenced, defined);
            for a in args {
                walk_expr(a, referenced, defined);
            }
        }
        HirExprKind::Field { object, .. } => {
            walk_expr(object, referenced, defined);
        }
        HirExprKind::Index { object, index } => {
            walk_expr(object, referenced, defined);
            walk_expr(index, referenced, defined);
        }
        HirExprKind::Array(elems) => {
            for e in elems {
                walk_expr(e, referenced, defined);
            }
        }
        HirExprKind::Map(entries) => {
            for (k, v) in entries {
                walk_expr(k, referenced, defined);
                walk_expr(v, referenced, defined);
            }
        }
        HirExprKind::Tuple(elems) => {
            for e in elems {
                walk_expr(e, referenced, defined);
            }
        }
        HirExprKind::Struct { fields, .. } => {
            for (_, v) in fields {
                walk_expr(v, referenced, defined);
            }
        }
        HirExprKind::EnumVariant { payload, .. } => {
            for p in payload {
                walk_expr(p, referenced, defined);
            }
        }
        HirExprKind::Spread(inner) => {
            walk_expr(inner, referenced, defined);
        }
        HirExprKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            walk_expr(condition, referenced, defined);
            walk_expr(then_expr, referenced, defined);
            if let Some(e) = else_expr {
                walk_expr(e, referenced, defined);
            }
        }
        HirExprKind::Block { stmts, expr } => {
            for s in stmts {
                walk_stmt(s, referenced, defined);
            }
            if let Some(e) = expr {
                walk_expr(e, referenced, defined);
            }
        }
        HirExprKind::Match { values, arms } => {
            for v in values {
                walk_expr(v, referenced, defined);
            }
            for arm in arms {
                // Match arm patterns can define variables
                walk_match_pattern_defs(&arm.pattern, defined);
                if let Some(g) = &arm.guard {
                    walk_expr(g, referenced, defined);
                }
                walk_expr(&arm.body, referenced, defined);
            }
        }
        HirExprKind::Range { start, end, .. } => {
            walk_expr(start, referenced, defined);
            walk_expr(end, referenced, defined);
        }
        HirExprKind::Ok(inner) | HirExprKind::Err(inner) | HirExprKind::Try(inner) => {
            walk_expr(inner, referenced, defined);
        }
        HirExprKind::UnwrapOrPanic { expr, message } => {
            walk_expr(expr, referenced, defined);
            walk_expr(message, referenced, defined);
        }
        HirExprKind::RouteBlock { routes } => {
            for r in routes {
                walk_expr(r, referenced, defined);
            }
        }
        HirExprKind::Move(inner) | HirExprKind::Clone(inner) => {
            walk_expr(inner, referenced, defined);
        }
        HirExprKind::Borrow { expr: inner, .. } => {
            walk_expr(inner, referenced, defined);
        }
        HirExprKind::Closure { params, body } => {
            // Closure params define new variables within the closure body
            for (name, _) in params {
                defined.insert(name.clone());
            }
            walk_expr(body, referenced, defined);
        }
        HirExprKind::Cast { value, .. } => {
            walk_expr(value, referenced, defined);
        }
        HirExprKind::Await(inner) => {
            walk_expr(inner, referenced, defined);
        }
        HirExprKind::Spawn { body } => {
            // Don't recurse into nested spawns — they handle their own captures
            // But we DO need to track references from within them that reference
            // our scope (those will be captured by this spawn, then re-captured by inner)
            walk_expr(body, referenced, defined);
        }
        HirExprKind::ScopeBlock { stmts } => {
            for s in stmts {
                walk_stmt(s, referenced, defined);
            }
        }
    }
}

/// Walk an HIR statement, collecting references and definitions.
fn walk_stmt(stmt: &HirStmt, referenced: &mut HashSet<String>, defined: &mut HashSet<String>) {
    match &stmt.kind {
        HirStmtKind::Let { name, value, .. } => {
            walk_expr(value, referenced, defined);
            defined.insert(name.clone());
        }
        HirStmtKind::TupleLet { names, value, .. } => {
            walk_expr(value, referenced, defined);
            for n in names {
                defined.insert(n.clone());
            }
        }
        HirStmtKind::Assign { target, value } => {
            walk_expr(target, referenced, defined);
            walk_expr(value, referenced, defined);
        }
        HirStmtKind::Expr(e) => {
            walk_expr(e, referenced, defined);
        }
        HirStmtKind::Return(exprs) => {
            for e in exprs {
                walk_expr(e, referenced, defined);
            }
        }
        HirStmtKind::Break | HirStmtKind::Continue => {}
        HirStmtKind::Drop { .. } => {}
        HirStmtKind::If {
            condition,
            then_block,
            else_block,
        } => {
            walk_expr(condition, referenced, defined);
            for s in then_block {
                walk_stmt(s, referenced, defined);
            }
            if let Some(stmts) = else_block {
                for s in stmts {
                    walk_stmt(s, referenced, defined);
                }
            }
        }
        HirStmtKind::While {
            condition,
            body,
            increment,
        } => {
            walk_expr(condition, referenced, defined);
            for s in body {
                walk_stmt(s, referenced, defined);
            }
            for s in increment {
                walk_stmt(s, referenced, defined);
            }
        }
        HirStmtKind::ManualErrorExtract {
            ok_names,
            error_name,
            expr,
        } => {
            walk_expr(expr, referenced, defined);
            for n in ok_names {
                defined.insert(n.clone());
            }
            defined.insert(error_name.clone());
        }
    }
}

/// Extract variable definitions from match patterns.
fn walk_match_pattern_defs(pattern: &doo_hir::HirMatchPattern, defined: &mut HashSet<String>) {
    match pattern {
        doo_hir::HirMatchPattern::EnumVariantPayload { bindings, .. } => {
            for name in bindings {
                defined.insert(name.clone());
            }
        }
        doo_hir::HirMatchPattern::Tuple(patterns) => {
            for p in patterns {
                walk_match_pattern_defs(p, defined);
            }
        }
        // Literal, Condition, Wildcard, EnumVariant — no variable bindings
        _ => {}
    }
}
