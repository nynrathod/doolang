//! Memory Instruction Handler
//!
//! Handles: Move, Copy, Clone, Assign, Drop, Borrow

use doo_core::constants::ffi_names;
use doo_core::types::TypeKind;
use doo_mir::sym::resolve;
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
                let dest_str = resolve(*dest);
                // Propagate struct type association if present
                if let Some(src_name) = get_operand_name(src) {
                    // First try temp_struct_type
                    let struct_name = ctx.get_temp_struct_type(&src_name).cloned().or_else(|| {
                        // Fallback: try variable_type -> TypeKind::Struct
                        ctx.get_variable_type(&src_name)
                            .and_then(|tid| ctx.get_struct_name_from_type_id(tid))
                    });
                    if let Some(name) = struct_name {
                        ctx.set_temp_struct_type(&dest_str, &name);
                    }
                    // Propagate variable type from source to dest
                    if let Some(src_type) = ctx.get_variable_type(&src_name) {
                        ctx.set_variable_type(&dest_str, src_type);
                    }
                    // Propagate array element types from temp to local for map/filter/slice results
                    // Critical for correct printing of float/int arrays returned from lambda methods
                    if let Some(&elem_type) = ctx.array_element_types.get(&src_name) {
                        ctx.array_element_types.insert(dest_str.clone(), elem_type);
                    }
                }
                ctx.set_local(dest_str, val);
                Some(val)
            }
            MirInstrKind::Copy { dest, src } => {
                // Copy is also a simple copy at LLVM level
                let val = operand_to_value(ctx, src)?;
                let dest_str = resolve(*dest);
                // Propagate struct type association if present
                if let Some(src_name) = get_operand_name(src) {
                    // First try temp_struct_type (most specific)
                    let struct_name = ctx.get_temp_struct_type(&src_name).cloned().or_else(|| {
                        // Fallback: try inferring from variable_type
                        ctx.get_variable_type(&src_name)
                            .and_then(|tid| ctx.get_struct_name_from_type_id(tid))
                    });
                    if let Some(name) = struct_name {
                        ctx.set_temp_struct_type(&dest_str, &name);
                    }
                    // Propagate variable type from source to dest
                    if let Some(src_type) = ctx.get_variable_type(&src_name) {
                        ctx.set_variable_type(&dest_str, src_type);
                    }
                    // Propagate array element types from temp to local for map/filter/slice results
                    // Critical for correct printing of float/int arrays returned from lambda methods
                    if let Some(&elem_type) = ctx.array_element_types.get(&src_name) {
                        ctx.array_element_types.insert(dest_str.clone(), elem_type);
                    }
                }
                ctx.set_local(dest_str, val);
                Some(val)
            }
            MirInstrKind::Clone { dest, src } => {
                // Deep clone based on type
                emit_deep_clone(ctx, &resolve(*dest), src)
            }
            MirInstrKind::Assign { dest, value } => {
                let val = operand_to_value(ctx, value)?;
                let dest_str = resolve(*dest);
                // Propagate struct type association if present
                if let Some(src_name) = get_operand_name(value) {
                    // First try temp_struct_type (most specific)
                    let struct_name = ctx.get_temp_struct_type(&src_name).cloned().or_else(|| {
                        // Fallback: try inferring from variable_type
                        ctx.get_variable_type(&src_name)
                            .and_then(|tid| ctx.get_struct_name_from_type_id(tid))
                    });
                    if let Some(name) = struct_name {
                        ctx.set_temp_struct_type(&dest_str, &name);
                    }
                    // Also propagate the variable type from source to dest
                    // This ensures that when a temp with known type is assigned to a local,
                    // the local gets the correct type for subsequent Clone operations
                    if let Some(src_type) = ctx.get_variable_type(&src_name) {
                        ctx.set_variable_type(&dest_str, src_type);
                    }
                    // Propagate array element types from temp to local for map/filter/slice results
                    // Critical for correct printing of float/int arrays returned from lambda methods
                    if let Some(&elem_type) = ctx.array_element_types.get(&src_name) {
                        ctx.array_element_types.insert(dest_str.clone(), elem_type);
                    }
                }
                ctx.set_local(dest_str, val);
                Some(val)
            }
            MirInstrKind::Drop { value } => {
                // Actual cleanup based on type
                emit_drop(ctx, &resolve(*value));
                None
            }
            MirInstrKind::Borrow { dest, src, .. } => {
                // Borrow at LLVM level is just getting the value
                let src_str = resolve(*src);
                let dest_str = resolve(*dest);
                let val = ctx.get_value(&src_str)?;
                ctx.set_local(dest_str.clone(), val);
                // Track borrow origin so mutating operations can store back
                ctx.set_borrow_origin(&dest_str, &src_str);
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
fn get_operand_name(operand: &MirOperand) -> Option<String> {
    match operand {
        MirOperand::Local(name) | MirOperand::Temp(name) | MirOperand::Global(name) => {
            Some(resolve(*name))
        }
        MirOperand::Const(_) => None,
        MirOperand::FuncRef(name) => Some(resolve(*name)),
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
    let type_id = get_operand_name(src).and_then(|name| ctx.get_variable_type(&name));

    // Get the TypeKind if we have a TypeId
    // CRITICAL: Resolve TypeRef to the actual type to handle imported types correctly
    let type_kind = type_id.and_then(|tid| {
        let kind = ctx.get_type_kind(tid)?;
        // If it's a TypeRef, resolve it to the actual type
        if let TypeKind::TypeRef { name } = &kind {
            let resolved_tid = ctx.type_registry.lookup(name)?;
            ctx.get_type_kind(resolved_tid)
        } else {
            Some(kind)
        }
    });

    // Also check if source has a struct type association to propagate
    let src_struct_type =
        get_operand_name(src).and_then(|name| ctx.get_temp_struct_type(&name).cloned());

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
                // Extract just name and type for clone_struct (visibility not needed)
                let field_pairs: Vec<_> = fields.iter().map(|(n, t, _)| (n.clone(), *t)).collect();
                clone_struct(ctx, val.into_pointer_value(), name, &field_pairs)
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
pub(crate) fn clone_string<'ctx>(
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

    // Handle null AND invalid pointer case.
    // CRITICAL: Check for pointers below page boundary (< 4096).
    // This catches cases where an integer value was incorrectly stored as a ptr
    // (e.g., DB id=1 became ptr 0x1 via inttoptr). Dereferencing such a pointer
    // in strlen would cause an access violation crash.
    let ptr_as_int = ctx
        .builder
        .build_ptr_to_int(src_ptr, i64_type, "ptr_int")
        .ok()?;
    let min_valid_addr = i64_type.const_int(4096, false);
    let is_invalid = ctx
        .builder
        .build_int_compare(
            inkwell::IntPredicate::ULT,
            ptr_as_int,
            min_valid_addr,
            "str_ptr_invalid",
        )
        .ok()?;

    let current_block = ctx.builder.get_insert_block()?;
    let func = current_block.get_parent()?;
    let clone_block = ctx.context.append_basic_block(func, "clone_str");
    let merge_block = ctx.context.append_basic_block(func, "clone_merge");

    ctx.builder
        .build_conditional_branch(is_invalid, merge_block, clone_block)
        .ok()?;

    // Clone block: do the actual cloning
    ctx.builder.position_at_end(clone_block);

    // len = strlen(src)
    let len = ctx
        .builder
        .build_call(strlen_fn, &[src_ptr.into()], "len")
        .ok()?
        .try_as_basic_value()
        .basic()?
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
        .basic()?
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

    // Merge block: phi between null and cloned string.
    // Return null for null/invalid input to preserve nil semantics.
    // This is CRITICAL for result error variables: when a Result is Ok,
    // the error variable is null. If clone_string coerces null to "",
    // then `err != nil` evaluates to true (breaking error checking).
    // Safety: All string builtins dispatch through null_coerce_str which
    // inserts a select before strlen/strcmp, preventing null-deref UB.
    // String comparison (==, !=) in arithmetic.rs also null-coerces.
    // Print path null-coerces before printf.
    ctx.builder.position_at_end(merge_block);
    let phi = ctx.builder.build_phi(ptr_type, "cloned_str").ok()?;
    phi.add_incoming(&[
        (&ptr_type.const_null(), current_block),
        (&dst_ptr, clone_block),
    ]);

    Some(phi.as_basic_value().into_pointer_value())
}

/// Clone a struct by allocating new memory and copying/cloning fields.
///
/// IMPORTANT: Handles null pointers safely - if src_ptr is null, returns null.
/// This is critical for error handling where error values may be null when
/// the operation succeeded.
pub(crate) fn clone_struct<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    src_ptr: inkwell::values::PointerValue<'ctx>,
    struct_name: &str,
    fields: &[(String, doo_core::types::TypeId)],
) -> Option<inkwell::values::PointerValue<'ctx>> {
    let ptr_type = ctx.ptr_type();
    let i64_type = ctx.context.i64_type();

    // Get or create struct type
    let struct_type = ctx.lookup_struct_type(struct_name)?;

    // CRITICAL: Use LLVM's size_of() IntValue DIRECTLY as malloc argument.
    // DO NOT extract via get_zero_extended_constant() — it fails on ConstantExpr
    // (sizeof returns ConstantExpr, not ConstantInt), causing fallback to
    // fields.len() * 8 which is WRONG when fields contain enums (16 bytes, not 8).
    // This was the root cause of heap buffer overflow crashes in JSON tests.
    let struct_size_val = struct_type.size_of().unwrap_or_else(|| {
        // Fallback: compute size accounting for enum fields (16 bytes each)
        let mut offset: u64 = 0;
        for (_, type_id) in fields.iter() {
            let field_size = match ctx.get_type_kind(*type_id) {
                Some(doo_core::types::TypeKind::Enum { .. }) => 16, // { i32, ptr }
                _ => 8, // Int, Float, Bool, Str(ptr), Array(ptr), Map(ptr), Struct(ptr)
            };
            // Align to 8 bytes
            offset = (offset + 7) & !7;
            offset += field_size;
        }
        // Pad to 8-byte alignment and ensure minimum 16 bytes
        let padded = ((offset + 7) & !7).max(16);
        i64_type.const_int(padded, false)
    });

    // Get or declare malloc
    let malloc_fn = ctx
        .module
        .get_function(ffi_names::MALLOC)
        .unwrap_or_else(|| {
            let fn_ty = ptr_type.fn_type(&[i64_type.into()], false);
            ctx.module.add_function(ffi_names::MALLOC, fn_ty, None)
        });

    // Handle null pointer case — needed for optional/error fields that can be null.
    // Returns const_null on null path (preserving original semantics), then passes
    // the PHI result through __doo_opaque_ptr (noinline optnone) to prevent LLVM
    // from seeing the null value and exploiting UB from subsequent field accesses.
    let is_null = ctx.builder.build_is_null(src_ptr, "struct_is_null").ok()?;

    let current_block = ctx.builder.get_insert_block()?;
    let func = current_block.get_parent()?;
    let clone_block = ctx.context.append_basic_block(func, "clone_struct_do");
    let merge_block = ctx.context.append_basic_block(func, "clone_struct_merge");

    ctx.builder
        .build_conditional_branch(is_null, merge_block, clone_block)
        .ok()?;

    // Clone block: do the actual cloning
    ctx.builder.position_at_end(clone_block);

    // Allocate new struct using LLVM-computed size
    let dst_ptr = ctx
        .builder
        .build_call(malloc_fn, &[struct_size_val.into()], "clone_struct")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_pointer_value();

    // Zero-initialize to prevent garbage in unset struct fields
    let memset_fn = ctx
        .module
        .get_function(ffi_names::MEMSET)
        .unwrap_or_else(|| {
            let fn_ty = ptr_type.fn_type(
                &[
                    ptr_type.into(),
                    ctx.context.i32_type().into(),
                    i64_type.into(),
                ],
                false,
            );
            ctx.module.add_function(ffi_names::MEMSET, fn_ty, None)
        });
    let _ = ctx.builder.build_call(
        memset_fn,
        &[
            dst_ptr.into(),
            ctx.context.i32_type().const_zero().into(),
            struct_size_val.into(),
        ],
        "",
    );

    // Copy each field
    for (idx, (field_name, field_type_id)) in fields.iter().enumerate() {
        let field_kind = ctx.get_type_kind(*field_type_id);
        // Apply P06 logical→physical field remapping
        let physical_idx = ctx.physical_field_index(struct_name, idx);

        // Get source field pointer
        let src_field_ptr = unsafe {
            ctx.builder
                .build_gep(
                    struct_type,
                    src_ptr,
                    &[
                        ctx.context.i32_type().const_zero(),
                        ctx.context.i32_type().const_int(physical_idx as u64, false),
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
                        ctx.context.i32_type().const_int(physical_idx as u64, false),
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
            Some(TypeKind::Struct {
                ref name,
                ref fields,
            }) if src_val.is_pointer_value() => {
                // Deep-clone nested struct fields to prevent use-after-free
                let fp: Vec<_> = fields.iter().map(|(n, t, _)| (n.clone(), *t)).collect();
                let sn = name.clone();
                clone_struct(ctx, src_val.into_pointer_value(), &sn, &fp)
                    .map(|p| p.into())
                    .unwrap_or(src_val)
            }
            Some(TypeKind::Array { element }) if src_val.is_pointer_value() => {
                // Deep-clone nested array fields
                let elem_ty = element;
                clone_array(ctx, src_val.into_pointer_value(), elem_ty)
                    .map(|p| p.into())
                    .unwrap_or(src_val)
            }
            Some(TypeKind::Map { key, value }) if src_val.is_pointer_value() => {
                // Deep-clone nested map fields
                clone_map(ctx, src_val.into_pointer_value(), key, value)
                    .map(|p| p.into())
                    .unwrap_or(src_val)
            }
            // Primitives and unknowns: direct copy
            _ => src_val,
        };

        // Store to dest
        ctx.builder.build_store(dst_field_ptr, cloned_val).ok()?;
    }

    // IMPORTANT: Get the current block after all field operations, because
    // clone_string may have created additional blocks (clone_str/clone_merge)
    // and we need the actual final block as our phi predecessor
    let final_clone_block = ctx.builder.get_insert_block()?;
    ctx.builder.build_unconditional_branch(merge_block).ok()?;

    // Merge: PHI between null (null path) and deep clone (non-null path)
    ctx.builder.position_at_end(merge_block);
    let phi = ctx.builder.build_phi(ptr_type, "cloned_struct").ok()?;
    phi.add_incoming(&[
        (&ptr_type.const_null(), current_block),
        (&dst_ptr, final_clone_block),
    ]);

    // Pass through opaque identity function to prevent LLVM from exploiting
    // the null value in the PHI for UB-based optimizations
    let opaque_fn = ctx.get_or_create_doo_opaque_ptr();
    let safe_result = ctx
        .builder
        .build_call(opaque_fn, &[phi.as_basic_value().into()], "safe_struct")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_pointer_value();

    Some(safe_result)
}

/// Clone an array by allocating new memory and copying elements.
///
/// IMPORTANT: Handles null pointers safely - if src_ptr is null, returns null.
/// This is critical for error handling where error values may be null when
/// the operation succeeded.
///
/// For primitive types (Int, Float, Bool), uses memcpy for efficiency.
/// For pointer types (Str), clones each element individually.
fn clone_array<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    src_ptr: inkwell::values::PointerValue<'ctx>,
    element_type: doo_core::types::TypeId,
) -> Option<inkwell::values::PointerValue<'ctx>> {
    use crate::layout::{alloc_with_header, copy_memory, get_array_length_from_data};
    use doo_core::types::builtin;

    let ptr_type = ctx.ptr_type();
    let i64_type = ctx.context.i64_type();

    // Handle null pointer case — needed for optional/error fields.
    // CRITICAL: On null path, allocate a valid zero-length array instead of returning
    // const_null(). This prevents LLVM O3 from exploiting UB on subsequent accesses.
    let is_null = ctx.builder.build_is_null(src_ptr, "arr_is_null").ok()?;

    let current_block = ctx.builder.get_insert_block()?;
    let func = current_block.get_parent()?;
    let clone_block = ctx.context.append_basic_block(func, "clone_array_do");
    let merge_block = ctx.context.append_basic_block(func, "clone_array_merge");

    ctx.builder
        .build_conditional_branch(is_null, merge_block, clone_block)
        .ok()?;

    // Clone block: allocate new array and copy elements
    ctx.builder.position_at_end(clone_block);

    // Get source array length
    let src_len = get_array_length_from_data(ctx, src_ptr)?;
    let src_len_i32 = ctx
        .builder
        .build_int_truncate(src_len, ctx.context.i32_type(), "len_i32")
        .ok()?;

    // In Doo, all array elements are 8 bytes (pointers or i64)
    let elem_size = i64_type.const_int(8, false);

    // Allocate new array with same length (use i64 as element type for allocation)
    let dst_ptr = alloc_with_header(ctx, src_len_i32, i64_type, "cloned_arr")?;

    // Check if this is a string or struct type that needs deep cloning
    let is_str = element_type == builtin::STR;
    let type_kind = ctx.get_type_kind(element_type);

    // Extract struct info BEFORE consuming type_kind, so we can deep-clone
    // struct elements. Without this, clone_array does a shallow pointer copy
    // and the original array's drop frees the structs — leaving dangling ptrs.
    let struct_info: Option<(String, Vec<(String, doo_core::types::TypeId)>)> = match &type_kind {
        Some(doo_core::types::TypeKind::Struct { name, fields }) => {
            let field_pairs: Vec<_> = fields.iter().map(|(n, t, _)| (n.clone(), *t)).collect();
            Some((name.clone(), field_pairs))
        }
        _ => None,
    };

    let needs_deep_clone = is_str || struct_info.is_some();

    if needs_deep_clone {
        // For strings and structs, iterate and clone each element
        // Create loop: for i = 0; i < len; i++
        let loop_preheader = ctx.builder.get_insert_block()?;
        let loop_body = ctx.context.append_basic_block(func, "clone_loop_body");
        let loop_end = ctx.context.append_basic_block(func, "clone_loop_end");

        // Check if length > 0
        let has_elements = ctx
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SGT,
                src_len,
                i64_type.const_zero(),
                "has_elements",
            )
            .ok()?;
        ctx.builder
            .build_conditional_branch(has_elements, loop_body, loop_end)
            .ok()?;

        // Loop body
        ctx.builder.position_at_end(loop_body);
        let idx_phi = ctx.builder.build_phi(i64_type, "idx").ok()?;
        idx_phi.add_incoming(&[(&i64_type.const_zero(), loop_preheader)]);
        let idx = idx_phi.as_basic_value().into_int_value();

        // Get source element pointer (src_ptr is data ptr, offset by idx * elem_size)
        let src_offset = ctx
            .builder
            .build_int_mul(idx, elem_size, "src_offset")
            .ok()?;
        let src_elem_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), src_ptr, &[src_offset], "src_elem")
                .ok()?
        };

        // Get dest element pointer
        let dst_offset = ctx
            .builder
            .build_int_mul(idx, elem_size, "dst_offset")
            .ok()?;
        let dst_elem_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), dst_ptr, &[dst_offset], "dst_elem")
                .ok()?
        };

        // Load source element and clone based on type
        if is_str {
            let src_val = ctx
                .builder
                .build_load(ptr_type, src_elem_ptr, "src_str")
                .ok()?
                .into_pointer_value();
            let cloned_val = clone_string(ctx, src_val).unwrap_or(src_val);
            ctx.builder.build_store(dst_elem_ptr, cloned_val).ok()?;
        } else if let Some((ref struct_name, ref field_pairs)) = struct_info {
            // Deep-clone struct elements to prevent use-after-free.
            // The original array's drop will free the original structs,
            // so the cloned array must own independent copies.
            let src_val = ctx
                .builder
                .build_load(ptr_type, src_elem_ptr, "src_ptr")
                .ok()?
                .into_pointer_value();
            let cloned_val =
                clone_struct(ctx, src_val, struct_name, field_pairs).unwrap_or(src_val);
            ctx.builder.build_store(dst_elem_ptr, cloned_val).ok()?;
        } else {
            // Fallback: copy the pointer
            let src_val = ctx
                .builder
                .build_load(ptr_type, src_elem_ptr, "src_ptr")
                .ok()?;
            ctx.builder.build_store(dst_elem_ptr, src_val).ok()?;
        }

        // CRITICAL: Capture current block AFTER all operations (clone_string may have changed it)
        let loop_back_block = ctx.builder.get_insert_block()?;

        // Increment index
        let next_idx = ctx
            .builder
            .build_int_add(idx, i64_type.const_int(1, false), "next_idx")
            .ok()?;

        // Use the actual current block as predecessor, not the original loop_body
        idx_phi.add_incoming(&[(&next_idx, loop_back_block)]);

        // Check loop condition
        let continue_loop = ctx
            .builder
            .build_int_compare(inkwell::IntPredicate::SLT, next_idx, src_len, "continue")
            .ok()?;
        ctx.builder
            .build_conditional_branch(continue_loop, loop_body, loop_end)
            .ok()?;

        // Loop end
        ctx.builder.position_at_end(loop_end);
    } else {
        // For primitives (Int, Float, Bool), use efficient memcpy
        let copy_size = ctx
            .builder
            .build_int_mul(src_len, elem_size, "copy_size")
            .ok()?;
        copy_memory(ctx, dst_ptr, src_ptr, copy_size)?;
    }

    let final_clone_block = ctx.builder.get_insert_block()?;
    ctx.builder.build_unconditional_branch(merge_block).ok()?;

    // Merge: PHI between null (null path) and deep clone (non-null path)
    ctx.builder.position_at_end(merge_block);
    let phi = ctx.builder.build_phi(ptr_type, "cloned_arr").ok()?;
    phi.add_incoming(&[
        (&ptr_type.const_null(), current_block),
        (&dst_ptr, final_clone_block),
    ]);

    // Pass through opaque identity function to prevent LLVM from exploiting null UB
    let opaque_fn = ctx.get_or_create_doo_opaque_ptr();
    let safe_result = ctx
        .builder
        .build_call(opaque_fn, &[phi.as_basic_value().into()], "safe_arr")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_pointer_value();

    Some(safe_result)
}

