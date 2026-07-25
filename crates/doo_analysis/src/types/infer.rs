//! Type Inference
//!
//! Infers types for expressions and statements, with special handling for closures.

use doo_core::infer::{infer_binop_result_type, infer_unaryop_result_type, BinOpKind, UnaryOpKind};
use doo_core::types::builtin;
use doo_core::types::registry::{TypeId, TypeKind, TypeRegistry};
use doo_hir::HirExpr;
use std::collections::HashMap;

/// Type inference engine.
pub struct TypeInference {
    constraints: Vec<TypeConstraint>,
    locals: HashMap<String, TypeId>,
    functions: HashMap<String, TypeId>,
}

#[derive(Debug, Clone)]
pub struct TypeConstraint {
    pub lhs: TypeId,
    pub rhs: TypeId,
}

#[derive(Debug, Clone)]
pub struct ClosureContext {
    pub param_types: Vec<TypeId>,
    pub expected_return: Option<TypeId>,
}

pub enum InferenceError {
    Mismatch(TypeId, TypeId),
    NotAClosure,
}

impl TypeInference {
    pub fn new() -> Self {
        Self {
            constraints: Vec::new(),
            locals: HashMap::new(),
            functions: HashMap::new(),
        }
    }

    pub fn register_function(&mut self, name: String, return_type: TypeId) {
        self.functions.insert(name, return_type);
    }

    pub fn constrain(&mut self, lhs: TypeId, rhs: TypeId) {
        self.constraints.push(TypeConstraint { lhs, rhs });
    }

    pub fn solve(&self) -> Result<(), InferenceError> {
        for c in &self.constraints {
            if !self.unify(c.lhs, c.rhs) {
                return Err(InferenceError::Mismatch(c.lhs, c.rhs));
            }
        }
        Ok(())
    }

    fn unify(&self, a: TypeId, b: TypeId) -> bool {
        if a == b {
            return true;
        }
        if a == builtin::ANY || b == builtin::ANY {
            return true;
        }
        false
    }

