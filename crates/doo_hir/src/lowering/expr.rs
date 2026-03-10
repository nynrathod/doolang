//! Expression lowering.

use super::{hir_binop_to_kind, hir_unaryop_to_kind};
use super::{Lower, LowerError};
use crate::types::*;
use doo_core::{
    constants::ffi_names,
    infer::{infer_binop_result_type, infer_unaryop_result_type},
    types::{builtin, TypeId, TypeKind, TypeRegistry},
};
use doo_frontend::ast::{Expr, ExprKind};

impl Lower {
    pub(crate) fn lower_expr(&mut self, expr: &Expr) -> HirExpr {
        let kind = match &expr.kind {
            ExprKind::IntLit(v) => HirExprKind::Const(ConstValue::Int(*v)),
            ExprKind::FloatLit(v) => HirExprKind::Const(ConstValue::Float(*v)),
            ExprKind::BoolLit(v) => HirExprKind::Const(ConstValue::Bool(*v)),
            ExprKind::StrLit(v) => HirExprKind::Const(ConstValue::Str(v.clone())),
            ExprKind::Nil => HirExprKind::Const(ConstValue::Nil),

            ExprKind::Ident(name) => HirExprKind::Local { name: name.clone() },

            ExprKind::Binary { left, op, right } => HirExprKind::BinOp {
                op: self.lower_binop(*op),
                lhs: Box::new(self.lower_expr(left)),
                rhs: Box::new(self.lower_expr(right)),
            },

            ExprKind::Unary { op, expr: inner } => HirExprKind::UnaryOp {
                op: self.lower_unaryop(*op),
                operand: Box::new(self.lower_expr(inner)),
            },

            ExprKind::Call { func, args } => HirExprKind::Call {
                func: Box::new(self.lower_expr(func)),
                args: args.iter().map(|a| self.lower_expr(a)).collect(),
            },

            ExprKind::MethodCall {
                object,
                method,
                args,
            } => {
                // Transform HTTP route methods with middleware arguments
                // app.get("/path", middleware, Handler) -> app.getWithMiddleware("/path", "middleware", Handler)
                // app.get("/path", m1, m2, Handler) -> app.getWithMiddleware("/path", "m1,m2", Handler)
                if Self::is_http_route_method(method) && args.len() > 2 {
                    return self.transform_route_with_middleware(object, method, args, expr.span);
                }

                // Transform app.group with route block
                // app.group("/api", middleware, { routes }) -> expand routes with prefix and middleware
                if method == "group" && args.len() >= 2 {
                    if let Some(route_block_arg) = args.last() {
                        if matches!(
                            route_block_arg,
                            Expr {
                                kind: ExprKind::RouteBlock { .. },
                                ..
                            }
                        ) {
                            return self.transform_group_with_routes(object, args, expr.span);
                        }
                    }
                }

                HirExprKind::MethodCall {
                    receiver: Box::new(self.lower_expr(object)),
                    method: method.clone(),
                    args: args.iter().map(|a| self.lower_expr(a)).collect(),
                }
            }

            ExprKind::Field { object, field } => HirExprKind::Field {
                object: Box::new(self.lower_expr(object)),
                field: field.clone(),
            },

            ExprKind::Index { object, index } => HirExprKind::Index {
                object: Box::new(self.lower_expr(object)),
                index: Box::new(self.lower_expr(index)),
            },

            ExprKind::ArrayLit(elements) => {
                HirExprKind::Array(elements.iter().map(|e| self.lower_expr(e)).collect())
            }

            ExprKind::MapLit(entries) => HirExprKind::Map(
                entries
                    .iter()
                    .map(|(k, v)| (self.lower_expr(k), self.lower_expr(v)))
                    .collect(),
            ),

            ExprKind::TupleLit(elements) => {
                HirExprKind::Tuple(elements.iter().map(|e| self.lower_expr(e)).collect())
            }

            ExprKind::ObjectLit(fields) | ExprKind::StructLit { fields, .. } => {
                let name = if let ExprKind::StructLit { name, .. } = &expr.kind {
                    name.clone()
                } else {
                    ffi_names::OBJECT_LIT_NAME.to_string()
                };
                HirExprKind::Struct {
                    name,
                    fields: fields
                        .iter()
                        .map(|(k, v)| (k.clone(), self.lower_expr(v)))
                        .collect(),
                }
            }

            ExprKind::EnumVariant {
                enum_name,
                variant,
                payload,
            } => {
                // Check if this is a qualified method call: Type::method(args)
                // e.g., Database::get() should resolve to the Database.get associated method
                let is_qualified_method = self
                    .known_qualified_methods
                    .get(enum_name)
                    .map(|methods| methods.contains(variant))
                    .unwrap_or(false);

                if is_qualified_method || self.known_functions.contains(variant) {
                    // Namespace-qualified function call: Namespace::Func(args) -> Func(args)
                    let func_expr = HirExpr::new(
                        HirExprKind::Global {
                            name: variant.clone(),
                        },
                        expr.span,
                    );
                    HirExprKind::Call {
                        func: Box::new(func_expr),
                        args: payload.iter().map(|e| self.lower_expr(e)).collect(),
                    }
                } else {
                    HirExprKind::EnumVariant {
                        enum_name: enum_name.clone(),
                        variant: variant.clone(),
                        payload: payload.iter().map(|e| self.lower_expr(e)).collect(),
                    }
                }
            }

            ExprKind::Range {
                start,
                end,
                inclusive,
            } => HirExprKind::Range {
                start: Box::new(self.lower_expr(start)),
                end: Box::new(self.lower_expr(end)),
                inclusive: *inclusive,
            },

            ExprKind::IfExpr {
                condition,
                then_branch,
                else_branch,
            } => HirExprKind::If {
                condition: Box::new(self.lower_expr(condition)),
                then_expr: Box::new(self.lower_expr(then_branch)),
                else_expr: else_branch.as_ref().map(|e| Box::new(self.lower_expr(e))),
            },

            ExprKind::Block(stmts, final_expr) => HirExprKind::Block {
                stmts: stmts.iter().map(|s| self.lower_stmt(s)).collect(),
                expr: final_expr.as_ref().map(|e| Box::new(self.lower_expr(e))),
            },

            ExprKind::Ok(values) => {
                let inner = if values.len() == 1 {
                    self.lower_expr(&values[0])
                } else {
                    HirExpr::new(
                        HirExprKind::Tuple(values.iter().map(|e| self.lower_expr(e)).collect()),
                        expr.span,
                    )
                };
                HirExprKind::Ok(Box::new(inner))
            }

            ExprKind::Err(inner) => HirExprKind::Err(Box::new(self.lower_expr(inner))),

            ExprKind::Try(inner) => HirExprKind::Try(Box::new(self.lower_expr(inner))),

            ExprKind::UnwrapOrPanic {
                expr: inner,
                message,
            } => HirExprKind::UnwrapOrPanic {
                expr: Box::new(self.lower_expr(inner)),
                message: Box::new(self.lower_expr(message)),
            },

            ExprKind::Closure { params, body, .. } => HirExprKind::Closure {
                params: params.iter().map(|(n, _)| (n.clone(), None)).collect(),
                body: Box::new(self.lower_expr(body)),
            },

            ExprKind::Match { values, arms } => {
                let lowered_values: Vec<HirExpr> =
                    values.iter().map(|v| self.lower_expr(v)).collect();
                let lowered_arms: Vec<HirMatchArm> = arms
                    .iter()
                    .map(|a| {
                        let mut pattern = self.lower_match_pattern(&a.pattern);
                        // Convert struct literal patterns to field-by-field comparisons
                        if let HirMatchPattern::Condition(ref cond_expr) = pattern {
                            if let HirExprKind::Struct { fields, .. } = &cond_expr.kind {
                                if let Some(matched_val) = lowered_values.first() {
                                    pattern = self.struct_pattern_to_condition(
                                        matched_val,
                                        fields,
                                        cond_expr.span,
                                    );
                                }
                            }
                        }
                        HirMatchArm {
                            pattern,
                            guard: a.guard.as_ref().map(|g| self.lower_expr(g)),
                            body: self.lower_expr(&a.body),
                            span: a.span,
                        }
                    })
                    .collect();
                HirExprKind::Match {
                    values: lowered_values,
                    arms: lowered_arms,
                }
            }

            ExprKind::Spread(inner) => HirExprKind::Spread(Box::new(self.lower_expr(inner))),

            ExprKind::RouteBlock { routes } => HirExprKind::RouteBlock {
                routes: routes.iter().map(|r| self.lower_expr(r)).collect(),
            },

            // === Async & Concurrency ===
            ExprKind::Await(inner) => HirExprKind::Await(Box::new(self.lower_expr(inner))),
            ExprKind::GoSpawn { body } => HirExprKind::Spawn {
                body: Box::new(self.lower_expr(body)),
            },
            ExprKind::ScopeBlock { body } => HirExprKind::ScopeBlock {
                stmts: body.iter().map(|s| self.lower_stmt(s)).collect(),
            },

            ExprKind::StringInterpolation(parts) => {
                // Desugar: "a ${b} c" -> "a" + (b as Str) + "c"
                if parts.is_empty() {
                    HirExprKind::Const(ConstValue::Str(String::new()))
                } else {
                    let mut current = self.lower_string_part(&parts[0]);
                    for part in &parts[1..] {
                        let next = self.lower_string_part(part);
                        current = HirExpr::new(
                            HirExprKind::BinOp {
                                op: HirBinOp::Add,
                                lhs: Box::new(current),
                                rhs: Box::new(next),
                            },
                            expr.span,
                        );
                    }
                    current.kind
                }
            }

            ExprKind::Ternary { .. } | ExprKind::Cast { .. } => {
                // Defer complex constructs
                self.errors.push(LowerError::new(
                    "Complex expression not yet lowered",
                    expr.span,
                ));
                HirExprKind::Const(ConstValue::Nil)
            }
        };

        let mut out = HirExpr::new(kind, expr.span);
        if let HirExprKind::Const(c) = &out.kind {
            out.type_id = Some(c.type_id());
        }
        out
    }

