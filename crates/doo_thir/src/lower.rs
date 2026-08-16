//! HIR to THIR Lowering
//!
//! Converts HIR nodes to THIR, resolving all types and trait/interface methods.
//!
//! ## Key Operations
//!
//! - Every HIR expression → THIR expression with explicit `ty` field
//! - Type from `type_id` on HIR nodes (no unresolved type vars allowed)
//! - MethodCall: THIS IS WHERE TRAIT RESOLUTION HAPPENS
//! - Pattern lowering with resolved types on bindings

use doo_core::types::{builtin, TypeId, TypeKind, TypeRegistry};
use doo_core::Span;
use doo_hir::{
    HirConst, HirDecorator, HirEnum, HirExpr, HirExprKind, HirField, HirFunction, HirImport,
    HirImportItem, HirInterface, HirInterfaceMethod, HirItem, HirMatchArm, HirMatchPattern,
    HirParam, HirProgram, HirStatic, HirStmt, HirStmtKind, HirStruct, HirVariant,
};
use rustc_hash::FxHashMap;

use crate::expr::{ThirBinOp, ThirExpr, ThirExprKind, ThirLiteral, ThirUnOp};
use crate::item::{
    ThirConst, ThirEnum, ThirField, ThirFunction, ThirImport, ThirImportItem, ThirInterface,
    ThirInterfaceMethod, ThirItem, ThirParam, ThirStatic, ThirStruct, ThirVariant,
};
use crate::pattern::{ThirPattern, ThirPatternKind};
use crate::solve::TraitSolver;
use crate::stmt::{ThirStmt, ThirStmtKind};
use crate::types::{ImplResolution, ThirCapture, ThirProgram};

/// THIR lowering error.
#[derive(Debug, Clone)]
pub struct ThirLowerError {
    pub message: String,
    pub span: Span,
}

/// Context for lowering HIR to THIR.
pub struct ThirLoweringContext<'a> {
    registry: &'a TypeRegistry,
    trait_solver: TraitSolver,
    errors: Vec<ThirLowerError>,
    /// Track method signatures for trait solver population
    method_signatures: FxHashMap<String, Vec<String>>,
}

impl<'a> ThirLoweringContext<'a> {
    pub fn new(registry: &'a TypeRegistry) -> Self {
        Self {
            registry,
            trait_solver: TraitSolver::new(),
            errors: Vec::new(),
            method_signatures: FxHashMap::default(),
        }
    }

    /// Lower an entire HIR program to THIR.
    pub fn lower_program(&mut self, hir: &HirProgram) -> ThirProgram {
        self.collect_inherent_methods(&hir.items);
        self.collect_interfaces(&hir.items);

        let items: Vec<ThirItem> = hir
            .items
            .iter()
            .filter_map(|item| self.lower_item(item))
            .collect();

        ThirProgram {
            items,
            span: hir.span,
        }
    }

    fn collect_inherent_methods(&mut self, items: &[HirItem]) {
        for item in items {
            if let HirItem::Function(f) = item {
                if let Some(assoc_type) = &f.receiver {
                    self.trait_solver
                        .register_inherent_method(assoc_type, &f.name);
                }
            }
        }
    }

    fn collect_interfaces(&mut self, items: &[HirItem]) {
        for item in items {
            if let HirItem::Interface(i) = item {
                let methods: Vec<String> = i.methods.iter().map(|m| m.name.clone()).collect();
                self.trait_solver.register_interface(&i.name, methods);
            }
        }
    }

    fn lower_item(&mut self, item: &HirItem) -> Option<ThirItem> {
        match item {
            HirItem::Const(c) => Some(ThirItem::Const(self.lower_const(c))),
            HirItem::Static(s) => Some(ThirItem::Static(self.lower_static(s))),
            HirItem::Function(f) => Some(ThirItem::Function(self.lower_function(f))),
            HirItem::Struct(s) => Some(ThirItem::Struct(self.lower_struct(s))),
            HirItem::Enum(e) => Some(ThirItem::Enum(self.lower_enum(e))),
            HirItem::Interface(i) => Some(ThirItem::Interface(self.lower_interface(i))),
            HirItem::Import(i) => Some(ThirItem::Import(self.lower_import(i))),
        }
    }

    fn lower_const(&mut self, c: &HirConst) -> ThirConst {
        ThirConst {
            name: c.name.clone(),
            is_public: c.is_public,
            value_expr: self.lower_expr(&c.value_expr),
            ty: c.type_id,
            span: c.span,
        }
    }

