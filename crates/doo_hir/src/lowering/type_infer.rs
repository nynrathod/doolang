//! Type inference helpers for module methods and closures.

use super::Lower;
use super::{hir_binop_to_kind, hir_unaryop_to_kind};
use crate::types::*;
use doo_core::{
    infer::{infer_binop_result_type, infer_unaryop_result_type, BinOpKind},
    types::{builtin, TypeId, TypeKind, TypeRegistry},
};
use rustc_hash::FxHashMap;

impl Lower {
    /// Infer the return type of module-level method calls (e.g., JSON.stringify, JSON.parse)
    /// For JSON.parse, if the argument is JSON.stringify(x), we can infer the type from x.
    /// Also tracks variables that were assigned from JSON.stringify for indirect inference.
    pub(crate) fn infer_module_method_type(
        &self,
        receiver: &HirExpr,
        method: &str,
        args: &[HirExpr],
        registry: &mut TypeRegistry,
    ) -> Option<TypeId> {
        // Check if receiver is a module identifier
        let module_name = match &receiver.kind {
            HirExprKind::Local { name } => name.as_str(),
            _ => return None,
        };

        match module_name {
            "JSON" => match method {
                "stringify" => Some(builtin::STR),
                "parse" => {
                    // For JSON.parse, try to infer type from the argument
                    if let Some(first_arg) = args.first() {
                        // Case 1: JSON.parse(JSON.stringify(x)) - the arg is a stringify call
                        if let Some(inner_type) = self.extract_stringify_arg_type(first_arg) {
                            return Some(inner_type);
                        }

                        // Case 2: JSON.parse(variable) where variable was assigned from JSON.stringify(x)
                        if let HirExprKind::Local { name } = &first_arg.kind {
                            if let Some(stringify_arg_type) = self.json_stringify_sources.get(name)
                            {
                                return Some(*stringify_arg_type);
                            }
                        }
                    }
                    // Fallback: JSON.parse returns ANY since we can't know the target type here
                    // The actual type will be determined by the context (assignment, return, etc.)
                    Some(builtin::ANY)
                }
                _ => None,
            },
            "Math" => match method {
                "abs" | "floor" | "ceil" | "round" | "sqrt" | "pow" | "sin" | "cos" | "tan"
                | "log" | "exp" | "min" | "max" => Some(builtin::FLOAT),
                "random" => Some(builtin::FLOAT),
                _ => None,
            },
            "Array" => match method {
                "new" => {
                    // Array.new() returns Array[Any]
                    Some(registry.register_array(builtin::ANY))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Extract the type of the argument to JSON.stringify if the expression is a stringify call.
    /// Returns None if the expression is not a JSON.stringify call or the type cannot be determined.
    pub(crate) fn extract_stringify_arg_type(&self, expr: &HirExpr) -> Option<TypeId> {
        if let HirExprKind::MethodCall {
            receiver,
            method,
            args,
        } = &expr.kind
        {
            // Check if this is JSON.stringify
            if method == "stringify" {
                if let HirExprKind::Local { name } = &receiver.kind {
                    if name == "JSON" {
                        // Get the type of the first argument to stringify
                        if let Some(first_arg) = args.first() {
                            return first_arg.type_id;
                        }
                    }
                }
            }
        }
        None
    }

    pub(crate) fn infer_method_call_type(
        &mut self,
        receiver_type: TypeId,
        method: &str,
        args: &mut [HirExpr],
        registry: &mut TypeRegistry,
    ) -> Option<TypeId> {
        let receiver_info = registry.get(receiver_type)?;
        match &receiver_info.kind {
            TypeKind::Array { element } => {
                self.infer_array_method_type(*element, method, args, registry)
            }
            TypeKind::Map { key, value } => {
                self.infer_map_method_type(*key, *value, method, args, registry)
            }
            _ => None,
        }
    }

    pub(crate) fn infer_array_method_type(
        &mut self,
        elem_type: TypeId,
        method: &str,
        args: &mut [HirExpr],
        registry: &mut TypeRegistry,
    ) -> Option<TypeId> {
        match method {
            "len" | "indexOf" => Some(builtin::INT),
            "isEmpty" | "contains" => Some(builtin::BOOL),
            "join" => Some(builtin::STR),
            "first" | "last" | "pop" => Some(elem_type),
            "slice" => Some(registry.register_array(elem_type)),
            "push" | "clear" | "sort" | "reverse" => Some(builtin::VOID),
            "map" => {
                let closure_return = args.get_mut(0).and_then(|arg| {
                    self.apply_closure_signature(arg, &[elem_type], None, registry)
                });
                let out_elem = closure_return.unwrap_or(builtin::ANY);
                Some(registry.register_array(out_elem))
            }
            "filter" => {
                let _ = args.get_mut(0).and_then(|arg| {
                    self.apply_closure_signature(arg, &[elem_type], Some(builtin::BOOL), registry)
                });
                Some(registry.register_array(elem_type))
            }
            "reduce" => {
                let init_type = args
                    .get(0)
                    .and_then(|arg| {
                        arg.type_id.or_else(|| match &arg.kind {
                            HirExprKind::Const(ConstValue::Int(_)) => Some(builtin::INT),
                            HirExprKind::Const(ConstValue::Float(_)) => Some(builtin::FLOAT),
                            HirExprKind::Const(ConstValue::Bool(_)) => Some(builtin::BOOL),
                            HirExprKind::Const(ConstValue::Str(_)) => Some(builtin::STR),
                            _ => None,
                        })
                    })
                    .unwrap_or(builtin::ANY);
                let closure_return = args.get_mut(1).and_then(|arg| {
                    self.apply_closure_signature(
                        arg,
                        &[init_type, elem_type],
                        Some(init_type),
                        registry,
                    )
                });
                if init_type != builtin::ANY {
                    Some(init_type)
                } else {
                    Some(closure_return.unwrap_or(builtin::ANY))
                }
            }
            _ => None,
        }
    }

    pub(crate) fn infer_map_method_type(
        &mut self,
        key_type: TypeId,
        value_type: TypeId,
        method: &str,
        _args: &mut [HirExpr],
        registry: &mut TypeRegistry,
    ) -> Option<TypeId> {
        match method {
            "keys" => {
                // keys() returns Array<K>
                Some(registry.register_array(key_type))
            }
            "values" => {
                // values() returns Array<V>
                Some(registry.register_array(value_type))
            }
            "has" => {
                // has(key) returns bool
                Some(builtin::BOOL)
            }
            "len" => {
                // len() returns int
                Some(builtin::INT)
            }
            "isEmpty" => {
                // isEmpty() returns bool
                Some(builtin::BOOL)
            }
            "clear" | "remove" => {
                // clear() and remove(key) return void
                Some(builtin::VOID)
            }
            _ => None,
        }
    }

    pub(crate) fn apply_closure_signature(
        &mut self,
        expr: &mut HirExpr,
        param_types: &[TypeId],
        return_type_hint: Option<TypeId>,
        registry: &mut TypeRegistry,
    ) -> Option<TypeId> {
        match &mut expr.kind {
            HirExprKind::Closure { params, body } => {
                // Apply param types from context
                for (idx, (_, param_type)) in params.iter_mut().enumerate() {
                    if param_type.is_none() {
                        if let Some(ty) = param_types.get(idx) {
                            *param_type = Some(*ty);
                        }
                    }
                }

                // Build local variable types map for body inference
                let mut locals: FxHashMap<String, TypeId> = FxHashMap::default();
                for (name, type_id) in params.iter() {
                    if let Some(tid) = type_id {
                        locals.insert(name.clone(), *tid);
                    }
                }

                // Infer body type if not set or if it was initially inferred as ANY
                // (which happens when closure param types weren't known during initial lowering,
                // e.g., `(x) => x * x` where both operands are untyped params → Mul(ANY, ANY) = ANY)
                if body.type_id.is_none() || body.type_id == Some(builtin::ANY) {
                    if let Some(ret) = return_type_hint {
                        body.type_id = Some(ret);
                    } else {
                        // Use centralized inference from doo_core
                        let inferred = self.infer_closure_body_type(body, &locals, registry);
                        body.type_id = Some(inferred);
                    }
                }

                let param_ids: Vec<TypeId> = params
                    .iter()
                    .map(|(_, t)| t.unwrap_or(builtin::ANY))
                    .collect();
                let return_type = body.type_id.unwrap_or(builtin::ANY);
                expr.type_id = Some(registry.register_function(param_ids, return_type));
                Some(return_type)
            }
            _ => expr
                .type_id
                .and_then(|type_id| match registry.get(type_id) {
                    Some(info) => match info.kind {
                        TypeKind::Function { returns, .. } => Some(returns),
                        _ => None,
                    },
                    None => None,
                }),
        }
    }

    /// Infer the type of a closure body expression.
    /// Uses centralized inference rules from doo_core::infer (single source of truth).
    pub(crate) fn infer_closure_body_type(
        &self,
        expr: &HirExpr,
        locals: &FxHashMap<String, TypeId>,
        registry: &mut TypeRegistry,
    ) -> TypeId {
        // If already has type, return it
        if let Some(tid) = expr.type_id {
            return tid;
        }

        match &expr.kind {
            HirExprKind::Const(c) => c.type_id(),

            HirExprKind::Local { name } => locals.get(name).copied().unwrap_or(builtin::ANY),

            HirExprKind::BinOp { op, lhs, rhs } => {
                let lhs_type = self.infer_closure_body_type(lhs, locals, registry);
                let rhs_type = self.infer_closure_body_type(rhs, locals, registry);
                let op_kind = hir_binop_to_kind(*op);
                // NullCoalesce (??): unwrap Optional/Result from lhs
                if op_kind == BinOpKind::NullCoalesce {
                    let inner = unwrap_optional_type(registry, lhs_type);
                    if inner != lhs_type {
                        return inner;
                    }
                }
                infer_binop_result_type(op_kind, lhs_type, rhs_type)
            }

            HirExprKind::UnaryOp { op, operand } => {
                let operand_type = self.infer_closure_body_type(operand, locals, registry);
                let op_kind = hir_unaryop_to_kind(*op);
                infer_unaryop_result_type(op_kind, operand_type)
            }

            HirExprKind::Index { object, .. } => {
                let obj_type = self.infer_closure_body_type(object, locals, registry);
                if let Some(info) = registry.get(obj_type) {
                    match &info.kind {
                        TypeKind::Array { element } => *element,
                        TypeKind::Map { value, .. } => *value,
                        TypeKind::Str => builtin::STR,
                        _ => builtin::ANY,
                    }
                } else {
                    builtin::ANY
                }
            }

            HirExprKind::Block { stmts, expr } => {
                // Build extended locals map including block-local let bindings
                let mut block_locals = locals.clone();
                for stmt in stmts.iter() {
                    if let HirStmtKind::Let { name, value, .. } = &stmt.kind {
                        // Infer the type of the let value and add to block_locals
                        let val_type = self.infer_closure_body_type(value, &block_locals, registry);
                        block_locals.insert(name.clone(), val_type);
                    }
                }

                // Priority 1: trailing expression
                if let Some(expr) = expr {
                    return self.infer_closure_body_type(expr, &block_locals, registry);
                }
                // Priority 2: look for return statements (for block closures with explicit return)
                for stmt in stmts.iter() {
                    if let HirStmtKind::Return(values) = &stmt.kind {
                        if let Some(val) = values.first() {
                            return self.infer_closure_body_type(val, &block_locals, registry);
                        }
                    }
                }
                // Priority 3: last expression statement
                if let Some(last) = stmts.last() {
                    if let HirStmtKind::Expr(e) = &last.kind {
                        return self.infer_closure_body_type(e, &block_locals, registry);
                    }
                }
                builtin::VOID
            }

            HirExprKind::If {
                then_expr,
                else_expr,
                ..
            } => {
                let then_type = self.infer_closure_body_type(then_expr, locals, registry);
                if let Some(else_expr) = else_expr {
                    let else_type = self.infer_closure_body_type(else_expr, locals, registry);
                    if then_type == else_type {
                        then_type
                    } else if then_type == builtin::ANY {
                        else_type
                    } else if else_type == builtin::ANY {
                        then_type
                    } else {
                        builtin::ANY
                    }
                } else {
                    builtin::VOID
                }
            }

            _ => builtin::ANY,
        }
    }

    /// Walk the HIR program and infer closure types from function call context.
    /// This handles closures passed as arguments to regular functions (not just array methods).
    pub fn infer_closure_types_in_program(
        &mut self,
        program: &mut HirProgram,
        registry: &mut TypeRegistry,
    ) {
        // Collect function signatures: name -> Vec<(param_name, param_type)>
        let mut fn_sigs: FxHashMap<String, Vec<(String, Option<TypeId>)>> = FxHashMap::default();
        for item in &program.items {
            if let HirItem::Function(f) = item {
                let params: Vec<(String, Option<TypeId>)> = f
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), p.type_id))
                    .collect();
                fn_sigs.insert(f.name.clone(), params);
            }
        }

        // Walk all expressions in function bodies
        for item in &mut program.items {
            if let HirItem::Function(f) = item {
                for stmt in &mut f.body {
                    self.infer_closure_types_in_stmt(stmt, &fn_sigs, registry);
                }
            }
        }
    }

    /// Recursively walk an expression and infer closure types from call context.
    fn infer_closure_types_in_expr(
        &mut self,
        expr: &mut HirExpr,
        fn_sigs: &FxHashMap<String, Vec<(String, Option<TypeId>)>>,
        registry: &mut TypeRegistry,
    ) {
        match &mut expr.kind {
            HirExprKind::Call { func, args } => {
                // Check if func is a known function name
                let fn_name = match &func.kind {
                    HirExprKind::Local { name } => name.clone(),
                    _ => String::new(),
                };

                if let Some(param_types) = fn_sigs.get(&fn_name) {
                    // For each argument that is a closure, infer types from expected param type
                    for (i, arg) in args.iter_mut().enumerate() {
                        if matches!(arg.kind, HirExprKind::Closure { .. }) {
                            // Extract expected parameter/return types from the function signature
                            let expected_info: Option<(Vec<TypeId>, Option<TypeId>)> =
                                param_types.get(i).and_then(|(_, opt_type)| {
                                    opt_type.and_then(|param_type| {
                                        registry.get(param_type).and_then(|info| {
                                            if let TypeKind::Function {
                                                params: ref expected_params,
                                                returns,
                                            } = &info.kind
                                            {
                                                Some((expected_params.clone(), Some(*returns)))
                                            } else {
                                                None
                                            }
                                        })
                                    })
                                });

                            if let Some((expected_params, expected_returns)) = expected_info {
                                self.apply_closure_signature(
                                    arg,
                                    &expected_params,
                                    expected_returns,
                                    registry,
                                );
                            }
                        }
                        // Recurse into arguments (they might contain nested closures)
                        self.infer_closure_types_in_expr(arg, fn_sigs, registry);
                    }
                }
                // Recurse into func (might be a complex expression)
                self.infer_closure_types_in_expr(func, fn_sigs, registry);
            }

            HirExprKind::Closure { body, .. } => {
                // Recurse into closure body
                self.infer_closure_types_in_expr(body, fn_sigs, registry);
            }

            HirExprKind::Block { stmts, expr: block_expr } => {
                for stmt in stmts.iter_mut() {
                    self.infer_closure_types_in_stmt(stmt, fn_sigs, registry);
                }
                if let Some(e) = block_expr {
                    self.infer_closure_types_in_expr(e, fn_sigs, registry);
                }
            }

            HirExprKind::BinOp { lhs, rhs, .. } => {
                self.infer_closure_types_in_expr(lhs, fn_sigs, registry);
                self.infer_closure_types_in_expr(rhs, fn_sigs, registry);
            }

            HirExprKind::UnaryOp { operand, .. } => {
                self.infer_closure_types_in_expr(operand, fn_sigs, registry);
            }

            HirExprKind::If { condition, then_expr, else_expr } => {
                self.infer_closure_types_in_expr(condition, fn_sigs, registry);
                self.infer_closure_types_in_expr(then_expr, fn_sigs, registry);
                if let Some(e) = else_expr {
                    self.infer_closure_types_in_expr(e, fn_sigs, registry);
                }
            }

            HirExprKind::Array(elements) => {
                for elem in elements.iter_mut() {
                    self.infer_closure_types_in_expr(elem, fn_sigs, registry);
                }
            }

            HirExprKind::Map(entries) => {
                for (k, v) in entries.iter_mut() {
                    self.infer_closure_types_in_expr(k, fn_sigs, registry);
                    self.infer_closure_types_in_expr(v, fn_sigs, registry);
                }
            }

            HirExprKind::MethodCall { receiver, args, .. } => {
                self.infer_closure_types_in_expr(receiver, fn_sigs, registry);
                for arg in args.iter_mut() {
                    self.infer_closure_types_in_expr(arg, fn_sigs, registry);
                }
            }

            HirExprKind::Match { values, arms } => {
                for v in values.iter_mut() {
                    self.infer_closure_types_in_expr(v, fn_sigs, registry);
                }
                for arm in arms.iter_mut() {
                    self.infer_closure_types_in_expr(&mut arm.body, fn_sigs, registry);
                }
            }

            HirExprKind::Struct { fields, .. } => {
                for (_, v) in fields.iter_mut() {
                    self.infer_closure_types_in_expr(v, fn_sigs, registry);
                }
            }

            HirExprKind::Field { object, .. } => {
                self.infer_closure_types_in_expr(object, fn_sigs, registry);
            }

            HirExprKind::Index { object, index } => {
                self.infer_closure_types_in_expr(object, fn_sigs, registry);
                self.infer_closure_types_in_expr(index, fn_sigs, registry);
            }

            HirExprKind::Cast { value, .. } => {
                self.infer_closure_types_in_expr(value, fn_sigs, registry);
            }

            HirExprKind::Try(inner)
            | HirExprKind::Clone(inner)
            | HirExprKind::Move(inner)
            | HirExprKind::Borrow { expr: inner, .. }
            | HirExprKind::Spread(inner)
            | HirExprKind::Await(inner)
            | HirExprKind::Ok(inner)
            | HirExprKind::Err(inner) => {
                self.infer_closure_types_in_expr(inner, fn_sigs, registry);
            }

            HirExprKind::Range { start, end, .. } => {
                self.infer_closure_types_in_expr(start, fn_sigs, registry);
                self.infer_closure_types_in_expr(end, fn_sigs, registry);
            }

            HirExprKind::UnwrapOrPanic { expr: inner, message } => {
                self.infer_closure_types_in_expr(inner, fn_sigs, registry);
                self.infer_closure_types_in_expr(message, fn_sigs, registry);
            }

            HirExprKind::Tuple(elements) => {
                for elem in elements.iter_mut() {
                    self.infer_closure_types_in_expr(elem, fn_sigs, registry);
                }
            }

            HirExprKind::Spawn { body } => {
                self.infer_closure_types_in_expr(body, fn_sigs, registry);
            }

            HirExprKind::ScopeBlock { stmts } => {
                for stmt in stmts.iter_mut() {
                    self.infer_closure_types_in_stmt(stmt, fn_sigs, registry);
                }
            }

            // Expressions that don't contain sub-expressions
            HirExprKind::Const(_)
            | HirExprKind::Local { .. }
            | HirExprKind::Global { .. }
            | HirExprKind::RouteBlock { .. } => {}

            _ => {}
        }
    }

    /// Helper to recurse into statements.
    fn infer_closure_types_in_stmt(
        &mut self,
        stmt: &mut HirStmt,
        fn_sigs: &FxHashMap<String, Vec<(String, Option<TypeId>)>>,
        registry: &mut TypeRegistry,
    ) {
        match &mut stmt.kind {
            HirStmtKind::Let { value, .. } => {
                self.infer_closure_types_in_expr(value, fn_sigs, registry);
            }
            HirStmtKind::Expr(e) => {
                self.infer_closure_types_in_expr(e, fn_sigs, registry);
            }
            HirStmtKind::Return(values) => {
                for v in values.iter_mut() {
                    self.infer_closure_types_in_expr(v, fn_sigs, registry);
                }
            }
            HirStmtKind::Assign { value, .. } => {
                self.infer_closure_types_in_expr(value, fn_sigs, registry);
            }
            _ => {}
        }
    }
}

/// Unwrap Optional/Result types to get the inner value type.
/// For `T?` (Optional), returns the inner `T`.
/// For `T ! E` (Result), returns the ok type `T`.
/// Otherwise returns the type unchanged.
pub(crate) fn unwrap_optional_type(registry: &TypeRegistry, type_id: TypeId) -> TypeId {
    if let Some(info) = registry.get(type_id) {
        match &info.kind {
            TypeKind::Optional { inner } => return *inner,
            TypeKind::Result { ok, .. } => return *ok,
            _ => {}
        }
    }
    type_id
}
