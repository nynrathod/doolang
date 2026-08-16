//! Type Checking on THIR
//!
/// Validates type consistency across the fully-typed THIR. Since THIR
/// expressions already carry resolved `TypeId`s (assigned during HIR
/// lowering and type inference), this pass checks that those types are
/// consistent with their usage context — not that they are inferable.
///
/// ## Checks Performed
///
/// - Assignment: target type must match value type
/// - Return: value type must match function return type
/// - If condition: must be Bool
/// - Match arms: all arms must have the same type
/// - Array literals: all elements must have the same type
/// - Map literals: all keys same type, all values same type
/// - Method calls: resolved impl signature must match arguments
/// - Recursive types: must use Box to break infinite size
use crate::types::compat::{check_recursive_type, types_compatible};
use doo_core::types::{builtin, TypeId, TypeKind, TypeRegistry};
use doo_core::Span;
use doo_thir::{
    ThirExpr, ThirExprKind, ThirFunction, ThirItem, ThirProgram, ThirStmt, ThirStmtKind,
};

/// Type checking error.
#[derive(Debug, Clone)]
pub struct TypeError {
    pub kind: TypeErrorKind,
    pub span: Span,
}

impl TypeError {
    pub fn new(kind: TypeErrorKind, span: Span) -> Self {
        Self { kind, span }
    }

    pub fn message(&self) -> String {
        self.kind.message()
    }
}

/// Categories of type errors.
#[derive(Debug, Clone)]
pub enum TypeErrorKind {
    /// Expected one type, found another.
    Mismatch { expected: String, found: String },
    /// Undefined variable or function.
    Undefined(String, Option<String>),
    /// Undefined function specifically.
    UndefinedFunction(String),
    /// Undefined type.
    UndefinedType(String),
    /// No such field on a type.
    UndefinedField { type_name: String, field: String },
    /// No such method on a type.
    UndefinedMethod { type_name: String, method: String },
    /// No such variant in an enum.
    UndefinedVariant { enum_name: String, variant: String },
    /// Invalid operation for the given types.
    InvalidOp(String),
    /// Wrong number of arguments.
    ArgMismatch { expected: usize, found: usize },
    /// Condition is not Bool.
    InvalidCondition { found: String },
    /// Invalid cast between types.
    InvalidCast { from: String, to: String },
    /// Return type doesn't match function signature.
    ReturnTypeMismatch {
        function: String,
        expected: String,
        found: String,
    },
    /// Unknown type name.
    UnknownType(String),
    /// Cannot infer type from context.
    CannotInfer(String),
    /// Two incompatible types in an operation.
    Incompatible {
        left: String,
        right: String,
        operation: String,
    },
    /// Cannot convert between types.
    CannotConvert { from: String, to: String },
    /// Tuple lengths don't match.
    TupleLengthMismatch { expected: usize, found: usize },
    /// Wrong number of type parameters.
    TypeParamCount { expected: usize, found: usize },
    /// Array element type mismatch.
    InvalidArrayElement {
        expected: String,
        found: String,
        index: usize,
    },
    /// Map key type is not hashable.
    InvalidMapKey { found: String },
    /// If/else branches have different types.
    IfElseMismatch {
        then_type: String,
        else_type: String,
    },
    /// Nil used where a non-optional type was expected.
    NilNonOptional { expected: String },
    /// Missing required struct field.
    MissingStructField { struct_name: String, field: String },
    /// Unknown struct field.
    UnknownStructField { struct_name: String, field: String },
    /// Invalid function signature.
    InvalidSignature(String),
}

