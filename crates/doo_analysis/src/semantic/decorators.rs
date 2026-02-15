//! Decorator Validation — Single Source of Truth
//!
//! All decorator rules, type checks, value validation, combination conflicts,
//! and HIR walking live here. compile.rs just calls `validate_program()`.
//!
//! ## Supported Decorators:
//! - Validation: `@email`, `@url`, `@min(n)`, `@max(n)`, `@pattern(regex)`
//! - Database: `@primary`, `@autoIncrement`, `@auto`, `@unique`, `@foreign(Struct)`
//! - Security: `@hash`
//! - Visibility: `@readOnly`, `@writeOnly`, `@internal`
//! - Default: `@default(value)`, `@optional`
//! - Timestamp: `@autoTimestamp` (struct-level only)
//! - HTTP: `@redirect`

use doo_core::errors::codes::{CompilerError, ErrorCode};
use doo_core::types::{builtin, TypeId, TypeKind, TypeRegistry};
use doo_core::Span;
use doo_hir::{
    ConstValue, HirDecorator, HirExpr, HirExprKind, HirItem, HirProgram, HirStmt, HirStmtKind,
};
use std::collections::HashMap;

/// Known decorator kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecoratorKind {
    Email,
    Url,
    Min,
    Max,
    Foreign,
    Unique,
    Primary,
    AutoIncrement,
    Auto,
    Hash,
    Optional,
    Default,
    Pattern,
    WriteOnly,
    ReadOnly,
    Internal,
    AutoTimestamp,
    Redirect,
    Unknown(String),
}

impl DecoratorKind {
    pub fn from_name(name: &str) -> Self {
        match name {
            "email" => Self::Email,
            "url" => Self::Url,
            "min" => Self::Min,
            "max" => Self::Max,
            "foreign" => Self::Foreign,
            "unique" => Self::Unique,
            "primary" => Self::Primary,
            "autoIncrement" => Self::AutoIncrement,
            "auto" => Self::Auto,
            "hash" => Self::Hash,
            "optional" => Self::Optional,
            "default" => Self::Default,
            "pattern" => Self::Pattern,
            "writeOnly" => Self::WriteOnly,
            "readOnly" => Self::ReadOnly,
            "internal" => Self::Internal,
            "autoTimestamp" => Self::AutoTimestamp,
            "redirect" => Self::Redirect,
            other => Self::Unknown(other.to_string()),
        }
    }

    /// Data-driven validation rule for this decorator.
    fn rule(&self) -> DecoratorRule {
        match self {
            // Str-only, no args
            Self::Email | Self::Url | Self::Hash | Self::Redirect => DecoratorRule {
                type_constraint: TypeConstraint::StrOnly,
                args: ArgRule::None,
            },
            // Str-only, exactly 1 arg
            Self::Pattern => DecoratorRule {
                type_constraint: TypeConstraint::StrOnly,
                args: ArgRule::Exactly(1),
            },
            // Int-only, no args
            Self::AutoIncrement | Self::Auto => DecoratorRule {
                type_constraint: TypeConstraint::IntOnly,
                args: ArgRule::None,
            },
            // Int-only, exactly 1 arg
            Self::Foreign => DecoratorRule {
                type_constraint: TypeConstraint::IntOnly,
                args: ArgRule::Exactly(1),
            },
            // Str/Int/Float, exactly 1 arg
            Self::Min | Self::Max => DecoratorRule {
                type_constraint: TypeConstraint::StrOrNumeric,
                args: ArgRule::Exactly(1),
            },
            // Any type, no args
            Self::Unique
            | Self::Primary
            | Self::Optional
            | Self::WriteOnly
            | Self::ReadOnly
            | Self::Internal => DecoratorRule {
                type_constraint: TypeConstraint::Any,
                args: ArgRule::None,
            },
            // Any type, exactly 1 arg
            Self::Default => DecoratorRule {
                type_constraint: TypeConstraint::Any,
                args: ArgRule::Exactly(1),
            },
            // Struct-level only
            Self::AutoTimestamp => DecoratorRule {
                type_constraint: TypeConstraint::StructLevel,
                args: ArgRule::None,
            },
            // Unknown
            Self::Unknown(_) => DecoratorRule {
                type_constraint: TypeConstraint::Any,
                args: ArgRule::Any,
            },
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Email => "email",
            Self::Url => "url",
            Self::Min => "min",
            Self::Max => "max",
            Self::Foreign => "foreign",
            Self::Unique => "unique",
            Self::Primary => "primary",
            Self::AutoIncrement => "autoIncrement",
            Self::Auto => "auto",
            Self::Hash => "hash",
            Self::Optional => "optional",
            Self::Default => "default",
            Self::Pattern => "pattern",
            Self::WriteOnly => "writeOnly",
            Self::ReadOnly => "readOnly",
            Self::Internal => "internal",
            Self::AutoTimestamp => "autoTimestamp",
            Self::Redirect => "redirect",
            Self::Unknown(n) => n,
        }
    }
}

