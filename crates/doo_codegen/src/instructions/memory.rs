//! Memory Instruction Handler
//!
//! Handles: Move, Copy, Clone, Assign, Drop, Borrow

use doo_core::constants::ffi_names;
use doo_core::types::TypeKind;
use doo_mir::{MirInstr, MirInstrKind, MirOperand};
use inkwell::values::BasicValueEnum;

use super::InstructionHandler;
use crate::context::CodegenContext;
use crate::utils::operand_to_value;

/// Memory instruction handler.
pub struct MemoryHandler;

impl<'ctx> InstructionHandler<'ctx> for MemoryHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(
            instr.kind,
            MirInstrKind::Move { .. }
                | MirInstrKind::Copy { .. }
                | MirInstrKind::Clone { .. }
                | MirInstrKind::Assign { .. }
                | MirInstrKind::Drop { .. }
                | MirInstrKind::Borrow { .. }
        )
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::Move { dest, src } => {
                // Move is a simple copy at LLVM level (ownership is compile-time)
                let val = operand_to_value(ctx, src)?;
                // Propagate struct type association if present
                if let Some(src_name) = get_operand_name(src) {
                    if let Some(struct_name) = ctx.get_temp_struct_type(src_name).cloned() {
                        ctx.set_temp_struct_type(dest, &struct_name);
                    }
                }
                ctx.set_local(dest.clone(), val);
                Some(val)
            }
            MirInstrKind::Copy { dest, src } => {
                // Copy is also a simple copy at LLVM level
                let val = operand_to_value(ctx, src)?;
                // Propagate struct type association if present
                if let Some(src_name) = get_operand_name(src) {
                    if let Some(struct_name) = ctx.get_temp_struct_type(src_name).cloned() {
                        ctx.set_temp_struct_type(dest, &struct_name);
                    }
                }
                ctx.set_local(dest.clone(), val);
                Some(val)
            }
            MirInstrKind::Clone { dest, src } => {
                // Deep clone based on type
                emit_deep_clone(ctx, dest, src)
            }
            MirInstrKind::Assign { dest, value } => {
                let val = operand_to_value(ctx, value)?;
                // Propagate struct type association if present
                if let Some(src_name) = get_operand_name(value) {
                    if let Some(struct_name) = ctx.get_temp_struct_type(src_name).cloned() {
                        ctx.set_temp_struct_type(dest, &struct_name);
                    }
                    // Also propagate the variable type from source to dest
                    // This ensures that when a temp with known type is assigned to a local,
                    // the local gets the correct type for subsequent Clone operations
                    if let Some(src_type) = ctx.get_variable_type(src_name) {
                        ctx.set_variable_type(dest, src_type);
                    }
                }
                ctx.set_local(dest.clone(), val);
                Some(val)
            }
            MirInstrKind::Drop { value } => {
                // Actual cleanup based on type
                emit_drop(ctx, value);
                None
            }
            MirInstrKind::Borrow { dest, src, .. } => {
                // Borrow at LLVM level is just getting the value
                let val = ctx.get_value(src.as_str())?;
                ctx.set_local(dest.clone(), val);
                Some(val)
            }
            _ => None,
        }
    }
}

// ============================================================================
// Deep Clone Implementation
// ============================================================================

/// Get the variable name from a MirOperand.
fn get_operand_name(operand: &MirOperand) -> Option<&str> {
    match operand {
        MirOperand::Local(name) | MirOperand::Temp(name) | MirOperand::Global(name) => {
            Some(name.as_str())
        }
        MirOperand::Const(_) => None,
    }
}

