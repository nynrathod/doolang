//! Memory Operations — alloca, GEP, heap allocation, and field access.
//!
//! Centralized helpers for LLVM memory instructions. All heap allocation
//! goes through `doo_alloc` / `doo_free` from `doo_ffi_core`).

use crate::context::CodegenContext;
use doo_core::constants::ffi_names;
use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::AddressSpace;

/// Allocate a local variable (alloca) in the function entry block.
///
/// Creating allocas at the entry block is required for LLVM mem2reg pass
/// and ensures the alloca is valid for the entire function lifetime.
pub fn alloca<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    ty: impl BasicType<'ctx>,
    name: &str,
) -> Option<PointerValue<'ctx>> {
    let current_block = ctx.builder.get_insert_block()?;
    let func = current_block.get_parent()?;

    // Position at entry block for alloca
    let entry = func.get_first_basic_block()?;
    let current_pos = ctx.builder.get_insert_block();

    // Find the position after existing allocas
    let insert_before = entry.get_first_instruction();

    if let Some(first_instr) = insert_before {
        // Walk past existing allocas
        let mut last_alloca = None;
        let mut instr = Some(first_instr);
        while let Some(i) = instr {
            if i.get_opcode() == inkwell::values::InstructionOpcode::Alloca {
                last_alloca = Some(i);
            } else {
                break;
            }
            instr = i.get_next_instruction();
        }

        if let Some(last) = last_alloca {
            if let Some(next) = last.get_next_instruction() {
                ctx.builder.position_before(&next);
            } else {
                ctx.builder.position_at_end(entry);
            }
        } else {
            ctx.builder.position_before(&first_instr);
        }
    } else {
        ctx.builder.position_at_end(entry);
    }

    let result = ctx.builder.build_alloca(ty, name).ok();

    // Restore original position
    if let Some(pos) = current_pos {
        ctx.builder.position_at_end(pos);
    }

    result
}

/// Allocate heap memory via `doo_alloc`.
///
/// Uses the centralized runtime allocator. All heap
/// allocations in generated code go through this function.
pub fn heap_alloc<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    size: inkwell::values::IntValue<'ctx>,
    name: &str,
) -> Option<PointerValue<'ctx>> {
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

    // Zero-initialize to prevent uninitialized fields
    let memset_fn = ctx
        .module
        .get_function(ffi_names::MEMSET)
        .unwrap_or_else(|| {
            let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
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
        "zero_init",
    );

    Some(ptr)
}

/// Free heap memory via `doo_free`.
pub fn heap_free<'ctx>(ctx: &mut CodegenContext<'ctx>, ptr: PointerValue<'ctx>) -> Option<()> {
    let free_fn = ctx
        .module
        .get_function(ffi_names::DOO_FREE)
        .or_else(|| ctx.module.get_function(ffi_names::FREE))?;

    ctx.builder
        .build_call(free_fn, &[ptr.into()], "free")
        .ok()?;
    Some(())
}

/// Get a pointer to a struct field via GEP (GetElementPtr).
pub fn struct_gep<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    struct_ty: inkwell::types::StructType<'ctx>,
    ptr: PointerValue<'ctx>,
    field_idx: u32,
    name: &str,
) -> Option<PointerValue<'ctx>> {
    ctx.builder
        .build_struct_gep(struct_ty, ptr, field_idx, name)
        .ok()
}

/// Get a pointer to an array element via GEP.
pub fn array_gep<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    elem_ty: impl BasicType<'ctx>,
    base: PointerValue<'ctx>,
    index: inkwell::values::IntValue<'ctx>,
    name: &str,
) -> Option<PointerValue<'ctx>> {
    unsafe { ctx.builder.build_gep(elem_ty, base, &[index], name) }.ok()
}

/// Load a value from a pointer.
pub fn load<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    ty: impl BasicType<'ctx>,
    ptr: PointerValue<'ctx>,
    name: &str,
) -> Option<BasicValueEnum<'ctx>> {
    ctx.builder.build_load(ty, ptr, name).ok()
}

/// Store a value to a pointer.
pub fn store<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    ptr: PointerValue<'ctx>,
    value: BasicValueEnum<'ctx>,
) -> Option<()> {
    ctx.builder.build_store(ptr, value).ok()?;
    Some(())
}
