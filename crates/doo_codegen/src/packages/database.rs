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
use crate::instructions::calls::call_ffi;
use doo_core::doo_debug;
use doo_mir::MirOperand;

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
    if let Some(converted) = call_ffi::try_convert_enum_to_json_string(ctx, operand) {
        return Some(converted.into());
    }

    // Try enum array → JSON array string conversion
    if let Some(converted) = call_ffi::try_convert_enum_array_to_json_string(ctx, operand) {
        return Some(converted.into());
    }

    None
}
