//! Enum Instruction Handler
//!
//! Handles: EnumCreate, EnumTag, EnumGetTag, EnumTagEquals, EnumPayload, EnumGetPayload
//!
//! ## Enum Layout (Centralized)
//!
//! All enums use a tagged union representation:
//! ```text
//! struct Enum {
//!     i32 tag;        // Discriminant (variant index)
//!     ptr payload;    // Pointer to payload (null if no payload)
//! }
//! ```
//!
//! This layout is consistent across all enum types for simplicity and
//! interoperability. The tag is always at offset 0, payload pointer at offset 1.

use super::InstructionHandler;
use crate::context::CodegenContext;
use doo_core::constants::ffi_names;
use doo_core::types::TypeId;
use doo_mir::sym::resolve;
use doo_mir::{MirConst, MirInstr, MirInstrKind, MirOperand};
use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;
use inkwell::IntPredicate;

// ============================================================================
// Enum Layout Constants
// ============================================================================

/// Tag field index in enum struct
const ENUM_TAG_INDEX: u32 = 0;
/// Payload field index in enum struct
const ENUM_PAYLOAD_INDEX: u32 = 1;

// ============================================================================
// Handler Implementation
// ============================================================================

/// Enum instruction handler.
pub struct EnumHandler;

impl<'ctx> InstructionHandler<'ctx> for EnumHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(
            instr.kind,
            MirInstrKind::EnumCreate { .. }
                | MirInstrKind::EnumTag { .. }
                | MirInstrKind::EnumGetTag { .. }
                | MirInstrKind::EnumTagEquals { .. }
                | MirInstrKind::EnumPayload { .. }
                | MirInstrKind::EnumGetPayload { .. }
        )
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            // ==================================================================
            // EnumCreate - Create a new enum variant
            // ==================================================================
            MirInstrKind::EnumCreate {
                dest,
                enum_name,
                variant,
                payload,
            } => emit_enum_create(
                ctx,
                &resolve(*dest),
                &resolve(*enum_name),
                &resolve(*variant),
                payload.as_ref(),
            ),

            // ==================================================================
            // EnumTag - Extract tag from enum (simple version)
            // ==================================================================
            MirInstrKind::EnumTag { dest, value } => emit_enum_tag(ctx, &resolve(*dest), value),

            // ==================================================================
            // EnumGetTag - Extract tag from enum with type info
            // ==================================================================
            MirInstrKind::EnumGetTag {
                dest,
                value,
                enum_name: _,
            } => {
                // Same implementation as EnumTag - enum_name available for future optimizations
                emit_enum_tag(ctx, &resolve(*dest), value)
            }

            // ==================================================================
            // EnumTagEquals - Compare tag with expected variant index
            // ==================================================================
            MirInstrKind::EnumTagEquals {
                dest,
                tag,
                variant_name,
                enum_name,
            } => emit_enum_tag_equals(
                ctx,
                &resolve(*dest),
                tag,
                &resolve(*variant_name),
                &resolve(*enum_name),
            ),

            // ==================================================================
            // EnumPayload - Extract payload (simple version, no type info)
            // ==================================================================
            MirInstrKind::EnumPayload {
                dest,
                value,
                variant,
            } => {
                // Extract payload pointer - variant name available for type lookup
                emit_enum_payload(ctx, &resolve(*dest), value, None, Some(&resolve(*variant)))
            }

            // ==================================================================
            // EnumGetPayload - Extract payload with type info (preferred)
            // ==================================================================
            MirInstrKind::EnumGetPayload {
                dest,
                value,
                variant_name,
                enum_name,
                index,
            } => {
                // Extract and dereference payload using type info
                // Pass index for tuple payload element extraction
                emit_enum_get_payload(
                    ctx,
                    &resolve(*dest),
                    value,
                    &resolve(*enum_name),
                    &resolve(*variant_name),
                    *index,
                )
            }

            _ => None,
        }
    }
}

// ============================================================================
// Enum Type Helper
// ============================================================================

/// Get the LLVM struct type for enums: { i32 tag, ptr payload }
fn get_enum_type<'ctx>(ctx: &CodegenContext<'ctx>) -> inkwell::types::StructType<'ctx> {
    let ptr_type = ctx.context.ptr_type(AddressSpace::default());
    ctx.context
        .struct_type(&[ctx.context.i32_type().into(), ptr_type.into()], false)
}

