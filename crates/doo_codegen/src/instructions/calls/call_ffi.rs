//! FFI call implementation — signatures, declarations, and call emission.
//!
//! This module is completely package-agnostic. All @extern functions are
//! called by name through the generic FFI path. The compiler never matches
//! on framework symbols like "doo_http" — it resolves function signatures
//! from @extern declarations via the type signature registry.

use super::call_utils::operand_to_value;
use crate::context::CodegenContext;
use doo_core::constants::ffi_names;
use doo_core::doo_debug;
use doo_core::types::{builtin, TypeId};
use doo_mir::sym::resolve;
use doo_mir::MirOperand;
use inkwell::module::Linkage;
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::{BasicValueEnum, FunctionValue};
use inkwell::AddressSpace;

// ============================================================================
// FFI Call Implementation
// ============================================================================

/// FFI function signature: (param_types, return_type, is_variadic)
type FfiSignature = (&'static [&'static str], &'static str, bool);

/// Get FFI function signature for C stdlib and runtime intrinsic functions.
///
/// This table ONLY contains signatures for functions that have NO Doo
/// `@extern` declaration (C stdlib, Doo runtime allocator). All Doo-declared
/// FFI functions get their signatures from the type signature registry
/// populated from MIR FfiLinkage.
fn get_ffi_signature(symbol: &str) -> Option<FfiSignature> {
    match symbol {
        // C Standard Library
        ffi_names::MALLOC => Some((&["i64"], "ptr", false)),
        ffi_names::FREE => Some((&["ptr"], "void", false)),
        ffi_names::REALLOC => Some((&["ptr", "i64"], "ptr", false)),
        ffi_names::STRLEN => Some((&["ptr"], "i64", false)),
        ffi_names::STRCMP => Some((&["ptr", "ptr"], "i32", false)),
        ffi_names::STRCPY => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::STRCAT => Some((&["ptr", "ptr"], "ptr", false)),
        ffi_names::MEMCPY => Some((&["ptr", "ptr", "i64"], "ptr", false)),
        ffi_names::MEMSET => Some((&["ptr", "i32", "i64"], "ptr", false)),
        ffi_names::PRINTF => Some((&["ptr"], "i32", true)),
        ffi_names::SNPRINTF => Some((&["ptr", "i64", "ptr"], "i32", true)),
        ffi_names::SPRINTF => Some((&["ptr", "ptr"], "i32", true)),
        ffi_names::PUTS => Some((&["ptr"], "i32", false)),
        ffi_names::PUTCHAR => Some((&["i32"], "i32", false)),
        ffi_names::FFLUSH => Some((&["ptr"], "i32", false)),
        ffi_names::EXIT => Some((&["i64"], "void", false)),

        // Doo Runtime Allocator
        ffi_names::DOO_ALLOC => Some((&["i64"], "ptr", false)),
        ffi_names::DOO_FREE => Some((&["ptr"], "void", false)),
        ffi_names::DOO_REALLOC => Some((&["ptr", "i64"], "ptr", false)),

        // Print/debug runtime (HIR `print` desugars to these names)
        ffi_names::DOO_PRINT_STR => Some((&["ptr"], "void", false)),
        ffi_names::DOO_PRINTLN => Some((&[], "void", false)),
        ffi_names::DOO_FLUSH => Some((&[], "void", false)),
        ffi_names::DOO_INT_TO_STR => Some((&["i64"], "ptr", false)),
        ffi_names::DOO_FLOAT_TO_STR => Some((&["f64"], "ptr", false)),
        ffi_names::DOO_BOOL_TO_STR => Some((&["i32"], "ptr", false)),
        ffi_names::DOO_NULL_TO_STR => Some((&[], "ptr", false)),
        ffi_names::DOO_STR_FREE => Some((&["ptr"], "void", false)),

        // Math intrinsics
        "fabs" => Some((&["f64"], "f64", false)),
        "floor" => Some((&["f64"], "f64", false)),
        "ceil" => Some((&["f64"], "f64", false)),
        "round" => Some((&["f64"], "f64", false)),
        "sqrt" => Some((&["f64"], "f64", false)),

        _ => None,
    }
}

