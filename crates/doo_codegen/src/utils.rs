//! Codegen Utilities
//!
//! Common helper functions used across instruction handlers.

use crate::context::CodegenContext;
use doo_core::doo_debug;
use doo_core::types::{builtin, TypeId, TypeKind};
use doo_mir::sym::resolve;
use doo_mir::{MirConst, MirOperand};
use inkwell::types::BasicTypeEnum;
use inkwell::values::PointerValue;
use inkwell::values::{BasicValueEnum, IntValue};
use inkwell::IntPredicate;

/// Convert a MirOperand to a BasicValueEnum.
pub fn operand_to_value<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    operand: &MirOperand,
) -> Option<BasicValueEnum<'ctx>> {
    match operand {
        MirOperand::Local(name) | MirOperand::Temp(name) | MirOperand::Global(name) => {
            let name_str = resolve(*name);
            // Check if this is a static global — directly read from OnceLock global
            if let Some(_) = ctx.static_globals.get(&name_str) {
                if let Some(once_lock) = ctx.module.get_global(&name_str) {
                    let once_lock_ptr = once_lock.as_pointer_value();
                    let once_lock_type = ctx.context.struct_type(
                        &[
                            ctx.context.bool_type().into(),
                            ctx.context.ptr_type(inkwell::AddressSpace::default()).into(),
                        ],
                        false,
                    );
                    // GEP to field 1 (ptr), load the stored pointer
                    let ptr_field = ctx.builder.build_struct_gep(
                        once_lock_type,
                        once_lock_ptr,
                        1,
                        "static_get_ptr",
                    );
                    if let Ok(ptr_field) = ptr_field {
                        let loaded = ctx.builder.build_load(
                            ctx.context.ptr_type(inkwell::AddressSpace::default()),
                            ptr_field,
                            "static_val",
                        ).ok();
                        if let Some(loaded_val) = loaded {
                            return Some(loaded_val);
                        }
                    }
                }
                // Fallback: return null pointer
                return Some(
                    ctx.context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                );
            }
            // First try to get as a value (local variable, temp, etc.)
            if let Some(val) = ctx.get_value(&name_str) {
                return Some(val);
            }
            // Fall back to function reference - convert function to pointer value
            if let Some(func) = ctx.get_function(&name_str) {
                return Some(func.as_global_value().as_pointer_value().into());
            }
            // Check if this is a type name (e.g., receiver of static method call like Service.new())
            // Return null pointer — associated/static methods don't need an instance receiver
            if ctx.type_registry.lookup(&name_str).is_some() {
                return Some(
                    ctx.context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into(),
                );
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
            // Try mangled name for methods
            if let Some(func) = ctx.get_function(&format!("_{}", name_str)) {
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
            TypeKind::Float32 | TypeKind::Float64 => ctx
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

/// Coerce a potentially-null string pointer to a valid empty string.
/// This prevents undefined behavior when the pointer is passed to strlen/strcmp/memcpy,
/// because LLVM infers `nonnull dereferenceable(1)` on those function arguments
/// when it recognizes them as standard C library functions.
/// A null pointer with nonnull annotation is UB that LLVM can exploit to miscompile code.
pub fn null_coerce_str<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    ptr: PointerValue<'ctx>,
) -> PointerValue<'ctx> {
    let is_null = match ctx.builder.build_is_null(ptr, "str_is_null") {
        Ok(v) => v,
        Err(_) => return ptr,
    };
    let empty = match ctx.builder.build_global_string_ptr("", "empty_str") {
        Ok(v) => v.as_pointer_value(),
        Err(_) => return ptr,
    };
    match ctx
        .builder
        .build_select(is_null, empty, ptr, "safe_str")
    {
        Ok(v) => v.into_pointer_value(),
        Err(_) => ptr,
    }
}

/// Emit string equality comparison using strcmp.
/// SAFETY: Both operands are null-coerced to prevent UB from LLVM's
/// nonnull inference on strcmp arguments.
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

    // Null-coerce both operands: LLVM infers nonnull+dereferenceable(1) on
    // strcmp args because it recognizes the function name. If either operand
    // is null (e.g., from a try-expression error path), the nonnull annotation
    // triggers UB-based miscompilation in large functions.
    let safe_lhs = null_coerce_str(ctx, lhs.into_pointer_value());
    let safe_rhs = null_coerce_str(ctx, rhs.into_pointer_value());

    let result = ctx
        .builder
        .build_call(strcmp, &[safe_lhs.into(), safe_rhs.into()], "strcmp_result")
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
    None
}

/// Get default value for a type (zero/null).
pub fn default_for_type<'ctx>(
    _ctx: &CodegenContext<'ctx>,
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
