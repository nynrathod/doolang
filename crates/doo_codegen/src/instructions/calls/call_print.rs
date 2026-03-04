//! Print instruction codegen — emit_print_value and related functions.

use crate::context::CodegenContext;
use crate::layout::load_len_i32;
use doo_core::constants::ffi_names;
use doo_core::doo_debug;
use doo_core::types::{builtin, TypeKind};
use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

/// Emit a call to `doo_format_float(f64) -> *mut c_char` then `printf("%s", result)`,
/// then `doo_free(result)`. Uses ryu for clean shortest-representation float formatting.
fn emit_print_float_ryu<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    float_val: BasicValueEnum<'ctx>,
    newline: bool,
) {
    let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
    let f64_ty = ctx.f64_type();

    // Get or declare doo_format_float(f64) -> *mut c_char
    let format_fn = ctx
        .module
        .get_function(ffi_names::DOO_FORMAT_FLOAT)
        .unwrap_or_else(|| {
            let fn_ty = ptr_ty.fn_type(&[f64_ty.into()], false);
            ctx.module
                .add_function(ffi_names::DOO_FORMAT_FLOAT, fn_ty, None)
        });

    // Get or declare doo_free(*mut c_char)
    let free_fn = ctx
        .module
        .get_function(ffi_names::DOO_FREE)
        .unwrap_or_else(|| {
            let fn_ty = ctx.context.void_type().fn_type(&[ptr_ty.into()], false);
            ctx.module
                .add_function(ffi_names::DOO_FREE, fn_ty, None)
        });

    // Call doo_format_float(val) → str_ptr
    if let Some(str_ptr) = ctx
        .builder
        .build_call(format_fn, &[float_val.into()], "fmt_float")
        .ok()
        .and_then(|v| v.try_as_basic_value().basic())
    {
        // printf("%s" or "%s\n", str_ptr)
        let fmt = if newline { "%s\n" } else { "%s" };
        let fmt = ctx.const_string(fmt);
        ctx.builder
            .build_call(printf, &[fmt.into(), str_ptr.into()], "print_f")
            .ok();

        // doo_free(str_ptr)
        ctx.builder
            .build_call(free_fn, &[str_ptr.into()], "free_fmt")
            .ok();
    }
}