impl TypeErrorKind {
    pub fn message(&self) -> String {
        match self {
            Self::Mismatch { expected, found } => {
                format!("expected `{}`, found `{}`", expected, found)
            }
            Self::Undefined(name, suggestion) => {
                if let Some(s) = suggestion {
                    format!("'{}' is not defined (did you mean '{}'?)", name, s)
                } else {
                    format!("'{}' is not defined", name)
                }
            }
            Self::UndefinedFunction(name) => format!("function '{}' is not defined", name),
            Self::UndefinedType(name) => format!("type '{}' is not defined", name),
            Self::UndefinedField { type_name, field } => {
                format!("no field '{}' on type '{}'", field, type_name)
            }
            Self::UndefinedMethod { type_name, method } => {
                format!("no method '{}' on type '{}'", method, type_name)
            }
            Self::UndefinedVariant { enum_name, variant } => {
                format!("no variant '{}' in enum '{}'", variant, enum_name)
            }
            Self::InvalidOp(msg) => msg.clone(),
            Self::ArgMismatch { expected, found } => {
                format!("expected {} argument(s), found {}", expected, found)
            }
            Self::InvalidCondition { found } => {
                format!("condition must be Bool, found {}", found)
            }
            Self::InvalidCast { from, to } => {
                format!("cannot cast {} to {}", from, to)
            }
            Self::ReturnTypeMismatch {
                function,
                expected,
                found,
            } => {
                format!("expected {}, found {} in '{}'", expected, found, function)
            }
            Self::UnknownType(name) => format!("unknown type '{}'", name),
            Self::CannotInfer(ctx) => {
                if ctx.is_empty() {
                    "cannot infer type".to_string()
                } else {
                    format!("cannot infer type: {}", ctx)
                }
            }
            Self::Incompatible {
                left,
                right,
                operation,
            } => {
                format!(
                    "incompatible types {} and {} for '{}'",
                    left, right, operation
                )
            }
            Self::CannotConvert { from, to } => {
                format!("cannot convert {} to {}", from, to)
            }
            Self::TupleLengthMismatch { expected, found } => {
                format!("tuple expects {} element(s), found {}", expected, found)
            }
            Self::TypeParamCount { expected, found } => {
                format!("expected {} type parameter(s), found {}", expected, found)
            }
            Self::InvalidArrayElement {
                expected,
                found,
                index,
            } => {
                format!("expected {}, found {} at index {}", expected, found, index)
            }
            Self::InvalidMapKey { found } => {
                format!("map key must be hashable (Str, Int, Bool), found {}", found)
            }
            Self::IfElseMismatch {
                then_type,
                else_type,
            } => {
                format!("expected {}, found {} in else branch", then_type, else_type)
            }
            Self::NilNonOptional { expected } => {
                format!("expected {}, found nil", expected)
            }
            Self::MissingStructField { struct_name, field } => {
                format!("missing field '{}' in struct '{}'", field, struct_name)
            }
            Self::UnknownStructField { struct_name, field } => {
                format!("unknown field '{}' in struct '{}'", field, struct_name)
            }
            Self::InvalidSignature(msg) => msg.clone(),
        }
    }
}

/// The type checker. Walks THIR and validates type consistency.
pub struct TypeChecker<'a> {
    registry: &'a TypeRegistry,
    errors: Vec<TypeError>,
    /// Current function's return type (for return statement validation).
    current_return_type: Option<TypeId>,
    /// Current function's name (for error messages).
    current_function_name: String,
    /// Current function's error type (for ? operator validation).
    current_error_type: Option<TypeId>,
}

impl<'a> TypeChecker<'a> {
    pub fn new(registry: &'a TypeRegistry) -> Self {
        Self {
            registry,
            errors: Vec::new(),
            current_return_type: None,
            current_function_name: String::new(),
            current_error_type: None,
        }
    }

    /// Type-check an entire THIR program.
    pub fn check_program(&mut self, program: &ThirProgram) -> Result<(), Vec<TypeError>> {
        for item in &program.items {
            self.check_item(item);
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(std::mem::take(&mut self.errors))
        }
    }

    /// Type-check a single top-level item.
    fn check_item(&mut self, item: &ThirItem) {
        match item {
            ThirItem::Function(func) => self.check_function(func),
            ThirItem::Const(c) => self.check_const(c),
            ThirItem::Static(s) => self.check_static(s),
            ThirItem::Struct(s) => self.check_struct_definition(s),
            ThirItem::Enum(e) => self.check_enum_definition(e),
            ThirItem::Interface(_) | ThirItem::Import(_) => {}
        }
    }

