//! Layout Utilities - DRY Helpers for Array/Map Header Access
//!
//! Centralized helper functions to eliminate duplicate GEP logic across
//! collections.rs, array.rs, map.rs and other builtin modules.
//!
//! ## Array Layout
//! ```text
//! struct Array {
//!     i64 length;
//!     i64 capacity;  
//!     [T] data;
//! }
//! ```
//!
//! ## Map Layout  
//! ```text
//! struct Map {
//!     i64 length;
//!     i64 capacity;
//!     [Entry] data;  // Entry = {key, value}
//! }
//! ```

use crate::context::CodegenContext;
use doo_core::constants::ffi_names;
use inkwell::types::BasicType;
use inkwell::values::{IntValue, PointerValue};
use inkwell::IntPredicate;

/// Map entry size in bytes: each entry stores key (8 bytes) + value (8 bytes).
/// Currently all Doo map keys/values are represented as either i64 or pointer,
/// both of which are 8 bytes on 64-bit systems. If we ever support different-sized
/// key/value types, this must become dynamic based on key_type.size_of() + value_type.size_of().
pub const MAP_ENTRY_SIZE: u64 = 16;

/// Compute map entry size from key and value TypeIds.
/// Currently returns the constant MAP_ENTRY_SIZE (16) since all Doo values are 8 bytes.
/// When struct/tuple map values are supported, this should query the TypeRegistry for
/// actual sizes: `type_registry.size_bytes(key_ty) + type_registry.size_bytes(val_ty)`.
pub fn map_entry_size_for_types(
    _key_ty: doo_core::types::TypeId,
    _val_ty: doo_core::types::TypeId,
) -> u64 {
    // All current Doo types are 8 bytes (i64, f64, or pointer).
    // When different-sized types are supported, compute dynamically from TypeRegistry.
    MAP_ENTRY_SIZE
}

// ============================================================================
// Array Layout Helpers
// ============================================================================

/// Get pointer to array length field (index 0)
pub fn get_array_length_ptr<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    arr_ptr: PointerValue<'ctx>,
) -> Option<PointerValue<'ctx>> {
    unsafe {
        ctx.builder
            .build_gep(
                ctx.context.i64_type(),
                arr_ptr,
                &[ctx.context.i64_type().const_int(0, false)],
                "arr_len_ptr",
            )
            .ok()
    }
}

/// Get array length value
pub fn get_array_length<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    arr_ptr: PointerValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    let len_ptr = get_array_length_ptr(ctx, arr_ptr)?;
    ctx.builder
        .build_load(ctx.context.i64_type(), len_ptr, "arr_len")
        .ok()?
        .into_int_value()
        .into()
}

/// Set array length value
pub fn set_array_length<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    arr_ptr: PointerValue<'ctx>,
    new_len: IntValue<'ctx>,
) -> Option<()> {
    let len_ptr = get_array_length_ptr(ctx, arr_ptr)?;
    ctx.builder.build_store(len_ptr, new_len).ok()?;
    Some(())
}

/// Get pointer to array capacity field (index 1)
pub fn get_array_capacity_ptr<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    arr_ptr: PointerValue<'ctx>,
) -> Option<PointerValue<'ctx>> {
    unsafe {
        ctx.builder
            .build_gep(
                ctx.context.i64_type(),
                arr_ptr,
                &[ctx.context.i64_type().const_int(1, false)],
                "arr_cap_ptr",
            )
            .ok()
    }
}

/// Get array capacity value
pub fn get_array_capacity<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    arr_ptr: PointerValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    let cap_ptr = get_array_capacity_ptr(ctx, arr_ptr)?;
    ctx.builder
        .build_load(ctx.context.i64_type(), cap_ptr, "arr_cap")
        .ok()?
        .into_int_value()
        .into()
}

/// Set array capacity value
pub fn set_array_capacity<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    arr_ptr: PointerValue<'ctx>,
    new_cap: IntValue<'ctx>,
) -> Option<()> {
    let cap_ptr = get_array_capacity_ptr(ctx, arr_ptr)?;
    ctx.builder.build_store(cap_ptr, new_cap).ok()?;
    Some(())
}