pub(crate) fn is_runtime_symbol(symbol: &str) -> bool {
    get_ffi_signature(symbol).is_some()
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
        "void" => None,
        "simple_result" => {
            let struct_ty = ctx
                .context
                .struct_type(&[ctx.i64_type().into(), ctx.ptr_type().into()], false);
            Some(struct_ty.into())
        }
        _ => Some(ctx.i64_type().into()),
    }
}

/// Convert a Doo TypeId to the corresponding FFI type string.
fn type_id_to_ffi_str(type_id: TypeId) -> &'static str {
    if type_id == builtin::INT {
        "i64"
    } else if type_id == builtin::FLOAT {
        "f64"
    } else if type_id == builtin::BOOL {
        "i32"
    } else if type_id == builtin::VOID {
        "void"
    } else {
        "ptr"
    }
}

/// Declare an FFI function with proper signature and external linkage.
///
/// Resolution order:
/// 1. Type signature registry — from MIR FfiLinkage (Doo @extern declarations)
/// 2. Hardcoded table — C stdlib/runtime functions without @extern
/// 3. Fallback — all-pointer inference for unknown symbols
pub(crate) fn declare_ffi_function<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    symbol: &str,
    arg_count: usize,
) -> FunctionValue<'ctx> {
    if let Some(func) = ctx.get_function(symbol) {
        return func;
    }

    let ptr_ty = ctx.context.ptr_type(AddressSpace::default());

    // Priority 1: Type signature registry (from Doo @extern declarations)
    if let Some((param_type_ids, return_type_id, is_result)) =
        ctx.ffi_type_signatures.get(symbol).cloned()
    {
        let params: Vec<BasicTypeEnum> = param_type_ids
            .iter()
            .filter_map(|tid| ffi_type_to_llvm(ctx, type_id_to_ffi_str(*tid)))
            .collect();

        let ret = if is_result {
            Some(ptr_ty.into())
        } else {
            match return_type_id {
                Some(tid) => ffi_type_to_llvm(ctx, type_id_to_ffi_str(tid)),
                None => None,
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

    // Priority 2: Hardcoded table (C stdlib / Doo runtime intrinsics)
    let (param_types_vec, return_type, is_variadic) =
        if let Some((param_strs, ret_str, variadic)) = get_ffi_signature(symbol) {
            let params: Vec<BasicTypeEnum> = param_strs
                .iter()
                .filter_map(|s| ffi_type_to_llvm(ctx, s))
                .collect();
            let ret = ffi_type_to_llvm(ctx, ret_str);
            (params, ret, variadic)
        } else {
            // Priority 3: Fallback — all-pointer inference
            let params: Vec<BasicTypeEnum> = (0..arg_count).map(|_| ptr_ty.into()).collect();
            (params, Some(ptr_ty.into()), false)
        };

    let param_meta: Vec<inkwell::types::BasicMetadataTypeEnum> =
        param_types_vec.iter().map(|t| (*t).into()).collect();

    let fn_type = match return_type {
        Some(ret) => ret.fn_type(&param_meta, is_variadic),
        None => ctx.context.void_type().fn_type(&param_meta, is_variadic),
    };

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
            if val.is_int_value() {
                let int_val = val.into_int_value();
                let bit_width = int_val.get_type().get_bit_width();
                if bit_width == 64 {
                    let truncated = ctx
                        .builder
                        .build_int_truncate(int_val, ctx.i32_type(), "i64_to_i32")
                        .unwrap();
                    return truncated.into();
                } else if bit_width < 32 {
                    let extended = ctx
                        .builder
                        .build_int_z_extend(int_val, ctx.i32_type(), "bool_to_i32")
                        .unwrap();
                    return extended.into();
                }
            }
            val.into()
        }
        Some("f64") => {
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
            if val.is_int_value() {
                let int_val = val.into_int_value();
                let ptr_type = ctx.ptr_type();

                if let Some(const_val) = int_val.get_zero_extended_constant() {
                    if const_val == 0 {
                        return ptr_type.const_null().into();
                    }
                }

                let i8_type = ctx.context.i8_type();
                let i64_type = ctx.i64_type();

                let buffer = ctx
                    .builder
                    .build_array_alloca(i8_type, i64_type.const_int(24, false), "int_to_str_buf")
                    .unwrap();

                let sprintf = ctx
                    .module
                    .get_function(ffi_names::SPRINTF)
                    .unwrap_or_else(|| {
                        let i32_type = ctx.i32_type();
                        let fn_type = i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], true);
                        ctx.module.add_function(ffi_names::SPRINTF, fn_type, None)
                    });

                let fmt = ctx.const_string("%lld");
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
                let ptr_type = ctx.ptr_type();
                let f64_type = ctx.f64_type();
                let format_fn = ctx
                    .module
                    .get_function("doo_format_float")
                    .unwrap_or_else(|| {
                        let fn_ty = ptr_type.fn_type(&[f64_type.into()], false);
                        ctx.module.add_function("doo_format_float", fn_ty, None)
                    });

                let float_val = val.into_float_value();
                let result = ctx
                    .builder
                    .build_call(format_fn, &[float_val.into()], "fmt_float")
                    .ok()
                    .and_then(|v| v.try_as_basic_value().basic());
                if let Some(str_ptr) = result {
                    return str_ptr.into();
                }
                return val.into();
            } else if val.is_struct_value() {
                let struct_val = val.into_struct_value();
                let alloca = ctx
                    .alloca_in_entry_block(struct_val.get_type(), "enum_box")
                    .unwrap();
                ctx.builder.build_store(alloca, struct_val).ok();
                return alloca.into();
            }
            val.into()
        }
        _ => val.into(),
    }
}

