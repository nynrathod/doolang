//! Type inference helpers for module methods and closures.

use doo_core::{
    doo_debug,
    infer::{infer_binop_result_type, infer_unaryop_result_type, BinOpKind, UnaryOpKind},
    types::{builtin, TypeId, TypeKind, TypeRegistry},
    Span,
};
use doo_frontend::ast::{
    self, BinaryOp, CompoundOp, Decorator, ElseBranch, EnumDecl, Expr, ExprKind, FunctionDecl,
    ImportDecl, IncDecOp, Item, Pattern, PatternKind, Program, Stmt, StmtKind, StructDecl,
    TypeExpr, UnaryOp,
};
use crate::types::*;
use super::{Lower, LowerError};
use rustc_hash::FxHashMap;
use super::{hir_binop_to_kind, hir_unaryop_to_kind};

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
                    .and_then(|arg| arg.type_id)
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
                // Convert HirBinOp to BinOpKind and use centralized inference
                let op_kind = hir_binop_to_kind(*op);
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
}