    fn lower_static(&mut self, s: &HirStatic) -> ThirStatic {
        ThirStatic {
            name: s.name.clone(),
            is_public: s.is_public,
            ty: s.type_id,
            span: s.span,
        }
    }

    fn lower_function(&mut self, f: &HirFunction) -> ThirFunction {
        let params: Vec<ThirParam> = f
            .params
            .iter()
            .map(|p| ThirParam {
                name: p.name.clone(),
                ty: p.type_id,
                span: p.span,
            })
            .collect();

        let body: Vec<ThirStmt> = f.body.iter().map(|s| self.lower_stmt(s)).collect();

        ThirFunction {
            name: f.name.clone(),
            type_params: f.type_params.clone(),
            params,
            return_type: f.return_type,
            error_type: f.error_type,
            body,
            is_async: f.is_async,
            span: f.span,
        }
    }

    fn lower_struct(&mut self, s: &HirStruct) -> ThirStruct {
        let fields: Vec<ThirField> = s
            .fields
            .iter()
            .map(|f| ThirField {
                name: f.name.clone(),
                ty: f.type_id,
                is_public: f.is_public,
                is_optional: f.is_optional,
                span: f.span,
            })
            .collect();

        ThirStruct {
            name: s.name.clone(),
            type_params: s.type_params.clone(),
            fields,
            span: s.span,
        }
    }

    fn lower_enum(&mut self, e: &HirEnum) -> ThirEnum {
        let variants: Vec<ThirVariant> = e
            .variants
            .iter()
            .map(|v| ThirVariant {
                name: v.name.clone(),
                payload: v.payload,
                span: v.span,
            })
            .collect();

        ThirEnum {
            name: e.name.clone(),
            variants,
            span: e.span,
        }
    }

    fn lower_interface(&mut self, i: &HirInterface) -> ThirInterface {
        let methods: Vec<ThirInterfaceMethod> = i
            .methods
            .iter()
            .map(|m| ThirInterfaceMethod {
                name: m.name.clone(),
                params: m
                    .params
                    .iter()
                    .map(|p| ThirParam {
                        name: p.name.clone(),
                        ty: p.type_id,
                        span: p.span,
                    })
                    .collect(),
                return_type: m.return_type,
                error_type: m.error_type,
                span: m.span,
            })
            .collect();

        ThirInterface {
            name: i.name.clone(),
            methods,
            span: i.span,
        }
    }

    fn lower_import(&mut self, i: &HirImport) -> ThirImport {
        let items: Vec<ThirImportItem> = i
            .items
            .iter()
            .map(|item| match item {
                HirImportItem::Symbol(s) => ThirImportItem::Symbol(s.clone()),
                HirImportItem::Alias { name, alias } => ThirImportItem::Alias {
                    name: name.clone(),
                    alias: alias.clone(),
                },
                HirImportItem::Wildcard => ThirImportItem::Wildcard,
            })
            .collect();

        ThirImport {
            path: i.path.clone(),
            items,
            span: i.span,
        }
    }

    // ========================================================================
    // Statement Lowering
    // ========================================================================

