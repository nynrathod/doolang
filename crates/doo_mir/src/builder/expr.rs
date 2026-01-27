use super::{ContainerKind, Decision, MirBuilder};
use crate::{BinaryOp, MirConst, MirInstrKind, MirOperand, MirTerminator};
use doo_core::{
    constants::ffi_names,
    types::{builtin, TypeId as CoreTypeId},
};
use doo_hir::{HirBinOp, HirExpr, HirExprKind};

/// Build an expression with an expected type hint.
/// This is used for return statements where we know the expected return type.
pub fn build_expr_with_expected_type(
    builder: &mut MirBuilder,
    expr: &HirExpr,
    expected_type: Option<CoreTypeId>,
) -> MirOperand {
    // For method calls, we may need to override the inferred type with the expected type
    // This is particularly important for JSON.parse() which returns ANY in HIR
    // but we know the expected type from the function's return type
    if let HirExprKind::MethodCall {
        receiver,
        method,
        args,
    } = &expr.kind
    {
        // Check if this is a JSON.parse call (receiver is the JSON module)
        let is_json_parse = matches!(&receiver.kind, HirExprKind::Local { name } if name == ffi_names::MODULE_JSON)
            && method == "parse";

        if is_json_parse {
            return build_method_call_with_type(builder, expr, receiver, method, args, expected_type);
        }
    }

    // For other expressions, use the regular build_expr
    build_expr(builder, expr)
}

/// Build a method call with an explicit return type (used for JSON.parse)
fn build_method_call_with_type(
    builder: &mut MirBuilder,
    expr: &HirExpr,
    receiver: &HirExpr,
    method: &str,
    args: &[HirExpr],
    expected_type: Option<CoreTypeId>,
) -> MirOperand {
    let span = builder.convert_span(expr.span);
    let recv = build_expr(builder, receiver);
    let receiver_type = receiver.type_id.unwrap_or(builtin::ANY);
    let arg_ops: Vec<_> = args.iter().map(|a| build_expr(builder, a)).collect();
    let arg_types: Vec<_> = args
        .iter()
        .zip(arg_ops.iter())
        .map(|(arg, op)| {
            arg.type_id
                .or_else(|| match op {
                    MirOperand::Temp(name) => builder.get_temp_type(name),
                    MirOperand::Local(name) => builder.get_local_type(name),
                    _ => None,
                })
                .unwrap_or(builtin::ANY)
        })
        .collect();
    let dest = builder.new_temp();

    // Use expected_type if provided, otherwise fall back to expr.type_id
    let return_type = expected_type.or(expr.type_id);
    if let Some(rt) = return_type {
        builder.set_temp_type(&dest, rt);
    }

    builder.emit(
        MirInstrKind::MethodCall {
            dest: Some(dest.clone()),
            receiver: recv,
            receiver_type,
            method: method.to_string(),
            args: arg_ops,
            arg_types,
            return_type,
        },
        span,
    );
    MirOperand::Temp(dest)
}

