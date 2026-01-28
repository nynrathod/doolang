//! Call Instruction Handler
//!
//! Handles: Call, MethodCall, FfiCall, Print

use super::InstructionHandler;
use crate::builtins::{ArrayBuiltins, JsonBuiltins, MapBuiltins, StringBuiltins};
use crate::context::CodegenContext;
use crate::layout::load_len_i32;
use doo_core::constants::ffi_names;
use doo_core::types::builtin;
use doo_core::types::TypeKind;
use doo_mir::{MirConst, MirInstr, MirInstrKind, MirOperand};
use inkwell::module::Linkage;
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

/// Call/invocation instruction handler.
pub struct CallHandler;

impl<'ctx> InstructionHandler<'ctx> for CallHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(
            instr.kind,
            MirInstrKind::Call { .. }
                | MirInstrKind::MethodCall { .. }
                | MirInstrKind::FfiCall { .. }
                | MirInstrKind::Print { .. }
                | MirInstrKind::TypeOf { .. }
                | MirInstrKind::WrapOk { .. }
                | MirInstrKind::WrapErr { .. }
                | MirInstrKind::IsOk { .. }
                | MirInstrKind::UnwrapOk { .. }
                | MirInstrKind::UnwrapErr { .. }
                | MirInstrKind::ManualErrorExtract { .. }
        )
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::Call { dest, func, args } => {
                let func_val = ctx.get_function(func)?;
                
                // Coerce arguments to match function parameter types
                // This handles cases like enum StructValues that need to be boxed to pointers
                let param_types = func_val.get_type().get_param_types();
                let arg_vals: Vec<_> = args
                    .iter()
                    .enumerate()
                    .filter_map(|(i, a)| {
                        let val = operand_to_value(ctx, a)?;
                        // Get expected parameter type from function signature
                        // Convert BasicMetadataTypeEnum to BasicTypeEnum if possible
                        let param_type: Option<BasicTypeEnum> = param_types.get(i).and_then(|t| {
                            match t {
                                inkwell::types::BasicMetadataTypeEnum::ArrayType(t) => Some((*t).into()),
                                inkwell::types::BasicMetadataTypeEnum::FloatType(t) => Some((*t).into()),
                                inkwell::types::BasicMetadataTypeEnum::IntType(t) => Some((*t).into()),
                                inkwell::types::BasicMetadataTypeEnum::PointerType(t) => Some((*t).into()),
                                inkwell::types::BasicMetadataTypeEnum::StructType(t) => Some((*t).into()),
                                inkwell::types::BasicMetadataTypeEnum::VectorType(t) => Some((*t).into()),
                                inkwell::types::BasicMetadataTypeEnum::ScalableVectorType(t) => Some((*t).into()),
                                inkwell::types::BasicMetadataTypeEnum::MetadataType(_) => None,
                            }
                        });
                        Some(coerce_arg_to_param_type(ctx, val, param_type))
                    })
                    .collect();

                let call_site = ctx.builder.build_call(func_val, &arg_vals, "call").ok()?;

                if let Some(dest_name) = dest {
                    if let Some(ret_val) = call_site.try_as_basic_value().left() {
                        ctx.set_temp(dest_name, ret_val);
                        return Some(ret_val);
                    }
                }
                None
            }

            MirInstrKind::MethodCall {
                dest,
                receiver,
                receiver_type,
                method,
                args,
                arg_types,
                return_type,
            } => {
                if std::env::var("DOO_DEBUG").is_ok() {
                    eprintln!(
                        "[CODEGEN] MethodCall: {:?}.{} -> {:?}, return_type={:?}",
                        receiver, method, dest, return_type
                    );
                }
                // Intercept JSON.stringify and JSON.parse (Static Specialization)
                // Check for both Local("JSON") and Global("JSON") for module calls
                let is_json_module = matches!(receiver, 
                    MirOperand::Local(name) | MirOperand::Global(name) if name == ffi_names::MODULE_JSON);
                
                if std::env::var("DOO_DEBUG").is_ok() && method == "parse" {
                    eprintln!("[CODEGEN] JSON.parse check: is_json_module={}, receiver={:?}", is_json_module, receiver);
                }
                    
                if is_json_module {
                    if method == "stringify" {
                        if let (Some(arg_op), Some(&arg_type)) =
                            (args.first(), arg_types.first())
                        {
                            if let Some(val) = operand_to_value(ctx, arg_op) {
                                // Dispatch to JSON codegen
                                let result = JsonBuiltins::emit_stringify(ctx, val, arg_type);
                                if let (Some(r), Some(dst)) = (result, dest) {
                                    ctx.set_temp(dst, r);
                                }
                                return result;
                            }
                        }
                        return None;
                    } else if method == "parse" {
                        if let Some(arg_op) = args.first() {
                            if let Some(val) = operand_to_value(ctx, arg_op) {
                                // Pass return_type to emit_parse for type-specific parsing
                                let result = JsonBuiltins::emit_parse(ctx, val, *return_type);
                                if let (Some(r), Some(dst)) = (result, dest) {
                                    ctx.set_temp(dst, r);
                                }
                                return result;
                            }
                        }
                        return None;
                    }
                }

                let recv_val = operand_to_value(ctx, receiver);
                if recv_val.is_none() && std::env::var("DOO_DEBUG").is_ok() {
                    eprintln!(
                        "[CODEGEN] MethodCall: failed to get receiver value for {:?}",
                        receiver
                    );
                    return None;
                }
                let recv_val = recv_val?;

                let arg_vals: Vec<_> = args
                    .iter()
                    .filter_map(|a| operand_to_value(ctx, a))
                    .collect();

                let receiver_name = match receiver {
                    MirOperand::Local(name) | MirOperand::Temp(name) => Some(name.as_str()),
                    _ => None,
                };

                if std::env::var("DOO_DEBUG").is_ok() {
                    eprintln!("[CODEGEN] MethodCall: recv_val type: is_pointer={}, is_int={}, recv_val={:?}", 
                        recv_val.is_pointer_value(), recv_val.is_int_value(), recv_val);
                }

                // Builtin dispatch (single source of truth via TypeRegistry)
                if recv_val.is_pointer_value() {
                    let recv_ptr = recv_val.into_pointer_value();
                    if let Some(kind) = ctx.get_type_kind(*receiver_type) {
                        if std::env::var("DOO_DEBUG").is_ok() {
                            eprintln!(
                                "[CODEGEN] MethodCall: receiver type {:?} -> kind {:?}",
                                receiver_type, kind
                            );
                        }
                        let builtin_result = match kind {
                            TypeKind::Str => StringBuiltins::dispatch(
                                ctx,
                                dest.as_deref(),
                                recv_ptr,
                                method,
                                &arg_vals,
                            ),
                            TypeKind::Array { .. } => ArrayBuiltins::dispatch(
                                ctx,
                                dest.as_deref(),
                                receiver_name,
                                *receiver_type,
                                recv_ptr,
                                method,
                                &arg_vals,
                            ),
                            TypeKind::Map { .. } => MapBuiltins::dispatch(
                                ctx,
                                dest.as_deref(),
                                receiver_name,
                                *receiver_type,
                                recv_ptr,
                                method,
                                &arg_vals,
                            ),
                            // For ANY type, try array builtins for common methods
                            TypeKind::Any => {
                                if matches!(
                                    method.as_str(),
                                    "len"
                                        | "push"
                                        | "pop"
                                        | "get"
                                        | "set"
                                        | "contains"
                                        | "slice"
                                        | "map"
                                        | "filter"
                                ) {
                                    ArrayBuiltins::dispatch(
                                        ctx,
                                        dest.as_deref(),
                                        receiver_name,
                                        *receiver_type,
                                        recv_ptr,
                                        method,
                                        &arg_vals,
                                    )
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };
                        if builtin_result.is_some() {
                            return builtin_result;
                        }
                    } else {
                        // Fallback for unknown type
                        if std::env::var("DOO_DEBUG").is_ok() {
                            eprintln!("[CODEGEN] MethodCall: fallback to array dispatch for {} (receiver_type: {:?})", method, receiver_type);
                        }
                        if matches!(
                            method.as_str(),
                            "len" | "push" | "pop" | "get" | "set" | "contains" | "slice"
                        ) {
                            let result = ArrayBuiltins::dispatch(
                                ctx,
                                dest.as_deref(),
                                receiver_name,
                                *receiver_type,
                                recv_ptr,
                                method,
                                &arg_vals,
                            );
                            if std::env::var("DOO_DEBUG").is_ok() {
                                eprintln!(
                                    "[CODEGEN] MethodCall: array dispatch result: {:?}",
                                    result.is_some()
                                );
                            }
                            if result.is_some() {
                                return result;
                            }
                        }
                    }
                }

                // Fallback: lookup method function, prepend receiver to args
                // Format: _method_{TypeName}_{MethodName}
                let type_name = if let Some(kind) = ctx.get_type_kind(*receiver_type) {
                    match kind {
                        TypeKind::Struct { name, .. } => Some(name),
                        TypeKind::Enum { name, .. } => Some(name),
                        _ => None,
                    }
                } else {
                    None
                };

                if let Some(tname) = type_name {
                    let method_name = format!("_method_{}_{}", tname, method);
                    if let Some(func_val) = ctx.get_function(&method_name) {
                        let mut all_args = vec![recv_val.into()];
                        for v in &arg_vals {
                            all_args.push((*v).into());
                        }

                        // Ensure we aren't passing garbage if args mismatch (basic check)
                        let call_site =
                            ctx.builder.build_call(func_val, &all_args, "mcall").ok()?;

                        if let Some(dest_name) = dest {
                            if let Some(ret_val) = call_site.try_as_basic_value().left() {
                                ctx.set_temp(dest_name, ret_val);
                                return Some(ret_val);
                            }
                        }
                        return None; // Void return
                    }
                }
                None
            }

            MirInstrKind::FfiCall {
                dest,
                lib: _,
                symbol,
                args,
            } => {
                // FFI call: declare external function if needed and call
                // Symbol is the C function name (e.g., "doo_file_read", "doo_http_server_new")
                emit_ffi_call(ctx, dest.as_deref(), symbol, args)
            }

            MirInstrKind::Print {
                values,
                value_types,
            } => {
                // Print built-in: call printf or custom print function
                // Declare printf if not already declared
                let printf = ctx.get_function(ffi_names::PRINTF).unwrap_or_else(|| {
                    let i32_ty = ctx.context.i32_type();
                    let ptr_ty = ctx.context.ptr_type(inkwell::AddressSpace::default());
                    let fn_ty = i32_ty.fn_type(&[ptr_ty.into()], true); // variadic
                    ctx.module.add_function(ffi_names::PRINTF, fn_ty, None)
                });

                for (i, val) in values.iter().enumerate() {
                    let ty = value_types
                        .get(i)
                        .copied()
                        .unwrap_or(doo_core::types::builtin::ANY);
                    let is_last = i + 1 == values.len();
                    if let Some(v) = operand_to_value(ctx, val) {
                        if let Some(kind) = ctx.get_type_kind(ty) {
                            match kind {
                                TypeKind::Str => {
                                    emit_print_value(ctx, printf, ty, v, false, false);
                                }
                                TypeKind::Bool => {
                                    emit_print_value(ctx, printf, ty, v, false, false);
                                }
                                TypeKind::Int | TypeKind::Float => {
                                    emit_print_value(ctx, printf, ty, v, false, false);
                                }
                                TypeKind::Array { element } => {
                                    if v.is_pointer_value() {
                                        emit_print_array(
                                            ctx,
                                            printf,
                                            v.into_pointer_value(),
                                            element,
                                        );
                                    } else {
                                        emit_print_value(ctx, printf, builtin::ANY, v, false, false);
                                    }
                                }
                                TypeKind::Map { key, value } => {
                                    if v.is_pointer_value() {
                                        emit_print_map(
                                            ctx,
                                            printf,
                                            v.into_pointer_value(),
                                            key,
                                            value,
                                        );
                                    } else {
                                        emit_print_value(ctx, printf, builtin::ANY, v, false, false);
                                    }
                                }
                                TypeKind::Struct { name, fields } => {
                                    if v.is_pointer_value() {
                                        emit_print_struct(
                                            ctx,
                                            printf,
                                            v.into_pointer_value(),
                                            &name,
                                            &fields,
                                        );
                                    } else {
                                        emit_print_value(ctx, printf, ty, v, false, false);
                                    }
                                }
                                TypeKind::Enum { name, variants } => {
                                    if v.is_pointer_value() {
                                        emit_print_enum(
                                            ctx,
                                            printf,
                                            v.into_pointer_value(),
                                            &name,
                                            &variants,
                                        );
                                    } else if v.is_struct_value() {
                                        // Enum as StructValue (inline) - use direct extraction
                                        emit_print_enum_value(
                                            ctx,
                                            printf,
                                            v.into_struct_value(),
                                            &name,
                                            &variants,
                                        );
                                    } else {
                                        // Fallback for other cases
                                        emit_print_value(ctx, printf, ty, v, false, false);
                                    }
                                }
                                _ => {
                                    emit_print_value(ctx, printf, ty, v, false, false);
                                }
                            }
                        } else {
                            emit_print_value(ctx, printf, ty, v, false, false);
                        }

                        if !is_last {
                            let fmt = ctx.const_string("%s");
                            let space = ctx.const_string(" ");
                            ctx.builder
                                .build_call(printf, &[fmt.into(), space.into()], "print_space")
                                .ok();
                        }
                    }
                }

                // Single newline at the end of the print call
                let fmt = ctx.const_string("%s");
                let nl = ctx.const_string("\n");
                ctx.builder
                    .build_call(printf, &[fmt.into(), nl.into()], "print_nl")
                    .ok();

                None
            }

            MirInstrKind::WrapOk { dest, value } => {
                // Result::Ok = { i32 tag=0, ptr payload }
                // Allocate Result struct, set tag=0, box value in payload
                let val = operand_to_value(ctx, value)?;

                // Convert value to pointer representation
                let value_ptr = value_to_ptr(ctx, val)?;

                // Create Result struct type: { i32 tag, ptr payload }
                let result_struct_type = ctx
                    .context
                    .struct_type(&[ctx.i32_type().into(), ctx.ptr_type().into()], false);

                // Allocate Result struct on stack
                let result_alloca = ctx
                    .builder
                    .build_alloca(result_struct_type, "result_ok")
                    .ok()?;

                // Set tag = 0 (Ok)
                let tag_ptr = ctx
                    .builder
                    .build_struct_gep(result_struct_type, result_alloca, 0, "ok_tag_ptr")
                    .ok()?;
                ctx.builder
                    .build_store(tag_ptr, ctx.i32_type().const_int(0, false))
                    .ok()?;

                // Set payload pointer
                let payload_ptr = ctx
                    .builder
                    .build_struct_gep(result_struct_type, result_alloca, 1, "ok_payload_ptr")
                    .ok()?;
                ctx.builder.build_store(payload_ptr, value_ptr).ok()?;

                // Load and return the struct
                let result_struct = ctx
                    .builder
                    .build_load(result_struct_type, result_alloca, "result_ok_struct")
                    .ok()?;

                ctx.set_temp(dest, result_struct);
                Some(result_struct)
            }

            MirInstrKind::WrapErr { dest, value } => {
                // Result::Err = { i32 tag=1, ptr payload }
                // Allocate Result struct, set tag=1, box error in payload
                let val = operand_to_value(ctx, value)?;

                // Convert value to pointer representation
                let value_ptr = value_to_ptr(ctx, val)?;

                // Create Result struct type: { i32 tag, ptr payload }
                let result_struct_type = ctx
                    .context
                    .struct_type(&[ctx.i32_type().into(), ctx.ptr_type().into()], false);

                // Allocate Result struct on stack
                let result_alloca = ctx
                    .builder
                    .build_alloca(result_struct_type, "result_err")
                    .ok()?;

                // Set tag = 1 (Err)
                let tag_ptr = ctx
                    .builder
                    .build_struct_gep(result_struct_type, result_alloca, 0, "err_tag_ptr")
                    .ok()?;
                ctx.builder
                    .build_store(tag_ptr, ctx.i32_type().const_int(1, false))
                    .ok()?;

                // Set payload pointer
                let payload_ptr = ctx
                    .builder
                    .build_struct_gep(result_struct_type, result_alloca, 1, "err_payload_ptr")
                    .ok()?;
                ctx.builder.build_store(payload_ptr, value_ptr).ok()?;

                // Load and return the struct
                let result_struct = ctx
                    .builder
                    .build_load(result_struct_type, result_alloca, "result_err_struct")
                    .ok()?;

                ctx.set_temp(dest, result_struct);
                Some(result_struct)
            }

            MirInstrKind::IsOk { dest, value } => {
                // Check if result is Ok (tag == 0)
                let result_val = operand_to_value(ctx, value)?;

                // Try to get the Result struct (load if pointer)
                if let Some(result_struct) = load_result_struct(ctx, result_val) {
                    // Extract tag (field 0)
                    let tag = ctx
                        .builder
                        .build_extract_value(result_struct, 0, "result_tag")
                        .ok()?
                        .into_int_value();

                    // Check if tag == 0 (Ok)
                    let is_ok = ctx
                        .builder
                        .build_int_compare(
                            IntPredicate::EQ,
                            tag,
                            ctx.i32_type().const_int(0, false),
                            "is_ok",
                        )
                        .ok()?;

                    ctx.set_temp(dest, is_ok.into());
                    Some(is_ok.into())
                } else {
                    // Not a Result type - treat as always Ok
                    // This handles the case where ? is used on non-Result values
                    let is_ok = ctx.const_bool(true);
                    ctx.set_temp(dest, is_ok.into());
                    Some(is_ok.into())
                }
            }

            MirInstrKind::UnwrapOk { dest, value, expected_type } => {
                // Extract Ok value from Result
                // The branching (error check) is now handled at MIR level
                // This just extracts the value from an Ok result
                let result_val = operand_to_value(ctx, value)?;

                // Try to get the Result struct (load if pointer)
                if let Some(result_struct) = load_result_struct(ctx, result_val) {
                    // Extract value pointer (field 1)
                    let value_ptr = ctx
                        .builder
                        .build_extract_value(result_struct, 1, "ok_value_ptr")
                        .ok()?
                        .into_pointer_value();

                    // Convert the pointer back to the expected type
                    // The payload was created using value_to_ptr which uses inttoptr for primitives
                    let final_value: BasicValueEnum = match expected_type {
                        Some(type_id) if *type_id == builtin::INT => {
                            // Convert pointer back to i64 using ptrtoint
                            ctx.builder
                                .build_ptr_to_int(value_ptr, ctx.i64_type(), "ptr_to_int")
                                .ok()?
                                .into()
                        }
                        Some(type_id) if *type_id == builtin::FLOAT => {
                            // Convert pointer to float (reverse of value_to_ptr)
                            let i64_val = ctx.builder
                                .build_ptr_to_int(value_ptr, ctx.i64_type(), "ptr_to_i64")
                                .ok()?;
                            let tmp = ctx.builder.build_alloca(ctx.i64_type(), "f_tmp").ok()?;
                            ctx.builder.build_store(tmp, i64_val).ok()?;
                            let f_ptr = ctx.builder
                                .build_pointer_cast(tmp, ctx.context.ptr_type(inkwell::AddressSpace::default()), "f_ptr")
                                .ok()?;
                            ctx.builder.build_load(ctx.f64_type(), f_ptr, "f_val").ok()?
                        }
                        Some(type_id) if *type_id == builtin::BOOL => {
                            // Convert pointer back to bool
                            let i64_val = ctx.builder
                                .build_ptr_to_int(value_ptr, ctx.i64_type(), "ptr_to_i64")
                                .ok()?;
                            ctx.builder
                                .build_int_truncate(i64_val, ctx.bool_type(), "to_bool")
                                .ok()?
                                .into()
                        }
                        _ => {
                            // For pointers (string, array, struct), the pointer is the value
                            value_ptr.into()
                        }
                    };

                    ctx.set_temp(dest, final_value);
                    Some(final_value)
                } else {
                    // Not a Result type - pass through the value as-is
                    // This handles the case where ? is used on non-Result values
                    ctx.set_temp(dest, result_val);
                    Some(result_val)
                }
            }

            MirInstrKind::UnwrapErr { dest, value } => {
                // Extract Err value from Result
                // The MIR is responsible for checking IsOk before calling UnwrapErr,
                // so we don't need to check again here - just extract the payload.
                let result_val = operand_to_value(ctx, value)?;

                // Try to get the Result struct (load if pointer)
                if let Some(result_struct) = load_result_struct(ctx, result_val) {
                    // Extract payload (field 1) - this is the error value
                    let value_ptr = ctx
                        .builder
                        .build_extract_value(result_struct, 1, "err_value_ptr")
                        .ok()?
                        .into_pointer_value();

                    // The payload is already a pointer - store it as the value
                    ctx.set_temp(dest, value_ptr.into());
                    Some(value_ptr.into())
                } else {
                    // Not a Result type - return null pointer as error
                    // This shouldn't normally happen but provides fallback
                    let null_ptr = ctx.ptr_type().const_null();
                    ctx.set_temp(dest, null_ptr.into());
                    Some(null_ptr.into())
                }
            }

            MirInstrKind::ManualErrorExtract {
                ok_names,
                error_name,
                result,
                ok_type,
                err_type: _,
            } => {
                // Manual error extraction: let a, b, err = expr;
                // Result struct layout: { i32 tag, void* value }
                // tag == 0 means Ok, tag == 1 means Err

                let result_val = operand_to_value(ctx, result)?;

                // Result must be a struct value or pointer to struct
                // If it's a pointer, load the struct first
                let result_struct = if result_val.is_pointer_value() {
                    let result_ptr = result_val.into_pointer_value();
                    let ptr_type = ctx.ptr_type();
                    let result_struct_type = ctx
                        .context
                        .struct_type(&[ctx.i32_type().into(), ptr_type.into()], false);
                    ctx.builder
                        .build_load(result_struct_type, result_ptr, "result_struct_load")
                        .ok()?
                        .into_struct_value()
                } else if result_val.is_struct_value() {
                    result_val.into_struct_value()
                } else {
                    // Not a Result - just assign the value to all destinations
                    for ok_name in ok_names {
                        ctx.set_temp(ok_name, result_val);
                    }
                    if error_name != "_" {
                        // Set error to nil (null pointer)
                        let nil = ctx.ptr_type().const_null();
                        ctx.set_temp(error_name, nil.into());
                    }
                    return Some(result_val);
                };

                // Extract tag (field 0)
                let tag = ctx
                    .builder
                    .build_extract_value(result_struct, 0, "result_tag")
                    .ok()?
                    .into_int_value();

                // Check if tag == 0 (Ok)
                let is_ok = ctx
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        tag,
                        ctx.i32_type().const_int(0, false),
                        "is_ok",
                    )
                    .ok()?;

                // Extract value pointer (field 1)
                let value_ptr = ctx
                    .builder
                    .build_extract_value(result_struct, 1, "result_value_ptr")
                    .ok()?
                    .into_pointer_value();

                // Create blocks for ok and err paths
                let func = ctx.builder.get_insert_block()?.get_parent()?;
                let ok_block = ctx.context.append_basic_block(func, "manual_ok");
                let err_block = ctx.context.append_basic_block(func, "manual_err");
                let cont_block = ctx.context.append_basic_block(func, "manual_cont");

                ctx.builder
                    .build_conditional_branch(is_ok, ok_block, err_block)
                    .ok()?;

                // Determine if we should use value-based phi (when error is ignored)
                // This prevents null pointer dereference when error occurs but is ignored
                let error_ignored = error_name == "_";
                
                // Check if ok_type is a scalar (Int, Float, Bool) - these need special handling
                // because Result stores scalars as inttoptr(value) which needs ptrtoint to extract
                let is_int = *ok_type == builtin::INT;
                let is_float = *ok_type == builtin::FLOAT;
                let is_bool = *ok_type == builtin::BOOL;
                let is_scalar_ok = is_int || is_float || is_bool;

                if error_ignored && is_scalar_ok {
                    // === SCALAR VALUE PATH (error ignored) ===
                    // When error is ignored and ok type is scalar, we must:
                    // 1. Convert the pointer to the actual value in ok_block (using ptrtoint)
                    // 2. Use a default value in err_block (not null pointer)
                    // 3. Phi the VALUES, not pointers
                    
                    let ok_llvm_type = ctx.get_llvm_type(*ok_type);
                    
                    // === Ok path ===
                    ctx.builder.position_at_end(ok_block);
                    
                    // Convert pointer to value (same logic as UnwrapOk)
                    let ok_extracted_val: BasicValueEnum = if is_int {
                        // Convert pointer back to i64 using ptrtoint
                        ctx.builder
                            .build_ptr_to_int(value_ptr, ctx.i64_type(), "ptr_to_int")
                            .ok()?
                            .into()
                    } else if is_float {
                        // Convert pointer to float (reverse of value_to_ptr)
                        let i64_val = ctx.builder
                            .build_ptr_to_int(value_ptr, ctx.i64_type(), "ptr_to_i64")
                            .ok()?;
                        let tmp = ctx.builder.build_alloca(ctx.i64_type(), "f_tmp").ok()?;
                        ctx.builder.build_store(tmp, i64_val).ok()?;
                        let f_ptr = ctx.builder
                            .build_pointer_cast(tmp, ctx.context.ptr_type(inkwell::AddressSpace::default()), "f_ptr")
                            .ok()?;
                        ctx.builder.build_load(ctx.f64_type(), f_ptr, "f_val").ok()?
                    } else {
                        // Bool: Convert pointer back to bool
                        let i64_val = ctx.builder
                            .build_ptr_to_int(value_ptr, ctx.i64_type(), "ptr_to_i64")
                            .ok()?;
                        ctx.builder
                            .build_int_truncate(i64_val, ctx.bool_type(), "to_bool")
                            .ok()?
                            .into()
                    };
                    
                    let ok_block_end = ctx.builder.get_insert_block()?;
                    ctx.builder.build_unconditional_branch(cont_block).ok()?;

                    // === Err path ===
                    ctx.builder.position_at_end(err_block);
                    // Use default value for the type (0 for Int, 0.0 for Float, false for Bool)
                    let default_val = crate::utils::default_for_type(ctx, ok_llvm_type);
                    let err_block_end = ctx.builder.get_insert_block()?;
                    ctx.builder.build_unconditional_branch(cont_block).ok()?;

                    // === Continue block - merge with phi nodes ===
                    ctx.builder.position_at_end(cont_block);

                    // Create phi node for the VALUE (not pointer)
                    let ok_phi = ctx.builder.build_phi(ok_llvm_type, "ok_val_phi").ok()?;
                    ok_phi.add_incoming(&[
                        (&ok_extracted_val, ok_block_end),
                        (&default_val, err_block_end),
                    ]);
                    let ok_result = ok_phi.as_basic_value();

                    // Store ok value(s) to all ok_names
                    for ok_name in ok_names {
                        ctx.set_temp(ok_name, ok_result);
                    }
                    // Error is ignored, no phi needed for it

                    Some(ok_result)
                } else {
                    // === POINTER PATH (original behavior) ===
                    // Used when error is NOT ignored, or ok type is not scalar
                    
                    // === Ok path ===
                    ctx.builder.position_at_end(ok_block);

                    // For Ok path: ok values get the actual value, error gets nil
                    let ok_val_from_ok = value_ptr;
                    let err_val_from_ok = ctx.ptr_type().const_null();

                    ctx.builder.build_unconditional_branch(cont_block).ok()?;

                    // === Err path ===
                    ctx.builder.position_at_end(err_block);

                    // For Err path: ok values get nil, error gets the actual error
                    let ok_val_from_err = ctx.ptr_type().const_null();
                    let err_val_from_err = value_ptr;

                    ctx.builder.build_unconditional_branch(cont_block).ok()?;

                    // === Continue block - merge with phi nodes ===
                    ctx.builder.position_at_end(cont_block);

                    // Create phi node for ok value
                    let ok_phi = ctx.builder.build_phi(ctx.ptr_type(), "ok_phi").ok()?;
                    ok_phi.add_incoming(&[(&ok_val_from_ok, ok_block), (&ok_val_from_err, err_block)]);
                    let ok_result = ok_phi.as_basic_value();

                    // Store ok value(s) to all ok_names
                    for ok_name in ok_names {
                        ctx.set_temp(ok_name, ok_result);
                    }

                    // Create phi node for error value (if not ignored)
                    if !error_ignored {
                        let err_phi = ctx.builder.build_phi(ctx.ptr_type(), "err_phi").ok()?;
                        err_phi.add_incoming(&[
                            (&err_val_from_ok, ok_block),
                            (&err_val_from_err, err_block),
                        ]);
                        let err_result = err_phi.as_basic_value();
                        ctx.set_temp(error_name, err_result);
                    }

                    Some(ok_result)
                }
            }

            MirInstrKind::TypeOf {
                dest,
                value: _,
                value_type,
            } => {
                // Get the type name string based on the type
                let type_name: String = if let Some(kind) = ctx.get_type_kind(*value_type) {
                    match kind {
                        TypeKind::Int => "Int".to_string(),
                        TypeKind::Float => "Float".to_string(),
                        TypeKind::Bool => "Bool".to_string(),
                        TypeKind::Str => "Str".to_string(),
                        TypeKind::Void => "Nil".to_string(),
                        TypeKind::Array { .. } => "Array".to_string(),
                        TypeKind::Map { .. } => "Map".to_string(),
                        TypeKind::Tuple { .. } => "Tuple".to_string(),
                        TypeKind::Struct { name, .. } => name,
                        TypeKind::Enum { name, .. } => name,
                        TypeKind::Function { .. } => "Function".to_string(),
                        TypeKind::Result { .. } => "Result".to_string(),
                        TypeKind::Optional { .. } => "Optional".to_string(),
                        TypeKind::Any => "Any".to_string(),
                        TypeKind::TypeRef { name } => name,
                        TypeKind::Error => "Error".to_string(),
                    }
                } else {
                    "Unknown".to_string()
                };

                let type_str = ctx.const_string(&type_name);
                ctx.set_temp(dest, type_str.into());
                Some(type_str.into())
            }

            _ => None,
        }
    }
}