struct DecoratorRule {
    type_constraint: TypeConstraint,
    args: ArgRule,
}

enum TypeConstraint {
    StrOnly,
    IntOnly,
    StrOrNumeric,
    Any,
    StructLevel,
}

enum ArgRule {
    None,
    Exactly(usize),
    Any,
}

impl TypeConstraint {
    fn expected_str(&self) -> &str {
        match self {
            Self::StrOnly => "Str",
            Self::IntOnly => "Int",
            Self::StrOrNumeric => "Str, Int, or Float",
            Self::Any | Self::StructLevel => "any",
        }
    }
}

/// Decorator validation error.
#[derive(Debug, Clone)]
pub enum DecoratorError {
    InvalidType {
        decorator: String,
        field: String,
        struct_name: String,
        expected: String,
        found: String,
    },
    InvalidArgs {
        decorator: String,
        field: String,
        message: String,
    },
    Unknown {
        decorator: String,
        field: String,
        struct_name: String,
    },
    Conflict {
        decorator1: String,
        decorator2: String,
        field: String,
        struct_name: String,
        reason: String,
    },
    InvalidOptional {
        decorator: String,
        field: String,
        struct_name: String,
        reason: String,
    },
    ValueViolation {
        decorator: String,
        field: String,
        struct_name: String,
        message: String,
    },
}

impl std::fmt::Display for DecoratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidType {
                decorator,
                field,
                struct_name,
                expected,
                found,
            } => write!(
                f,
                "@{} on {}.{} requires type {}, found {}",
                decorator, struct_name, field, expected, found
            ),
            Self::InvalidArgs {
                decorator,
                field,
                message,
            } => write!(f, "@{} on {}: {}", decorator, field, message),
            Self::Unknown {
                decorator,
                field,
                struct_name,
            } => write!(
                f,
                "Unknown decorator @{} on {}.{}",
                decorator, struct_name, field
            ),
            Self::Conflict {
                decorator1,
                decorator2,
                field,
                struct_name,
                reason,
            } => write!(
                f,
                "@{} and @{} conflict on {}.{}: {}",
                decorator1, decorator2, struct_name, field, reason
            ),
            Self::InvalidOptional {
                decorator,
                field,
                struct_name,
                reason,
            } => write!(
                f,
                "@{} on optional field {}.{}: {}",
                decorator, struct_name, field, reason
            ),
            Self::ValueViolation {
                decorator,
                field,
                struct_name,
                message,
            } => write!(
                f,
                "@{} on {}.{}: {}",
                decorator, struct_name, field, message
            ),
        }
    }
}

impl std::error::Error for DecoratorError {}

/// Single source of truth: DecoratorError → CompilerError conversion.
pub fn to_compiler_error(err: &DecoratorError, span: Span) -> CompilerError {
    match err {
        DecoratorError::InvalidType { decorator, field, struct_name, expected, found } =>
            CompilerError::new(ErrorCode::InvalidDecorator, format!("@{} requires type {}, found {}", decorator, expected, found), span)
                .with_suggestion(format!("change {}.{} type to {}", struct_name, field, expected)),
        DecoratorError::InvalidArgs { decorator, message, .. } =>
            CompilerError::new(ErrorCode::InvalidDecorator, format!("@{}: {}", decorator, message), span),
        DecoratorError::Unknown { decorator, .. } =>
            CompilerError::new(ErrorCode::InvalidDecorator, format!("unknown decorator @{}", decorator), span)
                .with_suggestion("valid: @email @url @min @max @primary @unique @hash @optional @default @pattern @writeOnly @readOnly @internal"),
        DecoratorError::Conflict { decorator1, decorator2, reason, .. } =>
            CompilerError::new(ErrorCode::ConflictingDecorators, format!("@{} and @{} conflict: {}", decorator1, decorator2, reason), span),
        DecoratorError::InvalidOptional { decorator, reason, .. } =>
            CompilerError::new(ErrorCode::InvalidDecorator, format!("@{} on optional field: {}", decorator, reason), span),
        DecoratorError::ValueViolation { decorator, message, .. } =>
            CompilerError::new(ErrorCode::InvalidDecorator, format!("@{}: {}", decorator, message), span),
    }
}

