use doo_hir::{HirExpr, HirExprKind, HirBinOp};
use doo_mir::{MirInstrKind, MirTerminator, MirOperand, MirConst, BinaryOp};
use crate::types::{MirConst as MirConstType};
use super::{MirBuilder, ContainerKind};
use doo_core::types::builtin;

pub fn build_expr(builder: &mut MirBuilder, expr: &HirExpr) -> MirOperand {
    let span = builder.convert_span(expr.span);
    
    match &expr.kind {
        HirExprKind::Const(cv) => {
            MirOperand::Const(builder.const_to_mir(cv))
        }

        HirExprKind::Local { name } => {
            MirOperand::Local(name.clone())
        }

        HirExprKind::Global { name } => {
            MirOperand::Global(name.clone())
        }

        HirExprKind::BinOp { op, lhs, rhs } => {
            if *op == HirBinOp::In {
                // Syntax: `value in container` -> lhs=value, rhs=container
                let value = builder.build_expr(lhs);
                let container = builder.build_expr(rhs);
                let dest = builder.new_temp();

                // Check container type from rhs (the actual container)
                let kind = builder.infer_container_kind(rhs);
                match kind {
                    Some(ContainerKind::Map) => {
                        let (key_type, val_type) = rhs
                            .type_id
                            .and_then(|t| builder.map_types_from_type_id(t))
                            .unwrap_or((builtin::ANY, builtin::ANY));
                        builder.emit(
                            MirInstrKind::MapHas {
                                dest: dest.clone(),
                                map: container,
                                key: value,
                                key_type,
                                val_type,
                            },
                            span,
                        );
                    }
                    Some(ContainerKind::Array) | None => {
                        let elem_type = rhs
                            .type_id
                            .and_then(|t| builder.array_elem_type_from_type_id(t))
                            .unwrap_or(builtin::ANY);
                        builder.emit(
                            MirInstrKind::ArrayContains {
                                dest: dest.clone(),
                                array: container,
                                value,
                                elem_type,
                            },
                            span,
                        );
                    }
                }

                MirOperand::Temp(dest)
            } else {
                let l = builder.build_expr(lhs);
                let r = builder.build_expr(rhs);
                let dest = builder.new_temp();
                builder.emit(MirInstrKind::BinaryOp {
                    dest: dest.clone(),
                    op: builder.binop_to_mir(*op),
                    lhs: l,
                    rhs: r,
                }, span);
                MirOperand::Temp(dest)
            }
        }

        HirExprKind::UnaryOp { op, operand } => {
            let inner = builder.build_expr(operand);
            let dest = builder.new_temp();
            builder.emit(MirInstrKind::UnaryOp {
                dest: dest.clone(),
                op: builder.unaryop_to_mir(*op),
                operand: inner,
            }, span);
            MirOperand::Temp(dest)
        }

        HirExprKind::Call { func, args } => {
            let func_name = builder.expr_to_name(func);
            let arg_ops: Vec<_> = args.iter().map(|a| builder.build_expr(a)).collect();

            // Intrinsic: print(...) -> MirInstrKind::Print
            if func_name == "print" {
                let value_types: Vec<_> = args
                    .iter()
                    .map(|a| a.type_id.unwrap_or(builtin::ANY))
                    .collect();
                builder.emit(MirInstrKind::Print { values: arg_ops, value_types }, span);
                MirOperand::Const(MirConst::Nil)
            } else {
                let dest = builder.new_temp();
                builder.emit(MirInstrKind::Call {
                    dest: Some(dest.clone()),
                    func: func_name,
                    args: arg_ops,
                }, span);
                MirOperand::Temp(dest)
            }
        }

        HirExprKind::MethodCall { receiver, method, args } => {
            if method == "__set" && args.len() == 2 {
                let recv_kind = builder.infer_container_kind(receiver);
                let recv = builder.build_expr(receiver);
                let idx = builder.build_expr(&args[0]);
                let val = builder.build_expr(&args[1]);
                match recv_kind {
                    Some(ContainerKind::Map) => {
                        let (key_type, val_type) = receiver
                            .type_id
                            .and_then(|t| builder.map_types_from_type_id(t))
                            .unwrap_or((builtin::ANY, builtin::ANY));
                        builder.emit(
                            MirInstrKind::MapSet {
                                map: recv,
                                key: idx,
                                value: val,
                                key_type,
                                val_type,
                            },
                            span,
                        );
                        return MirOperand::Const(MirConst::Nil);
                    }
                    Some(ContainerKind::Array) | None => {
                        let elem_type = receiver
                            .type_id
                            .and_then(|t| builder.array_elem_type_from_type_id(t))
                            .unwrap_or(builtin::ANY);
                        builder.emit(
                            MirInstrKind::ArraySet {
                                array: recv,
                                index: idx,
                                value: val,
                                elem_type,
                            },
                            span,
                        );
                        return MirOperand::Const(MirConst::Nil);
                    }
                }
            }

            // Centralized membership operations:
            // - map.has(key) -> MirInstrKind::MapHas
            // - array.contains(value) -> MirInstrKind::ArrayContains
            if method == "has" && args.len() == 1 {
                if matches!(builder.infer_container_kind(receiver), Some(ContainerKind::Map)) {
                    let (key_type, val_type) = receiver
                        .type_id
                        .and_then(|t| builder.map_types_from_type_id(t))
                        .unwrap_or((builtin::ANY, builtin::ANY));
                    let recv = builder.build_expr(receiver);
                    let key = builder.build_expr(&args[0]);
                    let dest = builder.new_temp();
                    builder.emit(
                        MirInstrKind::MapHas {
                            dest: dest.clone(),
                            map: recv,
                            key,
                            key_type,
                            val_type,
                        },
                        span,
                    );
                    return MirOperand::Temp(dest);
                }
            }

            if method == "contains" && args.len() == 1 {
                if matches!(builder.infer_container_kind(receiver), Some(ContainerKind::Array)) {
                    let elem_type = receiver
                        .type_id
                        .and_then(|t| builder.array_elem_type_from_type_id(t))
                        .unwrap_or(builtin::ANY);
                    let recv = builder.build_expr(receiver);
                    let needle = builder.build_expr(&args[0]);
                    let dest = builder.new_temp();
                    builder.emit(
                        MirInstrKind::ArrayContains {
                            dest: dest.clone(),
                            array: recv,
                            value: needle,
                            elem_type,
                        },
                        span,
                    );
                    return MirOperand::Temp(dest);
                }
            }

            let receiver_type = receiver.type_id.unwrap_or(builtin::ANY);
            let recv = builder.build_expr(receiver);
            let arg_ops: Vec<_> = args.iter().map(|a| builder.build_expr(a)).collect();
            let arg_types: Vec<_> = args.iter().map(|a| a.type_id.unwrap_or(builtin::ANY)).collect();
            let dest = builder.new_temp();
            builder.emit(MirInstrKind::MethodCall {
                dest: Some(dest.clone()),
                receiver: recv,
                receiver_type,
                method: method.clone(),
                args: arg_ops,
                arg_types,
            }, span);
            MirOperand::Temp(dest)
        }

        HirExprKind::Field { object, field } => {
            let obj = builder.build_expr(object);
            let dest = builder.new_temp();
            builder.emit(MirInstrKind::FieldGet {
                dest: dest.clone(),
                object: obj,
                field: field.clone(),
            }, span);
            MirOperand::Temp(dest)
        }

        HirExprKind::Index { object, index } => {
            let kind = builder.infer_container_kind(object);
            let obj = builder.build_expr(object);
            let idx = builder.build_expr(index);
            let dest = builder.new_temp();

            // Check for Range indexing (slicing)
            let is_range = index.type_id.is_some() && index.type_id == builder.type_registry.lookup("Range");
            
            if is_range {
                // If it's a Range, it's a slice operation
                // Range struct: { start, end, inclusive }
                // We need to extract start/end from the Range object at runtime?
                // Or does slice instruction take the Range object directly?
                // MirInstrKind::ArraySlice takes { array, start, end }
                // But idx is the Range object.
                // We should probably extract fields from range object.
                // Or update ArraySlice to take Range object.
                // Extracting is safer.
                
                let range_ty = builder.type_registry.lookup("Range").expect("Range type missing");
                let start_dest = builder.new_temp();
                builder.emit(MirInstrKind::FieldGet {
                    dest: start_dest.clone(),
                    object: idx.clone(),
                    field: "0".to_string(),
                }, span);
                
                let end_dest = builder.new_temp();
                builder.emit(MirInstrKind::FieldGet {
                    dest: end_dest.clone(),
                    object: idx.clone(),
                    field: "1".to_string(),
                }, span);

                // Assuming ArraySlice handles exclusive/inclusive or normalized end.
                // Range includes 'inclusive' field too.
                // For simplicity, let's assume ArraySlice takes start/end indices.
                
                let elem_type = object
                    .type_id
                    .and_then(|t| builder.array_elem_type_from_type_id(t))
                    .unwrap_or(builtin::ANY);

                builder.emit(MirInstrKind::ArraySlice {
                    dest: dest.clone(),
                    array: obj,
                    start: MirOperand::Temp(start_dest),
                    end: MirOperand::Temp(end_dest),
                    elem_type,
                }, span);
                
                return MirOperand::Temp(dest);
            }

            match kind {
                Some(ContainerKind::Map) => {
                    let (key_type, val_type) = object
                        .type_id
                        .and_then(|t| builder.map_types_from_type_id(t))
                        .unwrap_or((builtin::ANY, builtin::ANY));
                    builder.emit(
                        MirInstrKind::MapGet {
                            dest: dest.clone(),
                            map: obj,
                            key: idx,
                            key_type,
                            val_type,
                        },
                        span,
                    );
                }
                Some(ContainerKind::Array) | None => {
                    let elem_type = object
                        .type_id
                        .and_then(|t| builder.array_elem_type_from_type_id(t))
                        .unwrap_or(builtin::ANY);
                    builder.emit(
                        MirInstrKind::ArrayGet {
                            dest: dest.clone(),
                            array: obj,
                            index: idx,
                            elem_type,
                        },
                        span,
                    );
                }
            }
            MirOperand::Temp(dest)
        }

        HirExprKind::Array(elements) => {
            let has_spread = elements.iter().any(|e| matches!(e.kind, HirExprKind::Spread(_)));
            let elem_type = expr
                .type_id
                .and_then(|t| builder.array_elem_type_from_type_id(t))
                .unwrap_or(builtin::ANY);
            let dest = builder.new_temp();

            if has_spread {
                // Initialize empty array
                builder.emit(MirInstrKind::ArrayCreate {
                    dest: dest.clone(),
                    elements: Vec::new(),
                    elem_type,
                }, span);

                // Push/Extend elements
                for e in elements {
                    if let HirExprKind::Spread(inner) = &e.kind {
                        let val = builder.build_expr(inner);
                        builder.emit(MirInstrKind::ArrayExtend {
                            array: MirOperand::Temp(dest.clone()),
                            other: val,
                            elem_type,
                        }, span);
                    } else {
                        let val = builder.build_expr(e);
                        builder.emit(MirInstrKind::ArrayPush {
                            array: MirOperand::Temp(dest.clone()),
                            value: val,
                        }, span);
                    }
                }
            } else {
                let elems: Vec<_> = elements.iter().map(|e| builder.build_expr(e)).collect();
                builder.emit(MirInstrKind::ArrayCreate {
                    dest: dest.clone(),
                    elements: elems,
                    elem_type,
                }, span);
            }
            MirOperand::Temp(dest)
        }

        HirExprKind::Map(entries) => {
            let ents: Vec<_> = entries
                .iter()
                .map(|(k, v)| (builder.build_expr(k), builder.build_expr(v)))
                .collect();
            let dest = builder.new_temp();

            let (key_type, val_type) = expr
                .type_id
                .and_then(|t| builder.map_types_from_type_id(t))
                .unwrap_or((builtin::ANY, builtin::ANY));
            builder.emit(MirInstrKind::MapCreate {
                dest: dest.clone(),
                entries: ents,
                key_type,
                val_type,
            }, span);
            MirOperand::Temp(dest)
        }

        HirExprKind::Tuple(elements) => {
            let elems: Vec<_> = elements.iter().map(|e| builder.build_expr(e)).collect();
            let dest = builder.new_temp();
            builder.emit(MirInstrKind::TupleCreate {
                dest: dest.clone(),
                elements: elems,
            }, span);
            MirOperand::Temp(dest)
        }

        HirExprKind::Struct { name, fields } => {
            let field_ops: Vec<_> = fields.iter()
                .map(|(n, v)| (n.clone(), builder.build_expr(v)))
                .collect();
            let dest = builder.new_temp();
            builder.emit(MirInstrKind::StructCreate {
                dest: dest.clone(),
                struct_name: name.clone(),
                fields: field_ops,
            }, span);
            MirOperand::Temp(dest)
        }

        HirExprKind::EnumVariant { enum_name, variant, payload } => {
            let payload_op = if payload.is_empty() {
                None
            } else if payload.len() == 1 {
                Some(builder.build_expr(&payload[0]))
            } else {
                let ops: Vec<_> = payload.iter().map(|e| builder.build_expr(e)).collect();
                let tuple_dest = builder.new_temp();
                builder.emit(
                    MirInstrKind::TupleCreate {
                        dest: tuple_dest.clone(),
                        elements: ops,
                    },
                    span,
                );
                Some(MirOperand::Temp(tuple_dest))
            };

            let dest = builder.new_temp();
            builder.emit(
                MirInstrKind::EnumCreate {
                    dest: dest.clone(),
                    enum_name: enum_name.clone(),
                    variant: variant.clone(),
                    payload: payload_op,
                },
                span,
            );
            MirOperand::Temp(dest)
        }

        HirExprKind::Match { values, arms } => {
            let scrutinees: Vec<_> = values.iter().map(|v| builder.build_expr(v)).collect();

            let dest = builder.new_temp();
            let merge_label = builder.new_block_label("match_merge");

            let mut next_label: Option<String> = None;
            for (idx, arm) in arms.iter().enumerate() {
                let is_last = idx + 1 == arms.len();
                let arm_label = builder.new_block_label("match_arm");

                if idx == 0 {
                    // current block continues
                } else if let Some(label) = next_label.take() {
                    builder.add_block(&label);
                }

                if !is_last {
                    let next = builder.new_block_label("match_next");
                    let cond = builder.build_match_condition(&scrutinees, &arm.pattern, span);
                    builder.set_terminator(MirTerminator::Branch {
                        cond,
                        then_block: arm_label.clone(),
                        else_block: next.clone(),
                    });
                    next_label = Some(next);
                } else {
                    builder.set_terminator(MirTerminator::Goto {
                        target: arm_label.clone(),
                    });
                }

                builder.add_block(&arm_label);
                let body_val = builder.build_expr(&arm.body);
                builder.emit(
                    MirInstrKind::Assign {
                        dest: dest.clone(),
                        value: body_val,
                    },
                    span,
                );
                builder.set_terminator(MirTerminator::Goto {
                    target: merge_label.clone(),
                });
            }

            builder.add_block(&merge_label);
            MirOperand::Temp(dest)
        }

        HirExprKind::If { condition, then_expr, else_expr } => {
            let cond = builder.build_expr(condition);
            let dest = builder.new_temp();
            
            let then_label = builder.new_block_label("if_then");
            let else_label = builder.new_block_label("if_else");
            let merge_label = builder.new_block_label("if_merge");

            builder.set_terminator(MirTerminator::Branch {
                cond,
                then_block: then_label.clone(),
                else_block: else_label.clone(),
            });

            // Then
            builder.add_block(&then_label);
            let then_val = builder.build_expr(then_expr);
             builder.emit(MirInstrKind::Assign {
                dest: dest.clone(),
                value: then_val,
            }, span);
            builder.set_terminator(MirTerminator::Goto { target: merge_label.clone() });

            // Else
            builder.add_block(&else_label);
            if let Some(else_e) = else_expr {
                let else_val = builder.build_expr(else_e);
                builder.emit(MirInstrKind::Assign {
                    dest: dest.clone(),
                    value: else_val,
                }, span);
            } else {
                 builder.emit(MirInstrKind::Assign {
                    dest: dest.clone(),
                    value: MirOperand::Const(MirConst::Nil),
                }, span);
            }
            builder.set_terminator(MirTerminator::Goto { target: merge_label.clone() });

            builder.add_block(&merge_label);
            MirOperand::Temp(dest)
        }

        HirExprKind::Block { stmts, expr: final_expr } => {
            for s in stmts {
                builder.build_stmt(s);
            }
            if let Some(e) = final_expr {
                builder.build_expr(e)
            } else {
                MirOperand::Const(MirConst::Nil)
            }
        }

        HirExprKind::Range { start, end, inclusive } => {
            let s = builder.build_expr(start);
            let e = builder.build_expr(end);
            let dest = builder.new_temp();
            builder.emit(MirInstrKind::StructCreate {
                dest: dest.clone(),
                struct_name: "Range".to_string(),
                fields: vec![
                    ("start".to_string(), s),
                    ("end".to_string(), e),
                    ("inclusive".to_string(), MirOperand::Const(MirConst::Bool(*inclusive))),
                ],
            }, span);
            MirOperand::Temp(dest)
        }

        HirExprKind::Ok(inner) => {
            let val = builder.build_expr(inner);
            let dest = builder.new_temp();
            builder.emit(MirInstrKind::WrapOk { 
                dest: dest.clone(), 
                value: val 
            }, span);
            MirOperand::Temp(dest)
        }

        HirExprKind::Err(inner) => {
            let val = builder.build_expr(inner);
            let dest = builder.new_temp();
            builder.emit(MirInstrKind::WrapErr { 
                dest: dest.clone(), 
                value: val 
            }, span);
            MirOperand::Temp(dest)
        }

        HirExprKind::Try(inner) => {
            let val = builder.build_expr(inner);
            let dest = builder.new_temp();
            let is_ok_dest = builder.new_temp();
            
            builder.emit(MirInstrKind::IsOk { 
                dest: is_ok_dest.clone(), 
                value: val.clone() 
            }, span);
            builder.emit(MirInstrKind::UnwrapOk { 
                dest: dest.clone(), 
                value: val 
            }, span);
            
            MirOperand::Temp(dest)
        }

            HirExprKind::UnwrapOrPanic { expr: inner, message } => {
                let result_val = builder.build_expr(inner);
                let is_ok_dest = builder.new_temp();
                builder.emit(
                    MirInstrKind::IsOk {
                        dest: is_ok_dest.clone(),
                        value: result_val.clone(),
                    },
                    span,
                );

                let ok_label = builder.new_block_label("unwrap_ok");
                let err_label = builder.new_block_label("unwrap_err");
                let merge_label = builder.new_block_label("unwrap_merge");

                builder.set_terminator(MirTerminator::Branch {
                    cond: MirOperand::Temp(is_ok_dest),
                    then_block: ok_label.clone(),
                    else_block: err_label.clone(),
                });

                let dest = builder.new_temp();

                // Ok path: unwrap and continue
                builder.add_block(&ok_label);
                builder.emit(
                    MirInstrKind::UnwrapOk {
                        dest: dest.clone(),
                        value: result_val.clone(),
                    },
                    span,
                );
                builder.set_terminator(MirTerminator::Goto {
                    target: merge_label.clone(),
                });

                // Err path: evaluate panic expression and abort
                builder.add_block(&err_label);
                let _ = builder.build_expr(message);
                builder.set_terminator(MirTerminator::Unreachable);

                // Merge
                builder.add_block(&merge_label);

                MirOperand::Temp(dest)
            }

        HirExprKind::Clone(inner) => {
            let val = builder.build_expr(inner);
            let dest = builder.new_temp();
            builder.emit(MirInstrKind::Clone { 
                dest: dest.clone(), 
                src: val 
            }, span);
            MirOperand::Temp(dest)
        }

        HirExprKind::Move(inner) | HirExprKind::Borrow { expr: inner, .. } => {
            builder.build_expr(inner)
        }

        HirExprKind::Closure { params, body } => {
            let dest = builder.new_temp();
            let _body_val = builder.build_expr(body);
            MirOperand::Temp(dest)
        }
    }
}