/// Get pointer to array data start (index 2)
pub fn get_array_data_ptr<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    arr_ptr: PointerValue<'ctx>,
    elem_type: impl BasicType<'ctx>,
) -> Option<PointerValue<'ctx>> {
    unsafe {
        ctx.builder
            .build_gep(
                elem_type,
                arr_ptr,
                &[ctx.context.i64_type().const_int(2, false)],
                "arr_data_ptr",
            )
            .ok()
    }
}

/// Get pointer to specific array element (data_base + index)
pub fn get_array_element_ptr<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    arr_ptr: PointerValue<'ctx>,
    index: IntValue<'ctx>,
    elem_type: impl BasicType<'ctx> + Copy,
) -> Option<PointerValue<'ctx>> {
    let data_ptr = get_array_data_ptr(ctx, arr_ptr, elem_type)?;
    unsafe {
        ctx.builder
            .build_gep(elem_type, data_ptr, &[index], "arr_elem_ptr")
            .ok()
    }
}

/// Calculate array header size (2 * i64 = 16 bytes)
pub fn array_header_size<'ctx>(ctx: &CodegenContext<'ctx>) -> IntValue<'ctx> {
    ctx.context.i64_type().const_int(16, false)
}

/// Get header pointer from data pointer (data_ptr - 16)
/// Use this when you have a data pointer and need to access length/capacity
pub fn header_ptr_from_data<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    data_ptr: PointerValue<'ctx>,
) -> Option<PointerValue<'ctx>> {
    // -16 to go from data to header
    // CRITICAL: Use i64 for negative offset, cast -16i64 to u64 preserves two's complement representation
    // NOTE: Must NOT use inbounds — clone code produces PHIs with null data pointers,
    // and inbounds GEP on null is UB that LLVM O3 exploits to remove loop exits.
    let offset = ctx.context.i64_type().const_int(-16i64 as u64, true);
    unsafe {
        ctx.builder
            .build_gep(ctx.context.i8_type(), data_ptr, &[offset], "header_ptr")
            .ok()
    }
}

/// Get array length from a DATA pointer (not header pointer)
/// This is the primary function to use when you have the data ptr returned by alloc_with_header
pub fn get_array_length_from_data<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    data_ptr: PointerValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    let is_null = ctx.builder.build_is_null(data_ptr, "is_null").ok()?;
    let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
    let null_block = ctx.context.append_basic_block(current_fn, "arr_len_null");
    let ok_block = ctx.context.append_basic_block(current_fn, "arr_len_ok");
    let done_block = ctx.context.append_basic_block(current_fn, "arr_len_done");
    
    ctx.builder.build_conditional_branch(is_null, null_block, ok_block).ok()?;
    
    // Null block: return 0
    ctx.builder.position_at_end(null_block);
    let zero = ctx.context.i64_type().const_zero();
    ctx.builder.build_unconditional_branch(done_block).ok()?;
    
    // Ok block: load from header
    ctx.builder.position_at_end(ok_block);
    let header_ptr = header_ptr_from_data(ctx, data_ptr)?;
    let len = get_array_length(ctx, header_ptr)?;
    ctx.builder.build_unconditional_branch(done_block).ok()?;
    
    // Done block: phi
    ctx.builder.position_at_end(done_block);
    let phi = ctx.builder.build_phi(ctx.context.i64_type(), "arr_len_res").ok()?;
    phi.add_incoming(&[(&zero, null_block), (&len, ok_block)]);
    
    Some(phi.as_basic_value().into_int_value())
}

/// Get array capacity from a DATA pointer
pub fn get_array_capacity_from_data<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    data_ptr: PointerValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    let header_ptr = header_ptr_from_data(ctx, data_ptr)?;
    get_array_capacity(ctx, header_ptr)
}

/// Set array length from a DATA pointer
pub fn set_array_length_from_data<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    data_ptr: PointerValue<'ctx>,
    new_len: IntValue<'ctx>,
) -> Option<()> {
    let header_ptr = header_ptr_from_data(ctx, data_ptr)?;
    set_array_length(ctx, header_ptr, new_len)
}

// ============================================================================
// Map Layout Helpers
// ============================================================================

/// Get pointer to map length field (index 0)
pub fn get_map_length_ptr<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    map_ptr: PointerValue<'ctx>,
) -> Option<PointerValue<'ctx>> {
    unsafe {
        ctx.builder
            .build_gep(
                ctx.context.i64_type(),
                map_ptr,
                &[ctx.context.i64_type().const_int(0, false)],
                "map_len_ptr",
            )
            .ok()
    }
}