// ============================================================================
// Instruction Implementations
// ============================================================================

/// Emit EnumCreate instruction.
///
/// Creates an enum value with the given variant tag and optional payload.
/// Layout: { i32 tag, ptr payload }
fn emit_enum_create<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    dest: &str,
    enum_name: &str,
    variant: &str,
    payload: Option<&MirOperand>,
) -> Option<BasicValueEnum<'ctx>> {
    let enum_type = get_enum_type(ctx);
    let ptr_type = ctx.context.ptr_type(AddressSpace::default());

    // Allocate enum struct
    let enum_alloca = ctx.alloca_in_entry_block(enum_type, &format!("{}_enum", dest))?;

    // Get variant index from type registry (single source of truth)
    // Falls back to 0 if not found (shouldn't happen in well-typed programs)
    let variant_index = ctx.get_enum_variant_index(enum_name, variant).unwrap_or(0);

    // Store tag (variant index)
    let tag_ptr = ctx
        .builder
        .build_struct_gep(enum_type, enum_alloca, ENUM_TAG_INDEX, "tag_ptr")
        .ok()?;
    ctx.builder
        .build_store(
            tag_ptr,
            ctx.context
                .i32_type()
                .const_int(variant_index as u64, false),
        )
        .ok()?;

    // Store payload pointer
    let payload_ptr_field = ctx
        .builder
        .build_struct_gep(enum_type, enum_alloca, ENUM_PAYLOAD_INDEX, "payload_ptr")
        .ok()?;

    if let Some(payload_operand) = payload {
        // Get payload value
        if let Some(payload_val) = operand_to_value(ctx, payload_operand) {
            // Box the payload value - allocate on HEAP and store
            // CRITICAL: Use malloc NOT alloca - stack allocations become invalid
            // when the enum escapes the function scope.
            let payload_ptr = if payload_val.is_pointer_value() {
                // Already a pointer (Str, Array, Map, etc.) - use directly
                payload_val.into_pointer_value()
            } else {
                // Value type (Int, Float, Bool) - HEAP allocate and store
                let i64_type = ctx.i64_type();
                let heap_ptr_type = ctx.ptr_type();

                // Get or declare malloc
                let malloc_fn = ctx
                    .module
                    .get_function(ffi_names::MALLOC)
                    .unwrap_or_else(|| {
                        let fn_ty = heap_ptr_type.fn_type(&[i64_type.into()], false);
                        ctx.module.add_function(ffi_names::MALLOC, fn_ty, None)
                    });

                // Determine size based on type (minimum 8 bytes for alignment)
                let value_size = match payload_val.get_type() {
                    inkwell::types::BasicTypeEnum::IntType(it) => {
                        (it.get_bit_width() as u64 / 8).max(8)
                    }
                    inkwell::types::BasicTypeEnum::FloatType(_) => 8, // f64
                    _ => 8,                                           // Default to 8 bytes
                };

                let heap_ptr = ctx
                    .builder
                    .build_call(
                        malloc_fn,
                        &[i64_type.const_int(value_size, false).into()],
                        "payload_heap",
                    )
                    .ok()?
                    .try_as_basic_value()
                    .basic()?
                    .into_pointer_value();
                ctx.builder.build_store(heap_ptr, payload_val).ok()?;
                heap_ptr
            };
            ctx.builder
                .build_store(payload_ptr_field, payload_ptr)
                .ok()?;
        } else {
            // Failed to resolve payload - store null
            ctx.builder
                .build_store(payload_ptr_field, ptr_type.const_null())
                .ok()?;
        }
    } else {
        // No payload - store null pointer
        ctx.builder
            .build_store(payload_ptr_field, ptr_type.const_null())
            .ok()?;
    }

    // Load the complete enum value
    let enum_val = ctx
        .builder
        .build_load(enum_type, enum_alloca, &format!("{}_load", dest))
        .ok()?;

    // Store in temps
    ctx.set_temp(dest, enum_val);

    // Track enum type for this temp (used for enum-to-string conversion in FFI calls)
    ctx.temp_struct_types
        .insert(dest.to_string(), enum_name.to_string());

    Some(enum_val)
}