pub(super) fn emit_print_value<'ctx>(
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
            // Float - print using ryu for clean formatting
            emit_print_float_ryu(ctx, printf, val, newline);
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
                let close_quote = if newline {
                    ctx.const_string("\"\n")
                } else {
                    ctx.const_string("\"")
                };
                let fmt = ctx.const_string("%s");
                ctx.builder
                    .build_call(printf, &[fmt.into(), open_quote.into()], "print_quote_open")
                    .ok();
                ctx.builder
                    .build_call(printf, &[fmt.into(), val.into()], "print_str")
                    .ok();
                ctx.builder
                    .build_call(
                        printf,
                        &[fmt.into(), close_quote.into()],
                        "print_quote_close",
                    )
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
        } else if val.is_pointer_value() {
            // Pointer holding a bool (e.g., from ManualErrorExtract) — convert ptr→i64→i1
            let i64_val = ctx
                .builder
                .build_ptr_to_int(val.into_pointer_value(), ctx.i64_type(), "ptr_to_i64")
                .ok();
            if let Some(i64_val) = i64_val {
                let is_true = ctx
                    .builder
                    .build_int_compare(
                        IntPredicate::NE,
                        i64_val,
                        ctx.i64_type().const_zero(),
                        "is_true",
                    )
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
        }
        return;
    }

    if type_id == builtin::FLOAT {
        if val.is_float_value() {
            emit_print_float_ryu(ctx, printf, val, newline);
        } else if val.is_pointer_value() {
            // Pointer holding a float (e.g., from ManualErrorExtract) — convert ptr→i64→f64
            let i64_val = ctx
                .builder
                .build_ptr_to_int(val.into_pointer_value(), ctx.i64_type(), "ptr_to_i64")
                .ok();
            if let Some(i64_val) = i64_val {
                let tmp = ctx.alloca_in_entry_block(ctx.i64_type(), "f_tmp");
                if let Some(tmp) = tmp {
                    ctx.builder.build_store(tmp, i64_val).ok();
                    let f_ptr = ctx
                        .builder
                        .build_pointer_cast(
                            tmp,
                            ctx.context.ptr_type(inkwell::AddressSpace::default()),
                            "f_ptr",
                        )
                        .ok();
                    if let Some(f_ptr) = f_ptr {
                        let f_val = ctx.builder.build_load(ctx.f64_type(), f_ptr, "f_val").ok();
                        if let Some(f_val) = f_val {
                            emit_print_float_ryu(ctx, printf, f_val, newline);
                        }
                    }
                }
            }
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
                let result = ctx
                    .builder
                    .build_call(printf, &[fmt.into(), i64v.into()], "print_i");
                if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                    let blk = ctx
                        .builder
                        .get_insert_block()
                        .map(|b| b.get_name().to_string_lossy().to_string());
                    doo_debug!(
                        "CODEGEN",
                        "emit_print_value INT in block {:?}, call result: {:?}",
                        blk,
                        result.is_ok()
                    );
                }
                result.ok();
            } else if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                doo_debug!("CODEGEN", "emit_print_value INT: i64 extend failed");
            }
        } else if val.is_pointer_value() {
            // Pointer holding an int (e.g., from ManualErrorExtract) — convert ptr→i64
            let i64v = ctx
                .builder
                .build_ptr_to_int(val.into_pointer_value(), ctx.i64_type(), "ptr_to_int")
                .ok();
            if let Some(i64v) = i64v {
                let fmt = if newline { "%lld\n" } else { "%lld" };
                let fmt = ctx.const_string(fmt);
                ctx.builder
                    .build_call(printf, &[fmt.into(), i64v.into()], "print_i")
                    .ok();
            }
        } else if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
            doo_debug!(
                "CODEGEN",
                "emit_print_value INT: val is not int, is {:?}",
                val.get_type()
            );
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
                    // Extract just name and type for printing (visibility not needed)
                    let field_pairs: Vec<_> =
                        fields.iter().map(|(n, t, _)| (n.clone(), *t)).collect();
                    emit_print_struct(ctx, printf, ptr, &name, &field_pairs);
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

pub(super) fn emit_print_tuple<'ctx>(
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

pub(super) fn emit_print_struct<'ctx>(
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

pub(super) fn emit_print_enum<'ctx>(
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
pub(super) fn emit_print_enum_value<'ctx>(
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

pub(super) fn emit_print_array<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    array_ptr: PointerValue<'ctx>,
    elem_type: doo_core::types::TypeId,
) {
    let fmt = ctx.const_string("%s");

    // Handle null array pointer (print "nil" instead of crashing)
    let is_null = ctx.builder.build_is_null(array_ptr, "arr_is_null").ok();
    if let Some(is_null) = is_null {
        let current_fn = match ctx.builder.get_insert_block().and_then(|b| b.get_parent()) {
            Some(f) => f,
            None => return,
        };

        let print_nil_bb = ctx.context.append_basic_block(current_fn, "print_arr_nil");
        let print_arr_bb = ctx
            .context
            .append_basic_block(current_fn, "print_arr_content");
        let merge_bb = ctx.context.append_basic_block(current_fn, "print_arr_done");

        ctx.builder
            .build_conditional_branch(is_null, print_nil_bb, print_arr_bb)
            .ok();

        // Print "nil" for null arrays
        ctx.builder.position_at_end(print_nil_bb);
        let nil_str = ctx.const_string("nil");
        ctx.builder
            .build_call(printf, &[fmt.into(), nil_str.into()], "print_nil")
            .ok();
        ctx.builder.build_unconditional_branch(merge_bb).ok();

        // Print actual array contents
        ctx.builder.position_at_end(print_arr_bb);
        emit_print_array_contents(ctx, printf, array_ptr, elem_type, merge_bb);

        ctx.builder.position_at_end(merge_bb);
    } else {
        // Fallback: just print array contents without null check
        let current_fn = match ctx.builder.get_insert_block().and_then(|b| b.get_parent()) {
            Some(f) => f,
            None => return,
        };
        let merge_bb = ctx.context.append_basic_block(current_fn, "print_arr_done");
        emit_print_array_contents(ctx, printf, array_ptr, elem_type, merge_bb);
        ctx.builder.position_at_end(merge_bb);
    }
}

