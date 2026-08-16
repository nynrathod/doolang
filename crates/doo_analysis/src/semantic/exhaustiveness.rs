//! Match Exhaustiveness Checking
//!
//! Ensures all match expressions cover all possible patterns.
//!
//! ## Rules
//!
//! - **Bool**: Must cover `true` and `false`, or have a wildcard `_`
//! - **Enum**: Must cover all variants, or have a wildcard `_`
//! - **Int/Float/Str**: Infinite domain, requires wildcard `_` unless all literals
//!   are explicitly matched (which is impossible for infinite types)
//! - **Wildcard**: Always makes the match exhaustive
//!
//! ## Usage
//!
//! ```ignore
//! let checker = ExhaustivenessChecker::new(&type_registry);
//! let errors = checker.check_program(&hir_program);
//! ```

use doo_core::types::{builtin, TypeId, TypeKind, TypeRegistry};
use doo_core::Span;
use doo_hir::{
    ConstValue, HirExpr, HirExprKind, HirFunction, HirItem, HirMatchArm, HirMatchPattern,
    HirProgram, HirStmt, HirStmtKind,
};
use std::collections::HashSet;

// ============================================================================
// Error Types
// ============================================================================

/// Exhaustiveness error.
#[derive(Debug, Clone)]
pub struct ExhaustivenessError {
    pub kind: ExhaustivenessErrorKind,
    pub span: Span,
}

/// Kinds of exhaustiveness errors.
#[derive(Debug, Clone)]
pub enum ExhaustivenessErrorKind {
    /// Match is not exhaustive - missing cases.
    NonExhaustive {
        /// Description of missing patterns.
        missing: Vec<String>,
    },
    /// Unreachable pattern (after wildcard or duplicate).
    UnreachablePattern,
}

impl std::fmt::Display for ExhaustivenessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            ExhaustivenessErrorKind::NonExhaustive { missing } => {
                write!(f, "non-exhaustive match: missing {}", missing.join(", "))
            }
            ExhaustivenessErrorKind::UnreachablePattern => {
                write!(f, "unreachable pattern")
            }
        }
    }
}

// ============================================================================
// Covered Patterns Tracker
// ============================================================================

/// Tracks what patterns have been covered in a match expression.
#[derive(Debug, Default)]
struct CoveredPatterns {
    /// Has a wildcard pattern (covers everything).
    has_wildcard: bool,
    /// Covered boolean values.
    bool_values: HashSet<bool>,
    /// Covered enum variants (enum_name -> set of variant names).
    enum_variants: HashSet<String>,
    /// Covered literal values (for Int/Float/Str - stored as string representation).
    literals: HashSet<String>,
}

impl CoveredPatterns {
    fn new() -> Self {
        Self::default()
    }

    /// Mark that a wildcard was seen.
    fn add_wildcard(&mut self) {
        self.has_wildcard = true;
    }

    /// Mark a boolean value as covered.
    fn add_bool(&mut self, value: bool) {
        self.bool_values.insert(value);
    }

    /// Mark an enum variant as covered.
    fn add_enum_variant(&mut self, variant: &str) {
        self.enum_variants.insert(variant.to_string());
    }

    /// Mark a literal value as covered.
    fn add_literal(&mut self, repr: &str) {
        self.literals.insert(repr.to_string());
    }

    /// Check if the match is exhaustive for a given type.
    fn is_exhaustive(&self, type_id: TypeId, registry: &TypeRegistry) -> Result<(), Vec<String>> {
        // Wildcard covers everything
        if self.has_wildcard {
            return Ok(());
        }

        // Check based on type
        if type_id == builtin::BOOL {
            return self.check_bool_exhaustive();
        }

        // Check enums
        if let Some(info) = registry.get(type_id) {
            if let TypeKind::Enum { def } = &info.kind {
                let name = def.name.resolve();
                let variants: Vec<(String, Option<TypeId>)> = def
                    .variants
                    .iter()
                    .map(|v| (v.name.resolve().to_string(), v.payload))
                    .collect();
                return self.check_enum_exhaustive(name, &variants);
            }
        }

        // For Int/Float/Str and other types with infinite domains,
        // we require a wildcard pattern
        Err(vec!["_".to_string()])
    }