    fn lower_stmt(&mut self, stmt: &HirStmt) -> ThirStmt {
        let kind = match &stmt.kind {
            HirStmtKind::Let {
                name,
                type_id,
                value,
                mutable,
                ..
            } => ThirStmtKind::Let {
                name: name.clone(),
                ty: type_id.unwrap_or(builtin::ANY),
                value: self.lower_expr(value),
                mutable: *mutable,
            },

            HirStmtKind::TupleLet {
                names,
                type_ids,
                value,
                mutable,
            } => ThirStmtKind::TupleLet {
                names: names.clone(),
                type_ids: type_ids.clone(),
                value: self.lower_expr(value),
                mutable: *mutable,
            },

            HirStmtKind::Assign { target, value } => ThirStmtKind::Assign {
                target: self.lower_expr(target),
                value: self.lower_expr(value),
            },

            HirStmtKind::Expr(expr) => ThirStmtKind::Expr(self.lower_expr(expr)),

            HirStmtKind::Return(values) => {
                let val = values.first().map(|e| self.lower_expr(e));
                ThirStmtKind::Return(val)
            }

            HirStmtKind::Break => ThirStmtKind::Break(None),

            HirStmtKind::Continue => ThirStmtKind::Continue,

            HirStmtKind::Drop { name } => ThirStmtKind::Drop {
                name: name.clone(),
                ty: builtin::ANY,
            },

            HirStmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                let cond = self.lower_expr(condition);
                let then_stmts: Vec<ThirStmt> =
                    then_block.iter().map(|s| self.lower_stmt(s)).collect();
                let else_stmts: Option<Vec<ThirStmt>> = else_block
                    .as_ref()
                    .map(|stmts| stmts.iter().map(|s| self.lower_stmt(s)).collect());

                ThirStmtKind::Expr(ThirExpr::new(
                    ThirExprKind::If {
                        cond: Box::new(cond),
                        then: Box::new(ThirExpr::new(
                            ThirExprKind::Block(then_stmts, None),
                            builtin::VOID,
                            stmt.span,
                        )),
                        else_: else_stmts.map(|stmts| {
                            Box::new(ThirExpr::new(
                                ThirExprKind::Block(stmts, None),
                                builtin::VOID,
                                stmt.span,
                            ))
                        }),
                    },
                    builtin::VOID,
                    stmt.span,
                ))
            }

            HirStmtKind::While {
                condition,
                body,
                increment,
            } => ThirStmtKind::While {
                cond: self.lower_expr(condition),
                body: body.iter().map(|s| self.lower_stmt(s)).collect(),
                increment: increment.iter().map(|s| self.lower_stmt(s)).collect(),
            },

            HirStmtKind::ManualErrorExtract {
                ok_names,
                error_name,
                expr,
            } => ThirStmtKind::ManualErrorExtract {
                ok_names: ok_names.clone(),
                error_name: error_name.clone(),
                expr: self.lower_expr(expr),
            },
        };

