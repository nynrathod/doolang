//! Map Builtin Methods - Complete Implementation
//! Methods: has, size, isEmpty, keys, values, remove, clear
//! Note: MapCreate/Get/Set/Has are handled in collections.rs via MIR instructions.
//! This file handles MethodCall dispatch for additional map operations.

use crate::context::CodegenContext;
use crate::utils::emit_eq;
use doo_core::constants::ffi_names;
use doo_core::types::{TypeId, TypeKind};
use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

pub struct MapBuiltins;

impl MapBuiltins {
    /// Dispatch map method call
    pub fn dispatch<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        dest: Option<&str>,
        receiver_name: Option<&str>,
        receiver_type: TypeId,
        receiver_ptr: PointerValue<'ctx>,
        method: &str,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        let (key_type, val_type) = match ctx.get_type_kind(receiver_type)? {
            TypeKind::Map { key, value } => (key, value),
            _ => return None,
        };

        let result = match method {
            "size" => Self::emit_size(ctx, receiver_ptr),
            "isEmpty" => Self::emit_is_empty(ctx, receiver_ptr),
            "clear" => Self::emit_clear(ctx, receiver_ptr),
            "keys" => Self::emit_keys(ctx, key_type, val_type, receiver_ptr),
            "values" => Self::emit_values(ctx, key_type, val_type, receiver_ptr),
            "remove" => {
                Self::emit_remove(ctx, receiver_name, key_type, val_type, receiver_ptr, args)
            }
            _ => None,
        };

        if let (Some(val), Some(dest_name)) = (result, dest) {
            ctx.set_temp(dest_name, val);
        }

