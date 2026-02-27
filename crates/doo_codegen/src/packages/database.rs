//! Database package codegen hooks.
//!
//! Database-specific codegen behavior:
//! - Enum → JSON string conversion for `doo_db_raw_param` arg[2]
//! - Enum array → JSON array string conversion
//!
//! When `DB.rawWithParams()` receives an enum or enum array as a parameter value,
//! the compiler auto-converts it to a JSON string before passing to the FFI.
//! Third-party DB packages would handle this in their FFI Rust code instead.

use crate::context::CodegenContext;
use crate::instructions::calls::call_utils::operand_to_value;
use doo_core::constants::ffi_names;
use doo_core::doo_debug;
use doo_mir::sym::resolve;
use doo_mir::MirOperand;
use inkwell::values::{BasicValueEnum, PointerValue};

// ============================================================================
// Database FFI Symbol Constants (Package-Owned)
// ============================================================================

pub(crate) const DOO_DB_RAW_PARAM: &str = "doo_db_raw_param";
pub(crate) const DOO_DB_SERIALIZE_ENUM_ARRAY: &str = "doo_db_serialize_enum_array";

/// Check if a DB FFI argument needs package-specific conversion.
///
/// For `doo_db_raw_param` arg[2]: converts enum values and enum arrays
/// to JSON string representation before passing to the FFI.
///
/// Returns `Some(converted_value)` if conversion was applied, `None` otherwise.
pub(crate) fn convert_arg<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    symbol: &str,
    arg_index: usize,
    operand: &MirOperand,
) -> Option<inkwell::values::BasicMetadataValueEnum<'ctx>> {
    // Only applies to doo_db_raw_param arg[2] (the parameter value)
    if symbol != DOO_DB_RAW_PARAM || arg_index != 2 {
        return None;
    }

    let debug = std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok();

    // Check for empty array literal — pass "[]" directly
    if let MirOperand::Temp(name) = operand {
        let name_str = doo_mir::sym::resolve(*name);
        let has_elem_type = ctx.array_element_types.contains_key(&name_str);
        let has_elem_temps = ctx.array_element_temps.contains_key(&name_str);

        if debug {
            doo_debug!(
                "CODEGEN",
                "doo_db_raw_param arg[2]: temp={}, has_elem_type={}, has_elem_temps={}",
                name_str,
                has_elem_type,
                has_elem_temps
            );
        }

        // Empty array: tracked as array but has no element temps
        if has_elem_type && !has_elem_temps {
            if debug {
                doo_debug!(
                    "CODEGEN",
                    "Converting empty array {} to JSON \"[]\"",
                    name_str
                );
            }
            let empty_json = ctx.const_string("[]");
            return Some(empty_json.into());
        }
    } else if debug {
        doo_debug!(
            "CODEGEN",
            "doo_db_raw_param arg[2] is not a Temp: {:?}",
            operand
        );
    }

    // Try single enum → JSON string conversion
    if let Some(converted) = try_convert_enum_to_json_string(ctx, operand) {
        return Some(converted.into());
    }

    // Try enum array → JSON array string conversion
    if let Some(converted) = try_convert_enum_array_to_json_string(ctx, operand) {
        return Some(converted.into());
    }

    None
}

// ============================================================================
// Enum → JSON String Conversion (Database-specific)
// ============================================================================