/// Get map length value
pub fn get_map_length<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    map_ptr: PointerValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    let len_ptr = get_map_length_ptr(ctx, map_ptr)?;
    ctx.builder
        .build_load(ctx.context.i64_type(), len_ptr, "map_len")
        .ok()?
        .into_int_value()
        .into()
}

/// Set map length value
pub fn set_map_length<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    map_ptr: PointerValue<'ctx>,
    new_len: IntValue<'ctx>,
) -> Option<()> {
    let len_ptr = get_map_length_ptr(ctx, map_ptr)?;
    ctx.builder.build_store(len_ptr, new_len).ok()?;
    Some(())
}

/// Get pointer to map capacity field (index 1)
pub fn get_map_capacity_ptr<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    map_ptr: PointerValue<'ctx>,
) -> Option<PointerValue<'ctx>> {
    unsafe {
        ctx.builder
            .build_gep(
                ctx.context.i64_type(),
                map_ptr,
                &[ctx.context.i64_type().const_int(1, false)],
                "map_cap_ptr",
            )
            .ok()
    }
}

/// Get map capacity value
pub fn get_map_capacity<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    map_ptr: PointerValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    let cap_ptr = get_map_capacity_ptr(ctx, map_ptr)?;
    ctx.builder
        .build_load(ctx.context.i64_type(), cap_ptr, "map_cap")
        .ok()?
        .into_int_value()
        .into()
}

/// Set map capacity value
pub fn set_map_capacity<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    map_ptr: PointerValue<'ctx>,
    new_cap: IntValue<'ctx>,
) -> Option<()> {
    let cap_ptr = get_map_capacity_ptr(ctx, map_ptr)?;
    ctx.builder.build_store(cap_ptr, new_cap).ok()?;
    Some(())
}

/// Get pointer to map data start (index 2)
/// NOTE: This function assumes map_ptr is a HEADER pointer.
/// For data pointers returned by alloc_with_header, this is not needed
/// since alloc_with_header already returns the data pointer.
pub fn get_map_data_ptr<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    map_ptr: PointerValue<'ctx>,
) -> Option<PointerValue<'ctx>> {
    unsafe {
        ctx.builder
            .build_gep(
                ctx.context.i8_type(),
                map_ptr,
                &[ctx.context.i64_type().const_int(16, false)],
                "map_data_ptr",
            )
            .ok()
    }
}

/// Get map length from a DATA pointer (not header pointer)
/// This is the primary function to use when you have the data ptr returned by alloc_with_header
pub fn get_map_length_from_data<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    data_ptr: PointerValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    let header_ptr = header_ptr_from_data(ctx, data_ptr)?;
    get_map_length(ctx, header_ptr)
}

/// Set map length from a DATA pointer
pub fn set_map_length_from_data<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    data_ptr: PointerValue<'ctx>,
    new_len: IntValue<'ctx>,
) -> Option<()> {
    let header_ptr = header_ptr_from_data(ctx, data_ptr)?;
    set_map_length(ctx, header_ptr, new_len)
}

/// Get map capacity from a DATA pointer
pub fn get_map_capacity_from_data<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    data_ptr: PointerValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    let header_ptr = header_ptr_from_data(ctx, data_ptr)?;
    get_map_capacity(ctx, header_ptr)
}

/// Calculate map header size (2 * i64 = 16 bytes)
pub fn map_header_size<'ctx>(ctx: &CodegenContext<'ctx>) -> IntValue<'ctx> {
    ctx.context.i64_type().const_int(16, false)
}

// ============================================================================
// Common Allocation Helpers
// ============================================================================

// ============================================================================
// Overflow-Safe Arithmetic for Allocation Sizes
// ============================================================================