/// Convert MirOperand to LLVM value.
fn operand_to_value<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    operand: &MirOperand,
) -> Option<BasicValueEnum<'ctx>> {
    match operand {
        MirOperand::Const(c) => Some(const_to_value(ctx, c)),
        MirOperand::Local(name) | MirOperand::Temp(name) | MirOperand::Global(name) => {
            ctx.get_value(name)
        }
    }
}

/// Coerce an argument value to match the expected function parameter type.
/// 
/// This handles type mismatches between how values are produced (e.g., enum StructValues)
/// and how function parameters are declared (e.g., pointers for composite types).
fn coerce_arg_to_param_type<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
    expected_type: Option<BasicTypeEnum<'ctx>>,
) -> inkwell::values::BasicMetadataValueEnum<'ctx> {
    // If no expected type info, pass value as-is
    let Some(expected) = expected_type else {
        return val.into();
    };
    
    // If types already match, pass as-is
    if val.get_type() == expected {
        return val.into();
    }
    
    // Special case: StructValue passed where pointer is expected
    // This happens with enums: EnumCreate returns { i32, ptr } but function params expect ptr
    if val.is_struct_value() && expected.is_pointer_type() {
        // Box the struct value: allocate, store, return pointer
        let alloca = ctx
            .builder
            .build_alloca(val.get_type(), "arg_box")
            .unwrap();
        ctx.builder.build_store(alloca, val).ok();
        return alloca.into();
    }
    
    // Special case: PointerValue passed where struct is expected
    // This happens when JSON.parse returns a pointer to enum but function expects struct by value
    if val.is_pointer_value() && expected.is_struct_type() {
        // Load the struct from the pointer
        let loaded = ctx.builder.build_load(expected, val.into_pointer_value(), "arg_load").ok();
        if let Some(v) = loaded {
            return v.into();
        }
    }
    
    // Default: pass value as-is
    val.into()
}