/// Try to convert an enum operand to a JSON string for doo_db_raw_param.
/// Returns Some(pointer_value) if the operand is a known enum, None otherwise.
fn try_convert_enum_to_json_string<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    operand: &MirOperand,
) -> Option<PointerValue<'ctx>> {
    // Get the temp/local name to look up enum type
    let var_name = match operand {
        MirOperand::Temp(name) | MirOperand::Local(name) => resolve(*name),
        _ => return None,
    };

    // Check if this temp/local is a known enum type
    let enum_name = ctx.temp_struct_types.get(&var_name)?.clone();

    // Look up enum type in registry to get variants
    let type_id = ctx.type_registry.lookup(&enum_name)?;
    let type_info = ctx.type_registry.get(type_id)?;

    let variants: Vec<(String, u32)> = match &type_info.kind {
        doo_core::types::TypeKind::Enum { variants, .. } => variants
            .iter()
            .enumerate()
            .map(|(i, (name, _))| (name.clone(), i as u32))
            .collect(),
        _ => return None,
    };

    // Get the enum value
    let enum_val = operand_to_value(ctx, operand)?;
    let struct_val = if enum_val.is_struct_value() {
        enum_val.into_struct_value()
    } else {
        return None;
    };

    // Extract tag from enum struct
    let tag = ctx
        .builder
        .build_extract_value(struct_val, 0, "enum_tag_for_json")
        .ok()?
        .into_int_value();

    // Generate switch-case to convert tag -> JSON string
    let current_block = ctx.builder.get_insert_block()?;
    let target_fn = current_block.get_parent()?;
    let merge_block = ctx.context.append_basic_block(target_fn, "enum_json_merge");
    let default_block = ctx
        .context
        .append_basic_block(target_fn, "enum_json_default");

    // Build default block with unknown string
    ctx.builder.position_at_end(default_block);
    let unknown_str = ctx.const_string("[\"Unknown\"]");
    ctx.builder.build_unconditional_branch(merge_block).ok();

    // Build case blocks for each variant
    let ptr_type = ctx.ptr_type();
    let mut incoming_vals: Vec<(BasicValueEnum<'ctx>, inkwell::basic_block::BasicBlock<'ctx>)> =
        Vec::new();
    let mut cases: Vec<(
        inkwell::values::IntValue<'ctx>,
        inkwell::basic_block::BasicBlock<'ctx>,
    )> = Vec::new();

    incoming_vals.push((unknown_str.into(), default_block));

    for (variant_name, variant_idx) in &variants {
        let case_block = ctx
            .context
            .append_basic_block(target_fn, &format!("enum_case_{}", variant_name));
        ctx.builder.position_at_end(case_block);

        // Create JSON array string: ["VariantName"]
        let json_str = format!("[\"{}\"]", variant_name);
        let str_ptr = ctx.const_string(&json_str);
        ctx.builder.build_unconditional_branch(merge_block).ok();

        cases.push((
            ctx.context.i32_type().const_int(*variant_idx as u64, false),
            case_block,
        ));
        incoming_vals.push((str_ptr.into(), case_block));
    }

    // Build switch in original block
    ctx.builder.position_at_end(current_block);
    ctx.builder.build_switch(tag, default_block, &cases).ok();

    // Build phi in merge block
    ctx.builder.position_at_end(merge_block);
    let phi = ctx.builder.build_phi(ptr_type, "enum_json_str").ok()?;

    let incoming_refs: Vec<(
        &dyn inkwell::values::BasicValue<'ctx>,
        inkwell::basic_block::BasicBlock<'ctx>,
    )> = incoming_vals
        .iter()
        .map(|(v, b)| (v as &dyn inkwell::values::BasicValue<'ctx>, *b))
        .collect();
    phi.add_incoming(&incoming_refs);

    Some(phi.as_basic_value().into_pointer_value())
}

/// Try to convert an array of enums to a JSON string for doo_db_raw_param.
/// Returns Some(pointer_value) if the operand is an array of enums, None otherwise.
/// Handles both homogeneous enum arrays (all same type) and mixed enum arrays.
/// Also handles EMPTY arrays by returning "[]".
fn try_convert_enum_array_to_json_string<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    operand: &MirOperand,
) -> Option<PointerValue<'ctx>> {
    // Get the temp/local name to look up array element type
    let var_name = match operand {
        MirOperand::Temp(name) | MirOperand::Local(name) => resolve(*name),
        _ => return None,
    };

    // Check if this temp/local is a known array with element type
    let elem_type_id = ctx.array_element_types.get(&var_name)?.clone();

    // IMPORTANT: Check for EMPTY arrays FIRST before generating any LLVM code
    // Empty arrays are tracked in array_element_types but NOT in array_element_temps
    // We must handle them explicitly to return "[]" JSON string
    let has_element_temps = ctx.array_element_temps.contains_key(&var_name);
    if !has_element_temps {
        // This is an empty array - return "[]" directly
        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
            doo_debug!(
                "CODEGEN",
                "try_convert_enum_array_to_json_string: empty array {} -> \"[]\"",
                var_name
            );
        }
        return Some(ctx.const_string("[]"));
    }

    // Look up element type in registry to check if it's an enum
    let type_info = ctx.type_registry.get(elem_type_id);

    // Try homogeneous enum array first
    if let Some(info) = &type_info {
        if let doo_core::types::TypeKind::Enum { name, variants, .. } = &info.kind {
            let variant_names: Vec<String> =
                variants.iter().map(|(vname, _)| vname.clone()).collect();

            if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                doo_debug!(
                    "CODEGEN",
                    "Converting homogeneous enum array {} with variants: {:?}",
                    name,
                    variant_names
                );
            }

            // Get the array pointer
            let array_val = operand_to_value(ctx, operand)?;
            let array_ptr = if array_val.is_pointer_value() {
                array_val.into_pointer_value()
            } else {
                return None;
            };

            // Create variant names string (comma-separated)
            let variants_str = variant_names.join(",");
            let variants_ptr = ctx.const_string(&variants_str);

            // Enum stride is 16 bytes: { i32 tag, ptr payload } = 4 + 8 = 12, padded to 16
            let stride = ctx.i32_type().const_int(16, false);

            // Declare doo_db_serialize_enum_array if not already declared
            let serialize_fn = ctx
                .module
                .get_function(DOO_DB_SERIALIZE_ENUM_ARRAY)
                .unwrap_or_else(|| {
                    let ptr_type = ctx.ptr_type();
                    let i32_type = ctx.i32_type();
                    let fn_type = ptr_type
                        .fn_type(&[ptr_type.into(), ptr_type.into(), i32_type.into()], false);
                    ctx.module
                        .add_function(DOO_DB_SERIALIZE_ENUM_ARRAY, fn_type, None)
                });

            // Call doo_db_serialize_enum_array(array_ptr, variants, stride)
            let result = ctx
                .builder
                .build_call(
                    serialize_fn,
                    &[array_ptr.into(), variants_ptr.into(), stride.into()],
                    "enum_array_json",
                )
                .ok()?
                .try_as_basic_value()
                .basic()?;

            return Some(result.into_pointer_value());
        }
    }

    // Fallback: try mixed enum array via element temps
    try_convert_mixed_enum_array_to_json_string(ctx, &var_name)
}