    /// Check if all boolean values are covered.
    fn check_bool_exhaustive(&self) -> Result<(), Vec<String>> {
        let mut missing = Vec::new();
        if !self.bool_values.contains(&true) {
            missing.push("true".to_string());
        }
        if !self.bool_values.contains(&false) {
            missing.push("false".to_string());
        }
        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }

    /// Check if all enum variants are covered.
    fn check_enum_exhaustive(
        &self,
        enum_name: &str,
        variants: &[(String, Option<TypeId>)],
    ) -> Result<(), Vec<String>> {
        let mut missing = Vec::new();
        for (variant_name, _) in variants {
            if !self.enum_variants.contains(variant_name) {
                missing.push(format!("{}::{}", enum_name, variant_name));
            }
        }
        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }
}

// ============================================================================
// Exhaustiveness Checker
// ============================================================================

/// Checks match expressions for exhaustiveness.
pub struct ExhaustivenessChecker<'a> {
    /// Type registry for looking up type information.
    registry: &'a TypeRegistry,
    /// Collected errors.
    errors: Vec<ExhaustivenessError>,
    /// Tracked local variable types.
    locals: std::collections::HashMap<String, TypeId>,
}

impl<'a> ExhaustivenessChecker<'a> {
    /// Create a new exhaustiveness checker.
    pub fn new(registry: &'a TypeRegistry) -> Self {
        Self {
            registry,
            errors: Vec::new(),
            locals: std::collections::HashMap::new(),
        }
    }

    /// Check an entire program for match exhaustiveness.
    pub fn check_program(&mut self, program: &HirProgram) -> Vec<ExhaustivenessError> {
        for item in &program.items {
            self.check_item(item);
        }
        std::mem::take(&mut self.errors)
    }

    /// Check a single item.
    fn check_item(&mut self, item: &HirItem) {
        match item {
            HirItem::Const(_) | HirItem::Static(_) => {}
            HirItem::Function(func) => self.check_function(func),
            HirItem::Struct(_) | HirItem::Enum(_) | HirItem::Import(_) | HirItem::Interface(_) => {}
        }
    }