/// Clone a map by allocating new memory and copying key-value pairs.
///
/// IMPORTANT: Handles null pointers safely - if src_ptr is null, returns null.
/// This is critical for error handling where error values may be null when
/// the operation succeeded.
///
/// Maps are stored as arrays of (key, value) pairs with a header.
fn clone_map<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    src_ptr: inkwell::values::PointerValue<'ctx>,
    key_type: doo_core::types::TypeId,
    value_type: doo_core::types::TypeId,
) -> Option<inkwell::values::PointerValue<'ctx>> {
    use crate::layout::{alloc_with_header, copy_memory, get_array_length_from_data};
    use doo_core::types::builtin;

    let ptr_type = ctx.ptr_type();
    let i64_type = ctx.context.i64_type();

    let current_block = ctx.builder.get_insert_block()?;
    let func = current_block.get_parent()?;
    let clone_block = ctx.context.append_basic_block(func, "clone_map_do");
    let merge_block = ctx.context.append_basic_block(func, "clone_map_merge");

    // Handle null pointer case — needed for optional/error fields.
    let is_null = ctx.builder.build_is_null(src_ptr, "map_is_null").ok()?;
    ctx.builder
        .build_conditional_branch(is_null, merge_block, clone_block)
        .ok()?;

    // Clone block: allocate new map and copy entries
    ctx.builder.position_at_end(clone_block);

    // Get source map length
    let src_len = get_array_length_from_data(ctx, src_ptr)?;

    // Use i8 array type for map entries (MAP_ENTRY_SIZE bytes per entry)
    let pair_type = ctx
        .context
        .i8_type()
        .array_type(crate::layout::MAP_ENTRY_SIZE as u32);

    // Allocate new map with same length (pass i64 directly, no truncation)
    let dst_ptr = alloc_with_header(ctx, src_len, pair_type, "cloned_map")?;

    // Check if keys or values need deep cloning (strings)
    let key_is_str = key_type == builtin::STR;
    let val_is_str = value_type == builtin::STR;
    let needs_deep_clone = key_is_str || val_is_str;

    if needs_deep_clone {
        // For string keys/values, iterate and clone
        let loop_preheader = ctx.builder.get_insert_block()?;
        let loop_body = ctx.context.append_basic_block(func, "clone_map_loop");
        let loop_end = ctx.context.append_basic_block(func, "clone_map_end");

        let has_elements = ctx
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SGT,
                src_len,
                i64_type.const_zero(),
                "has_entries",
            )
            .ok()?;
        ctx.builder
            .build_conditional_branch(has_elements, loop_body, loop_end)
            .ok()?;

        ctx.builder.position_at_end(loop_body);
        let idx_phi = ctx.builder.build_phi(i64_type, "idx").ok()?;
        idx_phi.add_incoming(&[(&i64_type.const_zero(), loop_preheader)]);
        let idx = idx_phi.as_basic_value().into_int_value();

        // Calculate entry offset (idx * MAP_ENTRY_SIZE for fixed pair size)
        let entry_size = i64_type.const_int(crate::layout::MAP_ENTRY_SIZE, false);
        let entry_offset = ctx
            .builder
            .build_int_mul(idx, entry_size, "entry_offset")
            .ok()?;

        let src_entry_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), src_ptr, &[entry_offset], "src_entry")
                .ok()?
        };
        let dst_entry_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), dst_ptr, &[entry_offset], "dst_entry")
                .ok()?
        };

        // Clone key if string
        if key_is_str {
            let src_key = ctx
                .builder
                .build_load(ptr_type, src_entry_ptr, "src_key")
                .ok()?
                .into_pointer_value();
            let cloned_key = clone_string(ctx, src_key).unwrap_or(src_key);
            ctx.builder.build_store(dst_entry_ptr, cloned_key).ok()?;
        } else {
            let src_key = ctx
                .builder
                .build_load(i64_type, src_entry_ptr, "src_key")
                .ok()?;
            ctx.builder.build_store(dst_entry_ptr, src_key).ok()?;
        }

        // Clone value (offset by 8 bytes)
        let val_offset = ctx
            .builder
            .build_int_add(entry_offset, i64_type.const_int(8, false), "val_offset")
            .ok()?;
        let src_val_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), src_ptr, &[val_offset], "src_val_ptr")
                .ok()?
        };
        let dst_val_ptr = unsafe {
            ctx.builder
                .build_gep(ctx.context.i8_type(), dst_ptr, &[val_offset], "dst_val_ptr")
                .ok()?
        };

        if val_is_str {
            let src_val = ctx
                .builder
                .build_load(ptr_type, src_val_ptr, "src_val")
                .ok()?
                .into_pointer_value();
            let cloned_val = clone_string(ctx, src_val).unwrap_or(src_val);
            ctx.builder.build_store(dst_val_ptr, cloned_val).ok()?;
        } else {
            let src_val = ctx
                .builder
                .build_load(i64_type, src_val_ptr, "src_val")
                .ok()?;
            ctx.builder.build_store(dst_val_ptr, src_val).ok()?;
        }

        // CRITICAL: Capture current block AFTER all operations (clone_string may have changed it)
        let loop_back_block = ctx.builder.get_insert_block()?;

        // Increment and loop
        let next_idx = ctx
            .builder
            .build_int_add(idx, i64_type.const_int(1, false), "next_idx")
            .ok()?;

        // Use the actual current block as predecessor, not the original loop_body
        idx_phi.add_incoming(&[(&next_idx, loop_back_block)]);

        let continue_loop = ctx
            .builder
            .build_int_compare(inkwell::IntPredicate::SLT, next_idx, src_len, "continue")
            .ok()?;
        ctx.builder
            .build_conditional_branch(continue_loop, loop_body, loop_end)
            .ok()?;

        ctx.builder.position_at_end(loop_end);
    } else {
        // For primitive key/value types, use efficient memcpy
        let entry_size = i64_type.const_int(crate::layout::MAP_ENTRY_SIZE, false);
        let copy_size = ctx
            .builder
            .build_int_mul(src_len, entry_size, "copy_size")
            .ok()?;
        copy_memory(ctx, dst_ptr, src_ptr, copy_size)?;
    }

    let final_clone_block = ctx.builder.get_insert_block()?;
    ctx.builder.build_unconditional_branch(merge_block).ok()?;

    // Merge: PHI between null (null path) and deep clone (non-null path)
    ctx.builder.position_at_end(merge_block);
    let phi = ctx.builder.build_phi(ptr_type, "cloned_map").ok()?;
    phi.add_incoming(&[
        (&ptr_type.const_null(), current_block),
        (&dst_ptr, final_clone_block),
    ]);

    // Pass through opaque identity function to prevent LLVM from exploiting null UB
    let opaque_fn = ctx.get_or_create_doo_opaque_ptr();
    let safe_result = ctx
        .builder
        .build_call(opaque_fn, &[phi.as_basic_value().into()], "safe_map")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_pointer_value();

    Some(safe_result)
}

