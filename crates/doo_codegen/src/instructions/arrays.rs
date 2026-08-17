//! Array Instruction Handler
//!
//! Handles: ArrayCreate, ArrayGet, ArraySet, ArrayLen, ArrayContains
//!
//! IMPORTANT: Array pointers in this module are DATA pointers, not header pointers.
//! The header (length/capacity) is stored at offset -16 from the data pointer.
//! Use `get_array_length_from_data` to access the length.

use super::InstructionHandler;
use crate::context::CodegenContext;
use crate::layout::{alloc_with_header, get_array_length_from_data, int_to_i64};
use crate::utils::{emit_eq, operand_to_value};
use doo_core::constants::ffi_names;
use doo_mir::sym::resolve;
use doo_mir::{MirInstr, MirInstrKind, MirOperand};
use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use inkwell::IntPredicate;

/// Array instruction handler.
pub struct ArrayHandler;

// ============================================================================
// Bounds Checking Helper
// ============================================================================

/// Emit runtime bounds check for array access.
/// Panics with an informative error message if index >= length.
/// Returns the continue block where execution continues after the check.
///
/// IMPORTANT: arr_ptr must be a DATA pointer (from alloc_with_header),
/// not a header pointer. This function uses the centralized layout helpers.
fn emit_bounds_check<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    arr_ptr: PointerValue<'ctx>,
    index: IntValue<'ctx>,
    _operation: &str, // "access" or "assignment" (for future error messages)
) -> Option<()> {
    use crate::layout::get_array_length_from_data;

    // Get array length using centralized layout helper
    // This correctly handles the header at offset -16 from data pointer
    let array_length = get_array_length_from_data(ctx, arr_ptr)?;

    // Truncate length to i32 for comparison
    let array_length_i32 = ctx
        .builder
        .build_int_truncate(array_length, ctx.i32_type(), "array_length_bounds")
        .ok()?;

    // Cast index to i32 for comparison (index is typically i64)
    let index_i32 = if index.get_type().get_bit_width() > 32 {
        ctx.builder
            .build_int_truncate(index, ctx.i32_type(), "idx_i32")
            .ok()?
    } else if index.get_type().get_bit_width() < 32 {
        ctx.builder
            .build_int_z_extend(index, ctx.i32_type(), "idx_i32")
            .ok()?
    } else {
        index
    };

    // Check if index >= length (unsigned comparison handles negative indices too)
    let is_out_of_bounds = ctx
        .builder
        .build_int_compare(
            IntPredicate::UGE,
            index_i32,
            array_length_i32,
            "is_out_of_bounds",
        )
        .ok()?;

    // Create blocks for bounds check
    let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
    let panic_block = ctx
        .context
        .append_basic_block(current_fn, "array_bounds_panic");
    let continue_block = ctx
        .context
        .append_basic_block(current_fn, "array_bounds_ok");

    ctx.builder
        .build_conditional_branch(is_out_of_bounds, panic_block, continue_block)
        .ok()?;

    // === Panic block: print error and exit ===
    ctx.builder.position_at_end(panic_block);

    // Get or declare printf function
    let printf_fn = ctx
        .module
        .get_function(ffi_names::PRINTF)
        .unwrap_or_else(|| {
            let printf_type = ctx.i32_type().fn_type(
                &[ctx.ptr_type().into()],
                true, // variadic
            );
            ctx.module
                .add_function(ffi_names::PRINTF, printf_type, None)
        });

    // Create error message format string
    let error_fmt = ctx
        .builder
        .build_global_string_ptr(
            "panic: array index out of bounds: index %d, length %d\n",
            "array_bounds_error_fmt",
        )
        .ok()?;

    ctx.builder
        .build_call(
            printf_fn,
            &[
                error_fmt.as_pointer_value().into(),
                index_i32.into(),
                array_length_i32.into(),
            ],
            "print_bounds_error",
        )
        .ok()?;

    // CRITICAL: Use __doo_abort() instead of exit() directly.
    // LLVM recognizes exit() as noreturn (built-in C library knowledge) and
    // uses this to prove bounds-check panic paths never return, which lets it
    // remove for-in loop exit conditions. __doo_abort is noinline+optnone so
    // LLVM can't see through it and can't infer noreturn.
    let abort_fn = ctx.get_or_create_doo_abort();
    ctx.builder
        .build_call(
            abort_fn,
            &[ctx.i32_type().const_int(1, false).into()],
            "abort_bounds",
        )
        .ok()?;

    // Branch to continue block. __doo_abort(1) terminates the process before
    // this executes, but LLVM sees the panic path as "returnable" and
    // preserves loop exit conditions.
    ctx.builder
        .build_unconditional_branch(continue_block)
        .ok()?;

    // === Continue block: proceed with array access ===
    ctx.builder.position_at_end(continue_block);

    Some(())
}

