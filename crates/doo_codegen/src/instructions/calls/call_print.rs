//! Print instruction handler — formats values and emits doo_print calls.
//!
//! Converts each value to a string representation based on its type, then
//! calls the doo_print FFI function. For structs, a debug representation
//! is generated at compile time from the struct's field layout.

use crate::context::CodegenContext;
use doo_core::constants::ffi_names;
use doo_core::types::{builtin, TypeId, TypeKind};
use doo_mir::sym::resolve;
use doo_mir::MirOperand;
use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::IntPredicate;

use super::call_utils::operand_to_value;

/// Emit print: format each argument to a string and call doo_print.
pub(crate) fn emit_print<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    args: &[MirOperand],
) -> Option<BasicValueEnum<'ctx>> {
    let printf = get_or_declare_printf(ctx);
    let malloc = get_or_declare_malloc(ctx);
    let strlen = get_or_declare_strlen(ctx);
    let memcpy = get_or_declare_memcpy(ctx);

    let ptr_type = ctx.ptr_type();
    let i64_type = ctx.i64_type();
    let i32_type = ctx.i32_type();

    // Build a combined string from all arguments
    let mut combined_parts: Vec<PointerValue<'ctx>> = Vec::new();

    for (i, arg) in args.iter().enumerate() {
        let val = operand_to_value(ctx, arg)?;
        let type_id = match arg {
            MirOperand::Local(name) | MirOperand::Temp(name) | MirOperand::Global(name) => {
                ctx.get_variable_type(&resolve(*name))
            }
            _ => None,
        };

        let str_ptr = format_value_to_string(ctx, val, type_id)?;
        combined_parts.push(str_ptr);

        // Add space separator between arguments
        if i + 1 < args.len() {
            let space = ctx
                .builder
                .build_global_string_ptr(" ", "print_sep")
                .ok()
                .map(|g| g.as_pointer_value())
                .unwrap_or_else(|| ptr_type.const_null());
            combined_parts.push(space);
        }
    }

    // Add newline at the end
    let newline = ctx
        .builder
        .build_global_string_ptr("\n", "print_newline")
        .ok()
        .map(|g| g.as_pointer_value())
        .unwrap_or_else(|| ptr_type.const_null());
    combined_parts.push(newline);

    // Concatenate all parts
    if combined_parts.is_empty() {
        return None;
    }

    // Calculate total length
    let mut total_len = i64_type.const_zero();
    for part in &combined_parts {
        let len = ctx
            .builder
            .build_call(strlen, &[(*part).into()], "part_len")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();
        total_len = ctx
            .builder
            .build_int_add(total_len, len, "total_len")
            .ok()?;
    }

    let size = ctx
        .builder
        .build_int_add(total_len, i64_type.const_int(1, false), "alloc_size")
        .ok()?;

    let result = ctx
        .builder
        .build_call(malloc, &[size.into()], "print_buf")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_pointer_value();

    // Copy each part into the buffer
    let mut offset = i64_type.const_zero();
    for part in &combined_parts {
        let part_len = ctx
            .builder
            .build_call(strlen, &[(*part).into()], "plen")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();

        let dst = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), result, &[offset], "print_dst")
                .ok()?
        };

        ctx.builder
            .build_call(memcpy, &[dst.into(), (*part).into(), part_len.into()], "")
            .ok()?;

        offset = ctx
            .builder
            .build_int_add(offset, part_len, "print_off")
            .ok()?;
    }

    // Null terminate
    let null_ptr = unsafe {
        ctx.builder
            .build_gep(ctx.context.i8_type(), result, &[offset], "print_null")
            .ok()?
    };
    ctx.builder
        .build_store(null_ptr, ctx.context.i8_type().const_zero())
        .ok()?;

    // Call printf("%s\n", result) or doo_print(result)
    let fmt = ctx.const_string("%s");
    let _ = ctx
        .builder
        .build_call(printf, &[fmt.into(), result.into()], "print_call");

    None
}

