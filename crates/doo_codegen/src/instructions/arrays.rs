//! Array Instruction Handler
//!
//! Handles: ArrayCreate, ArrayGet, ArraySet, ArrayLen, ArrayContains
//!
//! IMPORTANT: Array pointers in this module are DATA pointers, not header pointers.
//! The header (length/capacity) is stored at offset -16 from the data pointer.
//! Use `get_array_length_from_data` to access the length.
//!
//! Architecture note: ArrayGet/ArraySet handle `arr[i]` indexing — this is a
//! language feature (like Rust's Index trait → GEP + load). Array methods like
//! len(), push() should eventually be in library/std/Array.doo.

use super::InstructionHandler;
use crate::context::CodegenContext;
use crate::layout::{alloc_with_header, get_array_length_from_data, int_to_i64};
use crate::utils::operand_to_value;
use doo_core::constants::ffi_names;
use doo_core::types::TypeKind;
use doo_mir::sym::resolve;
use doo_mir::{MirInstr, MirInstrKind, MirOperand};
use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

/// Array instruction handler.
pub struct ArrayHandler;

// ============================================================================
// i64 → Pointer Conversion Helper
// ============================================================================

/// Convert a value to a pointer, handling the case where arrays are stored
/// as i64 integers due to type coercion in set_local.
fn value_to_array_ptr<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
    name: &str,
) -> Option<PointerValue<'ctx>> {
    if val.is_pointer_value() {
        Some(val.into_pointer_value())
    } else if val.is_int_value() {
        eprintln!("[ARRAY-DEBUG] {} stored as i64, converting to ptr", name);
        ctx.builder
            .build_int_to_ptr(
                val.into_int_value(),
                ctx.context.i8_type().ptr_type(AddressSpace::default()),
                &format!("{}_inttoptr", name),
            )
            .ok()
    } else {
        eprintln!(
            "[ARRAY-DEBUG] {} is neither pointer nor int — FAILING",
            name
        );
        None
    }
}

// ============================================================================
// Bounds Checking Helper
// ============================================================================

