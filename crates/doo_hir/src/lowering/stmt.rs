//! Statement lowering and desugaring.

use super::Lower;
use crate::types::*;
use doo_core::{
    doo_debug,
    types::{builtin, TypeId, TypeKind, TypeRegistry},
};
use doo_frontend::ast::{self, ElseBranch, ExprKind, IncDecOp, PatternKind, Stmt, StmtKind};

impl Lower {
    pub(crate) fn lower_stmt(&mut self, stmt: &Stmt) -> HirStmt {
        let kind = match &stmt.kind {
            StmtKind::Let {
                mutable,
                pattern,
                type_ann: _,
                value,
            } => {
                // Check if this is a tuple pattern - if so, use TupleLet
                if let PatternKind::Tuple(patterns) = &pattern.kind {
                    let names: Vec<String> =
                        patterns.iter().map(|p| self.pattern_to_name(p)).collect();
                    let type_ids: Vec<Option<TypeId>> = vec![None; names.len()];
                    HirStmtKind::TupleLet {
                        names,
                        type_ids,
                        value: self.lower_expr(value),
                        mutable: *mutable,
                    }
                } else {
                    let name = self.pattern_to_name(pattern);
                    HirStmtKind::Let {
                        name,
                        type_id: None,
                        value: self.lower_expr(value),
                        mutable: *mutable,
                        ownership: Ownership::Owned,
                    }
                }
            }

            StmtKind::Assign { target, value } => {
                let target_expr = self.pattern_to_expr(target);
                HirStmtKind::Assign {
                    target: target_expr,
                    value: self.lower_expr(value),
                }
            }

            // === Desugaring: Compound Assignment ===
            // `x += 1` → `x = x + 1`
            StmtKind::CompoundAssign { target, op, value } => {
                let target_expr = self.pattern_to_expr(target);
                let target_read = self.pattern_to_expr(target);
                let hir_op = self.compound_op_to_binop(*op);

                let binop_expr = HirExpr::new(
                    HirExprKind::BinOp {
                        op: hir_op,
                        lhs: Box::new(target_read),
                        rhs: Box::new(self.lower_expr(value)),
                    },
                    stmt.span,
                );

                HirStmtKind::Assign {
                    target: target_expr,
                    value: binop_expr,
                }
            }

            // === Desugaring: Increment/Decrement ===
            // `x++` → `x = x + 1`
            // `x--` → `x = x - 1`
            StmtKind::IncDec { variable, op } => {
                let target = HirExpr::new(
                    HirExprKind::Local {
                        name: variable.clone(),
                    },
                    stmt.span,
                );
                let target_read = HirExpr::new(
                    HirExprKind::Local {
                        name: variable.clone(),
                    },
                    stmt.span,
                );
                let one = HirExpr::new(HirExprKind::Const(ConstValue::Int(1)), stmt.span);
                let hir_op = match op {
                    IncDecOp::Inc => HirBinOp::Add,
                    IncDecOp::Dec => HirBinOp::Sub,
                };

                let binop_expr = HirExpr::new(
                    HirExprKind::BinOp {
                        op: hir_op,
                        lhs: Box::new(target_read),
                        rhs: Box::new(one),
                    },
                    stmt.span,
                );

                HirStmtKind::Assign {
                    target,
                    value: binop_expr,
                }
            }

            StmtKind::Expr(expr) => HirStmtKind::Expr(self.lower_expr(expr)),

            StmtKind::Return(values) => {
                HirStmtKind::Return(values.iter().map(|e| self.lower_expr(e)).collect())
            }

            StmtKind::Break => HirStmtKind::Break,
            StmtKind::Continue => HirStmtKind::Continue,

            StmtKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                let then_stmts = then_block.iter().map(|s| self.lower_stmt(s)).collect();
                let else_stmts = else_branch.as_ref().map(|eb| match eb {
                    ElseBranch::Block(stmts) => stmts.iter().map(|s| self.lower_stmt(s)).collect(),
                    ElseBranch::ElseIf(if_stmt) => {
                        vec![self.lower_stmt(if_stmt)]
                    }
                });

                HirStmtKind::If {
                    condition: self.lower_expr(condition),
                    then_block: then_stmts,
                    else_block: else_stmts,
                }
            }

            // === Desugaring: For Loop ===
            // Three cases:
            // 1. `for i in start..end` → range-based index loop
            // 2. `for x in array` → index-based array iteration
            // 3. `for i, x in array` → index + element iteration
            // 4. `for { ... }` → infinite loop
            StmtKind::For {
                pattern,
                iterable,
                body,
            } => self.lower_for_loop(pattern, iterable.as_ref(), body, stmt.span),

            StmtKind::Block(stmts) => {
                // Lower block as expression statement
                let lowered: Vec<_> = stmts.iter().map(|s| self.lower_stmt(s)).collect();
                // Always wrap in Block to preserve scope boundaries
                HirStmtKind::Expr(HirExpr::new(
                    HirExprKind::Block {
                        stmts: lowered,
                        expr: None,
                    },
                    stmt.span,
                ))
            }

            StmtKind::Print(exprs) => {
                // Flatten StringInterpolation parts as separate print args
                // so composite types (Array, Map, Struct) get proper formatting
                // via the Print handler instead of broken string concat.
                let mut args = Vec::new();
                let mut has_interpolation = false;
                for e in exprs {
                    if let ExprKind::StringInterpolation(parts) = &e.kind {
                        has_interpolation = true;
                        for part in parts {
                            args.push(self.lower_string_part(part));
                        }
                    } else {
                        args.push(self.lower_expr(e));
                    }
                }
                let func_name = if has_interpolation {
                    "__print_interp"
                } else {
                    "print"
                };
                HirStmtKind::Expr(HirExpr::new(
                    HirExprKind::Call {
                        func: Box::new(HirExpr::new(
                            HirExprKind::Global {
                                name: func_name.to_string(),
                            },
                            stmt.span,
                        )),
                        args,
                    },
                    stmt.span,
                ))
            }

            StmtKind::ElementAssign {
                array,
                index,
                value,
            } => {
                // array[idx] = value → __array_set(array, idx, value)
                // For now, lower as method call
                HirStmtKind::Expr(HirExpr::new(
                    HirExprKind::MethodCall {
                        receiver: Box::new(self.lower_expr(array)),
                        method: "__set".to_string(),
                        args: vec![self.lower_expr(index), self.lower_expr(value)],
                    },
                    stmt.span,
                ))
            }

            StmtKind::FieldAssign {
                object,
                field,
                value,
            } => {
                // obj.field = value → direct assignment
                let target = HirExpr::new(
                    HirExprKind::Field {
                        object: Box::new(self.lower_expr(object)),
                        field: field.clone(),
                    },
                    stmt.span,
                );
                HirStmtKind::Assign {
                    target,
                    value: self.lower_expr(value),
                }
            }

            StmtKind::ManualErrorExtract {
                expr,
                ok_pattern,
                error_var,
            } => {
                let ok_names = self.pattern_to_names(ok_pattern);
                HirStmtKind::ManualErrorExtract {
                    ok_names,
                    error_name: error_var.clone(),
                    expr: self.lower_expr(expr),
                }
            }

            // Local struct/enum declarations: hoist to top-level items, emit no-op in body
            StmtKind::StructDecl(s) => {
                let hir_struct = self.lower_struct(s);
                self.hoisted_items.push(HirItem::Struct(hir_struct));
                HirStmtKind::Expr(HirExpr::new(HirExprKind::Const(ConstValue::Nil), stmt.span))
            }
            StmtKind::EnumDecl(e) => {
                let hir_enum = self.lower_enum(e);
                self.hoisted_items.push(HirItem::Enum(hir_enum));
                HirStmtKind::Expr(HirExpr::new(HirExprKind::Const(ConstValue::Nil), stmt.span))
            }
        };