/// Internal helper to print array contents (assumes array_ptr is not null)
fn emit_print_array_contents<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    array_ptr: PointerValue<'ctx>,
    elem_type: doo_core::types::TypeId,
    merge_bb: inkwell::basic_block::BasicBlock<'ctx>,
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
        ctx.builder.build_unconditional_branch(merge_bb).ok();
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
        ctx.builder.build_unconditional_branch(merge_bb).ok();
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
        ctx.builder.build_unconditional_branch(merge_bb).ok();
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

    let idx_alloca = ctx.alloca_in_entry_block(ctx.i64_type(), "idx");
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
    ctx.builder.build_unconditional_branch(merge_bb).ok();
}

pub(super) fn emit_print_map<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    map_ptr: PointerValue<'ctx>,
    key_type: doo_core::types::TypeId,
    val_type: doo_core::types::TypeId,
) {
    let fmt = ctx.const_string("%s");

    // Handle null map pointer (print "nil" instead of crashing)
    let is_null = ctx.builder.build_is_null(map_ptr, "map_is_null").ok();
    if let Some(is_null) = is_null {
        let current_fn = match ctx.builder.get_insert_block().and_then(|b| b.get_parent()) {
            Some(f) => f,
            None => return,
        };

        let print_nil_bb = ctx.context.append_basic_block(current_fn, "print_map_nil");
        let print_map_bb = ctx
            .context
            .append_basic_block(current_fn, "print_map_content");
        let merge_bb = ctx.context.append_basic_block(current_fn, "print_map_done");

        ctx.builder
            .build_conditional_branch(is_null, print_nil_bb, print_map_bb)
            .ok();

        // Print "nil" for null maps
        ctx.builder.position_at_end(print_nil_bb);
        let nil_str = ctx.const_string("nil");
        ctx.builder
            .build_call(printf, &[fmt.into(), nil_str.into()], "print_nil")
            .ok();
        ctx.builder.build_unconditional_branch(merge_bb).ok();

        // Print actual map contents
        ctx.builder.position_at_end(print_map_bb);
        emit_print_map_contents(ctx, printf, map_ptr, key_type, val_type, merge_bb);

        ctx.builder.position_at_end(merge_bb);
    } else {
        // Fallback: just print map contents without null check
        let current_fn = match ctx.builder.get_insert_block().and_then(|b| b.get_parent()) {
            Some(f) => f,
            None => return,
        };
        let merge_bb = ctx.context.append_basic_block(current_fn, "print_map_done");
        emit_print_map_contents(ctx, printf, map_ptr, key_type, val_type, merge_bb);
        ctx.builder.position_at_end(merge_bb);
    }
}

/// Internal helper to print map contents (assumes map_ptr is not null)
fn emit_print_map_contents<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    printf: FunctionValue<'ctx>,
    map_ptr: PointerValue<'ctx>,
    key_type: doo_core::types::TypeId,
    val_type: doo_core::types::TypeId,
    merge_bb: inkwell::basic_block::BasicBlock<'ctx>,
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
        ctx.builder.build_unconditional_branch(merge_bb).ok();
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
        ctx.builder.build_unconditional_branch(merge_bb).ok();
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
        ctx.builder.build_unconditional_branch(merge_bb).ok();
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

    let idx_alloca = ctx.alloca_in_entry_block(ctx.i64_type(), "idx");
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
    ctx.builder.build_unconditional_branch(merge_bb).ok();
}