/// Format any value to a string pointer based on its type.
pub(crate) fn format_value_to_string<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
    type_id: Option<TypeId>,
) -> Option<PointerValue<'ctx>> {
    // If it's already a pointer (string), return directly
    if val.is_pointer_value() {
        // Check if we have type info to distinguish strings from other pointers
        if let Some(tid) = type_id {
            if let Some(kind) = ctx.get_type_kind(tid) {
                match kind {
                    TypeKind::Str => {
                        return Some(val.into_pointer_value());
                    }
                    TypeKind::Array { element } => {
                        return format_array_to_string(ctx, val.into_pointer_value(), element);
                    }
                    TypeKind::Struct { def } => {
                        let name = def.name.resolve().to_string();
                        let field_pairs: Vec<_> = def
                            .fields
                            .iter()
                            .map(|f| (f.name.resolve().to_string(), f.type_id))
                            .collect();
                        return format_struct_to_string(
                            ctx,
                            val.into_pointer_value(),
                            &name,
                            &field_pairs,
                        );
                    }
                    TypeKind::Bool => {
                        return format_bool_to_string(ctx, val);
                    }
                    _ => {}
                }
            }
        }
        // Default: assume it's a string pointer
        return Some(val.into_pointer_value());
    }

    // Floats: use doo_format_float (ryu) for clean formatting
    if val.is_float_value() {
        let ptr_type = ctx.ptr_type();
        let f64_type = ctx.f64_type();
        let format_fn = ctx
            .module
            .get_function(ffi_names::DOO_FORMAT_FLOAT)
            .unwrap_or_else(|| {
                let fn_ty = ptr_type.fn_type(&[f64_type.into()], false);
                ctx.module
                    .add_function(ffi_names::DOO_FORMAT_FLOAT, fn_ty, None)
            });
        let result = ctx
            .builder
            .build_call(format_fn, &[val.into()], "fmt_float")
            .ok()?
            .try_as_basic_value()
            .basic()?;
        return Some(result.into_pointer_value());
    }

    // Integers and booleans: use sprintf
    if val.is_int_value() {
        let int_val = val.into_int_value();
        let bit_width = int_val.get_type().get_bit_width();

        // Check if it's a boolean (i1 or i8)
        if let Some(tid) = type_id {
            if tid == builtin::BOOL || bit_width <= 8 {
                if let Some(kind) = ctx.get_type_kind(tid) {
                    if matches!(kind, TypeKind::Bool) {
                        return format_bool_to_string(ctx, val);
                    }
                }
            }
        }

        return format_int_to_string(ctx, val);
    }

    None
}

/// Format an integer to a string using sprintf.
fn format_int_to_string<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
) -> Option<PointerValue<'ctx>> {
    let sprintf = get_or_declare_sprintf(ctx);
    let malloc = get_or_declare_malloc(ctx);
    let ptr_type = ctx.ptr_type();
    let i64_type = ctx.i64_type();

    let fmt = ctx.const_string("%lld");
    let buffer = ctx
        .builder
        .build_array_alloca(
            ctx.context.i8_type(),
            i64_type.const_int(24, false),
            "int_buf",
        )
        .ok()?;

    ctx.builder
        .build_call(
            sprintf,
            &[buffer.into(), fmt.into(), val.into()],
            "sprintf_int",
        )
        .ok();

    // Copy to heap so the caller can use it safely
    let strlen = get_or_declare_strlen(ctx);
    let memcpy = get_or_declare_memcpy(ctx);

    let len = ctx
        .builder
        .build_call(strlen, &[buffer.into()], "int_len")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_int_value();

    let size = ctx
        .builder
        .build_int_add(len, i64_type.const_int(1, false), "int_size")
        .ok()?;

    let heap_str = ctx
        .builder
        .build_call(malloc, &[size.into()], "int_str")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_pointer_value();

    ctx.builder
        .build_call(memcpy, &[heap_str.into(), buffer.into(), len.into()], "")
        .ok()?;

    let null_pos = unsafe {
        ctx.builder
            .build_gep(ctx.context.i8_type(), heap_str, &[len], "int_null")
            .ok()?
    };
    ctx.builder
        .build_store(null_pos, ctx.context.i8_type().const_zero())
        .ok()?;

    Some(heap_str)
}