        HirStmt::new(kind, stmt.span)
    }

    pub(crate) fn lower_stmt_typed(&mut self, stmt: &Stmt, registry: &mut TypeRegistry) -> HirStmt {
        let kind = match &stmt.kind {
            StmtKind::Let {
                mutable,
                pattern,
                type_ann,
                value,
            } => {
                // Check if this is a tuple pattern
                if let PatternKind::Tuple(patterns) = &pattern.kind {
                    let names: Vec<String> =
                        patterns.iter().map(|p| self.pattern_to_name(p)).collect();
                    let value_hir = self.lower_expr_typed(value, registry);

                    // Try to get element types from the value's tuple type
                    let mut type_ids: Vec<Option<TypeId>> = vec![None; names.len()];
                    if let Some(val_type_id) = value_hir.type_id {
                        if let Some(info) = registry.get(val_type_id) {
                            if let TypeKind::Tuple { elements } = &info.kind {
                                for (i, elem_type) in elements.iter().enumerate() {
                                    if i < type_ids.len() {
                                        type_ids[i] = Some(*elem_type);
                                        // Track each element's type
                                        self.var_types.insert(names[i].clone(), *elem_type);
                                    }
                                }
                            }
                        }
                    }

                    HirStmtKind::TupleLet {
                        names,
                        type_ids,
                        value: value_hir,
                        mutable: *mutable,
                    }
                } else {
                    let name = self.pattern_to_name(pattern);
                    let value_hir = self.lower_expr_typed(value, registry);
                    let annotated_type_id = type_ann
                        .as_ref()
                        .map(|t| self.resolve_type_expr(t, registry));
                    let inferred_type_id = annotated_type_id.or(value_hir.type_id);
                    // NOTE: Do NOT overwrite value_hir.type_id with annotated type.
                    // The value expression must keep its original type so the type checker
                    // can compare it against the annotation and detect mismatches.
                    // The Let statement's own type_id field carries the annotation.
                    // Track variable type for later lookups
                    if let Some(tid) = inferred_type_id {
                        self.var_types.insert(name.clone(), tid);
                    }

                    // Track JSON.stringify sources for later JSON.parse type inference
                    // If the value is JSON.stringify(x), remember that this variable contains JSON of type x
                    if let Some(stringify_arg_type) = self.extract_stringify_arg_type(&value_hir) {
                        self.json_stringify_sources
                            .insert(name.clone(), stringify_arg_type);
                    }

                    HirStmtKind::Let {
                        name,
                        type_id: inferred_type_id,
                        value: value_hir,
                        mutable: *mutable,
                        ownership: Ownership::Owned,
                    }
                }
            }

            StmtKind::Assign { target, value } => {
                let target_expr = self.pattern_to_expr(target);
                HirStmtKind::Assign {
                    target: target_expr,
                    value: self.lower_expr_typed(value, registry),
                }
            }

            StmtKind::CompoundAssign { target, op, value } => {
                let target_expr = self.pattern_to_expr(target);
                let target_read = self.pattern_to_expr(target);
                let hir_op = self.compound_op_to_binop(*op);

                let binop_expr = HirExpr::new(
                    HirExprKind::BinOp {
                        op: hir_op,
                        lhs: Box::new(target_read),
                        rhs: Box::new(self.lower_expr_typed(value, registry)),
                    },
                    stmt.span,
                );

                HirStmtKind::Assign {
                    target: target_expr,
                    value: binop_expr,
                }
            }

            StmtKind::IncDec { variable, op } => {
                let target = HirExpr::new(
                    HirExprKind::Local {
                        name: variable.clone(),
                    },
                    stmt.span,
                );
                let target_read = HirExpr::new(
                    HirExprKind::Local {
                        name: variable.clone(),
                    },
                    stmt.span,
                );
                let one = HirExpr::with_type(
                    HirExprKind::Const(ConstValue::Int(1)),
                    builtin::INT,
                    stmt.span,
                );
                let hir_op = match op {
                    IncDecOp::Inc => HirBinOp::Add,
                    IncDecOp::Dec => HirBinOp::Sub,
                };

                let binop_expr = HirExpr::new(
                    HirExprKind::BinOp {
                        op: hir_op,
                        lhs: Box::new(target_read),
                        rhs: Box::new(one),
                    },
                    stmt.span,
                );

                HirStmtKind::Assign {
                    target,
                    value: binop_expr,
                }
            }

            StmtKind::Expr(expr) => HirStmtKind::Expr(self.lower_expr_typed(expr, registry)),

            StmtKind::Return(values) => HirStmtKind::Return(
                values
                    .iter()
                    .map(|e| self.lower_expr_typed(e, registry))
                    .collect(),
            ),

            StmtKind::Break => HirStmtKind::Break,
            StmtKind::Continue => HirStmtKind::Continue,

            StmtKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                let then_stmts = then_block
                    .iter()
                    .map(|s| self.lower_stmt_typed(s, registry))
                    .collect();
                let else_stmts = else_branch.as_ref().map(|eb| match eb {
                    ElseBranch::Block(stmts) => stmts
                        .iter()
                        .map(|s| self.lower_stmt_typed(s, registry))
                        .collect(),
                    ElseBranch::ElseIf(if_stmt) => vec![self.lower_stmt_typed(if_stmt, registry)],
                });

                HirStmtKind::If {
                    condition: self.lower_expr_typed(condition, registry),
                    then_block: then_stmts,
                    else_block: else_stmts,
                }
            }

            StmtKind::For {
                pattern,
                iterable,
                body,
            } => self.lower_for_loop_typed(pattern, iterable.as_ref(), body, stmt.span, registry),

            StmtKind::Block(stmts) => {
                let lowered: Vec<_> = stmts
                    .iter()
                    .map(|s| self.lower_stmt_typed(s, registry))
                    .collect();
                // Always wrap in Block to preserve scope boundaries
                HirStmtKind::Expr(HirExpr::new(
                    HirExprKind::Block {
                        stmts: lowered,
                        expr: None,
                    },
                    stmt.span,
                ))
            }

            StmtKind::Print(exprs) => {
                // Flatten StringInterpolation parts as separate print args.
                // Don't cast to STR — keep original types so the Print handler
                // uses type-specific formatters (Array, Map, Struct, etc.)
                let mut args = Vec::new();
                let mut has_interpolation = false;
                for e in exprs {
                    if let ExprKind::StringInterpolation(parts) = &e.kind {
                        has_interpolation = true;
                        for part in parts {
                            match part {
                                ast::StringPart::Literal(s) => {
                                    args.push(HirExpr::with_type(
                                        HirExprKind::Const(ConstValue::Str(s.clone())),
                                        builtin::STR,
                                        stmt.span,
                                    ));
                                }
                                ast::StringPart::Expr(expr) => {
                                    // Lower WITHOUT Cast to STR — preserve original type
                                    args.push(self.lower_expr_typed(expr, registry));
                                }
                            }
                        }
                    } else {
                        args.push(self.lower_expr_typed(e, registry));
                    }
                }
                let func_name = if has_interpolation {
                    "__print_interp"
                } else {
                    "print"
                };
                HirStmtKind::Expr(HirExpr::new(
                    HirExprKind::Call {
                        func: Box::new(HirExpr::new(
                            HirExprKind::Global {
                                name: func_name.to_string(),
                            },
                            stmt.span,
                        )),
                        args,
                    },
                    stmt.span,
                ))
            }

            StmtKind::ElementAssign {
                array,
                index,
                value,
            } => HirStmtKind::Expr(HirExpr::new(
                HirExprKind::MethodCall {
                    receiver: Box::new(self.lower_expr_typed(array, registry)),
                    method: "__set".to_string(),
                    args: vec![
                        self.lower_expr_typed(index, registry),
                        self.lower_expr_typed(value, registry),
                    ],
                },
                stmt.span,
            )),

            StmtKind::FieldAssign {
                object,
                field,
                value,
            } => {
                let target = HirExpr::new(
                    HirExprKind::Field {
                        object: Box::new(self.lower_expr_typed(object, registry)),
                        field: field.clone(),
                    },
                    stmt.span,
                );
                HirStmtKind::Assign {
                    target,
                    value: self.lower_expr_typed(value, registry),
                }
            }

            StmtKind::ManualErrorExtract {
                expr,
                ok_pattern,
                error_var,
            } => {
                let ok_names = self.pattern_to_names(ok_pattern);
                HirStmtKind::ManualErrorExtract {
                    ok_names,
                    error_name: error_var.clone(),
                    expr: self.lower_expr_typed(expr, registry),
                }
            }

            // Local struct/enum declarations: hoist to top-level items, emit no-op in body
            StmtKind::StructDecl(s) => {
                let hir_struct = self.lower_struct_typed(s, registry);
                self.hoisted_items.push(HirItem::Struct(hir_struct));
                HirStmtKind::Expr(HirExpr::new(HirExprKind::Const(ConstValue::Nil), stmt.span))
            }
            StmtKind::EnumDecl(e) => {
                let hir_enum = self.lower_enum_typed(e, registry);
                self.hoisted_items.push(HirItem::Enum(hir_enum));
                HirStmtKind::Expr(HirExpr::new(HirExprKind::Const(ConstValue::Nil), stmt.span))
            }
        };

        HirStmt::new(kind, stmt.span)
    }
}
