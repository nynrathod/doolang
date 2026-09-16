//! Codegen Utilities — constant conversion and helper functions.
//!
//! Provides conversion from MIR constants to LLVM values, and shared
//! helper functions used across instruction handlers.

use crate::context::CodegenContext;
use doo_mir::sym::resolve;
use doo_mir::{MirConst, MirOperand};
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::{BasicValueEnum, FloatValue, IntValue, PointerValue, StructValue};
use inkwell::AddressSpace;

/// Convert a MIR constant to an LLVM constant value.
pub fn const_to_llvm<'ctx>(ctx: &CodegenContext<'ctx>, c: &MirConst) -> BasicValueEnum<'ctx> {
    match c {
        MirConst::Int(v) => llvm_const_int(ctx, *v).into(),
        MirConst::Float(v) => llvm_const_float(ctx, *v).into(),
        MirConst::Bool(v) => llvm_const_bool(ctx, *v).into(),
        MirConst::Nil => ctx.context.i64_type().const_zero().into(),
        MirConst::Str(s) => llvm_const_string(ctx, s).into(),
    }
}

/// Create an LLVM i64 constant.
pub fn llvm_const_int<'ctx>(ctx: &CodegenContext<'ctx>, val: i64) -> IntValue<'ctx> {
    ctx.context.i64_type().const_int(val as u64, true)
}

/// Create an LLVM f64 constant.
pub fn llvm_const_float<'ctx>(ctx: &CodegenContext<'ctx>, val: f64) -> FloatValue<'ctx> {
    ctx.context.f64_type().const_float(val)
}

/// Create an LLVM boolean (i1) constant.
pub fn llvm_const_bool<'ctx>(ctx: &CodegenContext<'ctx>, val: bool) -> IntValue<'ctx> {
    ctx.context.bool_type().const_int(val as u64, false)
}

/// Create an LLVM fat string constant ({ i8*, i64 }).
///
/// Creates a global constant string and wraps it in the fat pointer struct.
pub fn llvm_const_string<'ctx>(ctx: &CodegenContext<'ctx>, s: &str) -> StructValue<'ctx> {
    let ptr_ty = ctx.context.ptr_type(AddressSpace::default());
    let str_type = ctx
        .context
        .struct_type(&[ptr_ty.into(), ctx.context.i64_type().into()], false);

    let global = ctx.module.add_global(
        ctx.context.i8_type().array_type(s.len() as u32 + 1),
        Some(AddressSpace::default()),
        "",
    );
    global.set_constant(true);
    global.set_initializer(&ctx.context.const_string(s.as_bytes(), true));

    let ptr_val = global.as_pointer_value();
    let len_val = ctx.context.i64_type().const_int(s.len() as u64, false);

    let mut str_val = str_type.get_undef();
    str_val = ctx
        .builder
        .build_insert_value(str_val, ptr_val, 0, "str_ptr")
        .ok()
        .map(|v| v.into_struct_value())
        .unwrap_or(str_val);
    str_val = ctx
        .builder
        .build_insert_value(str_val, len_val, 1, "str_len")
        .ok()
        .map(|v| v.into_struct_value())
        .unwrap_or(str_val);

    str_val
}

/// Null-coerce a string pointer to prevent UB from null string operands.
///
/// LLVM infers `nonnull` and `dereferenceable(1)` on string function arguments
/// (strlen, strcmp, etc.). A null pointer triggers undefined behavior that
/// LLVM O3 can exploit. This replaces null with a pointer to an empty string.
pub fn null_coerce_str<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    ptr: PointerValue<'ctx>,
) -> PointerValue<'ctx> {
    let is_null = ctx.builder.build_is_null(ptr, "str_null_check").ok();

    if let Some(is_null) = is_null {
        let empty_str = ctx
            .builder
            .build_global_string_ptr("", "empty_str_fallback")
            .map(|g| g.as_pointer_value())
            .unwrap_or_else(|_| {
                ctx.context
                    .i8_type()
                    .ptr_type(AddressSpace::default())
                    .const_null()
            });

        let safe = ctx
            .builder
            .build_select(is_null, empty_str, ptr, "safe_str")
            .ok()
            .map(|v| v.into_pointer_value())
            .unwrap_or(ptr);

        safe
    } else {
        ptr
    }
}