        ThirStmt {
            kind,
            span: stmt.span,
        }
    }

    // ========================================================================
    // Expression Lowering
    // ========================================================================

    fn lower_expr(&mut self, expr: &HirExpr) -> ThirExpr {
        let ty = expr.type_id.unwrap_or(builtin::ANY);
        let kind = match &expr.kind {
            HirExprKind::Const(c) => {
                let lit = match c {
                    doo_hir::ConstValue::Int(v) => ThirLiteral::Int(*v),
                    doo_hir::ConstValue::Float(v) => ThirLiteral::Float(*v),
                    doo_hir::ConstValue::Bool(v) => ThirLiteral::Bool(*v),
                    doo_hir::ConstValue::Str(v) => ThirLiteral::String(v.clone()),
                    doo_hir::ConstValue::Nil => ThirLiteral::Null,
                };
                ThirExprKind::Literal(lit)
            }

            HirExprKind::Local { name } => ThirExprKind::Var(name.clone()),
            HirExprKind::Global { name } => ThirExprKind::Var(name.clone()),

            HirExprKind::BinOp { op, lhs, rhs } => {
                let thir_op = match op {
                    doo_hir::HirBinOp::Add => ThirBinOp::Add,
                    doo_hir::HirBinOp::Sub => ThirBinOp::Sub,
                    doo_hir::HirBinOp::Mul => ThirBinOp::Mul,
                    doo_hir::HirBinOp::Div => ThirBinOp::Div,
                    doo_hir::HirBinOp::Mod => ThirBinOp::Mod,
                    doo_hir::HirBinOp::Eq => ThirBinOp::Eq,
                    doo_hir::HirBinOp::NotEq => ThirBinOp::NotEq,
                    doo_hir::HirBinOp::Lt => ThirBinOp::Lt,
                    doo_hir::HirBinOp::Gt => ThirBinOp::Gt,
                    doo_hir::HirBinOp::LtEq => ThirBinOp::LtEq,
                    doo_hir::HirBinOp::GtEq => ThirBinOp::GtEq,
                    doo_hir::HirBinOp::And => ThirBinOp::And,
                    doo_hir::HirBinOp::Or => ThirBinOp::Or,
                    doo_hir::HirBinOp::BitAnd => ThirBinOp::BitAnd,
                    doo_hir::HirBinOp::BitOr => ThirBinOp::BitOr,
                    doo_hir::HirBinOp::BitXor => ThirBinOp::BitXor,
                    _ => ThirBinOp::Add,
                };
                ThirExprKind::Binary {
                    op: thir_op,
                    lhs: Box::new(self.lower_expr(lhs)),
                    rhs: Box::new(self.lower_expr(rhs)),
                }
            }

            HirExprKind::UnaryOp { op, operand } => {
                let thir_op = match op {
                    doo_hir::HirUnaryOp::Neg => ThirUnOp::Neg,
                    doo_hir::HirUnaryOp::Not => ThirUnOp::Not,
                };
                ThirExprKind::Unary {
                    op: thir_op,
                    expr: Box::new(self.lower_expr(operand)),
                }
            }

            HirExprKind::Call { func, args } => ThirExprKind::Call {
                func: Box::new(self.lower_expr(func)),
                args: args.iter().map(|a| self.lower_expr(a)).collect(),
            },

            HirExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                let recv_thir = self.lower_expr(receiver);
                let resolved_impl =
                    self.trait_solver
                        .resolve_method(recv_thir.ty, method, self.registry);
                ThirExprKind::MethodCall {
                    receiver: Box::new(recv_thir),
                    method: method.clone(),
                    resolved_impl,
                    args: args.iter().map(|a| self.lower_expr(a)).collect(),
                }
            }

            HirExprKind::Field { object, field } => {
                let obj_thir = self.lower_expr(object);
                let field_idx = self.get_field_idx(obj_thir.ty, field);
                ThirExprKind::FieldAccess {
                    object: Box::new(obj_thir),
                    field: field.clone(),
                    field_idx,
                }
            }

            HirExprKind::Index { object, index } => ThirExprKind::Index {
                object: Box::new(self.lower_expr(object)),
                index: Box::new(self.lower_expr(index)),
            },

            HirExprKind::Array(elements) => {
                ThirExprKind::ArrayLiteral(elements.iter().map(|e| self.lower_expr(e)).collect())
            }

            HirExprKind::Map(entries) => ThirExprKind::MapLiteral(
                entries
                    .iter()
                    .map(|(k, v)| (self.lower_expr(k), self.lower_expr(v)))
                    .collect(),
            ),

            HirExprKind::Tuple(elements) => {
                ThirExprKind::Tuple(elements.iter().map(|e| self.lower_expr(e)).collect())
            }

            HirExprKind::Struct { name, fields } => ThirExprKind::StructLiteral {
                name: name.clone(),
                fields: fields
                    .iter()
                    .map(|(n, v)| (n.clone(), self.lower_expr(v)))
                    .collect(),
            },

            HirExprKind::EnumVariant {
                enum_name,
                variant,
                payload,
            } => ThirExprKind::EnumVariant {
                enum_name: enum_name.clone(),
                variant: variant.clone(),
                payload: payload.iter().map(|e| self.lower_expr(e)).collect(),
            },

            HirExprKind::Spread(inner) => ThirExprKind::Spread(Box::new(self.lower_expr(inner))),

            HirExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => ThirExprKind::If {
                cond: Box::new(self.lower_expr(condition)),
                then: Box::new(self.lower_expr(then_expr)),
                else_: else_expr.as_ref().map(|e| Box::new(self.lower_expr(e))),
            },

            HirExprKind::Block { stmts, expr } => {
                let thir_stmts: Vec<ThirStmt> = stmts.iter().map(|s| self.lower_stmt(s)).collect();
                let thir_expr = expr.as_ref().map(|e| Box::new(self.lower_expr(e)));
                ThirExprKind::Block(thir_stmts, thir_expr)
            }

            HirExprKind::Match { values, arms } => {
                let match_expr = values
                    .first()
                    .map(|e| self.lower_expr(e))
                    .unwrap_or_else(|| {
                        ThirExpr::new(
                            ThirExprKind::Literal(ThirLiteral::Null),
                            builtin::ANY,
                            expr.span,
                        )
                    });
                let thir_arms: Vec<crate::expr::ThirArm> = arms
                    .iter()
                    .map(|arm| crate::expr::ThirArm {
                        pattern: self.lower_pattern(&arm.pattern),
                        guard: arm.guard.as_ref().map(|g| self.lower_expr(g)),
                        body: self.lower_expr(&arm.body),
                    })
                    .collect();
                ThirExprKind::Match {
                    expr: Box::new(match_expr),
                    arms: thir_arms,
                }
            }

            HirExprKind::Range {
                start,
                end,
                inclusive,
            } => ThirExprKind::Range {
                start: Some(Box::new(self.lower_expr(start))),
                end: Some(Box::new(self.lower_expr(end))),
                inclusive: *inclusive,
            },

            HirExprKind::Ok(inner) => ThirExprKind::Ok(Box::new(self.lower_expr(inner))),
            HirExprKind::Err(inner) => ThirExprKind::Err(Box::new(self.lower_expr(inner))),
            HirExprKind::Try(inner) => ThirExprKind::Try(Box::new(self.lower_expr(inner))),

            HirExprKind::UnwrapOrPanic { expr, message } => ThirExprKind::UnwrapOrPanic {
                expr: Box::new(self.lower_expr(expr)),
                message: Box::new(self.lower_expr(message)),
            },

            HirExprKind::Move(inner) => ThirExprKind::Move(Box::new(self.lower_expr(inner))),
            HirExprKind::Borrow { expr, mutable } => ThirExprKind::Borrow {
                expr: Box::new(self.lower_expr(expr)),
                mutable: *mutable,
            },
            HirExprKind::Clone(inner) => ThirExprKind::Clone(Box::new(self.lower_expr(inner))),

            HirExprKind::Closure { params, body, .. } => {
                let thir_params: Vec<(String, TypeId)> = params
                    .iter()
                    .map(|(n, t)| (n.clone(), t.unwrap_or(builtin::ANY)))
                    .collect();
                ThirExprKind::Closure {
                    params: thir_params,
                    body: Box::new(self.lower_expr(body)),
                    captures: Vec::new(),
                }
            }

            HirExprKind::Cast { value, to_type } => ThirExprKind::Cast {
                value: Box::new(self.lower_expr(value)),
                to_type: *to_type,
            },

            HirExprKind::Await(inner) => ThirExprKind::Await(Box::new(self.lower_expr(inner))),
            HirExprKind::Spawn { body } => ThirExprKind::Spawn(Box::new(self.lower_expr(body))),
            HirExprKind::ScopeBlock { stmts } => ThirExprKind::ScopeBlock {
                stmts: stmts.iter().map(|s| self.lower_stmt(s)).collect(),
            },
        };

        ThirExpr {
            kind,
            ty,
            span: expr.span,
        }
    }

    // ========================================================================
    // Pattern Lowering
    // ========================================================================

    fn lower_pattern(&mut self, pat: &HirMatchPattern) -> ThirPattern {
        let kind = match pat {
            HirMatchPattern::Literal(e) => {
                let lit = if let HirExprKind::Const(c) = &e.kind {
                    match c {
                        doo_hir::ConstValue::Int(v) => ThirLiteral::Int(*v),
                        doo_hir::ConstValue::Float(v) => ThirLiteral::Float(*v),
                        doo_hir::ConstValue::Bool(v) => ThirLiteral::Bool(*v),
                        doo_hir::ConstValue::Str(v) => ThirLiteral::String(v.clone()),
                        doo_hir::ConstValue::Nil => ThirLiteral::Null,
                    }
                } else {
                    ThirLiteral::Null
                };
                ThirPatternKind::Literal(lit)
            }
            HirMatchPattern::Condition(e) => {
                ThirPatternKind::Condition(Box::new(self.lower_expr(e)))
            }
            HirMatchPattern::Wildcard => ThirPatternKind::Wildcard,
            HirMatchPattern::EnumVariant { enum_name, variant } => ThirPatternKind::Enum {
                name: enum_name.clone(),
                variant: variant.clone(),
                payload: None,
            },
            HirMatchPattern::EnumVariantPayload {
                enum_name,
                variant,
                bindings,
            } => {
                let payload_pat = if bindings.is_empty() {
                    None
                } else {
                    Some(Box::new(ThirPattern {
                        kind: ThirPatternKind::Ident(bindings[0].clone(), false),
                        ty: None,
                        span: doo_core::Span::dummy(),
                    }))
                };
                ThirPatternKind::Enum {
                    name: enum_name.clone(),
                    variant: variant.clone(),
                    payload: payload_pat,
                }
            }
            HirMatchPattern::Tuple(parts) => {
                ThirPatternKind::Tuple(parts.iter().map(|p| self.lower_pattern(p)).collect())
            }
        };

        ThirPattern {
            kind,
            ty: None,
            span: doo_core::Span::dummy(),
        }
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    /// Get the index of a field in a struct definition.
    fn get_field_idx(&self, obj_type: TypeId, field: &str) -> usize {
        if let Some(info) = self.registry.get(obj_type) {
            if let TypeKind::Struct { def } = &info.kind {
                for (i, f) in def.fields.iter().enumerate() {
                    if f.name.resolve() == field {
                        return i;
                    }
                }
            }
        }
        0
    }

    pub fn errors(&self) -> &[ThirLowerError] {
        &self.errors
    }
}