/// Emit deep clone for a value based on its type.
fn emit_deep_clone<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    dest: &str,
    src: &MirOperand,
) -> Option<BasicValueEnum<'ctx>> {
    let val = operand_to_value(ctx, src)?;

    // Try to get the TypeId for the source variable
    let type_id = get_operand_name(src).and_then(|name| ctx.get_variable_type(name));

    // Get the TypeKind if we have a TypeId
    let type_kind = type_id.and_then(|tid| ctx.get_type_kind(tid));

    // Also check if source has a struct type association to propagate
    let src_struct_type =
        get_operand_name(src).and_then(|name| ctx.get_temp_struct_type(name).cloned());

    // Dispatch based on type
    let cloned = match &type_kind {
        // Primitives: simple copy (no heap allocation)
        Some(TypeKind::Int)
        | Some(TypeKind::Float)
        | Some(TypeKind::Bool)
        | Some(TypeKind::Void) => val,

        // String: allocate new buffer and copy
        Some(TypeKind::Str) => {
            if val.is_pointer_value() {
                clone_string(ctx, val.into_pointer_value())
                    .map(|p| p.into())
                    .unwrap_or(val)
            } else {
                val
            }
        }

        // Struct: allocate new struct and clone fields
        Some(TypeKind::Struct { name, fields }) => {
            // Propagate struct type association
            ctx.set_temp_struct_type(dest, name);
            if val.is_pointer_value() {
                clone_struct(ctx, val.into_pointer_value(), name, fields)
                    .map(|p| p.into())
                    .unwrap_or(val)
            } else {
                val
            }
        }

        // Array: allocate new array and clone elements
        Some(TypeKind::Array { element }) => {
            if val.is_pointer_value() {
                clone_array(ctx, val.into_pointer_value(), *element)
                    .map(|p| p.into())
                    .unwrap_or(val)
            } else {
                val
            }
        }

        // Map: allocate new map and clone pairs
        Some(TypeKind::Map { key, value }) => {
            if val.is_pointer_value() {
                clone_map(ctx, val.into_pointer_value(), *key, *value)
                    .map(|p| p.into())
                    .unwrap_or(val)
            } else {
                val
            }
        }

        // Optional: clone inner value if present
        Some(TypeKind::Optional { inner }) => {
            if val.is_pointer_value() {
                clone_optional(ctx, val.into_pointer_value(), *inner)
                    .map(|p| p.into())
                    .unwrap_or(val)
            } else {
                val
            }
        }

        // Unknown type or const: fallback to shallow copy
        // For pointer types, assume it's a string (most common case)
        None => {
            // If the source has a struct type association, propagate it even if we don't know the TypeKind
            if let Some(ref struct_name) = src_struct_type {
                ctx.set_temp_struct_type(dest, struct_name);
            }
            if val.is_pointer_value() {
                // Heuristic: if it's a pointer and we don't know the type,
                // assume it's a string and clone it
                clone_string(ctx, val.into_pointer_value())
                    .map(|p| p.into())
                    .unwrap_or(val)
            } else {
                val
            }
        }

        // Other complex types: shallow copy for now
        _ => {
            // Propagate struct type association if present
            if let Some(ref struct_name) = src_struct_type {
                ctx.set_temp_struct_type(dest, struct_name);
            }
            val
        }
    };

    ctx.set_local(dest.to_string(), cloned);
    Some(cloned)
}

// ============================================================================
// Type-Specific Clone Functions
// ============================================================================