/// Convert a MirOperand to an LLVM value.
pub fn operand_to_value<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    operand: &MirOperand,
) -> Option<BasicValueEnum<'ctx>> {
    match operand {
        MirOperand::Const(c) => Some(const_to_llvm(ctx, c)),
        MirOperand::Local(name) | MirOperand::Temp(name) | MirOperand::Global(name) => {
            let name_str = resolve(*name);

            // Check static globals (case-insensitive lookup)
            let resolved_static_name = if ctx.static_globals.contains_key(&name_str) {
                Some(name_str.clone())
            } else {
                let lowered = name_str.to_lowercase();
                ctx.static_globals
                    .keys()
                    .find(|k| k.to_lowercase() == lowered)
                    .cloned()
            };

            if let Some(static_name) = resolved_static_name {
                if let Some(once_lock) = ctx.module.get_global(&static_name) {
                    let once_lock_ptr = once_lock.as_pointer_value();
                    let once_lock_type = ctx.context.struct_type(
                        &[
                            ctx.context.bool_type().into(),
                            ctx.context.ptr_type(AddressSpace::default()).into(),
                        ],
                        false,
                    );
                    if let Ok(ptr_field) = ctx.builder.build_struct_gep(
                        once_lock_type,
                        once_lock_ptr,
                        1,
                        "static_get_ptr",
                    ) {
                        if let Ok(loaded) = ctx.builder.build_load(
                            ctx.context.ptr_type(AddressSpace::default()),
                            ptr_field,
                            "static_val",
                        ) {
                            return Some(loaded);
                        }
                    }
                }
                return Some(
                    ctx.context
                        .ptr_type(AddressSpace::default())
                        .const_null()
                        .into(),
                );
            }

            // Check if this is a type name
            if ctx.type_registry.lookup(&name_str).is_some() {
                return Some(
                    ctx.context
                        .ptr_type(AddressSpace::default())
                        .const_null()
                        .into(),
                );
            }

            // Check temps and locals
            if let Some(val) = ctx.get_value(&name_str) {
                return Some(val);
            }

            // Fall back to function reference
            if let Some(func) = ctx.get_function(&name_str) {
                return Some(func.as_global_value().as_pointer_value().into());
            }
            None
        }
        MirOperand::FuncRef(name) => {
            let name_str = resolve(*name);
            if let Some(func) = ctx.get_function(&name_str) {
                return Some(func.as_global_value().as_pointer_value().into());
            }
            None
        }
    }
}

/// Emit an equality comparison between two values of the same type.
pub fn emit_eq<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    type_id: doo_core::types::TypeId,
    lhs: BasicValueEnum<'ctx>,
    rhs: BasicValueEnum<'ctx>,
) -> Option<inkwell::values::IntValue<'ctx>> {
    use doo_core::types::TypeKind;
    use inkwell::IntPredicate;

    let kind = ctx.get_type_kind(type_id)?;

    match kind {
        TypeKind::Int
        | TypeKind::Int8
        | TypeKind::Int16
        | TypeKind::Int32
        | TypeKind::Int64
        | TypeKind::UInt
        | TypeKind::UInt8
        | TypeKind::UInt16
        | TypeKind::UInt32
        | TypeKind::UInt64
        | TypeKind::Bool
        | TypeKind::Char => {
            let lhs_int = lhs.into_int_value();
            let rhs_int = rhs.into_int_value();
            ctx.builder
                .build_int_compare(IntPredicate::EQ, lhs_int, rhs_int, "eq")
                .ok()
        }
        TypeKind::Float32 | TypeKind::Float64 => {
            let lhs_f = lhs.into_float_value();
            let rhs_f = rhs.into_float_value();
            ctx.builder
                .build_float_compare(inkwell::FloatPredicate::OEQ, lhs_f, rhs_f, "feq")
                .ok()
        }
        TypeKind::Str => {
            let safe_lhs = null_coerce_str(ctx, lhs.into_pointer_value());
            let safe_rhs = null_coerce_str(ctx, rhs.into_pointer_value());
            let strcmp = ctx
                .module
                .get_function(doo_core::constants::ffi_names::STRCMP)
                .unwrap_or_else(|| {
                    let ptr_type = ctx.context.ptr_type(AddressSpace::default());
                    let fn_type = ctx
                        .context
                        .i32_type()
                        .fn_type(&[ptr_type.into(), ptr_type.into()], false);
                    ctx.module
                        .add_function(doo_core::constants::ffi_names::STRCMP, fn_type, None)
                });
            let cmp_result = ctx
                .builder
                .build_call(strcmp, &[safe_lhs.into(), safe_rhs.into()], "strcmp")
                .ok()?
                .try_as_basic_value()
                .basic()?
                .into_int_value();
            ctx.builder
                .build_int_compare(
                    IntPredicate::EQ,
                    cmp_result,
                    ctx.context.i32_type().const_zero(),
                    "str_eq",
                )
                .ok()
        }
        _ => {
            // For other types, compare pointers
            if lhs.is_pointer_value() && rhs.is_pointer_value() {
                ctx.builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        ctx.builder
                            .build_ptr_to_int(lhs.into_pointer_value(), ctx.context.i64_type(), "l")
                            .ok()?,
                        ctx.builder
                            .build_ptr_to_int(rhs.into_pointer_value(), ctx.context.i64_type(), "r")
                            .ok()?,
                        "ptr_eq",
                    )
                    .ok()
            } else {
                None
            }
        }
    }
}

/// Generate a default value for a given LLVM type.
///
/// Used for map lookups that don't find a key, and optional unwrapping
/// when the value is absent.
pub fn default_for_type<'ctx>(
    ctx: &CodegenContext<'ctx>,
    ty: impl BasicType<'ctx>,
) -> BasicValueEnum<'ctx> {
    let ty_enum: BasicTypeEnum = ty.as_basic_type_enum();
    match ty_enum {
        BasicTypeEnum::IntType(it) => it.const_zero().into(),
        BasicTypeEnum::FloatType(ft) => ft.const_zero().into(),
        BasicTypeEnum::PointerType(pt) => pt.const_null().into(),
        BasicTypeEnum::StructType(st) => st.const_zero().into(),
        BasicTypeEnum::ArrayType(at) => at.const_zero().into(),
        BasicTypeEnum::VectorType(vt) => vt.const_zero().into(),
        BasicTypeEnum::ScalableVectorType(_) => ctx.context.i64_type().const_zero().into(), // Add this line
    }
}