/// Convert a mixed-type enum array to JSON string by checking individual element temps.
fn try_convert_mixed_enum_array_to_json_string<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    array_var_name: &str,
) -> Option<PointerValue<'ctx>> {
    // Get element temps for this array
    let element_temps = ctx.array_element_temps.get(array_var_name)?.clone();

    if element_temps.is_empty() {
        return None;
    }

    // Collect enum info for each element
    let mut enum_infos: Vec<(String, Vec<(String, u32)>)> = Vec::new();
    for temp_name in &element_temps {
        if let Some(enum_name) = ctx.temp_struct_types.get(temp_name) {
            let type_id = ctx.type_registry.lookup(enum_name)?;
            let type_info = ctx.type_registry.get(type_id)?;

            if let doo_core::types::TypeKind::Enum { variants, .. } = &type_info.kind {
                let variant_list: Vec<(String, u32)> = variants
                    .iter()
                    .enumerate()
                    .map(|(i, (name, _))| (name.clone(), i as u32))
                    .collect();
                enum_infos.push((enum_name.clone(), variant_list));
            } else {
                return None; // Not an enum element
            }
        } else {
            return None; // Element type not tracked
        }
    }

    if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
        doo_debug!(
            "CODEGEN",
            "Converting mixed enum array with {} elements",
            enum_infos.len()
        );
    }

    // Generate code to build JSON array string at runtime
    // We'll create: ["variant1", "variant2", ...]

    let ptr_type = ctx.ptr_type();
    let i64_type = ctx.i64_type();
    let i8_type = ctx.context.i8_type();

    // Allocate buffer for JSON string (generous size)
    let buffer_size = i64_type.const_int(256, false);
    let buffer = ctx
        .builder
        .build_array_alloca(i8_type, buffer_size, "mixed_json_buf")
        .ok()?;

    // Get sprintf
    let sprintf = ctx
        .module
        .get_function(ffi_names::SPRINTF)
        .unwrap_or_else(|| {
            let i32_type = ctx.i32_type();
            let fn_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], true);
            ctx.module.add_function(ffi_names::SPRINTF, fn_type, None)
        });

    // Get strlen
    let strlen = ctx
        .module
        .get_function(ffi_names::STRLEN)
        .unwrap_or_else(|| {
            let fn_type = i64_type.fn_type(&[ptr_type.into()], false);
            ctx.module.add_function(ffi_names::STRLEN, fn_type, None)
        });

    // Start with "["
    let open_bracket = ctx.const_string("[");
    ctx.builder
        .build_call(sprintf, &[buffer.into(), open_bracket.into()], "")
        .ok();

    // For each element, generate switch-case to append "variant"
    for (elem_idx, (temp_name, (_enum_name, variants))) in
        element_temps.iter().zip(enum_infos.iter()).enumerate()
    {
        // Get the current buffer position
        let current_len = ctx
            .builder
            .build_call(strlen, &[buffer.into()], "cur_len")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();
        let write_pos = unsafe {
            ctx.builder
                .build_gep(i8_type, buffer, &[current_len], "write_pos")
        }
        .ok()?;

        // Add comma if not first element
        if elem_idx > 0 {
            let comma_fmt = ctx.const_string(",");
            ctx.builder
                .build_call(sprintf, &[write_pos.into(), comma_fmt.into()], "")
                .ok();

            // Update position
            let current_len = ctx
                .builder
                .build_call(strlen, &[buffer.into()], "cur_len2")
                .ok()?
                .try_as_basic_value()
                .basic()?
                .into_int_value();
            let _write_pos = unsafe {
                ctx.builder
                    .build_gep(i8_type, buffer, &[current_len], "write_pos2")
            }
            .ok()?;
        }

        // Get the enum value from temps
        let enum_val = ctx.get_temp(temp_name)?;
        let struct_val = if enum_val.is_struct_value() {
            enum_val.into_struct_value()
        } else {
            continue;
        };

        // Extract tag
        let tag = ctx
            .builder
            .build_extract_value(struct_val, 0, "mixed_tag")
            .ok()?
            .into_int_value();

        // Get current position for writing
        let current_len = ctx
            .builder
            .build_call(strlen, &[buffer.into()], "cur_len3")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();
        let write_pos = unsafe {
            ctx.builder
                .build_gep(i8_type, buffer, &[current_len], "write_pos3")
        }
        .ok()?;

        // Generate switch for this element's variants
        let current_block = ctx.builder.get_insert_block()?;
        let target_fn = current_block.get_parent()?;
        let merge_block = ctx
            .context
            .append_basic_block(target_fn, &format!("mixed_merge_{}", elem_idx));
        let default_block = ctx
            .context
            .append_basic_block(target_fn, &format!("mixed_default_{}", elem_idx));

        // Default: write "Unknown"
        ctx.builder.position_at_end(default_block);
        let unknown_fmt = ctx.const_string("\"Unknown\"");
        ctx.builder
            .build_call(sprintf, &[write_pos.into(), unknown_fmt.into()], "")
            .ok();
        ctx.builder.build_unconditional_branch(merge_block).ok();

        // Cases for each variant
        let mut cases = Vec::new();
        for (variant_name, variant_idx) in variants {
            let case_block = ctx.context.append_basic_block(
                target_fn,
                &format!("mixed_case_{}_{}", elem_idx, variant_name),
            );
            ctx.builder.position_at_end(case_block);

            let variant_fmt = ctx.const_string(&format!("\"{}\"", variant_name));
            ctx.builder
                .build_call(sprintf, &[write_pos.into(), variant_fmt.into()], "")
                .ok();
            ctx.builder.build_unconditional_branch(merge_block).ok();

            cases.push((
                ctx.context.i32_type().const_int(*variant_idx as u64, false),
                case_block,
            ));
        }

        // Build switch
        ctx.builder.position_at_end(current_block);
        ctx.builder.build_switch(tag, default_block, &cases).ok();

        // Continue from merge block
        ctx.builder.position_at_end(merge_block);
    }

    // Append "]"
    let current_len = ctx
        .builder
        .build_call(strlen, &[buffer.into()], "final_len")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_int_value();
    let write_pos = unsafe {
        ctx.builder
            .build_gep(i8_type, buffer, &[current_len], "final_pos")
    }
    .ok()?;
    let close_bracket = ctx.const_string("]");
    ctx.builder
        .build_call(sprintf, &[write_pos.into(), close_bracket.into()], "")
        .ok();

    Some(buffer)
}