/// Format a boolean to "true" or "false".
fn format_bool_to_string<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
) -> Option<PointerValue<'ctx>> {
    let int_val = val.into_int_value();
    let zero = int_val.get_type().const_zero();
    let is_true = ctx
        .builder
        .build_int_compare(IntPredicate::NE, int_val, zero, "bool_check")
        .ok()?;

    let true_str = ctx
        .builder
        .build_global_string_ptr("true", "bool_true")
        .ok()
        .map(|g| g.as_pointer_value())
        .unwrap_or_else(|| ctx.ptr_type().const_null());

    let false_str = ctx
        .builder
        .build_global_string_ptr("false", "bool_false")
        .ok()
        .map(|g| g.as_pointer_value())
        .unwrap_or_else(|| ctx.ptr_type().const_null());

    let result = ctx
        .builder
        .build_select(is_true, true_str, false_str, "bool_str")
        .ok()?;

    Some(result.into_pointer_value())
}

/// Format an array to a debug string: [elem1, elem2, ...]
fn format_array_to_string<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    arr_ptr: PointerValue<'ctx>,
    elem_type: TypeId,
) -> Option<PointerValue<'ctx>> {
    use crate::layout::load_len_i32;

    let len_i32 = load_len_i32(ctx, arr_ptr)?;
    let len_i64 = ctx
        .builder
        .build_int_z_extend(len_i32, ctx.i64_type(), "arr_len")
        .ok()?;

    let elem_llvm = ctx.get_llvm_type(elem_type);
    let printf = get_or_declare_printf(ctx);
    let malloc = get_or_declare_malloc(ctx);
    let strlen = get_or_declare_strlen(ctx);
    let memcpy = get_or_declare_memcpy(ctx);

    // Start with "["
    let mut parts: Vec<PointerValue<'ctx>> = Vec::new();
    parts.push(
        ctx.builder
            .build_global_string_ptr("[", "arr_open")
            .ok()
            .map(|g| g.as_pointer_value())
            .unwrap_or_else(|| ctx.ptr_type().const_null()),
    );

    let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
    let loop_bb = ctx.context.append_basic_block(current_fn, "arr_fmt_loop");
    let body_bb = ctx.context.append_basic_block(current_fn, "arr_fmt_body");
    let end_bb = ctx.context.append_basic_block(current_fn, "arr_fmt_end");

    let idx_alloca = ctx.alloca_in_entry_block(ctx.i64_type(), "arr_idx")?;
    ctx.builder
        .build_store(idx_alloca, ctx.i64_type().const_zero())
        .ok();
    ctx.builder.build_unconditional_branch(loop_bb).ok()?;

    ctx.builder.position_at_end(loop_bb);
    let idx = ctx
        .builder
        .build_load(ctx.i64_type(), idx_alloca, "idx")
        .ok()?
        .into_int_value();
    let cond = ctx
        .builder
        .build_int_compare(IntPredicate::ULT, idx, len_i64, "arr_cond")
        .ok()?;
    ctx.builder
        .build_conditional_branch(cond, body_bb, end_bb)
        .ok()?;

    ctx.builder.position_at_end(body_bb);

    // Add comma separator after first element
    let first_iter = ctx
        .builder
        .build_int_compare(
            IntPredicate::EQ,
            idx,
            ctx.i64_type().const_zero(),
            "is_first",
        )
        .ok()?;

    let comma_str = ctx
        .builder
        .build_global_string_ptr(", ", "arr_sep")
        .ok()
        .map(|g| g.as_pointer_value())
        .unwrap_or_else(|| ctx.ptr_type().const_null());

    // For now, just print element pointers (simplified)
    // A full implementation would recursively format each element
    let elem_ptr = unsafe {
        ctx.builder
            .build_gep(elem_llvm, arr_ptr, &[idx], "elem_ptr")
            .ok()?
    };
    let elem_val = ctx.builder.build_load(elem_llvm, elem_ptr, "elem").ok()?;
    let elem_str = format_value_to_string(ctx, elem_val, Some(elem_type))?;
    parts.push(elem_str);

    let next = ctx
        .builder
        .build_int_add(idx, ctx.i64_type().const_int(1, false), "next")
        .ok()?;
    ctx.builder.build_store(idx_alloca, next).ok()?;
    ctx.builder.build_unconditional_branch(loop_bb).ok()?;

    ctx.builder.position_at_end(end_bb);

    // Add "]"
    parts.push(
        ctx.builder
            .build_global_string_ptr("]", "arr_close")
            .ok()
            .map(|g| g.as_pointer_value())
            .unwrap_or_else(|| ctx.ptr_type().const_null()),
    );

    // Concatenate all parts
    let mut total_len = ctx.i64_type().const_zero();
    for part in &parts {
        let len = ctx
            .builder
            .build_call(strlen, &[(*part).into()], "plen")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();
        total_len = ctx.builder.build_int_add(total_len, len, "total").ok()?;
    }

    let size = ctx
        .builder
        .build_int_add(total_len, ctx.i64_type().const_int(1, false), "size")
        .ok()?;

    let result = ctx
        .builder
        .build_call(malloc, &[size.into()], "arr_str")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_pointer_value();

    let mut offset = ctx.i64_type().const_zero();
    for part in &parts {
        let len = ctx
            .builder
            .build_call(strlen, &[(*part).into()], "plen")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();

        let dst = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), result, &[offset], "dst")
                .ok()?
        };

        ctx.builder
            .build_call(memcpy, &[dst.into(), (*part).into(), len.into()], "")
            .ok()?;

        offset = ctx.builder.build_int_add(offset, len, "off").ok()?;
    }

    let null_pos = unsafe {
        ctx.builder
            .build_gep(ctx.context.i8_type(), result, &[offset], "null")
            .ok()?
    };
    ctx.builder
        .build_store(null_pos, ctx.context.i8_type().const_zero())
        .ok()?;

    Some(result)
}