    /// Infer the type of an expression when type_id is not set.
    /// This is a simplified inference that handles common cases.
    fn infer_expr_type(&self, expr: &HirExpr) -> Option<TypeId> {
        match &expr.kind {
            HirExprKind::Const(c) => Some(c.type_id()),
            HirExprKind::Local { name } => {
                // Look up from our tracked locals first, then fall back to expr.type_id
                self.locals.get(name).copied().or(expr.type_id)
            }
            HirExprKind::Index { object, .. } => {
                // For array/map indexing, get element/value type
                let obj_type = object.type_id.or_else(|| self.infer_expr_type(object))?;
                if let Some(info) = self.registry.get(obj_type) {
                    match &info.kind {
                        TypeKind::Array { element } => Some(*element),
                        TypeKind::Map { value, .. } => Some(*value),
                        TypeKind::Str => Some(builtin::STR),
                        TypeKind::Tuple { elements } => {
                            // Can't statically determine tuple element type without constant index
                            elements.first().copied()
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            }
            HirExprKind::Field { object, field } => {
                // For struct field access, get the field type
                let obj_type = object.type_id.or_else(|| self.infer_expr_type(object))?;
                if let Some(info) = self.registry.get(obj_type) {
                    if let TypeKind::Struct { def, .. } = &info.kind {
                        return def
                            .fields
                            .iter()
                            .find(|f| f.name.resolve() == field)
                            .map(|f| f.type_id);
                    }
                }
                None
            }
            HirExprKind::MethodCall {
                receiver, method, ..
            } => {
                // For method calls, try to infer return type from common methods
                let recv_type = receiver
                    .type_id
                    .or_else(|| self.infer_expr_type(receiver))?;
                self.infer_method_return_type(recv_type, method)
            }
            _ => None,
        }
    }

    /// Infer return type for common methods.
    fn infer_method_return_type(&self, receiver_type: TypeId, method: &str) -> Option<TypeId> {
        if let Some(info) = self.registry.get(receiver_type) {
            match &info.kind {
                TypeKind::Array { element } => match method {
                    "get" | "first" | "last" | "pop" | "shift" => Some(*element),
                    "len" => Some(builtin::INT),
                    "isEmpty" => Some(builtin::BOOL),
                    "contains" => Some(builtin::BOOL),
                    _ => None,
                },
                TypeKind::Map { value, .. } => match method {
                    "get" => Some(*value),
                    "len" => Some(builtin::INT),
                    "isEmpty" | "has" | "containsKey" | "containsValue" => Some(builtin::BOOL),
                    _ => None,
                },
                TypeKind::Str => match method {
                    "len" | "indexOf" | "charCode" => Some(builtin::INT),
                    "isEmpty" | "contains" | "startsWith" | "endsWith" => Some(builtin::BOOL),
                    "trim" | "toUpperCase" | "toLowerCase" | "substr" | "replace" => {
                        Some(builtin::STR)
                    }
                    "charAt" => Some(builtin::STR),
                    _ => None,
                },
                _ => None,
            }
        } else {
            None
        }
    }

    /// Check a function.
    fn check_function(&mut self, func: &HirFunction) {
        // Clear locals for new function
        self.locals.clear();

        // Register function parameters
        for param in &func.params {
            if let Some(type_id) = param.type_id {
                self.locals.insert(param.name.clone(), type_id);
            }
        }

        for stmt in &func.body {
            self.check_stmt(stmt);
        }
    }

    /// Check a statement.
    fn check_stmt(&mut self, stmt: &HirStmt) {
        match &stmt.kind {
            HirStmtKind::Let { name, value, .. } => {
                self.check_expr(value);
                // Track the local variable type for exhaustiveness checking
                if let Some(type_id) = value.type_id.or_else(|| self.infer_expr_type(value)) {
                    self.locals.insert(name.clone(), type_id);
                }
            }
            HirStmtKind::TupleLet { names, value, .. } => {
                self.check_expr(value);
                // Track tuple element types if we can infer them
                if let Some(type_id) = value.type_id.or_else(|| self.infer_expr_type(value)) {
                    if let Some(info) = self.registry.get(type_id) {
                        if let TypeKind::Tuple { elements } = &info.kind {
                            for (name, elem_type) in names.iter().zip(elements.iter()) {
                                self.locals.insert(name.clone(), *elem_type);
                            }
                        }
                    }
                }
            }
            HirStmtKind::Assign { target, value } => {
                self.check_expr(target);
                self.check_expr(value);
            }
            HirStmtKind::Expr(expr) => {
                self.check_expr(expr);
            }
            HirStmtKind::Return(exprs) => {
                for expr in exprs {
                    self.check_expr(expr);
                }
            }
            HirStmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.check_expr(condition);
                for s in then_block {
                    self.check_stmt(s);
                }
                if let Some(else_stmts) = else_block {
                    for s in else_stmts {
                        self.check_stmt(s);
                    }
                }
            }
            HirStmtKind::While {
                condition,
                body,
                increment,
            } => {
                self.check_expr(condition);
                for s in body {
                    self.check_stmt(s);
                }
                for s in increment {
                    self.check_stmt(s);
                }
            }
            HirStmtKind::ManualErrorExtract { expr, .. } => {
                self.check_expr(expr);
            }
            HirStmtKind::Break | HirStmtKind::Continue | HirStmtKind::Drop { .. } => {}
        }
    }

    /// Check an expression, including match expressions.
    fn check_expr(&mut self, expr: &HirExpr) {
        match &expr.kind {
            HirExprKind::Match { values, arms } => {
                // For each matched value, check exhaustiveness
                // Currently we only support single-value matches
                if values.len() == 1 {
                    // Get the matched type - infer if not already set
                    let matched_type = values[0]
                        .type_id
                        .or_else(|| self.infer_expr_type(&values[0]))
                        .unwrap_or(builtin::ANY);
                    self.check_match_exhaustive(matched_type, arms, expr.span);
                }
                // Recurse into arm bodies
                for arm in arms {
                    self.check_expr(&arm.body);
                    if let Some(guard) = &arm.guard {
                        self.check_expr(guard);
                    }
                }
            }

            // Recurse into sub-expressions
            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.check_expr(lhs);
                self.check_expr(rhs);
            }
            HirExprKind::UnaryOp { operand, .. } => {
                self.check_expr(operand);
            }
            HirExprKind::Call { func, args } => {
                self.check_expr(func);
                for arg in args {
                    self.check_expr(arg);
                }
            }
            HirExprKind::MethodCall { receiver, args, .. } => {
                self.check_expr(receiver);
                for arg in args {
                    self.check_expr(arg);
                }
            }
            HirExprKind::Field { object, .. } => {
                self.check_expr(object);
            }
            HirExprKind::Index { object, index } => {
                self.check_expr(object);
                self.check_expr(index);
            }
            HirExprKind::Array(elements) => {
                for elem in elements {
                    self.check_expr(elem);
                }
            }
            HirExprKind::Map(entries) => {
                for (k, v) in entries {
                    self.check_expr(k);
                    self.check_expr(v);
                }
            }
            HirExprKind::Tuple(elements) => {
                for elem in elements {
                    self.check_expr(elem);
                }
            }
            HirExprKind::Struct { fields, .. } => {
                for (_, v) in fields {
                    self.check_expr(v);
                }
            }
            HirExprKind::EnumVariant { payload, .. } => {
                for p in payload {
                    self.check_expr(p);
                }
            }
            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.check_expr(condition);
                self.check_expr(then_expr);
                if let Some(e) = else_expr {
                    self.check_expr(e);
                }
            }
            HirExprKind::Block { stmts, expr } => {
                for s in stmts {
                    self.check_stmt(s);
                }
                if let Some(e) = expr {
                    self.check_expr(e);
                }
            }
            HirExprKind::Range { start, end, .. } => {
                self.check_expr(start);
                self.check_expr(end);
            }
            HirExprKind::Ok(e) | HirExprKind::Err(e) | HirExprKind::Try(e) => {
                self.check_expr(e);
            }
            HirExprKind::UnwrapOrPanic { expr, message } => {
                self.check_expr(expr);
                self.check_expr(message);
            }
            HirExprKind::Cast { value, .. } => {
                self.check_expr(value);
            }
            HirExprKind::Closure { body, .. } => {
                self.check_expr(body);
            }
            HirExprKind::Spread(inner) => {
                self.check_expr(inner);
            }
            HirExprKind::Move(inner) | HirExprKind::Clone(inner) => {
                self.check_expr(inner);
            }
            HirExprKind::Borrow { expr, .. } => {
                self.check_expr(expr);
            }

            // Async & concurrency
            HirExprKind::Await(inner) | HirExprKind::Spawn { body: inner } => {
                self.check_expr(inner);
            }
            HirExprKind::ScopeBlock { stmts } => {
                for s in stmts {
                    self.check_stmt(s);
                }
            }

            // Leaf expressions - no recursion needed
            HirExprKind::Const(_) | HirExprKind::Local { .. } | HirExprKind::Global { .. } => {}
        }
    }

