//! Array Instruction Handler
//!
//! Handles: ArrayCreate, ArrayGet, ArraySet, ArrayLen, ArrayContains

use inkwell::values::{BasicValueEnum};
use inkwell::IntPredicate;
use inkwell::AddressSpace;
use doo_mir::{MirInstr, MirInstrKind, MirOperand};
use crate::context::CodegenContext;
use crate::layout::{alloc_with_header, int_to_i64, load_len_i32};
use crate::utils::{operand_to_value, emit_eq};
use super::InstructionHandler;

/// Array instruction handler.
pub struct ArrayHandler;

impl<'ctx> InstructionHandler<'ctx> for ArrayHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(instr.kind,
            MirInstrKind::ArrayCreate { .. } |
            MirInstrKind::ArrayGet { .. } |
            MirInstrKind::ArraySet { .. } |
            MirInstrKind::ArrayLen { .. } |
            MirInstrKind::ArrayContains { .. }
        )
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::ArrayCreate { dest, elements, elem_type } => {
                let elem_llvm_ty = ctx.get_llvm_type(*elem_type);
                let len_i32 = ctx.i32_type().const_int(elements.len() as u64, false);
                let data_ptr = alloc_with_header(ctx, len_i32, elem_llvm_ty, "arr")?;

                let elem_ptr_ty = elem_llvm_ty.ptr_type(AddressSpace::default());
                let base = ctx
                    .builder
                    .build_pointer_cast(data_ptr, elem_ptr_ty, "arr_data_cast")
                    .ok()?;

                for (i, elem) in elements.iter().enumerate() {
                    let Some(val) = operand_to_value(ctx, elem) else { continue; };
                    let idx = ctx.i64_type().const_int(i as u64, false);
                    let elem_ptr = unsafe { ctx.builder.build_gep(elem_llvm_ty, base, &[idx], "elem_ptr") }
                        .ok()?;
                    ctx.builder.build_store(elem_ptr, val).ok();
                }

                ctx.set_temp(dest, data_ptr.into());
                Some(data_ptr.into())
            }

            MirInstrKind::ArrayGet { dest, array, index, elem_type } => {
                let arr = operand_to_value(ctx, array)?;
                let idx = operand_to_value(ctx, index)?;
                if !arr.is_pointer_value() || !idx.is_int_value() {
                    return None;
                }

                let arr_ptr = arr.into_pointer_value();
                let idx_i64 = int_to_i64(ctx, idx.into_int_value())?;
                let elem_llvm_ty = ctx.get_llvm_type(*elem_type);
                let elem_ptr_ty = elem_llvm_ty.ptr_type(AddressSpace::default());
                let base = ctx
                    .builder
                    .build_pointer_cast(arr_ptr, elem_ptr_ty, "arr_data_cast")
                    .ok()?;

                let elem_ptr = unsafe { ctx.builder.build_gep(elem_llvm_ty, base, &[idx_i64], "elem_ptr") }
                    .ok()?;
                let val = ctx.builder.build_load(elem_llvm_ty, elem_ptr, dest).ok()?;
                ctx.set_temp(dest, val);
                Some(val)
            }

            MirInstrKind::ArraySet { array, index, value, elem_type } => {
                let arr = operand_to_value(ctx, array)?;
                let idx = operand_to_value(ctx, index)?;
                let val = operand_to_value(ctx, value)?;
                if !arr.is_pointer_value() || !idx.is_int_value() {
                    return None;
                }

                let arr_ptr = arr.into_pointer_value();
                let idx_i64 = int_to_i64(ctx, idx.into_int_value())?;
                let elem_llvm_ty = ctx.get_llvm_type(*elem_type);
                let elem_ptr_ty = elem_llvm_ty.ptr_type(AddressSpace::default());
                let base = ctx
                    .builder
                    .build_pointer_cast(arr_ptr, elem_ptr_ty, "arr_data_cast")
                    .ok()?;

                let elem_ptr = unsafe { ctx.builder.build_gep(elem_llvm_ty, base, &[idx_i64], "elem_ptr") }
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
                let len_i32 = load_len_i32(ctx, arr_ptr)?;
                let len_i64 = ctx
                    .builder
                    .build_int_z_extend(len_i32, ctx.i64_type(), "len_i64")
                    .ok()?;
                ctx.set_temp(dest, len_i64.into());
                Some(len_i64.into())
            }

            MirInstrKind::ArrayContains { dest, array, value, elem_type } => {
                let arr = operand_to_value(ctx, array)?;
                let needle = operand_to_value(ctx, value)?;
                if !arr.is_pointer_value() {
                    return None;
                }

                let arr_ptr = arr.into_pointer_value();
                let elem_llvm_ty = ctx.get_llvm_type(*elem_type);
                let elem_ptr_ty = elem_llvm_ty.ptr_type(AddressSpace::default());
                let base = ctx
                    .builder
                    .build_pointer_cast(arr_ptr, elem_ptr_ty, "arr_data_cast")
                    .ok()?;

                let len_i32 = load_len_i32(ctx, arr_ptr)?;
                let len_i64 = ctx
                    .builder
                    .build_int_z_extend(len_i32, ctx.i64_type(), "len_i64")
                    .ok()?;

                let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
                let loop_bb = ctx.context.append_basic_block(current_fn, "arr_contains_loop");
                let check_bb = ctx.context.append_basic_block(current_fn, "arr_contains_check");
                let inc_bb = ctx.context.append_basic_block(current_fn, "arr_contains_inc");
                let found_bb = ctx.context.append_basic_block(current_fn, "arr_contains_found");
                let end_bb = ctx.context.append_basic_block(current_fn, "arr_contains_end");

                let idx_alloca = ctx.builder.build_alloca(ctx.i64_type(), "idx").ok()?;
                ctx.builder
                    .build_store(idx_alloca, ctx.i64_type().const_zero())
                    .ok();

                let res_alloca = ctx.builder.build_alloca(ctx.bool_type(), "res").ok()?;
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
                ctx.builder.build_conditional_branch(cond, check_bb, end_bb).ok()?;

                ctx.builder.position_at_end(check_bb);
                let elem_ptr = unsafe { ctx.builder.build_gep(elem_llvm_ty, base, &[idx], "elem_ptr") }
                    .ok()?;
                let elem_val = ctx.builder.build_load(elem_llvm_ty, elem_ptr, "elem").ok()?;
                let is_eq = emit_eq(ctx, *elem_type, elem_val, needle)?;
                ctx.builder.build_conditional_branch(is_eq, found_bb, inc_bb).ok()?;

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
                let res = ctx.builder.build_load(ctx.bool_type(), res_alloca, dest).ok()?;
                ctx.set_temp(dest, res);
                Some(res)
            }

            MirInstrKind::ArrayPush { array, value } => {
                let arr_val = operand_to_value(ctx, array)?;
                let val = operand_to_value(ctx, value)?;
                if !arr_val.is_pointer_value() {
                    return None;
                }
                
                let old_data = arr_val.into_pointer_value();
                let len_i32 = load_len_i32(ctx, old_data)?;
                
                // Calculate new length
                let new_len_i32 = ctx.builder.build_int_add(len_i32, ctx.i32_type().const_int(1, false), "new_len").ok()?;
                
                // Reallocate
                // Note: We need element type. MIR instruction doesn't provide it here unless we look up type of 'value'?
                // Or we assume 'value' type matches array element type.
                // We need to know element SIZE for realloc.
                let val_type = val.get_type();
                let elem_size = val_type.size_of()?; // This might be wrong if val is pointer but array holds structs
                // Better: rely on type info from a registry if available, but codegen usually works on LLVM types.
                // Assuming homogeneous array, element type is type of 'val'.
                
                let elem_llvm_ty = val_type;
                let pair_size = elem_llvm_ty.size_of()?;
                
                // Realloc logic similar to MapSet
                use crate::layout::realloc_array_capacity;
                let new_data = realloc_array_capacity(ctx, old_data, new_len_i32, pair_size)?;
                
                // Store updated pointer back to 'array' operand location if it's a local/temp
                if let MirOperand::Local(name) | MirOperand::Temp(name) = array {
                    ctx.set_temp(name, new_data.into()); // Update SSA value mapping
                    if let Some(local_ptr) = ctx.get_local(name) {
                        ctx.builder.build_store(local_ptr, new_data).ok();
                    }
                }
                
                // Append value
                let elem_ptr_ty = elem_llvm_ty.ptr_type(AddressSpace::default());
                let base = ctx.builder.build_pointer_cast(new_data, elem_ptr_ty, "arr_new_cast").ok()?;
                let idx_i64 = ctx.builder.build_int_z_extend(len_i32, ctx.i64_type(), "idx_i64").ok()?;
                let elem_ptr = unsafe { ctx.builder.build_gep(elem_llvm_ty, base, &[idx_i64], "elem_ptr") }.ok()?;
                ctx.builder.build_store(elem_ptr, val).ok();
                
                None
            }

            MirInstrKind::ArrayExtend { array, other, elem_type } => {
                let arr1_val = operand_to_value(ctx, array)?;
                let arr2_val = operand_to_value(ctx, other)?;
                let arr1 = arr1_val.into_pointer_value();
                let arr2 = arr2_val.into_pointer_value();
                
                let len1 = load_len_i32(ctx, arr1)?;
                let len2 = load_len_i32(ctx, arr2)?;
                
                let new_len_i32 = ctx.builder.build_int_add(len1, len2, "new_len").ok()?;
                
                let elem_llvm_ty = ctx.get_llvm_type(*elem_type);
                let pair_size = elem_llvm_ty.size_of()?;
                
                use crate::layout::realloc_array_capacity;
                let new_data = realloc_array_capacity(ctx, arr1, new_len_i32, pair_size)?;
                
                if let MirOperand::Local(name) | MirOperand::Temp(name) = array {
                    ctx.set_temp(name, new_data.into());
                    if let Some(local_ptr) = ctx.get_local(name) {
                        ctx.builder.build_store(local_ptr, new_data).ok();
                    }
                }
                
                // Copy/Memcpy second array data to new space
                // Dest: new_data + len1 * stride
                let elem_ptr_ty = elem_llvm_ty.ptr_type(AddressSpace::default());
                let base = ctx.builder.build_pointer_cast(new_data, elem_ptr_ty, "arr_base").ok()?;
                let offset = ctx.builder.build_int_z_extend(len1, ctx.i64_type(), "offset").ok()?;
                let dest_ptr = unsafe { ctx.builder.build_gep(elem_llvm_ty, base, &[offset], "dest_ptr") }.ok()?;
                
                // Source: arr2
                let src_base = ctx.builder.build_pointer_cast(arr2, elem_ptr_ty, "src_base").ok()?;
                
                // Memcpy
                let len2_i64 = ctx.builder.build_int_z_extend(len2, ctx.i64_type(), "len2_i64").ok()?;
                let copy_bytes = ctx.builder.build_int_mul(len2_i64, pair_size, "copy_bytes").ok()?;
                
                // Alignments? Assuming default
                ctx.builder.build_memcpy(dest_ptr, 1, src_base, 1, copy_bytes).ok()?;
                
                None
            }

            MirInstrKind::ArraySlice { dest, array, start, end, elem_type } => {
                let arr = operand_to_value(ctx, array)?.into_pointer_value();
                let start_val = operand_to_value(ctx, start)?.into_int_value();
                let end_val = operand_to_value(ctx, end)?.into_int_value();
                
                // Calculate length: end - start
                // Assuming bounds checked or handled elsewhere or user responsibly? 
                // For safety, should clamp or check. But simplifying to raw slice logic.
                let len = ctx.builder.build_int_sub(end_val, start_val, "len_i64").ok()?;
                let len_i32 = ctx.builder.build_int_cast(len, ctx.i32_type(), "len_i32").ok()?;
                
                let elem_llvm_ty = ctx.get_llvm_type(*elem_type);
                // Create new array
                let new_data = alloc_with_header(ctx, len_i32, elem_llvm_ty, "slice")?;
                
                // Source pointer: arr + start
                let elem_ptr_ty = elem_llvm_ty.ptr_type(AddressSpace::default());
                let src_base = ctx.builder.build_pointer_cast(arr, elem_ptr_ty, "src_base").ok()?;
                let start_idx = ctx.builder.build_int_cast(start_val, ctx.i64_type(), "start_idx").ok()?;
                let src_ptr = unsafe { ctx.builder.build_gep(elem_llvm_ty, src_base, &[start_idx], "src_ptr") }.ok()?;
                
                // Dest pointer
                let dest_base = ctx.builder.build_pointer_cast(new_data, elem_ptr_ty, "dest_base").ok()?;
                
                // Memcpy
                let pair_size = elem_llvm_ty.size_of()?;
                let copy_bytes = ctx.builder.build_int_mul(len, pair_size, "copy_bytes").ok()?;
                
                ctx.builder.build_memcpy(dest_base, 1, src_ptr, 1, copy_bytes).ok()?;
                
                ctx.set_temp(dest, new_data.into());
                Some(new_data.into())
            }