/// Perform checked multiplication for allocation sizes.
/// If the multiplication overflows, aborts the program with an error message
/// instead of silently wrapping around (which would cause undersized allocations
/// and heap buffer overflows).
///
/// Uses LLVM's `@llvm.umul.with.overflow.i64` intrinsic for zero-cost overflow detection.
pub fn checked_mul_or_abort<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    lhs: IntValue<'ctx>,
    rhs: IntValue<'ctx>,
    name: &str,
) -> Option<IntValue<'ctx>> {
    let i64_type = ctx.context.i64_type();

    // Ensure both operands are i64
    let lhs_i64 = if lhs.get_type().get_bit_width() != 64 {
        ctx.builder
            .build_int_z_extend(lhs, i64_type, "lhs_i64")
            .ok()?
    } else {
        lhs
    };
    let rhs_i64 = if rhs.get_type().get_bit_width() != 64 {
        ctx.builder
            .build_int_z_extend(rhs, i64_type, "rhs_i64")
            .ok()?
    } else {
        rhs
    };

    // Declare @llvm.umul.with.overflow.i64 intrinsic
    let overflow_struct_type = ctx
        .context
        .struct_type(&[i64_type.into(), ctx.context.bool_type().into()], false);
    let intrinsic_type = overflow_struct_type.fn_type(&[i64_type.into(), i64_type.into()], false);
    let intrinsic = ctx
        .module
        .get_function("llvm.umul.with.overflow.i64")
        .unwrap_or_else(|| {
            ctx.module
                .add_function("llvm.umul.with.overflow.i64", intrinsic_type, None)
        });

    // Call the intrinsic: returns { i64 result, i1 overflow }
    let call_result = ctx
        .builder
        .build_call(
            intrinsic,
            &[lhs_i64.into(), rhs_i64.into()],
            &format!("{}_overflow", name),
        )
        .ok()?
        .try_as_basic_value()
        .basic()?;

    // Extract result and overflow flag
    let mul_result = ctx
        .builder
        .build_extract_value(call_result.into_struct_value(), 0, name)
        .ok()?
        .into_int_value();
    let did_overflow = ctx
        .builder
        .build_extract_value(
            call_result.into_struct_value(),
            1,
            &format!("{}_flag", name),
        )
        .ok()?
        .into_int_value();

    // Branch: if overflow, abort; else continue
    let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
    let overflow_bb = ctx
        .context
        .append_basic_block(current_fn, &format!("{}_overflow_abort", name));
    let ok_bb = ctx
        .context
        .append_basic_block(current_fn, &format!("{}_ok", name));

    ctx.builder
        .build_conditional_branch(did_overflow, overflow_bb, ok_bb)
        .ok()?;

    // Overflow block: print error and abort
    ctx.builder.position_at_end(overflow_bb);
    emit_allocation_overflow_abort(ctx, "allocation size overflow: multiplication overflow");
    // Branch to ok_bb: __doo_abort terminates the process, but LLVM must
    // see this path as returnable to prevent noreturn inference.
    let _ = ctx.builder.build_unconditional_branch(ok_bb);

    // Continue in the ok block
    ctx.builder.position_at_end(ok_bb);

    Some(mul_result)
}

/// Perform checked addition for allocation sizes.
/// If the addition overflows, aborts the program with an error message.
///
/// Uses LLVM's `@llvm.uadd.with.overflow.i64` intrinsic for zero-cost overflow detection.
pub fn checked_add_or_abort<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    lhs: IntValue<'ctx>,
    rhs: IntValue<'ctx>,
    name: &str,
) -> Option<IntValue<'ctx>> {
    let i64_type = ctx.context.i64_type();

    // Ensure both operands are i64
    let lhs_i64 = if lhs.get_type().get_bit_width() != 64 {
        ctx.builder
            .build_int_z_extend(lhs, i64_type, "lhs_i64")
            .ok()?
    } else {
        lhs
    };
    let rhs_i64 = if rhs.get_type().get_bit_width() != 64 {
        ctx.builder
            .build_int_z_extend(rhs, i64_type, "rhs_i64")
            .ok()?
    } else {
        rhs
    };

    // Declare @llvm.uadd.with.overflow.i64 intrinsic
    let overflow_struct_type = ctx
        .context
        .struct_type(&[i64_type.into(), ctx.context.bool_type().into()], false);
    let intrinsic_type = overflow_struct_type.fn_type(&[i64_type.into(), i64_type.into()], false);
    let intrinsic = ctx
        .module
        .get_function("llvm.uadd.with.overflow.i64")
        .unwrap_or_else(|| {
            ctx.module
                .add_function("llvm.uadd.with.overflow.i64", intrinsic_type, None)
        });

    // Call the intrinsic: returns { i64 result, i1 overflow }
    let call_result = ctx
        .builder
        .build_call(
            intrinsic,
            &[lhs_i64.into(), rhs_i64.into()],
            &format!("{}_overflow", name),
        )
        .ok()?
        .try_as_basic_value()
        .basic()?;

    // Extract result and overflow flag
    let add_result = ctx
        .builder
        .build_extract_value(call_result.into_struct_value(), 0, name)
        .ok()?
        .into_int_value();
    let did_overflow = ctx
        .builder
        .build_extract_value(
            call_result.into_struct_value(),
            1,
            &format!("{}_flag", name),
        )
        .ok()?
        .into_int_value();

    // Branch: if overflow, abort; else continue
    let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
    let overflow_bb = ctx
        .context
        .append_basic_block(current_fn, &format!("{}_overflow_abort", name));
    let ok_bb = ctx
        .context
        .append_basic_block(current_fn, &format!("{}_ok", name));

    ctx.builder
        .build_conditional_branch(did_overflow, overflow_bb, ok_bb)
        .ok()?;

    // Overflow block: print error and abort
    ctx.builder.position_at_end(overflow_bb);
    emit_allocation_overflow_abort(ctx, "allocation size overflow: addition overflow");
    // Branch to ok_bb: __doo_abort terminates the process, but LLVM must
    // see this path as returnable to prevent noreturn inference.
    let _ = ctx.builder.build_unconditional_branch(ok_bb);

    // Continue in the ok block
    ctx.builder.position_at_end(ok_bb);

    Some(add_result)
}