/// Clone an optional value.
///
/// IMPORTANT: Handles null pointers safely - if src_ptr is null, returns null.
/// This is critical for error handling where error values may be null when
/// the operation succeeded.
///
/// Optional values are represented as nullable pointers - null means None,
/// non-null means Some(value). For non-null, we clone the inner value.
fn clone_optional<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    src_ptr: inkwell::values::PointerValue<'ctx>,
    inner_type: doo_core::types::TypeId,
) -> Option<inkwell::values::PointerValue<'ctx>> {
    use doo_core::types::builtin;

    let ptr_type = ctx.ptr_type();

    // Handle null pointer case - null means None, so return null
    let is_null = ctx
        .builder
        .build_is_null(src_ptr, "optional_is_null")
        .ok()?;

    let current_block = ctx.builder.get_insert_block()?;
    let func = current_block.get_parent()?;
    let clone_block = ctx.context.append_basic_block(func, "clone_optional_do");
    let merge_block = ctx.context.append_basic_block(func, "clone_optional_merge");

    ctx.builder
        .build_conditional_branch(is_null, merge_block, clone_block)
        .ok()?;

    // Clone block: clone the inner value based on its type
    ctx.builder.position_at_end(clone_block);

    let cloned_ptr = if inner_type == builtin::STR {
        // String: deep clone
        clone_string(ctx, src_ptr).unwrap_or(src_ptr)
    } else {
        // For other types, just use the same pointer (shallow copy)
        // This is safe because we've already handled null above
        src_ptr
    };

    ctx.builder.build_unconditional_branch(merge_block).ok()?;

    // Merge block: phi between null and cloned
    ctx.builder.position_at_end(merge_block);
    let phi = ctx.builder.build_phi(ptr_type, "cloned_optional").ok()?;
    phi.add_incoming(&[
        (&ptr_type.const_null(), current_block),
        (&cloned_ptr, clone_block),
    ]);

    Some(phi.as_basic_value().into_pointer_value())
}

