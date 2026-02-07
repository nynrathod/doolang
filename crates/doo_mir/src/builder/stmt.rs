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
            // Detect container kind BEFORE building expression (for proper type tracking)
            let container_kind = builder.infer_container_kind(value);

            // Use build_expr_with_expected_type if we have a type annotation
            // This enables JSON.parse to use the expected type for proper parsing
            let val_operand = if type_id.is_some() {
                builder.build_expr_with_expected_type(value, *type_id)
            } else {
                builder.build_expr(value)
            };

            // Register the local variable in the function
            let var_type_id = type_id.unwrap_or_else(|| {
                // Infer type from the value operand using builder's method
                builder.infer_operand_type(&val_operand)
            });

            // Register container kind for this variable (used in index assignment)
            if let Some(kind) = container_kind {
                builder.container_kinds.insert(name.clone(), kind);
            }

            if let Some(f) = &mut builder.current_func {
                // Check if already registered - if so, update the type if different
                // This handles cases like payload bindings reusing variable names
                if let Some(existing) = f.locals.iter_mut().find(|l| l.name == *name) {
                    // Update to the new type (let binding takes precedence)
                    existing.type_id = var_type_id;
                    existing.mutable = *mutable;
                } else {
                    f.locals.push(LocalDef {
                        name: name.clone(),
                        type_id: var_type_id,
                        mutable: *mutable,
                    });
                }
            }

            // Also track the type in temp_types for method call type resolution
            // This allows db.raw() to know that 'db' is of type Database
            builder.set_temp_type(name, var_type_id);

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
            // Check if this is a call to a Result-returning function
            // If so, use ManualErrorExtract instead of TupleLet
            // Resolve aliases to handle imported associated functions
            let is_result_call = if let HirExprKind::Call { func, .. } = &value.kind {
                // Extract function name
                let func_name = match &func.kind {
                    HirExprKind::Local { name } => Some(name.as_str()),
                    _ => None,
                };
                // Check if it returns a Result (resolve alias first)
                func_name
                    .map(|name| {
                        let resolved_name = builder.resolve_function_name(name);
                        builder.function_result_types.contains_key(&resolved_name)
                    })
                    .unwrap_or(false)
            } else {
                false
            };

            if is_result_call && names.len() >= 2 {
                // Result type with tuple destructuring: last name is error variable
                // Split names into ok_names and error_name
                let (ok_names, error_name) = if names.len() == 2 {
                    // Simple case: let ok, err = ...
                    (vec![names[0].clone()], names[1].clone())
                } else {
                    // Multi-value Ok: let a, b, err = ... (last is error)
                    let ok_names: Vec<String> = names[..names.len() - 1].to_vec();
                    let error_name = names[names.len() - 1].clone();
                    (ok_names, error_name)
                };

                // Get Result's ok and err types from function_result_types
                // Resolve aliases to handle imported associated functions
                let (ok_type, err_type) = if let HirExprKind::Call { func, .. } = &value.kind {
                    if let HirExprKind::Local { name } = &func.kind {
                        let resolved_name = builder.resolve_function_name(name);
                        builder
                            .function_result_types
                            .get(&resolved_name)
                            .copied()
                            .unwrap_or((builtin::ANY, builtin::ANY))
                    } else {
                        (builtin::ANY, builtin::ANY)
                    }
                } else {
                    (builtin::ANY, builtin::ANY)
                };

                // Register ok variables as locals
                for ok_name in &ok_names {
                    if let Some(f) = &mut builder.current_func {
                        if !f.locals.iter().any(|l| l.name == *ok_name) {
                            f.locals.push(LocalDef {
                                name: ok_name.clone(),
                                type_id: ok_type,
                                mutable: *mutable,
                            });
                        }
                    }
                }

                // Register error variable as local
                if error_name != "_" {
                    if let Some(f) = &mut builder.current_func {
                        if !f.locals.iter().any(|l| l.name == error_name) {
                            f.locals.push(LocalDef {
                                name: error_name.clone(),
                                type_id: err_type,
                                mutable: *mutable,
                            });
                        }
                    }
                }

                // Build the call expression
                let result_operand = builder.build_expr(value);

                // Emit ManualErrorExtract instruction
                builder.emit(
                    MirInstrKind::ManualErrorExtract {
                        ok_names,
                        error_name,
                        result: result_operand,
                        ok_type,
                        err_type,
                    },
                    span,
                );

                return; // Early return - we've handled this case
            }

            // Standard tuple destructuring (not a Result-returning call)
            // Build the tuple value first
            let tuple_operand = builder.build_expr(value);

            // Get the value type - first from HIR, then from temp_types if it was a call
            let value_type = value.type_id.or_else(|| {
                if let MirOperand::Temp(ref temp_name) = tuple_operand {
                    builder.get_temp_type(temp_name)
                } else {
                    None
                }
            });

            let tuple_type = value_type;

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
                    // For index assignment like `arr[i] = value` or `map[key] = value`
                    let array_operand = builder.build_expr(object);
                    let index_operand = builder.build_expr(index);

                    // Determine if this is a Map or Array:
                    // 1. First check container_kinds cache (most reliable for inferred types)
                    // 2. Then try HIR type_id or infer from expression
                    // 3. Then look up from locals
                    let is_map = if let HirExprKind::Local { name } = &object.kind {
                        // Check container_kinds cache first
                        if let Some(kind) = builder.container_kinds.get(name).copied() {
                            matches!(kind, super::ContainerKind::Map)
                        } else {
                            // Fall back to type registry lookup
                            builder
                                .get_local_type(name)
                                .and_then(|t| builder.type_registry.get(t))
                                .map(|info| matches!(info.kind, TypeKind::Map { .. }))
                                .unwrap_or(false)
                        }
                    } else {
                        // For non-local objects (e.g., field access like self.Users),
                        // recursively infer the type
                        builder
                            .infer_hir_expr_type(object)
                            .and_then(|t| builder.type_registry.get(t))
                            .map(|info| matches!(info.kind, TypeKind::Map { .. }))
                            .unwrap_or(false)
                    };

                    // Get container type for extracting key/value types
                    // Use the recursive infer method for all expression kinds
                    let container_type = builder.infer_hir_expr_type(object);

                    if is_map {
                        // Map set: map[key] = value
                        let (key_type, val_type) = container_type
                            .and_then(|t| builder.type_registry.get(t))
                            .and_then(|info| {
                                if let TypeKind::Map { key, value } = &info.kind {
                                    Some((*key, *value))
                                } else {
                                    None
                                }
                            })
                            .unwrap_or((builtin::ANY, builtin::ANY));

                        builder.emit(
                            MirInstrKind::MapSet {
                                map: array_operand.clone(),
                                key: index_operand,
                                value: val_operand,
                                key_type,
                                val_type,
                            },
                            span,
                        );

                        // CRITICAL: If the map is a field, write back the potentially
                        // reallocated pointer. MapSet may realloc the map, updating
                        // the temp, but the original field still points to old memory.
                        if let HirExprKind::Field {
                            object: field_obj,
                            field,
                        } = &object.kind
                        {
                            let obj_operand = builder.build_expr(field_obj);
                            builder.emit(
                                MirInstrKind::FieldSet {
                                    object: obj_operand,
                                    field: field.clone(),
                                    value: array_operand,
                                },
                                span,
                            );
                        }
                    } else {
                        // Array set: arr[index] = value
                        let elem_type = container_type
                            .and_then(|t| builder.array_elem_type_from_type_id(t))
                            .unwrap_or(builtin::ANY);
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
            // Check if this is an Ok or Err expression - these implicitly return
            match &expr.kind {
                HirExprKind::Ok(_) | HirExprKind::Err(_) => {
                    // Build the expression to get the wrapped Result value
                    let result_operand = builder.build_expr(expr);
                    // Set the return terminator with this value
                    builder.set_terminator(MirTerminator::Return {
                        values: vec![result_operand],
                    });
                }
                _ => {
                    // Just evaluate the expression for side effects
                    builder.build_expr(expr);
                }
            }
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
            // Only add Goto if the block doesn't already have a terminator
            // (e.g., Ok/Err expressions implicitly return)
            builder.set_terminator_if_none(MirTerminator::Goto {
                target: merge_label.clone(),
            });

            // Else block
            if let Some(else_stmts) = else_block {
                builder.add_block(&else_label);
                for stmt in else_stmts {
                    build_stmt(builder, stmt);
                }
                // Only add Goto if the block doesn't already have a terminator
                builder.set_terminator_if_none(MirTerminator::Goto {
                    target: merge_label.clone(),
                });
            }

            // Merge block
            builder.add_block(&merge_label);
        }

        HirStmtKind::While { condition, body } => {
            if std::env::var("DOO_DEBUG").is_ok() {
                eprintln!(
                    "[MIR] Building While loop with {} body statements",
                    body.len()
                );
            }
            let cond_label = builder.new_block_label("while_cond");
            let body_label = builder.new_block_label("while_body");
            let exit_label = builder.new_block_label("while_exit");

            if std::env::var("DOO_DEBUG").is_ok() {
                eprintln!(
                    "[MIR] While labels: cond={}, body={}, exit={}",
                    cond_label, body_label, exit_label
                );
            }

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
            // Only add Goto if the block doesn't already have a terminator
            // (e.g., return inside the loop body)
            builder.set_terminator_if_none(MirTerminator::Goto { target: cond_label });

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