/// Emit code to print an error message and call exit(1).
/// Used by overflow check helpers to abort on allocation size overflow.
fn emit_allocation_overflow_abort<'ctx>(ctx: &mut CodegenContext<'ctx>, message: &str) {
    // Get or declare fprintf(stderr, ...)
    let printf_type = ctx.context.i32_type().fn_type(
        &[ctx
            .context
            .i8_type()
            .ptr_type(inkwell::AddressSpace::default())
            .into()],
        true,
    );
    let printf = ctx
        .module
        .get_function(ffi_names::PRINTF)
        .unwrap_or_else(|| {
            ctx.module
                .add_function(ffi_names::PRINTF, printf_type, None)
        });

    let error_msg = ctx.const_string(&format!("fatal: {}\n", message));
    let _ = ctx
        .builder
        .build_call(printf, &[error_msg.into()], "print_overflow_err");

    // Flush stdout
    let fflush_type = ctx.context.i32_type().fn_type(
        &[ctx
            .context
            .i8_type()
            .ptr_type(inkwell::AddressSpace::default())
            .into()],
        false,
    );
    let fflush = ctx
        .module
        .get_function(ffi_names::FFLUSH)
        .unwrap_or_else(|| {
            ctx.module
                .add_function(ffi_names::FFLUSH, fflush_type, None)
        });
    let null_ptr = ctx
        .context
        .i8_type()
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    let _ = ctx.builder.build_call(fflush, &[null_ptr.into()], "flush");

    // Use __doo_abort() instead of exit() directly — see get_or_create_doo_abort docs.
    let abort_fn = ctx.get_or_create_doo_abort();
    let exit_code = ctx.context.i32_type().const_int(1, false);
    let _ = ctx
        .builder
        .build_call(abort_fn, &[exit_code.into()], "abort_overflow");
}

/// Allocate memory using centralized `doo_alloc`
/// SAFETY: Zero-initializes all allocated memory to prevent reading garbage
/// (including -1/0xFFFFFFFFFFFFFFFF) from uninitialized struct fields.
pub fn alloc_memory<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    size: IntValue<'ctx>,
    name: &str,
) -> Option<PointerValue<'ctx>> {
    use doo_core::constants::ffi_names;

    let alloc_fn = ctx
        .module
        .get_function(ffi_names::DOO_ALLOC)
        .or_else(|| ctx.module.get_function(ffi_names::MALLOC))?;

    let ptr = ctx
        .builder
        .build_call(alloc_fn, &[size.into()], name)
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_pointer_value();

    // Zero the allocated memory to prevent uninitialized pointer fields
    // from containing garbage values like -1 that crash on dereference.
    let memset_fn = ctx
        .module
        .get_function(ffi_names::MEMSET)
        .unwrap_or_else(|| {
            let ptr_type = ctx
                .context
                .i8_type()
                .ptr_type(inkwell::AddressSpace::default());
            let fn_ty = ptr_type.fn_type(
                &[
                    ptr_type.into(),
                    ctx.context.i32_type().into(),
                    ctx.context.i64_type().into(),
                ],
                false,
            );
            ctx.module.add_function(ffi_names::MEMSET, fn_ty, None)
        });
    let _ = ctx.builder.build_call(
        memset_fn,
        &[
            ptr.into(),
            ctx.context.i32_type().const_zero().into(),
            size.into(),
        ],
        "",
    );

    Some(ptr)
}