/// Clone a string (null-terminated C string).
/// Allocates new buffer with strlen + 1, copies bytes including null terminator.
fn clone_string<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    src_ptr: inkwell::values::PointerValue<'ctx>,
) -> Option<inkwell::values::PointerValue<'ctx>> {
    let ptr_type = ctx.ptr_type();
    let i64_type = ctx.context.i64_type();

    // Get or declare strlen
    let strlen_fn = ctx
        .module
        .get_function(ffi_names::STRLEN)
        .unwrap_or_else(|| {
            let fn_ty = i64_type.fn_type(&[ptr_type.into()], false);
            ctx.module.add_function(ffi_names::STRLEN, fn_ty, None)
        });

    // Get or declare malloc
    let malloc_fn = ctx
        .module
        .get_function(ffi_names::MALLOC)
        .unwrap_or_else(|| {
            let fn_ty = ptr_type.fn_type(&[i64_type.into()], false);
            ctx.module.add_function(ffi_names::MALLOC, fn_ty, None)
        });

    // Get or declare memcpy
    let memcpy_fn = ctx
        .module
        .get_function(ffi_names::MEMCPY)
        .unwrap_or_else(|| {
            let fn_ty =
                ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false);
            ctx.module.add_function(ffi_names::MEMCPY, fn_ty, None)
        });

    // Handle null pointer case
    let is_null = ctx.builder.build_is_null(src_ptr, "is_null").ok()?;

    let current_block = ctx.builder.get_insert_block()?;
    let func = current_block.get_parent()?;
    let clone_block = ctx.context.append_basic_block(func, "clone_str");
    let merge_block = ctx.context.append_basic_block(func, "clone_merge");

    ctx.builder
        .build_conditional_branch(is_null, merge_block, clone_block)
        .ok()?;

    // Clone block: do the actual cloning
    ctx.builder.position_at_end(clone_block);

    // len = strlen(src)
    let len = ctx
        .builder
        .build_call(strlen_fn, &[src_ptr.into()], "len")
        .ok()?
        .try_as_basic_value()
        .left()?
        .into_int_value();

    // size = len + 1 (for null terminator)
    let size = ctx
        .builder
        .build_int_add(len, i64_type.const_int(1, false), "size")
        .ok()?;

    // dst = malloc(size)
    let dst_ptr = ctx
        .builder
        .build_call(malloc_fn, &[size.into()], "dst")
        .ok()?
        .try_as_basic_value()
        .left()?
        .into_pointer_value();

    // memcpy(dst, src, size)
    ctx.builder
        .build_call(
            memcpy_fn,
            &[dst_ptr.into(), src_ptr.into(), size.into()],
            "",
        )
        .ok()?;

    ctx.builder.build_unconditional_branch(merge_block).ok()?;

    // Merge block: phi between null and cloned
    ctx.builder.position_at_end(merge_block);
    let phi = ctx.builder.build_phi(ptr_type, "cloned_str").ok()?;
    phi.add_incoming(&[
        (&ptr_type.const_null(), current_block),
        (&dst_ptr, clone_block),
    ]);

    Some(phi.as_basic_value().into_pointer_value())
}

/// Clone a struct by allocating new memory and copying/cloning fields.
fn clone_struct<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    src_ptr: inkwell::values::PointerValue<'ctx>,
    struct_name: &str,
    fields: &[(String, doo_core::types::TypeId)],
) -> Option<inkwell::values::PointerValue<'ctx>> {
    let ptr_type = ctx.ptr_type();
    let i64_type = ctx.context.i64_type();

    // Get or create struct type
    let struct_type = ctx.lookup_struct_type(struct_name)?;

    // Calculate struct size using LLVM's size_of() for correct padding
    // CRITICAL FIX: The old calculation (fields.len() * 8) was WRONG because:
    // - Enum fields are { i32, ptr } = 16 bytes (not 8)
    // - Struct padding can vary based on field alignment requirements
    let struct_size = struct_type
        .size_of()
        .map(|v| v.get_zero_extended_constant().unwrap_or(64))
        .unwrap_or((fields.len() * 8) as u64); // Fallback for safety

    // Get or declare malloc
    let malloc_fn = ctx
        .module
        .get_function(ffi_names::MALLOC)
        .unwrap_or_else(|| {
            let fn_ty = ptr_type.fn_type(&[i64_type.into()], false);
            ctx.module.add_function(ffi_names::MALLOC, fn_ty, None)
        });

    // Allocate new struct
    let dst_ptr = ctx
        .builder
        .build_call(
            malloc_fn,
            &[i64_type.const_int(struct_size.max(16), false).into()], // min 16 bytes
            "clone_struct",
        )
        .ok()?
        .try_as_basic_value()
        .left()?
        .into_pointer_value();

    // Copy each field
    for (idx, (field_name, field_type_id)) in fields.iter().enumerate() {
        let field_kind = ctx.get_type_kind(*field_type_id);

        // Get source field pointer
        let src_field_ptr = unsafe {
            ctx.builder
                .build_gep(
                    struct_type,
                    src_ptr,
                    &[
                        ctx.context.i32_type().const_zero(),
                        ctx.context.i32_type().const_int(idx as u64, false),
                    ],
                    &format!("src_{}", field_name),
                )
                .ok()?
        };

        // Get dest field pointer
        let dst_field_ptr = unsafe {
            ctx.builder
                .build_gep(
                    struct_type,
                    dst_ptr,
                    &[
                        ctx.context.i32_type().const_zero(),
                        ctx.context.i32_type().const_int(idx as u64, false),
                    ],
                    &format!("dst_{}", field_name),
                )
                .ok()?
        };

        // Load source field value
        let field_llvm_type = ctx.get_llvm_type(*field_type_id);
        let src_val = ctx
            .builder
            .build_load(field_llvm_type, src_field_ptr, "field_val")
            .ok()?;

        // Clone or copy the field based on its type
        let cloned_val = match field_kind {
            Some(TypeKind::Str) if src_val.is_pointer_value() => {
                clone_string(ctx, src_val.into_pointer_value())
                    .map(|p| p.into())
                    .unwrap_or(src_val)
            }
            // Primitives and unknowns: direct copy
            _ => src_val,
        };

        // Store to dest
        ctx.builder.build_store(dst_field_ptr, cloned_val).ok()?;
    }

    Some(dst_ptr)
}

