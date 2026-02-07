//! Array Builtin Methods - Complete Implementation with Lambda Support
//! Methods: len, first, last, isEmpty, push, pop, contains, indexOf, sort, reverse, slice, clear, join
//! Lambda Methods: map, filter, reduce
//!
//! Note: ArrayCreate/Get/Set are handled in collections.rs via MIR instructions.
//! This file handles MethodCall dispatch for ALL array method calls.

use crate::context::CodegenContext;
use doo_core::constants::ffi_names;
use doo_core::types::{builtin, TypeId};
use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

pub struct ArrayBuiltins;

impl ArrayBuiltins {
    /// Dispatch array method call
    pub fn dispatch<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        dest: Option<&str>,
        receiver_name: Option<&str>,
        receiver_type: doo_core::types::TypeId,
        receiver_ptr: PointerValue<'ctx>,
        method: &str,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        // Get element type if available, use ANY as fallback for unknown types
        let elem_type_id = match ctx.get_type_kind(receiver_type) {
            Some(doo_core::types::TypeKind::Array { element }) => element,
            Some(doo_core::types::TypeKind::Any) => doo_core::types::builtin::ANY,
            _ => doo_core::types::builtin::ANY, // Fallback for unknown types
        };

        let result = match method {
            // Basic methods
            "len" => Self::emit_len(ctx, receiver_ptr),
            "first" => Self::emit_first(ctx, receiver_ptr),
            "last" => Self::emit_last(ctx, receiver_ptr),
            "isEmpty" => Self::emit_is_empty(ctx, receiver_ptr),
            "pop" => Self::emit_pop(ctx, receiver_ptr),
            "indexOf" => Self::emit_index_of(ctx, receiver_ptr, args),
            "reverse" => Self::emit_reverse(ctx, receiver_ptr),
            "clear" => Self::emit_clear(ctx, receiver_ptr),
            "join" => Self::emit_join(ctx, elem_type_id, receiver_ptr, args),

            "push" => Self::emit_push(ctx, receiver_name, elem_type_id, receiver_ptr, args),
            "sort" => Self::emit_sort(ctx, elem_type_id, receiver_ptr),
            "slice" => Self::emit_slice(ctx, elem_type_id, receiver_ptr, args),

            // Lambda methods - require closure argument
            "map" => Self::emit_map(ctx, elem_type_id, receiver_ptr, args),
            "filter" => Self::emit_filter(ctx, elem_type_id, receiver_ptr, args),
            "reduce" => Self::emit_reduce(ctx, elem_type_id, receiver_ptr, args),

            _ => None,
        };

        if let (Some(val), Some(dest_name)) = (result, dest) {
            ctx.set_temp(dest_name, val);
        }