/// Convert MirConst to LLVM value.
fn const_to_value<'ctx>(ctx: &CodegenContext<'ctx>, c: &MirConst) -> BasicValueEnum<'ctx> {
    match c {
        MirConst::Int(v) => ctx.const_i64(*v).into(),
        MirConst::Float(v) => ctx.const_f64(*v).into(),
        MirConst::Bool(v) => ctx.const_bool(*v).into(),
        MirConst::Nil => ctx.const_i64(0).into(),
        MirConst::Str(s) => ctx.const_string(s).into(),
    }
}

fn emit_print_value<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    type_id: doo_core::types::TypeId,
    val: BasicValueEnum<'ctx>,
    newline: bool,
    quote_strings: bool,
) {
    // Handle ANY type by inferring from LLVM value type
    if type_id == builtin::ANY {
        if val.is_int_value() {
            // Integer - print as number
            let fmt = if newline { "%lld\n" } else { "%lld" };
            let fmt = ctx.const_string(fmt);
            let i64v = ctx
                .builder
                .build_int_z_extend_or_bit_cast(val.into_int_value(), ctx.i64_type(), "print_i64")
                .ok();
            if let Some(i64v) = i64v {
                ctx.builder
                    .build_call(printf, &[fmt.into(), i64v.into()], "print_i")
                    .ok();
            }
            return;
        } else if val.is_float_value() {
            // Float - print as decimal
            let fmt = if newline { "%f\n" } else { "%f" };
            let fmt = ctx.const_string(fmt);
            ctx.builder
                .build_call(printf, &[fmt.into(), val.into()], "print_f")
                .ok();
            return;
        } else if val.is_pointer_value() {
            // Pointer - assume string (most common case for ANY)
            let fmt = if newline { "%s\n" } else { "%s" };
            let fmt = ctx.const_string(fmt);
            ctx.builder
                .build_call(printf, &[fmt.into(), val.into()], "print_str")
                .ok();
            return;
        }
        // Fallthrough to generic handling
    }

    if type_id == builtin::STR {
        if val.is_pointer_value() {
            if quote_strings {
                // Print string with surrounding quotes for collection display
                let open_quote = ctx.const_string("\"");
                let close_quote = if newline { ctx.const_string("\"\n") } else { ctx.const_string("\"") };
                let fmt = ctx.const_string("%s");
                ctx.builder
                    .build_call(printf, &[fmt.into(), open_quote.into()], "print_quote_open")
                    .ok();
                ctx.builder
                    .build_call(printf, &[fmt.into(), val.into()], "print_str")
                    .ok();
                ctx.builder
                    .build_call(printf, &[fmt.into(), close_quote.into()], "print_quote_close")
                    .ok();
            } else {
                let fmt = if newline { "%s\n" } else { "%s" };
                let fmt = ctx.const_string(fmt);
                ctx.builder
                    .build_call(printf, &[fmt.into(), val.into()], "print_str")
                    .ok();
            }
        }
        return;
    }

    if type_id == builtin::BOOL {
        if val.is_int_value() {
            let v = val.into_int_value();
            let is_true = ctx
                .builder
                .build_int_compare(IntPredicate::NE, v, v.get_type().const_zero(), "is_true")
                .ok();
            if let Some(is_true) = is_true {
                let true_s = ctx.const_string(if newline { "true\n" } else { "true" });
                let false_s = ctx.const_string(if newline { "false\n" } else { "false" });
                let out = ctx
                    .builder
                    .build_select(is_true, true_s, false_s, "bool_s")
                    .ok();
                if let Some(out) = out {
                    let fmt = ctx.const_string("%s");
                    ctx.builder
                        .build_call(printf, &[fmt.into(), out.into()], "print_bool")
                        .ok();
                }
            }
        }
        return;
    }

    if type_id == builtin::FLOAT {
        if val.is_float_value() {
            let fmt = if newline { "%f\n" } else { "%f" };
            let fmt = ctx.const_string(fmt);
            ctx.builder
                .build_call(printf, &[fmt.into(), val.into()], "print_f")
                .ok();
        }
        return;
    }

    if type_id == builtin::INT {
        if val.is_int_value() {
            let fmt = if newline { "%lld\n" } else { "%lld" };
            let fmt = ctx.const_string(fmt);
            let i64v = ctx
                .builder
                .build_int_z_extend_or_bit_cast(val.into_int_value(), ctx.i64_type(), "print_i64")
                .ok();
            if let Some(i64v) = i64v {
                ctx.builder
                    .build_call(printf, &[fmt.into(), i64v.into()], "print_i")
                    .ok();
            }
        }
        return;
    }

    // Handle enum as StructValue (inline { i32, ptr }) - must check BEFORE pointer check
    if val.is_struct_value() {
        if let Some(TypeKind::Enum { name, variants }) = ctx.get_type_kind(type_id) {
            emit_print_enum_value(ctx, printf, val.into_struct_value(), &name, &variants);
            if newline {
                let nl = ctx.const_string("\n");
                ctx.builder
                    .build_call(printf, &[ctx.const_string("%s").into(), nl.into()], "")
                    .ok();
            }
            return;
        }
    }

    if val.is_pointer_value() {
        let ptr = val.into_pointer_value();

        if let Some(kind) = ctx.get_type_kind(type_id) {
            match kind {
                TypeKind::Tuple { elements } => {
                    emit_print_tuple(ctx, printf, ptr, &elements);
                    if newline {
                        let nl = ctx.const_string("\n");
                        ctx.builder
                            .build_call(printf, &[ctx.const_string("%s").into(), nl.into()], "")
                            .ok();
                    }
                    return;
                }
                TypeKind::Struct { name, fields } => {
                    emit_print_struct(ctx, printf, ptr, &name, &fields);
                    if newline {
                        let nl = ctx.const_string("\n");
                        ctx.builder
                            .build_call(printf, &[ctx.const_string("%s").into(), nl.into()], "")
                            .ok();
                    }
                    return;
                }
                TypeKind::Enum { name, variants } => {
                    emit_print_enum(ctx, printf, ptr, &name, &variants);
                    if newline {
                        let nl = ctx.const_string("\n");
                        ctx.builder
                            .build_call(printf, &[ctx.const_string("%s").into(), nl.into()], "")
                            .ok();
                    }
                    return;
                }
                TypeKind::Array { element } => {
                    emit_print_array(ctx, printf, ptr, element);
                    if newline {
                        let nl = ctx.const_string("\n");
                        ctx.builder
                            .build_call(printf, &[ctx.const_string("%s").into(), nl.into()], "")
                            .ok();
                    }
                    return;
                }
                TypeKind::Map { key, value } => {
                    emit_print_map(ctx, printf, ptr, key, value);
                    if newline {
                        let nl = ctx.const_string("\n");
                        ctx.builder
                            .build_call(printf, &[ctx.const_string("%s").into(), nl.into()], "")
                            .ok();
                    }
                    return;
                }
                _ => {}
            }
        }

        // For unknown pointer types, assume string
        let fmt = if newline { "%s\n" } else { "%s" };
        let fmt = ctx.const_string(fmt);
        ctx.builder
            .build_call(printf, &[fmt.into(), ptr.into()], "print_str")
            .ok();
        return;
    }
}