/// Clone an array (simplified: shallow clone for now).
/// TODO: Implement proper deep clone with element iteration.
fn clone_array<'ctx>(
    _ctx: &mut CodegenContext<'ctx>,
    src_ptr: inkwell::values::PointerValue<'ctx>,
    _element_type: doo_core::types::TypeId,
) -> Option<inkwell::values::PointerValue<'ctx>> {
    // For now, just return the source pointer (shallow copy)
    // Full implementation would:
    // 1. Read array length from header
    // 2. Allocate new array with same capacity
    // 3. Clone each element recursively
    // 4. Return new array pointer
    Some(src_ptr)
}

/// Clone a map (simplified: shallow clone for now).
/// TODO: Implement proper deep clone with key/value iteration.
fn clone_map<'ctx>(
    _ctx: &mut CodegenContext<'ctx>,
    src_ptr: inkwell::values::PointerValue<'ctx>,
    _key_type: doo_core::types::TypeId,
    _value_type: doo_core::types::TypeId,
) -> Option<inkwell::values::PointerValue<'ctx>> {
    // For now, just return the source pointer (shallow copy)
    // Full implementation would iterate all entries and clone
    Some(src_ptr)
}

/// Clone an optional value.
fn clone_optional<'ctx>(
    _ctx: &mut CodegenContext<'ctx>,
    src_ptr: inkwell::values::PointerValue<'ctx>,
    _inner_type: doo_core::types::TypeId,
) -> Option<inkwell::values::PointerValue<'ctx>> {
    // For now, just return the source pointer (shallow copy)
    // Full implementation would check has_value flag and clone inner if present
    Some(src_ptr)
}

// ============================================================================
// Drop (Cleanup) Implementation
// ============================================================================

