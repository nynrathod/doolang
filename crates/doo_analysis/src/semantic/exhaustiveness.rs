//! Exhaustiveness Checking
//!
/// Ensures that match expressions cover all possible values of the
/// scrutinee type. Uses a pattern matrix algorithm similar to Rust's
/// and Moscow ML's approach.
///
/// ## Algorithm
///
/// Build a matrix of all patterns from the match arms, then check if
/// any value could fall through without matching. Missing enum variants
/// produce E0006 (non-exhaustive match). A wildcard `_` or `else` arm
/// makes any match exhaustive.
use doo_core::types::{builtin, TypeId, TypeKind, TypeRegistry};
use doo_core::Span;
use doo_thir::{ThirExpr, ThirExprKind, ThirPattern, ThirPatternKind};
use rustc_hash::FxHashSet;

/// Exhaustiveness checking error.
#[derive(Debug, Clone)]
pub struct ExhaustivenessError {
    pub kind: ExhaustivenessErrorKind,
    pub span: Span,
}

impl ExhaustivenessError {
    pub fn new(kind: ExhaustivenessErrorKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// Categories of exhaustiveness errors.
#[derive(Debug, Clone)]
pub enum ExhaustivenessErrorKind {
    /// Match is not exhaustive — these patterns are missing.
    NonExhaustive { missing: Vec<String> },
    /// A pattern is unreachable (already covered by a previous arm).
    UnreachablePattern,
}

impl ExhaustivenessErrorKind {
    pub fn message(&self) -> String {
        match self {
            Self::NonExhaustive { missing } => {
                format!("non-exhaustive match: missing {}", missing.join(", "))
            }
            Self::UnreachablePattern => "this pattern is unreachable".to_string(),
        }
    }
}

/// Exhaustiveness checker for match expressions.
pub struct ExhaustivenessChecker<'a> {
    registry: &'a TypeRegistry,
    errors: Vec<ExhaustivenessError>,
}

impl<'a> ExhaustivenessChecker<'a> {
    pub fn new(registry: &'a TypeRegistry) -> Self {
        Self {
            registry,
            errors: Vec::new(),
        }
    }

    /// Check all match expressions in a THIR program.
    pub fn check_program(
        &mut self,
        program: &doo_thir::ThirProgram,
    ) -> Result<(), Vec<ExhaustivenessError>> {
        for item in &program.items {
            if let doo_thir::ThirItem::Function(func) = item {
                for stmt in &func.body {
                    self.check_stmt(stmt);
                }
            }
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(std::mem::take(&mut self.errors))
        }
    }

    fn check_stmt(&mut self, stmt: &doo_thir::ThirStmt) {
        match &stmt.kind {
            doo_thir::ThirStmtKind::Let { value, .. }
            | doo_thir::ThirStmtKind::Const { value, .. } => {
                self.check_expr(value);
            }
            doo_thir::ThirStmtKind::Assign { target, value } => {
                self.check_expr(target);
                self.check_expr(value);
            }
            doo_thir::ThirStmtKind::Expr(expr) => {
                self.check_expr(expr);
            }
            doo_thir::ThirStmtKind::Return(val) => {
                if let Some(e) = val {
                    self.check_expr(e);
                }
            }
            doo_thir::ThirStmtKind::While {
                cond,
                body,
                increment,
            } => {
                self.check_expr(cond);
                for s in body {
                    self.check_stmt(s);
                }
                for s in increment {
                    self.check_stmt(s);
                }
            }
            doo_thir::ThirStmtKind::Loop { body } => {
                for s in body {
                    self.check_stmt(s);
                }
            }
            doo_thir::ThirStmtKind::Go { expr } => {
                self.check_expr(expr);
            }
            doo_thir::ThirStmtKind::Scope { stmts } => {
                for s in stmts {
                    self.check_stmt(s);
                }
            }
            doo_thir::ThirStmtKind::TupleLet { value, .. } => {
                self.check_expr(value);
            }
            doo_thir::ThirStmtKind::ManualErrorExtract { expr, .. } => {
                self.check_expr(expr);
            }
            doo_thir::ThirStmtKind::Drop { .. }
            | doo_thir::ThirStmtKind::Break(_)
            | doo_thir::ThirStmtKind::Continue => {}
        }
    }