/// Decorator validator — centralized validation for the entire program.
pub struct DecoratorValidator<'a> {
    type_registry: &'a TypeRegistry,
}

impl<'a> DecoratorValidator<'a> {
    pub fn new(type_registry: &'a TypeRegistry) -> Self {
        Self { type_registry }
    }

    // ── Type Helpers ────────────────────────────────────────────────────

    fn is_str(&self, id: TypeId) -> bool {
        match self.type_registry.get(id).map(|t| &t.kind) {
            Some(TypeKind::Str) => true,
            Some(TypeKind::Optional { inner }) => self.is_str(*inner),
            _ => id == builtin::STR,
        }
    }

    fn is_int(&self, id: TypeId) -> bool {
        match self.type_registry.get(id).map(|t| &t.kind) {
            Some(TypeKind::Int) => true,
            Some(TypeKind::Optional { inner }) => self.is_int(*inner),
            _ => id == builtin::INT,
        }
    }

    fn is_str_or_numeric(&self, id: TypeId) -> bool {
        match self.type_registry.get(id).map(|t| &t.kind) {
            Some(TypeKind::Str) | Some(TypeKind::Int) | Some(TypeKind::Float) => true,
            Some(TypeKind::Optional { inner }) => self.is_str_or_numeric(*inner),
            _ => id == builtin::STR || id == builtin::INT || id == builtin::FLOAT,
        }
    }