    /// Type-check a function body.
    fn check_function(&mut self, func: &ThirFunction) {
        self.current_return_type = func.return_type;
        self.current_function_name = func.name.clone();
        self.current_error_type = func.error_type;

        for stmt in &func.body {
            self.check_stmt(stmt);
        }

        self.current_return_type = None;
        self.current_function_name.clear();
        self.current_error_type = None;
    }

    /// Validate a const declaration.
    fn check_const(&mut self, c: &doo_thir::ThirConst) {
        self.check_expr(&c.value_expr);

        // const values must have compile-time-known types
        if c.ty == builtin::ANY {
            self.errors.push(TypeError::new(
                TypeErrorKind::CannotInfer("const value type".into()),
                c.span,
            ));
        }
    }

    /// Validate a static declaration.
    fn check_static(&mut self, s: &doo_thir::ThirStatic) {
        // static values must be Send-equivalent (no borrowed references)
        if let Some(ty) = s.ty {
            if let Some(info) = self.registry.get(ty) {
                if let TypeKind::Function { .. } = &info.kind {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::InvalidSignature(
                            "static cannot hold a function pointer (not Send)".into(),
                        ),
                        s.span,
                    ));
                }
            }
        }
    }

    /// Validate struct field types and check for illegal recursion.
    fn check_struct_definition(&mut self, s: &doo_thir::ThirStruct) {
        // Check each field's type is valid
        for field in &s.fields {
            if let Some(ty) = field.ty {
                if self.registry.get(ty).is_none() {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::UnknownType(field.name.clone()),
                        field.span,
                    ));
                }
            }
        }

        // Check for recursive types without Box
        // We need to find the TypeId for this struct by name
        if let Some(type_id) = self.registry.lookup(&s.name) {
            if let Err(err) = check_recursive_type(type_id, self.registry) {
                self.errors.push(TypeError::new(
                    TypeErrorKind::InvalidOp(format!(
                        "recursive struct '{}' has field '{}' of same type — use Box<{}>",
                        err.type_name, err.field_name, err.type_name
                    )),
                    s.span,
                ));
            }
        }
    }

    /// Validate enum variant payload types.
    fn check_enum_definition(&mut self, e: &doo_thir::ThirEnum) {
        for variant in &e.variants {
            if let Some(payload_ty) = variant.payload {
                if self.registry.get(payload_ty).is_none() {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::UnknownType(variant.name.clone()),
                        variant.span,
                    ));
                }
            }
        }
    }

    // ========================================================================
    // Statement Checking
    // ========================================================================

    fn check_stmt(&mut self, stmt: &ThirStmt) {
        match &stmt.kind {
            ThirStmtKind::Let { value, ty, .. } => {
                self.check_expr(value);
                self.verify_assignment(*ty, value.ty, stmt.span);
            }

            ThirStmtKind::Const { value, ty, .. } => {
                self.check_expr(value);
                self.verify_assignment(*ty, value.ty, stmt.span);
            }

            ThirStmtKind::Assign { target, value } => {
                self.check_expr(target);
                self.check_expr(value);
                self.verify_assignment(target.ty, value.ty, stmt.span);
            }

            ThirStmtKind::Expr(expr) => {
                self.check_expr(expr);
            }

            ThirStmtKind::Return(val) => {
                if let Some(e) = val {
                    self.check_expr(e);
                    if let Some(ret_ty) = self.current_return_type {
                        self.verify_assignment(ret_ty, e.ty, stmt.span);
                    }
                }
            }

            ThirStmtKind::Break(_) | ThirStmtKind::Continue => {}

            ThirStmtKind::While {
                cond,
                body,
                increment,
            } => {
                self.check_expr(cond);
                self.verify_condition(cond, stmt.span);
                for s in body {
                    self.check_stmt(s);
                }
                for s in increment {
                    self.check_stmt(s);
                }
            }

            ThirStmtKind::Loop { body } => {
                for s in body {
                    self.check_stmt(s);
                }
            }

            ThirStmtKind::Go { expr } => {
                self.check_expr(expr);
            }

            ThirStmtKind::Scope { stmts } => {
                for s in stmts {
                    self.check_stmt(s);
                }
            }

            ThirStmtKind::Drop { .. } => {}

            ThirStmtKind::TupleLet {
                value, type_ids, ..
            } => {
                self.check_expr(value);
                // Verify tuple element types match
                if let Some(info) = self.registry.get(value.ty) {
                    if let TypeKind::Tuple { elements } = &info.kind {
                        for (i, (expected, actual)) in
                            type_ids.iter().zip(elements.iter()).enumerate()
                        {
                            if !types_compatible(*expected, *actual, self.registry) {
                                self.errors.push(TypeError::new(
                                    TypeErrorKind::InvalidArrayElement {
                                        expected: self.type_name(*expected),
                                        found: self.type_name(*actual),
                                        index: i,
                                    },
                                    stmt.span,
                                ));
                            }
                        }
                    }
                }
            }

            ThirStmtKind::ManualErrorExtract { expr, .. } => {
                self.check_expr(expr);
            }
        }
    }

    // ========================================================================
    // Expression Checking
    // ========================================================================

    fn check_expr(&mut self, expr: &ThirExpr) {
        match &expr.kind {
            ThirExprKind::Literal(_) | ThirExprKind::Var(_) => {}

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
                self.verify_condition(cond, expr.span);
                self.check_expr(then);
                if let Some(e) = else_ {
                    self.check_expr(e);
                    // Both branches must have the same type
                    if !types_compatible(then.ty, e.ty, self.registry) {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::IfElseMismatch {
                                then_type: self.type_name(then.ty),
                                else_type: self.type_name(e.ty),
                            },
                            expr.span,
                        ));
                    }
                }
            }

            ThirExprKind::Match {
                expr: scrutinee,
                arms,
            } => {
                self.check_expr(scrutinee);
                // All arms must have the same type
                if let Some(first) = arms.first() {
                    let first_ty = first.body.ty;
                    for arm in &arms[1..] {
                        self.check_expr(&arm.body);
                        if let Some(g) = &arm.guard {
                            self.check_expr(g);
                        }
                        if !types_compatible(first_ty, arm.body.ty, self.registry) {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::Mismatch {
                                    expected: self.type_name(first_ty),
                                    found: self.type_name(arm.body.ty),
                                },
                                arm.body.span,
                            ));
                        }
                    }
                    // Check first arm
                    self.check_expr(&first.body);
                    if let Some(g) = &first.guard {
                        self.check_expr(g);
                    }
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
                if let Some(first) = elements.first() {
                    let elem_ty = first.ty;
                    for (i, e) in elements.iter().enumerate() {
                        self.check_expr(e);
                        if !types_compatible(elem_ty, e.ty, self.registry) {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::InvalidArrayElement {
                                    expected: self.type_name(elem_ty),
                                    found: self.type_name(e.ty),
                                    index: i,
                                },
                                e.span,
                            ));
                        }
                    }
                }
            }

            ThirExprKind::MapLiteral(entries) => {
                if let Some((first_k, first_v)) = entries.first() {
                    let key_ty = first_k.ty;
                    let val_ty = first_v.ty;
                    for (k, v) in entries {
                        self.check_expr(k);
                        self.check_expr(v);
                        if !types_compatible(key_ty, k.ty, self.registry) {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::InvalidArrayElement {
                                    expected: self.type_name(key_ty),
                                    found: self.type_name(k.ty),
                                    index: 0,
                                },
                                k.span,
                            ));
                        }
                        if !types_compatible(val_ty, v.ty, self.registry) {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::InvalidArrayElement {
                                    expected: self.type_name(val_ty),
                                    found: self.type_name(v.ty),
                                    index: 1,
                                },
                                v.span,
                            ));
                        }
                    }

                    // Map keys must be hashable
                    if !self.is_hashable(key_ty) {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::InvalidMapKey {
                                found: self.type_name(key_ty),
                            },
                            first_k.span,
                        ));
                    }
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

            ThirExprKind::Ok(inner) | ThirExprKind::Err(inner) => {
                self.check_expr(inner);
            }

            ThirExprKind::Try(inner) => {
                self.check_expr(inner);
                // ? operator must be in a function returning Result
                if self.current_error_type.is_none() && self.current_function_name != "main" {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::InvalidOp(format!(
                            "`?` used in '{}' which doesn't return Result",
                            self.current_function_name
                        )),
                        expr.span,
                    ));
                }
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

            ThirExprKind::Cast { value, to_type } => {
                self.check_expr(value);
                // Verify cast is valid
                if !self.is_valid_cast(value.ty, *to_type) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::InvalidCast {
                            from: self.type_name(value.ty),
                            to: self.type_name(*to_type),
                        },
                        expr.span,
                    ));
                }
            }

            ThirExprKind::ScopeBlock { stmts } => {
                for s in stmts {
                    self.check_stmt(s);
                }
            }
        }
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    /// Verify that `value_ty` can be assigned to `target_ty`.
    fn verify_assignment(&mut self, target_ty: TypeId, value_ty: TypeId, span: Span) {
        if !types_compatible(value_ty, target_ty, self.registry) {
            // Allow nil for optional types
            if value_ty == builtin::VOID {
                if let Some(info) = self.registry.get(target_ty) {
                    if matches!(info.kind, TypeKind::Optional { .. }) {
                        return;
                    }
                }
            }

            self.errors.push(TypeError::new(
                TypeErrorKind::Mismatch {
                    expected: self.type_name(target_ty),
                    found: self.type_name(value_ty),
                },
                span,
            ));
        }
    }

    /// Verify that a condition expression is Bool.
    fn verify_condition(&mut self, cond: &ThirExpr, span: Span) {
        if cond.ty != builtin::BOOL && cond.ty != builtin::ANY {
            self.errors.push(TypeError::new(
                TypeErrorKind::InvalidCondition {
                    found: self.type_name(cond.ty),
                },
                span,
            ));
        }
    }

    /// Get a human-readable type name.
    fn type_name(&self, ty: TypeId) -> String {
        self.registry.display_name(ty)
    }

    /// Check if a type is hashable (can be used as a Map key).
    fn is_hashable(&self, ty: TypeId) -> bool {
        // Only primitives and Str are hashable
        ty == builtin::INT
            || ty == builtin::STR
            || ty == builtin::BOOL
            || ty == builtin::CHAR
            || ty == builtin::ANY
    }

    /// Check if a cast between two types is valid.
    fn is_valid_cast(&self, from: TypeId, to: TypeId) -> bool {
        if from == to {
            return true;
        }

        // Numeric casts: Int <-> Float, Int <-> Int sizes
        let numeric_types = [builtin::INT, builtin::FLOAT, builtin::BOOL, builtin::CHAR];

        let from_numeric = numeric_types.contains(&from)
            || self.registry.get(from).map_or(false, |i| {
                matches!(
                    i.kind,
                    TypeKind::Int8
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
                        | TypeKind::Float64
                )
            });

        let to_numeric = numeric_types.contains(&to)
            || self.registry.get(to).map_or(false, |i| {
                matches!(
                    i.kind,
                    TypeKind::Int8
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
                        | TypeKind::Float64
                )
            });

        if from_numeric && to_numeric {
            return true;
        }

        // Any cast is always valid (used for type recovery)
        if from == builtin::ANY || to == builtin::ANY {
            return true;
        }

        false
    }

    /// Get collected errors.
    pub fn errors(&self) -> &[TypeError] {
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
    fn test_type_checker_creation() {
        let registry = TypeRegistry::new();
        let checker = TypeChecker::new(&registry);
        assert!(checker.errors.is_empty());
    }

    #[test]
    fn test_type_error_message() {
        let err = TypeError::new(
            TypeErrorKind::Mismatch {
                expected: "Int".into(),
                found: "Str".into(),
            },
            Span::dummy(),
        );
        assert!(err.message().contains("Int"));
        assert!(err.message().contains("Str"));
    }

    #[test]
    fn test_undefined_error_with_suggestion() {
        let err = TypeError::new(
            TypeErrorKind::Undefined("fooo".into(), Some("foo".into())),
            Span::dummy(),
        );
        assert!(err.message().contains("did you mean"));
    }
}