fn emit_print_tuple<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    tuple_ptr: PointerValue<'ctx>,
    element_types: &[doo_core::types::TypeId],
) {
    let open = ctx.const_string("(");
    let fmt_s = ctx.const_string("%s");
    ctx.builder
        .build_call(printf, &[fmt_s.into(), open.into()], "")
        .ok();

    // Struct/Tuple layout: fields are pointers stored sequentially
    // But codegen might store values directly if primitive?
    // Current Tuple implementation (composites.rs) stores *pointers* to values or values?
    // Usually it delegates to generic struct logic.
    // Assuming pointers or specific types.
    // Wait, Generic CodeGen maps TypeId to LLVM Type.
    // Struct/Tuple are StructType in LLVM.

    // We need LLVM type of the tuple to build GEP.
    // But `val` is just `ptr` (i8* or opaque).
    // We should cast it to the specific struct type.

    // BUT we don't have easy access to the LLVM struct type here without regenerating it.
    // `ctx.get_llvm_type(type_id)` should return it.
    // However, if we don't pass `type_id` of the Tuple itself...
    // `element_types` allows us to reconstruct it?
    // Actually, `emit_print_value` has `type_id`.
    // Let's rely on that? No, I need the inner logic.

    // Simpler approach: offsets.
    // But LLVM structs have padding. GEP is safer.
    // Construct LLVM type for tuple.
    let elem_types: Vec<_> = element_types
        .iter()
        .map(|t| ctx.get_llvm_type(*t).into())
        .collect();
    let tuple_llvm_type = ctx.context.struct_type(&elem_types, false);
    let tuple_typed_ptr = ctx
        .builder
        .build_pointer_cast(
            tuple_ptr,
            tuple_llvm_type.ptr_type(AddressSpace::default()),
            "tuple_cast",
        )
        .ok();

    if let Some(base) = tuple_typed_ptr {
        for (i, &ty) in element_types.iter().enumerate() {
            if i > 0 {
                let comma = ctx.const_string(", ");
                ctx.builder
                    .build_call(printf, &[fmt_s.into(), comma.into()], "")
                    .ok();
            }

            let field_ptr = ctx
                .builder
                .build_struct_gep(tuple_llvm_type, base, i as u32, "field")
                .ok();
            if let Some(fp) = field_ptr {
                let llvm_ty = ctx.get_llvm_type(ty);
                let val = ctx.builder.build_load(llvm_ty, fp, "val").ok();
                if let Some(v) = val {
                    emit_print_value(ctx, printf, ty, v, false, true);
                }
            }
        }
    }

    let close = ctx.const_string(")");
    ctx.builder
        .build_call(printf, &[fmt_s.into(), close.into()], "")
        .ok();
}

