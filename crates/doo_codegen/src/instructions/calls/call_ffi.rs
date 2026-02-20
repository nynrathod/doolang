//! FFI call implementation — signatures, declarations, and call emission.
//!
//! This module is **completely package-agnostic**. All package-specific behavior
//! (HTTP handler wrappers, WS event wrappers, DB enum conversion, middleware)
//! is handled by the `packages/` dispatch system.
//!
//! Adding a new FFI package requires ZERO changes here.

use super::call_utils::operand_to_value;
use crate::context::CodegenContext;
use doo_core::constants::ffi_names;
use doo_core::doo_debug;
use doo_core::types::{builtin, TypeId};
use doo_mir::sym::resolve;
use doo_mir::MirOperand;
use inkwell::module::Linkage;
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};
use inkwell::AddressSpace;
// ============================================================================
// FFI Call Implementation
// ============================================================================

/// FFI function signature: (param_types, return_type, is_variadic)
/// - param_types: slice of ("ptr" | "i64" | "i32" | "f64" | "void")
/// - return_type: "ptr" | "i64" | "i32" | "f64" | "void"
/// - is_variadic: whether function accepts variable arguments
type FfiSignature = (&'static [&'static str], &'static str, bool);

/// Get FFI function signature for C stdlib and runtime intrinsic functions.
///
/// **Package-Ready Design**: This table ONLY contains signatures for functions
/// that have NO Doo `@extern` declaration (C stdlib, Doo runtime allocator).
/// All Doo-declared FFI functions (http, db, auth, json, file, ws, process,
/// config, and any third-party packages) get their signatures from the
/// type signature registry populated from MIR FfiLinkage.
///
/// A third-party package author NEVER needs to add entries here.
fn get_ffi_signature(symbol: &str) -> Option<FfiSignature> {
    match symbol {
        // =====================================================================
        // C Standard Library — no Doo declarations, need hardcoded signatures
        // =====================================================================
        ffi_names::MALLOC => Some((&["i64"], "ptr", false)),
        ffi_names::FREE => Some((&["ptr"], "void", false)),
        ffi_names::REALLOC => Some((&["ptr", "i64"], "ptr", false)),
        ffi_names::STRLEN => Some((&["ptr"], "i64", false)),
        ffi_names::STRCMP => Some((&["ptr", "ptr"], "i32", false)),
        ffi_names::STRCPY => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::STRCAT => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::MEMCPY => Some((&["ptr", "ptr", "i64"], "ptr", false)),
        ffi_names::MEMSET => Some((&["ptr", "i32", "i64"], "ptr", false)),
        ffi_names::PRINTF => Some((&["ptr"], "i32", true)), // variadic
        ffi_names::SNPRINTF => Some((&["ptr", "i64", "ptr"], "i32", true)),
        ffi_names::PUTS => Some((&["ptr"], "i32", false)),
        ffi_names::PUTCHAR => Some((&["i32"], "i32", false)),

        // =====================================================================
        // Doo Runtime Allocator — compiler-internal, no @extern declarations
        // =====================================================================
        ffi_names::DOO_ALLOC => Some((&["i64"], "ptr", false)),
        ffi_names::DOO_FREE => Some((&["ptr"], "void", false)),
        ffi_names::DOO_REALLOC => Some((&["ptr", "i64"], "ptr", false)),

        // =====================================================================
        // Math intrinsics — linked from libm, no @extern declarations
        // =====================================================================
        ffi_names::FABS => Some((&["f64"], "f64", false)),
        ffi_names::FLOOR => Some((&["f64"], "f64", false)),
        ffi_names::CEIL => Some((&["f64"], "f64", false)),
        ffi_names::ROUND => Some((&["f64"], "f64", false)),
        ffi_names::SQRT => Some((&["f64"], "f64", false)),

        // =====================================================================
        // All other FFI functions (doo_http_*, doo_db_*, doo_auth_*, doo_json_*,
        // doo_file_*, doo_ws_*, doo_process_*, doo_config_*, and any third-party
        // packages) are resolved via the type signature registry from their
        // @extern Doo declarations. No entries needed here.
        // =====================================================================
        _ => None,
    }
}

