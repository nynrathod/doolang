//! Call Utilities — shared helpers for call instruction codegen.
//!
//! Contains operand conversion, type coercion, and result struct loading.

use crate::context::CodegenContext;
use doo_mir::sym::resolve;
use doo_mir::{MirConst, MirOperand};
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValueEnum, PointerValue};
/// Convert MirOperand to LLVM value.
pub(super) fn operand_to_value<'ctx>(
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
            // This handles cases like passing `getUserHandler` as a callback argument
            if let Some(func) = ctx.get_function(&name_str) {
                return Some(func.as_global_value().as_pointer_value().into());
            }
            None
        }
        MirOperand::FuncRef(name) => {
            let name_str = resolve(*name);
            // Explicit function reference - return function as pointer value
            // Used when passing functions to FFI (e.g., app.get("/users", getUserHandler))
            if let Some(func) = ctx.get_function(&name_str) {
                return Some(func.as_global_value().as_pointer_value().into());
            }
            None
        }
    }
}

/// Coerce an argument value to match the expected function parameter type.
///
/// This handles type mismatches between how values are produced (e.g., enum StructValues)
/// and how function parameters are declared (e.g., pointers for composite types).
pub(super) fn coerce_arg_to_param_type<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
    expected_type: Option<BasicTypeEnum<'ctx>>,
) -> inkwell::values::BasicMetadataValueEnum<'ctx> {
    // If no expected type info, pass value as-is
    let Some(expected) = expected_type else {
        return val.into();
    };

    // If types already match, pass as-is
    if val.get_type() == expected {
        return val.into();
    }

    // Special case: StructValue passed where pointer is expected
    // This happens with enums: EnumCreate returns { i32, ptr } but function params expect ptr
    if val.is_struct_value() && expected.is_pointer_type() {
        // Box the struct value: allocate, store, return pointer
        let alloca = ctx.builder.build_alloca(val.get_type(), "arg_box").unwrap();
        ctx.builder.build_store(alloca, val).ok();
        return alloca.into();
    }

    // Special case: PointerValue passed where struct is expected
    // This happens when JSON.parse returns a pointer to enum but function expects struct by value
    if val.is_pointer_value() && expected.is_struct_type() {
        // Load the struct from the pointer
        let loaded = ctx
            .builder
            .build_load(expected, val.into_pointer_value(), "arg_load")
            .ok();
        if let Some(v) = loaded {
            return v.into();
        }
    }

    // CRITICAL FIX: IntValue passed where pointer is expected
    // This can happen when:
    // 1. Struct type info is lost during tuple extraction (TupleGet fallback)
    // 2. Field load defaults to i64 when struct type lookup fails
    // The value is actually a pointer stored as i64, convert it back.
    // This is a defensive measure - the proper fix is ensuring type info flows correctly.
    if val.is_int_value() && expected.is_pointer_type() {
        let int_val = val.into_int_value();
        if let Ok(ptr) =
            ctx.builder
                .build_int_to_ptr(int_val, expected.into_pointer_type(), "int_to_ptr_coerce")
        {
            return ptr.into();
        }
    }

    // Default: pass value as-is
    val.into()
}

/// Convert MirConst to LLVM value.
pub(super) fn const_to_value<'ctx>(ctx: &CodegenContext<'ctx>, c: &MirConst) -> BasicValueEnum<'ctx> {
    match c {
        MirConst::Int(v) => ctx.const_i64(*v).into(),
        MirConst::Float(v) => ctx.const_f64(*v).into(),
        MirConst::Bool(v) => ctx.const_bool(*v).into(),
        MirConst::Nil => ctx.const_i64(0).into(),
        MirConst::Str(s) => ctx.const_string(s).into(),
    }
}

// ============================================================================
// Result/Error Handling Helpers
// ============================================================================

/// Convert a value to a pointer representation for storing in Result payload.
/// - Pointers: pass through as-is
/// - Integers: use inttoptr
/// - Floats: bitcast to i64, then inttoptr
pub(super) fn value_to_ptr<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
) -> Option<PointerValue<'ctx>> {
    if val.is_pointer_value() {
        // Already a pointer (string, array, map, struct)
        Some(val.into_pointer_value())
    } else if val.is_int_value() {
        // Cast integer to pointer using inttoptr
        let int_val = val.into_int_value();
        let int_64 = if int_val.get_type().get_bit_width() == 64 {
            int_val
        } else {
            ctx.builder
                .build_int_z_extend(int_val, ctx.i64_type(), "ext")
                .ok()?
        };
        ctx.builder
            .build_int_to_ptr(int_64, ctx.ptr_type(), "int_as_ptr")
            .ok()
    } else if val.is_float_value() {
        // Bitcast float to i64 then to pointer
        let float_val = val.into_float_value();
        let alloca = ctx.builder.build_alloca(ctx.f64_type(), "f_tmp").ok()?;
        ctx.builder.build_store(alloca, float_val).ok()?;
        let i64_ptr = ctx
            .builder
            .build_pointer_cast(alloca, ctx.ptr_type(), "i64_ptr")
            .ok()?;
        let i64_val = ctx
            .builder
            .build_load(ctx.i64_type(), i64_ptr, "f_as_i64")
            .ok()?
            .into_int_value();
        ctx.builder
            .build_int_to_ptr(i64_val, ctx.ptr_type(), "float_as_ptr")
            .ok()
    } else if val.is_struct_value() {
        // Heap-allocate struct and return pointer
        let struct_val = val.into_struct_value();
        let struct_type = struct_val.get_type();
        let heap_ptr = ctx.builder.build_malloc(struct_type, "struct_heap").ok()?;
        ctx.builder.build_store(heap_ptr, struct_val).ok()?;
        Some(heap_ptr)
    } else {
        // Fallback: use null pointer
        Some(ctx.ptr_type().const_null())
    }
}

/// Load a Result struct from a value that may be a pointer or struct.
pub(super) fn load_result_struct<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    result_val: BasicValueEnum<'ctx>,
) -> Option<inkwell::values::StructValue<'ctx>> {
    // Result struct layout: { i64 tag, i64 value }
    // Using i64 for both fields for consistent ABI with FFI SimpleResult
    let result_struct_type = ctx
        .context
        .struct_type(&[ctx.i64_type().into(), ctx.i64_type().into()], false);

    if result_val.is_pointer_value() && !result_val.is_struct_value() {
        // Load from pointer
        let result_ptr = result_val.into_pointer_value();
        ctx.builder
            .build_load(result_struct_type, result_ptr, "result_struct_load")
            .ok()?
            .try_into()
            .ok()
    } else if result_val.is_struct_value() {
        // Already a struct value
        Some(result_val.into_struct_value())
    } else {
        // Not a Result - return None
        None
    }
}