fn emit_print_struct<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    struct_ptr: PointerValue<'ctx>,
    name: &str,
    fields: &[(String, doo_core::types::TypeId)],
) {
    let type_name_utf8 = format!("{} {{ ", name);
    let prefix = ctx.const_string(&type_name_utf8);
    let fmt_s = ctx.const_string("%s");
    ctx.builder
        .build_call(printf, &[fmt_s.into(), prefix.into()], "")
        .ok();

    // Use the cached named struct type if available, otherwise create from field types
    // Use get_llvm_type for consistent type mapping (matches JSON.parse, StructCreate, etc.)
    let struct_llvm_type = if let Some(cached) = ctx.lookup_struct_type(name) {
        cached
    } else {
        // Manually create the struct type using get_llvm_type for consistency
        let field_llvm_types: Vec<inkwell::types::BasicTypeEnum> = fields
            .iter()
            .map(|(_, type_id)| ctx.get_llvm_type(*type_id))
            .collect();
        ctx.context.struct_type(&field_llvm_types, false)
    };
    
    let base = struct_ptr;

    for (i, (fname, fty)) in fields.iter().enumerate() {
        if i > 0 {
            let comma = ctx.const_string(", ");
            ctx.builder
                .build_call(printf, &[fmt_s.into(), comma.into()], "")
                .ok();
        }

        // Print field name
        let fname_s = ctx.const_string(&format!("{}: ", fname));
        ctx.builder
            .build_call(printf, &[fmt_s.into(), fname_s.into()], "")
            .ok();

        let field_ptr = ctx
            .builder
            .build_struct_gep(struct_llvm_type, base, i as u32, "field")
            .ok();
        if let Some(fp) = field_ptr {
            // Use get_llvm_type for consistent type mapping
            let llvm_ty = ctx.get_llvm_type(*fty);
            let val = ctx.builder.build_load(llvm_ty, fp, "val").ok();
            if let Some(v) = val {
                emit_print_value(ctx, printf, *fty, v, false, true);
            }
        }
    }

    let close = ctx.const_string(" }");
    ctx.builder
        .build_call(printf, &[fmt_s.into(), close.into()], "")
        .ok();
}

fn emit_print_enum<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    enum_ptr: PointerValue<'ctx>,
    name: &str,
    variants: &[(String, Option<doo_core::types::TypeId>)],
) {
    // Enum layout: { i32 tag (at offset 0), ptr payload (at offset 8) }
    let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
    let i32_type = ctx.context.i32_type();

    // Get tag using raw byte offset (more reliable than struct GEP for mixed allocations)
    let tag_ptr = ctx
        .builder
        .build_pointer_cast(
            enum_ptr,
            i32_type.ptr_type(AddressSpace::default()),
            "tag_ptr",
        )
        .ok();

    let tag_val = if let Some(tp) = tag_ptr {
        ctx.builder
            .build_load(i32_type, tp, "tag")
            .ok()
            .map(|v| v.into_int_value())
    } else {
        None
    };

    let Some(tag) = tag_val else {
        return;
    };

    // Emit switch or if-chain to print correct variant
    // For simplicity here, we'll iterate variants and generate runtime check
    // Optimization: Use a switch statement block structure, but `emit_print_value` is recursive helper inside a block.
    // Generating complex control flow inside this helper is hard because it returns () and appends to current block.
    // We can do it!

    let current_fn = ctx
        .builder
        .get_insert_block()
        .unwrap()
        .get_parent()
        .unwrap();
    let merge_bb = ctx.context.append_basic_block(current_fn, "print_enum_end");
    let default_bb = ctx
        .context
        .append_basic_block(current_fn, "print_enum_default");

    // Generate switch
    let mut cases = Vec::with_capacity(variants.len());
    let mut target_bbs = Vec::with_capacity(variants.len());

    for (i, _) in variants.iter().enumerate() {
        let bb = ctx
            .context
            .append_basic_block(current_fn, &format!("print_enum_var_{}", i));
        cases.push((ctx.context.i32_type().const_int(i as u64, false), bb));
        target_bbs.push(bb);
    }

    ctx.builder.build_switch(tag, default_bb, &cases).ok();

    // Default (Should technically be unreachable if valid enum)
    ctx.builder.position_at_end(default_bb);
    let unk = ctx.const_string(&format!("{}::Unknown", name));
    let fmt_s = ctx.const_string("%s");
    ctx.builder
        .build_call(printf, &[fmt_s.into(), unk.into()], "")
        .ok();
    ctx.builder.build_unconditional_branch(merge_bb).ok();

    // Variants
    for (i, (var_name, payload_ty)) in variants.iter().enumerate() {
        let bb = target_bbs[i];
        ctx.builder.position_at_end(bb);

        // Print Variant Name
        let prefix = format!("{}::", name);
        let prefix_s = ctx.const_string(&prefix);
        ctx.builder
            .build_call(printf, &[fmt_s.into(), prefix_s.into()], "")
            .ok();

        let vname_s = ctx.const_string(var_name);
        ctx.builder
            .build_call(printf, &[fmt_s.into(), vname_s.into()], "")
            .ok();

        if let Some(pty) = payload_ty {
            let open = ctx.const_string("(");
            ctx.builder
                .build_call(printf, &[fmt_s.into(), open.into()], "")
                .ok();

            // Get the payload pointer at offset 8 (after tag + padding)
            let payload_ptr_field = unsafe {
                ctx.builder
                    .build_gep(
                        ctx.context.i8_type(),
                        enum_ptr,
                        &[ctx.context.i64_type().const_int(8, false)],
                        "payload_ptr_field",
                    )
                    .ok()
            };

            if let Some(ppf) = payload_ptr_field {
                // Cast to ptr* to load the stored pointer
                let ppf_typed = ctx
                    .builder
                    .build_pointer_cast(
                        ppf,
                        ptr_type.ptr_type(AddressSpace::default()),
                        "ppf_typed",
                    )
                    .ok();
                
                let payload_ptr = ppf_typed.and_then(|pt| {
                    ctx.builder
                        .build_load(ptr_type, pt, "payload_ptr")
                        .ok()
                        .map(|v| v.into_pointer_value())
                });

                if let Some(pp) = payload_ptr {
                    // For pointer types (Str, Array, Map, etc.), the payload_ptr IS the value
                    // For value types (Int, Float, Bool), payload_ptr points TO the value
                    let llvm_pty = ctx.get_llvm_type(*pty);
                    
                    if llvm_pty.is_pointer_type() {
                        // Pointer type: the payload IS the value (string ptr, array ptr, etc.)
                        emit_print_value(ctx, printf, *pty, pp.into(), false, true);
                    } else {
                        // Value type: load the actual value from the payload pointer
                        let val = ctx.builder.build_load(llvm_pty, pp, "pval").ok();
                        if let Some(v) = val {
                            emit_print_value(ctx, printf, *pty, v, false, true);
                        }
                    }
                }
            }

            let close = ctx.const_string(")");
            ctx.builder
                .build_call(printf, &[fmt_s.into(), close.into()], "")
                .ok();
        }

        ctx.builder.build_unconditional_branch(merge_bb).ok();
    }

    ctx.builder.position_at_end(merge_bb);
}