// ============================================================================
// Drop (Cleanup) Implementation
// ============================================================================

/// Emit drop (cleanup) for a variable based on its type.
pub(crate) fn emit_drop<'ctx>(ctx: &mut CodegenContext<'ctx>, var_name: &str) {
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
            // Extract just name and type for drop_struct (visibility not needed)
            let field_pairs: Vec<_> = fields.iter().map(|(n, t, _)| (n.clone(), *t)).collect();
            drop_struct(ctx, ptr, name, &field_pairs);
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
            // Apply P06 logical→physical field remapping
            let physical_idx = ctx.physical_field_index(struct_name, idx);
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
                // Get field pointer using physical index (P06 remapped)
                let field_ptr = unsafe {
                    match ctx.builder.build_gep(
                        struct_type,
                        ptr,
                        &[
                            ctx.context.i32_type().const_zero(),
                            ctx.context.i32_type().const_int(physical_idx as u64, false),
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
                            // Extract just name and type for nested drop_struct
                            let nested_pairs: Vec<_> = nested_fields
                                .iter()
                                .map(|(n, t, _)| (n.clone(), *t))
                                .collect();
                            drop_struct(ctx, field_ptr_val, &nested_name, &nested_pairs);
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

/// Drop an array by dropping each element (if element type needs cleanup), then freeing.
/// Array layout: [length: i64][capacity: i64][data...]
/// We store the DATA pointer, so we need to go back 16 bytes to get the header.
fn drop_array<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    ptr: inkwell::values::PointerValue<'ctx>,
    element_type: doo_core::types::TypeId,
) {
    use crate::layout::get_array_length_from_data;

    // CRITICAL: Check if data pointer is null BEFORE calculating header pointer!
    let is_null = match ctx.builder.build_is_null(ptr, "arr_data_is_null") {
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

    let free_block = ctx.context.append_basic_block(func, "drop_arr_free");
    let continue_block = ctx.context.append_basic_block(func, "drop_arr_continue");

    if ctx
        .builder
        .build_conditional_branch(is_null, continue_block, free_block)
        .is_err()
    {
        return;
    }

    ctx.builder.position_at_end(free_block);

    // Check if elements need dropping (non-primitive pointer types)
    let type_kind = ctx.get_type_kind(element_type);
    let needs_element_drop = matches!(
        type_kind,
        Some(TypeKind::Str)
            | Some(TypeKind::Array { .. })
            | Some(TypeKind::Map { .. })
            | Some(TypeKind::Struct { .. })
            | Some(TypeKind::Optional { .. })
    );

    if needs_element_drop {
        // Get array length and iterate elements
        let i64_type = ctx.context.i64_type();
        let ptr_type = ctx.ptr_type();
        let elem_size = i64_type.const_int(8, false); // All elements are 8 bytes (ptrs or i64)

        if let Some(arr_len) = get_array_length_from_data(ctx, ptr) {
            // Create element drop loop
            let loop_body = ctx.context.append_basic_block(func, "arr_drop_loop");
            let loop_end = ctx.context.append_basic_block(func, "arr_drop_done");

            let has_elements = ctx
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::SGT,
                    arr_len,
                    i64_type.const_zero(),
                    "has_elems",
                )
                .unwrap_or_else(|_| ctx.context.bool_type().const_zero());

            let loop_preheader = ctx.builder.get_insert_block().unwrap_or(free_block);
            let _ = ctx
                .builder
                .build_conditional_branch(has_elements, loop_body, loop_end);

            // Loop body
            ctx.builder.position_at_end(loop_body);
            let idx_phi = match ctx.builder.build_phi(i64_type, "drop_idx") {
                Ok(p) => p,
                Err(_) => {
                    ctx.builder.position_at_end(loop_end);
                    // Fall through to free
                    let i8_type = ctx.context.i8_type();
                    let i32_type = ctx.context.i32_type();
                    let header_ptr = unsafe {
                        ctx.builder
                            .build_gep(
                                i8_type,
                                ptr,
                                &[i32_type.const_int((-16i64) as u64, true)],
                                "arr_header_ptr",
                            )
                            .ok()
                    };
                    if let Some(hp) = header_ptr {
                        let free_fn = get_or_declare_free(ctx);
                        let _ = ctx.builder.build_call(free_fn, &[hp.into()], "");
                    }
                    let _ = ctx.builder.build_unconditional_branch(continue_block);
                    ctx.builder.position_at_end(continue_block);
                    return;
                }
            };
            idx_phi.add_incoming(&[(&i64_type.const_zero(), loop_preheader)]);
            let idx = idx_phi.as_basic_value().into_int_value();

            // Load element at ptr + idx * 8
            let offset = ctx
                .builder
                .build_int_mul(idx, elem_size, "elem_offset")
                .unwrap_or(i64_type.const_zero());
            let elem_ptr = unsafe {
                match ctx
                    .builder
                    .build_gep(ctx.context.i8_type(), ptr, &[offset], "elem_ptr")
                {
                    Ok(p) => p,
                    Err(_) => {
                        let _ = ctx.builder.build_unconditional_branch(loop_end);
                        ctx.builder.position_at_end(loop_end);
                        // Fall through to header free below
                        let i8_type = ctx.context.i8_type();
                        let i32_type = ctx.context.i32_type();
                        let header_ptr = unsafe {
                            ctx.builder
                                .build_gep(
                                    i8_type,
                                    ptr,
                                    &[i32_type.const_int((-16i64) as u64, true)],
                                    "arr_header_ptr",
                                )
                                .ok()
                        };
                        if let Some(hp) = header_ptr {
                            let free_fn = get_or_declare_free(ctx);
                            let _ = ctx.builder.build_call(free_fn, &[hp.into()], "");
                        }
                        let _ = ctx.builder.build_unconditional_branch(continue_block);
                        ctx.builder.position_at_end(continue_block);
                        return;
                    }
                }
            };

            // Load the element pointer value
            if let Ok(elem_val) = ctx.builder.build_load(ptr_type, elem_ptr, "elem_val") {
                if elem_val.is_pointer_value() {
                    let elem_ptr_val = elem_val.into_pointer_value();
                    // Drop based on element type
                    match &type_kind {
                        Some(TypeKind::Str) => drop_string(ctx, elem_ptr_val),
                        Some(TypeKind::Struct { name, fields }) => {
                            let name = name.clone();
                            let pairs: Vec<_> =
                                fields.iter().map(|(n, t, _)| (n.clone(), *t)).collect();
                            drop_struct(ctx, elem_ptr_val, &name, &pairs);
                        }
                        Some(TypeKind::Array { element }) => {
                            drop_array(ctx, elem_ptr_val, *element);
                        }
                        Some(TypeKind::Map { key, value }) => {
                            drop_map(ctx, elem_ptr_val, *key, *value);
                        }
                        Some(TypeKind::Optional { inner }) => {
                            drop_optional(ctx, elem_ptr_val, *inner);
                        }
                        _ => drop_pointer(ctx, elem_ptr_val),
                    }
                }
            }

            // Get the current block (may have changed due to recursive drops)
            let loop_back = ctx.builder.get_insert_block().unwrap_or(loop_body);

            // Increment index
            let next_idx = ctx
                .builder
                .build_int_add(idx, i64_type.const_int(1, false), "next_idx")
                .unwrap_or(i64_type.const_zero());
            idx_phi.add_incoming(&[(&next_idx, loop_back)]);

            let continue_loop = ctx
                .builder
                .build_int_compare(inkwell::IntPredicate::SLT, next_idx, arr_len, "cont_loop")
                .unwrap_or_else(|_| ctx.context.bool_type().const_zero());
            let _ = ctx
                .builder
                .build_conditional_branch(continue_loop, loop_body, loop_end);

            // After loop: free
            ctx.builder.position_at_end(loop_end);
        }
    }

    // Free the header allocation (data_ptr - 16)
    let i8_type = ctx.context.i8_type();
    let i32_type = ctx.context.i32_type();

    let header_ptr = match unsafe {
        ctx.builder.build_gep(
            i8_type,
            ptr,
            &[i32_type.const_int((-16i64) as u64, true)],
            "arr_header_ptr",
        )
    } {
        Ok(p) => p,
        Err(_) => {
            ctx.builder.position_at_end(continue_block);
            return;
        }
    };

    let free_fn = get_or_declare_free(ctx);
    let _ = ctx.builder.build_call(free_fn, &[header_ptr.into()], "");
    let _ = ctx.builder.build_unconditional_branch(continue_block);

    ctx.builder.position_at_end(continue_block);
}

/// Drop a map by iterating entries, dropping keys/values that need cleanup, then freeing.
/// Map layout: [length: i64][capacity: i64][entries...]
/// Each entry is MAP_ENTRY_SIZE (16) bytes: key (8 bytes) + value (8 bytes).
fn drop_map<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    ptr: inkwell::values::PointerValue<'ctx>,
    key_type: doo_core::types::TypeId,
    value_type: doo_core::types::TypeId,
) {
    use crate::layout::{get_map_length_from_data, MAP_ENTRY_SIZE};

    let is_null = match ctx.builder.build_is_null(ptr, "map_data_is_null") {
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

    let free_block = ctx.context.append_basic_block(func, "drop_map_free");
    let continue_block = ctx.context.append_basic_block(func, "drop_map_continue");

    if ctx
        .builder
        .build_conditional_branch(is_null, continue_block, free_block)
        .is_err()
    {
        return;
    }

    ctx.builder.position_at_end(free_block);

    // Check if keys or values need dropping
    let key_kind = ctx.get_type_kind(key_type);
    let value_kind = ctx.get_type_kind(value_type);

    let key_needs_drop = matches!(
        key_kind,
        Some(TypeKind::Str)
            | Some(TypeKind::Array { .. })
            | Some(TypeKind::Map { .. })
            | Some(TypeKind::Struct { .. })
            | Some(TypeKind::Optional { .. })
    );
    let value_needs_drop = matches!(
        value_kind,
        Some(TypeKind::Str)
            | Some(TypeKind::Array { .. })
            | Some(TypeKind::Map { .. })
            | Some(TypeKind::Struct { .. })
            | Some(TypeKind::Optional { .. })
    );

    if key_needs_drop || value_needs_drop {
        let i64_type = ctx.context.i64_type();
        let ptr_type = ctx.ptr_type();
        let entry_size = i64_type.const_int(MAP_ENTRY_SIZE, false);

        if let Some(map_len) = get_map_length_from_data(ctx, ptr) {
            let loop_body = ctx.context.append_basic_block(func, "map_drop_loop");
            let loop_end = ctx.context.append_basic_block(func, "map_drop_done");

            let has_entries = ctx
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::SGT,
                    map_len,
                    i64_type.const_zero(),
                    "has_entries",
                )
                .unwrap_or_else(|_| ctx.context.bool_type().const_zero());

            let loop_preheader = ctx.builder.get_insert_block().unwrap_or(free_block);
            let _ = ctx
                .builder
                .build_conditional_branch(has_entries, loop_body, loop_end);

            // Loop body
            ctx.builder.position_at_end(loop_body);
            let idx_phi = match ctx.builder.build_phi(i64_type, "map_drop_idx") {
                Ok(p) => p,
                Err(_) => {
                    ctx.builder.position_at_end(loop_end);
                    // Fall through to free below
                    goto_map_free(ctx, ptr, continue_block);
                    return;
                }
            };
            idx_phi.add_incoming(&[(&i64_type.const_zero(), loop_preheader)]);
            let idx = idx_phi.as_basic_value().into_int_value();

            // Calculate entry offset: idx * MAP_ENTRY_SIZE
            let offset = ctx
                .builder
                .build_int_mul(idx, entry_size, "entry_offset")
                .unwrap_or(i64_type.const_zero());

            // Drop key if needed (key is at entry + 0)
            if key_needs_drop {
                if let Ok(key_ptr) = unsafe {
                    ctx.builder
                        .build_gep(ctx.context.i8_type(), ptr, &[offset], "key_ptr")
                } {
                    if let Ok(key_val) = ctx.builder.build_load(ptr_type, key_ptr, "key_val") {
                        if key_val.is_pointer_value() {
                            drop_by_type_kind(ctx, key_val.into_pointer_value(), &key_kind);
                        }
                    }
                }
            }

            // Drop value if needed (value is at entry + 8)
            if value_needs_drop {
                let val_byte_offset = ctx
                    .builder
                    .build_int_add(offset, i64_type.const_int(8, false), "val_offset")
                    .unwrap_or(i64_type.const_zero());
                if let Ok(val_ptr) = unsafe {
                    ctx.builder
                        .build_gep(ctx.context.i8_type(), ptr, &[val_byte_offset], "val_ptr")
                } {
                    if let Ok(val_val) = ctx.builder.build_load(ptr_type, val_ptr, "val_val") {
                        if val_val.is_pointer_value() {
                            drop_by_type_kind(ctx, val_val.into_pointer_value(), &value_kind);
                        }
                    }
                }
            }

            // Get current block (may have changed due to recursive drops)
            let loop_back = ctx.builder.get_insert_block().unwrap_or(loop_body);

            let next_idx = ctx
                .builder
                .build_int_add(idx, i64_type.const_int(1, false), "next_idx")
                .unwrap_or(i64_type.const_zero());
            idx_phi.add_incoming(&[(&next_idx, loop_back)]);

            let cont = ctx
                .builder
                .build_int_compare(inkwell::IntPredicate::SLT, next_idx, map_len, "cont_map")
                .unwrap_or_else(|_| ctx.context.bool_type().const_zero());
            let _ = ctx
                .builder
                .build_conditional_branch(cont, loop_body, loop_end);

            ctx.builder.position_at_end(loop_end);
        }
    }

    // Free the header allocation (data_ptr - 16)
    let i8_type = ctx.context.i8_type();
    let i32_type = ctx.context.i32_type();

    let header_ptr = match unsafe {
        ctx.builder.build_gep(
            i8_type,
            ptr,
            &[i32_type.const_int((-16i64) as u64, true)],
            "map_header_ptr",
        )
    } {
        Ok(p) => p,
        Err(_) => {
            ctx.builder.position_at_end(continue_block);
            return;
        }
    };

    let free_fn = get_or_declare_free(ctx);
    let _ = ctx.builder.build_call(free_fn, &[header_ptr.into()], "");
    let _ = ctx.builder.build_unconditional_branch(continue_block);

    ctx.builder.position_at_end(continue_block);
}

/// Helper: Jump to map free (header_ptr - 16 → free → continue)
fn goto_map_free<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    ptr: inkwell::values::PointerValue<'ctx>,
    continue_block: inkwell::basic_block::BasicBlock<'ctx>,
) {
    let i8_type = ctx.context.i8_type();
    let i32_type = ctx.context.i32_type();
    if let Ok(header_ptr) = unsafe {
        ctx.builder.build_gep(
            i8_type,
            ptr,
            &[i32_type.const_int((-16i64) as u64, true)],
            "map_header_ptr",
        )
    } {
        let free_fn = get_or_declare_free(ctx);
        let _ = ctx.builder.build_call(free_fn, &[header_ptr.into()], "");
    }
    let _ = ctx.builder.build_unconditional_branch(continue_block);
}

/// Drop a pointer value based on its TypeKind (used by map/array element iteration).
fn drop_by_type_kind<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    ptr: inkwell::values::PointerValue<'ctx>,
    kind: &Option<TypeKind>,
) {
    match kind {
        Some(TypeKind::Str) => drop_string(ctx, ptr),
        Some(TypeKind::Struct { name, fields }) => {
            let name = name.clone();
            let pairs: Vec<_> = fields.iter().map(|(n, t, _)| (n.clone(), *t)).collect();
            drop_struct(ctx, ptr, &name, &pairs);
        }
        Some(TypeKind::Array { element }) => drop_array(ctx, ptr, *element),
        Some(TypeKind::Map { key, value }) => drop_map(ctx, ptr, *key, *value),
        Some(TypeKind::Optional { inner }) => drop_optional(ctx, ptr, *inner),
        _ => drop_pointer(ctx, ptr),
    }
}