impl<'ctx> InstructionHandler<'ctx> for ArrayHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(
            instr.kind,
            MirInstrKind::ArrayCreate { .. }
                | MirInstrKind::ArrayGet { .. }
                | MirInstrKind::ArraySet { .. }
                | MirInstrKind::ArrayLen { .. }
                | MirInstrKind::ArrayContains { .. }
                | MirInstrKind::ArrayPush { .. }
                | MirInstrKind::ArrayExtend { .. }
                | MirInstrKind::ArraySlice { .. }
        )
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::ArrayCreate {
                dest,
                elements,
                elem_type,
            } => {
                if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {}
                let elem_llvm_ty = ctx.get_llvm_type(*elem_type);
                let len_i32 = ctx.i32_type().const_int(elements.len() as u64, false);
                let data_ptr = alloc_with_header(ctx, len_i32, elem_llvm_ty, "arr");
                if data_ptr.is_none()
                    && std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok()
                {
                    return None;
                }
                let data_ptr = data_ptr?;

                let elem_ptr_ty = ctx.ptr_type();
                let base = ctx
                    .builder
                    .build_pointer_cast(data_ptr, elem_ptr_ty, "arr_data_cast")
                    .ok()?;

                // Track element temp names for mixed-type array serialization
                let mut element_temp_names = Vec::new();

                for (i, elem) in elements.iter().enumerate() {
                    // Extract temp name for tracking
                    if let MirOperand::Temp(name) = elem {
                        element_temp_names.push(resolve(*name));
                    }

                    let Some(val) = operand_to_value(ctx, elem) else {
                        continue;
                    };
                    // For string arrays, clone constant string elements into heap memory.
                    // Static string constants (global pointers) cannot be safely freed,
                    // but array Drop frees all string elements. Cloning ensures consistency.
                    let store_val = if *elem_type == doo_core::types::builtin::STR
                        && matches!(elem, MirOperand::Const(doo_mir::MirConst::Str(_)))
                        && val.is_pointer_value()
                    {
                        if let Some(cloned) =
                            super::memory::clone_string(ctx, val.into_pointer_value())
                        {
                            cloned.into()
                        } else {
                            val
                        }
                    } else {
                        val
                    };
                    let idx = ctx.i64_type().const_int(i as u64, false);
                    let elem_ptr = unsafe {
                        ctx.builder
                            .build_gep(elem_llvm_ty, base, &[idx], "elem_ptr")
                    }
                    .ok()?;
                    ctx.builder.build_store(elem_ptr, store_val).ok();
                }

                ctx.set_temp(&resolve(*dest), data_ptr.into());

                // Track array element type for enum serialization in FFI calls
                ctx.array_element_types.insert(resolve(*dest), *elem_type);

                // Track element temp names for mixed-type arrays
                if !element_temp_names.is_empty() {
                    ctx.array_element_temps
                        .insert(resolve(*dest), element_temp_names);
                }

                Some(data_ptr.into())
            }

            MirInstrKind::ArrayGet {
                dest,
                array,
                index,
                elem_type,
            } => {
                let arr = operand_to_value(ctx, array)?;
                let idx = operand_to_value(ctx, index)?;
                if !arr.is_pointer_value() || !idx.is_int_value() {
                    return None;
                }

                let arr_ptr = arr.into_pointer_value();
                let idx_int = idx.into_int_value();

                // === BOUNDS CHECK ===
                emit_bounds_check(ctx, arr_ptr, idx_int, "access")?;

                let idx_i64 = int_to_i64(ctx, idx_int)?;
                let elem_llvm_ty = ctx.get_llvm_type(*elem_type);
                let elem_ptr_ty = ctx.ptr_type();
                let base = ctx
                    .builder
                    .build_pointer_cast(arr_ptr, elem_ptr_ty, "arr_data_cast")
                    .ok()?;

                let elem_ptr = unsafe {
                    ctx.builder
                        .build_gep(elem_llvm_ty, base, &[idx_i64], "elem_ptr")
                }
                .ok()?;
                let val = ctx
                    .builder
                    .build_load(elem_llvm_ty, elem_ptr, &resolve(*dest))
                    .ok()?;

                // CRITICAL: Deep-clone struct/string elements on access.
                // Array still owns the original, so accessing an element must produce
                // an independent copy to avoid double-free when the array is dropped.
                // This aligns with Doo's ownership model: auto-clone when variable reused.
                let val = match ctx.get_type_kind(*elem_type) {
                    Some(doo_core::types::TypeKind::Struct { def }) => {
                        if val.is_pointer_value() {
                            let field_pairs: Vec<_> = def
                                .fields
                                .iter()
                                .map(|f| (f.name.resolve().to_string(), f.type_id))
                                .collect();
                            let struct_name = def.name.resolve();
                            super::memory::clone_struct(
                                ctx,
                                val.into_pointer_value(),
                                struct_name,
                                &field_pairs,
                            )
                            .map(|p| p.into())
                            .unwrap_or(val)
                        } else {
                            val
                        }
                    }
                    Some(doo_core::types::TypeKind::Str) => {
                        if val.is_pointer_value() {
                            super::memory::clone_string(ctx, val.into_pointer_value())
                                .map(|p| -> BasicValueEnum { p.into() })
                                .unwrap_or(val)
                        } else {
                            val
                        }
                    }
                    _ => val, // Primitives (Int, Float, Bool) — copy is safe
                };

                ctx.set_temp(&resolve(*dest), val);
                // Set the type for the temp so Clone knows the correct element type
                ctx.set_variable_type(&resolve(*dest), *elem_type);

                // CRITICAL: If element type is a struct, propagate struct type info
                // This enables chained field access like user.name where user comes from array
                if let Some(struct_name) = ctx.get_struct_name_from_type_id(*elem_type) {
                    ctx.set_temp_struct_type(&resolve(*dest), &struct_name);
                }

                Some(val)
            }

            MirInstrKind::ArraySet {
                array,
                index,
                value,
                elem_type,
            } => {
                let arr = operand_to_value(ctx, array)?;
                let idx = operand_to_value(ctx, index)?;
                let val = operand_to_value(ctx, value)?;
                if !arr.is_pointer_value() || !idx.is_int_value() {
                    return None;
                }

                let arr_ptr = arr.into_pointer_value();
                let idx_int = idx.into_int_value();

                // === BOUNDS CHECK ===
                emit_bounds_check(ctx, arr_ptr, idx_int, "assignment")?;

                let idx_i64 = int_to_i64(ctx, idx_int)?;
                let elem_llvm_ty = ctx.get_llvm_type(*elem_type);
                let elem_ptr_ty = ctx.ptr_type();
                let base = ctx
                    .builder
                    .build_pointer_cast(arr_ptr, elem_ptr_ty, "arr_data_cast")
                    .ok()?;

                let elem_ptr = unsafe {
                    ctx.builder
                        .build_gep(elem_llvm_ty, base, &[idx_i64], "elem_ptr")
                }
                .ok()?;
                ctx.builder.build_store(elem_ptr, val).ok();
                None
            }

            MirInstrKind::ArrayLen { dest, array } => {
                let arr = operand_to_value(ctx, array)?;
                if !arr.is_pointer_value() {
                    return None;
                }
                let arr_ptr = arr.into_pointer_value();
                // arr_ptr is a DATA pointer, use the _from_data variant
                let len_i64 = get_array_length_from_data(ctx, arr_ptr)?;
                ctx.set_temp(&resolve(*dest), len_i64.into());
                Some(len_i64.into())
            }

            MirInstrKind::ArrayContains {
                dest,
                array,
                value,
                elem_type,
            } => {
                let arr = operand_to_value(ctx, array)?;
                let needle = operand_to_value(ctx, value)?;
                if !arr.is_pointer_value() {
                    return None;
                }

                let arr_ptr = arr.into_pointer_value();
                let elem_llvm_ty = ctx.get_llvm_type(*elem_type);
                let elem_ptr_ty = ctx.ptr_type();
                let base = ctx
                    .builder
                    .build_pointer_cast(arr_ptr, elem_ptr_ty, "arr_data_cast")
                    .ok()?;

                // arr_ptr is a DATA pointer, use _from_data variant
                let len_i64 = get_array_length_from_data(ctx, arr_ptr)?;

                let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
                let loop_bb = ctx
                    .context
                    .append_basic_block(current_fn, "arr_contains_loop");
                let check_bb = ctx
                    .context
                    .append_basic_block(current_fn, "arr_contains_check");
                let inc_bb = ctx
                    .context
                    .append_basic_block(current_fn, "arr_contains_inc");
                let found_bb = ctx
                    .context
                    .append_basic_block(current_fn, "arr_contains_found");
                let end_bb = ctx
                    .context
                    .append_basic_block(current_fn, "arr_contains_end");

                let idx_alloca = ctx.alloca_in_entry_block(ctx.i64_type(), "idx")?;
                ctx.builder
                    .build_store(idx_alloca, ctx.i64_type().const_zero())
                    .ok();

                let res_alloca = ctx.alloca_in_entry_block(ctx.bool_type(), "res")?;
                ctx.builder
                    .build_store(res_alloca, ctx.bool_type().const_zero())
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
                    .build_int_compare(IntPredicate::ULT, idx, len_i64, "cond")
                    .ok()?;
                ctx.builder
                    .build_conditional_branch(cond, check_bb, end_bb)
                    .ok()?;

                ctx.builder.position_at_end(check_bb);
                let elem_ptr = unsafe {
                    ctx.builder
                        .build_gep(elem_llvm_ty, base, &[idx], "elem_ptr")
                }
                .ok()?;
                let elem_val = ctx
                    .builder
                    .build_load(elem_llvm_ty, elem_ptr, "elem")
                    .ok()?;
                let is_eq = emit_eq(ctx, *elem_type, elem_val, needle)?;
                ctx.builder
                    .build_conditional_branch(is_eq, found_bb, inc_bb)
                    .ok()?;

                ctx.builder.position_at_end(inc_bb);
                let next = ctx
                    .builder
                    .build_int_add(idx, ctx.i64_type().const_int(1, false), "next")
                    .ok()?;
                ctx.builder.build_store(idx_alloca, next).ok();
                ctx.builder.build_unconditional_branch(loop_bb).ok()?;

                ctx.builder.position_at_end(found_bb);
                ctx.builder
                    .build_store(res_alloca, ctx.bool_type().const_int(1, false))
                    .ok();
                ctx.builder.build_unconditional_branch(end_bb).ok()?;

                ctx.builder.position_at_end(end_bb);
                let res = ctx
                    .builder
                    .build_load(ctx.bool_type(), res_alloca, &resolve(*dest))
                    .ok()?;
                ctx.set_temp(&resolve(*dest), res);
                Some(res)
            }

            MirInstrKind::ArrayPush { array, value } => {
                let arr_val = operand_to_value(ctx, array)?;
                let val = operand_to_value(ctx, value)?;
                if !arr_val.is_pointer_value() {
                    return None;
                }

                let old_data = arr_val.into_pointer_value();
                // old_data is a DATA pointer, use _from_data variant
                let len_i64 = get_array_length_from_data(ctx, old_data)?;

                // Calculate new length
                let new_len_i64 = ctx
                    .builder
                    .build_int_add(len_i64, ctx.i64_type().const_int(1, false), "new_len")
                    .ok()?;

                // Convert to i32 for realloc_array_capacity
                let new_len_i32 = ctx
                    .builder
                    .build_int_truncate(new_len_i64, ctx.i32_type(), "new_len_i32")
                    .ok()?;

                // Reallocate
                // Note: We need element type. MIR instruction doesn't provide it here unless we look up type of 'value'?
                // Or we assume 'value' type matches array element type.
                // We need to know element SIZE for realloc.
                let val_type = val.get_type();
                let _elem_size = val_type.size_of()?; // This might be wrong if val is pointer but array holds structs
                                                      // Better: rely on type info from a registry if available, but codegen usually works on LLVM types.
                                                      // Assuming homogeneous array, element type is type of 'val'.

                let elem_llvm_ty = val_type;
                let pair_size = elem_llvm_ty.size_of()?;

                // Realloc logic similar to MapSet
                use crate::layout::realloc_array_capacity;
                let new_data = realloc_array_capacity(ctx, old_data, new_len_i32, pair_size)?;

                // Store updated pointer back to 'array' operand location if it's a local/temp
                if let MirOperand::Local(name) | MirOperand::Temp(name) = array {
                    ctx.set_temp(&resolve(*name), new_data.into()); // Update SSA value mapping
                    if let Some(local_ptr) = ctx.get_local(&resolve(*name)) {
                        ctx.builder.build_store(local_ptr, new_data).ok();
                    }
                }

                // Append value
                let elem_ptr_ty = ctx.ptr_type();
                let base = ctx
                    .builder
                    .build_pointer_cast(new_data, elem_ptr_ty, "arr_new_cast")
                    .ok()?;
                // Use len_i64 directly as the index (position at old length = new element position)
                let elem_ptr = unsafe {
                    ctx.builder
                        .build_gep(elem_llvm_ty, base, &[len_i64], "elem_ptr")
                }
                .ok()?;
                ctx.builder.build_store(elem_ptr, val).ok();

                None
            }

            MirInstrKind::ArrayExtend {
                array,
                other,
                elem_type,
            } => {
                let arr1_val = operand_to_value(ctx, array)?;
                let arr2_val = operand_to_value(ctx, other)?;
                let arr1 = arr1_val.into_pointer_value();
                let arr2 = arr2_val.into_pointer_value();

                // arr1 and arr2 are DATA pointers, use _from_data variant
                let len1_i64 = get_array_length_from_data(ctx, arr1)?;
                let len2_i64 = get_array_length_from_data(ctx, arr2)?;

                let new_len_i64 = ctx
                    .builder
                    .build_int_add(len1_i64, len2_i64, "new_len")
                    .ok()?;
                let new_len_i32 = ctx
                    .builder
                    .build_int_truncate(new_len_i64, ctx.i32_type(), "new_len_i32")
                    .ok()?;

                let elem_llvm_ty = ctx.get_llvm_type(*elem_type);
                let pair_size = elem_llvm_ty.size_of()?;

                use crate::layout::realloc_array_capacity;
                let new_data = realloc_array_capacity(ctx, arr1, new_len_i32, pair_size)?;

                if let MirOperand::Local(name) | MirOperand::Temp(name) = array {
                    ctx.set_temp(&resolve(*name), new_data.into());
                    if let Some(local_ptr) = ctx.get_local(&resolve(*name)) {
                        ctx.builder.build_store(local_ptr, new_data).ok();
                    }
                }

                // Copy/Memcpy second array data to new space
                // Dest: new_data + len1 * stride
                let elem_ptr_ty = ctx.ptr_type();
                let base = ctx
                    .builder
                    .build_pointer_cast(new_data, elem_ptr_ty, "arr_base")
                    .ok()?;
                let dest_ptr = unsafe {
                    ctx.builder
                        .build_gep(elem_llvm_ty, base, &[len1_i64], "dest_ptr")
                }
                .ok()?;

                // Source: arr2
                let src_base = ctx
                    .builder
                    .build_pointer_cast(arr2, elem_ptr_ty, "src_base")
                    .ok()?;

                // Memcpy
                let copy_bytes = ctx
                    .builder
                    .build_int_mul(len2_i64, pair_size, "copy_bytes")
                    .ok()?;

                // Alignments? Assuming default
                ctx.builder
                    .build_memcpy(dest_ptr, 1, src_base, 1, copy_bytes)
                    .ok()?;

                None
            }

            MirInstrKind::ArraySlice {
                dest,
                array,
                start,
                end,
                elem_type,
            } => {
                let arr = operand_to_value(ctx, array)?.into_pointer_value();
                let start_val = operand_to_value(ctx, start)?.into_int_value();
                let end_val = operand_to_value(ctx, end)?.into_int_value();

                // Calculate length: end - start
                // Assuming bounds checked or handled elsewhere or user responsibly?
                // For safety, should clamp or check. But simplifying to raw slice logic.
                let len = ctx
                    .builder
                    .build_int_sub(end_val, start_val, "len_i64")
                    .ok()?;
                let len_i32 = ctx
                    .builder
                    .build_int_cast(len, ctx.i32_type(), "len_i32")
                    .ok()?;

                let elem_llvm_ty = ctx.get_llvm_type(*elem_type);
                // Create new array
                let new_data = alloc_with_header(ctx, len_i32, elem_llvm_ty, "slice")?;

                // Source pointer: arr + start
                let elem_ptr_ty = ctx.ptr_type();
                let src_base = ctx
                    .builder
                    .build_pointer_cast(arr, elem_ptr_ty, "src_base")
                    .ok()?;
                let start_idx = ctx
                    .builder
                    .build_int_cast(start_val, ctx.i64_type(), "start_idx")
                    .ok()?;
                let src_ptr = unsafe {
                    ctx.builder
                        .build_gep(elem_llvm_ty, src_base, &[start_idx], "src_ptr")
                }
                .ok()?;

                // Dest pointer
                let dest_base = ctx
                    .builder
                    .build_pointer_cast(new_data, elem_ptr_ty, "dest_base")
                    .ok()?;

                // Memcpy
                let pair_size = elem_llvm_ty.size_of()?;
                let copy_bytes = ctx
                    .builder
                    .build_int_mul(len, pair_size, "copy_bytes")
                    .ok()?;

                ctx.builder
                    .build_memcpy(dest_base, 1, src_ptr, 1, copy_bytes)
                    .ok()?;

                ctx.set_temp(&resolve(*dest), new_data.into());
                Some(new_data.into())
            }
            _ => None,
        }
    }
}