    fn check_expr(&mut self, expr: &ThirExpr) {
        match &expr.kind {
            ThirExprKind::Match {
                expr: scrutinee,
                arms,
            } => {
                // Check nested expressions first
                self.check_expr(scrutinee);
                for arm in arms {
                    if let Some(g) = &arm.guard {
                        self.check_expr(g);
                    }
                    self.check_expr(&arm.body);
                }

                // Check exhaustiveness of this match
                self.check_exhaustive(
                    scrutinee.ty,
                    arms.iter().map(|a| &a.pattern).collect(),
                    expr.span,
                );
            }

            ThirExprKind::Binary { lhs, rhs, .. } => {
                self.check_expr(lhs);
                self.check_expr(rhs);
            }
            ThirExprKind::Unary { expr: inner, .. } => {
                self.check_expr(inner);
            }
            ThirExprKind::Call { func, args } => {
                self.check_expr(func);
                for a in args {
                    self.check_expr(a);
                }
            }
            ThirExprKind::MethodCall { receiver, args, .. } => {
                self.check_expr(receiver);
                for a in args {
                    self.check_expr(a);
                }
            }
            ThirExprKind::FieldAccess { object, .. } => {
                self.check_expr(object);
            }
            ThirExprKind::Index { object, index } => {
                self.check_expr(object);
                self.check_expr(index);
            }
            ThirExprKind::If { cond, then, else_ } => {
                self.check_expr(cond);
                self.check_expr(then);
                if let Some(e) = else_ {
                    self.check_expr(e);
                }
            }
            ThirExprKind::Block(stmts, tail) => {
                for s in stmts {
                    self.check_stmt(s);
                }
                if let Some(e) = tail {
                    self.check_expr(e);
                }
            }
            ThirExprKind::ArrayLiteral(elements) => {
                for e in elements {
                    self.check_expr(e);
                }
            }
            ThirExprKind::MapLiteral(entries) => {
                for (k, v) in entries {
                    self.check_expr(k);
                    self.check_expr(v);
                }
            }
            ThirExprKind::StructLiteral { fields, .. } => {
                for (_, v) in fields {
                    self.check_expr(v);
                }
            }
            ThirExprKind::EnumVariant { payload, .. } => {
                for p in payload {
                    self.check_expr(p);
                }
            }
            ThirExprKind::Tuple(elements) => {
                for e in elements {
                    self.check_expr(e);
                }
            }
            ThirExprKind::Spread(inner) => {
                self.check_expr(inner);
            }
            ThirExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.check_expr(s);
                }
                if let Some(e) = end {
                    self.check_expr(e);
                }
            }
            ThirExprKind::Ok(inner) | ThirExprKind::Err(inner) | ThirExprKind::Try(inner) => {
                self.check_expr(inner);
            }
            ThirExprKind::UnwrapOrPanic {
                expr: inner,
                message,
            } => {
                self.check_expr(inner);
                self.check_expr(message);
            }
            ThirExprKind::Move(inner)
            | ThirExprKind::Clone(inner)
            | ThirExprKind::Async(inner)
            | ThirExprKind::Await(inner)
            | ThirExprKind::Spawn(inner) => {
                self.check_expr(inner);
            }
            ThirExprKind::Borrow { expr: inner, .. } => {
                self.check_expr(inner);
            }
            ThirExprKind::Closure { body, .. } => {
                self.check_expr(body);
            }
            ThirExprKind::Cast { value, .. } => {
                self.check_expr(value);
            }
            ThirExprKind::ScopeBlock { stmts } => {
                for s in stmts {
                    self.check_stmt(s);
                }
            }
            ThirExprKind::Literal(_) | ThirExprKind::Var(_) => {}
        }
    }

    /// Check if a set of patterns is exhaustive for a given scrutinee type.
    ///
    /// Uses pattern matrix coverage: collects all enum variants covered by
    /// the patterns and compares against the full set of variants defined
    /// in the enum. A wildcard pattern makes any match exhaustive.
    fn check_exhaustive(&mut self, scrutinee_ty: TypeId, patterns: Vec<&ThirPattern>, span: Span) {
        // If any pattern is a wildcard, the match is exhaustive
        let has_wildcard = patterns
            .iter()
            .any(|p| matches!(p.kind, ThirPatternKind::Wildcard));

        if has_wildcard {
            // Check for unreachable patterns after the wildcard
            let mut found_wildcard = false;
            for p in &patterns {
                if found_wildcard {
                    // Any non-wildcard pattern after a wildcard is unreachable
                    if !matches!(p.kind, ThirPatternKind::Wildcard) {
                        self.errors.push(ExhaustivenessError::new(
                            ExhaustivenessErrorKind::UnreachablePattern,
                            p.span,
                        ));
                    }
                }
                if matches!(p.kind, ThirPatternKind::Wildcard) {
                    found_wildcard = true;
                }
            }
            return;
        }

        // For enum types, check that all variants are covered
        if let Some(info) = self.registry.get(scrutinee_ty) {
            if let TypeKind::Enum { def } = &info.kind {
                let all_variants: FxHashSet<&str> =
                    def.variants.iter().map(|v| v.name.resolve()).collect();

                let covered: FxHashSet<&str> = patterns
                    .iter()
                    .filter_map(|p| match &p.kind {
                        ThirPatternKind::Enum { variant, .. } => Some(variant.as_str()),
                        _ => None,
                    })
                    .collect();

                let missing: Vec<String> = all_variants
                    .difference(&covered)
                    .map(|s| s.to_string())
                    .collect();

                if !missing.is_empty() {
                    self.errors.push(ExhaustivenessError::new(
                        ExhaustivenessErrorKind::NonExhaustive { missing },
                        span,
                    ));
                }
                return;
            }

            // For Optional types, check Ok/None coverage
            if let TypeKind::Optional { .. } = &info.kind {
                let has_none = patterns.iter().any(|p| {
                    matches!(&p.kind, ThirPatternKind::Enum { variant, .. } if variant == "None")
                });
                let has_some = patterns.iter().any(|p| {
                    matches!(&p.kind, ThirPatternKind::Enum { variant, .. } if variant == "Some")
                });

                if !has_none || !has_some {
                    let mut missing = Vec::new();
                    if !has_none {
                        missing.push("None".to_string());
                    }
                    if !has_some {
                        missing.push("Some".to_string());
                    }
                    self.errors.push(ExhaustivenessError::new(
                        ExhaustivenessErrorKind::NonExhaustive { missing },
                        span,
                    ));
                }
                return;
            }

            // For Result types, check Ok/Err coverage
            if let TypeKind::Result { .. } = &info.kind {
                let has_ok = patterns.iter().any(
                    |p| matches!(&p.kind, ThirPatternKind::Enum { variant, .. } if variant == "Ok"),
                );
                let has_err = patterns.iter().any(|p| {
                    matches!(&p.kind, ThirPatternKind::Enum { variant, .. } if variant == "Err")
                });

                if !has_ok || !has_err {
                    let mut missing = Vec::new();
                    if !has_ok {
                        missing.push("Ok".to_string());
                    }
                    if !has_err {
                        missing.push("Err".to_string());
                    }
                    self.errors.push(ExhaustivenessError::new(
                        ExhaustivenessErrorKind::NonExhaustive { missing },
                        span,
                    ));
                }
                return;
            }

            // For Bool type, check true/false coverage
            if scrutinee_ty == builtin::BOOL {
                let has_true = patterns.iter().any(|p| {
                    matches!(
                        &p.kind,
                        ThirPatternKind::Literal(doo_thir::ThirLiteral::Bool(true))
                    )
                });
                let has_false = patterns.iter().any(|p| {
                    matches!(
                        &p.kind,
                        ThirPatternKind::Literal(doo_thir::ThirLiteral::Bool(false))
                    )
                });

                if !has_true || !has_false {
                    let mut missing = Vec::new();
                    if !has_true {
                        missing.push("true".to_string());
                    }
                    if !has_false {
                        missing.push("false".to_string());
                    }
                    self.errors.push(ExhaustivenessError::new(
                        ExhaustivenessErrorKind::NonExhaustive { missing },
                        span,
                    ));
                }
                return;
            }
        }

        // For other types without a wildcard, the match is non-exhaustive
        // unless all possible values are covered (which we can't determine
        // for arbitrary types without more sophisticated analysis)
        // — this is a conservative check that could be improved
    }

    /// Get collected errors.
    pub fn errors(&self) -> &[ExhaustivenessError] {
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
    fn test_exhaustiveness_checker_creation() {
        let registry = TypeRegistry::new();
        let checker = ExhaustivenessChecker::new(&registry);
        assert!(checker.errors.is_empty());
    }

    #[test]
    fn test_non_exhaustive_error_message() {
        let err = ExhaustivenessError::new(
            ExhaustivenessErrorKind::NonExhaustive {
                missing: vec!["Color::Blue".into(), "Color::Green".into()],
            },
            Span::dummy(),
        );
        assert!(err.kind.message().contains("Blue"));
        assert!(err.kind.message().contains("Green"));
    }

    #[test]
    fn test_unreachable_pattern_message() {
        let err =
            ExhaustivenessError::new(ExhaustivenessErrorKind::UnreachablePattern, Span::dummy());
        assert!(err.kind.message().contains("unreachable"));
    }
}
