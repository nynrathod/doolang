//! Step limiting and forbidden operation detection for CTFE.

use doo_thir::{ThirExpr, ThirExprKind, ThirStmt, ThirStmtKind};

/// Default maximum number of evaluation steps before aborting.
/// Prevents infinite loops from hanging the compiler.
pub const DEFAULT_STEP_LIMIT: u64 = 10_000_000;

/// Check if a THIR expression is forbidden in const context.
///
/// Returns `Some(reason)` if forbidden, `None` if allowed.
pub fn is_forbidden_expr(expr: &ThirExpr) -> Option<String> {
    match &expr.kind {
        ThirExprKind::Call { .. } => {
            Some("function calls are not allowed in const context".to_string())
        }
        ThirExprKind::MethodCall { .. } => {
            Some("method calls are not allowed in const context".to_string())
        }
        ThirExprKind::Async(_) => Some("async is not allowed in const context".to_string()),
        ThirExprKind::Await(_) => Some("await is not allowed in const context".to_string()),
        ThirExprKind::Spawn(_) => Some("go/spawn is not allowed in const context".to_string()),
        ThirExprKind::Try(_) => Some("? operator is not allowed in const context".to_string()),
        ThirExprKind::UnwrapOrPanic { .. } => {
            Some("?? (panic) is not allowed in const context".to_string())
        }
        ThirExprKind::Borrow { .. } => Some("borrows are not allowed in const context".to_string()),
        ThirExprKind::Closure { .. } => {
            Some("closures are not allowed in const context".to_string())
        }
        ThirExprKind::Spread(_) => Some("spread is not allowed in const context".to_string()),
        _ => None,
    }
}

/// Check if a THIR statement is forbidden in const context.
pub fn is_forbidden_stmt(stmt: &ThirStmt) -> Option<String> {
    match &stmt.kind {
        ThirStmtKind::Go { .. } => Some("go blocks are not allowed in const context".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_step_limit() {
        assert_eq!(DEFAULT_STEP_LIMIT, 10_000_000);
    }
}
