//! Pattern matching, operators, and utility methods.

use doo_core::{
    types::{builtin, TypeId, TypeKind, TypeRegistry},
    Span,
};
use doo_frontend::ast::{
    self, BinaryOp, CompoundOp, Pattern, PatternKind,
    TypeExpr, UnaryOp,
};
use crate::types::*;
use super::Lower;

impl Lower {
    pub(crate) fn pattern_to_name(&self, pattern: &Pattern) -> String {
        match &pattern.kind {
            PatternKind::Ident(name) => name.clone(),
            PatternKind::Wildcard => "_".to_string(),
            PatternKind::Tuple(_) => "__tuple".to_string(),
            PatternKind::Index { object, .. } => {
                // For index patterns, use the base object name
                self.pattern_to_name(object)
            }
            PatternKind::Field { object, .. } => {
                // For field patterns, use the base object name
                self.pattern_to_name(object)
            }
        }
    }

    pub(crate) fn pattern_to_expr(&mut self, pattern: &Pattern) -> HirExpr {
        match &pattern.kind {
            PatternKind::Ident(name) => {
                HirExpr::new(HirExprKind::Local { name: name.clone() }, pattern.span)
            }
            PatternKind::Wildcard => HirExpr::new(
                HirExprKind::Local {
                    name: "_".to_string(),
                },
                pattern.span,
            ),
            PatternKind::Tuple(patterns) => {
                let exprs = patterns.iter().map(|p| self.pattern_to_expr(p)).collect();
                HirExpr::new(HirExprKind::Tuple(exprs), pattern.span)
            }
            PatternKind::Index { object, index } => {
                let object_expr = self.pattern_to_expr(object);
                let index_expr = self.lower_expr(index);
                HirExpr::new(
                    HirExprKind::Index {
                        object: Box::new(object_expr),
                        index: Box::new(index_expr),
                    },
                    pattern.span,
                )
            }
            PatternKind::Field { object, field } => {
                let object_expr = self.pattern_to_expr(object);
                HirExpr::new(
                    HirExprKind::Field {
                        object: Box::new(object_expr),
                        field: field.clone(),
                    },
                    pattern.span,
                )
            }
        }
    }

    pub(crate) fn pattern_to_names(&self, pattern: &Pattern) -> Vec<String> {
        match &pattern.kind {
            PatternKind::Tuple(patterns) => {
                patterns.iter().map(|p| self.pattern_to_name(p)).collect()
            }
            _ => vec![self.pattern_to_name(pattern)],
        }
    }

    pub(crate) fn lower_binop(&self, op: BinaryOp) -> HirBinOp {
        match op {
            BinaryOp::Add => HirBinOp::Add,
            BinaryOp::Sub => HirBinOp::Sub,
            BinaryOp::Mul => HirBinOp::Mul,
            BinaryOp::Div => HirBinOp::Div,
            BinaryOp::Mod => HirBinOp::Mod,
            BinaryOp::Eq => HirBinOp::Eq,
            BinaryOp::NotEq => HirBinOp::NotEq,
            BinaryOp::Lt => HirBinOp::Lt,
            BinaryOp::Gt => HirBinOp::Gt,
            BinaryOp::LtEq => HirBinOp::LtEq,
            BinaryOp::GtEq => HirBinOp::GtEq,
            BinaryOp::And => HirBinOp::And,
            BinaryOp::Or => HirBinOp::Or,
            BinaryOp::BitAnd => HirBinOp::BitAnd,
            BinaryOp::BitOr => HirBinOp::BitOr,
            BinaryOp::NullCoalesce => HirBinOp::Or, // Simplify for now
            BinaryOp::In => HirBinOp::In,
        }
    }

    pub(crate) fn lower_match_pattern(&mut self, p: &ast::MatchPattern) -> HirMatchPattern {
        match p {
            ast::MatchPattern::Literal(e) => HirMatchPattern::Literal(Box::new(self.lower_expr(e))),
            ast::MatchPattern::Condition(e) => {
                HirMatchPattern::Condition(Box::new(self.lower_expr(e)))
            }
            ast::MatchPattern::Wildcard => HirMatchPattern::Wildcard,
            ast::MatchPattern::EnumVariant { enum_name, variant } => HirMatchPattern::EnumVariant {
                enum_name: enum_name.clone(),
                variant: variant.clone(),
            },
            ast::MatchPattern::EnumVariantPayload {
                enum_name,
                variant,
                bindings,
            } => HirMatchPattern::EnumVariantPayload {
                enum_name: enum_name.clone(),
                variant: variant.clone(),
                bindings: bindings.clone(),
            },
            ast::MatchPattern::Tuple(parts) => {
                HirMatchPattern::Tuple(parts.iter().map(|x| self.lower_match_pattern(x)).collect())
            }
        }
    }