    /// Check if a match expression is exhaustive.
    fn check_match_exhaustive(&mut self, matched_type: TypeId, arms: &[HirMatchArm], span: Span) {
        let mut covered = CoveredPatterns::new();
        let mut has_wildcard = false;

        // If the matched type is unknown/ANY, try to infer it from enum variant patterns
        let effective_type = if matched_type == builtin::ANY {
            // Look for enum variant patterns and extract the enum name
            let enum_name = arms.iter().find_map(|arm| match &arm.pattern {
                HirMatchPattern::EnumVariant { enum_name, .. }
                | HirMatchPattern::EnumVariantPayload { enum_name, .. } => Some(enum_name.clone()),
                _ => None,
            });
            if let Some(ref name) = enum_name {
                // Look up the enum type in the registry
                self.registry.lookup(name).unwrap_or(matched_type)
            } else {
                matched_type
            }
        } else {
            matched_type
        };

        for arm in arms {
            // If we already have a wildcard, subsequent patterns are unreachable
            if has_wildcard {
                self.errors.push(ExhaustivenessError {
                    kind: ExhaustivenessErrorKind::UnreachablePattern,
                    span: arm.span,
                });
                continue;
            }

            // Process the pattern
            match &arm.pattern {
                HirMatchPattern::Wildcard => {
                    covered.add_wildcard();
                    has_wildcard = true;
                }
                HirMatchPattern::Literal(expr) => {
                    self.process_literal_pattern(&mut covered, expr, matched_type);
                }
                HirMatchPattern::Condition(_) => {
                    // Condition patterns don't contribute to exhaustiveness
                    // unless they're trivially true (which we don't analyze)
                }
                HirMatchPattern::EnumVariant { variant, .. } => {
                    covered.add_enum_variant(variant);
                }
                HirMatchPattern::EnumVariantPayload { variant, .. } => {
                    covered.add_enum_variant(variant);
                }
                HirMatchPattern::Tuple(parts) => {
                    // Check if any part is a wildcard
                    if parts.iter().any(|p| matches!(p, HirMatchPattern::Wildcard)) {
                        covered.add_wildcard();
                        has_wildcard = true;
                    }
                }
            }
        }

        // Check exhaustiveness
        if let Err(missing) = covered.is_exhaustive(effective_type, self.registry) {
            self.errors.push(ExhaustivenessError {
                kind: ExhaustivenessErrorKind::NonExhaustive { missing },
                span,
            });
        }
    }