    pub fn infer_expr_type(&mut self, expr: &mut HirExpr, registry: &mut TypeRegistry) -> TypeId {
        if let Some(t) = expr.type_id {
            return t;
        }

        let inferred = match &mut expr.kind {
            doo_hir::HirExprKind::Const(c) => c.type_id(),
            doo_hir::HirExprKind::Local { name } => {
                self.locals.get(name).copied().unwrap_or(builtin::ANY)
            }
            doo_hir::HirExprKind::BinOp { op, lhs, rhs, .. } => {
                let lhs_type = self.infer_expr_type(lhs, registry);
                let rhs_type = self.infer_expr_type(rhs, registry);
                let kind = match op {
                    doo_hir::HirBinOp::Add => BinOpKind::Add,
                    doo_hir::HirBinOp::Sub => BinOpKind::Sub,
                    doo_hir::HirBinOp::Mul => BinOpKind::Mul,
                    doo_hir::HirBinOp::Div => BinOpKind::Div,
                    doo_hir::HirBinOp::Mod => BinOpKind::Mod,
                    doo_hir::HirBinOp::Eq => BinOpKind::Eq,
                    doo_hir::HirBinOp::NotEq => BinOpKind::NotEq,
                    doo_hir::HirBinOp::Lt => BinOpKind::Lt,
                    doo_hir::HirBinOp::Gt => BinOpKind::Gt,
                    doo_hir::HirBinOp::LtEq => BinOpKind::LtEq,
                    doo_hir::HirBinOp::GtEq => BinOpKind::GtEq,
                    doo_hir::HirBinOp::And => BinOpKind::And,
                    doo_hir::HirBinOp::Or => BinOpKind::Or,
                    doo_hir::HirBinOp::BitAnd => BinOpKind::BitAnd,
                    doo_hir::HirBinOp::BitOr => BinOpKind::BitOr,
                    doo_hir::HirBinOp::BitXor => BinOpKind::BitXor,
                    doo_hir::HirBinOp::In => BinOpKind::In,
                    doo_hir::HirBinOp::NullCoalesce => BinOpKind::NullCoalesce,
                };
                infer_binop_result_type(kind, lhs_type, rhs_type)
            }
            doo_hir::HirExprKind::UnaryOp { op, operand, .. } => {
                let operand_type = self.infer_expr_type(operand, registry);
                let kind = match op {
                    doo_hir::HirUnaryOp::Neg => UnaryOpKind::Neg,
                    doo_hir::HirUnaryOp::Not => UnaryOpKind::Not,
                };
                infer_unaryop_result_type(kind, operand_type)
            }
            doo_hir::HirExprKind::Array(elements) => {
                if elements.is_empty() {
                    return registry.register_array(builtin::ANY);
                }
                let first_type = self.infer_expr_type(&mut elements[0].clone(), registry);
                for elem in elements.iter_mut().skip(1) {
                    let elem_type = self.infer_expr_type(elem, registry);
                    self.constrain(elem_type, first_type);
                }
                registry.register_array(first_type)
            }
            doo_hir::HirExprKind::Index { object, .. } => {
                let object_type = self.infer_expr_type(object, registry);
                if let Some(info) = registry.get(object_type) {
                    match &info.kind {
                        TypeKind::Array { element } => return *element,
                        TypeKind::Map { value, .. } => return *value,
                        TypeKind::Str => return builtin::STR,
                        _ => {}
                    }
                }
                builtin::ANY
            }
            doo_hir::HirExprKind::Map(entries) => {
                if entries.is_empty() {
                    return registry.register_map(builtin::ANY, builtin::ANY);
                }
                let (first_key, first_val) = &mut entries[0].clone();
                let key_type = self.infer_expr_type(first_key, registry);
                let val_type = self.infer_expr_type(first_val, registry);
                for (k, v) in entries.iter_mut().skip(1) {
                    let k_type = self.infer_expr_type(k, registry);
                    let v_type = self.infer_expr_type(v, registry);
                    self.constrain(k_type, key_type);
                    self.constrain(v_type, val_type);
                }
                registry.register_map(key_type, val_type)
            }
            doo_hir::HirExprKind::Field { object, field, .. } => {
                let obj_type = self.infer_expr_type(object, registry);
                if let Some(info) = registry.get(obj_type) {
                    if let TypeKind::Struct { def, .. } = &info.kind {
                        if let Some(f) = def
                            .fields
                            .iter()
                            .find(|f| f.name.resolve() == field.as_str())
                        {
                            return f.type_id;
                        }
                    }
                }
                builtin::ANY
            }
            doo_hir::HirExprKind::Block { stmts, expr, .. } => {
                for stmt in stmts.iter_mut() {
                    self.process_stmt_for_inference(stmt, registry);
                }
                if let Some(e) = expr {
                    self.infer_expr_type(e, registry)
                } else {
                    builtin::VOID
                }
            }
            doo_hir::HirExprKind::If {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                let _ = self.infer_expr_type(condition, registry);
                let then_type = self.infer_expr_type(then_expr, registry);
                if let Some(e) = else_expr {
                    let else_type = self.infer_expr_type(e, registry);
                    if then_type != builtin::ANY {
                        then_type
                    } else {
                        else_type
                    }
                } else {
                    builtin::VOID
                }
            }
            doo_hir::HirExprKind::Call { func, args, .. } => {
                for arg in args.iter_mut() {
                    self.infer_expr_type(arg, registry);
                }
                if let doo_hir::HirExprKind::Local { name } = &func.kind {
                    if let Some(&ret_type) = self.functions.get(name) {
                        return ret_type;
                    }
                }
                builtin::ANY
            }
            doo_hir::HirExprKind::Tuple(elements) => {
                let elem_types: Vec<TypeId> = elements
                    .iter_mut()
                    .map(|e| self.infer_expr_type(e, registry))
                    .collect();
                registry.register_tuple(elem_types)
            }
            doo_hir::HirExprKind::Ok(inner) => {
                let inner_type = self.infer_expr_type(inner, registry);
                registry.register_result(inner_type, builtin::ERROR)
            }
            doo_hir::HirExprKind::Err(inner) => {
                let inner_type = self.infer_expr_type(inner, registry);
                registry.register_result(builtin::ANY, inner_type)
            }
            doo_hir::HirExprKind::Try(inner) => {
                let inner_type = self.infer_expr_type(inner, registry);
                if let Some(info) = registry.get(inner_type) {
                    if let TypeKind::Result { ok, .. } = &info.kind {
                        return *ok;
                    }
                }
                builtin::ANY
            }
            doo_hir::HirExprKind::Cast { to_type, .. } => *to_type,
            _ => builtin::ANY,
        };

        expr.type_id = Some(inferred);
        inferred
    }

    pub fn process_stmt_for_inference(
        &mut self,
        stmt: &mut doo_hir::HirStmt,
        registry: &mut TypeRegistry,
    ) {
        match &mut stmt.kind {
            doo_hir::HirStmtKind::Let {
                name,
                value,
                type_id,
                ..
            } => {
                let value_type = self.infer_expr_type(value, registry);
                let var_type = type_id.unwrap_or(value_type);
                self.locals.insert(name.clone(), var_type);
                if type_id.is_none() {
                    *type_id = Some(value_type);
                }
            }
            doo_hir::HirStmtKind::TupleLet {
                names,
                value,
                type_ids,
                ..
            } => {
                let value_type = self.infer_expr_type(value, registry);
                let element_types: Vec<TypeId> = if let Some(info) = registry.get(value_type) {
                    if let TypeKind::Tuple { elements } = &info.kind {
                        elements.clone()
                    } else {
                        vec![builtin::ANY; names.len()]
                    }
                } else {
                    vec![builtin::ANY; names.len()]
                };
                for (i, name) in names.iter().enumerate() {
                    let var_type = element_types.get(i).copied().unwrap_or(builtin::ANY);
                    self.locals.insert(name.clone(), var_type);
                    if type_ids.get(i).map(|t| t.is_none()).unwrap_or(true) {
                        if i < type_ids.len() {
                            type_ids[i] = Some(var_type);
                        }
                    }
                }
            }
            doo_hir::HirStmtKind::Expr(expr) => {
                self.infer_expr_type(expr, registry);
            }
            doo_hir::HirStmtKind::Assign { value, .. } => {
                self.infer_expr_type(value, registry);
            }
            doo_hir::HirStmtKind::Return(exprs) => {
                for expr in exprs.iter_mut() {
                    self.infer_expr_type(expr, registry);
                }
            }
            doo_hir::HirStmtKind::If {
                condition,
                then_block,
                ..
            } => {
                self.infer_expr_type(condition, registry);
                for s in then_block.iter_mut() {
                    self.process_stmt_for_inference(s, registry);
                }
            }
            _ => {}
        }
    }