/// Free memory using centralized `doo_free`
pub fn free_memory<'ctx>(ctx: &mut CodegenContext<'ctx>, ptr: PointerValue<'ctx>) -> Option<()> {
    use doo_core::constants::ffi_names;

    let free_fn = ctx
        .module
        .get_function(ffi_names::DOO_FREE)
        .or_else(|| ctx.module.get_function(ffi_names::FREE))?;

    ctx.builder.build_call(free_fn, &[ptr.into()], "").ok()?;

    Some(())
}

/// Copy memory using memcpy
pub fn copy_memory<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    dest: PointerValue<'ctx>,
    src: PointerValue<'ctx>,
    size: IntValue<'ctx>,
) -> Option<()> {
    use doo_core::constants::ffi_names;

    let memcpy_fn = ctx.module.get_function(ffi_names::MEMCPY)?;

    ctx.builder
        .build_call(memcpy_fn, &[dest.into(), src.into(), size.into()], "")
        .ok()?;

    Some(())
}

// ============================================================================
// Growth Strategy Helpers
// ============================================================================

/// Calculate new capacity using growth factor (2x)
pub fn calculate_new_capacity<'ctx>(
    ctx: &CodegenContext<'ctx>,
    current_cap: IntValue<'ctx>,
    min_required: IntValue<'ctx>,
) -> IntValue<'ctx> {
    // new_cap = max(current * 2, min_required)
    let doubled = ctx
        .builder
        .build_int_mul(
            current_cap,
            ctx.context.i64_type().const_int(2, false),
            "doubled_cap",
        )
        .ok();

    if let Some(doubled_cap) = doubled {
        let cmp = ctx
            .builder
            .build_int_compare(
                IntPredicate::UGT,
                doubled_cap,
                min_required,
                "is_doubled_enough",
            )
            .ok();

        if let Some(cond) = cmp {
            return ctx
                .builder
                .build_select(cond, doubled_cap, min_required, "new_cap")
                .ok()
                .and_then(|v| v.into_int_value().into())
                .unwrap_or(min_required);
        }
    }

    min_required
}

// ============================================================================
// Array Reallocation (for push, etc.)
// ============================================================================