/// Convert FFI type string to LLVM type.
fn ffi_type_to_llvm<'ctx>(
    ctx: &CodegenContext<'ctx>,
    type_str: &str,
) -> Option<BasicTypeEnum<'ctx>> {
    match type_str {
        "ptr" => Some(ctx.context.ptr_type(AddressSpace::default()).into()),
        "i64" => Some(ctx.i64_type().into()),
        "i32" => Some(ctx.i32_type().into()),
        "f64" => Some(ctx.f64_type().into()),
        "void" => None, // void is not a BasicType
        // SimpleResult: { i64 tag, i64 value } - returned by value for Result types
        // Using i64 for both fields ensures proper Windows x64 ABI compatibility.
        // On Windows x64, a struct of exactly 2x i64 (16 bytes) is returned via RAX:RDX registers.
        // This avoids sret (hidden pointer) issues that occur with { i32, ptr } layouts.
        "simple_result" => {
            let struct_ty = ctx
                .context
                .struct_type(&[ctx.i64_type().into(), ctx.i64_type().into()], false);
            Some(struct_ty.into())
        }
        _ => Some(ctx.i64_type().into()), // default to i64
    }
}

/// Convert a Doo TypeId to the corresponding FFI type string.
/// This maps Doo's type system to the C ABI types used at the FFI boundary.
fn type_id_to_ffi_str(type_id: TypeId) -> &'static str {
    if type_id == builtin::INT {
        "i64"
    } else if type_id == builtin::FLOAT {
        "f64"
    } else if type_id == builtin::BOOL {
        "i32" // C ABI uses i32 for bools
    } else if type_id == builtin::VOID {
        "void"
    } else {
        // All other types (Str, structs, arrays, maps, enums, Any, etc.)
        // are passed as pointers at the FFI boundary
        "ptr"
    }
}

/// Declare an FFI function with proper signature and external linkage.
///
/// Resolution order (package-ready):
/// 1. **Type signature registry** — populated from MIR FfiLinkage (Doo declarations).
///    This is the primary path and handles ALL @extern functions including third-party packages.
/// 2. **Hardcoded table** — `get_ffi_signature()` for C stdlib/runtime functions
///    (malloc, free, printf, etc.) that have no Doo `@extern` declaration.
/// 3. **Fallback** — all-pointer inference (ptr params, ptr return) for unknown symbols.
pub(crate) fn declare_ffi_function<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    symbol: &str,
    arg_count: usize,
) -> FunctionValue<'ctx> {
    // Check if already declared
    if let Some(func) = ctx.get_function(symbol) {
        return func;
    }

    let ptr_ty = ctx.context.ptr_type(AddressSpace::default());

    // === Priority 1: Type signature registry (from Doo @extern declarations) ===
    // This is the package-ready path — works for ALL FFI functions with Doo declarations,
    // including third-party packages. No hardcoded table entry needed.
    if let Some((param_type_ids, return_type_id, is_result)) =
        ctx.ffi_type_signatures.get(symbol).cloned()
    {
        let params: Vec<BasicTypeEnum> = param_type_ids
            .iter()
            .filter_map(|tid| ffi_type_to_llvm(ctx, type_id_to_ffi_str(*tid)))
            .collect();

        let ret = if is_result {
            // Result-returning FFI functions return *mut SimpleResult (pointer)
            Some(ptr_ty.into())
        } else {
            match return_type_id {
                Some(tid) => ffi_type_to_llvm(ctx, type_id_to_ffi_str(tid)),
                None => None, // void
            }
        };

        let param_meta: Vec<inkwell::types::BasicMetadataTypeEnum> =
            params.iter().map(|t| (*t).into()).collect();
        let fn_type = match ret {
            Some(r) => r.fn_type(&param_meta, false),
            None => ctx.context.void_type().fn_type(&param_meta, false),
        };

        let func = ctx
            .module
            .add_function(symbol, fn_type, Some(Linkage::External));
        return func;
    }

    // === Priority 2: Hardcoded table (C stdlib / Doo runtime intrinsics only) ===
    let (param_types_vec, return_type, is_variadic) =
        if let Some((param_strs, ret_str, variadic)) = get_ffi_signature(symbol) {
            let params: Vec<BasicTypeEnum> = param_strs
                .iter()
                .filter_map(|s| ffi_type_to_llvm(ctx, s))
                .collect();
            let ret = ffi_type_to_llvm(ctx, ret_str);
            (params, ret, variadic)
        } else {
            // === Priority 3: Fallback — all-pointer inference ===
            let params: Vec<BasicTypeEnum> = (0..arg_count).map(|_| ptr_ty.into()).collect();
            (params, Some(ptr_ty.into()), false)
        };

    // Build function type
    let param_meta: Vec<inkwell::types::BasicMetadataTypeEnum> =
        param_types_vec.iter().map(|t| (*t).into()).collect();

    let fn_type = match return_type {
        Some(ret) => ret.fn_type(&param_meta, is_variadic),
        None => ctx.context.void_type().fn_type(&param_meta, is_variadic),
    };

    // Declare with external linkage for FFI
    let func = ctx
        .module
        .add_function(symbol, fn_type, Some(Linkage::External));
    func
}