/// Emit EnumTag/EnumGetTag instruction.
///
/// Extracts the tag (discriminant) from an enum value.
fn emit_enum_tag<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    dest: &str,
    value: &MirOperand,
) -> Option<BasicValueEnum<'ctx>> {
    let enum_val = operand_to_value(ctx, value)?;
    let enum_type = get_enum_type(ctx);

    if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {}

    // If it's a struct value, extract directly
    if let BasicValueEnum::StructValue(struct_val) = enum_val {
        let tag_val = ctx
            .builder
            .build_extract_value(struct_val, ENUM_TAG_INDEX, &format!("{}_tag", dest))
            .ok()?;

        ctx.set_temp(dest, tag_val);
        return Some(tag_val);
    }

    // If it's a pointer, load the struct first then extract
    if enum_val.is_pointer_value() {
        let enum_ptr = enum_val.into_pointer_value();
        let loaded = ctx
            .builder
            .build_load(enum_type, enum_ptr, "enum_loaded")
            .ok()?;

        if let BasicValueEnum::StructValue(struct_val) = loaded {
            let tag_val = ctx
                .builder
                .build_extract_value(struct_val, ENUM_TAG_INDEX, &format!("{}_tag", dest))
                .ok()?;

            ctx.set_temp(dest, tag_val);
            return Some(tag_val);
        }
    }

    None
}

/// Emit EnumTagEquals instruction.
///
/// Compares an enum tag with the expected variant index.
/// Returns a boolean (i1) result.
fn emit_enum_tag_equals<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    dest: &str,
    tag: &MirOperand,
    variant_name: &str,
    enum_name: &str,
) -> Option<BasicValueEnum<'ctx>> {
    let tag_val = operand_to_value(ctx, tag)?;

    if !tag_val.is_int_value() {
        return None;
    }

    let tag_int = tag_val.into_int_value();

    // Get variant index from type registry (single source of truth)
    let expected_index = ctx
        .get_enum_variant_index(enum_name, variant_name)
        .unwrap_or(0);
    let expected_val = ctx
        .context
        .i32_type()
        .const_int(expected_index as u64, false);

    // Handle potential bit width mismatch (tag might be i64 from some paths)
    let tag_i32 = if tag_int.get_type().get_bit_width() != 32 {
        ctx.builder
            .build_int_truncate(tag_int, ctx.context.i32_type(), "tag_i32")
            .ok()?
    } else {
        tag_int
    };

    let cmp_result = ctx
        .builder
        .build_int_compare(IntPredicate::EQ, tag_i32, expected_val, dest)
        .ok()?;

    ctx.set_temp(dest, cmp_result.into());
    Some(cmp_result.into())
}