/// Drop an optional value by checking has_value, dropping inner if present, then freeing.
/// Optional layout: { i8 has_value, T inner_value }
fn drop_optional<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    ptr: inkwell::values::PointerValue<'ctx>,
    inner_type: doo_core::types::TypeId,
) {
    let is_null = match ctx.builder.build_is_null(ptr, "opt_is_null") {
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

    let free_block = ctx.context.append_basic_block(func, "drop_opt_free");
    let continue_block = ctx.context.append_basic_block(func, "drop_opt_continue");

    if ctx
        .builder
        .build_conditional_branch(is_null, continue_block, free_block)
        .is_err()
    {
        return;
    }

    ctx.builder.position_at_end(free_block);

    let inner_kind = ctx.get_type_kind(inner_type);
    let inner_needs_drop = matches!(
        inner_kind,
        Some(TypeKind::Str)
            | Some(TypeKind::Array { .. })
            | Some(TypeKind::Map { .. })
            | Some(TypeKind::Struct { .. })
            | Some(TypeKind::Optional { .. })
    );

    if inner_needs_drop {
        // For droppable types, inner is always a pointer (8 bytes).
        // Optional struct is { i8, ptr } with natural alignment.
        let i8_type = ctx.context.i8_type();
        let ptr_type = ctx.ptr_type();
        let opt_struct_type = ctx
            .context
            .struct_type(&[i8_type.into(), ptr_type.into()], false);

        // Load has_value flag (field 0)
        if let Ok(has_val_ptr) =
            ctx.builder
                .build_struct_gep(opt_struct_type, ptr, 0, "has_val_ptr")
        {
            if let Ok(has_val) = ctx.builder.build_load(i8_type, has_val_ptr, "has_val") {
                let has_val_int = has_val.into_int_value();
                let is_some = ctx
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        has_val_int,
                        i8_type.const_zero(),
                        "is_some",
                    )
                    .unwrap_or_else(|_| ctx.context.bool_type().const_zero());

                let drop_inner = ctx.context.append_basic_block(func, "opt_drop_inner");
                let skip_inner = ctx.context.append_basic_block(func, "opt_skip_inner");

                let _ = ctx
                    .builder
                    .build_conditional_branch(is_some, drop_inner, skip_inner);

                // Drop inner value if present
                ctx.builder.position_at_end(drop_inner);
                if let Ok(inner_ptr) =
                    ctx.builder
                        .build_struct_gep(opt_struct_type, ptr, 1, "inner_ptr")
                {
                    if let Ok(inner_val) = ctx.builder.build_load(ptr_type, inner_ptr, "inner_val")
                    {
                        if inner_val.is_pointer_value() {
                            drop_by_type_kind(ctx, inner_val.into_pointer_value(), &inner_kind);
                        }
                    }
                }
                // Branch to skip_inner (get current block since recursive drop may have changed it)
                let _ = ctx.builder.build_unconditional_branch(skip_inner);

                ctx.builder.position_at_end(skip_inner);
            }
        }
    }

    // Free the optional struct
    let free_fn = get_or_declare_free(ctx);
    let _ = ctx.builder.build_call(free_fn, &[ptr.into()], "");
    let _ = ctx.builder.build_unconditional_branch(continue_block);

    ctx.builder.position_at_end(continue_block);
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use doo_core::types::TypeRegistry;
    use doo_mir::sym::sym;
    use doo_mir::Span;
    use inkwell::context::Context;
    use std::sync::Arc;

    #[test]
    fn test_memory_handler_handles() {
        let handler = MemoryHandler;

        // Should handle Move
        let move_instr = MirInstr {
            kind: MirInstrKind::Move {
                dest: sym("x"),
                src: MirOperand::Const(doo_mir::MirConst::Int(42)),
            },
            span: Span::default(),
        };
        assert!(handler.handles(&move_instr));

        // Should handle Copy
        let copy_instr = MirInstr {
            kind: MirInstrKind::Copy {
                dest: sym("x"),
                src: MirOperand::Const(doo_mir::MirConst::Int(42)),
            },
            span: Span::default(),
        };
        assert!(handler.handles(&copy_instr));

        // Should handle Clone
        let clone_instr = MirInstr {
            kind: MirInstrKind::Clone {
                dest: sym("x"),
                src: MirOperand::Const(doo_mir::MirConst::Int(42)),
            },
            span: Span::default(),
        };
        assert!(handler.handles(&clone_instr));

        // Should handle Drop
        let drop_instr = MirInstr {
            kind: MirInstrKind::Drop { value: sym("x") },
            span: Span::default(),
        };
        assert!(handler.handles(&drop_instr));

        // Should handle Assign
        let assign_instr = MirInstr {
            kind: MirInstrKind::Assign {
                dest: sym("x"),
                value: MirOperand::Const(doo_mir::MirConst::Int(42)),
            },
            span: Span::default(),
        };
        assert!(handler.handles(&assign_instr));

        // Should handle Borrow
        let borrow_instr = MirInstr {
            kind: MirInstrKind::Borrow {
                dest: sym("ref_x"),
                src: sym("x"),
                mutable: false,
            },
            span: Span::default(),
        };
        assert!(handler.handles(&borrow_instr));

        // Should NOT handle other instructions (e.g., BinaryOp)
        let binary_op = MirInstr {
            kind: MirInstrKind::BinaryOp {
                dest: sym("y"),
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
                dest: sym("x"),
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
                dest: sym("y"),
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
                dest: sym("cloned_str"),
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
                value: sym("nonexistent"),
            },
            span: Span::default(),
        };

        let result = handler.emit(&mut codegen, &drop_instr);
        assert!(result.is_none()); // Drop returns None
    }

    #[test]
    fn test_get_operand_name() {
        // Test Local
        let local = MirOperand::Local(sym("local_var"));
        assert_eq!(get_operand_name(&local), Some("local_var".to_string()));

        // Test Temp
        let temp = MirOperand::Temp(sym("temp_0"));
        assert_eq!(get_operand_name(&temp), Some("temp_0".to_string()));

        // Test Global
        let global = MirOperand::Global(sym("global_var"));
        assert_eq!(get_operand_name(&global), Some("global_var".to_string()));

        // Test Const - should return None
        let const_val = MirOperand::Const(doo_mir::MirConst::Int(42));
        assert_eq!(get_operand_name(&const_val), None);
    }
}