        result
    }

    // =========================================================================
    // len() -> Int
    // =========================================================================
    fn emit_len<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        arr_ptr: PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let len_i32 = load_len_i32(ctx, arr_ptr)?;
        let len_i64 = ctx
            .builder
            .build_int_z_extend(len_i32, ctx.context.i64_type(), "len64")
            .ok()?;
        Some(len_i64.into())
    }

    // =========================================================================
    // first() -> T
    // =========================================================================
    fn emit_first<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        arr_ptr: PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let elem_ty = ctx.context.i64_type();
        let elem = ctx.builder.build_load(elem_ty, arr_ptr, "first").ok()?;
        Some(elem)
    }

    // =========================================================================
    // last() -> T
    // =========================================================================
    fn emit_last<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        arr_ptr: PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let len_i32 = load_len_i32(ctx, arr_ptr)?;
        let len_i64 = ctx
            .builder
            .build_int_z_extend(len_i32, ctx.context.i64_type(), "len64")
            .ok()?;
        let last_idx = ctx
            .builder
            .build_int_sub(
                len_i64,
                ctx.context.i64_type().const_int(1, false),
                "last_idx",
            )
            .ok()?;

        let elem_ty = ctx.context.i64_type();
        let elem_ptr = unsafe {
            ctx.builder
                .build_in_bounds_gep(elem_ty, arr_ptr, &[last_idx], "last_ptr")
                .ok()?
        };
        let elem = ctx.builder.build_load(elem_ty, elem_ptr, "last").ok()?;
        Some(elem)
    }

    // =========================================================================
    // isEmpty() -> Bool
    // =========================================================================
    fn emit_is_empty<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        arr_ptr: PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let len_i32 = load_len_i32(ctx, arr_ptr)?;
        let is_zero = ctx
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                len_i32,
                ctx.context.i32_type().const_zero(),
                "is_empty",
            )
            .ok()?;
        let result = ctx
            .builder
            .build_int_z_extend(is_zero, ctx.context.i32_type(), "bool")
            .ok()?;
        Some(result.into())
    }

    // =========================================================================
    // pop() -> T
    // =========================================================================
    fn emit_pop<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        arr_ptr: PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let len_i32 = load_len_i32(ctx, arr_ptr)?;
        let len_i64 = ctx
            .builder
            .build_int_z_extend(len_i32, ctx.context.i64_type(), "len64")
            .ok()?;
        let last_idx = ctx
            .builder
            .build_int_sub(
                len_i64,
                ctx.context.i64_type().const_int(1, false),
                "last_idx",
            )
            .ok()?;

        let elem_ty = ctx.context.i64_type();
        let elem_ptr = unsafe {
            ctx.builder
                .build_in_bounds_gep(elem_ty, arr_ptr, &[last_idx], "last_ptr")
                .ok()?
        };
        let elem = ctx.builder.build_load(elem_ty, elem_ptr, "pop").ok()?;

        let new_len = ctx
            .builder
            .build_int_sub(
                len_i32,
                ctx.context.i32_type().const_int(1, false),
                "new_len",
            )
            .ok()?;
        // Use proper header-aware function: arr_ptr is DATA pointer
        let new_len_i64 = ctx
            .builder
            .build_int_z_extend(new_len, ctx.context.i64_type(), "new_len64")
            .ok()?;
        set_array_length_from_data(ctx, arr_ptr, new_len_i64)?;

        Some(elem)
    }

    // =========================================================================
    // indexOf(value: T) -> Int
    // =========================================================================
    fn emit_index_of<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        arr_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }
        let needle = args[0];

        let len_i32 = load_len_i32(ctx, arr_ptr)?;
        let len_i64 = ctx
            .builder
            .build_int_z_extend(len_i32, ctx.context.i64_type(), "len64")
            .ok()?;

        let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
        let loop_bb = ctx.context.append_basic_block(current_fn, "idx_loop");
        let check_bb = ctx.context.append_basic_block(current_fn, "idx_check");
        let found_bb = ctx.context.append_basic_block(current_fn, "idx_found");
        let inc_bb = ctx.context.append_basic_block(current_fn, "idx_inc");
        let end_bb = ctx.context.append_basic_block(current_fn, "idx_end");

        let idx_alloca = ctx
            .builder
            .build_alloca(ctx.context.i64_type(), "idx")
            .ok()?;
        let res_alloca = ctx
            .builder
            .build_alloca(ctx.context.i32_type(), "res")
            .ok()?;
        ctx.builder
            .build_store(idx_alloca, ctx.context.i64_type().const_zero())
            .ok()?;
        ctx.builder
            .build_store(
                res_alloca,
                ctx.context.i32_type().const_int((-1_i32) as u64, true),
            )
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
            .build_conditional_branch(cond, check_bb, end_bb)
            .ok()?;

        ctx.builder.position_at_end(check_bb);
        let elem_ty = ctx.context.i64_type();
        let elem_ptr = unsafe {
            ctx.builder
                .build_in_bounds_gep(elem_ty, arr_ptr, &[idx], "elem_ptr")
                .ok()?
        };
        let elem = ctx
            .builder
            .build_load(elem_ty, elem_ptr, "elem")
            .ok()?
            .into_int_value();

        let needle_int = if needle.is_int_value() {
            needle.into_int_value()
        } else {
            return None;
        };
        let is_eq = ctx
            .builder
            .build_int_compare(IntPredicate::EQ, elem, needle_int, "eq")
            .ok()?;
        ctx.builder
            .build_conditional_branch(is_eq, found_bb, inc_bb)
            .ok()?;

        ctx.builder.position_at_end(found_bb);
        let idx_i32 = ctx
            .builder
            .build_int_truncate(idx, ctx.context.i32_type(), "idx32")
            .ok()?;
        ctx.builder.build_store(res_alloca, idx_i32).ok()?;
        ctx.builder.build_unconditional_branch(end_bb).ok()?;

        ctx.builder.position_at_end(inc_bb);
        let next = ctx
            .builder
            .build_int_add(idx, ctx.context.i64_type().const_int(1, false), "next")
            .ok()?;
        ctx.builder.build_store(idx_alloca, next).ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(end_bb);
        let result_i32 = ctx
            .builder
            .build_load(ctx.context.i32_type(), res_alloca, "indexOf_i32")
            .ok()?
            .into_int_value();
        // Sign-extend to i64 so -1 stays as -1 (not 4294967295)
        let result = ctx
            .builder
            .build_int_s_extend(result_i32, ctx.context.i64_type(), "indexOf")
            .ok()?;
        Some(result.into())
    }

    // =========================================================================
    // reverse() -> mutates in place
    // =========================================================================
    fn emit_reverse<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        arr_ptr: PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let len_i32 = load_len_i32(ctx, arr_ptr)?;
        let len_i64 = ctx
            .builder
            .build_int_z_extend(len_i32, ctx.context.i64_type(), "len64")
            .ok()?;
        let half = ctx
            .builder
            .build_int_unsigned_div(len_i64, ctx.context.i64_type().const_int(2, false), "half")
            .ok()?;

        let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
        let loop_bb = ctx.context.append_basic_block(current_fn, "rev_loop");
        let body_bb = ctx.context.append_basic_block(current_fn, "rev_body");
        let after_bb = ctx.context.append_basic_block(current_fn, "rev_after");

        let idx_alloca = ctx
            .builder
            .build_alloca(ctx.context.i64_type(), "idx")
            .ok()?;
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
            .build_int_compare(IntPredicate::ULT, idx, half, "cond")
            .ok()?;
        ctx.builder
            .build_conditional_branch(cond, body_bb, after_bb)
            .ok()?;

        ctx.builder.position_at_end(body_bb);
        let elem_ty = ctx.context.i64_type();

        let rev_idx = ctx
            .builder
            .build_int_sub(len_i64, ctx.context.i64_type().const_int(1, false), "lm1")
            .ok()?;
        let rev_idx = ctx.builder.build_int_sub(rev_idx, idx, "rev_idx").ok()?;

        let ptr1 = unsafe {
            ctx.builder
                .build_in_bounds_gep(elem_ty, arr_ptr, &[idx], "ptr1")
                .ok()?
        };
        let ptr2 = unsafe {
            ctx.builder
                .build_in_bounds_gep(elem_ty, arr_ptr, &[rev_idx], "ptr2")
                .ok()?
        };

        let val1 = ctx.builder.build_load(elem_ty, ptr1, "v1").ok()?;
        let val2 = ctx.builder.build_load(elem_ty, ptr2, "v2").ok()?;
        ctx.builder.build_store(ptr1, val2).ok()?;
        ctx.builder.build_store(ptr2, val1).ok()?;

        let next = ctx
            .builder
            .build_int_add(idx, ctx.context.i64_type().const_int(1, false), "next")
            .ok()?;
        ctx.builder.build_store(idx_alloca, next).ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(after_bb);
        Some(arr_ptr.into())
    }

    // =========================================================================
    // clear() -> mutates in place
    // =========================================================================
    fn emit_clear<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        arr_ptr: PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        // Use proper header-aware function: arr_ptr is DATA pointer
        let zero_i64 = ctx.context.i64_type().const_zero();
        set_array_length_from_data(ctx, arr_ptr, zero_i64)?;
        Some(arr_ptr.into())
    }

    // =========================================================================
    // push(value: T) -> Void
    // =========================================================================
    fn emit_push<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        receiver_name: Option<&str>,
        elem_type: doo_core::types::TypeId,
        arr_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }
        let val = args[0];

        let elem_llvm = ctx.get_llvm_type(elem_type);
        // NOTE: realloc_array_capacity handles REALLOC internally

        let len_i32 = load_len_i32(ctx, arr_ptr)?;
        let new_len_i32 = ctx
            .builder
            .build_int_add(
                len_i32,
                ctx.context.i32_type().const_int(1, false),
                "new_len",
            )
            .ok()?;
        let new_len_i64 = ctx
            .builder
            .build_int_z_extend(new_len_i32, ctx.context.i64_type(), "new_len64")
            .ok()?;

        let elem_size = elem_llvm
            .size_of()
            .unwrap_or(ctx.context.i64_type().const_int(8, false));

        // Use shared realloc helper (Deduplication)
        use crate::layout::realloc_array_capacity;
        let new_data = realloc_array_capacity(ctx, arr_ptr, new_len_i32, elem_size)?;

        let base = ctx
            .builder
            .build_pointer_cast(
                new_data,
                elem_llvm.ptr_type(AddressSpace::default()),
                "arr_cast",
            )
            .ok()?;
        let append_idx = ctx
            .builder
            .build_int_z_extend(len_i32, ctx.context.i64_type(), "append_idx")
            .ok()?;
        let elem_ptr = unsafe {
            ctx.builder
                .build_gep(elem_llvm, base, &[append_idx], "elem_ptr")
        }
        .ok()?;
        ctx.builder.build_store(elem_ptr, val).ok()?;

        if let Some(name) = receiver_name {
            // Use get_local_or_borrow_origin to find the alloca for storing back
            // This handles both direct locals and borrowed temps
            if let Some(local_ptr) = ctx.get_local_or_borrow_origin(name) {
                ctx.builder.build_store(local_ptr, new_data).ok();
            } else {
                ctx.set_temp(name, new_data.into());
            }
        }

        // Return the new array pointer so that FieldSet can use it for storing back
        // to struct fields (e.g., self.Tasks.push(item) needs to update self.Tasks)
        Some(new_data.into())
    }

    // =========================================================================
    // sort() -> Void (currently supports Int arrays)
    // =========================================================================
    fn emit_sort<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        elem_type: doo_core::types::TypeId,
        arr_ptr: PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        if elem_type != builtin::INT {
            return Some(ctx.context.i8_type().const_zero().into());
        }

        let len_i32 = load_len_i32(ctx, arr_ptr)?;
        let len_i64 = ctx
            .builder
            .build_int_z_extend(len_i32, ctx.context.i64_type(), "len64")
            .ok()?;

        let elem_llvm = ctx.get_llvm_type(elem_type);
        let base = ctx
            .builder
            .build_pointer_cast(
                arr_ptr,
                elem_llvm.ptr_type(AddressSpace::default()),
                "arr_cast",
            )
            .ok()?;

        // bubble sort
        let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
        let outer_bb = ctx.context.append_basic_block(current_fn, "sort_outer");
        let outer_body_bb = ctx
            .context
            .append_basic_block(current_fn, "sort_outer_body");
        let inner_bb = ctx.context.append_basic_block(current_fn, "sort_inner");
        let inner_body_bb = ctx
            .context
            .append_basic_block(current_fn, "sort_inner_body");
        let inner_end_bb = ctx.context.append_basic_block(current_fn, "sort_inner_end");
        let outer_end_bb = ctx.context.append_basic_block(current_fn, "sort_outer_end");

        let i_alloca = ctx.builder.build_alloca(ctx.context.i64_type(), "i").ok()?;
        let j_alloca = ctx.builder.build_alloca(ctx.context.i64_type(), "j").ok()?;
        ctx.builder
            .build_store(i_alloca, ctx.context.i64_type().const_zero())
            .ok()?;
        ctx.builder.build_unconditional_branch(outer_bb).ok()?;

        ctx.builder.position_at_end(outer_bb);
        let i = ctx
            .builder
            .build_load(ctx.context.i64_type(), i_alloca, "i")
            .ok()?
            .into_int_value();
        let i_cond = ctx
            .builder
            .build_int_compare(IntPredicate::ULT, i, len_i64, "i_cond")
            .ok()?;
        ctx.builder
            .build_conditional_branch(i_cond, outer_body_bb, outer_end_bb)
            .ok()?;

        ctx.builder.position_at_end(outer_body_bb);
        ctx.builder
            .build_store(j_alloca, ctx.context.i64_type().const_zero())
            .ok()?;
        ctx.builder.build_unconditional_branch(inner_bb).ok()?;

        ctx.builder.position_at_end(inner_bb);
        let j = ctx
            .builder
            .build_load(ctx.context.i64_type(), j_alloca, "j")
            .ok()?
            .into_int_value();
        let one = ctx.context.i64_type().const_int(1, false);
        let last = ctx.builder.build_int_sub(len_i64, one, "last").ok()?;
        let j_cond = ctx
            .builder
            .build_int_compare(IntPredicate::ULT, j, last, "j_cond")
            .ok()?;
        ctx.builder
            .build_conditional_branch(j_cond, inner_body_bb, inner_end_bb)
            .ok()?;

        ctx.builder.position_at_end(inner_body_bb);
        let jp1 = ctx.builder.build_int_add(j, one, "jp1").ok()?;
        let a_ptr = unsafe { ctx.builder.build_gep(elem_llvm, base, &[j], "a_ptr") }.ok()?;
        let b_ptr = unsafe { ctx.builder.build_gep(elem_llvm, base, &[jp1], "b_ptr") }.ok()?;
        let a = ctx
            .builder
            .build_load(elem_llvm, a_ptr, "a")
            .ok()?
            .into_int_value();
        let b = ctx
            .builder
            .build_load(elem_llvm, b_ptr, "b")
            .ok()?
            .into_int_value();
        let swap = ctx
            .builder
            .build_int_compare(IntPredicate::SGT, a, b, "swap")
            .ok()?;
        let swap_bb = ctx.context.append_basic_block(current_fn, "sort_swap");
        let cont_bb = ctx.context.append_basic_block(current_fn, "sort_cont");
        ctx.builder
            .build_conditional_branch(swap, swap_bb, cont_bb)
            .ok()?;

        ctx.builder.position_at_end(swap_bb);
        ctx.builder.build_store(a_ptr, b).ok()?;
        ctx.builder.build_store(b_ptr, a).ok()?;
        ctx.builder.build_unconditional_branch(cont_bb).ok()?;

        ctx.builder.position_at_end(cont_bb);
        let j_next = ctx.builder.build_int_add(j, one, "j_next").ok()?;
        ctx.builder.build_store(j_alloca, j_next).ok()?;
        ctx.builder.build_unconditional_branch(inner_bb).ok()?;

        ctx.builder.position_at_end(inner_end_bb);
        let i_next = ctx.builder.build_int_add(i, one, "i_next").ok()?;
        ctx.builder.build_store(i_alloca, i_next).ok()?;
        ctx.builder.build_unconditional_branch(outer_bb).ok()?;

        ctx.builder.position_at_end(outer_end_bb);
        Some(ctx.context.i8_type().const_zero().into())
    }

    // =========================================================================
    // slice(start: Int, end: Int) -> [T]
    // =========================================================================
    fn emit_slice<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        elem_type: doo_core::types::TypeId,
        arr_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.len() < 2 {
            return None;
        }
        let start = if args[0].is_int_value() {
            args[0].into_int_value()
        } else {
            return None;
        };
        let end = if args[1].is_int_value() {
            args[1].into_int_value()
        } else {
            return None;
        };

        let elem_llvm = ctx.get_llvm_type(elem_type);
        let start64 = int_to_i64(ctx, start)?;
        let end64 = int_to_i64(ctx, end)?;
        let slice_len64 = ctx
            .builder
            .build_int_sub(end64, start64, "slice_len")
            .ok()?;
        let slice_len32 = ctx
            .builder
            .build_int_truncate(slice_len64, ctx.context.i32_type(), "slice_len32")
            .ok()?;

        let out_data = alloc_with_header(ctx, slice_len32, elem_llvm, "slice")?;

        let in_base = ctx
            .builder
            .build_pointer_cast(
                arr_ptr,
                elem_llvm.ptr_type(AddressSpace::default()),
                "in_cast",
            )
            .ok()?;
        let out_base = ctx
            .builder
            .build_pointer_cast(
                out_data,
                elem_llvm.ptr_type(AddressSpace::default()),
                "out_cast",
            )
            .ok()?;

        let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
        let loop_bb = ctx.context.append_basic_block(current_fn, "slice_loop");
        let body_bb = ctx.context.append_basic_block(current_fn, "slice_body");
        let end_bb = ctx.context.append_basic_block(current_fn, "slice_end");

        let idx_alloca = ctx
            .builder
            .build_alloca(ctx.context.i64_type(), "idx")
            .ok()?;
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
            .build_int_compare(IntPredicate::ULT, idx, slice_len64, "cond")
            .ok()?;
        ctx.builder
            .build_conditional_branch(cond, body_bb, end_bb)
            .ok()?;

        ctx.builder.position_at_end(body_bb);
        let src_idx = ctx.builder.build_int_add(start64, idx, "src_idx").ok()?;
        let src_ptr = unsafe {
            ctx.builder
                .build_gep(elem_llvm, in_base, &[src_idx], "src_ptr")
        }
        .ok()?;
        let val = ctx.builder.build_load(elem_llvm, src_ptr, "val").ok()?;
        let dst_ptr = unsafe {
            ctx.builder
                .build_gep(elem_llvm, out_base, &[idx], "dst_ptr")
        }
        .ok()?;
        ctx.builder.build_store(dst_ptr, val).ok()?;
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
    // join(separator: Str) -> Str
    // =========================================================================
    fn emit_join<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        elem_type: doo_core::types::TypeId,
        arr_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }
        if !args[0].is_pointer_value() {
            return None;
        }
        let sep_ptr = args[0].into_pointer_value();

        let len_i32 = load_len_i32(ctx, arr_ptr)?;
        let is_empty = ctx
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                len_i32,
                ctx.context.i32_type().const_zero(),
                "is_empty",
            )
            .ok()?;

        let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
        let empty_bb = ctx.context.append_basic_block(current_fn, "join_empty");
        let non_empty_bb = ctx.context.append_basic_block(current_fn, "join_non_empty");
        let end_bb = ctx.context.append_basic_block(current_fn, "join_end");

        let res_alloca = ctx
            .builder
            .build_alloca(
                ctx.context.i8_type().ptr_type(AddressSpace::default()),
                "join_res",
            )
            .ok()?;

        ctx.builder
            .build_conditional_branch(is_empty, empty_bb, non_empty_bb)
            .ok()?;

        ctx.builder.position_at_end(empty_bb);
        let empty = ctx
            .builder
            .build_global_string_ptr("", "empty")
            .ok()?
            .as_pointer_value();
        ctx.builder.build_store(res_alloca, empty).ok()?;
        ctx.builder.build_unconditional_branch(end_bb).ok()?;

        ctx.builder.position_at_end(non_empty_bb);
        let len_i64 = ctx
            .builder
            .build_int_z_extend(len_i32, ctx.context.i64_type(), "len64")
            .ok()?;
        let one = ctx.context.i64_type().const_int(1, false);
        let len_m1 = ctx.builder.build_int_sub(len_i64, one, "len_m1").ok()?;

        let strlen = get_or_declare_strlen(ctx);
        let malloc = get_or_declare_malloc(ctx);
        let memcpy = get_or_declare_memcpy(ctx);

        let sep_len = ctx
            .builder
            .build_call(strlen, &[sep_ptr.into()], "sep_len")
            .ok()?
            .try_as_basic_value()
            .left()?
            .into_int_value();

        // total = elem_total + (len-1)*sep_len + 1
        let elem_total = if elem_type == builtin::STR {
            let total_alloca = ctx
                .builder
                .build_alloca(ctx.context.i64_type(), "total")
                .ok()?;
            ctx.builder
                .build_store(total_alloca, ctx.context.i64_type().const_zero())
                .ok()?;
            let idx_alloca = ctx
                .builder
                .build_alloca(ctx.context.i64_type(), "idx")
                .ok()?;
            ctx.builder
                .build_store(idx_alloca, ctx.context.i64_type().const_zero())
                .ok()?;

            let loop_bb = ctx.context.append_basic_block(current_fn, "join_len_loop");
            let body_bb = ctx.context.append_basic_block(current_fn, "join_len_body");
            let after_bb = ctx.context.append_basic_block(current_fn, "join_len_after");
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
                .build_conditional_branch(cond, body_bb, after_bb)
                .ok()?;

            ctx.builder.position_at_end(body_bb);
            let total = ctx
                .builder
                .build_load(ctx.context.i64_type(), total_alloca, "total")
                .ok()?
                .into_int_value();

            let elem_llvm = ctx.get_llvm_type(elem_type);
            let base = ctx
                .builder
                .build_pointer_cast(
                    arr_ptr,
                    elem_llvm.ptr_type(AddressSpace::default()),
                    "arr_cast",
                )
                .ok()?;
            let elem_ptr =
                unsafe { ctx.builder.build_gep(elem_llvm, base, &[idx], "elem_ptr") }.ok()?;
            let elem = ctx
                .builder
                .build_load(elem_llvm, elem_ptr, "elem")
                .ok()?
                .into_pointer_value();
            let elen = ctx
                .builder
                .build_call(strlen, &[elem.into()], "elen")
                .ok()?
                .try_as_basic_value()
                .left()?
                .into_int_value();
            let total2 = ctx.builder.build_int_add(total, elen, "total2").ok()?;
            ctx.builder.build_store(total_alloca, total2).ok()?;

            let next = ctx.builder.build_int_add(idx, one, "next").ok()?;
            ctx.builder.build_store(idx_alloca, next).ok()?;
            ctx.builder.build_unconditional_branch(loop_bb).ok()?;

            ctx.builder.position_at_end(after_bb);
            ctx.builder
                .build_load(ctx.context.i64_type(), total_alloca, "elem_total")
                .ok()?
                .into_int_value()
        } else {
            let per = ctx.context.i64_type().const_int(32, false);
            ctx.builder.build_int_mul(len_i64, per, "elem_total").ok()?
        };

        let sep_total = ctx
            .builder
            .build_int_mul(len_m1, sep_len, "sep_total")
            .ok()?;
        let total_no_nul = ctx
            .builder
            .build_int_add(elem_total, sep_total, "total")
            .ok()?;
        let total = ctx
            .builder
            .build_int_add(total_no_nul, one, "total_nul")
            .ok()?;

        let out_ptr = ctx
            .builder
            .build_call(malloc, &[total.into()], "join_out")
            .ok()?
            .try_as_basic_value()
            .left()?
            .into_pointer_value();

        // Calculate buffer end for bounds checking with snprintf
        let buf_end = unsafe {
            ctx.builder
                .build_in_bounds_gep(ctx.context.i8_type(), out_ptr, &[total], "buf_end")
        }
        .ok()?;
        let buf_end_alloca = ctx
            .builder
            .build_alloca(
                ctx.context.i8_type().ptr_type(AddressSpace::default()),
                "buf_end",
            )
            .ok()?;
        ctx.builder.build_store(buf_end_alloca, buf_end).ok()?;

        let cursor_alloca = ctx
            .builder
            .build_alloca(
                ctx.context.i8_type().ptr_type(AddressSpace::default()),
                "cursor",
            )
            .ok()?;
        ctx.builder.build_store(cursor_alloca, out_ptr).ok()?;

        let idx_alloca = ctx
            .builder
            .build_alloca(ctx.context.i64_type(), "idx")
            .ok()?;
        ctx.builder
            .build_store(idx_alloca, ctx.context.i64_type().const_zero())
            .ok()?;

        let loop_bb = ctx.context.append_basic_block(current_fn, "join_fill_loop");
        let body_bb = ctx.context.append_basic_block(current_fn, "join_fill_body");
        let after_bb = ctx
            .context
            .append_basic_block(current_fn, "join_fill_after");
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
            .build_conditional_branch(cond, body_bb, after_bb)
            .ok()?;

        ctx.builder.position_at_end(body_bb);
        let cursor = ctx
            .builder
            .build_load(
                ctx.context.i8_type().ptr_type(AddressSpace::default()),
                cursor_alloca,
                "cursor",
            )
            .ok()?
            .into_pointer_value();

        if elem_type == builtin::STR {
            let elem_llvm = ctx.get_llvm_type(elem_type);
            let base = ctx
                .builder
                .build_pointer_cast(
                    arr_ptr,
                    elem_llvm.ptr_type(AddressSpace::default()),
                    "arr_cast",
                )
                .ok()?;
            let elem_ptr =
                unsafe { ctx.builder.build_gep(elem_llvm, base, &[idx], "elem_ptr") }.ok()?;
            let elem = ctx
                .builder
                .build_load(elem_llvm, elem_ptr, "elem")
                .ok()?
                .into_pointer_value();
            let elen = ctx
                .builder
                .build_call(strlen, &[elem.into()], "elen")
                .ok()?
                .try_as_basic_value()
                .left()?
                .into_int_value();
            ctx.builder
                .build_call(memcpy, &[cursor.into(), elem.into(), elen.into()], "")
                .ok()?;
            let cursor2 = unsafe {
                ctx.builder
                    .build_in_bounds_gep(ctx.context.i8_type(), cursor, &[elen], "cursor2")
            }
            .ok()?;
            ctx.builder.build_store(cursor_alloca, cursor2).ok()?;
        } else {
            let snprintf = get_or_declare_snprintf(ctx);
            let fmt = if elem_type == builtin::INT {
                ctx.builder
                    .build_global_string_ptr("%lld", "fmt_i64")
                    .ok()?
                    .as_pointer_value()
            } else if elem_type == builtin::FLOAT {
                ctx.builder
                    .build_global_string_ptr("%g", "fmt_f64")
                    .ok()?
                    .as_pointer_value()
            } else if elem_type == builtin::BOOL {
                ctx.builder
                    .build_global_string_ptr("%s", "fmt_s")
                    .ok()?
                    .as_pointer_value()
            } else {
                ctx.builder
                    .build_global_string_ptr("%p", "fmt_p")
                    .ok()?
                    .as_pointer_value()
            };

            let elem_llvm = ctx.get_llvm_type(elem_type);
            let base = ctx
                .builder
                .build_pointer_cast(
                    arr_ptr,
                    elem_llvm.ptr_type(AddressSpace::default()),
                    "arr_cast",
                )
                .ok()?;
            let elem_ptr =
                unsafe { ctx.builder.build_gep(elem_llvm, base, &[idx], "elem_ptr") }.ok()?;
            let raw_elem = ctx.builder.build_load(elem_llvm, elem_ptr, "elem").ok()?;

            // Calculate remaining buffer space for snprintf
            let buf_end = ctx
                .builder
                .build_load(
                    ctx.context.i8_type().ptr_type(AddressSpace::default()),
                    buf_end_alloca,
                    "buf_end",
                )
                .ok()?
                .into_pointer_value();
            let remaining = ctx
                .builder
                .build_ptr_diff(ctx.context.i8_type(), buf_end, cursor, "remaining")
                .ok()?;

            let mut snprintf_args: Vec<inkwell::values::BasicMetadataValueEnum> =
                vec![cursor.into(), remaining.into(), fmt.into()];
            if elem_type == builtin::BOOL {
                let b = if raw_elem.is_int_value() {
                    raw_elem.into_int_value()
                } else {
                    return None;
                };
                let is_true = ctx
                    .builder
                    .build_int_compare(IntPredicate::NE, b, b.get_type().const_zero(), "is_true")
                    .ok()?;
                let t = ctx
                    .builder
                    .build_global_string_ptr("true", "true")
                    .ok()?
                    .as_pointer_value();
                let f = ctx
                    .builder
                    .build_global_string_ptr("false", "false")
                    .ok()?
                    .as_pointer_value();
                let s = ctx.builder.build_select(is_true, t, f, "bstr").ok()?;
                snprintf_args.push(s.into());
            } else {
                snprintf_args.push(raw_elem.into());
            }

            let written = ctx
                .builder
                .build_call(snprintf, &snprintf_args, "snprintf")
                .ok()?
                .try_as_basic_value()
                .left()?
                .into_int_value();
            let written64 = ctx
                .builder
                .build_int_z_extend(written, ctx.context.i64_type(), "w64")
                .ok()?;
            let cursor_now = ctx
                .builder
                .build_load(
                    ctx.context.i8_type().ptr_type(AddressSpace::default()),
                    cursor_alloca,
                    "cursor",
                )
                .ok()?
                .into_pointer_value();
            let cursor2 = unsafe {
                ctx.builder.build_in_bounds_gep(
                    ctx.context.i8_type(),
                    cursor_now,
                    &[written64],
                    "cursor2",
                )
            }
            .ok()?;
            ctx.builder.build_store(cursor_alloca, cursor2).ok()?;
        }

        let idx_is_last = ctx
            .builder
            .build_int_compare(IntPredicate::EQ, idx, len_m1, "is_last")
            .ok()?;
        let sep_bb = ctx.context.append_basic_block(current_fn, "join_sep");
        let cont_bb = ctx.context.append_basic_block(current_fn, "join_cont");
        ctx.builder
            .build_conditional_branch(idx_is_last, cont_bb, sep_bb)
            .ok()?;

        ctx.builder.position_at_end(sep_bb);
        let cursor = ctx
            .builder
            .build_load(
                ctx.context.i8_type().ptr_type(AddressSpace::default()),
                cursor_alloca,
                "cursor",
            )
            .ok()?
            .into_pointer_value();
        ctx.builder
            .build_call(memcpy, &[cursor.into(), sep_ptr.into(), sep_len.into()], "")
            .ok()?;
        let cursor2 = unsafe {
            ctx.builder
                .build_in_bounds_gep(ctx.context.i8_type(), cursor, &[sep_len], "cursor2")
        }
        .ok()?;
        ctx.builder.build_store(cursor_alloca, cursor2).ok()?;
        ctx.builder.build_unconditional_branch(cont_bb).ok()?;

        ctx.builder.position_at_end(cont_bb);
        let next = ctx.builder.build_int_add(idx, one, "next").ok()?;
        ctx.builder.build_store(idx_alloca, next).ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(after_bb);
        let cursor = ctx
            .builder
            .build_load(
                ctx.context.i8_type().ptr_type(AddressSpace::default()),
                cursor_alloca,
                "cursor",
            )
            .ok()?
            .into_pointer_value();
        ctx.builder
            .build_store(cursor, ctx.context.i8_type().const_int(0, false))
            .ok()?;
        ctx.builder.build_store(res_alloca, out_ptr).ok()?;
        ctx.builder.build_unconditional_branch(end_bb).ok()?;

        ctx.builder.position_at_end(end_bb);
        let res = ctx
            .builder
            .build_load(
                ctx.context.i8_type().ptr_type(AddressSpace::default()),
                res_alloca,
                "join_res",
            )
            .ok()?;
        Some(res)
    }

    // =========================================================================
    // map(fn: (T) -> U) -> [U]
    // Creates new array with transformed elements
    // =========================================================================
    fn emit_map<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        elem_type_id: doo_core::types::TypeId,
        arr_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }
        let closure_ptr = args[0].into_pointer_value();

        let len_i32 = load_len_i32(ctx, arr_ptr)?;
        let len_i64 = ctx
            .builder
            .build_int_z_extend(len_i32, ctx.context.i64_type(), "len64")
            .ok()?;

        // Get actual element LLVM type based on type_id
        let elem_ty = ctx.get_llvm_type(elem_type_id);
        let elem_size = elem_ty
            .size_of()
            .unwrap_or(ctx.context.i64_type().const_int(8, false));

        // Allocate result array (same size)
        // Standard array layout: 16-byte header (i64 length + i64 capacity) + data
        let doo_alloc = ctx.get_function(ffi_names::DOO_ALLOC)?;
        let total_size = ctx
            .builder
            .build_int_mul(len_i64, elem_size, "data_size")
            .ok()?;
        let header_size = ctx.context.i64_type().const_int(16, false); // length (i64) + capacity (i64)
        let alloc_size = ctx
            .builder
            .build_int_add(header_size, total_size, "alloc_size")
            .ok()?;

        let result_heap = ctx
            .builder
            .build_call(doo_alloc, &[alloc_size.into()], "map_result")
            .ok()?
            .try_as_basic_value()
            .left()?
            .into_pointer_value();

        // Store header: length and capacity (both i64)
        store_header(ctx, result_heap, len_i64, len_i64)?;

        // Get data pointer (offset 16, after 16-byte header)
        let result_data = unsafe {
            ctx.builder
                .build_gep(
                    ctx.context.i8_type(),
                    result_heap,
                    &[ctx.context.i64_type().const_int(16, false)],
                    "result_data",
                )
                .ok()?
        };

        // Loop over elements, call closure, store result
        let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
        let loop_bb = ctx.context.append_basic_block(current_fn, "map_loop");
        let body_bb = ctx.context.append_basic_block(current_fn, "map_body");
        let end_bb = ctx.context.append_basic_block(current_fn, "map_end");

        let idx_alloca = ctx
            .builder
            .build_alloca(ctx.context.i64_type(), "idx")
            .ok()?;
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
        // Load element using correct element type
        let elem_ptr = unsafe {
            ctx.builder
                .build_in_bounds_gep(elem_ty, arr_ptr, &[idx], "elem_ptr")
                .ok()?
        };
        let elem = ctx.builder.build_load(elem_ty, elem_ptr, "elem").ok()?;

        // Call closure with element - return type is same as element type for map
        let mapped = call_closure(ctx, closure_ptr, &[elem], elem_ty)?;

        // Store result using correct element type
        let result_elem_ptr = unsafe {
            ctx.builder
                .build_in_bounds_gep(elem_ty, result_data, &[idx], "res_elem")
                .ok()?
        };
        ctx.builder.build_store(result_elem_ptr, mapped).ok()?;

        // Increment
        let next = ctx
            .builder
            .build_int_add(idx, ctx.context.i64_type().const_int(1, false), "next")
            .ok()?;
        ctx.builder.build_store(idx_alloca, next).ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(end_bb);
        Some(result_data.into())
    }

    // =========================================================================
    // filter(fn: (T) -> Bool) -> [T]
    // Creates new array with elements that pass predicate
    // =========================================================================
    fn emit_filter<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        elem_type_id: doo_core::types::TypeId,
        arr_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }
        let closure_ptr = args[0].into_pointer_value();

        let len_i32 = load_len_i32(ctx, arr_ptr)?;
        let len_i64 = ctx
            .builder
            .build_int_z_extend(len_i32, ctx.context.i64_type(), "len64")
            .ok()?;

        // Get actual element LLVM type based on type_id
        let elem_ty = ctx.get_llvm_type(elem_type_id);
        let elem_size = elem_ty
            .size_of()
            .unwrap_or(ctx.context.i64_type().const_int(8, false));

        // Allocate result array (max size = original size, will track actual count)
        // Standard array layout: 16-byte header (i64 length + i64 capacity) + data
        let doo_alloc = ctx.get_function(ffi_names::DOO_ALLOC)?;
        let total_size = ctx
            .builder
            .build_int_mul(len_i64, elem_size, "data_size")
            .ok()?;
        let header_size = ctx.context.i64_type().const_int(16, false); // length (i64) + capacity (i64)
        let alloc_size = ctx
            .builder
            .build_int_add(header_size, total_size, "alloc_size")
            .ok()?;

        let result_heap = ctx
            .builder
            .build_call(doo_alloc, &[alloc_size.into()], "filter_result")
            .ok()?
            .try_as_basic_value()
            .left()?
            .into_pointer_value();

        // We'll store the initial capacity, length will be set at the end
        // Store capacity = original length (max possible), length = 0 initially
        store_header(
            ctx,
            result_heap,
            ctx.context.i64_type().const_zero(),
            len_i64,
        )?;

        // Get data pointer (offset 16)
        let result_data = unsafe {
            ctx.builder
                .build_gep(
                    ctx.context.i8_type(),
                    result_heap,
                    &[ctx.context.i64_type().const_int(16, false)],
                    "result_data",
                )
                .ok()?
        };

        // Loop with separate result counter
        let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
        let loop_bb = ctx.context.append_basic_block(current_fn, "filter_loop");
        let check_bb = ctx.context.append_basic_block(current_fn, "filter_check");
        let store_bb = ctx.context.append_basic_block(current_fn, "filter_store");
        let inc_bb = ctx.context.append_basic_block(current_fn, "filter_inc");
        let end_bb = ctx.context.append_basic_block(current_fn, "filter_end");

        let idx_alloca = ctx
            .builder
            .build_alloca(ctx.context.i64_type(), "idx")
            .ok()?;
        let res_idx_alloca = ctx
            .builder
            .build_alloca(ctx.context.i64_type(), "res_idx")
            .ok()?;
        ctx.builder
            .build_store(idx_alloca, ctx.context.i64_type().const_zero())
            .ok()?;
        ctx.builder
            .build_store(res_idx_alloca, ctx.context.i64_type().const_zero())
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
            .build_conditional_branch(cond, check_bb, end_bb)
            .ok()?;

        ctx.builder.position_at_end(check_bb);
        let elem_ptr = unsafe {
            ctx.builder
                .build_in_bounds_gep(elem_ty, arr_ptr, &[idx], "elem_ptr")
                .ok()?
        };
        let elem = ctx.builder.build_load(elem_ty, elem_ptr, "elem").ok()?;

        // Call closure predicate - filter predicates return Bool (i1)
        let bool_ty = ctx.context.bool_type().into();
        let pred_result = call_closure(ctx, closure_ptr, &[elem], bool_ty)?;
        let pred_bool = if pred_result.is_int_value() {
            let int_val = pred_result.into_int_value();
            ctx.builder
                .build_int_compare(
                    IntPredicate::NE,
                    int_val,
                    int_val.get_type().const_zero(),
                    "pred",
                )
                .ok()?
        } else {
            return None;
        };
        ctx.builder
            .build_conditional_branch(pred_bool, store_bb, inc_bb)
            .ok()?;

        ctx.builder.position_at_end(store_bb);
        let res_idx = ctx
            .builder
            .build_load(ctx.context.i64_type(), res_idx_alloca, "res_idx")
            .ok()?
            .into_int_value();
        let result_elem_ptr = unsafe {
            ctx.builder
                .build_in_bounds_gep(elem_ty, result_data, &[res_idx], "res_elem")
                .ok()?
        };
        ctx.builder.build_store(result_elem_ptr, elem).ok()?;
        let next_res = ctx
            .builder
            .build_int_add(
                res_idx,
                ctx.context.i64_type().const_int(1, false),
                "next_res",
            )
            .ok()?;
        ctx.builder.build_store(res_idx_alloca, next_res).ok()?;
        ctx.builder.build_unconditional_branch(inc_bb).ok()?;

        ctx.builder.position_at_end(inc_bb);
        let next = ctx
            .builder
            .build_int_add(idx, ctx.context.i64_type().const_int(1, false), "next")
            .ok()?;
        ctx.builder.build_store(idx_alloca, next).ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(end_bb);
        // Update result length (i64) in header at offset 0
        let final_len = ctx
            .builder
            .build_load(ctx.context.i64_type(), res_idx_alloca, "final_len")
            .ok()?
            .into_int_value();
        // Store the final length at header offset 0 (i64)
        set_array_length(ctx, result_heap, final_len)?;

        Some(result_data.into())
    }

    // =========================================================================
    // reduce(init: U, fn: (U, T) -> U) -> U
    // Reduces array to single value
    // =========================================================================
    fn emit_reduce<'ctx>(
        ctx: &mut CodegenContext<'ctx>,
        elem_type_id: TypeId,
        arr_ptr: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.len() < 2 {
            return None;
        }
        let init_val = args[0];
        let closure_ptr = args[1].into_pointer_value();

        let len_i32 = load_len_i32(ctx, arr_ptr)?;
        let len_i64 = ctx
            .builder
            .build_int_z_extend(len_i32, ctx.context.i64_type(), "len64")
            .ok()?;

        // Get the accumulator type from init_val (single source of truth)
        let acc_ty = init_val.get_type();

        // Get the element type from elem_type_id (single source of truth)
        let elem_ty = ctx.get_llvm_type(elem_type_id);

        let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
        let loop_bb = ctx.context.append_basic_block(current_fn, "reduce_loop");
        let body_bb = ctx.context.append_basic_block(current_fn, "reduce_body");
        let end_bb = ctx.context.append_basic_block(current_fn, "reduce_end");

        let idx_alloca = ctx
            .builder
            .build_alloca(ctx.context.i64_type(), "idx")
            .ok()?;
        let acc_alloca = ctx.builder.build_alloca(acc_ty, "acc").ok()?;
        ctx.builder
            .build_store(idx_alloca, ctx.context.i64_type().const_zero())
            .ok()?;
        ctx.builder.build_store(acc_alloca, init_val).ok()?;
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
        let acc = ctx.builder.build_load(acc_ty, acc_alloca, "acc").ok()?;
        let elem_ptr = unsafe {
            ctx.builder
                .build_in_bounds_gep(elem_ty, arr_ptr, &[idx], "elem_ptr")
                .ok()?
        };
        let elem = ctx.builder.build_load(elem_ty, elem_ptr, "elem").ok()?;

        // Call closure with (acc, elem) - reduce returns the accumulator type
        let new_acc = call_closure(ctx, closure_ptr, &[acc, elem], acc_ty)?;
        ctx.builder.build_store(acc_alloca, new_acc).ok()?;

        let next = ctx
            .builder
            .build_int_add(idx, ctx.context.i64_type().const_int(1, false), "next")
            .ok()?;
        ctx.builder.build_store(idx_alloca, next).ok()?;
        ctx.builder.build_unconditional_branch(loop_bb).ok()?;

        ctx.builder.position_at_end(end_bb);
        let result = ctx.builder.build_load(acc_ty, acc_alloca, "result").ok()?;
        Some(result)
    }
}