    pub fn define_local(&mut self, name: String, type_id: TypeId) {
        self.locals.insert(name, type_id);
    }

    pub fn lookup_local(&self, name: &str) -> Option<TypeId> {
        self.locals.get(name).copied()
    }

    pub fn push_scope(&self) -> HashMap<String, TypeId> {
        self.locals.clone()
    }

    pub fn pop_scope(&mut self, saved: HashMap<String, TypeId>) {
        self.locals = saved;
    }
}

impl Default for TypeInference {
    fn default() -> Self {
        Self::new()
    }
}

// Helper functions for method return type inference (kept for doo_analysis)
pub fn infer_method_return_type(
    registry: &mut TypeRegistry,
    receiver_type: TypeId,
    method: &str,
) -> Option<TypeId> {
    let info = registry.get(receiver_type)?;
    match &info.kind {
        TypeKind::Array { element } => match method {
            "len" | "indexOf" => Some(builtin::INT),
            "isEmpty" | "contains" => Some(builtin::BOOL),
            "join" => Some(builtin::STR),
            "first" | "last" | "pop" => Some(*element),
            "slice" => Some(registry.register_array(*element)),
            "push" | "clear" | "sort" | "reverse" => Some(builtin::VOID),
            _ => None,
        },
        TypeKind::Map { key, value, .. } => match method {
            "keys" => Some(registry.register_array(*key)),
            "values" => Some(registry.register_array(*value)),
            "has" => Some(builtin::BOOL),
            "len" => Some(builtin::INT),
            "isEmpty" => Some(builtin::BOOL),
            "clear" | "remove" => Some(builtin::VOID),
            "get" => Some(*value),
            _ => None,
        },
        TypeKind::Str => match method {
            "len" | "indexOf" | "charCode" => Some(builtin::INT),
            "isEmpty" | "contains" | "startsWith" | "endsWith" => Some(builtin::BOOL),
            "split" => Some(registry.register_array(builtin::STR)),
            "charAt" | "substring" | "concat" | "toUpper" | "toLower" | "replace" | "trim"
            | "reverse" | "repeat" => Some(builtin::STR),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use doo_core::Span;

    #[test]
    fn test_infer_binop_int() {
        let lhs = doo_core::types::builtin::INT;
        let rhs = doo_core::types::builtin::INT;
        let result = infer_binop_result_type(BinOpKind::Add, lhs, rhs);
        assert_eq!(result, doo_core::types::builtin::INT);
    }

    #[test]
    fn test_infer_binop_float() {
        let lhs = doo_core::types::builtin::INT;
        let rhs = doo_core::types::builtin::FLOAT;
        let result = infer_binop_result_type(BinOpKind::Add, lhs, rhs);
        assert_eq!(result, doo_core::types::builtin::FLOAT);
    }

    #[test]
    fn test_infer_comparison_bool() {
        let lhs = doo_core::types::builtin::INT;
        let rhs = doo_core::types::builtin::INT;
        let result = infer_binop_result_type(BinOpKind::Eq, lhs, rhs);
        assert_eq!(result, doo_core::types::builtin::BOOL);
    }

    #[test]
    fn test_infer_unary_neg() {
        let result = infer_unaryop_result_type(UnaryOpKind::Neg, doo_core::types::builtin::INT);
        assert_eq!(result, doo_core::types::builtin::INT);
    }

    #[test]
    fn test_infer_unary_not() {
        let result = infer_unaryop_result_type(UnaryOpKind::Not, doo_core::types::builtin::INT);
        assert_eq!(result, doo_core::types::builtin::BOOL);
    }

    #[test]
    fn test_infer_array_index_type() {
        use doo_hir::ConstValue;
        let mut registry = TypeRegistry::new();
        let mut inf = TypeInference::new();
        let array_type = registry.register_array(doo_core::types::builtin::INT);
        let mut arr_expr = HirExpr::new(
            doo_hir::HirExprKind::Local {
                name: "arr".to_string(),
            },
            Span::dummy(),
        );
        arr_expr.type_id = Some(array_type);
        let mut index_access = HirExpr::new(
            doo_hir::HirExprKind::Index {
                object: Box::new(arr_expr),
                index: Box::new(HirExpr::new(
                    doo_hir::HirExprKind::Const(ConstValue::Int(0)),
                    Span::dummy(),
                )),
            },
            Span::dummy(),
        );
        let result_type = inf.infer_expr_type(&mut index_access, &mut registry);
        assert_eq!(result_type, doo_core::types::builtin::INT);
    }
}