/// Emit drop (cleanup) for a variable based on its type.
fn emit_drop<'ctx>(ctx: &mut CodegenContext<'ctx>, var_name: &str) {
    // Skip internal loop variables - they are just copies of array pointers
    // and should not be freed (the original array variable handles cleanup).
    // Internal variables are prefixed with "__" (e.g., __num_arr, __idx).
    if var_name.starts_with("__") {
        return;
    }

    // Try to get the value - if it doesn't exist, nothing to drop
    let val = match ctx.get_value(var_name) {
        Some(v) => v,
        None => return,
    };

    // Only drop pointer types (heap-allocated)
    if !val.is_pointer_value() {
        return;
    }

    let ptr = val.into_pointer_value();

    // Try to get the TypeId for the variable
    let type_id = ctx.get_variable_type(var_name);

    // Get the TypeKind if we have a TypeId
    let type_kind = type_id.and_then(|tid| ctx.get_type_kind(tid));

    // Dispatch based on type
    match &type_kind {
        // Primitives: no-op (not heap allocated)
        Some(TypeKind::Int)
        | Some(TypeKind::Float)
        | Some(TypeKind::Bool)
        | Some(TypeKind::Void) => {
            // No cleanup needed
        }

        // String: DO NOT free - strings are either static constants or borrowed
        // from arrays. We don't have dynamic string allocation yet.
        Some(TypeKind::Str) => {
            // No cleanup - strings point to static or array memory
        }

        // Struct: drop each field, then free
        Some(TypeKind::Struct { name, fields }) => {
            drop_struct(ctx, ptr, name, fields);
        }

        // Array: drop each element, then free
        Some(TypeKind::Array { element }) => {
            drop_array(ctx, ptr, *element);
        }

        // Map: drop each pair, then free
        Some(TypeKind::Map { key, value }) => {
            drop_map(ctx, ptr, *key, *value);
        }

        // Optional: drop inner if present, then free
        Some(TypeKind::Optional { inner }) => {
            drop_optional(ctx, ptr, *inner);
        }

        // Unknown type: do nothing to be safe
        // We can't know if it's heap-allocated or static
        None => {
            // No cleanup - unknown type, better to leak than crash
        }

        // Other complex types: do nothing to be safe
        _ => {
            // No cleanup - unknown type, better to leak than crash
        }
    }
}

// ============================================================================
// Type-Specific Drop Functions
// ============================================================================

/// Get or declare the free function.
fn get_or_declare_free<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
) -> inkwell::values::FunctionValue<'ctx> {
    let ptr_type = ctx.ptr_type();
    let void_type = ctx.context.void_type();

    ctx.module.get_function(ffi_names::FREE).unwrap_or_else(|| {
        let fn_ty = void_type.fn_type(&[ptr_type.into()], false);
        ctx.module.add_function(ffi_names::FREE, fn_ty, None)
    })
}

/// Drop a pointer by calling free (with null check).
fn drop_pointer<'ctx>(ctx: &mut CodegenContext<'ctx>, ptr: inkwell::values::PointerValue<'ctx>) {
    // Null check before freeing
    let is_null = match ctx.builder.build_is_null(ptr, "is_null") {
        Ok(v) => v,
        Err(_) => return,
    };

    let current_block = match ctx.builder.get_insert_block() {
        Some(b) => b,
        None => return,
    };
    let func = match current_block.get_parent() {
        Some(f) => f,
        None => return,
    };

    let free_block = ctx.context.append_basic_block(func, "drop_free");
    let continue_block = ctx.context.append_basic_block(func, "drop_continue");

    if ctx
        .builder
        .build_conditional_branch(is_null, continue_block, free_block)
        .is_err()
    {
        return;
    }

    // Free block: call free
    ctx.builder.position_at_end(free_block);
    let free_fn = get_or_declare_free(ctx);
    let _ = ctx.builder.build_call(free_fn, &[ptr.into()], "");
    let _ = ctx.builder.build_unconditional_branch(continue_block);

    // Continue block
    ctx.builder.position_at_end(continue_block);
}

/// Drop a string (null-terminated C string).
/// Simply frees the buffer after null check.
fn drop_string<'ctx>(ctx: &mut CodegenContext<'ctx>, ptr: inkwell::values::PointerValue<'ctx>) {
    drop_pointer(ctx, ptr);
}