        result
    }

    // =========================================================================
    // size() -> Int
    // =========================================================================
    fn emit_size<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        map_ptr: PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let len_i32 = load_len_i32(ctx, map_ptr)?;
        let len_i64 = ctx
            .builder
            .build_int_z_extend(len_i32, ctx.context.i64_type(), "size")
            .ok()?;
        Some(len_i64.into())
    }

    // =========================================================================
    // isEmpty() -> Bool
    // =========================================================================
    fn emit_is_empty<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        map_ptr: PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let len_i32 = load_len_i32(ctx, map_ptr)?;
        let is_zero = ctx
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                len_i32,
                ctx.context.i32_type().const_zero(),
                "is_empty",
            )
            .ok()?;
        // Bool is i8 in Doo — must match so `!` (logical NOT) works correctly
        let result = ctx
            .builder
            .build_int_z_extend(is_zero, ctx.context.i8_type(), "bool")
            .ok()?;
        Some(result.into())
    }

    // =========================================================================
    // clear() -> mutates in place
    // =========================================================================
    fn emit_clear<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        map_ptr: PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        // Use proper header-aware function: map_ptr is DATA pointer
        let zero_i64 = ctx.context.i64_type().const_zero();
        set_map_length_from_data(ctx, map_ptr, zero_i64)?;
        Some(map_ptr.into())
    }

    // =========================================================================
    // keys() -> [K] (returns array of keys)
    // =========================================================================
    pub fn emit_keys<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        key_type: TypeId,
        val_type: TypeId,
        map_ptr: PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let len_i32 = load_len_i32(ctx, map_ptr)?;
        let key_llvm = ctx.get_llvm_type(key_type);
        let val_llvm = ctx.get_llvm_type(val_type);
        let pair_ty = ctx
            .context
            .struct_type(&[key_llvm.into(), val_llvm.into()], false);
        let pair_ptr_ty = ctx.ptr_type();
        let map_base = ctx
            .builder
            .build_pointer_cast(map_ptr, pair_ptr_ty, "map_data_cast")
            .ok()?;

        let out_data = alloc_with_header(ctx, len_i32, key_llvm, "map_keys")?;
        let out_base = ctx
            .builder
            .build_pointer_cast(
                out_data,
                ctx.ptr_type(),
                "keys_cast",
            )
            .ok()?;

        let len_i64 = ctx
            .builder
            .build_int_z_extend(len_i32, ctx.context.i64_type(), "len64")
            .ok()?;
        let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
        let loop_bb = ctx.context.append_basic_block(current_fn, "keys_loop");
        let body_bb = ctx.context.append_basic_block(current_fn, "keys_body");
        let end_bb = ctx.context.append_basic_block(current_fn, "keys_end");
        let idx_alloca = ctx
            .alloca_in_entry_block(ctx.context.i64_type(), "idx")?;
        ctx.builder
            .build_store(idx_alloca, ctx.context.i64_type().const_zero())
            .ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(loop_bb);
        let idx = ctx
            .builder
            .build_load(ctx.context.i64_type(), idx_alloca, "idx")
            .ok()?
            .into_int_value();
        let cond = ctx
            .builder
            .build_int_compare(IntPredicate::ULT, idx, len_i64, "cond")
            .ok()?;
        ctx.builder
            .build_conditional_branch(cond, body_bb, end_bb)
            .ok()?;

        ctx.builder.position_at_end(body_bb);
        let pair_ptr =
            unsafe { ctx.builder.build_gep(pair_ty, map_base, &[idx], "pair_ptr") }.ok()?;
        let key_ptr = ctx
            .builder
            .build_struct_gep(pair_ty, pair_ptr, 0, "key_ptr")
            .ok()?;
        let keyv = ctx.builder.build_load(key_llvm, key_ptr, "key").ok()?;
        let out_ptr =
            unsafe { ctx.builder.build_gep(key_llvm, out_base, &[idx], "out_ptr") }.ok()?;
        ctx.builder.build_store(out_ptr, keyv).ok()?;
        let next = ctx
            .builder
            .build_int_add(idx, ctx.context.i64_type().const_int(1, false), "next")
            .ok()?;
        ctx.builder.build_store(idx_alloca, next).ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(end_bb);
        Some(out_data.into())
    }

    // =========================================================================
    // values() -> [V] (returns array of values)
    // =========================================================================
    fn emit_values<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        key_type: TypeId,
        val_type: TypeId,
        map_ptr: PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let len_i32 = load_len_i32(ctx, map_ptr)?;
        let key_llvm = ctx.get_llvm_type(key_type);
        let val_llvm = ctx.get_llvm_type(val_type);
        let pair_ty = ctx
            .context
            .struct_type(&[key_llvm.into(), val_llvm.into()], false);
        let pair_ptr_ty = ctx.ptr_type();
        let map_base = ctx
            .builder
            .build_pointer_cast(map_ptr, pair_ptr_ty, "map_data_cast")
            .ok()?;

        let out_data = alloc_with_header(ctx, len_i32, val_llvm, "map_values")?;
        let out_base = ctx
            .builder
            .build_pointer_cast(
                out_data,
                ctx.ptr_type(),
                "values_cast",
            )
            .ok()?;

        let len_i64 = ctx
            .builder
            .build_int_z_extend(len_i32, ctx.context.i64_type(), "len64")
            .ok()?;
        let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
        let loop_bb = ctx.context.append_basic_block(current_fn, "values_loop");
        let body_bb = ctx.context.append_basic_block(current_fn, "values_body");
        let end_bb = ctx.context.append_basic_block(current_fn, "values_end");
        let idx_alloca = ctx
            .alloca_in_entry_block(ctx.context.i64_type(), "idx")?;
        ctx.builder
            .build_store(idx_alloca, ctx.context.i64_type().const_zero())
            .ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(loop_bb);
        let idx = ctx
            .builder
            .build_load(ctx.context.i64_type(), idx_alloca, "idx")
            .ok()?
            .into_int_value();
        let cond = ctx
            .builder
            .build_int_compare(IntPredicate::ULT, idx, len_i64, "cond")
            .ok()?;
        ctx.builder
            .build_conditional_branch(cond, body_bb, end_bb)
            .ok()?;

        ctx.builder.position_at_end(body_bb);
        let pair_ptr =
            unsafe { ctx.builder.build_gep(pair_ty, map_base, &[idx], "pair_ptr") }.ok()?;
        let val_ptr = ctx
            .builder
            .build_struct_gep(pair_ty, pair_ptr, 1, "val_ptr")
            .ok()?;
        let valv = ctx.builder.build_load(val_llvm, val_ptr, "val").ok()?;
        let out_ptr =
            unsafe { ctx.builder.build_gep(val_llvm, out_base, &[idx], "out_ptr") }.ok()?;
        ctx.builder.build_store(out_ptr, valv).ok()?;
        let next = ctx
            .builder
            .build_int_add(idx, ctx.context.i64_type().const_int(1, false), "next")
            .ok()?;
        ctx.builder.build_store(idx_alloca, next).ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(end_bb);
        Some(out_data.into())
    }

    // =========================================================================
    // remove(key: K) -> removes key from map
    // =========================================================================
    fn emit_remove<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        receiver_name: Option<&str>,
        key_type: TypeId,
        val_type: TypeId,
        map_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }
        let needle = args[0];

        let key_llvm = ctx.get_llvm_type(key_type);
        let val_llvm = ctx.get_llvm_type(val_type);
        let pair_ty = ctx
            .context
            .struct_type(&[key_llvm.into(), val_llvm.into()], false);
        let pair_ptr_ty = ctx.ptr_type();

        let len_i32 = load_len_i32(ctx, map_ptr)?;
        let len_i64 = ctx
            .builder
            .build_int_z_extend(len_i32, ctx.context.i64_type(), "len64")
            .ok()?;

        let old_base = ctx
            .builder
            .build_pointer_cast(map_ptr, pair_ptr_ty, "map_data_cast")
            .ok()?;

        let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
        let loop_bb = ctx
            .context
            .append_basic_block(current_fn, "map_remove_loop");
        let check_bb = ctx
            .context
            .append_basic_block(current_fn, "map_remove_check");
        let inc_bb = ctx.context.append_basic_block(current_fn, "map_remove_inc");
        let found_bb = ctx
            .context
            .append_basic_block(current_fn, "map_remove_found");
        let not_found_bb = ctx
            .context
            .append_basic_block(current_fn, "map_remove_not_found");
        let end_bb = ctx.context.append_basic_block(current_fn, "map_remove_end");

        let idx_alloca = ctx
            .alloca_in_entry_block(ctx.context.i64_type(), "idx")?;
        ctx.builder
            .build_store(idx_alloca, ctx.context.i64_type().const_zero())
            .ok()?;
        let found_idx_alloca = ctx
            .alloca_in_entry_block(ctx.context.i64_type(), "found_idx")?;
        ctx.builder
            .build_store(found_idx_alloca, ctx.context.i64_type().const_zero())
            .ok()?;
        let found_flag_alloca = ctx
            .alloca_in_entry_block(ctx.context.bool_type(), "found")?;
        ctx.builder
            .build_store(found_flag_alloca, ctx.context.bool_type().const_zero())
            .ok()?;

        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(loop_bb);
        let idx = ctx
            .builder
            .build_load(ctx.context.i64_type(), idx_alloca, "idx")
            .ok()?
            .into_int_value();
        let cond = ctx
            .builder
            .build_int_compare(IntPredicate::ULT, idx, len_i64, "cond")
            .ok()?;
        ctx.builder
            .build_conditional_branch(cond, check_bb, not_found_bb)
            .ok()?;

        ctx.builder.position_at_end(check_bb);
        let pair_ptr =
            unsafe { ctx.builder.build_gep(pair_ty, old_base, &[idx], "pair_ptr") }.ok()?;
        let key_ptr = ctx
            .builder
            .build_struct_gep(pair_ty, pair_ptr, 0, "key_ptr")
            .ok()?;
        let stored_key = ctx
            .builder
            .build_load(key_llvm, key_ptr, "stored_key")
            .ok()?;
        let is_eq = emit_eq(ctx, key_type, stored_key, needle)?;
        ctx.builder
            .build_conditional_branch(is_eq, found_bb, inc_bb)
            .ok()?;

        ctx.builder.position_at_end(found_bb);
        ctx.builder
            .build_store(
                found_flag_alloca,
                ctx.context.bool_type().const_int(1, false),
            )
            .ok()?;
        ctx.builder.build_store(found_idx_alloca, idx).ok()?;
        ctx.builder.build_unconditional_branch(not_found_bb).ok()?;

        ctx.builder.position_at_end(inc_bb);
        let next = ctx
            .builder
            .build_int_add(idx, ctx.context.i64_type().const_int(1, false), "next")
            .ok()?;
        ctx.builder.build_store(idx_alloca, next).ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(not_found_bb);
        let found = ctx
            .builder
            .build_load(ctx.context.bool_type(), found_flag_alloca, "found")
            .ok()?
            .into_int_value();
        let should_remove = ctx
            .builder
            .build_int_compare(
                IntPredicate::NE,
                found,
                ctx.context.bool_type().const_zero(),
                "should_remove",
            )
            .ok()?;

        let do_remove_bb = ctx.context.append_basic_block(current_fn, "map_remove_do");
        let no_remove_bb = ctx.context.append_basic_block(current_fn, "map_remove_no");
        ctx.builder
            .build_conditional_branch(should_remove, do_remove_bb, no_remove_bb)
            .ok()?;

        // no removal
        ctx.builder.position_at_end(no_remove_bb);
        ctx.builder.build_unconditional_branch(end_bb).ok()?;

        // do removal
        ctx.builder.position_at_end(do_remove_bb);
        let found_idx = ctx
            .builder
            .build_load(ctx.context.i64_type(), found_idx_alloca, "found_idx")
            .ok()?
            .into_int_value();
        let new_len_i32 = ctx
            .builder
            .build_int_sub(
                len_i32,
                ctx.context.i32_type().const_int(1, false),
                "new_len",
            )
            .ok()?;
        let new_len_i64 = ctx
            .builder
            .build_int_z_extend(new_len_i32, ctx.context.i64_type(), "new_len64")
            .ok()?;

        // Copy old pairs into a temporary stack buffer BEFORE realloc (realloc may move/free old)
        let tmp_alloca = ctx
            .builder
            .build_array_alloca(pair_ty, new_len_i64, "tmp_pairs")
            .ok()?;

        let idx2_alloca = ctx.alloca_in_entry_block(ctx.context.i64_type(), "i")?;
        ctx.builder
            .build_store(idx2_alloca, ctx.context.i64_type().const_zero())
            .ok()?;
        let pre_loop = ctx
            .context
            .append_basic_block(current_fn, "map_remove_pre_copy_loop");
        let pre_body = ctx
            .context
            .append_basic_block(current_fn, "map_remove_pre_copy_body");
        let pre_end = ctx
            .context
            .append_basic_block(current_fn, "map_remove_pre_copy_end");
        ctx.builder.build_unconditional_branch(pre_loop).ok()?;

        ctx.builder.position_at_end(pre_loop);
        let i = ctx
            .builder
            .build_load(ctx.context.i64_type(), idx2_alloca, "i")
            .ok()?
            .into_int_value();
        let cond = ctx
            .builder
            .build_int_compare(IntPredicate::ULT, i, new_len_i64, "cond")
            .ok()?;
        ctx.builder
            .build_conditional_branch(cond, pre_body, pre_end)
            .ok()?;

        ctx.builder.position_at_end(pre_body);
        let lt = ctx
            .builder
            .build_int_compare(IntPredicate::ULT, i, found_idx, "lt")
            .ok()?;
        let i1 = ctx
            .builder
            .build_int_add(i, ctx.context.i64_type().const_int(1, false), "i1")
            .ok()?;
        let src_i = ctx
            .builder
            .build_select(lt, i, i1, "src_i")
            .ok()?
            .into_int_value();

        let src_pair = unsafe {
            ctx.builder
                .build_gep(pair_ty, old_base, &[src_i], "src_pair")
        }
        .ok()?;
        let dst_pair =
            unsafe { ctx.builder.build_gep(pair_ty, tmp_alloca, &[i], "dst_pair") }.ok()?;
        let sk = ctx
            .builder
            .build_struct_gep(pair_ty, src_pair, 0, "sk")
            .ok()?;
        let sv = ctx
            .builder
            .build_struct_gep(pair_ty, src_pair, 1, "sv")
            .ok()?;
        let dk = ctx
            .builder
            .build_struct_gep(pair_ty, dst_pair, 0, "dk")
            .ok()?;
        let dv = ctx
            .builder
            .build_struct_gep(pair_ty, dst_pair, 1, "dv")
            .ok()?;
        let kval = ctx.builder.build_load(key_llvm, sk, "kval").ok()?;
        let vval = ctx.builder.build_load(val_llvm, sv, "vval").ok()?;
        ctx.builder.build_store(dk, kval).ok()?;
        ctx.builder.build_store(dv, vval).ok()?;

        let next = ctx
            .builder
            .build_int_add(i, ctx.context.i64_type().const_int(1, false), "next")
            .ok()?;
        ctx.builder.build_store(idx2_alloca, next).ok()?;
        ctx.builder.build_unconditional_branch(pre_loop).ok()?;

        // shrink + copy from tmp into new data
        ctx.builder.position_at_end(pre_end);
        // Use standard realloc (fallback from DOO_REALLOC)
        let realloc_fn = ctx
            .module
            .get_function(ffi_names::DOO_REALLOC)
            .or_else(|| ctx.module.get_function(ffi_names::REALLOC))?;
        let header_ptr = header_ptr_from_data(ctx, map_ptr)?;
        let pair_size = pair_ty.size_of()?;
        let data_bytes = ctx
            .builder
            .build_int_mul(new_len_i64, pair_size, "data_bytes")
            .ok()?;
        // Header is 16 bytes (2 x i64: length + capacity)
        let total = ctx
            .builder
            .build_int_add(
                ctx.context.i64_type().const_int(16, false),
                data_bytes,
                "total",
            )
            .ok()?;
        let new_header = ctx
            .builder
            .build_call(realloc_fn, &[header_ptr.into(), total.into()], "realloc")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_pointer_value();
        // Store length as i64 (header expects i64)
        let new_len_i64_store = ctx
            .builder
            .build_int_z_extend(new_len_i32, ctx.context.i64_type(), "new_len_i64_store")
            .ok()?;
        store_len_at_header(ctx, new_header, new_len_i64_store)?;
        let new_data = data_ptr_from_header(ctx, new_header)?;
        let new_base = ctx
            .builder
            .build_pointer_cast(new_data, pair_ptr_ty, "new_base")
            .ok()?;

        // copy tmp -> new
        ctx.builder
            .build_store(idx2_alloca, ctx.context.i64_type().const_zero())
            .ok()?;
        let copy_loop = ctx
            .context
            .append_basic_block(current_fn, "map_remove_copy_loop");
        let copy_body = ctx
            .context
            .append_basic_block(current_fn, "map_remove_copy_body");
        let copy_end = ctx
            .context
            .append_basic_block(current_fn, "map_remove_copy_end");
        ctx.builder.build_unconditional_branch(copy_loop).ok()?;

        ctx.builder.position_at_end(copy_loop);
        let i = ctx
            .builder
            .build_load(ctx.context.i64_type(), idx2_alloca, "i")
            .ok()?
            .into_int_value();
        let cond = ctx
            .builder
            .build_int_compare(IntPredicate::ULT, i, new_len_i64, "cond")
            .ok()?;
        ctx.builder
            .build_conditional_branch(cond, copy_body, copy_end)
            .ok()?;

        ctx.builder.position_at_end(copy_body);
        let src_pair =
            unsafe { ctx.builder.build_gep(pair_ty, tmp_alloca, &[i], "src_pair") }.ok()?;
        let dst_pair =
            unsafe { ctx.builder.build_gep(pair_ty, new_base, &[i], "dst_pair") }.ok()?;
        let sk = ctx
            .builder
            .build_struct_gep(pair_ty, src_pair, 0, "sk")
            .ok()?;
        let sv = ctx
            .builder
            .build_struct_gep(pair_ty, src_pair, 1, "sv")
            .ok()?;
        let dk = ctx
            .builder
            .build_struct_gep(pair_ty, dst_pair, 0, "dk")
            .ok()?;
        let dv = ctx
            .builder
            .build_struct_gep(pair_ty, dst_pair, 1, "dv")
            .ok()?;
        let kval = ctx.builder.build_load(key_llvm, sk, "kval").ok()?;
        let vval = ctx.builder.build_load(val_llvm, sv, "vval").ok()?;
        ctx.builder.build_store(dk, kval).ok()?;
        ctx.builder.build_store(dv, vval).ok()?;
        let next = ctx
            .builder
            .build_int_add(i, ctx.context.i64_type().const_int(1, false), "next")
            .ok()?;
        ctx.builder.build_store(idx2_alloca, next).ok()?;
        ctx.builder.build_unconditional_branch(copy_loop).ok()?;

        ctx.builder.position_at_end(copy_end);
        if let Some(name) = receiver_name {
            // Use get_local_or_borrow_origin to find the alloca for storing back
            // This handles both direct locals and borrowed temps
            if let Some(local_ptr) = ctx.get_local_or_borrow_origin(name) {
                ctx.builder.build_store(local_ptr, new_data).ok();
            } else {
                ctx.set_temp(name, new_data.into());
            }
        }
        ctx.builder.build_unconditional_branch(end_bb).ok()?;

        ctx.builder.position_at_end(end_bb);
        Some(ctx.context.i8_type().const_zero().into())
    }
}

// =============================================================================
// Helper Functions - Use centralized layout module
// =============================================================================

use crate::layout::{
    alloc_with_header, data_ptr_from_header,
    header_ptr_from_data, load_len_i32, set_map_length_from_data,
    store_len_at_header,
};
