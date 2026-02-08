//! Codegen Utilities
//!
//! Common helper functions used across instruction handlers.

use crate::context::CodegenContext;
use doo_core::doo_debug;
use doo_core::types::{builtin, TypeId, TypeKind};
use doo_mir::{MirConst, MirOperand};
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValueEnum, IntValue};
use inkwell::IntPredicate;

/// Convert a MirOperand to a BasicValueEnum.
pub fn operand_to_value<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    operand: &MirOperand,
) -> Option<BasicValueEnum<'ctx>> {
    match operand {
        MirOperand::Local(name) | MirOperand::Temp(name) | MirOperand::Global(name) => {
            // First try to get as a value (local variable, temp, etc.)
            if let Some(val) = ctx.get_value(name.as_str()) {
                return Some(val);
            }
            // Fall back to function reference - convert function to pointer value
            // This handles cases like passing `getUserHandler` as a callback argument
            if let Some(func) = ctx.get_function(name) {
                return Some(func.as_global_value().as_pointer_value().into());
            }
            None
        }
        MirOperand::FuncRef(name) => {
            // Explicit function reference - return function as pointer value
            // Used when passing functions to FFI (e.g., app.get("/users", getUserHandler))
            if let Some(func) = ctx.get_function(name) {
                return Some(func.as_global_value().as_pointer_value().into());
            }
            // Try mangled name for methods
            if let Some(func) = ctx.get_function(&format!("_{}", name)) {
                return Some(func.as_global_value().as_pointer_value().into());
            }
            None
        }
        MirOperand::Const(c) => match c {
            MirConst::Int(val) => Some(ctx.const_i64(*val).into()),
            MirConst::Float(val) => Some(ctx.const_f64(*val).into()),
            MirConst::Bool(val) => Some(ctx.const_bool(*val).into()),
            MirConst::Str(s) => {
                let global = ctx.builder.build_global_string_ptr(s, "str_const").ok()?;
                Some(global.as_pointer_value().into())
            }
            MirConst::Nil => Some(ctx.const_i64(0).into()),
        },
    }
}

/// Emit equality comparison for two values of the given type.
/// Accepts TypeId and maps to basic comparison strategy.
/// Falls back to LLVM value type inspection when TypeId doesn't match known types.
pub fn emit_eq<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    ty: TypeId,
    lhs: BasicValueEnum<'ctx>,
    rhs: BasicValueEnum<'ctx>,
) -> Option<IntValue<'ctx>> {
    // First, try to use TypeKind from the registry for more accurate type info
    if let Some(kind) = ctx.get_type_kind(ty) {
        return match kind {
            TypeKind::Int | TypeKind::Bool => {
                let mut lhs_int = lhs.into_int_value();
                let mut rhs_int = rhs.into_int_value();
                // Normalize int widths (e.g., Bool variable i64 vs Bool literal i1)
                let lw = lhs_int.get_type().get_bit_width();
                let rw = rhs_int.get_type().get_bit_width();
                if lw > rw {
                    rhs_int = ctx.builder.build_int_z_extend(rhs_int, lhs_int.get_type(), "zext").ok()?;
                } else if rw > lw {
                    lhs_int = ctx.builder.build_int_z_extend(lhs_int, rhs_int.get_type(), "zext").ok()?;
                }
                ctx.builder
                    .build_int_compare(IntPredicate::EQ, lhs_int, rhs_int, "eq")
                    .ok()
            }
            TypeKind::Float => ctx
                .builder
                .build_float_compare(
                    inkwell::FloatPredicate::OEQ,
                    lhs.into_float_value(),
                    rhs.into_float_value(),
                    "eq",
                )
                .ok(),
            TypeKind::Str => emit_str_eq(ctx, lhs, rhs),
            _ => {
                // For other types, fallback to LLVM value inspection
                emit_eq_by_llvm_type(ctx, lhs, rhs)
            }
        };
    }

    // Fallback: check builtin TypeId constants
    if ty == builtin::INT || ty == builtin::BOOL {
        let mut lhs_int = lhs.into_int_value();
        let mut rhs_int = rhs.into_int_value();
        let lw = lhs_int.get_type().get_bit_width();
        let rw = rhs_int.get_type().get_bit_width();
        if lw > rw {
            rhs_int = ctx.builder.build_int_z_extend(rhs_int, lhs_int.get_type(), "zext").ok()?;
        } else if rw > lw {
            lhs_int = ctx.builder.build_int_z_extend(lhs_int, rhs_int.get_type(), "zext").ok()?;
        }
        ctx.builder
            .build_int_compare(
                IntPredicate::EQ,
                lhs_int,
                rhs_int,
                "eq",
            )
            .ok()
    } else if ty == builtin::FLOAT {
        ctx.builder
            .build_float_compare(
                inkwell::FloatPredicate::OEQ,
                lhs.into_float_value(),
                rhs.into_float_value(),
                "eq",
            )
            .ok()
    } else if ty == builtin::STR {
        emit_str_eq(ctx, lhs, rhs)
    } else {
        // Final fallback: inspect actual LLVM types
        emit_eq_by_llvm_type(ctx, lhs, rhs)
    }
}