    pub(crate) fn lower_match_pattern_typed(
        &mut self,
        p: &ast::MatchPattern,
        registry: &mut TypeRegistry,
    ) -> HirMatchPattern {
        match p {
            ast::MatchPattern::Literal(e) => {
                HirMatchPattern::Literal(Box::new(self.lower_expr_typed(e, registry)))
            }
            ast::MatchPattern::Condition(e) => {
                HirMatchPattern::Condition(Box::new(self.lower_expr_typed(e, registry)))
            }
            ast::MatchPattern::Wildcard => HirMatchPattern::Wildcard,
            ast::MatchPattern::EnumVariant { enum_name, variant } => HirMatchPattern::EnumVariant {
                enum_name: enum_name.clone(),
                variant: variant.clone(),
            },
            ast::MatchPattern::EnumVariantPayload {
                enum_name,
                variant,
                bindings,
            } => HirMatchPattern::EnumVariantPayload {
                enum_name: enum_name.clone(),
                variant: variant.clone(),
                bindings: bindings.clone(),
            },
            ast::MatchPattern::Tuple(parts) => HirMatchPattern::Tuple(
                parts
                    .iter()
                    .map(|x| self.lower_match_pattern_typed(x, registry))
                    .collect(),
            ),
        }
    }

    /// Convert a struct literal match pattern to a field-by-field comparison condition.
    ///
    /// Transforms `Point { x: 10, y: 20 }` into `matched.x == 10 && matched.y == 20`.
    pub(crate) fn struct_pattern_to_condition(
        &self,
        matched_val: &HirExpr,
        fields: &[(String, HirExpr)],
        span: Span,
    ) -> HirMatchPattern {
        if fields.is_empty() {
            // No fields to compare → always matches
            return HirMatchPattern::Wildcard;
        }

        // Build comparisons: matched.field == value
        let mut comparisons: Vec<HirExpr> = Vec::new();
        for (field_name, field_val) in fields {
            let field_access = HirExpr::new(
                HirExprKind::Field {
                    object: Box::new(matched_val.clone()),
                    field: field_name.clone(),
                },
                span,
            );
            let eq_check = HirExpr::new(
                HirExprKind::BinOp {
                    op: HirBinOp::Eq,
                    lhs: Box::new(field_access),
                    rhs: Box::new(field_val.clone()),
                },
                span,
            );
            comparisons.push(eq_check);
        }

        // Chain comparisons with &&
        let mut result = comparisons.remove(0);
        for comp in comparisons {
            result = HirExpr::new(
                HirExprKind::BinOp {
                    op: HirBinOp::And,
                    lhs: Box::new(result),
                    rhs: Box::new(comp),
                },
                span,
            );
        }

        HirMatchPattern::Condition(Box::new(result))
    }

    pub(crate) fn resolve_type_expr(&mut self, ty: &TypeExpr, registry: &mut TypeRegistry) -> TypeId {
        match &ty.kind {
            doo_frontend::ast::TypeExprKind::Named(name) => {
                // Check if this name is a registered type parameter first.
                // Type params are stored as "__typeparam_T" in the registry.
                let tp_key = format!("__typeparam_{}", name);
                if let Some(tp_id) = registry.lookup(&tp_key) {
                    tp_id
                } else {
                    registry
                        .lookup(name)
                        .unwrap_or_else(|| registry.declare_named(name))
                }
            }
            doo_frontend::ast::TypeExprKind::Array(inner) => {
                let elem = self.resolve_type_expr(inner, registry);
                registry.register_array(elem)
            }
            doo_frontend::ast::TypeExprKind::Map(k, v) => {
                let key = self.resolve_type_expr(k, registry);
                let value = self.resolve_type_expr(v, registry);
                registry.register_map(key, value)
            }
            doo_frontend::ast::TypeExprKind::Tuple(parts) => {
                let elements = parts
                    .iter()
                    .map(|p| self.resolve_type_expr(p, registry))
                    .collect();
                registry.register_tuple(elements)
            }
            doo_frontend::ast::TypeExprKind::Optional(inner) => {
                let inner_id = self.resolve_type_expr(inner, registry);
                registry.register_optional(inner_id)
            }
            doo_frontend::ast::TypeExprKind::Result(ok, err) => {
                let ok_id = self.resolve_type_expr(ok, registry);
                let err_id = self.resolve_type_expr(err, registry);
                registry.register_result(ok_id, err_id)
            }
            doo_frontend::ast::TypeExprKind::Function { params, returns } => {
                let params_ids = params
                    .iter()
                    .map(|p| self.resolve_type_expr(p, registry))
                    .collect();
                let returns_id = self.resolve_type_expr(returns, registry);
                registry.register_function(params_ids, returns_id)
            }
            doo_frontend::ast::TypeExprKind::Range(_inner) => registry.declare_named("Range"),
            doo_frontend::ast::TypeExprKind::Any => builtin::ANY,
            doo_frontend::ast::TypeExprKind::Void => builtin::VOID,
            doo_frontend::ast::TypeExprKind::Error => builtin::ERROR,
        }
    }

