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
            } => emit_enum_create(ctx, dest, enum_name, variant, payload.as_ref()),

            // ==================================================================
            // EnumTag - Extract tag from enum (simple version)
            // ==================================================================
            MirInstrKind::EnumTag { dest, value } => emit_enum_tag(ctx, dest, value),

            // ==================================================================
            // EnumGetTag - Extract tag from enum with type info
            // ==================================================================
            MirInstrKind::EnumGetTag {
                dest,
                value,
                enum_name: _,
            } => {
                // Same implementation as EnumTag - enum_name available for future optimizations
                emit_enum_tag(ctx, dest, value)
            }

            // ==================================================================
            // EnumTagEquals - Compare tag with expected variant index
            // ==================================================================
            MirInstrKind::EnumTagEquals {
                dest,
                tag,
                variant_name,
                enum_name,
            } => emit_enum_tag_equals(ctx, dest, tag, variant_name, enum_name),

            // ==================================================================
            // EnumPayload - Extract payload (simple version)
            // ==================================================================
            MirInstrKind::EnumPayload {
                dest,
                value,
                variant: _,
            } => {
                // Extract payload pointer - variant name available for type lookup
                emit_enum_payload(ctx, dest, value)
            }

            // ==================================================================
            // EnumGetPayload - Extract payload with type info
            // ==================================================================
            MirInstrKind::EnumGetPayload {
                dest,
                value,
                variant_name: _,
                enum_name: _,
                index: _,
            } => {
                // Extract payload pointer - type info available for future optimizations
                emit_enum_payload(ctx, dest, value)
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
    let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
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
    let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());

    // Allocate enum struct
    let enum_alloca = ctx
        .builder
        .build_alloca(enum_type, &format!("{}_enum", dest))
        .ok()?;

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
            // Box the payload value - allocate and store
            let payload_ptr = if payload_val.is_pointer_value() {
                // Already a pointer - use directly
                payload_val.into_pointer_value()
            } else {
                // Value type - allocate and store
                let alloca = ctx
                    .builder
                    .build_alloca(payload_val.get_type(), "payload_alloca")
                    .ok()?;
                ctx.builder.build_store(alloca, payload_val).ok()?;
                alloca
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
/// Extracts the payload pointer from an enum value.
/// The caller is responsible for knowing the payload type and loading appropriately.
fn emit_enum_payload<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    dest: &str,
    value: &MirOperand,
) -> Option<BasicValueEnum<'ctx>> {
    let enum_val = operand_to_value(ctx, value)?;
    let enum_type = get_enum_type(ctx);

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

        ctx.set_temp(dest, payload_ptr);
        return Some(payload_ptr);
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

            ctx.set_temp(dest, payload_ptr);
            return Some(payload_ptr);
        }
    }

    None
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
            ctx.get_value(name)
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