// =============================================================================
// Helper Functions - Use centralized layout module
// =============================================================================

use crate::layout::{
    alloc_with_header, data_ptr_from_header, get_array_data_ptr, get_array_length,
    header_ptr_from_data, int_to_i64, load_len_i32, set_array_length, set_array_length_from_data,
    store_header, store_header_len_only, store_len, store_len_at_header,
};

fn get_or_declare_malloc<'ctx>(ctx: &CodegenContext<'ctx>) -> FunctionValue<'ctx> {
    ctx.module
        .get_function(ffi_names::MALLOC)
        .unwrap_or_else(|| {
            let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
            let fn_type = ptr_type.fn_type(&[ctx.context.i64_type().into()], false);
            ctx.module.add_function(ffi_names::MALLOC, fn_type, None)
        })
}

fn get_or_declare_strlen<'ctx>(ctx: &CodegenContext<'ctx>) -> FunctionValue<'ctx> {
    ctx.module
        .get_function(ffi_names::STRLEN)
        .unwrap_or_else(|| {
            let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
            let fn_type = ctx.context.i64_type().fn_type(&[ptr_type.into()], false);
            ctx.module.add_function(ffi_names::STRLEN, fn_type, None)
        })
}

fn get_or_declare_memcpy<'ctx>(ctx: &CodegenContext<'ctx>) -> FunctionValue<'ctx> {
    ctx.module
        .get_function(ffi_names::MEMCPY)
        .unwrap_or_else(|| {
            let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
            let fn_type = ptr_type.fn_type(
                &[
                    ptr_type.into(),
                    ptr_type.into(),
                    ctx.context.i64_type().into(),
                ],
                false,
            );
            ctx.module.add_function(ffi_names::MEMCPY, fn_type, None)
        })
}