    /// Determine the common element type for an array literal.
    /// Handles Spread elements by extracting the element type from the spread source.
    pub(crate) fn common_array_elem_type(&self, elements: &[HirExpr], registry: &TypeRegistry) -> TypeId {
        let mut current: Option<TypeId> = None;
        for e in elements {
            // For Spread elements, extract the element type from the inner array
            let elem_type = if let HirExprKind::Spread(inner) = &e.kind {
                // Get the inner expression's type (should be an array)
                inner.type_id.and_then(|arr_type| {
                    registry.get(arr_type).and_then(|info| {
                        if let TypeKind::Array { element } = &info.kind {
                            Some(*element)
                        } else {
                            None
                        }
                    })
                })
            } else {
                // For non-spread elements, use the element's type directly
                e.type_id
            };

            let Some(t) = elem_type else {
                return builtin::ANY;
            };
            match current {
                None => current = Some(t),
                Some(existing) if existing == t => {}
                Some(_) => return builtin::ANY,
            }
        }
        current.unwrap_or(builtin::ANY)
    }

    pub(crate) fn common_type_or_any(&self, exprs: &[HirExpr]) -> TypeId {
        let mut current: Option<TypeId> = None;
        for e in exprs {
            let Some(t) = e.type_id else {
                return builtin::ANY;
            };
            match current {
                None => current = Some(t),
                Some(existing) if existing == t => {}
                Some(_) => return builtin::ANY,
            }
        }
        current.unwrap_or(builtin::ANY)
    }

    pub(crate) fn lower_unaryop(&self, op: UnaryOp) -> HirUnaryOp {
        match op {
            UnaryOp::Neg => HirUnaryOp::Neg,
            UnaryOp::Not => HirUnaryOp::Not,
        }
    }

    pub(crate) fn compound_op_to_binop(&self, op: CompoundOp) -> HirBinOp {
        match op {
            CompoundOp::Add => HirBinOp::Add,
            CompoundOp::Sub => HirBinOp::Sub,
            CompoundOp::Mul => HirBinOp::Mul,
            CompoundOp::Div => HirBinOp::Div,
            CompoundOp::Mod => HirBinOp::Mod,
        }
    }

    pub(crate) fn lower_string_part(&mut self, part: &ast::StringPart) -> HirExpr {
        match part {
            ast::StringPart::Literal(s) => HirExpr::new(
                HirExprKind::Const(ConstValue::Str(s.clone())),
                Span::dummy(),
            ),
            ast::StringPart::Expr(e) => {
                // Should cast to string, but Cast not fully lowered in untyped pass.
                // Assuming untyped pass relies on implicit behavior or basic lowering.
                self.lower_expr(e)
            }
        }
    }

    pub(crate) fn lower_string_part_typed(
        &mut self,
        part: &ast::StringPart,
        registry: &mut TypeRegistry,
    ) -> HirExpr {
        match part {
            ast::StringPart::Literal(s) => HirExpr::with_type(
                HirExprKind::Const(ConstValue::Str(s.clone())),
                builtin::STR,
                Span::dummy(),
            ),
            ast::StringPart::Expr(e) => {
                let expr_hir = self.lower_expr_typed(e, registry);
                if expr_hir.type_id == Some(builtin::STR) {
                    expr_hir
                } else {
                    // Cast to String — use the expression's span for error reporting
                    let span = expr_hir.span;
                    HirExpr::with_type(
                        HirExprKind::Cast {
                            value: Box::new(expr_hir),
                            to_type: builtin::STR,
                        },
                        builtin::STR,
                        span,
                    )
                }
            }
        }
    }
}