/// Format a struct to a debug string: StructName { field1: val1, field2: val2 }
fn format_struct_to_string<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    struct_ptr: PointerValue<'ctx>,
    struct_name: &str,
    fields: &[(String, TypeId)],
) -> Option<PointerValue<'ctx>> {
    let printf = get_or_declare_printf(ctx);
    let malloc = get_or_declare_malloc(ctx);
    let strlen = get_or_declare_strlen(ctx);
    let memcpy = get_or_declare_memcpy(ctx);

    // Build: "StructName { field1: val1, field2: val2 }"
    let mut parts: Vec<PointerValue<'ctx>> = Vec::new();

    // Prefix: "StructName { "
    let prefix = format!("{} {{ ", struct_name);
    parts.push(
        ctx.builder
            .build_global_string_ptr(&prefix, "struct_prefix")
            .ok()
            .map(|g| g.as_pointer_value())
            .unwrap_or_else(|| ctx.ptr_type().const_null()),
    );

    let struct_type = ctx.get_or_build_struct_type(struct_name)?;

    for (i, (field_name, field_type_id)) in fields.iter().enumerate() {
        if i > 0 {
            parts.push(
                ctx.builder
                    .build_global_string_ptr(", ", "struct_sep")
                    .ok()
                    .map(|g| g.as_pointer_value())
                    .unwrap_or_else(|| ctx.ptr_type().const_null()),
            );
        }

        // Field name + ": "
        let field_prefix = format!("{}: ", field_name);
        parts.push(
            ctx.builder
                .build_global_string_ptr(&field_prefix, "field_name")
                .ok()
                .map(|g| g.as_pointer_value())
                .unwrap_or_else(|| ctx.ptr_type().const_null()),
        );

        // Field value
        let physical_i = ctx.physical_field_index(struct_name, i);
        let field_ptr = ctx
            .builder
            .build_struct_gep(struct_type, struct_ptr, physical_i as u32, "field_ptr")
            .ok()?;

        let field_llvm = ctx.get_llvm_type(*field_type_id);
        let field_val = ctx
            .builder
            .build_load(field_llvm, field_ptr, "field_val")
            .ok()?;

        let field_str = format_value_to_string(ctx, field_val, Some(*field_type_id))?;
        parts.push(field_str);
    }

    // Suffix: " }"
    parts.push(
        ctx.builder
            .build_global_string_ptr(" }", "struct_suffix")
            .ok()
            .map(|g| g.as_pointer_value())
            .unwrap_or_else(|| ctx.ptr_type().const_null()),
    );

    // Concatenate all parts
    let mut total_len = ctx.i64_type().const_zero();
    for part in &parts {
        let len = ctx
            .builder
            .build_call(strlen, &[(*part).into()], "plen")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();
        total_len = ctx.builder.build_int_add(total_len, len, "total").ok()?;
    }

    let size = ctx
        .builder
        .build_int_add(total_len, ctx.i64_type().const_int(1, false), "size")
        .ok()?;

    let result = ctx
        .builder
        .build_call(malloc, &[size.into()], "struct_str")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_pointer_value();

    let mut offset = ctx.i64_type().const_zero();
    for part in &parts {
        let len = ctx
            .builder
            .build_call(strlen, &[(*part).into()], "plen")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();

        let dst = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), result, &[offset], "dst")
                .ok()?
        };

        ctx.builder
            .build_call(memcpy, &[dst.into(), (*part).into(), len.into()], "")
            .ok()?;

        offset = ctx.builder.build_int_add(offset, len, "off").ok()?;
    }

    let null_pos = unsafe {
        ctx.builder
            .build_gep(ctx.context.i8_type(), result, &[offset], "null")
            .ok()?
    };
    ctx.builder
        .build_store(null_pos, ctx.context.i8_type().const_zero())
        .ok()?;

    Some(result)
}