fn get_or_declare_snprintf<'ctx>(ctx: &CodegenContext<'ctx>) -> FunctionValue<'ctx> {
    ctx.module.get_function("snprintf").unwrap_or_else(|| {
        let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
        let i64_type = ctx.context.i64_type();
        // snprintf(char *str, size_t size, const char *format, ...)
        let fn_type = ctx
            .context
            .i32_type()
            .fn_type(&[ptr_type.into(), i64_type.into(), ptr_type.into()], true);
        ctx.module.add_function("snprintf", fn_type, None)
    })
}

/// Call a closure with given arguments
fn call_closure<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    closure_ptr: PointerValue<'ctx>,
    args: &[BasicValueEnum<'ctx>],
    return_type: inkwell::types::BasicTypeEnum<'ctx>,
) -> Option<BasicValueEnum<'ctx>> {
    let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
    let closure_type = ctx
        .context
        .struct_type(&[ptr_type.into(), ptr_type.into()], false);

    // Load fn_ptr (offset 0)
    let fn_ptr_slot = ctx
        .builder
        .build_struct_gep(closure_type, closure_ptr, 0, "fn_ptr_slot")
        .ok()?;
    let fn_ptr = ctx
        .builder
        .build_load(ptr_type, fn_ptr_slot, "fn_ptr")
        .ok()?
        .into_pointer_value();

    // Load env_ptr (offset 1)
    let env_slot = ctx
        .builder
        .build_struct_gep(closure_type, closure_ptr, 1, "env_slot")
        .ok()?;
    let env_ptr = ctx.builder.build_load(ptr_type, env_slot, "env_ptr").ok()?;

    // Build function type: (env, ...args) -> return_type
    // Use actual argument types for correct function signature
    let param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = std::iter::once(ptr_type.into())
        .chain(args.iter().map(|arg| arg.get_type().into()))
        .collect();
    let fn_type = return_type.fn_type(&param_types, false);

    // Cast fn_ptr to correct function pointer type
    let fn_ptr_typed = ctx
        .builder
        .build_pointer_cast(
            fn_ptr,
            fn_type.ptr_type(AddressSpace::default()),
            "fn_typed",
        )
        .ok()?;

    // Build call args: env, ...user_args
    let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = vec![env_ptr.into()];
    for arg in args {
        call_args.push((*arg).into());
    }

    // Call
    let result = ctx
        .builder
        .build_indirect_call(fn_type, fn_ptr_typed, &call_args, "closure_call")
        .ok()?;
    result.try_as_basic_value().left()
}