/// Drop a struct by dropping each field, then freeing the struct memory.
fn drop_struct<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    ptr: inkwell::values::PointerValue<'ctx>,
    struct_name: &str,
    fields: &[(String, doo_core::types::TypeId)],
) {
    // Null check
    let is_null = match ctx.builder.build_is_null(ptr, "struct_is_null") {
        Ok(v) => v,
        Err(_) => return,
    };

    let current_block = match ctx.builder.get_insert_block() {
        Some(b) => b,
        None => return,
    };
    let func = match current_block.get_parent() {
        Some(f) => f,
        None => return,
    };

    let drop_block = ctx.context.append_basic_block(func, "drop_struct");
    let continue_block = ctx.context.append_basic_block(func, "drop_struct_continue");

    if ctx
        .builder
        .build_conditional_branch(is_null, continue_block, drop_block)
        .is_err()
    {
        return;
    }

    // Drop block: drop each field, then free struct
    ctx.builder.position_at_end(drop_block);

    // Get struct type if available
    if let Some(struct_type) = ctx.lookup_struct_type(struct_name) {
        // Clone fields to avoid borrow issues
        let fields_copy: Vec<_> = fields.to_vec();

        // Drop each field that needs cleanup
        for (idx, (field_name, field_type_id)) in fields_copy.iter().enumerate() {
            let field_kind = ctx.get_type_kind(*field_type_id);

            // Only drop pointer/heap-allocated fields
            let needs_drop = matches!(
                field_kind,
                Some(TypeKind::Str)
                    | Some(TypeKind::Array { .. })
                    | Some(TypeKind::Map { .. })
                    | Some(TypeKind::Struct { .. })
                    | Some(TypeKind::Optional { .. })
            );

            if needs_drop {
                // Get field pointer
                let field_ptr = unsafe {
                    match ctx.builder.build_gep(
                        struct_type,
                        ptr,
                        &[
                            ctx.context.i32_type().const_zero(),
                            ctx.context.i32_type().const_int(idx as u64, false),
                        ],
                        &format!("field_{}", field_name),
                    ) {
                        Ok(p) => p,
                        Err(_) => continue,
                    }
                };

                // Load field value
                let field_llvm_type = ctx.get_llvm_type(*field_type_id);
                let field_val =
                    match ctx
                        .builder
                        .build_load(field_llvm_type, field_ptr, "field_val")
                    {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                if field_val.is_pointer_value() {
                    let field_ptr_val = field_val.into_pointer_value();

                    // Recursively drop based on field type
                    // NOTE: DO NOT free strings - they're either static constants
                    // or borrowed. We don't have dynamic string allocation yet.
                    match field_kind {
                        Some(TypeKind::Str) => {
                            // No cleanup for strings - they point to static memory
                        }
                        Some(TypeKind::Struct {
                            name: nested_name,
                            fields: nested_fields,
                        }) => {
                            let nested_name = nested_name.clone();
                            let nested_fields = nested_fields.clone();
                            drop_struct(ctx, field_ptr_val, &nested_name, &nested_fields);
                        }
                        Some(TypeKind::Array { element }) => {
                            drop_array(ctx, field_ptr_val, element);
                        }
                        Some(TypeKind::Map { key, value }) => {
                            drop_map(ctx, field_ptr_val, key, value);
                        }
                        _ => drop_pointer(ctx, field_ptr_val),
                    }
                }
            }
        }
    }

    // Free the struct itself
    let free_fn = get_or_declare_free(ctx);
    let _ = ctx.builder.build_call(free_fn, &[ptr.into()], "");
    let _ = ctx.builder.build_unconditional_branch(continue_block);

    // Continue block
    ctx.builder.position_at_end(continue_block);
}

/// Drop an array by freeing the header (data pointer - 16 bytes).
/// Array layout: [length: i64][capacity: i64][data...]
/// We store the DATA pointer, so we need to go back 16 bytes to get the header.
fn drop_array<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    ptr: inkwell::values::PointerValue<'ctx>,
    _element_type: doo_core::types::TypeId,
) {
    // Array stores DATA pointer, but we need to free the HEADER pointer
    // Header is at (data_ptr - 16 bytes)
    let i8_type = ctx.context.i8_type();
    let i32_type = ctx.context.i32_type();

    // Calculate header pointer: data_ptr - 16
    let header_ptr = match unsafe {
        ctx.builder.build_in_bounds_gep(
            i8_type,
            ptr,
            &[i32_type.const_int((-16i64) as u64, true)],
            "arr_header_ptr",
        )
    } {
        Ok(p) => p,
        Err(_) => return,
    };

    // Free the header (which includes the data region)
    drop_pointer(ctx, header_ptr);
}