    /// Process a literal pattern and add it to covered patterns.
    fn process_literal_pattern(
        &self,
        covered: &mut CoveredPatterns,
        expr: &HirExpr,
        _matched_type: TypeId,
    ) {
        match &expr.kind {
            HirExprKind::Const(c) => match c {
                ConstValue::Bool(b) => covered.add_bool(*b),
                ConstValue::Int(i) => covered.add_literal(&i.to_string()),
                ConstValue::Float(f) => covered.add_literal(&f.to_string()),
                ConstValue::Str(s) => covered.add_literal(s),
                ConstValue::Nil => covered.add_literal("nil"),
            },
            _ => {
                // Non-constant literal patterns don't contribute to exhaustiveness
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

    #[test]
    fn test_covered_patterns_bool() {
        let mut covered = CoveredPatterns::new();

        // Missing both
        let registry = TypeRegistry::new();
        assert!(covered.is_exhaustive(builtin::BOOL, &registry).is_err());

        // Add true
        covered.add_bool(true);
        assert!(covered.is_exhaustive(builtin::BOOL, &registry).is_err());

        // Add false - now exhaustive
        covered.add_bool(false);
        assert!(covered.is_exhaustive(builtin::BOOL, &registry).is_ok());
    }

    #[test]
    fn test_covered_patterns_wildcard() {
        let mut covered = CoveredPatterns::new();
        let registry = TypeRegistry::new();

        // Wildcard covers everything
        covered.add_wildcard();
        assert!(covered.is_exhaustive(builtin::BOOL, &registry).is_ok());
        assert!(covered.is_exhaustive(builtin::INT, &registry).is_ok());
        assert!(covered.is_exhaustive(builtin::STR, &registry).is_ok());
    }

    #[test]
    fn test_covered_patterns_int_needs_wildcard() {
        let covered = CoveredPatterns::new();
        let registry = TypeRegistry::new();

        // Int without wildcard is not exhaustive
        assert!(covered.is_exhaustive(builtin::INT, &registry).is_err());
    }
}