/// Emit string equality comparison using strcmp
fn emit_str_eq<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    lhs: BasicValueEnum<'ctx>,
    rhs: BasicValueEnum<'ctx>,
) -> Option<IntValue<'ctx>> {
    use doo_core::constants::ffi_names;
    let strcmp = ctx
        .module
        .get_function(ffi_names::STRCMP)
        .unwrap_or_else(|| {
            let i32_ty = ctx.context.i32_type();
            let ptr_ty = ctx.context.ptr_type(inkwell::AddressSpace::default());
            let fn_ty = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
            ctx.module.add_function(ffi_names::STRCMP, fn_ty, None)
        });

    let result = ctx
        .builder
        .build_call(strcmp, &[lhs.into(), rhs.into()], "strcmp_result")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_int_value();

    ctx.builder
        .build_int_compare(
            IntPredicate::EQ,
            result,
            ctx.context.i32_type().const_zero(),
            "str_eq",
        )
        .ok()
}

/// Emit equality by inspecting actual LLVM value types.
/// This is a fallback when TypeId doesn't provide enough info.
fn emit_eq_by_llvm_type<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    lhs: BasicValueEnum<'ctx>,
    rhs: BasicValueEnum<'ctx>,
) -> Option<IntValue<'ctx>> {
    // Check if both are int values
    if lhs.is_int_value() && rhs.is_int_value() {
        let mut lhs_int = lhs.into_int_value();
        let mut rhs_int = rhs.into_int_value();
        // Normalize int widths (e.g., Bool i64 vs i1)
        let lw = lhs_int.get_type().get_bit_width();
        let rw = rhs_int.get_type().get_bit_width();
        if lw > rw {
            rhs_int = ctx.builder.build_int_z_extend(rhs_int, lhs_int.get_type(), "zext").ok()?;
        } else if rw > lw {
            lhs_int = ctx.builder.build_int_z_extend(lhs_int, rhs_int.get_type(), "zext").ok()?;
        }
        return ctx
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                lhs_int,
                rhs_int,
                "eq",
            )
            .ok();
    }

    // Check if both are float values
    if lhs.is_float_value() && rhs.is_float_value() {
        return ctx
            .builder
            .build_float_compare(
                inkwell::FloatPredicate::OEQ,
                lhs.into_float_value(),
                rhs.into_float_value(),
                "eq",
            )
            .ok();
    }

    // Check if both are pointer values (strings or complex types)
    if lhs.is_pointer_value() && rhs.is_pointer_value() {
        // Pointer equality comparison
        return ctx
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                ctx.builder
                    .build_ptr_to_int(lhs.into_pointer_value(), ctx.context.i64_type(), "ptr1")
                    .ok()?,
                ctx.builder
                    .build_ptr_to_int(rhs.into_pointer_value(), ctx.context.i64_type(), "ptr2")
                    .ok()?,
                "ptr_eq",
            )
            .ok();
    }

    // If types don't match, try to handle common mismatches gracefully
    // For example, one might be int and other might be pointer (shouldn't happen in well-typed code)
    // Return None to signal failure
    doo_debug!("CODEGEN", "emit_eq_by_llvm_type: mismatched types lhs={:?} rhs={:?}",
        lhs.get_type(),
        rhs.get_type()
    );
    None
}

/// Get default value for a type (zero/null).
pub fn default_for_type<'ctx>(
    ctx: &CodegenContext<'ctx>,
    ty: BasicTypeEnum<'ctx>,
) -> BasicValueEnum<'ctx> {
    match ty {
        BasicTypeEnum::IntType(it) => it.const_zero().into(),
        BasicTypeEnum::FloatType(ft) => ft.const_zero().into(),
        BasicTypeEnum::PointerType(pt) => pt.const_null().into(),
        BasicTypeEnum::StructType(st) => st.const_zero().into(),
        BasicTypeEnum::ArrayType(at) => at.const_zero().into(),
        BasicTypeEnum::VectorType(vt) => vt.const_zero().into(),
        BasicTypeEnum::ScalableVectorType(svt) => svt.const_zero().into(),
    }
}