/// Print an enum from a StructValue (inline enum) - extracts tag and payload directly without boxing
fn emit_print_enum_value<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    enum_val: inkwell::values::StructValue<'ctx>,
    name: &str,
    variants: &[(String, Option<doo_core::types::TypeId>)],
) {
    // Extract tag from struct value (field 0)
    let tag = match ctx.builder.build_extract_value(enum_val, 0, "tag") {
        Ok(v) => v.into_int_value(),
        Err(_) => return,
    };

    // Extract payload pointer from struct value (field 1) 
    let payload_ptr = match ctx.builder.build_extract_value(enum_val, 1, "payload_ptr") {
        Ok(v) => v.into_pointer_value(),
        Err(_) => return,
    };

    let current_fn = ctx
        .builder
        .get_insert_block()
        .unwrap()
        .get_parent()
        .unwrap();
    let merge_bb = ctx.context.append_basic_block(current_fn, "print_enum_end");
    let default_bb = ctx
        .context
        .append_basic_block(current_fn, "print_enum_default");

    let mut cases = Vec::with_capacity(variants.len());
    let mut target_bbs = Vec::with_capacity(variants.len());

    for (i, _) in variants.iter().enumerate() {
        let bb = ctx
            .context
            .append_basic_block(current_fn, &format!("print_enum_var_{}", i));
        cases.push((ctx.context.i32_type().const_int(i as u64, false), bb));
        target_bbs.push(bb);
    }

    ctx.builder.build_switch(tag, default_bb, &cases).ok();

    // Default
    ctx.builder.position_at_end(default_bb);
    let unk = ctx.const_string(&format!("{}::Unknown", name));
    let fmt_s = ctx.const_string("%s");
    ctx.builder
        .build_call(printf, &[fmt_s.into(), unk.into()], "")
        .ok();
    ctx.builder.build_unconditional_branch(merge_bb).ok();

    // Variants
    for (i, (var_name, payload_ty)) in variants.iter().enumerate() {
        let bb = target_bbs[i];
        ctx.builder.position_at_end(bb);

        let fmt_s = ctx.const_string("%s");
        let prefix = format!("{}::", name);
        let prefix_s = ctx.const_string(&prefix);
        ctx.builder
            .build_call(printf, &[fmt_s.into(), prefix_s.into()], "")
            .ok();

        let vname_s = ctx.const_string(var_name);
        ctx.builder
            .build_call(printf, &[fmt_s.into(), vname_s.into()], "")
            .ok();

        if let Some(pty) = payload_ty {
            let open = ctx.const_string("(");
            ctx.builder
                .build_call(printf, &[fmt_s.into(), open.into()], "")
                .ok();

            // For pointer types, payload_ptr IS the value
            // For value types, load from payload_ptr
            let llvm_pty = ctx.get_llvm_type(*pty);
            
            if llvm_pty.is_pointer_type() {
                emit_print_value(ctx, printf, *pty, payload_ptr.into(), false, true);
            } else {
                let val = ctx.builder.build_load(llvm_pty, payload_ptr, "pval").ok();
                if let Some(v) = val {
                    emit_print_value(ctx, printf, *pty, v, false, true);
                }
            }

            let close = ctx.const_string(")");
            ctx.builder
                .build_call(printf, &[fmt_s.into(), close.into()], "")
                .ok();
        }

        ctx.builder.build_unconditional_branch(merge_bb).ok();
    }

    ctx.builder.position_at_end(merge_bb);
}

fn emit_print_array<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    array_ptr: PointerValue<'ctx>,
    elem_type: doo_core::types::TypeId,
) {
    let open = ctx.const_string("[");
    let fmt = ctx.const_string("%s");
    ctx.builder
        .build_call(printf, &[fmt.into(), open.into()], "print_arr_open")
        .ok();

    let Some(len_i32) = load_len_i32(ctx, array_ptr) else {
        let close = ctx.const_string("]");
        ctx.builder
            .build_call(printf, &[fmt.into(), close.into()], "print_arr_close")
            .ok();
        return;
    };
    let len_i64 = ctx
        .builder
        .build_int_z_extend(len_i32, ctx.i64_type(), "len_i64")
        .ok();
    let Some(len_i64) = len_i64 else {
        let close = ctx.const_string("]");
        ctx.builder
            .build_call(printf, &[fmt.into(), close.into()], "print_arr_close")
            .ok();
        return;
    };

    let elem_llvm = ctx.get_llvm_type(elem_type);
    let elem_ptr_ty = elem_llvm.ptr_type(AddressSpace::default());
    let base = ctx
        .builder
        .build_pointer_cast(array_ptr, elem_ptr_ty, "arr_data_cast")
        .ok();
    let Some(base) = base else {
        let close = ctx.const_string("]");
        ctx.builder
            .build_call(printf, &[fmt.into(), close.into()], "print_arr_close")
            .ok();
        return;
    };

    let current_fn = match ctx.builder.get_insert_block().and_then(|b| b.get_parent()) {
        Some(f) => f,
        None => return,
    };

    let loop_bb = ctx.context.append_basic_block(current_fn, "print_arr_loop");
    let body_bb = ctx.context.append_basic_block(current_fn, "print_arr_body");
    let inc_bb = ctx.context.append_basic_block(current_fn, "print_arr_inc");
    let end_bb = ctx.context.append_basic_block(current_fn, "print_arr_end");

    let idx_alloca = ctx.builder.build_alloca(ctx.i64_type(), "idx").ok();
    let Some(idx_alloca) = idx_alloca else {
        return;
    };
    ctx.builder
        .build_store(idx_alloca, ctx.i64_type().const_zero())
        .ok();

    ctx.builder.build_unconditional_branch(loop_bb).ok();

    ctx.builder.position_at_end(loop_bb);
    let idx = ctx
        .builder
        .build_load(ctx.i64_type(), idx_alloca, "idx")
        .ok()
        .map(|v| v.into_int_value());
    let Some(idx) = idx else {
        return;
    };
    let cond = ctx
        .builder
        .build_int_compare(IntPredicate::ULT, idx, len_i64, "cond")
        .ok();
    let Some(cond) = cond else {
        return;
    };
    ctx.builder
        .build_conditional_branch(cond, body_bb, end_bb)
        .ok();

    ctx.builder.position_at_end(body_bb);
    let need_comma = ctx
        .builder
        .build_int_compare(
            IntPredicate::UGT,
            idx,
            ctx.i64_type().const_zero(),
            "need_comma",
        )
        .ok();
    if let Some(need_comma) = need_comma {
        let comma_bb = ctx
            .context
            .append_basic_block(current_fn, "print_arr_comma");
        let after_comma_bb = ctx
            .context
            .append_basic_block(current_fn, "print_arr_after_comma");
        ctx.builder
            .build_conditional_branch(need_comma, comma_bb, after_comma_bb)
            .ok();

        ctx.builder.position_at_end(comma_bb);
        let comma = ctx.const_string(", ");
        ctx.builder
            .build_call(printf, &[fmt.into(), comma.into()], "print_comma")
            .ok();
        ctx.builder.build_unconditional_branch(after_comma_bb).ok();

        ctx.builder.position_at_end(after_comma_bb);
    }

    let elem_ptr = unsafe { ctx.builder.build_gep(elem_llvm, base, &[idx], "elem_ptr") }.ok();
    if let Some(elem_ptr) = elem_ptr {
        let elem_val = ctx.builder.build_load(elem_llvm, elem_ptr, "elem").ok();
        if let Some(elem_val) = elem_val {
            emit_print_value(ctx, printf, elem_type, elem_val, false, true);
        }
    }
    ctx.builder.build_unconditional_branch(inc_bb).ok();

    ctx.builder.position_at_end(inc_bb);
    let next = ctx
        .builder
        .build_int_add(idx, ctx.i64_type().const_int(1, false), "next")
        .ok();
    if let Some(next) = next {
        ctx.builder.build_store(idx_alloca, next).ok();
    }
    ctx.builder.build_unconditional_branch(loop_bb).ok();

    ctx.builder.position_at_end(end_bb);
    let close = ctx.const_string("]");
    ctx.builder
        .build_call(printf, &[fmt.into(), close.into()], "print_arr_close")
        .ok();
}