/// Reallocate array to accommodate new length
/// IMPORTANT: arr_ptr must be a DATA pointer (returned by alloc_with_header)
/// Returns new DATA pointer with updated header
pub fn realloc_array_capacity<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    data_ptr: PointerValue<'ctx>,
    new_len: IntValue<'ctx>,
    elem_size: IntValue<'ctx>,
) -> Option<PointerValue<'ctx>> {
    use doo_core::constants::ffi_names;

    // Get header pointer from data pointer
    let header_ptr = header_ptr_from_data(ctx, data_ptr)?;

    // Get current capacity from header
    let current_cap = get_array_capacity(ctx, header_ptr)?;

    // Convert new_len to i64
    let new_len_i64 = if new_len.get_type().get_bit_width() == 32 {
        ctx.builder
            .build_int_z_extend(new_len, ctx.context.i64_type(), "new_len64")
            .ok()?
    } else {
        new_len
    };

    // Check if we need to grow
    let needs_grow = ctx
        .builder
        .build_int_compare(IntPredicate::UGT, new_len_i64, current_cap, "needs_grow")
        .ok()?;

    let current_fn = ctx.builder.get_insert_block()?.get_parent()?;
    let grow_bb = ctx.context.append_basic_block(current_fn, "arr_grow");
    let done_bb = ctx.context.append_basic_block(current_fn, "arr_done");

    let result_alloca = ctx.alloca_in_entry_block(
        ctx.context
            .i8_type()
            .ptr_type(inkwell::AddressSpace::default()),
        "arr_result",
    )?;
    // Store the current DATA pointer
    ctx.builder.build_store(result_alloca, data_ptr).ok()?;

    ctx.builder
        .build_conditional_branch(needs_grow, grow_bb, done_bb)
        .ok()?;

    // Grow path
    ctx.builder.position_at_end(grow_bb);

    // new_cap = max(current * 2, new_len)
    let new_cap = calculate_new_capacity(ctx, current_cap, new_len_i64);

    // Calculate new total size: header (16) + (capacity * elem_size)
    // CRITICAL: Use checked arithmetic to prevent integer overflow.
    // Overflow here would cause undersized allocation → heap buffer overflow.
    let header_size = array_header_size(ctx);
    let data_size = checked_mul_or_abort(ctx, new_cap, elem_size, "data_size")?;
    let total_size = checked_add_or_abort(ctx, header_size, data_size, "total_size")?;

    // Reallocate the HEADER pointer, not the data pointer
    let realloc_fn = ctx
        .module
        .get_function(ffi_names::DOO_REALLOC)
        .or_else(|| ctx.module.get_function(ffi_names::REALLOC))?;

    let new_header_ptr = ctx
        .builder
        .build_call(
            realloc_fn,
            &[header_ptr.into(), total_size.into()],
            "new_arr_header",
        )
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_pointer_value();

    // Update capacity in new header
    set_array_capacity(ctx, new_header_ptr, new_cap)?;

    // Calculate new data pointer from new header
    let new_data_ptr = unsafe {
        ctx.builder
            .build_gep(
                ctx.context.i8_type(),
                new_header_ptr,
                &[header_size],
                "new_arr_data",
            )
            .ok()?
    };

    ctx.builder.build_store(result_alloca, new_data_ptr).ok()?;
    ctx.builder.build_unconditional_branch(done_bb).ok()?;

    // Done
    ctx.builder.position_at_end(done_bb);
    let result_data_ptr = ctx
        .builder
        .build_load(
            ctx.context
                .i8_type()
                .ptr_type(inkwell::AddressSpace::default()),
            result_alloca,
            "arr_data",
        )
        .ok()?
        .into_pointer_value();

    // Update length in header (go back from data to header)
    let final_header_ptr = header_ptr_from_data(ctx, result_data_ptr)?;
    set_array_length(ctx, final_header_ptr, new_len_i64)?;

    Some(result_data_ptr)
}

/// Minimum allocation size for arrays/maps.
/// Even empty arrays need header (16 bytes) + some data space to ensure
/// the data pointer (header + 16) is within allocated memory.
/// This prevents crashes when accessing empty arrays.
pub const MIN_ALLOCATION_SIZE: u64 = 32;

/// Allocate array with header (length + capacity fields)
/// Returns the DATA pointer (offset +16 from header), NOT the header pointer.
/// This allows consumers to directly index elements at [0], [1], etc.
/// Layout: [header: 16 bytes][data...]
///         ^header_ptr      ^data_ptr (returned)
///
/// IMPORTANT: Always allocates at least MIN_ALLOCATION_SIZE bytes to ensure
/// the returned data pointer is within valid allocated memory, even for empty arrays.
pub fn alloc_with_header<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    len: IntValue<'ctx>,
    elem_type: impl BasicType<'ctx>,
    name: &str,
) -> Option<PointerValue<'ctx>> {
    let len_i64 = if len.get_type().get_bit_width() == 32 {
        ctx.builder
            .build_int_z_extend(len, ctx.context.i64_type(), "len64")
            .ok()?
    } else {
        len
    };

    let elem_size = elem_type
        .size_of()
        .unwrap_or(ctx.context.i64_type().const_int(8, false));
    let header_size = array_header_size(ctx);
    // CRITICAL: Use checked arithmetic to prevent integer overflow.
    // Overflow here would cause undersized allocation → heap buffer overflow.
    let data_size = checked_mul_or_abort(ctx, len_i64, elem_size, "data_size")?;
    let computed_size = checked_add_or_abort(ctx, header_size, data_size, "computed_size")?;

    // Ensure minimum allocation size to prevent empty array crash
    // The data pointer (header + 16) must be within allocated memory
    let min_size = ctx.context.i64_type().const_int(MIN_ALLOCATION_SIZE, false);
    let needs_min = ctx
        .builder
        .build_int_compare(IntPredicate::ULT, computed_size, min_size, "needs_min")
        .ok()?;
    let total_size = ctx
        .builder
        .build_select(needs_min, min_size, computed_size, "total_size")
        .ok()?
        .into_int_value();

    let header_ptr = alloc_memory(ctx, total_size, &format!("{}_header", name))?;

    // Initialize header at header_ptr
    set_array_length(ctx, header_ptr, len_i64)?;
    set_array_capacity(ctx, header_ptr, len_i64)?;

    // Return data pointer (header_ptr + 16)
    let data_ptr = unsafe {
        ctx.builder
            .build_gep(
                ctx.context.i8_type(),
                header_ptr,
                &[header_size],
                &format!("{}_data", name),
            )
            .ok()?
    };

    Some(data_ptr)
}