pub fn build_expr(builder: &mut MirBuilder, expr: &HirExpr) -> MirOperand {
    let span = builder.convert_span(expr.span);

    match &expr.kind {
        HirExprKind::Const(cv) => MirOperand::Const(builder.const_to_mir(cv)),

        HirExprKind::Local { name } => {
            // Built-in modules (JSON, Math, File, etc.) are treated as globals, not locals
            if ffi_names::is_builtin_module(name) {
                return MirOperand::Global(name.clone());
            }
            
            // Check ownership decision for this variable use
            if let Some(decision) = builder.get_ownership_decision(name, expr.span) {
                let dest = builder.new_temp();

                // Propagate type from local to temp for proper type tracking
                if let Some(local_type) = builder.get_local_type(name) {
                    builder.set_temp_type(&dest, local_type);
                }

                match decision {
                    Decision::Move => {
                        builder.emit(
                            MirInstrKind::Move {
                                dest: dest.clone(),
                                src: MirOperand::Local(name.clone()),
                            },
                            span,
                        );
                        MirOperand::Temp(dest)
                    }
                    Decision::Copy => {
                        builder.emit(
                            MirInstrKind::Copy {
                                dest: dest.clone(),
                                src: MirOperand::Local(name.clone()),
                            },
                            span,
                        );
                        MirOperand::Temp(dest)
                    }
                    Decision::Clone => {
                        builder.emit(
                            MirInstrKind::Clone {
                                dest: dest.clone(),
                                src: MirOperand::Local(name.clone()),
                            },
                            span,
                        );
                        MirOperand::Temp(dest)
                    }
                    Decision::Borrow { mutable } => {
                        builder.emit(
                            MirInstrKind::Borrow {
                                dest: dest.clone(),
                                src: name.clone(),
                                mutable,
                            },
                            span,
                        );
                        MirOperand::Temp(dest)
                    }
                }
            } else {
                // No ownership decision available - default to direct reference
                MirOperand::Local(name.clone())
            }
        }

        HirExprKind::Global { name } => MirOperand::Global(name.clone()),

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
                builder.emit(
                    MirInstrKind::BinaryOp {
                        dest: dest.clone(),
                        op: builder.binop_to_mir(*op),
                        lhs: l,
                        rhs: r,
                    },
                    span,
                );
                MirOperand::Temp(dest)
            }
        }

        HirExprKind::UnaryOp { op, operand } => {
            let inner = builder.build_expr(operand);
            let dest = builder.new_temp();
            builder.emit(
                MirInstrKind::UnaryOp {
                    dest: dest.clone(),
                    op: builder.unaryop_to_mir(*op),
                    operand: inner,
                },
                span,
            );
            MirOperand::Temp(dest)
        }

        HirExprKind::Call { func, args } => {
            let func_name = builder.expr_to_name(func);
            
            // Get expected parameter types for this function call
            // This enables JSON.parse and similar to use the expected type
            let param_types = builder.get_function_param_types(&func_name).cloned();
            
            // Build arguments with expected types when available
            let arg_ops: Vec<_> = args.iter()
                .enumerate()
                .map(|(i, a)| {
                    let expected_type = param_types.as_ref()
                        .and_then(|types| types.get(i).copied());
                    builder.build_expr_with_expected_type(a, expected_type)
                })
                .collect();

            // Intrinsic: print(...) -> MirInstrKind::Print
            if func_name == "print" {
                // Use infer_operand_type for proper type tracking (handles temps with recorded types)
                let value_types: Vec<_> = arg_ops
                    .iter()
                    .map(|op| builder.infer_operand_type(op))
                    .collect();
                builder.emit(
                    MirInstrKind::Print {
                        values: arg_ops,
                        value_types,
                    },
                    span,
                );
                MirOperand::Const(MirConst::Nil)
            // Intrinsic: typeOf(x) -> MirInstrKind::TypeOf
            } else if func_name == "typeOf" && args.len() == 1 {
                let dest = builder.new_temp();
                let value_type = args[0].type_id.unwrap_or(builtin::ANY);
                builder.emit(
                    MirInstrKind::TypeOf {
                        dest: dest.clone(),
                        value: arg_ops.into_iter().next().unwrap(),
                        value_type,
                    },
                    span,
                );
                MirOperand::Temp(dest)
            } else {
                let dest = builder.new_temp();
                builder.emit(
                    MirInstrKind::Call {
                        dest: Some(dest.clone()),
                        func: func_name.clone(),
                        args: arg_ops,
                    },
                    span,
                );

                // Record the return type of the call for type propagation
                // This is critical for tuple returns and other complex types
                if let Some(return_type) = builder.get_function_return_type(&func_name) {
                    builder.set_temp_type(&dest, return_type);
                }

                MirOperand::Temp(dest)
            }
        }

        HirExprKind::MethodCall {
            receiver,
            method,
            args,
        } => {
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
                        // Try receiver type_id first, then look up from locals
                        let container_type = receiver.type_id.or_else(|| {
                            if let HirExprKind::Local { name } = &receiver.kind {
                                builder.get_local_type(name)
                            } else {
                                None
                            }
                        });
                        let elem_type = container_type
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
                if matches!(
                    builder.infer_container_kind(receiver),
                    Some(ContainerKind::Map)
                ) {
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
                if matches!(
                    builder.infer_container_kind(receiver),
                    Some(ContainerKind::Array)
                ) {
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

            // Check if receiver is a module-like name (uppercase first char = static module call)
            // Modules like JSON, Math, File, etc. don't exist as local variables
            let is_module_receiver = matches!(&receiver.kind, HirExprKind::Local { name } 
                if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false));

            // Get receiver type - try HIR type first, then look up from locals/temps
            // Build receiver - for modules, use Global instead of Local
            let recv = if is_module_receiver {
                if let HirExprKind::Local { name } = &receiver.kind {
                    MirOperand::Global(name.clone())
                } else {
                    builder.build_expr(receiver)
                }
            } else {
                builder.build_expr(receiver)
            };

            let receiver_type = receiver
                .type_id
                .or_else(|| match &receiver.kind {
                    HirExprKind::Local { name } => builder.get_local_type(name),
                    _ => None,
                })
                .or_else(|| {
                    // If HIR didn't have type, check the built operand
                    match &recv {
                        MirOperand::Temp(name) => builder.get_temp_type(name),
                        MirOperand::Local(name) => builder.get_local_type(name),
                        _ => None,
                    }
                })
                .unwrap_or(builtin::ANY);

            let arg_ops: Vec<_> = args.iter().map(|a| builder.build_expr(a)).collect();

            // Get argument types - try HIR type first, then look up from the built operand
            let arg_types: Vec<_> = args
                .iter()
                .zip(arg_ops.iter())
                .map(|(a, op)| {
                    a.type_id
                        .or_else(|| {
                            // If HIR didn't have type, check the built operand
                            match op {
                                MirOperand::Temp(name) => builder.get_temp_type(name),
                                MirOperand::Local(name) => builder.get_local_type(name),
                                _ => None,
                            }
                        })
                        .unwrap_or(builtin::ANY)
                })
                .collect();
            let dest = builder.new_temp();

            // If the HIR expression has a type_id, set it on the temp
            // This ensures type inference from HIR is preserved
            let return_type = expr.type_id;
            if let Some(rt) = return_type {
                builder.set_temp_type(&dest, rt);
            }

            builder.emit(
                MirInstrKind::MethodCall {
                    dest: Some(dest.clone()),
                    receiver: recv,
                    receiver_type,
                    method: method.clone(),
                    args: arg_ops,
                    arg_types,
                    return_type,
                },
                span,
            );
            MirOperand::Temp(dest)
        }

        HirExprKind::Field { object, field } => {
            let obj = builder.build_expr(object);
            let dest = builder.new_temp();

            // Infer field type from the object's struct type
            let object_type = object.type_id.or_else(|| {
                if let HirExprKind::Local { name } = &object.kind {
                    builder.get_local_type(name)
                } else {
                    None
                }
            });
            if let Some(struct_type) = object_type {
                if let Some(field_type) = builder.struct_field_type(struct_type, field) {
                    builder.set_temp_type(&dest, field_type);
                }
            }

            builder.emit(
                MirInstrKind::FieldGet {
                    dest: dest.clone(),
                    object: obj,
                    field: field.clone(),
                },
                span,
            );
            MirOperand::Temp(dest)
        }

        HirExprKind::Index { object, index } => {
            let kind = builder.infer_container_kind(object);
            let dest = builder.new_temp();

            // Try to get container type from object.type_id, falling back to local lookup
            let container_type = object.type_id.or_else(|| {
                // If object is a local variable, look up its type
                if let HirExprKind::Local { name } = &object.kind {
                    builder.get_local_type(name)
                } else {
                    None
                }
            });

            // Check for Range indexing (slicing) by examining the HIR expression directly
            // This is more reliable than checking type_id which may not be set
            if let HirExprKind::Range {
                start,
                end,
                inclusive,
            } = &index.kind
            {
                // It's a slice operation: arr[start..end] or arr[start..=end]
                let obj = builder.build_expr(object);
                let start_val = builder.build_expr(start);
                let end_val = builder.build_expr(end);

                // For inclusive ranges, we need end + 1 for the slice
                let final_end = if *inclusive {
                    let end_temp = builder.new_temp();
                    builder.emit(
                        MirInstrKind::BinaryOp {
                            dest: end_temp.clone(),
                            op: BinaryOp::Add,
                            lhs: end_val,
                            rhs: MirOperand::Const(MirConst::Int(1)),
                        },
                        span,
                    );
                    MirOperand::Temp(end_temp)
                } else {
                    end_val
                };

                let elem_type = container_type
                    .and_then(|t| builder.array_elem_type_from_type_id(t))
                    .unwrap_or(builtin::ANY);

                builder.emit(
                    MirInstrKind::ArraySlice {
                        dest: dest.clone(),
                        array: obj,
                        start: start_val,
                        end: final_end,
                        elem_type,
                    },
                    span,
                );

                // Record the temp type as the same array type (slice produces same type)
                if let Some(arr_type) = container_type {
                    builder.set_temp_type(&dest, arr_type);
                }

                return MirOperand::Temp(dest);
            }

            // Not a Range - regular index access
            let obj = builder.build_expr(object);
            let idx = builder.build_expr(index);

            match kind {
                Some(ContainerKind::Map) => {
                    let (key_type, val_type) = container_type
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
                    // Record value type for the dest temp
                    builder.set_temp_type(&dest, val_type);
                }
                Some(ContainerKind::Array) | None => {
                    let elem_type = container_type
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
                    // Record element type for the dest temp
                    builder.set_temp_type(&dest, elem_type);
                }
            }
            MirOperand::Temp(dest)
        }

        HirExprKind::Array(elements) => {
            let has_spread = elements
                .iter()
                .any(|e| matches!(e.kind, HirExprKind::Spread(_)));

            // Infer elem_type: try expr.type_id first, then from elements
            let elem_type = expr
                .type_id
                .and_then(|t| builder.array_elem_type_from_type_id(t))
                .or_else(|| {
                    // Try to infer from first element (either spread or literal)
                    elements.first().and_then(|e| {
                        if let HirExprKind::Spread(inner) = &e.kind {
                            // For spread, get element type from the inner array
                            // First try inner's type_id, then look up local if it's a variable
                            inner
                                .type_id
                                .and_then(|t| builder.array_elem_type_from_type_id(t))
                                .or_else(|| {
                                    // If inner is a Local, look up its type
                                    if let HirExprKind::Local { name } = &inner.kind {
                                        builder
                                            .get_local_type(name)
                                            .and_then(|t| builder.array_elem_type_from_type_id(t))
                                    } else {
                                        None
                                    }
                                })
                        } else {
                            // Use the element's type directly
                            e.type_id
                        }
                    })
                })
                .unwrap_or(builtin::ANY);
            let dest = builder.new_temp();

            if has_spread {
                // Initialize empty array
                builder.emit(
                    MirInstrKind::ArrayCreate {
                        dest: dest.clone(),
                        elements: Vec::new(),
                        elem_type,
                    },
                    span,
                );

                // Push/Extend elements
                for e in elements {
                    if let HirExprKind::Spread(inner) = &e.kind {
                        let val = builder.build_expr(inner);
                        builder.emit(
                            MirInstrKind::ArrayExtend {
                                array: MirOperand::Temp(dest.clone()),
                                other: val,
                                elem_type,
                            },
                            span,
                        );
                    } else {
                        let val = builder.build_expr(e);
                        builder.emit(
                            MirInstrKind::ArrayPush {
                                array: MirOperand::Temp(dest.clone()),
                                value: val,
                            },
                            span,
                        );
                    }
                }
            } else {
                let elems: Vec<_> = elements.iter().map(|e| builder.build_expr(e)).collect();
                builder.emit(
                    MirInstrKind::ArrayCreate {
                        dest: dest.clone(),
                        elements: elems,
                        elem_type,
                    },
                    span,
                );
            }
            // Propagate array type to temp for type inference in later operations
            if let Some(array_type) = expr.type_id {
                builder.set_temp_type(&dest, array_type);
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
            builder.emit(
                MirInstrKind::MapCreate {
                    dest: dest.clone(),
                    entries: ents,
                    key_type,
                    val_type,
                },
                span,
            );
            // Propagate map type to temp for type inference in later operations
            if let Some(map_type) = expr.type_id {
                builder.set_temp_type(&dest, map_type);
            }
            MirOperand::Temp(dest)
        }

        HirExprKind::Tuple(elements) => {
            let elems: Vec<_> = elements.iter().map(|e| builder.build_expr(e)).collect();
            let dest = builder.new_temp();
            builder.emit(
                MirInstrKind::TupleCreate {
                    dest: dest.clone(),
                    elements: elems,
                },
                span,
            );
            MirOperand::Temp(dest)
        }

        HirExprKind::Struct { name, fields } => {
            let field_ops: Vec<_> = fields
                .iter()
                .map(|(n, v)| (n.clone(), builder.build_expr(v)))
                .collect();
            let dest = builder.new_temp();
            builder.emit(
                MirInstrKind::StructCreate {
                    dest: dest.clone(),
                    struct_name: name.clone(),
                    fields: field_ops,
                },
                span,
            );
            MirOperand::Temp(dest)
        }

        HirExprKind::EnumVariant {
            enum_name,
            variant,
            payload,
        } => {
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

        HirExprKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
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
            builder.emit(
                MirInstrKind::Assign {
                    dest: dest.clone(),
                    value: then_val,
                },
                span,
            );
            builder.set_terminator(MirTerminator::Goto {
                target: merge_label.clone(),
            });

            // Else
            builder.add_block(&else_label);
            if let Some(else_e) = else_expr {
                let else_val = builder.build_expr(else_e);
                builder.emit(
                    MirInstrKind::Assign {
                        dest: dest.clone(),
                        value: else_val,
                    },
                    span,
                );
            } else {
                builder.emit(
                    MirInstrKind::Assign {
                        dest: dest.clone(),
                        value: MirOperand::Const(MirConst::Nil),
                    },
                    span,
                );
            }
            builder.set_terminator(MirTerminator::Goto {
                target: merge_label.clone(),
            });

            builder.add_block(&merge_label);
            MirOperand::Temp(dest)
        }

        HirExprKind::Block {
            stmts,
            expr: final_expr,
        } => {
            for s in stmts {
                builder.build_stmt(s);
            }
            if let Some(e) = final_expr {
                builder.build_expr(e)
            } else {
                MirOperand::Const(MirConst::Nil)
            }
        }

        HirExprKind::Range {
            start,
            end,
            inclusive,
        } => {
            let s = builder.build_expr(start);
            let e = builder.build_expr(end);
            let dest = builder.new_temp();
            builder.emit(
                MirInstrKind::StructCreate {
                    dest: dest.clone(),
                    struct_name: "Range".to_string(),
                    fields: vec![
                        ("start".to_string(), s),
                        ("end".to_string(), e),
                        (
                            "inclusive".to_string(),
                            MirOperand::Const(MirConst::Bool(*inclusive)),
                        ),
                    ],
                },
                span,
            );
            MirOperand::Temp(dest)
        }

        HirExprKind::Ok(inner) => {
            let val = builder.build_expr(inner);
            let dest = builder.new_temp();
            builder.emit(
                MirInstrKind::WrapOk {
                    dest: dest.clone(),
                    value: val,
                },
                span,
            );
            MirOperand::Temp(dest)
        }

        HirExprKind::Err(inner) => {
            let val = builder.build_expr(inner);
            let dest = builder.new_temp();
            builder.emit(
                MirInstrKind::WrapErr {
                    dest: dest.clone(),
                    value: val,
                },
                span,
            );
            MirOperand::Temp(dest)
        }

        HirExprKind::Try(inner) => {
            let val = builder.build_expr(inner);
            let dest = builder.new_temp();
            let is_ok_dest = builder.new_temp();

            builder.emit(
                MirInstrKind::IsOk {
                    dest: is_ok_dest.clone(),
                    value: val.clone(),
                },
                span,
            );
            builder.emit(
                MirInstrKind::UnwrapOk {
                    dest: dest.clone(),
                    value: val,
                },
                span,
            );

            MirOperand::Temp(dest)
        }

        HirExprKind::UnwrapOrPanic {
            expr: inner,
            message,
        } => {
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
            builder.emit(
                MirInstrKind::Clone {
                    dest: dest.clone(),
                    src: val,
                },
                span,
            );
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

        HirExprKind::Spread(inner) => {
            // Spread is just passing through the inner expression
            builder.build_expr(inner)
        }

        HirExprKind::Cast { value, to_type } => {
            // Build the value and emit a cast instruction
            let val = builder.build_expr(value);
            let dest = builder.new_temp();
            let span = builder.convert_span(expr.span);
            builder.emit(
                MirInstrKind::Cast {
                    dest: dest.clone(),
                    value: val,
                    to_type: *to_type,
                },
                span,
            );
            MirOperand::Temp(dest)
        }
    }
}