/// Emit EnumPayload/EnumGetPayload instruction.
///
/// Extracts the payload from an enum value and dereferences it based on type info.
/// If type info is provided, the payload pointer is dereferenced to get the actual value.
fn emit_enum_payload<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    dest: &str,
    value: &MirOperand,
    enum_name: Option<&str>,
    variant_name: Option<&str>,
) -> Option<BasicValueEnum<'ctx>> {
    let enum_val = operand_to_value(ctx, value)?;
    let enum_type = get_enum_type(ctx);

    // Get payload type if we have enum/variant info
    let payload_type_id = if let (Some(en), Some(vn)) = (enum_name, variant_name) {
        ctx.get_enum_variant_payload_type(en, vn)
    } else {
        None
    };

    // Helper to dereference payload pointer based on type
    // IMPORTANT: For pointer types (Str, Array, Map, Struct, etc.), the payload pointer
    // IS the value - no dereference needed. Only value types (Int, Float, Bool) need
    // to be loaded from the heap-allocated payload.
    let dereference_payload = |ctx: &mut CodegenContext<'ctx>,
                               payload_ptr: BasicValueEnum<'ctx>,
                               type_id: Option<TypeId>|
     -> BasicValueEnum<'ctx> {
        // If we don't have type info or the payload is not a pointer, return as-is
        let Some(type_id) = type_id else {
            return payload_ptr;
        };
        if !payload_ptr.is_pointer_value() {
            return payload_ptr;
        }

        let ptr = payload_ptr.into_pointer_value();

        // Check if this is a pointer type (Str, Array, Map, Struct, etc.)
        // For pointer types, the payload IS the value - don't dereference
        if let Some(type_kind) = ctx.get_type_kind(type_id) {
            match type_kind {
                // Pointer types - the payload already IS the pointer to the data
                doo_core::types::TypeKind::Str
                | doo_core::types::TypeKind::Array { .. }
                | doo_core::types::TypeKind::Map { .. }
                | doo_core::types::TypeKind::Struct { .. }
                | doo_core::types::TypeKind::Enum { .. }
                | doo_core::types::TypeKind::Tuple { .. } => {
                    // Return the payload pointer directly as the value
                    return payload_ptr.into_pointer_value().into();
                }
                // Value types - need to load from the heap-allocated payload
                doo_core::types::TypeKind::Int
                | doo_core::types::TypeKind::Float32
                | doo_core::types::TypeKind::Float64
                | doo_core::types::TypeKind::Bool => {
                    // Fall through to load logic below
                }
                // Other types - check LLVM type
                _ => {
                    // If the LLVM type is already a pointer, don't dereference
                    let llvm_type = ctx.get_llvm_type(type_id);
                    if llvm_type.is_pointer_type() {
                        return payload_ptr.into_pointer_value().into();
                    }
                }
            }
        }

        // Value types: load the actual value from the heap-allocated payload pointer
        let llvm_type = ctx.get_llvm_type(type_id);
        ctx.builder
            .build_load(llvm_type, ptr, &format!("{}_value", dest))
            .ok()
            .map(|v| v.into())
            .unwrap_or(payload_ptr.into())
    };

    // Check if dest has a type conflict - if the local has a DIFFERENT type than the payload,
    // we must NOT store to it (would cause LLVM type mismatch). Use temp storage instead.
    let has_type_conflict = if let Some(payload_tid) = payload_type_id {
        if let Some(local_tid) = ctx.get_variable_type(dest) {
            // Type conflict: local is registered with a different type than the payload
            local_tid != payload_tid
        } else {
            false
        }
    } else {
        false
    };

    // Helper to store the result based on type conflict status
    let store_result = |ctx: &mut CodegenContext<'ctx>, value: BasicValueEnum<'ctx>| {
        if has_type_conflict {
            // Type conflict - store as temp to avoid corrupting the local's alloca
            ctx.set_temp(dest, value);
        } else {
            // No conflict - use set_local which stores to alloca if present
            ctx.set_local(dest.to_string(), value);
        }
    };

    // If it's a struct value, extract directly
    if let BasicValueEnum::StructValue(struct_val) = enum_val {
        let payload_ptr = ctx
            .builder
            .build_extract_value(
                struct_val,
                ENUM_PAYLOAD_INDEX,
                &format!("{}_payload_ptr", dest),
            )
            .ok()?;

        // Dereference the payload pointer to get the actual value
        let payload_value = dereference_payload(ctx, payload_ptr, payload_type_id);

        // Store result appropriately based on type conflict
        store_result(ctx, payload_value);
        return Some(payload_value);
    }

    // If it's a pointer, load the struct first then extract
    if enum_val.is_pointer_value() {
        let enum_ptr = enum_val.into_pointer_value();
        let loaded = ctx
            .builder
            .build_load(enum_type, enum_ptr, "enum_loaded")
            .ok()?;

        if let BasicValueEnum::StructValue(struct_val) = loaded {
            let payload_ptr = ctx
                .builder
                .build_extract_value(
                    struct_val,
                    ENUM_PAYLOAD_INDEX,
                    &format!("{}_payload_ptr", dest),
                )
                .ok()?;

            // Dereference the payload pointer to get the actual value
            let payload_value = dereference_payload(ctx, payload_ptr, payload_type_id);

            // Store result appropriately based on type conflict
            store_result(ctx, payload_value);
            return Some(payload_value);
        }
    }

    None
}

