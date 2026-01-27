//! Statement building for MIR

use super::MirBuilder;
use crate::{LocalDef, MirInstrKind, MirOperand, MirTerminator};
use doo_core::types::{builtin, TypeId as CoreTypeId, TypeKind};
use doo_hir::{HirExprKind, HirStmt, HirStmtKind};

/// Build a MIR statement from a HIR statement.
pub fn build_stmt(builder: &mut MirBuilder, stmt: &HirStmt) {
    let span = builder.convert_span(stmt.span);

    match &stmt.kind {
        HirStmtKind::Let {
            name,
            type_id,
            value,
            mutable,
            ..
        } => {
            let val_operand = builder.build_expr(value);

            // Register the local variable in the function
            let var_type_id = type_id.unwrap_or_else(|| {
                // Infer type from the value operand using builder's method
                builder.infer_operand_type(&val_operand)
            });

            if let Some(f) = &mut builder.current_func {
                // Check if already registered (e.g., loop index vars)
                if !f.locals.iter().any(|l| l.name == *name) {
                    f.locals.push(LocalDef {
                        name: name.clone(),
                        type_id: var_type_id,
                        mutable: *mutable,
                    });
                }
            }

            builder.emit(
                MirInstrKind::Assign {
                    dest: name.clone(),
                    value: val_operand,
                },
                span,
            );
        }

        HirStmtKind::TupleLet {
            names,
            type_ids,
            value,
            mutable,
        } => {
            // Build the tuple value first
            let tuple_operand = builder.build_expr(value);

            // Get the tuple type - first from HIR, then from temp_types if it was a call
            let tuple_type = value.type_id.or_else(|| {
                if let MirOperand::Temp(ref temp_name) = tuple_operand {
                    builder.get_temp_type(temp_name)
                } else {
                    None
                }
            });

            // Extract element types from the tuple type using TypeRegistry
            let element_types: Vec<CoreTypeId> = if let Some(tuple_type_id) = tuple_type {
                if let Some(info) = builder.type_registry.get(tuple_type_id) {
                    if let TypeKind::Tuple { elements } = &info.kind {
                        elements.clone()
                    } else {
                        // Not a tuple type, fall back to HIR type_ids or ANY
                        type_ids.iter().map(|t| t.unwrap_or(builtin::ANY)).collect()
                    }
                } else {
                    type_ids.iter().map(|t| t.unwrap_or(builtin::ANY)).collect()
                }
            } else {
                type_ids.iter().map(|t| t.unwrap_or(builtin::ANY)).collect()
            };

            // Create a temp to hold the tuple if it's not already a local
            let tuple_temp = match &tuple_operand {
                MirOperand::Local(name) => name.clone(),
                _ => {
                    let temp = builder.new_temp();
                    builder.emit(
                        MirInstrKind::Assign {
                            dest: temp.clone(),
                            value: tuple_operand.clone(),
                        },
                        span,
                    );
                    temp
                }
            };

            // Extract each element and assign to corresponding name
            for (i, name) in names.iter().enumerate() {
                // Get the type for this element from computed element_types
                let elem_type = element_types.get(i).copied().unwrap_or(builtin::ANY);

                // Register the local variable
                if let Some(f) = &mut builder.current_func {
                    if !f.locals.iter().any(|l| l.name == *name) {
                        f.locals.push(LocalDef {
                            name: name.clone(),
                            type_id: elem_type,
                            mutable: *mutable,
                        });
                    }
                }

                // Extract tuple element and assign
                let extract_temp = builder.new_temp();
                builder.emit(
                    MirInstrKind::TupleGet {
                        dest: extract_temp.clone(),
                        tuple: MirOperand::Local(tuple_temp.clone()),
                        index: i,
                        tuple_type,
                    },
                    span,
                );

                builder.emit(
                    MirInstrKind::Assign {
                        dest: name.clone(),
                        value: MirOperand::Local(extract_temp),
                    },
                    span,
                );
            }
        }

        HirStmtKind::Assign { target, value } => {
            let val_operand = builder.build_expr(value);

            // Extract the destination name directly from the target expression
            // Don't use build_expr on target - it would apply ownership decisions and return a temp
            match &target.kind {
                HirExprKind::Local { name } => {
                    builder.emit(
                        MirInstrKind::Assign {
                            dest: name.clone(),
                            value: val_operand,
                        },
                        span,
                    );
                }
                HirExprKind::Field { object, field } => {
                    // For field assignment like `obj.field = value`
                    let obj_operand = builder.build_expr(object);
                    builder.emit(
                        MirInstrKind::FieldSet {
                            object: obj_operand,
                            field: field.clone(),
                            value: val_operand,
                        },
                        span,
                    );
                }
                HirExprKind::Index { object, index } => {
                    // For index assignment like `arr[i] = value`
                    let array_operand = builder.build_expr(object);
                    let index_operand = builder.build_expr(index);

                    // Get container type: first try HIR type_id, then look up from locals
                    let container_type = object.type_id.or_else(|| {
                        if let HirExprKind::Local { name } = &object.kind {
                            builder.get_local_type(name)
                        } else {
                            None
                        }
                    });

                    let elem_type = container_type
                        .and_then(|t| builder.array_elem_type_from_type_id(t))
                        .unwrap_or(doo_core::types::builtin::ANY);
                    builder.emit(
                        MirInstrKind::ArraySet {
                            array: array_operand,
                            index: index_operand,
                            value: val_operand,
                            elem_type,
                        },
                        span,
                    );
                }
                _ => {
                    // Fallback: try to build as expression (may not work for all cases)
                    let target_operand = builder.build_expr(target);
                    if let MirOperand::Local(name) = target_operand {
                        builder.emit(
                            MirInstrKind::Assign {
                                dest: name,
                                value: val_operand,
                            },
                            span,
                        );
                    }
                }
            }
        }

        HirStmtKind::Expr(expr) => {
            // Just evaluate the expression for side effects
            builder.build_expr(expr);
        }

        HirStmtKind::Return(exprs) => {
            let expected_return_type = builder.get_current_function_return_type();
            let values: Vec<_> = exprs
                .iter()
                .map(|expr| builder.build_expr_with_expected_type(expr, expected_return_type))
                .collect();
            builder.set_terminator(MirTerminator::Return { values });
        }

        HirStmtKind::Break => {
            // Break jumps to the loop exit block
            if let Some(exit_label) = builder.break_targets.last().cloned() {
                builder.set_terminator(MirTerminator::Goto { target: exit_label });
            }
        }

        HirStmtKind::Continue => {
            // Continue jumps to the loop condition/header block
            if let Some(continue_label) = builder.continue_targets.last().cloned() {
                builder.set_terminator(MirTerminator::Goto {
                    target: continue_label,
                });
            }
        }

        HirStmtKind::If {
            condition,
            then_block,
            else_block,
        } => {
            let cond_val = builder.build_expr(condition);

            let then_label = builder.new_block_label("then");
            let else_label = builder.new_block_label("else");
            let merge_label = builder.new_block_label("merge");

            builder.set_terminator(MirTerminator::Branch {
                cond: cond_val,
                then_block: then_label.clone(),
                else_block: if else_block.is_some() {
                    else_label.clone()
                } else {
                    merge_label.clone()
                },
            });

            // Then block
            builder.add_block(&then_label);
            for stmt in then_block {
                build_stmt(builder, stmt);
            }
            builder.set_terminator(MirTerminator::Goto {
                target: merge_label.clone(),
            });

            // Else block
            if let Some(else_stmts) = else_block {
                builder.add_block(&else_label);
                for stmt in else_stmts {
                    build_stmt(builder, stmt);
                }
                builder.set_terminator(MirTerminator::Goto {
                    target: merge_label.clone(),
                });
            }

            // Merge block
            builder.add_block(&merge_label);
        }

        HirStmtKind::While { condition, body } => {
            let cond_label = builder.new_block_label("while_cond");
            let body_label = builder.new_block_label("while_body");
            let exit_label = builder.new_block_label("while_exit");

            // Jump to condition
            builder.set_terminator(MirTerminator::Goto {
                target: cond_label.clone(),
            });

            // Push loop labels for break/continue
            builder.break_targets.push(exit_label.clone());
            builder.continue_targets.push(cond_label.clone());

            // Condition block
            builder.add_block(&cond_label);
            let cond_val = builder.build_expr(condition);
            builder.set_terminator(MirTerminator::Branch {
                cond: cond_val,
                then_block: body_label.clone(),
                else_block: exit_label.clone(),
            });

            // Body block
            builder.add_block(&body_label);
            for stmt in body {
                build_stmt(builder, stmt);
            }
            builder.set_terminator(MirTerminator::Goto { target: cond_label });

            // Pop loop labels
            builder.break_targets.pop();
            builder.continue_targets.pop();

            // Exit block
            builder.add_block(&exit_label);
        }

        HirStmtKind::Drop { name } => {
            // Drop is a no-op at MIR level for now (could emit Drop instruction later)
            let span = builder.convert_span(stmt.span);
            builder.emit(
                MirInstrKind::Drop {
                    value: name.clone(),
                },
                span,
            );
        }

        HirStmtKind::ManualErrorExtract {
            ok_names,
            error_name,
            expr,
        } => {
            // Extract value and error from a Result-like expression
            let src = builder.build_expr(expr);
            let span = builder.convert_span(stmt.span);

            // Get the result type info from the expression
            // For now, use Any/Any as fallback types - the codegen will
            // determine actual types from the Result struct at runtime
            let ok_type = doo_core::types::builtin::ANY;
            let err_type = doo_core::types::builtin::ANY;

            // Emit ManualErrorExtract instruction
            builder.emit(
                MirInstrKind::ManualErrorExtract {
                    ok_names: ok_names.clone(),
                    error_name: error_name.clone(),
                    result: src,
                    ok_type,
                    err_type,
                },
                span,
            );
        }
    }
}