// ============================================================================
// libc Function Declarations
// ============================================================================

fn get_or_declare_printf<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    ctx.module
        .get_function(ffi_names::PRINTF)
        .unwrap_or_else(|| {
            let fn_type = ctx.i32_type().fn_type(&[ctx.ptr_type().into()], true);
            ctx.module.add_function(ffi_names::PRINTF, fn_type, None)
        })
}

fn get_or_declare_malloc<'ctx>(ctx: &CodegenContext<'ctx>) -> inkwell::values::FunctionValue<'ctx> {
    ctx.module
        .get_function(ffi_names::MALLOC)
        .unwrap_or_else(|| {
            let ptr_type = ctx.ptr_type();
            let fn_type = ptr_type.fn_type(&[ctx.i64_type().into()], false);
            ctx.module.add_function(ffi_names::MALLOC, fn_type, None)
        })
}

fn get_or_declare_strlen<'ctx>(ctx: &CodegenContext<'ctx>) -> inkwell::values::FunctionValue<'ctx> {
    ctx.module
        .get_function(ffi_names::STRLEN)
        .unwrap_or_else(|| {
            let fn_type = ctx.i64_type().fn_type(&[ctx.ptr_type().into()], false);
            ctx.module.add_function(ffi_names::STRLEN, fn_type, None)
        })
}

fn get_or_declare_memcpy<'ctx>(ctx: &CodegenContext<'ctx>) -> inkwell::values::FunctionValue<'ctx> {
    ctx.module
        .get_function(ffi_names::MEMCPY)
        .unwrap_or_else(|| {
            let ptr_type = ctx.ptr_type();
            let fn_type = ptr_type.fn_type(
                &[ptr_type.into(), ptr_type.into(), ctx.i64_type().into()],
                false,
            );
            ctx.module.add_function(ffi_names::MEMCPY, fn_type, None)
        })
}

fn get_or_declare_sprintf<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    ctx.module
        .get_function(ffi_names::SPRINTF)
        .unwrap_or_else(|| {
            let i32_type = ctx.i32_type();
            let ptr_type = ctx.ptr_type();
            let fn_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], true);
            ctx.module.add_function(ffi_names::SPRINTF, fn_type, None)
        })
}