fn emit_bounds_check<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    arr_ptr: PointerValue<'ctx>,
    index: IntValue<'ctx>,
    _operation: &str,
) -> Option<()> {
    use crate::layout::get_array_length_from_data;

    let array_length = get_array_length_from_data(ctx, arr_ptr)?;

    let array_length_i32 = ctx
        .builder
        .build_int_truncate(array_length, ctx.i32_type(), "array_length_bounds")
        .ok()?;

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

    let is_out_of_bounds = ctx
        .builder
        .build_int_compare(
            IntPredicate::UGE,
            index_i32,
            array_length_i32,
            "is_out_of_bounds",
        )
        .ok()?;

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

    // === Panic block ===
    ctx.builder.position_at_end(panic_block);

    let printf_fn = ctx
        .module
        .get_function(ffi_names::PRINTF)
        .unwrap_or_else(|| {
            let printf_type = ctx.i32_type().fn_type(&[ctx.ptr_type().into()], true);
            ctx.module
                .add_function(ffi_names::PRINTF, printf_type, None)
        });

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

    let abort_fn = ctx.get_or_create_doo_abort();
    ctx.builder
        .build_call(
            abort_fn,
            &[ctx.i32_type().const_int(1, false).into()],
            "abort_bounds",
        )
        .ok()?;

    ctx.builder
        .build_unconditional_branch(continue_block)
        .ok()?;

    // === Continue block ===
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
            // ================================================================
            // ArrayCreate — allocate array with header, store elements
            // ================================================================
            MirInstrKind::ArrayCreate {
                dest,
                elements,
                elem_type,
            } => {
                let elem_llvm_ty = ctx.get_llvm_type(*elem_type);
                let len_i32 = ctx.i32_type().const_int(elements.len() as u64, false);
                let data_ptr = alloc_with_header(ctx, len_i32, elem_llvm_ty, "arr")?;
                let data_ptr: PointerValue = data_ptr;

                let elem_ptr_ty = ctx.ptr_type();
                let base = ctx
                    .builder
                    .build_pointer_cast(data_ptr, elem_ptr_ty, "arr_data_cast")
                    .ok()?;

                let mut element_temp_names = Vec::new();

                for (i, elem) in elements.iter().enumerate() {
                    if let MirOperand::Temp(name) = elem {
                        element_temp_names.push(resolve(*name));
                    }

                    let Some(val) = operand_to_value(ctx, elem) else {
                        continue;
                    };
                    let store_val = if *elem_type == doo_core::types::builtin::STR {
                        // String elements must ALWAYS be cloned to heap.
                        // val may be a plain i8* pointer OR a fat string { ptr, i64 }.
                        // Extract the pointer from either form, clone it, store the clone.
                        // Never store static pointers — drop_array would free() them → crash.
                        let src_ptr = if val.is_pointer_value() {
                            Some(val.into_pointer_value())
                        } else if val.is_struct_value() {
                            // Fat string { ptr, i64 } — extract ptr field (index 0)
                            ctx.builder
                                .build_extract_value(val.into_struct_value(), 0, "arr_str_ptr")
                                .ok()
                                .and_then(|v| {
                                    if v.is_pointer_value() {
                                        Some(v.into_pointer_value())
                                    } else if v.is_int_value() {
                                        ctx.builder
                                            .build_int_to_ptr(
                                                v.into_int_value(),
                                                ctx.ptr_type(),
                                                "arr_i2p",
                                            )
                                            .ok()
                                    } else {
                                        None
                                    }
                                })
                        } else {
                            None
                        };
                        match src_ptr.and_then(|p| super::memory::clone_string(ctx, p)) {
                            Some(cloned) => cloned.into(),
                            None => ctx.ptr_type().const_null().into(),
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
                ctx.array_element_types.insert(resolve(*dest), *elem_type);

                if !element_temp_names.is_empty() {
                    ctx.array_element_temps
                        .insert(resolve(*dest), element_temp_names);
                }

                Some(data_ptr.into())
            }

            // ================================================================
            // ArrayGet — arr[index]  (language feature, like Rust's Index trait)
            // ================================================================
            MirInstrKind::ArrayGet {
                dest,
                array,
                index,
                elem_type,
            } => {
                let arr = operand_to_value(ctx, array)?;
                let idx = operand_to_value(ctx, index)?;
                if !idx.is_int_value() {
                    return None;
                }

                // FIX: Handle i64 array pointers (stored as i64 due to type coercion)
                let arr_ptr = value_to_array_ptr(ctx, arr, "array_get")?;
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

                // Deep-clone struct/string elements on access
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
                    _ => val,
                };

                ctx.set_temp(&resolve(*dest), val);
                ctx.set_variable_type(&resolve(*dest), *elem_type);

                if let Some(struct_name) = ctx.get_struct_name_from_type_id(*elem_type) {
                    ctx.set_temp_struct_type(&resolve(*dest), &struct_name);
                }

                Some(val)
            }

            // ================================================================
            // ArraySet — arr[index] = value  (language feature)
            // ================================================================
            MirInstrKind::ArraySet {
                array,
                index,
                value,
                elem_type,
            } => {
                let arr = operand_to_value(ctx, array)?;
                let idx = operand_to_value(ctx, index)?;
                let val = operand_to_value(ctx, value)?;
                if !idx.is_int_value() {
                    return None;
                }

                // FIX: Handle i64 array pointers
                let arr_ptr = value_to_array_ptr(ctx, arr, "array_set")?;
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

                // Handle value stored as i64 when elem_type expects pointer
                let store_val = if elem_llvm_ty.is_pointer_type() && val.is_int_value() {
                    ctx.builder
                        .build_int_to_ptr(
                            val.into_int_value(),
                            elem_llvm_ty.into_pointer_type(),
                            "set_val_inttoptr",
                        )
                        .ok()
                        .map(|p| p.into())
                        .unwrap_or(val)
                } else {
                    val
                };

                ctx.builder.build_store(elem_ptr, store_val).ok();
                None
            }

            // ================================================================
            // ArrayLen — get array length  (also handled via MethodCall)
            // ================================================================
            MirInstrKind::ArrayLen { dest, array } => {
                let arr = operand_to_value(ctx, array)?;

                // FIX: Handle i64 array pointers
                let arr_ptr = value_to_array_ptr(ctx, arr, "array_len")?;
                let len_i64 = get_array_length_from_data(ctx, arr_ptr)?;
                ctx.set_temp(&resolve(*dest), len_i64.into());
                Some(len_i64.into())
            }

            // ================================================================
            // ArrayContains
            // ================================================================
            MirInstrKind::ArrayContains {
                dest,
                array,
                value,
                elem_type,
            } => {
                let arr = operand_to_value(ctx, array)?;
                let needle = operand_to_value(ctx, value)?;

                let arr_ptr = value_to_array_ptr(ctx, arr, "array_contains")?;

                let len_i64 = get_array_length_from_data(ctx, arr_ptr)?;
                let i64_type = ctx.context.i64_type();
                let bool_type = ctx.context.bool_type();

                let elem_llvm_ty = ctx.get_llvm_type(*elem_type);
                let elem_ptr_ty = ctx.ptr_type();
                let base = ctx
                    .builder
                    .build_pointer_cast(arr_ptr, elem_ptr_ty, "c_base")
                    .ok()?;

                let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
                let loop_bb = ctx.context.append_basic_block(current_fn, "c_loop");
                let body_bb = ctx.context.append_basic_block(current_fn, "c_body");
                let found_bb = ctx.context.append_basic_block(current_fn, "c_found");
                let inc_bb = ctx.context.append_basic_block(current_fn, "c_inc");
                let end_bb = ctx.context.append_basic_block(current_fn, "c_end");

                let idx_alloca = ctx.alloca_in_entry_block(i64_type, "c_idx")?;
                ctx.builder
                    .build_store(idx_alloca, i64_type.const_zero())
                    .ok()?;

                let res_alloca = ctx.alloca_in_entry_block(bool_type, "c_res")?;
                ctx.builder
                    .build_store(res_alloca, bool_type.const_zero())
                    .ok()?;

                ctx.builder.build_unconditional_branch(loop_bb).ok()?;

                // Loop header
                ctx.builder.position_at_end(loop_bb);
                let idx = ctx
                    .builder
                    .build_load(i64_type, idx_alloca, "c_idx_load")
                    .ok()?
                    .into_int_value();
                let cond = ctx
                    .builder
                    .build_int_compare(IntPredicate::ULT, idx, len_i64, "c_cond")
                    .ok()?;
                ctx.builder
                    .build_conditional_branch(cond, body_bb, end_bb)
                    .ok()?;

                // Body: load element and compare using SIMPLE comparison
                // (NOT emit_eq — that creates extra blocks that break the loop)
                ctx.builder.position_at_end(body_bb);
                let elem_ptr = unsafe {
                    ctx.builder
                        .build_gep(elem_llvm_ty, base, &[idx], "c_elem_ptr")
                }
                .ok()?;
                let elem_val = ctx
                    .builder
                    .build_load(elem_llvm_ty, elem_ptr, "c_elem_val")
                    .ok()?;

                // Simple type-based comparison — one comparison per type, no extra blocks
                let type_kind = ctx.get_type_kind(*elem_type);
                let is_eq = match &type_kind {
                    Some(TypeKind::Int) | Some(TypeKind::Bool) => {
                        if elem_val.is_int_value() && needle.is_int_value() {
                            ctx.builder
                                .build_int_compare(
                                    IntPredicate::EQ,
                                    elem_val.into_int_value(),
                                    needle.into_int_value(),
                                    "c_eq_int",
                                )
                                .ok()?
                        } else {
                            bool_type.const_zero()
                        }
                    }
                    Some(TypeKind::Str) => {
                        // Use strcmp for strings — one call, no extra blocks
                        let ptr_type = ctx
                            .context
                            .i8_type()
                            .ptr_type(inkwell::AddressSpace::default());
                        let strcmp_fn =
                            ctx.module
                                .get_function(ffi_names::STRCMP)
                                .unwrap_or_else(|| {
                                    let fn_ty = ctx
                                        .i32_type()
                                        .fn_type(&[ptr_type.into(), ptr_type.into()], false);
                                    ctx.module.add_function(ffi_names::STRCMP, fn_ty, None)
                                });
                        let ep = if elem_val.is_pointer_value() {
                            elem_val.into_pointer_value()
                        } else {
                            ctx.builder
                                .build_int_to_ptr(elem_val.into_int_value(), ptr_type, "c_ep")
                                .ok()
                                .unwrap_or(ptr_type.const_null())
                        };
                        let np = if needle.is_pointer_value() {
                            needle.into_pointer_value()
                        } else {
                            ctx.builder
                                .build_int_to_ptr(needle.into_int_value(), ptr_type, "c_np")
                                .ok()
                                .unwrap_or(ptr_type.const_null())
                        };
                        let cmp_result = ctx
                            .builder
                            .build_call(strcmp_fn, &[ep.into(), np.into()], "c_strcmp")
                            .ok()?;
                        let cmp_int = cmp_result.try_as_basic_value().basic()?.into_int_value();
                        ctx.builder
                            .build_int_compare(
                                IntPredicate::EQ,
                                cmp_int,
                                ctx.i32_type().const_zero(),
                                "c_str_eq",
                            )
                            .ok()?
                    }
                    Some(TypeKind::Float32 | TypeKind::Float64) => {
                        if elem_val.is_float_value() && needle.is_float_value() {
                            ctx.builder
                                .build_float_compare(
                                    inkwell::FloatPredicate::OEQ,
                                    elem_val.into_float_value(),
                                    needle.into_float_value(),
                                    "c_eq_flt",
                                )
                                .ok()?
                        } else {
                            bool_type.const_zero()
                        }
                    }
                    _ => bool_type.const_zero(),
                };

                // Branch based on comparison result — no extra blocks created
                ctx.builder
                    .build_conditional_branch(is_eq, found_bb, inc_bb)
                    .ok()?;

                // Found: store true
                ctx.builder.position_at_end(found_bb);
                ctx.builder
                    .build_store(res_alloca, bool_type.const_int(1, false))
                    .ok()?;
                ctx.builder.build_unconditional_branch(end_bb).ok()?;

                // Increment: idx++, back to loop
                ctx.builder.position_at_end(inc_bb);
                let next_idx = ctx
                    .builder
                    .build_int_add(idx, i64_type.const_int(1, false), "c_next")
                    .ok()?;
                ctx.builder.build_store(idx_alloca, next_idx).ok()?;
                ctx.builder.build_unconditional_branch(loop_bb).ok()?;

                // End: load result
                ctx.builder.position_at_end(end_bb);
                let res = ctx
                    .builder
                    .build_load(bool_type, res_alloca, &resolve(*dest))
                    .ok()?;
                ctx.set_temp(&resolve(*dest), res);
                Some(res)
            }
            // ================================================================
            // ArrayPush
            // ================================================================
            MirInstrKind::ArrayPush { array, value } => {
                let arr_val = operand_to_value(ctx, array)?;
                let val = operand_to_value(ctx, value)?;

                // FIX: Handle i64 array pointers
                let old_data = value_to_array_ptr(ctx, arr_val, "array_push")?;

                let len_i64 = get_array_length_from_data(ctx, old_data)?;

                let new_len_i64 = ctx
                    .builder
                    .build_int_add(len_i64, ctx.i64_type().const_int(1, false), "new_len")
                    .ok()?;

                let new_len_i32 = ctx
                    .builder
                    .build_int_truncate(new_len_i64, ctx.i32_type(), "new_len_i32")
                    .ok()?;

                let val_type = val.get_type();
                let elem_llvm_ty = val_type;
                let pair_size = elem_llvm_ty.size_of()?;

                use crate::layout::realloc_array_capacity;
                let new_data = realloc_array_capacity(ctx, old_data, new_len_i32, pair_size)?;

                if let MirOperand::Local(name) | MirOperand::Temp(name) = array {
                    ctx.set_temp(&resolve(*name), new_data.into());
                    if let Some(local_ptr) = ctx.get_local(&resolve(*name)) {
                        ctx.builder.build_store(local_ptr, new_data).ok();
                    }
                }

                let elem_ptr_ty = ctx.ptr_type();
                let base = ctx
                    .builder
                    .build_pointer_cast(new_data, elem_ptr_ty, "arr_new_cast")
                    .ok()?;
                let elem_ptr = unsafe {
                    ctx.builder
                        .build_gep(elem_llvm_ty, base, &[len_i64], "elem_ptr")
                }
                .ok()?;

                // Handle value stored as i64 when elem type expects pointer
                let store_val = if val_type.is_pointer_type() && val.is_int_value() {
                    ctx.builder
                        .build_int_to_ptr(
                            val.into_int_value(),
                            val_type.into_pointer_type(),
                            "push_val_inttoptr",
                        )
                        .ok()
                        .map(|p| p.into())
                        .unwrap_or(val)
                } else {
                    val
                };

                ctx.builder.build_store(elem_ptr, store_val).ok();

                None
            }

            // ================================================================
            // ArrayExtend
            // ================================================================
            MirInstrKind::ArrayExtend {
                array,
                other,
                elem_type,
            } => {
                let arr1_val = operand_to_value(ctx, array)?;
                let arr2_val = operand_to_value(ctx, other)?;

                // FIX: Handle i64 array pointers
                let arr1 = value_to_array_ptr(ctx, arr1_val, "array_extend_1")?;
                let arr2 = value_to_array_ptr(ctx, arr2_val, "array_extend_2")?;

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

                let src_base = ctx
                    .builder
                    .build_pointer_cast(arr2, elem_ptr_ty, "src_base")
                    .ok()?;

                let copy_bytes = ctx
                    .builder
                    .build_int_mul(len2_i64, pair_size, "copy_bytes")
                    .ok()?;

                ctx.builder
                    .build_memcpy(dest_ptr, 1, src_base, 1, copy_bytes)
                    .ok()?;

                None
            }

            // ================================================================
            // ArraySlice
            // ================================================================
            MirInstrKind::ArraySlice {
                dest,
                array,
                start,
                end,
                elem_type,
            } => {
                let arr_val = operand_to_value(ctx, array)?;
                let start_val = operand_to_value(ctx, start)?.into_int_value();
                let end_val = operand_to_value(ctx, end)?.into_int_value();

                // FIX: Handle i64 array pointers
                let arr = value_to_array_ptr(ctx, arr_val, "array_slice")?;

                let len = ctx
                    .builder
                    .build_int_sub(end_val, start_val, "len_i64")
                    .ok()?;
                let len_i32 = ctx
                    .builder
                    .build_int_cast(len, ctx.i32_type(), "len_i32")
                    .ok()?;

                let elem_llvm_ty = ctx.get_llvm_type(*elem_type);
                let new_data = alloc_with_header(ctx, len_i32, elem_llvm_ty, "slice")?;

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

                let dest_base = ctx
                    .builder
                    .build_pointer_cast(new_data, elem_ptr_ty, "dest_base")
                    .ok()?;

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