    fn type_name(&self, id: TypeId) -> String {
        self.type_registry
            .get(id)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| format!("Type#{}", id.0))
    }

    fn check_type_constraint(&self, constraint: &TypeConstraint, type_id: TypeId) -> bool {
        match constraint {
            TypeConstraint::StrOnly => self.is_str(type_id),
            TypeConstraint::IntOnly => self.is_int(type_id),
            TypeConstraint::StrOrNumeric => self.is_str_or_numeric(type_id),
            TypeConstraint::Any => true,
            TypeConstraint::StructLevel => false,
        }
    }

    // ── Single Decorator Validation (data-driven) ──────────────────────

    fn validate_decorator(
        &self,
        dec: &HirDecorator,
        field_type: TypeId,
        field: &str,
        sname: &str,
    ) -> Result<(), DecoratorError> {
        let kind = DecoratorKind::from_name(&dec.name);

        if let DecoratorKind::Unknown(ref n) = kind {
            return Err(DecoratorError::Unknown {
                decorator: n.clone(),
                field: field.into(),
                struct_name: sname.into(),
            });
        }

        let rule = kind.rule();
        let dname = kind.name().to_string();

        // Type constraint
        if !self.check_type_constraint(&rule.type_constraint, field_type) {
            if matches!(rule.type_constraint, TypeConstraint::StructLevel) {
                return Err(DecoratorError::InvalidArgs {
                    decorator: dname,
                    field: field.into(),
                    message: "is a struct-level decorator, not a field decorator".into(),
                });
            }
            return Err(DecoratorError::InvalidType {
                decorator: dname,
                field: field.into(),
                struct_name: sname.into(),
                expected: rule.type_constraint.expected_str().into(),
                found: self.type_name(field_type),
            });
        }

        // Arg count
        match rule.args {
            ArgRule::None if !dec.args.is_empty() => {
                return Err(DecoratorError::InvalidArgs {
                    decorator: dname,
                    field: field.into(),
                    message: "takes no arguments".into(),
                })
            }
            ArgRule::Exactly(n) if dec.args.len() != n => {
                return Err(DecoratorError::InvalidArgs {
                    decorator: dname,
                    field: field.into(),
                    message: format!("requires exactly {} argument(s)", n),
                })
            }
            _ => {}
        }

        Ok(())
    }

    // ── Combination Conflicts ──────────────────────────────────────────

    fn validate_combinations(
        &self,
        decorators: &[HirDecorator],
        is_optional: bool,
        field: &str,
        sname: &str,
    ) -> Vec<DecoratorError> {
        let kinds: Vec<DecoratorKind> = decorators
            .iter()
            .map(|d| DecoratorKind::from_name(&d.name))
            .collect();
        let has = |k: &DecoratorKind| kinds.contains(k);
        let has_auto = has(&DecoratorKind::Auto) || has(&DecoratorKind::AutoIncrement);
        let mut errs = Vec::new();

        // Conflict table: (condition, decorator1, decorator2, reason)
        let conflicts: &[(bool, &str, &str, &str)] = &[
            (
                has(&DecoratorKind::Internal) && has(&DecoratorKind::WriteOnly),
                "internal",
                "writeOnly",
                "@writeOnly requires field in request, but @internal excludes it",
            ),
            (
                has(&DecoratorKind::WriteOnly) && has(&DecoratorKind::ReadOnly),
                "writeOnly",
                "readOnly",
                "field cannot be both request-only and response-only",
            ),
            (
                has_auto && has(&DecoratorKind::WriteOnly),
                "auto",
                "writeOnly",
                "@auto fields are server-generated and cannot accept input",
            ),
            (
                has(&DecoratorKind::Redirect) && !has(&DecoratorKind::Url),
                "redirect",
                "url",
                "@redirect requires @url on the same field",
            ),
        ];

        for (cond, d1, d2, reason) in conflicts {
            if *cond {
                errs.push(DecoratorError::Conflict {
                    decorator1: d1.to_string(),
                    decorator2: d2.to_string(),
                    field: field.into(),
                    struct_name: sname.into(),
                    reason: reason.to_string(),
                });
            }
        }

        if has(&DecoratorKind::Internal) && is_optional {
            errs.push(DecoratorError::InvalidOptional {
                decorator: "internal".into(),
                field: field.into(),
                struct_name: sname.into(),
                reason: "optional marker '?' is meaningless for internal fields".into(),
            });
        }

        errs
    }

    // ── Compile-Time Value Validation ──────────────────────────────────

    fn extract_numeric_arg(dec: &HirDecorator) -> Option<f64> {
        dec.args.first().and_then(|a| match &a.kind {
            HirExprKind::Const(ConstValue::Int(v)) => Some(*v as f64),
            HirExprKind::Const(ConstValue::Float(v)) => Some(*v),
            _ => None,
        })
    }

    fn validate_const_value(
        decorators: &[HirDecorator],
        value: &ConstValue,
        field: &str,
        sname: &str,
    ) -> Vec<DecoratorError> {
        let mut errs = Vec::new();
        for dec in decorators {
            match DecoratorKind::from_name(&dec.name) {
                DecoratorKind::Email => {
                    if let ConstValue::Str(s) = value {
                        if !Self::is_valid_email(s) {
                            errs.push(DecoratorError::ValueViolation {
                                decorator: "email".into(),
                                field: field.into(),
                                struct_name: sname.into(),
                                message: format!("invalid email \"{}\"", s),
                            });
                        }
                    }
                }
                DecoratorKind::Url => {
                    if let ConstValue::Str(s) = value {
                        if !Self::is_valid_url(s) {
                            errs.push(DecoratorError::ValueViolation {
                                decorator: "url".into(),
                                field: field.into(),
                                struct_name: sname.into(),
                                message: format!("invalid URL \"{}\"", s),
                            });
                        }
                    }
                }
                DecoratorKind::Min => {
                    Self::check_min_max(dec, value, field, sname, true, &mut errs)
                }
                DecoratorKind::Max => {
                    Self::check_min_max(dec, value, field, sname, false, &mut errs)
                }
                _ => {}
            }
        }
        errs
    }

    /// Shared min/max check — avoids duplicating logic for min vs max.
    fn check_min_max(
        dec: &HirDecorator,
        value: &ConstValue,
        field: &str,
        sname: &str,
        is_min: bool,
        errs: &mut Vec<DecoratorError>,
    ) {
        let Some(limit) = Self::extract_numeric_arg(dec) else {
            return;
        };
        let name = if is_min { "min" } else { "max" };

        let violation = match value {
            ConstValue::Int(v) => {
                if is_min {
                    (*v as f64) < limit
                } else {
                    (*v as f64) > limit
                }
            }
            ConstValue::Float(v) => {
                if is_min {
                    *v < limit
                } else {
                    *v > limit
                }
            }
            ConstValue::Str(s) => {
                let l = s.len() as f64;
                if is_min {
                    l < limit
                } else {
                    l > limit
                }
            }
            _ => return,
        };

        if violation {
            let op = if is_min { "<" } else { ">" };
            let msg = match value {
                ConstValue::Int(v) => format!("{} {} {}", v, op, limit as i64),
                ConstValue::Float(v) => format!("{} {} {}", v, op, limit),
                ConstValue::Str(s) => format!("length {} {} {}", s.len(), op, limit as i64),
                _ => unreachable!(),
            };
            errs.push(DecoratorError::ValueViolation {
                decorator: name.into(),
                field: field.into(),
                struct_name: sname.into(),
                message: msg,
            });
        }
    }

    fn is_valid_email(s: &str) -> bool {
        let parts: Vec<&str> = s.splitn(2, '@').collect();
        if parts.len() != 2 {
            return false;
        }
        let (local, domain) = (parts[0], parts[1]);
        !local.is_empty()
            && !domain.is_empty()
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
    }

    fn is_valid_url(s: &str) -> bool {
        s.starts_with("http://") || s.starts_with("https://")
    }

    // ── Public API: validate_program() — the ONLY method compile.rs calls

    pub fn validate_program(&self, hir: &HirProgram) -> Vec<CompilerError> {
        let mut errors = Vec::new();
        let mut struct_defs: HashMap<String, Vec<FieldInfo>> = HashMap::new();

        // Phase A: Validate decorator type/args/conflicts on struct field definitions
        for item in &hir.items {
            if let HirItem::Struct(s) = item {
                let mut field_infos = Vec::new();
                for field in &s.fields {
                    field_infos.push(FieldInfo {
                        name: field.name.clone(),
                        decorators: field.decorators.clone(),
                    });
                    if field.decorators.is_empty() {
                        continue;
                    }

                    let ftype = field.type_id.unwrap_or(builtin::ANY);

                    // Individual decorator checks
                    for dec in &field.decorators {
                        if let Err(e) = self.validate_decorator(dec, ftype, &field.name, &s.name) {
                            errors.push(to_compiler_error(&e, dec.span));
                        }
                    }

                    // Combination conflict checks
                    let dec_span = field
                        .decorators
                        .first()
                        .map(|d| d.span)
                        .unwrap_or(field.span);
                    for e in self.validate_combinations(
                        &field.decorators,
                        field.is_optional,
                        &field.name,
                        &s.name,
                    ) {
                        errors.push(to_compiler_error(&e, dec_span));
                    }
                }
                struct_defs.insert(s.name.clone(), field_infos);
            }
        }

        // Phase B: Walk function bodies — validate struct literal constant values
        for item in &hir.items {
            if let HirItem::Function(f) = item {
                Self::walk_stmts(&f.body, &struct_defs, &mut errors);
            }
        }

        errors
    }

    // ── HIR Walker ─────────────────────────────────────────────────────

    fn walk_stmts(
        stmts: &[HirStmt],
        defs: &HashMap<String, Vec<FieldInfo>>,
        errs: &mut Vec<CompilerError>,
    ) {
        for stmt in stmts {
            match &stmt.kind {
                HirStmtKind::Let { value, .. }
                | HirStmtKind::Assign { value, .. }
                | HirStmtKind::TupleLet { value, .. }
                | HirStmtKind::ManualErrorExtract { expr: value, .. } => {
                    Self::walk_expr(value, defs, errs)
                }
                HirStmtKind::Expr(e) => Self::walk_expr(e, defs, errs),
                HirStmtKind::Return(exprs) => {
                    for e in exprs {
                        Self::walk_expr(e, defs, errs);
                    }
                }
                HirStmtKind::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    Self::walk_expr(condition, defs, errs);
                    Self::walk_stmts(then_block, defs, errs);
                    if let Some(eb) = else_block {
                        Self::walk_stmts(eb, defs, errs);
                    }
                }
                HirStmtKind::While {
                    condition,
                    body,
                    increment,
                } => {
                    Self::walk_expr(condition, defs, errs);
                    Self::walk_stmts(body, defs, errs);
                    Self::walk_stmts(increment, defs, errs);
                }
                _ => {}
            }
        }
    }

    fn walk_expr(
        expr: &HirExpr,
        defs: &HashMap<String, Vec<FieldInfo>>,
        errs: &mut Vec<CompilerError>,
    ) {
        match &expr.kind {
            HirExprKind::Struct { name, fields } => {
                if let Some(def_fields) = defs.get(name) {
                    for (fname, fvalue) in fields {
                        if let Some(info) = def_fields.iter().find(|f| f.name == *fname) {
                            if !info.decorators.is_empty() {
                                if let HirExprKind::Const(cv) = &fvalue.kind {
                                    for e in Self::validate_const_value(
                                        &info.decorators,
                                        cv,
                                        fname,
                                        name,
                                    ) {
                                        errs.push(to_compiler_error(&e, fvalue.span));
                                    }
                                }
                            }
                        }
                    }
                }
                for (_, fv) in fields {
                    Self::walk_expr(fv, defs, errs);
                }
            }
            HirExprKind::Call { func, args }
            | HirExprKind::MethodCall {
                receiver: func,
                args,
                ..
            } => {
                Self::walk_expr(func, defs, errs);
                for a in args {
                    Self::walk_expr(a, defs, errs);
                }
            }
            HirExprKind::BinOp { lhs, rhs, .. }
            | HirExprKind::Index {
                object: lhs,
                index: rhs,
            }
            | HirExprKind::Range {
                start: lhs,
                end: rhs,
                ..
            }
            | HirExprKind::UnwrapOrPanic {
                expr: lhs,
                message: rhs,
            } => {
                Self::walk_expr(lhs, defs, errs);
                Self::walk_expr(rhs, defs, errs);
            }
            HirExprKind::UnaryOp { operand: e, .. }
            | HirExprKind::Field { object: e, .. }
            | HirExprKind::Ok(e)
            | HirExprKind::Err(e)
            | HirExprKind::Try(e)
            | HirExprKind::Spread(e)
            | HirExprKind::Move(e)
            | HirExprKind::Clone(e)
            | HirExprKind::Borrow { expr: e, .. }
            | HirExprKind::Cast { value: e, .. }
            | HirExprKind::Closure { body: e, .. } => {
                Self::walk_expr(e, defs, errs);
            }
            HirExprKind::Array(els)
            | HirExprKind::Tuple(els)
            | HirExprKind::RouteBlock { routes: els } => {
                for e in els {
                    Self::walk_expr(e, defs, errs);
                }
            }
            HirExprKind::EnumVariant { payload, .. } => {
                for p in payload {
                    Self::walk_expr(p, defs, errs);
                }
            }
            HirExprKind::Map(pairs) => {
                for (k, v) in pairs {
                    Self::walk_expr(k, defs, errs);
                    Self::walk_expr(v, defs, errs);
                }
            }
            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::walk_expr(condition, defs, errs);
                Self::walk_expr(then_expr, defs, errs);
                if let Some(e) = else_expr {
                    Self::walk_expr(e, defs, errs);
                }
            }
            HirExprKind::Block { stmts, expr } => {
                Self::walk_stmts(stmts, defs, errs);
                if let Some(e) = expr {
                    Self::walk_expr(e, defs, errs);
                }
            }
            HirExprKind::Match { values, arms } => {
                for v in values {
                    Self::walk_expr(v, defs, errs);
                }
                for a in arms {
                    Self::walk_expr(&a.body, defs, errs);
                }
            }
            _ => {}
        }
    }
}

/// Field info for struct literal validation lookups.
struct FieldInfo {
    name: String,
    decorators: Vec<HirDecorator>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decorator_kind_from_name() {
        assert_eq!(DecoratorKind::from_name("email"), DecoratorKind::Email);
        assert_eq!(DecoratorKind::from_name("min"), DecoratorKind::Min);
        assert!(matches!(
            DecoratorKind::from_name("foo"),
            DecoratorKind::Unknown(_)
        ));
    }

    #[test]
    fn test_email_validation() {
        assert!(DecoratorValidator::is_valid_email("user@example.com"));
        assert!(!DecoratorValidator::is_valid_email("not-an-email"));
        assert!(!DecoratorValidator::is_valid_email("@example.com"));
        assert!(!DecoratorValidator::is_valid_email("user@"));
        assert!(!DecoratorValidator::is_valid_email("user@.com"));
    }
}