/// Drop a map by dropping each key/value pair, then freeing.
/// TODO: Full implementation with entry iteration.
fn drop_map<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    ptr: inkwell::values::PointerValue<'ctx>,
    _key_type: doo_core::types::TypeId,
    _value_type: doo_core::types::TypeId,
) {
    // Map stores DATA pointer, but we need to free the HEADER pointer
    // Header is at (data_ptr - 16 bytes)
    let i8_type = ctx.context.i8_type();
    let i32_type = ctx.context.i32_type();

    // Calculate header pointer: data_ptr - 16
    let header_ptr = match unsafe {
        ctx.builder.build_in_bounds_gep(
            i8_type,
            ptr,
            &[i32_type.const_int((-16i64) as u64, true)],
            "map_header_ptr",
        )
    } {
        Ok(p) => p,
        Err(_) => return,
    };

    // Free the header (which includes the data region)
    drop_pointer(ctx, header_ptr);
}

/// Drop an optional value.
/// TODO: Full implementation checking has_value flag.
fn drop_optional<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    ptr: inkwell::values::PointerValue<'ctx>,
    _inner_type: doo_core::types::TypeId,
) {
    // Simplified: just free the optional pointer
    // Full implementation would check has_value and drop inner if present
    drop_pointer(ctx, ptr);
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use doo_core::types::TypeRegistry;
    use doo_mir::Span;
    use inkwell::context::Context;
    use std::sync::Arc;

    #[test]
    fn test_memory_handler_handles() {
        let handler = MemoryHandler;

        // Should handle Move
        let move_instr = MirInstr {
            kind: MirInstrKind::Move {
                dest: "x".to_string(),
                src: MirOperand::Const(doo_mir::MirConst::Int(42)),
            },
            span: Span::default(),
        };
        assert!(handler.handles(&move_instr));

        // Should handle Copy
        let copy_instr = MirInstr {
            kind: MirInstrKind::Copy {
                dest: "x".to_string(),
                src: MirOperand::Const(doo_mir::MirConst::Int(42)),
            },
            span: Span::default(),
        };
        assert!(handler.handles(&copy_instr));

        // Should handle Clone
        let clone_instr = MirInstr {
            kind: MirInstrKind::Clone {
                dest: "x".to_string(),
                src: MirOperand::Const(doo_mir::MirConst::Int(42)),
            },
            span: Span::default(),
        };
        assert!(handler.handles(&clone_instr));

        // Should handle Drop
        let drop_instr = MirInstr {
            kind: MirInstrKind::Drop {
                value: "x".to_string(),
            },
            span: Span::default(),
        };
        assert!(handler.handles(&drop_instr));

        // Should handle Assign
        let assign_instr = MirInstr {
            kind: MirInstrKind::Assign {
                dest: "x".to_string(),
                value: MirOperand::Const(doo_mir::MirConst::Int(42)),
            },
            span: Span::default(),
        };
        assert!(handler.handles(&assign_instr));

        // Should handle Borrow
        let borrow_instr = MirInstr {
            kind: MirInstrKind::Borrow {
                dest: "ref_x".to_string(),
                src: "x".to_string(),
                mutable: false,
            },
            span: Span::default(),
        };
        assert!(handler.handles(&borrow_instr));

        // Should NOT handle other instructions (e.g., BinaryOp)
        let binary_op = MirInstr {
            kind: MirInstrKind::BinaryOp {
                dest: "y".to_string(),
                op: doo_mir::BinaryOp::Add,
                lhs: MirOperand::Const(doo_mir::MirConst::Int(1)),
                rhs: MirOperand::Const(doo_mir::MirConst::Int(2)),
            },
            span: Span::default(),
        };
        assert!(!handler.handles(&binary_op));
    }

    #[test]
    fn test_emit_move_with_constant() {
        let ctx = Context::create();
        let registry = Arc::new(TypeRegistry::new());
        let mut codegen = CodegenContext::new(&ctx, "test", registry);

        let handler = MemoryHandler;

        // Set up a basic block for builder operations
        let fn_type = ctx.void_type().fn_type(&[], false);
        let func = codegen.module.add_function("test_fn", fn_type, None);
        let entry = ctx.append_basic_block(func, "entry");
        codegen.builder.position_at_end(entry);

        // Test Move with integer constant
        let move_instr = MirInstr {
            kind: MirInstrKind::Move {
                dest: "x".to_string(),
                src: MirOperand::Const(doo_mir::MirConst::Int(42)),
            },
            span: Span::default(),
        };

        let result = handler.emit(&mut codegen, &move_instr);
        assert!(result.is_some());
        assert!(result.unwrap().is_int_value());
    }

    #[test]
    fn test_emit_copy_with_constant() {
        let ctx = Context::create();
        let registry = Arc::new(TypeRegistry::new());
        let mut codegen = CodegenContext::new(&ctx, "test", registry);

        let handler = MemoryHandler;

        // Set up a basic block
        let fn_type = ctx.void_type().fn_type(&[], false);
        let func = codegen.module.add_function("test_fn", fn_type, None);
        let entry = ctx.append_basic_block(func, "entry");
        codegen.builder.position_at_end(entry);

        // Test Copy with float constant
        let copy_instr = MirInstr {
            kind: MirInstrKind::Copy {
                dest: "y".to_string(),
                src: MirOperand::Const(doo_mir::MirConst::Float(3.14)),
            },
            span: Span::default(),
        };

        let result = handler.emit(&mut codegen, &copy_instr);
        assert!(result.is_some());
        assert!(result.unwrap().is_float_value());
    }

    #[test]
    fn test_emit_clone_with_string() {
        let ctx = Context::create();
        let registry = Arc::new(TypeRegistry::new());
        let mut codegen = CodegenContext::new(&ctx, "test", registry);

        let handler = MemoryHandler;

        // Set up a basic block
        let fn_type = ctx.void_type().fn_type(&[], false);
        let func = codegen.module.add_function("test_fn", fn_type, None);
        let entry = ctx.append_basic_block(func, "entry");
        codegen.builder.position_at_end(entry);

        // Test Clone with string constant - should generate clone code
        let clone_instr = MirInstr {
            kind: MirInstrKind::Clone {
                dest: "cloned_str".to_string(),
                src: MirOperand::Const(doo_mir::MirConst::Str("hello".to_string())),
            },
            span: Span::default(),
        };

        let result = handler.emit(&mut codegen, &clone_instr);
        assert!(result.is_some());
        // Result should be a pointer (string)
        assert!(result.unwrap().is_pointer_value());
    }

    #[test]
    fn test_emit_drop_nonexistent_var() {
        let ctx = Context::create();
        let registry = Arc::new(TypeRegistry::new());
        let mut codegen = CodegenContext::new(&ctx, "test", registry);

        let handler = MemoryHandler;

        // Set up a basic block
        let fn_type = ctx.void_type().fn_type(&[], false);
        let func = codegen.module.add_function("test_fn", fn_type, None);
        let entry = ctx.append_basic_block(func, "entry");
        codegen.builder.position_at_end(entry);

        // Test Drop on non-existent variable - should not panic
        let drop_instr = MirInstr {
            kind: MirInstrKind::Drop {
                value: "nonexistent".to_string(),
            },
            span: Span::default(),
        };

        let result = handler.emit(&mut codegen, &drop_instr);
        assert!(result.is_none()); // Drop returns None
    }

    #[test]
    fn test_get_operand_name() {
        // Test Local
        let local = MirOperand::Local("local_var".to_string());
        assert_eq!(get_operand_name(&local), Some("local_var"));

        // Test Temp
        let temp = MirOperand::Temp("temp_0".to_string());
        assert_eq!(get_operand_name(&temp), Some("temp_0"));

        // Test Global
        let global = MirOperand::Global("global_var".to_string());
        assert_eq!(get_operand_name(&global), Some("global_var"));

        // Test Const - should return None
        let const_val = MirOperand::Const(doo_mir::MirConst::Int(42));
        assert_eq!(get_operand_name(&const_val), None);
    }
}
