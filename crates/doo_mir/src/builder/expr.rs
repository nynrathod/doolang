use super::{ContainerKind, Decision, MirBuilder, LocalDef};
use crate::{BinaryOp, MirConst, MirInstrKind, MirOperand, MirTerminator};
use crate::sym::{Sym, sym, resolve};
use doo_core::{
    constants::ffi_names,
    doo_debug,
    types::{builtin, TypeId as CoreTypeId, TypeKind},
};
use doo_hir::{HirBinOp, HirExpr, HirExprKind, HirMatchPattern};

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
                    MirOperand::Temp(name) => builder.get_temp_type(*name),
                    MirOperand::Local(name) => builder.get_local_type(&resolve(*name)),
                    _ => None,
                })
                .unwrap_or(builtin::ANY)
        })
        .collect();
    let dest = builder.new_temp();

    // Use expected_type if provided, otherwise fall back to expr.type_id
    let return_type = expected_type.or(expr.type_id);
    if let Some(rt) = return_type {
        builder.set_temp_type(dest, rt);
    }

    builder.emit(
        MirInstrKind::MethodCall {
            dest: Some(dest),
            receiver: recv,
            receiver_type,
            method: sym(method),
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

    // DEBUG: Track what kind of expressions are being processed
    let is_add_func = builder.current_func.as_ref().map(|f| resolve(f.name).contains("add")).unwrap_or(false);
    if is_add_func {
        doo_debug!("MIR", "build_expr: Processing {:?} in add function", expr.kind);
    }

    match &expr.kind {
        HirExprKind::Const(cv) => MirOperand::Const(builder.const_to_mir(cv)),

        HirExprKind::Local { name } => {
            // Built-in modules (JSON, Math, File, etc.) are treated as globals, not locals
            if ffi_names::is_builtin_module(name) {
                return MirOperand::Global(sym(name));
            }

            // CRITICAL: Check if name is a known local variable/parameter FIRST.
            // Local variables shadow function aliases (e.g., a param named "auth"
            // must NOT resolve to _method_Server_auth via function_aliases).
            let is_local = builder.get_local_type(name).is_some();
            
            // Check if this is a function reference (not a variable)
            // Function references are used when passing functions to FFI (e.g., handlers)
            if !is_local && builder.is_function_name(name) {
                return MirOperand::FuncRef(sym(name));
            }

            // Check if this is a type name (struct or enum) used as a value
            // Type names are converted to their string representation for FFI
            // This handles cases like `app.auth("/signup", "/login", User, db)`
            // where `User` is a struct name passed to FFI expecting its string name
            if !is_local && builder.is_type_name(name) {
                return MirOperand::Const(MirConst::Str(name.clone()));
            }
            
            // Check ownership decision for this variable use
            if let Some(decision) = builder.get_ownership_decision(name, expr.span) {
                let dest = builder.new_temp();

                // Propagate type from local/temp to new temp for proper type tracking.
                // CRITICAL: Check temp_types FIRST to handle shadowed bindings correctly.
                // When a match binding shadows a local with a different type, we register
                // the binding's type in temp_types, which should take precedence.
                let propagated_type = builder.get_temp_type(sym(name))
                    .or_else(|| builder.get_local_type(name));
                if let Some(ty) = propagated_type {
                    builder.set_temp_type(dest, ty);
                }

                match decision {
                    Decision::Move => {
                        builder.emit(
                            MirInstrKind::Move {
                                dest,
                                src: MirOperand::Local(sym(name)),
                            },
                            span,
                        );
                        MirOperand::Temp(dest)
                    }
                    Decision::Copy => {
                        builder.emit(
                            MirInstrKind::Copy {
                                dest,
                                src: MirOperand::Local(sym(name)),
                            },
                            span,
                        );
                        MirOperand::Temp(dest)
                    }
                    Decision::Clone => {
                        builder.emit(
                            MirInstrKind::Clone {
                                dest,
                                src: MirOperand::Local(sym(name)),
                            },
                            span,
                        );
                        MirOperand::Temp(dest)
                    }
                    Decision::Borrow { mutable } => {
                        builder.emit(
                            MirInstrKind::Borrow {
                                dest,
                                src: sym(name),
                                mutable,
                            },
                            span,
                        );
                        MirOperand::Temp(dest)
                    }
                }
            } else {
                // No ownership decision available - default to direct reference
                MirOperand::Local(sym(name))
            }
        }

        HirExprKind::Global { name } => MirOperand::Global(sym(name)),

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
                                dest,
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
                                dest,
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
                
                // Propagate type information from the expression, or infer from operands
                let type_id = expr.type_id.or_else(|| {
                    // Infer result type from operands
                    // For comparison ops, result is always Bool
                    match op {
                        HirBinOp::Eq | HirBinOp::NotEq | HirBinOp::Lt | HirBinOp::Gt | HirBinOp::LtEq | HirBinOp::GtEq => {
                            Some(doo_core::types::builtin::BOOL)
                        }
                        HirBinOp::And | HirBinOp::Or => {
                            Some(doo_core::types::builtin::BOOL)
                        }
                        _ => {
                            // For Add/Sub/Mul/Div/Mod, use LHS type, or infer from operands
                            lhs.type_id.or_else(|| {
                                let inferred = builder.infer_operand_type(&l);
                                if inferred != doo_core::types::builtin::ANY {
                                    Some(inferred)
                                } else {
                                    None
                                }
                            })
                        }
                    }
                });
                if let Some(tid) = type_id {
                    builder.set_temp_type(dest, tid);
                }
                
                builder.emit(
                    MirInstrKind::BinaryOp {
                        dest,
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
            
            // Propagate type information from the expression or operand
            // For Neg, the result type is the same as the operand type
            if let Some(type_id) = expr.type_id.or(operand.type_id) {
                builder.set_temp_type(dest, type_id);
            }
            
            builder.emit(
                MirInstrKind::UnaryOp {
                    dest,
                    op: builder.unaryop_to_mir(*op),
                    operand: inner,
                },
                span,
            );
            MirOperand::Temp(dest)
        }

        HirExprKind::Call { func, args } => {
            let func_name = builder.expr_to_name(func);
            // Resolve alias to get canonical function name
            let resolved_func_name = builder.resolve_function_name(&func_name);
            
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

            // Intrinsic: print(...) or println(...) -> MirInstrKind::Print
            if func_name == "print" || func_name == "println" || func_name == "__print_interp" {
                let separator = func_name != "__print_interp";
                // Use infer_operand_type for proper type tracking (handles temps with recorded types)
                let value_types: Vec<_> = arg_ops
                    .iter()
                    .map(|op| builder.infer_operand_type(op))
                    .collect();
                builder.emit(
                    MirInstrKind::Print {
                        values: arg_ops,
                        value_types,
                        separator,
                    },
                    span,
                );
                MirOperand::Const(MirConst::Nil)
            // Intrinsic: sleep(ms) -> MirInstrKind::Sleep
            } else if func_name == "sleep" && args.len() == 1 {
                builder.emit(
                    MirInstrKind::Sleep {
                        ms: arg_ops.into_iter().next().unwrap(),
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
                        dest,
                        value: arg_ops.into_iter().next().unwrap(),
                        value_type,
                    },
                    span,
                );
                MirOperand::Temp(dest)
            } else if let Some(ffi_info) = builder.get_ffi_info(&func_name).cloned() {
                // FFI function call - emit FfiCall instead of Call
                let dest = builder.new_temp();

                // Pad missing optional parameters with Nil (null pointer)
                // This handles calls like Fetch(url) where options: {Str: Str}? is omitted
                let mut ffi_args = arg_ops;
                if let Some(param_types) = builder.get_function_param_types(&func_name) {
                    let expected = param_types.len();
                    while ffi_args.len() < expected {
                        ffi_args.push(MirOperand::Const(MirConst::Nil));
                    }
                }

                builder.emit(
                    MirInstrKind::FfiCall {
                        dest: Some(dest),
                        lib: sym(&ffi_info.library),
                        symbol: sym(&ffi_info.symbol),
                        args: ffi_args,
                    },
                    span,
                );

                // Record the return type of the call for type propagation
                if let Some(return_type) = builder.get_function_return_type(&func_name) {
                    builder.set_temp_type(dest, return_type);
                }

                MirOperand::Temp(dest)
            } else {
                let dest = builder.new_temp();
                builder.emit(
                    MirInstrKind::Call {
                        dest: Some(dest),
                        func: sym(&resolved_func_name),
                        args: arg_ops,
                    },
                    span,
                );

                // Record the return type of the call for type propagation
                // This is critical for tuple returns and other complex types
                if let Some(return_type) = builder.get_function_return_type(&func_name) {
                    builder.set_temp_type(dest, return_type);
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
                            dest,
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
                            dest,
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

            // Build receiver FIRST - for modules, use Global instead of Local
            // We need the receiver built before we can determine its type for FFI lookup
            let recv = if is_module_receiver {
                if let HirExprKind::Local { name } = &receiver.kind {
                    MirOperand::Global(sym(name))
                } else {
                    builder.build_expr(receiver)
                }
            } else {
                builder.build_expr(receiver)
            };

            // Compute receiver_type using all available sources (HIR, local, temp)
            let receiver_type = receiver
                .type_id
                .or_else(|| match &receiver.kind {
                    HirExprKind::Local { name } => builder.get_local_type(name),
                    _ => None,
                })
                .or_else(|| {
                    // If HIR didn't have type, check the built operand
                    // This is CRITICAL for method chaining like .use(...).use(...)
                    // where the receiver is a temp from a previous call
                    match &recv {
                        MirOperand::Temp(name) => builder.get_temp_type(*name),
                        MirOperand::Local(name) => builder.get_local_type(&resolve(*name)),
                        _ => None,
                    }
                })
                .unwrap_or(builtin::ANY);

            // Get the receiver's type name for FFI method lookup
            // For static calls like Server::new, use the receiver name directly
            // For instance calls like app.get, we need to get the struct type name
            // IMPORTANT: This is computed AFTER receiver is built so we can use temp types
            let receiver_type_name: Option<String> = if is_module_receiver {
                if let HirExprKind::Local { name } = &receiver.kind {
                    Some(name.clone())
                } else {
                    None
                }
            } else {
                // Get type name from receiver_type (already computed with all fallbacks)
                builder.type_registry.get(receiver_type).and_then(|info| {
                    if let TypeKind::Struct { name, .. } = &info.kind {
                        Some(name.clone())
                    } else {
                        None
                    }
                })
            };

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
                                MirOperand::Temp(name) => builder.get_temp_type(*name),
                                MirOperand::Local(name) => builder.get_local_type(&resolve(*name)),
                                _ => None,
                            }
                        })
                        .unwrap_or(builtin::ANY)
                })
                .collect();
            let dest = builder.new_temp();

            // If the HIR expression has a type_id, set it on the temp
            // This ensures type inference from HIR is preserved
            let return_type = expr.type_id.or_else(|| {
                // For lambda methods (map, filter, reduce), get closure type from first arg
                // This enables proper [U] return type inference from closure signature
                let closure_type = args.first().and_then(|a| a.type_id).or_else(|| {
                    arg_types.first().copied()
                });
                
                // Try builtin method return type with closure info
                // This is the SINGLE SOURCE OF TRUTH from doo_core::methods
                if let Some(rt) = builder.get_builtin_method_return_type_with_closure(
                    receiver_type, method, closure_type
                ) {
                    return Some(rt);
                }
                
                // Fallback: Look up the method's return type from the function registry
                // Method functions are named _method_{TypeName}_{method}
                // CRITICAL: Use receiver_type_name for static calls (e.g., Server.new())
                // where receiver_type may be ANY but we know the type name from the receiver
                let type_name = receiver_type_name.as_ref().cloned().or_else(|| {
                    builder.type_registry.get(receiver_type).and_then(|info| {
                        if let TypeKind::Struct { name, .. } = &info.kind {
                            Some(name.clone())
                        } else {
                            None
                        }
                    })
                });
                if let Some(name) = type_name {
                    let mangled_name = format!("_method_{}_{}", name, method);
                    return builder.get_function_return_type(&mangled_name);
                }
                None
            });
            if let Some(rt) = return_type {
                builder.set_temp_type(dest, rt);
            }

            // Check if this method is an FFI function
            // Method functions are named _method_{TypeName}_{method}
            let mangled_method_name = receiver_type_name.as_ref()
                .map(|type_name| format!("_method_{}_{}", type_name, method));
            
            let ffi_info = mangled_method_name.as_ref()
                .and_then(|name| builder.get_ffi_info(name).cloned());

            if let Some(ffi) = ffi_info {
                // FFI method call - determine if receiver should be passed as argument
                // Static calls (is_module_receiver=true): receiver is a type/module name, NOT passed
                //   e.g. Database::get(), Server::new(":3000") - receiver is just for method lookup
                // Instance calls (is_module_receiver=false): receiver IS the object, passed as first arg
                //   e.g. app.get("/path", handler) - receiver is the actual server instance
                let mut ffi_args = if is_module_receiver {
                    // Static call: don't include module/type name as argument
                    arg_ops
                } else {
                    // Instance call: include receiver as first argument
                    let mut args = vec![recv.clone()];
                    args.extend(arg_ops);
                    args
                };

                // Pad missing optional parameters with Nil (null pointer)
                // This handles calls like app.cors() where options: {Str: Str}? is omitted
                if let Some(mangled) = &mangled_method_name {
                    if let Some(param_types) = builder.get_function_param_types(mangled) {
                        let expected = param_types.len();
                        while ffi_args.len() < expected {
                            ffi_args.push(MirOperand::Const(MirConst::Nil));
                        }
                    }
                }
                
                builder.emit(
                    MirInstrKind::FfiCall {
                        dest: Some(dest),
                        lib: sym(&ffi.library),
                        symbol: sym(&ffi.symbol),
                        args: ffi_args,
                    },
                    span,
                );
            } else {
                builder.emit(
                    MirInstrKind::MethodCall {
                        dest: Some(dest),
                        receiver: recv.clone(),
                        receiver_type,
                        method: sym(method),
                        args: arg_ops,
                        arg_types,
                        return_type,
                    },
                    span,
                );
            }

            // For mutating methods called on field receivers (e.g., self.Tasks.push(item)),
            // we need to write back the result to the original field since push/pop/etc
            // may reallocate the array and return a new pointer.
            // This is the SINGLE SOURCE OF TRUTH for field write-back on mutating operations.
            if matches!(method.as_str(), "push" | "pop" | "clear" | "reverse" | "sort") {
                if let HirExprKind::Field { object, field } = &receiver.kind {
                    // The method result (in dest) needs to be written back to object.field
                    // CRITICAL: Use the original object reference directly, NOT build_expr(object)!
                    // build_expr may emit Clone instructions which would create a copy.
                    // FieldSet must target the ORIGINAL struct to update its field correctly.
                    let obj_operand = match &object.kind {
                        HirExprKind::Local { name } => MirOperand::Local(sym(name)),
                        _ => builder.build_expr(object),
                    };
                    builder.emit(
                        MirInstrKind::FieldSet {
                            object: obj_operand,
                            field: sym(field),
                            value: MirOperand::Temp(dest),
                        },
                        span,
                    );
                }
            }

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
                    builder.set_temp_type(dest, field_type);
                }
            }

            builder.emit(
                MirInstrKind::FieldGet {
                    dest,
                    object: obj,
                    field: sym(field),
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
                            dest: end_temp,
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
                        dest,
                        array: obj,
                        start: start_val,
                        end: final_end,
                        elem_type,
                    },
                    span,
                );

                // Record the temp type as the same array type (slice produces same type)
                if let Some(arr_type) = container_type {
                    builder.set_temp_type(dest, arr_type);
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
                            dest,
                            map: obj,
                            key: idx,
                            key_type,
                            val_type,
                        },
                        span,
                    );
                    // Record value type for the dest temp
                    builder.set_temp_type(dest, val_type);
                }
                Some(ContainerKind::Array) | None => {
                    let elem_type = container_type
                        .and_then(|t| builder.array_elem_type_from_type_id(t))
                        .unwrap_or(builtin::ANY);
                    builder.emit(
                        MirInstrKind::ArrayGet {
                            dest,
                            array: obj,
                            index: idx,
                            elem_type,
                        },
                        span,
                    );
                    // Record element type for the dest temp
                    builder.set_temp_type(dest, elem_type);
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
                        dest,
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
                                array: MirOperand::Temp(dest),
                                other: val,
                                elem_type,
                            },
                            span,
                        );
                    } else {
                        let val = builder.build_expr(e);
                        builder.emit(
                            MirInstrKind::ArrayPush {
                                array: MirOperand::Temp(dest),
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
                        dest,
                        elements: elems,
                        elem_type,
                    },
                    span,
                );
            }
            // Propagate array type to temp for type inference in later operations
            if let Some(array_type) = expr.type_id {
                builder.set_temp_type(dest, array_type);
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
                    dest,
                    entries: ents,
                    key_type,
                    val_type,
                },
                span,
            );
            // Propagate map type to temp for type inference in later operations
            if let Some(map_type) = expr.type_id {
                builder.set_temp_type(dest, map_type);
            }
            MirOperand::Temp(dest)
        }

        HirExprKind::Tuple(elements) => {
            let elems: Vec<_> = elements.iter().map(|e| builder.build_expr(e)).collect();
            let dest = builder.new_temp();
            builder.emit(
                MirInstrKind::TupleCreate {
                    dest,
                    elements: elems,
                },
                span,
            );
            MirOperand::Temp(dest)
        }

        HirExprKind::Struct { name, fields } => {
            let field_ops: Vec<_> = fields
                .iter()
                .map(|(n, v)| (sym(n), builder.build_expr(v)))
                .collect();
            let dest = builder.new_temp();
            builder.emit(
                MirInstrKind::StructCreate {
                    dest,
                    struct_name: sym(name),
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
            // Check if this is actually an associated function call (e.g., Server::new(...))
            // rather than a true enum variant creation
            let mangled_name = format!("_method_{}_{}", enum_name, variant);
            if let Some(ffi_info) = builder.get_ffi_info(&mangled_name).cloned() {
                // This is an FFI associated function call, not enum creation
                let dest = builder.new_temp();
                let args: Vec<MirOperand> = payload.iter().map(|e| builder.build_expr(e)).collect();
                
                builder.emit(
                    MirInstrKind::FfiCall {
                        dest: Some(dest),
                        lib: sym(&ffi_info.library),
                        symbol: sym(&ffi_info.symbol),
                        args,
                    },
                    span,
                );
                
                // Set the return type of the temp based on the associated type
                if let Some(type_id) = builder.type_registry.lookup(enum_name) {
                    builder.set_temp_type(dest, type_id);
                }
                
                return MirOperand::Temp(dest);
            }
            
            // Regular enum variant creation
            let payload_op = if payload.is_empty() {
                None
            } else if payload.len() == 1 {
                Some(builder.build_expr(&payload[0]))
            } else {
                let ops: Vec<_> = payload.iter().map(|e| builder.build_expr(e)).collect();
                let tuple_dest = builder.new_temp();
                builder.emit(
                    MirInstrKind::TupleCreate {
                        dest: tuple_dest,
                        elements: ops,
                    },
                    span,
                );
                Some(MirOperand::Temp(tuple_dest))
            };

            let dest = builder.new_temp();
            builder.emit(
                MirInstrKind::EnumCreate {
                    dest,
                    enum_name: sym(enum_name),
                    variant: sym(variant),
                    payload: payload_op,
                },
                span,
            );
            
            // Register the enum type for the temp so that type inference works correctly
            // This ensures variables assigned from enum creation get the right type
            if let Some(enum_type_id) = builder.type_registry.lookup(enum_name) {
                builder.set_temp_type(dest, enum_type_id);
            }
            
            MirOperand::Temp(dest)
        }

        HirExprKind::Match { values, arms } => {
            let scrutinees: Vec<_> = values.iter().map(|v| builder.build_expr(v)).collect();

            let dest = builder.new_temp();
            let merge_label = builder.new_block_label("match_merge");

            let mut next_label: Option<Sym> = None;
            for (idx, arm) in arms.iter().enumerate() {
                let is_last = idx + 1 == arms.len();
                let arm_label = builder.new_block_label("match_arm");

                if idx == 0 {
                    // current block continues
                } else if let Some(label) = next_label.take() {
                    builder.add_block(label);
                }

                if !is_last {
                    let next = builder.new_block_label("match_next");
                    let cond = builder.build_match_condition(&scrutinees, &arm.pattern, span);
                    builder.set_terminator(MirTerminator::Branch {
                        cond,
                        then_block: arm_label,
                        else_block: next,
                    });
                    next_label = Some(next);
                } else {
                    builder.set_terminator(MirTerminator::Goto {
                        target: arm_label,
                    });
                }

                builder.add_block(arm_label);
                
                // Extract payload bindings INSIDE the arm block (after we know the pattern matched)
                // This ensures SSA values are defined in the correct basic block
                if let HirMatchPattern::EnumVariantPayload { enum_name, variant, bindings } = &arm.pattern {
                    if !scrutinees.is_empty() {
                        // Look up the actual payload type BEFORE borrowing current_func mutably
                        let payload_type = builder.get_enum_variant_payload_type(enum_name, variant)
                            .unwrap_or(builtin::ANY);
                        
                        // For tuple payloads, get the element types to assign correct types to bindings
                        // CRITICAL: Each binding should get the type of its corresponding tuple element,
                        // not the whole tuple type.
                        let element_types = builder.get_tuple_element_types(payload_type);
                        
                        for (i, binding) in bindings.iter().enumerate() {
                            if binding != "_" {
                                // Get the correct element type for this binding
                                // If payload is a tuple with multiple elements, use the element at index i
                                // Otherwise (single element), use the payload_type itself
                                let binding_type = if let Some(ref elem_types) = element_types {
                                    elem_types.get(i).copied().unwrap_or(payload_type)
                                } else {
                                    // Not a tuple (single-element payload) - use payload_type
                                    payload_type
                                };
                                
                                // Check if this binding name already exists with a DIFFERENT type
                                // If so, we need to use a unique internal name to avoid type conflicts
                                let (actual_dest, need_new_local) = if let Some(f) = &builder.current_func {
                                    if let Some(existing) = f.locals.iter().find(|l| l.name == sym(binding)) {
                                        if existing.type_id != binding_type {
                                            // Type conflict - use a unique temp name internally
                                            // but store the value so the original binding still works
                                            (builder.new_temp(), true)
                                        } else {
                                            // Same type - use the existing local
                                            (sym(binding), false)
                                        }
                                    } else {
                                        // New binding - create it
                                        (sym(binding), true)
                                    }
                                } else {
                                    (sym(binding), true)
                                };
                                
                                builder.emit(
                                    MirInstrKind::EnumGetPayload {
                                        dest: actual_dest,
                                        value: scrutinees[0].clone(),
                                        variant_name: sym(variant),
                                        enum_name: sym(enum_name),
                                        index: i as u32,
                                    },
                                    span,
                                );
                                
                                // Register as local with the correct element type (not the tuple type)
                                if need_new_local {
                                    if let Some(f) = &mut builder.current_func {
                                        // Only add if not already present
                                        if !f.locals.iter().any(|l| l.name == actual_dest) {
                                            f.locals.push(LocalDef {
                                                name: actual_dest,
                                                type_id: binding_type,
                                                mutable: false,
                                            });
                                        }
                                    }
                                }
                                
                                // If we used a temp, alias it to the original binding name
                                // so that code using the binding name can find it.
                                // CRITICAL: Also register the binding's type as a temp type
                                // so that infer_operand_type finds the correct (shadowed) type
                                // instead of the original local's type.
                                if actual_dest != sym(binding) {
                                    // Register the correct type for the binding name
                                    // This shadows the original local's type for this scope
                                    builder.set_temp_type(sym(binding), binding_type);
                                    
                                    builder.emit(
                                        MirInstrKind::Assign {
                                            dest: sym(binding),
                                            value: MirOperand::Temp(actual_dest),
                                        },
                                        span,
                                    );
                                }
                            }
                        }
                    }
                }
                
                let body_val = builder.build_expr(&arm.body);
                // Infer type from first arm for the match result dest
                if idx == 0 {
                    let mut arm_type = builder.infer_operand_type(&body_val);
                    // If type is still unknown but the HIR body has a type, use that
                    if arm_type == builtin::ANY {
                        if let Some(tid) = arm.body.type_id {
                            arm_type = tid;
                        }
                    }
                    // Fallback: use function return type (common for match-as-return-value)
                    if arm_type == builtin::ANY {
                        if let Some(f) = &builder.current_func {
                            if let Some(ret_type) = f.return_type {
                                arm_type = ret_type;
                            }
                        }
                    }
                    builder.set_temp_type(dest, arm_type);
                    builder.add_temp_local(dest, arm_type);
                }
                builder.emit(
                    MirInstrKind::Assign {
                        dest,
                        value: body_val,
                    },
                    span,
                );
                builder.set_terminator(MirTerminator::Goto {
                    target: merge_label,
                });
            }

            // If no arm set the type (e.g., all arms returned ANY), register as ANY
            if builder.get_temp_type(dest).is_none() {
                builder.set_temp_type(dest, builtin::ANY);
                builder.add_temp_local(dest, builtin::ANY);
            }

            builder.add_block(merge_label);
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
                then_block: then_label,
                else_block: else_label,
            });

            // Then
            builder.add_block(then_label);
            let then_val = builder.build_expr(then_expr);
            // Infer type from then branch
            let then_type = builder.infer_operand_type(&then_val);
            builder.emit(
                MirInstrKind::Assign {
                    dest,
                    value: then_val,
                },
                span,
            );
            builder.set_terminator(MirTerminator::Goto {
                target: merge_label,
            });

            // Else
            builder.add_block(else_label);
            let else_type = if let Some(else_e) = else_expr {
                let else_val = builder.build_expr(else_e);
                let else_type = builder.infer_operand_type(&else_val);
                builder.emit(
                    MirInstrKind::Assign {
                        dest,
                        value: else_val,
                    },
                    span,
                );
                else_type
            } else {
                builder.emit(
                    MirInstrKind::Assign {
                        dest,
                        value: MirOperand::Const(MirConst::Nil),
                    },
                    span,
                );
                builtin::ANY
            };
            builder.set_terminator(MirTerminator::Goto {
                target: merge_label,
            });

            // Set temp type for dest - prefer then_type if concrete, else use else_type
            let result_type = if then_type != builtin::ANY {
                then_type
            } else {
                else_type
            };
            builder.set_temp_type(dest, result_type);
            
            // Also add to locals so codegen can find the type
            builder.add_temp_local(dest, result_type);

            builder.add_block(merge_label);
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
                    dest,
                    struct_name: sym("Range"),
                    fields: vec![
                        (sym("start"), s),
                        (sym("end"), e),
                        (
                            sym("inclusive"),
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
            // Only wrap in Result if function has an error type
            // Otherwise, Ok is just syntactic sugar for returning the value
            if builder.get_current_function_error_type().is_some() {
                let dest = builder.new_temp();
                builder.emit(
                    MirInstrKind::WrapOk {
                        dest,
                        value: val,
                    },
                    span,
                );
                MirOperand::Temp(dest)
            } else {
                // No error type - just pass through the value
                val
            }
        }

        HirExprKind::Err(inner) => {
            let val = builder.build_expr(inner);
            // Err is only valid in functions with error type
            // but we still emit it (semantic analysis should catch invalid usage)
            let dest = builder.new_temp();
            builder.emit(
                MirInstrKind::WrapErr {
                    dest,
                    value: val,
                },
                span,
            );
            MirOperand::Temp(dest)
        }

        HirExprKind::Try(inner) => {
            let val = builder.build_expr(inner);
            
            // Check if this is actually a Result type
            // If not, just return the value as-is (no unwrapping needed)
            let value_type = inner.type_id.or_else(|| {
                if let MirOperand::Temp(ref temp_name) = val {
                    builder.get_temp_type(*temp_name)
                } else {
                    None
                }
            });
            
            // Check if it's a Result type by looking for the function in function_result_types
            // Handle both Call and MethodCall expressions
            // Check both Local and Global - namespace-qualified calls (like File::Read)
            // are lowered to Call with Global { name } func
            let is_result_type = match &inner.kind {
                HirExprKind::Call { func, .. } => {
                    let func_name = match &func.kind {
                        HirExprKind::Local { name } => Some(name.as_str()),
                        HirExprKind::Global { name } => Some(name.as_str()),
                        _ => None,
                    };
                    if let Some(name) = func_name {
                        let resolved_name = builder.resolve_function_name(name);
                        let found = builder.function_result_types.contains_key(&resolved_name);
                        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                            doo_debug!("MIR", "Try: Call '{}' resolved to '{}', is_result={}", name, resolved_name, found);
                        }
                        found
                    } else {
                        false
                    }
                }
                HirExprKind::MethodCall { receiver, method, .. } => {
                    // For method calls like Database::postgres(), we need to check 
                    // _method_{ReceiverType}_{method} in function_result_types
                    // First, try to get the receiver type name
                    let receiver_type_name = if let HirExprKind::Local { name } = &receiver.kind {
                        // Check if this is a type name (static call) or a variable (instance call)
                        // Type names start with uppercase, variables start with lowercase
                        if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                            // Static method call: Database::postgres() - receiver is the type name
                            Some(name.clone())
                        } else {
                            // Variable name - need to look up its type
                            // First try the temp type registry for variables
                            builder.get_temp_type(sym(name))
                                .and_then(|tid| builder.type_registry.get(tid))
                                .and_then(|info| {
                                    if let TypeKind::Struct { name: type_name, .. } = &info.kind {
                                        Some(type_name.clone())
                                    } else {
                                        None
                                    }
                                })
                                .or_else(|| {
                                    // Fallback: try to get from receiver's type_id
                                    receiver.type_id
                                        .and_then(|tid| builder.type_registry.get(tid))
                                        .and_then(|info| {
                                            if let TypeKind::Struct { name: type_name, .. } = &info.kind {
                                                Some(type_name.clone())
                                            } else {
                                                None
                                            }
                                        })
                                })
                        }
                    } else {
                        // Instance method call: db.raw() - get type from receiver
                        receiver.type_id
                            .and_then(|tid| builder.type_registry.get(tid))
                            .and_then(|info| {
                                if let TypeKind::Struct { name, .. } = &info.kind {
                                    Some(name.clone())
                                } else {
                                    None
                                }
                            })
                    };
                    
                    if let Some(ref type_name) = receiver_type_name {
                        let mangled_name = format!("_method_{}_{}", type_name, method);
                        let found = builder.function_result_types.contains_key(&mangled_name);
                        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                            doo_debug!("MIR", "Try: MethodCall '{}.{}' -> '{}', is_result={}", type_name, method, mangled_name, found);
                        }
                        found
                    } else {
                        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                            doo_debug!("MIR", "Try: MethodCall method='{}' - no receiver type found", method);
                        }
                        false
                    }
                }
                _ => {
                    if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                        doo_debug!("MIR", "Try: Unknown inner expr kind {:?}", std::mem::discriminant(&inner.kind));
                    }
                    false
                },
            };
            
            if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                doo_debug!("MIR", "Try: is_result_type={}", is_result_type);
            }
            
            // If not a Result type, just return the value directly
            if !is_result_type {
                return val;
            }
            
            let dest = builder.new_temp();
            let is_ok_dest = builder.new_temp();

            // Get the expected type for the unwrapped value
            let expected_type = value_type;
            
            // Track the unwrapped type for downstream code
            if let Some(type_id) = expected_type {
                builder.set_temp_type(dest, type_id);
            }

            // Check if Ok
            builder.emit(
                MirInstrKind::IsOk {
                    dest: is_ok_dest,
                    value: val.clone(),
                },
                span,
            );

            // Create labels for branching
            let ok_label = builder.new_block_label("try_ok");
            let err_label = builder.new_block_label("try_err");
            let cont_label = builder.new_block_label("try_cont");

            // Branch based on is_ok
            builder.set_terminator(MirTerminator::Branch {
                cond: MirOperand::Temp(is_ok_dest),
                then_block: ok_label,
                else_block: err_label,
            });

            // Ok path: unwrap and continue
            builder.add_block(ok_label);
            builder.emit(
                MirInstrKind::UnwrapOk {
                    dest,
                    value: val.clone(),
                    expected_type,
                },
                span,
            );
            builder.set_terminator(MirTerminator::Goto { target: cont_label });

            // Err path: propagate error (return the Result as-is)
            // For functions with error types, this should return early
            // For main or functions without error types, this becomes a panic
            builder.add_block(err_label);
            
            if builder.get_current_function_error_type().is_some() {
                // Function has an error type - propagate the error
                let err_dest = builder.new_temp();
                builder.emit(
                    MirInstrKind::UnwrapErr {
                        dest: err_dest,
                        value: val,
                    },
                    span,
                );
                // Wrap the error and return it (propagate)
                let wrapped_err = builder.new_temp();
                builder.emit(
                    MirInstrKind::WrapErr {
                        dest: wrapped_err,
                        value: MirOperand::Temp(err_dest),
                    },
                    span,
                );
                builder.set_terminator(MirTerminator::Return {
                    values: vec![MirOperand::Temp(wrapped_err)],
                });
            } else {
                // Function has no error type - panic on error
                // Extract the error message and panic
                let err_dest = builder.new_temp();
                builder.emit(
                    MirInstrKind::UnwrapErr {
                        dest: err_dest,
                        value: val,
                    },
                    span,
                );
                builder.emit(
                    MirInstrKind::Panic {
                        message: MirOperand::Temp(err_dest),
                    },
                    span,
                );
                // Panic doesn't return, but we need a terminator for LLVM
                builder.set_terminator(MirTerminator::Unreachable);
            }

            // Continue block for ok path
            builder.add_block(cont_label);

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
                    dest: is_ok_dest,
                    value: result_val.clone(),
                },
                span,
            );

            let ok_label = builder.new_block_label("unwrap_ok");
            let err_label = builder.new_block_label("unwrap_err");
            let merge_label = builder.new_block_label("unwrap_merge");

            builder.set_terminator(MirTerminator::Branch {
                cond: MirOperand::Temp(is_ok_dest),
                then_block: ok_label,
                else_block: err_label,
            });

            let dest = builder.new_temp();

            // Extract the Ok type from the Result type
            // inner.type_id is the Result<T, E> type, we need T
            let ok_type = inner.type_id.and_then(|result_type_id| {
                builder.type_registry.get(result_type_id).and_then(|info| {
                    if let TypeKind::Result { ok, .. } = &info.kind {
                        Some(*ok)
                    } else {
                        // Not a Result type - might be from a function call
                        // Try to infer from function_result_types
                        None
                    }
                })
            }).or_else(|| {
                // Fallback: try to get from function call if inner is a Call
                // Resolve aliases to handle imported associated functions
                if let HirExprKind::Call { func, .. } = &inner.kind {
                    if let HirExprKind::Local { name } | HirExprKind::Global { name } = &func.kind {
                        let resolved_name = builder.resolve_function_name(name);
                        builder.function_result_types.get(&resolved_name).map(|(ok, _)| *ok)
                    } else {
                        None
                    }
                } else {
                    None
                }
            });

            // Ok path: unwrap and continue
            builder.add_block(ok_label);
            builder.emit(
                MirInstrKind::UnwrapOk {
                    dest,
                    value: result_val.clone(),
                    expected_type: ok_type,
                },
                span,
            );
            
            // Set the temp type for proper type inference
            if let Some(type_id) = ok_type {
                builder.set_temp_type(dest, type_id);
            }
            
            builder.set_terminator(MirTerminator::Goto {
                target: merge_label,
            });

            // Err path: print panic message and abort
            builder.add_block(err_label);
            let msg_val = builder.build_expr(message);
            builder.emit(
                MirInstrKind::Panic { message: msg_val },
                span,
            );
            builder.set_terminator(MirTerminator::Unreachable);

            // Merge
            builder.add_block(merge_label);

            MirOperand::Temp(dest)
        }

        HirExprKind::Clone(inner) => {
            let val = builder.build_expr(inner);
            let dest = builder.new_temp();
            
            // Infer the type of the cloned value
            let inner_type = builder.infer_operand_type(&val);
            
            builder.emit(
                MirInstrKind::Clone {
                    dest,
                    src: val,
                },
                span,
            );
            
            // Set the temp type for proper type tracking
            builder.set_temp_type(dest, inner_type);
            
            MirOperand::Temp(dest)
        }

        HirExprKind::Move(inner) | HirExprKind::Borrow { expr: inner, .. } => {
            builder.build_expr(inner)
        }

        HirExprKind::Closure { params, body } => {
            // Generate a unique name for the closure function
            let closure_name = format!("__closure_{}", builder.closure_counter);
            builder.closure_counter += 1;

            // Store the closure for later processing (don't build body inline!)
            // The body will be built as a separate MIR function
            builder
                .pending_closures
                .push((closure_name.clone(), params.clone(), body.clone(), Vec::new()));

            // Emit ClosureCreate instruction that references the closure function
            let dest = builder.new_temp();
            let span = builder.convert_span(expr.span);
            
            // Set the closure's function type on the temp if HIR provided it
            // This enables proper type inference for lambda methods like map/filter/reduce
            if let Some(func_type) = expr.type_id {
                builder.set_temp_type(dest, func_type);
            }
            // If HIR didn't provide type, the body type might still be set
            // Store closure info for later type lookup
            let return_type = body.type_id.unwrap_or(builtin::ANY);
            builder.closure_return_types.insert(closure_name.clone(), return_type);
            
            builder.emit(
                MirInstrKind::ClosureCreate {
                    dest,
                    func: sym(&closure_name),
                    captures: Vec::new(), // TODO: capture analysis
                },
                span,
            );
            MirOperand::Temp(dest)
        }

        HirExprKind::Spread(inner) => {
            // Spread is just passing through the inner expression
            builder.build_expr(inner)
        }

        HirExprKind::RouteBlock { routes } => {
            // Route block is a collection of route expressions.
            // Build each route (side effects for registering routes),
            // and return an array of their results.
            let elements: Vec<MirOperand> = routes
                .iter()
                .map(|r| builder.build_expr(r))
                .collect();
            let dest = builder.new_temp();
            let span = builder.convert_span(expr.span);
            builder.emit(
                MirInstrKind::ArrayCreate {
                    dest,
                    elements,
                    elem_type: doo_core::types::builtin::ANY,
                },
                span,
            );
            MirOperand::Temp(dest)
        }

        HirExprKind::Cast { value, to_type } => {
            // Build the value and emit a cast instruction
            let val = builder.build_expr(value);
            let dest = builder.new_temp();
            let span = builder.convert_span(expr.span);
            builder.emit(
                MirInstrKind::Cast {
                    dest,
                    value: val,
                    to_type: *to_type,
                },
                span,
            );
            MirOperand::Temp(dest)
        }

        // === Async & Concurrency ===
        HirExprKind::Await(inner) => {
            // In Doo's compiled model, async fns run synchronously.
            // `await someFunc()` just calls the function — no TaskHandle to join.
            // Only emit MirInstrKind::Await when awaiting a variable (task handle from `go {}`).
            match &inner.kind {
                HirExprKind::Call { .. } | HirExprKind::MethodCall { .. } => {
                    // Function call — runs synchronously, await is pass-through
                    builder.build_expr(inner)
                }
                _ => {
                    // Variable or other expression — likely a TaskHandle, emit real await
                    let handle = builder.build_expr(inner);
                    let dest = builder.new_temp();
                    let span = builder.convert_span(expr.span);
                    builder.emit(
                        MirInstrKind::Await {
                            dest,
                            handle,
                        },
                        span,
                    );
                    MirOperand::Temp(dest)
                }
            }
        }

        HirExprKind::Spawn { body } => {
            // Spawn wraps body into a closure-like function,
            // then emits a Spawn or ScopeSpawn instruction.
            let closure_name = format!("__spawn_{}", builder.closure_counter);
            builder.closure_counter += 1;

            // Collect free variables from the body — these must be captured
            let captures = super::capture::collect_free_vars(body, builder);

            builder
                .pending_closures
                .push((closure_name.clone(), Vec::new(), body.clone(), captures.clone()));
            let span = builder.convert_span(expr.span);

            // Build capture operands from the outer scope
            let capture_operands: Vec<MirOperand> = captures
                .iter()
                .map(|name| MirOperand::Local(sym(name)))
                .collect();

            // If we're inside a scope, emit ScopeSpawn (tracked by scope_stack)
            if let Some(scope_var) = builder.scope_stack.last().cloned() {
                builder.emit(
                    MirInstrKind::ScopeSpawn {
                        scope: MirOperand::Temp(scope_var),
                        func: sym(&closure_name),
                        captures: capture_operands,
                    },
                    span,
                );
                // ScopeSpawn is void — return a dummy empty result
                let dest = builder.new_temp();
                builder.emit(
                    MirInstrKind::Assign {
                        dest,
                        value: MirOperand::Const(crate::types::MirConst::Int(0)),
                    },
                    span,
                );
                MirOperand::Temp(dest)
            } else {
                // Regular spawn — returns a TaskHandle
                let dest = builder.new_temp();
                builder.emit(
                    MirInstrKind::Spawn {
                        dest,
                        func: sym(&closure_name),
                        captures: capture_operands,
                    },
                    span,
                );
                MirOperand::Temp(dest)
            }
        }

        HirExprKind::ScopeBlock { stmts } => {
            let scope_dest = builder.new_temp();
            let span = builder.convert_span(expr.span);
            builder.emit(
                MirInstrKind::ScopeCreate {
                    dest: scope_dest,
                },
                span,
            );
            // Push scope onto stack so inner `go { }` emits ScopeSpawn
            builder.scope_stack.push(scope_dest);
            // Build all statements inside the scope
            for s in stmts {
                builder.build_stmt(s);
            }
            // Pop scope stack
            builder.scope_stack.pop();
            // Wait for all scope tasks
            let result_dest = builder.new_temp();
            builder.emit(
                MirInstrKind::ScopeWait {
                    dest: result_dest,
                    scope: MirOperand::Temp(scope_dest),
                },
                span,
            );
            MirOperand::Temp(result_dest)
        }
    }
}