/// Emit EnumGetPayload instruction with tuple element extraction.
///
/// This handles multi-field enum variants like `Okkk(Int, Str)` where the payload
/// is stored as a tuple. The index parameter specifies which element to extract.
///
/// For single-field variants (index 0 with non-tuple payload), the entire payload is returned.
/// For multi-field variants (tuple payloads), the specific element at `index` is extracted.
fn emit_enum_get_payload<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    dest: &str,
    value: &MirOperand,
    enum_name: &str,
    variant_name: &str,
    index: u32,
) -> Option<BasicValueEnum<'ctx>> {
    let enum_val = operand_to_value(ctx, value)?;

    // Get payload type from type registry (single source of truth)
    let payload_type_id = ctx.get_enum_variant_payload_type(enum_name, variant_name);

    // Check if payload is a tuple type - if so, we need to extract the element at index
    let (is_tuple_payload, tuple_element_types) = if let Some(type_id) = payload_type_id {
        if let Some(type_kind) = ctx.get_type_kind(type_id) {
            if let doo_core::types::TypeKind::Tuple { elements } = type_kind {
                (true, Some(elements.clone()))
            } else {
                (false, None)
            }
        } else {
            (false, None)
        }
    } else {
        (false, None)
    };

    // Extract payload pointer from enum struct
    let extract_payload_ptr = |ctx: &mut CodegenContext<'ctx>,
                               enum_val: BasicValueEnum<'ctx>|
     -> Option<inkwell::values::PointerValue<'ctx>> {
        if let BasicValueEnum::StructValue(struct_val) = enum_val {
            ctx.builder
                .build_extract_value(
                    struct_val,
                    ENUM_PAYLOAD_INDEX,
                    &format!("{}_payload_ptr", dest),
                )
                .ok()
                .and_then(|v| {
                    if v.is_pointer_value() {
                        Some(v.into_pointer_value())
                    } else {
                        None
                    }
                })
        } else if enum_val.is_pointer_value() {
            // Load struct from pointer first
            let enum_ptr = enum_val.into_pointer_value();
            let loaded = ctx
                .builder
                .build_load(get_enum_type(ctx), enum_ptr, "enum_loaded")
                .ok()?;

            if let BasicValueEnum::StructValue(struct_val) = loaded {
                ctx.builder
                    .build_extract_value(
                        struct_val,
                        ENUM_PAYLOAD_INDEX,
                        &format!("{}_payload_ptr", dest),
                    )
                    .ok()
                    .and_then(|v| {
                        if v.is_pointer_value() {
                            Some(v.into_pointer_value())
                        } else {
                            None
                        }
                    })
            } else {
                None
            }
        } else {
            None
        }
    };

    let payload_ptr = extract_payload_ptr(ctx, enum_val)?;

    // If this is a tuple payload, extract the specific element at index
    if is_tuple_payload {
        if let Some(elem_types) = tuple_element_types {
            // CRITICAL: TupleCreate stores ALL elements as i64 (see composites.rs TupleCreate)
            // This is the memory layout used when the tuple was created, so we MUST match it here.
            // Using ctx.get_llvm_type() would give wrong types (e.g., ptr for Str) and cause crashes.
            let llvm_elem_types: Vec<inkwell::types::BasicTypeEnum> = elem_types
                .iter()
                .map(|_| ctx.i64_type().into()) // All elements stored as i64
                .collect();
            let tuple_struct_type = ctx.context.struct_type(&llvm_elem_types, false);

            // Get the element at the specified index using GEP
            if let Ok(elem_ptr) = ctx.builder.build_struct_gep(
                tuple_struct_type,
                payload_ptr,
                index,
                &format!("{}_elem_ptr", dest),
            ) {
                // Always load as i64 since that's how TupleCreate stores elements
                let i64_type = ctx.i64_type();
                if let Ok(elem_val) = ctx.builder.build_load(i64_type, elem_ptr, dest) {
                    // Check if this element is a pointer type (Str, Array, etc.)
                    // If so, we need to convert the i64 back to a pointer
                    let elem_type_id = elem_types.get(index as usize).copied();
                    let final_val = if let Some(tid) = elem_type_id {
                        if let Some(type_kind) = ctx.get_type_kind(tid) {
                            match type_kind {
                                // Pointer types - convert i64 to pointer
                                doo_core::types::TypeKind::Str
                                | doo_core::types::TypeKind::Array { .. }
                                | doo_core::types::TypeKind::Map { .. }
                                | doo_core::types::TypeKind::Struct { .. }
                                | doo_core::types::TypeKind::Enum { .. }
                                | doo_core::types::TypeKind::Tuple { .. } => {
                                    // IntToPtr to get the actual pointer
                                    ctx.builder
                                        .build_int_to_ptr(
                                            elem_val.into_int_value(),
                                            ctx.ptr_type(),
                                            &format!("{}_ptr", dest),
                                        )
                                        .ok()
                                        .map(|p| p.into())
                                        .unwrap_or(elem_val)
                                }
                                // Value types - i64 is already correct
                                _ => elem_val,
                            }
                        } else {
                            elem_val
                        }
                    } else {
                        elem_val
                    };

                    ctx.set_temp(dest, final_val);
                    return Some(final_val);
                }
            }
        }
        // Fallback: return payload pointer if tuple extraction fails
        ctx.set_temp(dest, payload_ptr.into());
        return Some(payload_ptr.into());
    }

    // Non-tuple payload: dereference based on type
    let final_value = if let Some(type_id) = payload_type_id {
        if let Some(type_kind) = ctx.get_type_kind(type_id) {
            match type_kind {
                // Pointer types - the payload IS the pointer
                doo_core::types::TypeKind::Str
                | doo_core::types::TypeKind::Array { .. }
                | doo_core::types::TypeKind::Map { .. }
                | doo_core::types::TypeKind::Struct { .. }
                | doo_core::types::TypeKind::Enum { .. } => payload_ptr.into(),
                // Value types - load from the heap-allocated payload
                doo_core::types::TypeKind::Int
                | doo_core::types::TypeKind::Float32
                | doo_core::types::TypeKind::Float64
                | doo_core::types::TypeKind::Bool => {
                    let llvm_type = ctx.get_llvm_type(type_id);
                    ctx.builder
                        .build_load(llvm_type, payload_ptr, &format!("{}_value", dest))
                        .ok()
                        .map(|v| v.into())
                        .unwrap_or(payload_ptr.into())
                }
                _ => {
                    // Check LLVM type
                    let llvm_type = ctx.get_llvm_type(type_id);
                    if llvm_type.is_pointer_type() {
                        payload_ptr.into()
                    } else {
                        ctx.builder
                            .build_load(llvm_type, payload_ptr, &format!("{}_value", dest))
                            .ok()
                            .map(|v| v.into())
                            .unwrap_or(payload_ptr.into())
                    }
                }
            }
        } else {
            payload_ptr.into()
        }
    } else {
        payload_ptr.into()
    };

    ctx.set_temp(dest, final_value);
    Some(final_value)
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Convert MirOperand to LLVM value.
fn operand_to_value<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    operand: &MirOperand,
) -> Option<BasicValueEnum<'ctx>> {
    match operand {
        MirOperand::Const(c) => Some(const_to_value(ctx, c)),
        MirOperand::Local(name) | MirOperand::Temp(name) | MirOperand::Global(name) => {
            let name_str = resolve(*name);
            // First try to get as a value (local variable, temp, etc.)
            if let Some(val) = ctx.get_value(&name_str) {
                return Some(val);
            }
            // Fall back to function reference - convert function to pointer value
            if let Some(func) = ctx.get_function(&name_str) {
                return Some(func.as_global_value().as_pointer_value().into());
            }
            None
        }
        MirOperand::FuncRef(name) => {
            let name_str = resolve(*name);
            // FuncRef is an explicit function reference - convert to pointer
            if let Some(func) = ctx.get_function(&name_str) {
                return Some(func.as_global_value().as_pointer_value().into());
            }
            None
        }
    }
}

/// Convert MirConst to LLVM value.
fn const_to_value<'ctx>(ctx: &CodegenContext<'ctx>, c: &MirConst) -> BasicValueEnum<'ctx> {
    match c {
        MirConst::Int(v) => ctx.const_i64(*v).into(),
        MirConst::Float(v) => ctx.const_f64(*v).into(),
        MirConst::Bool(v) => ctx.const_bool(*v).into(),
        MirConst::Nil => ctx.const_i64(0).into(),
        MirConst::Str(s) => ctx.const_string(s).into(),
    }
}