/// Emit an FFI call with proper type handling.
///
/// All @extern functions are called by name through this single generic path.
/// FuncRef arguments are passed as raw function pointers — the FFI crate must
/// accept `extern "C" fn(...)`.
pub(crate) fn emit_ffi_call<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    dest: Option<&str>,
    symbol: &str,
    args: &[MirOperand],
) -> Option<BasicValueEnum<'ctx>> {
    if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {}

    let func = declare_ffi_function(ctx, symbol, args.len());

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

    let ptr_type = ctx.context.ptr_type(AddressSpace::default());
    let mut arg_vals: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::with_capacity(args.len());

    for (i, a) in args.iter().enumerate() {
        // FuncRef: pass the raw function pointer directly.
        // Third-party FFI crates must accept extern "C" fn(...).
        if let MirOperand::FuncRef(func_name) = a {
            let func_name_str = resolve(*func_name);

            if let Some(user_fn) = ctx.get_function(&func_name_str) {
                arg_vals.push(user_fn.as_global_value().as_pointer_value().into());
            } else {
                arg_vals.push(ptr_type.const_null().into());
            }
            continue;
        }

        // Standard argument conversion with type coercion to FFI types
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
            // FFI Bool returns i32 (C ABI), but internal Doo Bool is i8 — truncate if needed
            let ret_val = if ret_val.is_int_value() {
                let int_val = ret_val.into_int_value();
                if int_val.get_type().get_bit_width() == 32 {
                    // Check if the registered return type is Bool
                    if let Some((_, Some(ret_type_id), _)) = ctx.ffi_type_signatures.get(symbol) {
                        if *ret_type_id == builtin::BOOL {
                            let truncated = ctx
                                .builder
                                .build_int_truncate(int_val, ctx.context.i8_type(), "i32_to_bool")
                                .unwrap();
                            truncated.into()
                        } else {
                            ret_val
                        }
                    } else {
                        ret_val
                    }
                } else {
                    ret_val
                }
            } else {
                ret_val
            };
            ctx.set_temp(dest_name, ret_val);
            return Some(ret_val);
        }
    }

    // For void functions, return None
    call_site.try_as_basic_value().basic()
}