    pub(crate) fn lower_expr_typed(&mut self, expr: &Expr, registry: &mut TypeRegistry) -> HirExpr {
        let kind = match &expr.kind {
            ExprKind::IntLit(v) => HirExprKind::Const(ConstValue::Int(*v)),
            ExprKind::FloatLit(v) => HirExprKind::Const(ConstValue::Float(*v)),
            ExprKind::BoolLit(v) => HirExprKind::Const(ConstValue::Bool(*v)),
            ExprKind::StrLit(v) => HirExprKind::Const(ConstValue::Str(v.clone())),
            ExprKind::Nil => HirExprKind::Const(ConstValue::Nil),

            ExprKind::Ident(name) => {
                // Look up the variable type if tracked
                let kind = HirExprKind::Local { name: name.clone() };
                if let Some(&type_id) = self.var_types.get(name) {
                    return HirExpr::with_type(kind, type_id, expr.span);
                }
                kind
            }

            ExprKind::Binary { left, op, right } => HirExprKind::BinOp {
                op: self.lower_binop(*op),
                lhs: Box::new(self.lower_expr_typed(left, registry)),
                rhs: Box::new(self.lower_expr_typed(right, registry)),
            },

            ExprKind::Unary { op, expr: inner } => HirExprKind::UnaryOp {
                op: self.lower_unaryop(*op),
                operand: Box::new(self.lower_expr_typed(inner, registry)),
            },

            ExprKind::Call { func, args } => HirExprKind::Call {
                func: Box::new(self.lower_expr_typed(func, registry)),
                args: args
                    .iter()
                    .map(|a| self.lower_expr_typed(a, registry))
                    .collect(),
            },

            ExprKind::MethodCall {
                object,
                method,
                args,
            } => {
                // Transform HTTP route methods with middleware arguments
                // app.get("/path", middleware, Handler) -> app.getWithMiddleware("/path", "middleware", Handler)
                if Self::is_http_route_method(method) && args.len() > 2 {
                    return self.transform_route_with_middleware_typed(
                        object, method, args, expr.span, registry,
                    );
                }

                // Transform app.group with route block
                if method == "group" && args.len() >= 2 {
                    if let Some(route_block_arg) = args.last() {
                        if matches!(
                            route_block_arg,
                            Expr {
                                kind: ExprKind::RouteBlock { .. },
                                ..
                            }
                        ) {
                            return self.transform_group_with_routes_typed(
                                object, args, expr.span, registry,
                            );
                        }
                    }
                }

                HirExprKind::MethodCall {
                    receiver: Box::new(self.lower_expr_typed(object, registry)),
                    method: method.clone(),
                    args: args
                        .iter()
                        .map(|a| self.lower_expr_typed(a, registry))
                        .collect(),
                }
            }

            ExprKind::Field { object, field } => HirExprKind::Field {
                object: Box::new(self.lower_expr_typed(object, registry)),
                field: field.clone(),
            },

            ExprKind::Index { object, index } => HirExprKind::Index {
                object: Box::new(self.lower_expr_typed(object, registry)),
                index: Box::new(self.lower_expr_typed(index, registry)),
            },

            ExprKind::ArrayLit(elements) => HirExprKind::Array(
                elements
                    .iter()
                    .map(|e| self.lower_expr_typed(e, registry))
                    .collect(),
            ),

            ExprKind::MapLit(entries) => HirExprKind::Map(
                entries
                    .iter()
                    .map(|(k, v)| {
                        (
                            self.lower_expr_typed(k, registry),
                            self.lower_expr_typed(v, registry),
                        )
                    })
                    .collect(),
            ),

            ExprKind::TupleLit(elements) => HirExprKind::Tuple(
                elements
                    .iter()
                    .map(|e| self.lower_expr_typed(e, registry))
                    .collect(),
            ),

            ExprKind::ObjectLit(fields) | ExprKind::StructLit { fields, .. } => {
                let name = if let ExprKind::StructLit { name, .. } = &expr.kind {
                    name.clone()
                } else {
                    ffi_names::OBJECT_LIT_NAME.to_string()
                };
                HirExprKind::Struct {
                    name,
                    fields: fields
                        .iter()
                        .map(|(k, v)| (k.clone(), self.lower_expr_typed(v, registry)))
                        .collect(),
                }
            }

            ExprKind::EnumVariant {
                enum_name,
                variant,
                payload,
            } => {
                // Check if this is actually a static method call on a struct
                // e.g., Database::Postgres() should be MethodCall, not EnumVariant
                let type_id = registry.lookup(enum_name);
                let is_struct = type_id
                    .and_then(|tid| registry.get(tid))
                    .map(|info| matches!(info.kind, TypeKind::Struct { .. }))
                    .unwrap_or(false);
                let _is_enum = type_id
                    .and_then(|tid| registry.get(tid))
                    .map(|info| matches!(info.kind, TypeKind::Enum { .. }))
                    .unwrap_or(false);

                if is_struct {
                    // Convert to MethodCall: Type::method(args) -> Type.method(args)
                    let receiver = HirExpr::with_type(
                        HirExprKind::Local {
                            name: enum_name.clone(),
                        },
                        type_id.unwrap(),
                        expr.span,
                    );
                    HirExprKind::MethodCall {
                        receiver: Box::new(receiver),
                        method: variant.clone(),
                        args: payload
                            .iter()
                            .map(|e| self.lower_expr_typed(e, registry))
                            .collect(),
                    }
                } else if self
                    .known_qualified_methods
                    .get(enum_name)
                    .map(|methods| methods.contains(variant))
                    .unwrap_or(false)
                    || self.known_functions.contains(variant)
                {
                    // Namespace-qualified function call: Array::Sum(args) -> Sum(args)
                    // or Type::method(args) -> method(args) for associated functions
                    // The parser treats Name::Name(args) as EnumVariant, but when
                    // the "variant" name matches a known function and the "enum_name"
                    // is not a real enum, this is a qualified call through a namespace
                    // import (e.g., `import std::Array` then `Array::Sum(...)`).
                    let func_expr = HirExpr::new(
                        HirExprKind::Global {
                            name: variant.clone(),
                        },
                        expr.span,
                    );
                    HirExprKind::Call {
                        func: Box::new(func_expr),
                        args: payload
                            .iter()
                            .map(|e| self.lower_expr_typed(e, registry))
                            .collect(),
                    }
                } else {
                    // It's a real enum variant
                    HirExprKind::EnumVariant {
                        enum_name: enum_name.clone(),
                        variant: variant.clone(),
                        payload: payload
                            .iter()
                            .map(|e| self.lower_expr_typed(e, registry))
                            .collect(),
                    }
                }
            }

            ExprKind::Range {
                start,
                end,
                inclusive,
            } => HirExprKind::Range {
                start: Box::new(self.lower_expr_typed(start, registry)),
                end: Box::new(self.lower_expr_typed(end, registry)),
                inclusive: *inclusive,
            },

            ExprKind::IfExpr {
                condition,
                then_branch,
                else_branch,
            } => HirExprKind::If {
                condition: Box::new(self.lower_expr_typed(condition, registry)),
                then_expr: Box::new(self.lower_expr_typed(then_branch, registry)),
                else_expr: else_branch
                    .as_ref()
                    .map(|e| Box::new(self.lower_expr_typed(e, registry))),
            },

            ExprKind::Block(stmts, final_expr) => HirExprKind::Block {
                stmts: stmts
                    .iter()
                    .map(|s| self.lower_stmt_typed(s, registry))
                    .collect(),
                expr: final_expr
                    .as_ref()
                    .map(|e| Box::new(self.lower_expr_typed(e, registry))),
            },

            ExprKind::Ok(values) => {
                let inner = if values.len() == 1 {
                    self.lower_expr_typed(&values[0], registry)
                } else {
                    HirExpr::new(
                        HirExprKind::Tuple(
                            values
                                .iter()
                                .map(|e| self.lower_expr_typed(e, registry))
                                .collect(),
                        ),
                        expr.span,
                    )
                };
                HirExprKind::Ok(Box::new(inner))
            }

            ExprKind::Err(inner) => {
                HirExprKind::Err(Box::new(self.lower_expr_typed(inner, registry)))
            }
            ExprKind::Try(inner) => {
                HirExprKind::Try(Box::new(self.lower_expr_typed(inner, registry)))
            }

            ExprKind::UnwrapOrPanic {
                expr: inner,
                message,
            } => HirExprKind::UnwrapOrPanic {
                expr: Box::new(self.lower_expr_typed(inner, registry)),
                message: Box::new(self.lower_expr_typed(message, registry)),
            },

            ExprKind::Closure {
                params,
                body,
                return_type,
                ..
            } => {
                // Save var_types before lowering closure body.
                // Closure-local variables (e.g., `let s = a + b;`) must NOT leak into
                // subsequent closures that reuse the same name (e.g., `(s) => s.len()`).
                let saved_var_types = self.var_types.clone();
                let mut body_hir = self.lower_expr_typed(body, registry);
                // Restore var_types — closure-scoped bindings are discarded
                self.var_types = saved_var_types;
                if let Some(ret_type) = return_type {
                    body_hir.type_id = Some(self.resolve_type_expr(ret_type, registry));
                }
                HirExprKind::Closure {
                    params: params
                        .iter()
                        .map(|(n, t)| {
                            (
                                n.clone(),
                                t.as_ref().map(|tt| self.resolve_type_expr(tt, registry)),
                            )
                        })
                        .collect(),
                    body: Box::new(body_hir),
                }
            }

            ExprKind::Match { values, arms } => {
                let lowered_values: Vec<HirExpr> = values
                    .iter()
                    .map(|v| self.lower_expr_typed(v, registry))
                    .collect();
                let lowered_arms: Vec<HirMatchArm> = arms
                    .iter()
                    .map(|a| {
                        let mut pattern = self.lower_match_pattern_typed(&a.pattern, registry);
                        // Convert struct literal patterns to field-by-field comparisons
                        if let HirMatchPattern::Condition(ref cond_expr) = pattern {
                            if let HirExprKind::Struct { fields, .. } = &cond_expr.kind {
                                if let Some(matched_val) = lowered_values.first() {
                                    pattern = self.struct_pattern_to_condition(
                                        matched_val,
                                        fields,
                                        cond_expr.span,
                                    );
                                }
                            }
                        }
                        HirMatchArm {
                            pattern,
                            guard: a.guard.as_ref().map(|g| self.lower_expr_typed(g, registry)),
                            body: self.lower_expr_typed(&a.body, registry),
                            span: a.span,
                        }
                    })
                    .collect();
                HirExprKind::Match {
                    values: lowered_values,
                    arms: lowered_arms,
                }
            }

            ExprKind::Spread(inner) => {
                HirExprKind::Spread(Box::new(self.lower_expr_typed(inner, registry)))
            }

            ExprKind::RouteBlock { routes } => HirExprKind::RouteBlock {
                routes: routes
                    .iter()
                    .map(|r| self.lower_expr_typed(r, registry))
                    .collect(),
            },

            // === Async & Concurrency ===
            ExprKind::Await(inner) => {
                HirExprKind::Await(Box::new(self.lower_expr_typed(inner, registry)))
            }
            ExprKind::GoSpawn { body } => HirExprKind::Spawn {
                body: Box::new(self.lower_expr_typed(body, registry)),
            },
            ExprKind::ScopeBlock { body } => HirExprKind::ScopeBlock {
                stmts: body
                    .iter()
                    .map(|s| self.lower_stmt_typed(s, registry))
                    .collect(),
            },

            ExprKind::StringInterpolation(parts) => {
                if parts.is_empty() {
                    HirExprKind::Const(ConstValue::Str(String::new()))
                } else {
                    let mut current = self.lower_string_part_typed(&parts[0], registry);
                    for part in &parts[1..] {
                        let next = self.lower_string_part_typed(part, registry);
                        current = HirExpr::new(
                            HirExprKind::BinOp {
                                op: HirBinOp::Add,
                                lhs: Box::new(current),
                                rhs: Box::new(next),
                            },
                            expr.span,
                        );
                        current.type_id = Some(builtin::STR);
                    }
                    current.kind
                }
            }

            ExprKind::Cast {
                expr: inner,
                target,
            } => {
                let inner_hir = self.lower_expr_typed(inner, registry);
                let target_type = self.resolve_type_expr(target, registry);
                HirExprKind::Cast {
                    value: Box::new(inner_hir),
                    to_type: target_type,
                }
            }

            ExprKind::Ternary { .. } => {
                self.errors.push(LowerError::new(
                    "Complex expression not yet lowered",
                    expr.span,
                ));
                HirExprKind::Const(ConstValue::Nil)
            }
        };

        let mut out = HirExpr::new(kind, expr.span);
        if let HirExprKind::Const(c) = &out.kind {
            out.type_id = Some(c.type_id());
        }

        match &mut out.kind {
            HirExprKind::Array(elements) => {
                // Get element type, handling Spread elements specially
                let elem_type = self.common_array_elem_type(elements, registry);
                out.type_id = Some(registry.register_array(elem_type));
            }
            HirExprKind::Map(entries) => {
                let keys: Vec<HirExpr> = entries.iter().map(|(k, _)| k.clone()).collect();
                let vals: Vec<HirExpr> = entries.iter().map(|(_, v)| v.clone()).collect();
                let key_type = self.common_type_or_any(&keys);
                let val_type = self.common_type_or_any(&vals);
                out.type_id = Some(registry.register_map(key_type, val_type));
            }
            HirExprKind::Tuple(elements) => {
                let element_types: Vec<TypeId> = elements
                    .iter()
                    .map(|e| e.type_id.unwrap_or(builtin::ANY))
                    .collect();
                out.type_id = Some(registry.register_tuple(element_types));
            }
            HirExprKind::Struct { name, .. } => {
                // ObjectLit is a dynamic map — use ANY to avoid
                // creating a self-referential TypeRef that hangs is_compatible
                out.type_id = Some(if ffi_names::is_object_lit(name) {
                    builtin::ANY
                } else {
                    registry
                        .lookup(name)
                        .unwrap_or_else(|| registry.declare_named(name))
                });
            }
            HirExprKind::EnumVariant { enum_name, .. } => {
                out.type_id = Some(
                    registry
                        .lookup(enum_name)
                        .unwrap_or_else(|| registry.declare_named(enum_name)),
                );
            }
            HirExprKind::Closure { params, body } => {
                let param_types: Vec<TypeId> = params
                    .iter()
                    .map(|(_, t)| t.unwrap_or(builtin::ANY))
                    .collect();
                let return_type = body.type_id.unwrap_or(builtin::ANY);
                out.type_id = Some(registry.register_function(param_types, return_type));
            }
            HirExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                // Check if this is a module method call (e.g., JSON.parse, JSON.stringify)
                if let Some(return_type) =
                    self.infer_module_method_type(receiver, method, args, registry)
                {
                    out.type_id = Some(return_type);
                } else {
                    let receiver_type = receiver.type_id.unwrap_or(builtin::ANY);
                    if let Some(return_type) =
                        self.infer_method_call_type(receiver_type, method, args, registry)
                    {
                        out.type_id = Some(return_type);
                    }
                }
            }
            HirExprKind::Cast { to_type, .. } => {
                out.type_id = Some(*to_type);
            }
            HirExprKind::UnaryOp { op, operand } => {
                // Infer type for unary operations (e.g., -7 should be Int)
                let operand_type = operand.type_id.unwrap_or(builtin::ANY);
                let op_kind = hir_unaryop_to_kind(*op);
                out.type_id = Some(infer_unaryop_result_type(op_kind, operand_type));
            }
            HirExprKind::BinOp { op, lhs, rhs } => {
                // Infer type for binary operations
                let lhs_type = lhs.type_id.unwrap_or(builtin::ANY);
                let rhs_type = rhs.type_id.unwrap_or(builtin::ANY);
                let op_kind = hir_binop_to_kind(*op);
                out.type_id = Some(infer_binop_result_type(op_kind, lhs_type, rhs_type));
            }
            HirExprKind::Field { object, field } => {
                // Infer field access type from struct type, resolving TypeRef chains
                if let Some(obj_type) = object.type_id {
                    let mut current_type = obj_type;
                    // Follow TypeRef chains (max 10 to prevent infinite loops)
                    for _ in 0..10 {
                        if let Some(info) = registry.get(current_type) {
                            match &info.kind {
                                TypeKind::Struct { fields, .. } => {
                                    if let Some((_, field_type, _)) =
                                        fields.iter().find(|(n, _, _)| n == field)
                                    {
                                        out.type_id = Some(*field_type);
                                    }
                                    break;
                                }
                                TypeKind::TypeRef { name } => {
                                    if let Some(resolved) = registry.lookup(name) {
                                        current_type = resolved;
                                    } else {
                                        break;
                                    }
                                }
                                _ => break,
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
            _ => {}
        }

        out
    }
}