fn emit_print_map<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    map_ptr: PointerValue<'ctx>,
    key_type: doo_core::types::TypeId,
    val_type: doo_core::types::TypeId,
) {
    let open = ctx.const_string("{");
    let fmt = ctx.const_string("%s");
    ctx.builder
        .build_call(printf, &[fmt.into(), open.into()], "print_map_open")
        .ok();

    let Some(len_i32) = load_len_i32(ctx, map_ptr) else {
        let close = ctx.const_string("}");
        ctx.builder
            .build_call(printf, &[fmt.into(), close.into()], "print_map_close")
            .ok();
        return;
    };
    let len_i64 = ctx
        .builder
        .build_int_z_extend(len_i32, ctx.i64_type(), "len_i64")
        .ok();
    let Some(len_i64) = len_i64 else {
        let close = ctx.const_string("}");
        ctx.builder
            .build_call(printf, &[fmt.into(), close.into()], "print_map_close")
            .ok();
        return;
    };

    let key_llvm = ctx.get_llvm_type(key_type);
    let val_llvm = ctx.get_llvm_type(val_type);
    let pair_ty = ctx
        .context
        .struct_type(&[key_llvm.into(), val_llvm.into()], false);
    let pair_ptr_ty = pair_ty.ptr_type(AddressSpace::default());
    let base = ctx
        .builder
        .build_pointer_cast(map_ptr, pair_ptr_ty, "map_data_cast")
        .ok();
    let Some(base) = base else {
        let close = ctx.const_string("}");
        ctx.builder
            .build_call(printf, &[fmt.into(), close.into()], "print_map_close")
            .ok();
        return;
    };

    let current_fn = match ctx.builder.get_insert_block().and_then(|b| b.get_parent()) {
        Some(f) => f,
        None => return,
    };

    let loop_bb = ctx.context.append_basic_block(current_fn, "print_map_loop");
    let body_bb = ctx.context.append_basic_block(current_fn, "print_map_body");
    let inc_bb = ctx.context.append_basic_block(current_fn, "print_map_inc");
    let end_bb = ctx.context.append_basic_block(current_fn, "print_map_end");

    let idx_alloca = ctx.builder.build_alloca(ctx.i64_type(), "idx").ok();
    let Some(idx_alloca) = idx_alloca else {
        return;
    };
    ctx.builder
        .build_store(idx_alloca, ctx.i64_type().const_zero())
        .ok();
    ctx.builder.build_unconditional_branch(loop_bb).ok();

    ctx.builder.position_at_end(loop_bb);
    let idx = ctx
        .builder
        .build_load(ctx.i64_type(), idx_alloca, "idx")
        .ok()
        .map(|v| v.into_int_value());
    let Some(idx) = idx else {
        return;
    };
    let cond = ctx
        .builder
        .build_int_compare(IntPredicate::ULT, idx, len_i64, "cond")
        .ok();
    let Some(cond) = cond else {
        return;
    };
    ctx.builder
        .build_conditional_branch(cond, body_bb, end_bb)
        .ok();

    ctx.builder.position_at_end(body_bb);
    let need_comma = ctx
        .builder
        .build_int_compare(
            IntPredicate::UGT,
            idx,
            ctx.i64_type().const_zero(),
            "need_comma",
        )
        .ok();
    if let Some(need_comma) = need_comma {
        let comma_bb = ctx
            .context
            .append_basic_block(current_fn, "print_map_comma");
        let after_comma_bb = ctx
            .context
            .append_basic_block(current_fn, "print_map_after_comma");
        ctx.builder
            .build_conditional_branch(need_comma, comma_bb, after_comma_bb)
            .ok();

        ctx.builder.position_at_end(comma_bb);
        let comma = ctx.const_string(", ");
        ctx.builder
            .build_call(printf, &[fmt.into(), comma.into()], "print_comma")
            .ok();
        ctx.builder.build_unconditional_branch(after_comma_bb).ok();

        ctx.builder.position_at_end(after_comma_bb);
    }

    let pair_ptr = unsafe { ctx.builder.build_gep(pair_ty, base, &[idx], "pair_ptr") }.ok();
    if let Some(pair_ptr) = pair_ptr {
        let kptr = ctx
            .builder
            .build_struct_gep(pair_ty, pair_ptr, 0, "kptr")
            .ok();
        let vptr = ctx
            .builder
            .build_struct_gep(pair_ty, pair_ptr, 1, "vptr")
            .ok();
        if let (Some(kptr), Some(vptr)) = (kptr, vptr) {
            let k = ctx.builder.build_load(key_llvm, kptr, "k").ok();
            let v = ctx.builder.build_load(val_llvm, vptr, "v").ok();
            if let (Some(k), Some(v)) = (k, v) {
                emit_print_value(ctx, printf, key_type, k, false, true);
                let sep = ctx.const_string(": ");
                ctx.builder
                    .build_call(printf, &[fmt.into(), sep.into()], "print_sep")
                    .ok();
                emit_print_value(ctx, printf, val_type, v, false, true);
            }
        }
    }
    ctx.builder.build_unconditional_branch(inc_bb).ok();

    ctx.builder.position_at_end(inc_bb);
    let next = ctx
        .builder
        .build_int_add(idx, ctx.i64_type().const_int(1, false), "next")
        .ok();
    if let Some(next) = next {
        ctx.builder.build_store(idx_alloca, next).ok();
    }
    ctx.builder.build_unconditional_branch(loop_bb).ok();

    ctx.builder.position_at_end(end_bb);
    let close = ctx.const_string("}");
    ctx.builder
        .build_call(printf, &[fmt.into(), close.into()], "print_map_close")
        .ok();
}

// ============================================================================
// FFI Call Implementation
// ============================================================================

/// FFI function signature: (param_types, return_type, is_variadic)
/// - param_types: slice of ("ptr" | "i64" | "i32" | "f64" | "void")
/// - return_type: "ptr" | "i64" | "i32" | "f64" | "void"
/// - is_variadic: whether function accepts variable arguments
type FfiSignature = (&'static [&'static str], &'static str, bool);