/// Convert a Doo value to FFI-compatible value if needed.
fn convert_to_ffi_arg<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
    expected_type: Option<&str>,
) -> inkwell::values::BasicMetadataValueEnum<'ctx> {
    match expected_type {
        Some("i32") => {
            // Convert i64 to i32 if needed
            if val.is_int_value() {
                let int_val = val.into_int_value();
                if int_val.get_type().get_bit_width() == 64 {
                    let truncated = ctx
                        .builder
                        .build_int_truncate(int_val, ctx.i32_type(), "i64_to_i32")
                        .unwrap();
                    return truncated.into();
                }
            }
            val.into()
        }
        Some("f64") => {
            // Ensure float type
            if val.is_int_value() {
                let int_val = val.into_int_value();
                let float_val = ctx
                    .builder
                    .build_signed_int_to_float(int_val, ctx.f64_type(), "int_to_f64")
                    .unwrap();
                return float_val.into();
            }
            val.into()
        }
        Some("ptr") => {
            // Convert non-pointer types to string pointers
            if val.is_int_value() {
                let int_val = val.into_int_value();
                let ptr_type = ctx.ptr_type();

                // Check if this is a null/nil value (i64 0 from MirConst::Nil)
                // In that case, pass a null pointer directly instead of stringifying
                if let Some(const_val) = int_val.get_zero_extended_constant() {
                    if const_val == 0 {
                        return ptr_type.const_null().into();
                    }
                }

                // Convert i64 to string using sprintf
                // Allocate buffer for max int64 string: "-9223372036854775808" (20 chars + null)
                let i8_type = ctx.context.i8_type();
                let i64_type = ctx.i64_type();

                // Allocate 24 bytes for safety
                let buffer = ctx
                    .builder
                    .build_array_alloca(i8_type, i64_type.const_int(24, false), "int_to_str_buf")
                    .unwrap();

                // Get or declare sprintf
                let sprintf = ctx
                    .module
                    .get_function(ffi_names::SPRINTF)
                    .unwrap_or_else(|| {
                        let i32_type = ctx.i32_type();
                        let fn_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], true);
                        ctx.module.add_function(ffi_names::SPRINTF, fn_type, None)
                    });

                // Format string: "%lld"
                let fmt = ctx.const_string("%lld");

                // Call sprintf(buffer, "%lld", value)
                let int_val = val.into_int_value();
                ctx.builder
                    .build_call(
                        sprintf,
                        &[buffer.into(), fmt.into(), int_val.into()],
                        "sprintf_int",
                    )
                    .ok();

                return buffer.into();
            } else if val.is_float_value() {
                // Convert f64 to string using sprintf
                let i8_type = ctx.context.i8_type();
                let i64_type = ctx.i64_type();
                let ptr_type = ctx.ptr_type();

                // Allocate 32 bytes for float string
                let buffer = ctx
                    .builder
                    .build_array_alloca(i8_type, i64_type.const_int(32, false), "float_to_str_buf")
                    .unwrap();

                // Get or declare sprintf
                let sprintf = ctx
                    .module
                    .get_function(ffi_names::SPRINTF)
                    .unwrap_or_else(|| {
                        let i32_type = ctx.i32_type();
                        let fn_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], true);
                        ctx.module.add_function(ffi_names::SPRINTF, fn_type, None)
                    });

                // Format string: "%g" (compact float format)
                let fmt = ctx.const_string("%g");

                // Call sprintf(buffer, "%g", value)
                let float_val = val.into_float_value();
                ctx.builder
                    .build_call(
                        sprintf,
                        &[buffer.into(), fmt.into(), float_val.into()],
                        "sprintf_float",
                    )
                    .ok();

                return buffer.into();
            } else if val.is_struct_value() {
                // Struct value (e.g., enum { i32, ptr }) needs to be boxed to pointer
                // This handles single enum values passed to FFI functions like doo_db_raw_param
                let struct_val = val.into_struct_value();
                let alloca = ctx
                    .builder
                    .build_alloca(struct_val.get_type(), "enum_box")
                    .unwrap();
                ctx.builder.build_store(alloca, struct_val).ok();
                return alloca.into();
            }
            // Already a pointer - pass through
            val.into()
        }
        _ => val.into(),
    }
}