// ============================================================================
// Utility Functions for Type Conversions
// ============================================================================

/// Convert any int type to i64
pub fn int_to_i64<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: IntValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    let bit_width = val.get_type().get_bit_width();
    if bit_width < 64 {
        ctx.builder
            .build_int_z_extend(val, ctx.context.i64_type(), "i64")
            .ok()
    } else if bit_width > 64 {
        ctx.builder
            .build_int_truncate(val, ctx.context.i64_type(), "i64")
            .ok()
    } else {
        Some(val)
    }
}

/// Convert i64 to i32
pub fn i64_to_i32<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: IntValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    ctx.builder
        .build_int_truncate(val, ctx.context.i32_type(), "i32")
        .ok()
}

// ============================================================================
// Legacy Compatibility Functions
// ============================================================================

/// Load length as i32 from a DATA pointer.
/// For arrays/maps where alloc_with_header returns a data pointer,
/// this function goes back to the header to retrieve the length.
pub fn load_len_i32<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    data_ptr: PointerValue<'ctx>,
) -> Option<IntValue<'ctx>> {
    // Go back to header to get length (i64 at offset -16)
    let len_i64 = get_array_length_from_data(ctx, data_ptr)?;
    // Truncate to i32 for compatibility
    ctx.builder
        .build_int_truncate(len_i64, ctx.context.i32_type(), "len_i32")
        .ok()
}

/// Get data pointer from a header pointer (for arrays/maps with header layout).
/// Assumes header is 16 bytes (2 x i64: length + capacity).
pub fn data_ptr_from_header<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    header_ptr: PointerValue<'ctx>,
) -> Option<PointerValue<'ctx>> {
    let header_size = ctx.context.i64_type().const_int(16, false);
    let data_ptr = unsafe {
        ctx.builder
            .build_gep(
                ctx.context.i8_type(),
                header_ptr,
                &[header_size],
                "data_ptr",
            )
            .ok()?
    };
    Some(data_ptr)
}

/// Store length value at header (first i64).
pub fn store_len_at_header<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    header_ptr: PointerValue<'ctx>,
    len: IntValue<'ctx>,
) -> Option<()> {
    ctx.builder.build_store(header_ptr, len).ok()?;
    Some(())
}

/// Store a length value (generic helper).
pub fn store_len<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    ptr: PointerValue<'ctx>,
    len: IntValue<'ctx>,
) -> Option<()> {
    ctx.builder.build_store(ptr, len).ok()?;
    Some(())
}

/// Store header values (length and capacity).
pub fn store_header<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    header_ptr: PointerValue<'ctx>,
    len: IntValue<'ctx>,
    cap: IntValue<'ctx>,
) -> Option<()> {
    // Store length at offset 0
    ctx.builder.build_store(header_ptr, len).ok()?;

    // Store capacity at offset 8 (after length)
    let cap_ptr = unsafe {
        ctx.builder
            .build_gep(
                ctx.context.i64_type(),
                header_ptr,
                &[ctx.context.i64_type().const_int(1, false)],
                "cap_ptr",
            )
            .ok()?
    };
    ctx.builder.build_store(cap_ptr, cap).ok()?;
    Some(())
}

/// Store just the length at header (for single-field headers).
/// Legacy compatibility: some older code uses single-field RC+len headers.
pub fn store_header_len_only<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    header_ptr: PointerValue<'ctx>,
    len: IntValue<'ctx>,
) -> Option<()> {
    ctx.builder.build_store(header_ptr, len).ok()?;
    Some(())
}