/// Get FFI function signature for known functions.
/// Returns (param_types, return_type, is_variadic).
fn get_ffi_signature(symbol: &str) -> Option<FfiSignature> {
    // Use match for compile-time known signatures
    match symbol {
        // Standard C Library
        ffi_names::MALLOC => Some((&["i64"], "ptr", false)),
        ffi_names::FREE => Some((&["ptr"], "void", false)),
        ffi_names::REALLOC => Some((&["ptr", "i64"], "ptr", false)),
        ffi_names::STRLEN => Some((&["ptr"], "i64", false)),
        ffi_names::STRCMP => Some((&["ptr", "ptr"], "i32", false)),
        ffi_names::STRCPY => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::STRCAT => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::MEMCPY => Some((&["ptr", "ptr", "i64"], "ptr", false)),
        ffi_names::MEMSET => Some((&["ptr", "i32", "i64"], "ptr", false)),
        ffi_names::PRINTF => Some((&["ptr"], "i32", true)), // variadic
        ffi_names::SNPRINTF => Some((&["ptr", "i64", "ptr"], "i32", true)),
        ffi_names::PUTS => Some((&["ptr"], "i32", false)),
        ffi_names::PUTCHAR => Some((&["i32"], "i32", false)),

        // Doo Runtime
        ffi_names::DOO_ALLOC => Some((&["i64"], "ptr", false)),
        ffi_names::DOO_FREE => Some((&["ptr"], "void", false)),
        ffi_names::DOO_REALLOC => Some((&["ptr", "i64"], "ptr", false)),

        // JSON FFI
        ffi_names::DOO_JSON_WRITER_NEW => Some((&[], "ptr", false)),
        ffi_names::DOO_JSON_WRITER_FREE => Some((&["ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITER_FINISH => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_JSON_WRITE_START_OBJECT => Some((&["ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITE_END_OBJECT => Some((&["ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITE_START_ARRAY => Some((&["ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITE_END_ARRAY => Some((&["ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITE_COMMA => Some((&["ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITE_COLON => Some((&["ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITE_KEY => Some((&["ptr", "ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITE_KEY_INT => Some((&["ptr", "i64"], "void", false)),
        ffi_names::DOO_JSON_WRITE_KEY_FLOAT => Some((&["ptr", "f64"], "void", false)),
        ffi_names::DOO_JSON_WRITE_KEY_BOOL => Some((&["ptr", "i1"], "void", false)),
        ffi_names::DOO_JSON_WRITE_INT => Some((&["ptr", "i64"], "void", false)),
        ffi_names::DOO_JSON_WRITE_FLOAT => Some((&["ptr", "f64"], "void", false)),
        ffi_names::DOO_JSON_WRITE_BOOL => Some((&["ptr", "i32"], "void", false)),
        ffi_names::DOO_JSON_WRITE_STRING => Some((&["ptr", "ptr"], "void", false)),
        ffi_names::DOO_JSON_WRITE_NULL => Some((&["ptr"], "void", false)),
        ffi_names::DOO_JSON_PARSE => Some((&["ptr"], "ptr", false)),

        // File FFI
        ffi_names::DOO_FILE_READ => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_FILE_WRITE => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_FILE_APPEND => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_FILE_DELETE => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_FILE_EXISTS => Some((&["ptr"], "i32", false)),
        ffi_names::DOO_FILE_METADATA => Some((&["ptr"], "ptr", false)),

        // HTTP FFI
        ffi_names::DOO_HTTP_SERVER_NEW => Some((&[], "ptr", false)),
        ffi_names::DOO_HTTP_SERVER_LISTEN => Some((&["ptr", "i32"], "i32", false)),
        ffi_names::DOO_HTTP_REGISTER_ROUTE => Some((&["ptr", "ptr", "ptr", "ptr"], "void", false)),
        ffi_names::DOO_HTTP_REQ_GET_HEADER => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_REQ_GET_BODY => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_REQ_GET_PARAM => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_REQ_GET_QUERY => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_HTTP_RES_SET_STATUS => Some((&["ptr", "i32"], "void", false)),
        ffi_names::DOO_HTTP_RES_SET_HEADER => Some((&["ptr", "ptr", "ptr"], "void", false)),
        ffi_names::DOO_HTTP_RES_SET_BODY => Some((&["ptr", "ptr"], "void", false)),
        ffi_names::DOO_HTTP_RES_JSON => Some((&["ptr", "ptr"], "void", false)),

        // Database FFI
        ffi_names::DOO_DB_POSTGRES => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_DB_FIND => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_FIND_ALL => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_INSERT => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_UPDATE => Some((&["ptr", "ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_DELETE => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_RAW => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_QUERY => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_DB_EXISTS => Some((&["ptr", "ptr", "ptr"], "i32", false)),
        ffi_names::DOO_DB_RESULT_FREE => Some((&["ptr"], "void", false)),

        // Auth FFI
        ffi_names::DOO_AUTH_HASH_PASSWORD => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_AUTH_VERIFY_PASSWORD => Some((&["ptr", "ptr"], "i32", false)),
        ffi_names::DOO_AUTH_SIGN_TOKEN => Some((&["ptr", "ptr", "i64"], "ptr", false)),
        ffi_names::DOO_AUTH_VERIFY_TOKEN => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_AUTH_FREE_RESULT => Some((&["ptr"], "void", false)),

        // String FFI
        ffi_names::DOO_STRING_LEN_UTF8 => Some((&["ptr"], "i64", false)),
        ffi_names::DOO_STRING_CHAR_AT_UTF8 => Some((&["ptr", "i64"], "ptr", false)),
        ffi_names::DOO_STRING_REVERSE_UTF8 => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_STRING_SUBSTRING_UTF8 => Some((&["ptr", "i64", "i64"], "ptr", false)),
        ffi_names::DOO_STRING_REPLACE => Some((&["ptr", "ptr", "ptr"], "ptr", false)),
        ffi_names::DOO_STRING_TRIM => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_STRING_TRIM_START => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_STRING_TRIM_END => Some((&["ptr"], "ptr", false)),
        ffi_names::DOO_STRING_SPLIT => Some((&["ptr", "ptr"], "ptr", false)),

        // Math FFI
        ffi_names::FABS => Some((&["f64"], "f64", false)),
        ffi_names::FLOOR => Some((&["f64"], "f64", false)),
        ffi_names::CEIL => Some((&["f64"], "f64", false)),
        ffi_names::ROUND => Some((&["f64"], "f64", false)),
        ffi_names::SQRT => Some((&["f64"], "f64", false)),

        // Unknown - use default signature
        _ => None,
    }
}

/// Convert FFI type string to LLVM type.
fn ffi_type_to_llvm<'ctx>(
    ctx: &CodegenContext<'ctx>,
    type_str: &str,
) -> Option<BasicTypeEnum<'ctx>> {
    match type_str {
        "ptr" => Some(ctx.context.ptr_type(AddressSpace::default()).into()),
        "i64" => Some(ctx.i64_type().into()),
        "i32" => Some(ctx.i32_type().into()),
        "f64" => Some(ctx.f64_type().into()),
        "void" => None,                   // void is not a BasicType
        _ => Some(ctx.i64_type().into()), // default to i64
    }
}

/// Declare an FFI function with proper signature and external linkage.
fn declare_ffi_function<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    symbol: &str,
    arg_count: usize,
) -> FunctionValue<'ctx> {
    // Check if already declared
    if let Some(func) = ctx.get_function(symbol) {
        return func;
    }

    let ptr_ty = ctx.context.ptr_type(AddressSpace::default());

    // Get known signature or build default
    let (param_types_vec, return_type, is_variadic) =
        if let Some((param_strs, ret_str, variadic)) = get_ffi_signature(symbol) {
            // Known function: use precise signature
            let params: Vec<BasicTypeEnum> = param_strs
                .iter()
                .filter_map(|s| ffi_type_to_llvm(ctx, s))
                .collect();

            let ret = ffi_type_to_llvm(ctx, ret_str);
            (params, ret, variadic)
        } else {
            // Unknown function: infer from argument count
            // Default: ptr params, ptr return
            let params: Vec<BasicTypeEnum> = (0..arg_count).map(|_| ptr_ty.into()).collect();
            (params, Some(ptr_ty.into()), false)
        };

    // Build function type
    let param_meta: Vec<inkwell::types::BasicMetadataTypeEnum> =
        param_types_vec.iter().map(|t| (*t).into()).collect();

    let fn_type = match return_type {
        Some(ret) => ret.fn_type(&param_meta, is_variadic),
        None => ctx.context.void_type().fn_type(&param_meta, is_variadic),
    };

    // Declare with external linkage for FFI
    let func = ctx
        .module
        .add_function(symbol, fn_type, Some(Linkage::External));

    // Cache the function
    // Note: function_cache is private, so we rely on module.get_function
    func
}

/// Convert a Doo value to FFI-compatible value if needed.
fn convert_to_ffi_arg<'ctx>(
    ctx: &CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
    expected_type: Option<&str>,
) -> inkwell::values::BasicMetadataValueEnum<'ctx> {
    match expected_type {
        Some("i32") => {
            // Convert i64 to i32 if needed
            if val.is_int_value() {
                let int_val = val.into_int_value();
                if int_val.get_type().get_bit_width() == 64 {
                    let truncated = ctx
                        .builder
                        .build_int_truncate(int_val, ctx.i32_type(), "i64_to_i32")
                        .unwrap();
                    return truncated.into();
                }
            }
            val.into()
        }
        Some("f64") => {
            // Ensure float type
            if val.is_int_value() {
                let int_val = val.into_int_value();
                let float_val = ctx
                    .builder
                    .build_signed_int_to_float(int_val, ctx.f64_type(), "int_to_f64")
                    .unwrap();
                return float_val.into();
            }
            val.into()
        }
        _ => val.into(),
    }
}

/// Emit an FFI call with proper type handling.
fn emit_ffi_call<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    dest: Option<&str>,
    symbol: &str,
    args: &[MirOperand],
) -> Option<BasicValueEnum<'ctx>> {
    if std::env::var("DOO_DEBUG").is_ok() {
        eprintln!(
            "[CODEGEN] FfiCall: {} with {} args -> {:?}",
            symbol,
            args.len(),
            dest
        );
    }

    // Declare FFI function if not already declared
    let func = declare_ffi_function(ctx, symbol, args.len());

    // Get expected param types from signature (for conversion)
    let expected_types: Vec<Option<&str>> =
        if let Some((param_strs, _, _)) = get_ffi_signature(symbol) {
            param_strs.iter().map(|s| Some(*s)).collect()
        } else {
            args.iter().map(|_| None).collect()
        };

    // Convert arguments
    let arg_vals: Vec<inkwell::values::BasicMetadataValueEnum> = args
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            let val = operand_to_value(ctx, a)?;
            let expected = expected_types.get(i).copied().flatten();
            Some(convert_to_ffi_arg(ctx, val, expected))
        })
        .collect();

    // Build call
    let call_site = ctx.builder.build_call(func, &arg_vals, "ffi_call").ok()?;

    // Handle return value
    if let Some(dest_name) = dest {
        if let Some(ret_val) = call_site.try_as_basic_value().left() {
            ctx.set_temp(dest_name, ret_val);
            return Some(ret_val);
        }
    }

    // For void functions, return None
    call_site.try_as_basic_value().left()
}

// ============================================================================
// Result/Error Handling Helpers
// ============================================================================

/// Convert a value to a pointer representation for storing in Result payload.
/// - Pointers: pass through as-is
/// - Integers: use inttoptr
/// - Floats: bitcast to i64, then inttoptr
fn value_to_ptr<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
) -> Option<PointerValue<'ctx>> {
    if val.is_pointer_value() {
        // Already a pointer (string, array, map, struct)
        Some(val.into_pointer_value())
    } else if val.is_int_value() {
        // Cast integer to pointer using inttoptr
        let int_val = val.into_int_value();
        let int_64 = if int_val.get_type().get_bit_width() == 64 {
            int_val
        } else {
            ctx.builder
                .build_int_z_extend(int_val, ctx.i64_type(), "ext")
                .ok()?
        };
        ctx.builder
            .build_int_to_ptr(int_64, ctx.ptr_type(), "int_as_ptr")
            .ok()
    } else if val.is_float_value() {
        // Bitcast float to i64 then to pointer
        let float_val = val.into_float_value();
        let alloca = ctx.builder.build_alloca(ctx.f64_type(), "f_tmp").ok()?;
        ctx.builder.build_store(alloca, float_val).ok()?;
        let i64_ptr = ctx
            .builder
            .build_pointer_cast(alloca, ctx.ptr_type(), "i64_ptr")
            .ok()?;
        let i64_val = ctx
            .builder
            .build_load(ctx.i64_type(), i64_ptr, "f_as_i64")
            .ok()?
            .into_int_value();
        ctx.builder
            .build_int_to_ptr(i64_val, ctx.ptr_type(), "float_as_ptr")
            .ok()
    } else if val.is_struct_value() {
        // Heap-allocate struct and return pointer
        let struct_val = val.into_struct_value();
        let struct_type = struct_val.get_type();
        let heap_ptr = ctx.builder.build_malloc(struct_type, "struct_heap").ok()?;
        ctx.builder.build_store(heap_ptr, struct_val).ok()?;
        Some(heap_ptr)
    } else {
        // Fallback: use null pointer
        Some(ctx.ptr_type().const_null())
    }
}

/// Load a Result struct from a value that may be a pointer or struct.
fn load_result_struct<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    result_val: BasicValueEnum<'ctx>,
) -> Option<inkwell::values::StructValue<'ctx>> {
    let result_struct_type = ctx
        .context
        .struct_type(&[ctx.i32_type().into(), ctx.ptr_type().into()], false);

    if result_val.is_pointer_value() && !result_val.is_struct_value() {
        // Load from pointer
        let result_ptr = result_val.into_pointer_value();
        ctx.builder
            .build_load(result_struct_type, result_ptr, "result_struct_load")
            .ok()?
            .try_into()
            .ok()
    } else if result_val.is_struct_value() {
        // Already a struct value
        Some(result_val.into_struct_value())
    } else {
        // Not a Result - return None
        None
    }
}

/// Emit panic code: print message and exit(1).
fn emit_panic<'ctx>(ctx: &mut CodegenContext<'ctx>, message: &str) -> Option<()> {
    // Get or declare printf
    let printf_type = ctx.i32_type().fn_type(&[ctx.ptr_type().into()], true);
    let printf = ctx
        .module
        .get_function("printf")
        .unwrap_or_else(|| ctx.module.add_function("printf", printf_type, None));

    // Print panic message
    let panic_fmt = ctx.const_string("panic: %s\n");
    let panic_msg = ctx.const_string(message);
    ctx.builder
        .build_call(printf, &[panic_fmt.into(), panic_msg.into()], "print_panic")
        .ok()?;

    // Get or declare exit
    let exit_type = ctx
        .context
        .void_type()
        .fn_type(&[ctx.i32_type().into()], false);
    let exit_fn = ctx
        .module
        .get_function("exit")
        .unwrap_or_else(|| ctx.module.add_function("exit", exit_type, None));

    // Exit with code 1
    let exit_code = ctx.i32_type().const_int(1, false);
    ctx.builder
        .build_call(exit_fn, &[exit_code.into()], "exit_on_panic")
        .ok()?;

    ctx.builder.build_unreachable().ok()?;
    Some(())
}