/// Try to convert an enum operand to a JSON string for doo_db_raw_param.
/// Returns Some(pointer_value) if the operand is a known enum, None otherwise.
pub(crate) fn try_convert_enum_to_json_string<'ctx>(
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
pub(crate) fn try_convert_enum_array_to_json_string<'ctx>(
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
                .get_function(ffi_names::DOO_DB_SERIALIZE_ENUM_ARRAY)
                .unwrap_or_else(|| {
                    let ptr_type = ctx.ptr_type();
                    let i32_type = ctx.i32_type();
                    let fn_type = ptr_type
                        .fn_type(&[ptr_type.into(), ptr_type.into(), i32_type.into()], false);
                    ctx.module
                        .add_function(ffi_names::DOO_DB_SERIALIZE_ENUM_ARRAY, fn_type, None)
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
    for (elem_idx, (temp_name, (enum_name, variants))) in
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
            let write_pos = unsafe {
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

/// Emit an FFI call with proper type handling.
///
/// This function is **completely package-agnostic**. All package-specific behavior
/// is delegated to `crate::packages` dispatch:
/// - **Pre-call hooks**: metadata registration, middleware setup (HTTP), etc.
/// - **FuncRef wrapping**: HTTP handler wrappers, WS event wrappers, etc.
/// - **Arg conversion**: DB enum→JSON, etc.
///
/// Adding a new FFI package requires ZERO changes here.
pub(crate) fn emit_ffi_call<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    dest: Option<&str>,
    symbol: &str,
    args: &[MirOperand],
) -> Option<BasicValueEnum<'ctx>> {
    if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
        doo_debug!(
            "CODEGEN",
            "FfiCall: {} with {} args -> {:?}",
            symbol,
            args.len(),
            dest
        );
    }

    // Declare FFI function if not already declared
    let func = declare_ffi_function(ctx, symbol, args.len());

    // Get expected param types from signature (for argument conversion).
    // Priority 1: Type signature registry (from Doo @extern declarations — package-ready)
    // Priority 2: Hardcoded table (C stdlib / runtime intrinsics)
    // Priority 3: No type info (fall through to default conversion)
    let expected_types: Vec<Option<&str>> =
        if let Some((param_type_ids, _, _)) = ctx.ffi_type_signatures.get(symbol) {
            param_type_ids
                .iter()
                .map(|tid| Some(type_id_to_ffi_str(*tid)))
                .collect()
        } else if let Some((param_strs, _, _)) = get_ffi_signature(symbol) {
            param_strs.iter().map(|s| Some(*s)).collect()
        } else {
            args.iter().map(|_| None).collect()
        };

    // ======================================================================
    // Package dispatch: pre-call hooks
    // ======================================================================
    // Delegates to the appropriate package module based on library name.
    // Each package handles its own setup (metadata, middleware, etc.).
    // Unknown packages: no-op.
    let library = crate::packages::resolve_library(ctx, symbol);
    crate::packages::pre_call(ctx, &library, symbol, args);

    // ======================================================================
    // Convert arguments — generic with package dispatch for specials
    // ======================================================================
    let ptr_type = ctx.context.ptr_type(AddressSpace::default());
    let mut arg_vals: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::with_capacity(args.len());

    for (i, a) in args.iter().enumerate() {
        // FuncRef: generate callback wrapper via package dispatch
        if let MirOperand::FuncRef(func_name) = a {
            let func_name_str = resolve(*func_name);

            // Ask the package for a specialized wrapper
            // (HTTP → handler wrapper, WS → event wrapper, etc.)
            if let Some(wrapper) =
                crate::packages::wrap_func_ref(ctx, &library, symbol, &func_name_str, args)
            {
                arg_vals.push(wrapper.as_global_value().as_pointer_value().into());
                continue;
            }

            // Generic passthrough: raw function pointer for unknown packages.
            // Third-party FFI crate must accept `extern "C" fn(...)`.
            if let Some(user_fn) = ctx.get_function(&func_name_str) {
                arg_vals.push(user_fn.as_global_value().as_pointer_value().into());
            } else {
                arg_vals.push(ptr_type.const_null().into());
            }
            continue;
        }

        // Package-specific argument conversion
        // (e.g., DB enum→JSON for doo_db_raw_param)
        if let Some(converted) = crate::packages::convert_arg(ctx, &library, symbol, i, a) {
            arg_vals.push(converted);
            continue;
        }

        // Standard argument conversion (type coercion to FFI types)
        if let Some(val) = operand_to_value(ctx, a) {
            let expected = expected_types.get(i).copied().flatten();
            arg_vals.push(convert_to_ffi_arg(ctx, val, expected));
        }
    }

    // Build call
    let call_site = ctx.builder.build_call(func, &arg_vals, "ffi_call").ok()?;

    // Handle return value
    if let Some(dest_name) = dest {
        if let Some(ret_val) = call_site.try_as_basic_value().basic() {
            ctx.set_temp(dest_name, ret_val);
            return Some(ret_val);
        }
    }

    // For void functions, return None
    call_site.try_as_basic_value().basic()
}
