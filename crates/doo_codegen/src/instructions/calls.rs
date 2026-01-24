//! Call Instruction Handler
//!
//! Handles: Call, MethodCall, Print

use doo_core::constants::ffi_names;
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};
use doo_mir::{MirInstr, MirInstrKind, MirOperand, MirConst};
use doo_core::types::TypeKind;
use doo_core::types::builtin;
use crate::context::CodegenContext;
use crate::builtins::{StringBuiltins, ArrayBuiltins, MapBuiltins, JsonBuiltins};
use super::InstructionHandler;

/// Call/invocation instruction handler.
pub struct CallHandler;

impl<'ctx> InstructionHandler<'ctx> for CallHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(instr.kind,
            MirInstrKind::Call { .. } |
            MirInstrKind::MethodCall { .. } |
            MirInstrKind::FfiCall { .. } |
            MirInstrKind::Print { .. } |
            MirInstrKind::WrapOk { .. } |
            MirInstrKind::WrapErr { .. } |
            MirInstrKind::IsOk { .. } |
            MirInstrKind::UnwrapOk { .. } |
            MirInstrKind::UnwrapErr { .. }
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
                let arg_vals: Vec<_> = args.iter()
                    .filter_map(|a| operand_to_value(ctx, a))
                    .map(|v| v.into())
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

            MirInstrKind::MethodCall { dest, receiver, receiver_type, method, args, arg_types } => {
                // Intercept JSON.stringify and JSON.parse (Static Specialization)
                if let MirOperand::Local(name) = receiver {
                    if name == "JSON" {
                        if method == "stringify" {
                            if let (Some(arg_op), Some(&arg_type)) = (args.first(), arg_types.first()) {
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
                                    let result = JsonBuiltins::emit_parse(ctx, val);
                                    if let (Some(r), Some(dst)) = (result, dest) {
                                        ctx.set_temp(dst, r);
                                    }
                                    return result;
                                }
                            }
                            return None;
                        }
                    }
                }

                let recv_val = operand_to_value(ctx, receiver)?;
                let arg_vals: Vec<_> = args
                    .iter()
                    .filter_map(|a| operand_to_value(ctx, a))
                    .collect();

                let receiver_name = match receiver {
                    MirOperand::Local(name) | MirOperand::Temp(name) => Some(name.as_str()),
                    _ => None,
                };

                // Builtin dispatch (single source of truth via TypeRegistry)
                if recv_val.is_pointer_value() {
                    let recv_ptr = recv_val.into_pointer_value();
                    if let Some(kind) = ctx.get_type_kind(*receiver_type) {
                        let builtin_result = match kind {
                            TypeKind::Str => {
                                StringBuiltins::dispatch(ctx, dest.as_deref(), recv_ptr, method, &arg_vals)
                            }
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
                            _ => None,
                        };
                        if builtin_result.is_some() {
                            return builtin_result;
                        }
                    }
                }

                // Fallback: lookup method function, prepend receiver to args
                // Format: _method_{TypeName}_{MethodName}
                let type_name = if let Some(kind) = ctx.get_type_kind(*receiver_type) {
                    match kind {
                        TypeKind::Struct(name, _) => Some(name),
                        TypeKind::Enum(name, _) => Some(name),
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
                        let call_site = ctx.builder.build_call(func_val, &all_args, "mcall").ok()?;

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

            MirInstrKind::FfiCall { dest, lib: _, symbol, args } => {
                // FFI call: declare external function if needed and call
                // Symbol is the C function name (e.g., "doo_http_router_create")
                
                // Get or declare the FFI function
                let func = if let Some(f) = ctx.get_function(symbol) {
                    f
                } else {
                    // Auto-declare as i64(...) -> i64 for now
                    // Full implementation would use FFI type metadata
                    let i64_type = ctx.i64_type();
                    let arg_types: Vec<_> = args.iter()
                        .map(|_| i64_type.into())
                        .collect();
                    let fn_type = i64_type.fn_type(&arg_types.iter().map(|t: &inkwell::types::BasicTypeEnum| (*t).into()).collect::<Vec<_>>(), false);
                    ctx.module.add_function(symbol, fn_type, None)
                };
                
                let arg_vals: Vec<_> = args.iter()
                    .filter_map(|a| operand_to_value(ctx, a))
                    .map(|v| v.into())
                    .collect();
                
                let call_site = ctx.builder.build_call(func, &arg_vals, "ffi_call").ok()?;
                
                if let Some(dest_name) = dest {
                    if let Some(ret_val) = call_site.try_as_basic_value().left() {
                        ctx.set_temp(dest_name, ret_val);
                        return Some(ret_val);
                    }
                }
                None
            }

            MirInstrKind::Print { values, value_types } => {
                // Print built-in: call printf or custom print function
                if let Some(printf) = ctx.get_function(ffi_names::PRINTF) {
                    for (i, val) in values.iter().enumerate() {
                        let ty = value_types.get(i).copied().unwrap_or(doo_core::types::builtin::ANY);
                        let is_last = i + 1 == values.len();
                        if let Some(v) = operand_to_value(ctx, val) {
                            if let Some(kind) = ctx.get_type_kind(ty) {
                                match kind {
                                    TypeKind::Str => {
                                        emit_print_value(ctx, printf, ty, v, false);
                                    }
                                    TypeKind::Bool => {
                                        emit_print_value(ctx, printf, ty, v, false);
                                    }
                                    TypeKind::Int | TypeKind::Float => {
                                        emit_print_value(ctx, printf, ty, v, false);
                                    }
                                    TypeKind::Array { element } => {
                                        if v.is_pointer_value() {
                                            emit_print_array(ctx, printf, v.into_pointer_value(), element);
                                        } else {
                                            emit_print_value(ctx, printf, builtin::ANY, v, false);
                                        }
                                    }
                                    TypeKind::Map { key, value } => {
                                        if v.is_pointer_value() {
                                            emit_print_map(ctx, printf, v.into_pointer_value(), key, value);
                                        } else {
                                            emit_print_value(ctx, printf, builtin::ANY, v, false);
                                        }
                                    }
                                    _ => {
                                        emit_print_value(ctx, printf, builtin::ANY, v, false);
                                    }
                                }
                            } else {
                                emit_print_value(ctx, printf, ty, v, false);
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
                }
                None
            }

            MirInstrKind::WrapOk { dest, value } => {
                // Result::Ok = (tag=0, value)
                let val = operand_to_value(ctx, value)?;
                // Simplified: just pass through the value, Ok is implicit
                ctx.set_temp(dest, val);
                Some(val)
            }

            MirInstrKind::WrapErr { dest, value } => {
                // Result::Err = (tag=1, value)
                let val = operand_to_value(ctx, value)?;
                // For errors, we'd normally wrap in Result struct
                ctx.set_temp(dest, val);
                Some(val)
            }

            MirInstrKind::IsOk { dest, value } => {
                // Check if result is Ok (tag == 0)
                let _val = operand_to_value(ctx, value)?;
                // Simplified: assume all results are Ok for now
                let is_ok = ctx.const_bool(true);
                ctx.set_temp(dest, is_ok.into());
                Some(is_ok.into())
            }

            MirInstrKind::UnwrapOk { dest, value } => {
                // Extract Ok value from Result
                let val = operand_to_value(ctx, value)?;
                ctx.set_temp(dest, val);
                Some(val)
            }

            MirInstrKind::UnwrapErr { dest, value } => {
                // Extract Err value from Result
                let val = operand_to_value(ctx, value)?;
                ctx.set_temp(dest, val);
                Some(val)
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
) {
    if type_id == builtin::STR {
        if val.is_pointer_value() {
            let fmt = if newline { "%s\n" } else { "%s" };
            let fmt = ctx.const_string(fmt);
            ctx.builder
                .build_call(printf, &[fmt.into(), val.into()], "print_str")
                .ok();
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

    if val.is_pointer_value() {
        let ptr = val.into_pointer_value();
        
        if let Some(kind) = ctx.get_type_kind(type_id) {
            match kind {
                TypeKind::Tuple(types) => {
                    emit_print_tuple(ctx, printf, ptr, &types);
                    if newline {
                        let nl = ctx.const_string("\n");
                        ctx.builder.build_call(printf, &[ctx.const_string("%s").into(), nl.into()], "").ok();
                    }
                    return;
                }
                TypeKind::Struct(name, fields) => {
                    emit_print_struct(ctx, printf, ptr, &name, &fields);
                    if newline {
                        let nl = ctx.const_string("\n");
                        ctx.builder.build_call(printf, &[ctx.const_string("%s").into(), nl.into()], "").ok();
                    }
                    return;
                }
                TypeKind::Enum(name, variants) => {
                    emit_print_enum(ctx, printf, ptr, &name, &variants);
                    if newline {
                        let nl = ctx.const_string("\n");
                        ctx.builder.build_call(printf, &[ctx.const_string("%s").into(), nl.into()], "").ok();
                    }
                    return;
                }
                _ => {}
            }
        }

        let fmt = if newline { "%p\n" } else { "%p" };
        let fmt = ctx.const_string(fmt);
        ctx.builder
            .build_call(printf, &[fmt.into(), ptr.into()], "print_ptr")
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
    ctx.builder.build_call(printf, &[fmt_s.into(), open.into()], "").ok();

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
    let elem_types: Vec<_> = element_types.iter().map(|t| ctx.get_llvm_type(*t).into()).collect();
    let tuple_llvm_type = ctx.context.struct_type(&elem_types, false);
    let tuple_typed_ptr = ctx.builder.build_pointer_cast(tuple_ptr, tuple_llvm_type.ptr_type(AddressSpace::default()), "tuple_cast").ok();
    
    if let Some(base) = tuple_typed_ptr {
        for (i, &ty) in element_types.iter().enumerate() {
            if i > 0 {
                let comma = ctx.const_string(", ");
                ctx.builder.build_call(printf, &[fmt_s.into(), comma.into()], "").ok();
            }
            
            let field_ptr = ctx.builder.build_struct_gep(tuple_llvm_type, base, i as u32, "field").ok();
            if let Some(fp) = field_ptr {
                let val = ctx.builder.build_load(ctx.get_llvm_type(ty), fp, "val").ok();
                if let Some(v) = val {
                    emit_print_value(ctx, printf, ty, v, false);
                }
            }
        }
    }

    let close = ctx.const_string(")");
    ctx.builder.build_call(printf, &[fmt_s.into(), close.into()], "").ok();
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
    ctx.builder.build_call(printf, &[fmt_s.into(), prefix.into()], "").ok();

    let field_types: Vec<_> = fields.iter().map(|(_, t)| ctx.get_llvm_type(*t).into()).collect();
    let struct_llvm_type = ctx.context.struct_type(&field_types, false);
    let struct_typed_ptr = ctx.builder.build_pointer_cast(struct_ptr, struct_llvm_type.ptr_type(AddressSpace::default()), "struct_cast").ok();

    if let Some(base) = struct_typed_ptr {
        for (i, (fname, fty)) in fields.iter().enumerate() {
            if i > 0 {
                let comma = ctx.const_string(", ");
                ctx.builder.build_call(printf, &[fmt_s.into(), comma.into()], "").ok();
            }
            
            // Print field name
            let fname_s = ctx.const_string(&format!("{}: ", fname));
            ctx.builder.build_call(printf, &[fmt_s.into(), fname_s.into()], "").ok();

            let field_ptr = ctx.builder.build_struct_gep(struct_llvm_type, base, i as u32, "field").ok();
            if let Some(fp) = field_ptr {
                let val = ctx.builder.build_load(ctx.get_llvm_type(*fty), fp, "val").ok();
                if let Some(v) = val {
                    emit_print_value(ctx, printf, *fty, v, false);
                }
            }
        }
    }

    let close = ctx.const_string(" }");
    ctx.builder.build_call(printf, &[fmt_s.into(), close.into()], "").ok();
}

fn emit_print_enum<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    enum_ptr: PointerValue<'ctx>,
    name: &str,
    variants: &[(String, Option<doo_core::types::TypeId>)],
) {
    // Enum Memory Layout: { tag: i32, payload: Union }
    // But payload size varies.
    // If we simply treat it as { i32, max_align_payload }?
    // Or we cast to { i32, PayloadType } based on tag?
    // We first read tag (first 4 bytes).
    
    let i32_ptr_ty = ctx.context.i32_type().ptr_type(AddressSpace::default());
    let tag_ptr = ctx.builder.build_pointer_cast(enum_ptr, i32_ptr_ty, "tag_ptr").ok();
    
    let tag_val = if let Some(tp) = tag_ptr {
        ctx.builder.build_load(ctx.context.i32_type(), tp, "tag").ok().map(|v| v.into_int_value())
    } else {
        None
    };

    let Some(tag) = tag_val else { return; };

    // Emit switch or if-chain to print correct variant
    // For simplicity here, we'll iterate variants and generate runtime check
    // Optimization: Use a switch statement block structure, but `emit_print_value` is recursive helper inside a block.
    // Generating complex control flow inside this helper is hard because it returns () and appends to current block.
    // We can do it!
    
    let current_fn = ctx.builder.get_insert_block().unwrap().get_parent().unwrap();
    let merge_bb = ctx.context.append_basic_block(current_fn, "print_enum_end");
    let default_bb = ctx.context.append_basic_block(current_fn, "print_enum_default");
    
    // Generate switch
    let mut cases = Vec::with_capacity(variants.len());
    let mut target_bbs = Vec::with_capacity(variants.len());
    
    for (i, _) in variants.iter().enumerate() {
        let bb = ctx.context.append_basic_block(current_fn, &format!("print_enum_var_{}", i));
        cases.push((ctx.context.i32_type().const_int(i as u64, false), bb));
        target_bbs.push(bb);
    }
    
    ctx.builder.build_switch(tag, default_bb, &cases).ok();
    
    // Default (Should technically be unreachable if valid enum)
    ctx.builder.position_at_end(default_bb);
    let unk = ctx.const_string(&format!("{}::Unknown", name));
    let fmt_s = ctx.const_string("%s");
    ctx.builder.build_call(printf, &[fmt_s.into(), unk.into()], "").ok();
    ctx.builder.build_unconditional_branch(merge_bb).ok();
    
    // Variants
    for (i, (var_name, payload_ty)) in variants.iter().enumerate() {
        let bb = target_bbs[i];
        ctx.builder.position_at_end(bb);
        
        // Print Variant Name
        let prefix = format!("{}::", name);
        let prefix_s = ctx.const_string(&prefix);
        ctx.builder.build_call(printf, &[fmt_s.into(), prefix_s.into()], "").ok();
        
        let vname_s = ctx.const_string(var_name);
        ctx.builder.build_call(printf, &[fmt_s.into(), vname_s.into()], "").ok();
        
        if let Some(pty) = payload_ty {
            let open = ctx.const_string("(");
            ctx.builder.build_call(printf, &[fmt_s.into(), open.into()], "").ok();
            
            // Payload pointer
            // Enum layout: { tag(4), padding(4), payload... } (typically aligned to 8 or max align)
            // Assuming 8 byte alignment for payload if we use `alloc_enum` logic.
            // Let's calculate payload address. 
            // Better: cast enum_ptr to { i32, Payload }*? 
            // Or { i64, Payload } if aligned?
            // Safer: Cast to packed struct { i32, Payload }? No, alignment rules apply.
            // Best bet: Payload starts at offset 8 (standard for our compiler simplifications? or 4? check `enums.rs`)
            // Assuming offset 8 for now to be safe/lazy?
            // Let's check: layout logic usually standardizes.
            
            // Let's trust `enums.rs` or standard layout.
            // Assuming offset 8 (tag is 4 bytes, usually padded to 8 for alignment).
            let payload_offset = 8;
            let payload_base = unsafe {
                ctx.builder.build_gep(
                    ctx.context.i8_type(),
                    enum_ptr,
                    &[ctx.context.i64_type().const_int(payload_offset, false)],
                    "payload_base"
                ).ok()
            };
            
            if let Some(base) = payload_base {
                // Cast base to payload pointer type
                let llvm_pty = ctx.get_llvm_type(*pty);
                let pptr = ctx.builder.build_pointer_cast(base, llvm_pty.ptr_type(AddressSpace::default()), "pptr").ok();
                
                if let Some(p) = pptr {
                    let val = ctx.builder.build_load(llvm_pty, p, "pval").ok();
                    if let Some(v) = val {
                        emit_print_value(ctx, printf, *pty, v, false);
                    }
                }
            }
            
            let close = ctx.const_string(")");
            ctx.builder.build_call(printf, &[fmt_s.into(), close.into()], "").ok();
        }
        
        ctx.builder.build_unconditional_branch(merge_bb).ok();
    }
    
    ctx.builder.position_at_end(merge_bb);

fn load_len_i32<'ctx>(ctx: &mut CodegenContext<'ctx>, data_ptr: PointerValue<'ctx>) -> Option<IntValue<'ctx>> {
    let header_ptr = unsafe {
        ctx.builder.build_gep(
            ctx.i8_type(),
            data_ptr,
            &[ctx.i64_type().const_int((-8_i64) as u64, true)],
            "hdr_ptr",
        )
    }
    .ok()?;
    let len_ptr_i8 = unsafe {
        ctx.builder.build_gep(
            ctx.i8_type(),
            header_ptr,
            &[ctx.i64_type().const_int(4, false)],
            "len_ptr_i8",
        )
    }
    .ok()?;
    let len_ptr = ctx
        .builder
        .build_pointer_cast(
            len_ptr_i8,
            ctx.i32_type().ptr_type(AddressSpace::default()),
            "len_ptr",
        )
        .ok()?;
    Some(
        ctx.builder
            .build_load(ctx.i32_type(), len_ptr, "len")
            .ok()?
            .into_int_value(),
    )
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
    let Some(idx_alloca) = idx_alloca else { return; };
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
    let Some(idx) = idx else { return; };
    let cond = ctx
        .builder
        .build_int_compare(IntPredicate::ULT, idx, len_i64, "cond")
        .ok();
    let Some(cond) = cond else { return; };
    ctx.builder.build_conditional_branch(cond, body_bb, end_bb).ok();

    ctx.builder.position_at_end(body_bb);
    let need_comma = ctx
        .builder
        .build_int_compare(IntPredicate::UGT, idx, ctx.i64_type().const_zero(), "need_comma")
        .ok();
    if let Some(need_comma) = need_comma {
        let comma_bb = ctx.context.append_basic_block(current_fn, "print_arr_comma");
        let after_comma_bb = ctx.context.append_basic_block(current_fn, "print_arr_after_comma");
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
            emit_print_value(ctx, printf, elem_type, elem_val, false);
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
    let pair_ty = ctx.context.struct_type(&[key_llvm.into(), val_llvm.into()], false);
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
    let Some(idx_alloca) = idx_alloca else { return; };
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
    let Some(idx) = idx else { return; };
    let cond = ctx
        .builder
        .build_int_compare(IntPredicate::ULT, idx, len_i64, "cond")
        .ok();
    let Some(cond) = cond else { return; };
    ctx.builder.build_conditional_branch(cond, body_bb, end_bb).ok();

    ctx.builder.position_at_end(body_bb);
    let need_comma = ctx
        .builder
        .build_int_compare(IntPredicate::UGT, idx, ctx.i64_type().const_zero(), "need_comma")
        .ok();
    if let Some(need_comma) = need_comma {
        let comma_bb = ctx.context.append_basic_block(current_fn, "print_map_comma");
        let after_comma_bb = ctx.context.append_basic_block(current_fn, "print_map_after_comma");
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
        let kptr = ctx.builder.build_struct_gep(pair_ty, pair_ptr, 0, "kptr").ok();
        let vptr = ctx.builder.build_struct_gep(pair_ty, pair_ptr, 1, "vptr").ok();
        if let (Some(kptr), Some(vptr)) = (kptr, vptr) {
            let k = ctx.builder.build_load(key_llvm, kptr, "k").ok();
            let v = ctx.builder.build_load(val_llvm, vptr, "v").ok();
            if let (Some(k), Some(v)) = (k, v) {
                emit_print_value(ctx, printf, key_type, k, false);
                let sep = ctx.const_string(": ");
                ctx.builder
                    .build_call(printf, &[fmt.into(), sep.into()], "print_sep")
                    .ok();
                emit_print_value(ctx, printf, val_type, v, false);
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
