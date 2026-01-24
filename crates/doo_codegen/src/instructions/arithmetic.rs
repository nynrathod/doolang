//! Arithmetic Instruction Handler
//!
//! Handles: BinaryOp, UnaryOp

use inkwell::values::BasicValueEnum;
use inkwell::IntPredicate;
use inkwell::FloatPredicate;
use doo_core::constants::ffi_names;
use doo_mir::{MirInstr, MirInstrKind, BinaryOp, UnaryOp, MirOperand, MirConst};
use crate::context::CodegenContext;
use super::InstructionHandler;

/// Arithmetic instruction handler.
pub struct ArithmeticHandler;

impl<'ctx> InstructionHandler<'ctx> for ArithmeticHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(instr.kind, 
            MirInstrKind::BinaryOp { .. } | 
            MirInstrKind::UnaryOp { .. }
        )
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::BinaryOp { dest, op, lhs, rhs } => {
                let lhs_val = operand_to_value(ctx, lhs)?;
                let rhs_val = operand_to_value(ctx, rhs)?;

                let result = emit_binop(ctx, *op, lhs_val, rhs_val)?;
                ctx.set_temp(dest, result);
                Some(result)
            }

            MirInstrKind::UnaryOp { dest, op, operand } => {
                let val = operand_to_value(ctx, operand)?;
                let result = emit_unaryop(ctx, *op, val)?;
                ctx.set_temp(dest, result);
                Some(result)
            }

            _ => None,
        }
    }
}

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

/// Emit binary operation.
fn emit_binop<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    op: BinaryOp,
    lhs: BasicValueEnum<'ctx>,
    rhs: BasicValueEnum<'ctx>,
) -> Option<BasicValueEnum<'ctx>> {
    // Handle string concatenation (both operands are pointers)
    // We assume pointers in Add are Strings for now (until Analysis types available here)
    if matches!(op, BinaryOp::Concat) || (matches!(op, BinaryOp::Add) && lhs.is_pointer_value() && rhs.is_pointer_value()) {
        return emit_string_concat(ctx, lhs, rhs);
    }
    
    if lhs.is_int_value() && rhs.is_int_value() {
        let lhs_int = lhs.into_int_value();
        let rhs_int = rhs.into_int_value();

        let result = match op {
            BinaryOp::Add => ctx.builder.build_int_add(lhs_int, rhs_int, "add").ok()?.into(),
            BinaryOp::Sub => ctx.builder.build_int_sub(lhs_int, rhs_int, "sub").ok()?.into(),
            BinaryOp::Mul => ctx.builder.build_int_mul(lhs_int, rhs_int, "mul").ok()?.into(),
            BinaryOp::Div => ctx.builder.build_int_signed_div(lhs_int, rhs_int, "div").ok()?.into(),
            BinaryOp::Mod => ctx.builder.build_int_signed_rem(lhs_int, rhs_int, "mod").ok()?.into(),
            BinaryOp::Eq => ctx.builder.build_int_compare(IntPredicate::EQ, lhs_int, rhs_int, "eq").ok()?.into(),
            BinaryOp::Ne => ctx.builder.build_int_compare(IntPredicate::NE, lhs_int, rhs_int, "ne").ok()?.into(),
            BinaryOp::Lt => ctx.builder.build_int_compare(IntPredicate::SLT, lhs_int, rhs_int, "lt").ok()?.into(),
            BinaryOp::Le => ctx.builder.build_int_compare(IntPredicate::SLE, lhs_int, rhs_int, "le").ok()?.into(),
            BinaryOp::Gt => ctx.builder.build_int_compare(IntPredicate::SGT, lhs_int, rhs_int, "gt").ok()?.into(),
            BinaryOp::Ge => ctx.builder.build_int_compare(IntPredicate::SGE, lhs_int, rhs_int, "ge").ok()?.into(),
            BinaryOp::And => ctx.builder.build_and(lhs_int, rhs_int, "and").ok()?.into(),
            BinaryOp::Or => ctx.builder.build_or(lhs_int, rhs_int, "or").ok()?.into(),
            BinaryOp::Concat => return None, // Handled above
        };
        Some(result)
    } else if lhs.is_float_value() && rhs.is_float_value() {
        let lhs_float = lhs.into_float_value();
        let rhs_float = rhs.into_float_value();

        let result = match op {
            BinaryOp::Add => ctx.builder.build_float_add(lhs_float, rhs_float, "fadd").ok()?.into(),
            BinaryOp::Sub => ctx.builder.build_float_sub(lhs_float, rhs_float, "fsub").ok()?.into(),
            BinaryOp::Mul => ctx.builder.build_float_mul(lhs_float, rhs_float, "fmul").ok()?.into(),
            BinaryOp::Div => ctx.builder.build_float_div(lhs_float, rhs_float, "fdiv").ok()?.into(),
            BinaryOp::Eq => ctx.builder.build_float_compare(FloatPredicate::OEQ, lhs_float, rhs_float, "feq").ok()?.into(),
            BinaryOp::Ne => ctx.builder.build_float_compare(FloatPredicate::ONE, lhs_float, rhs_float, "fne").ok()?.into(),
            BinaryOp::Lt => ctx.builder.build_float_compare(FloatPredicate::OLT, lhs_float, rhs_float, "flt").ok()?.into(),
            BinaryOp::Le => ctx.builder.build_float_compare(FloatPredicate::OLE, lhs_float, rhs_float, "fle").ok()?.into(),
            BinaryOp::Gt => ctx.builder.build_float_compare(FloatPredicate::OGT, lhs_float, rhs_float, "fgt").ok()?.into(),
            BinaryOp::Ge => ctx.builder.build_float_compare(FloatPredicate::OGE, lhs_float, rhs_float, "fge").ok()?.into(),
            _ => return None,
        };
        Some(result)
    } else {
        None
    }
}

/// Emit string concatenation.
/// Calls strlen, malloc, memcpy to build a new concatenated string.
fn emit_string_concat<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    lhs: BasicValueEnum<'ctx>,
    rhs: BasicValueEnum<'ctx>,
) -> Option<BasicValueEnum<'ctx>> {
    let str1_ptr = lhs.into_pointer_value();
    let str2_ptr = rhs.into_pointer_value();
    
    // Declare external functions if needed
    let strlen = ctx.module.get_function(ffi_names::STRLEN).unwrap_or_else(|| {
        let i64_ty = ctx.context.i64_type();
        let ptr_ty = ctx.context.i8_type().ptr_type(inkwell::AddressSpace::default());
        let fn_ty = i64_ty.fn_type(&[ptr_ty.into()], false);
        ctx.module.add_function(ffi_names::STRLEN, fn_ty, None)
    });
    
    let malloc = ctx.module.get_function(ffi_names::MALLOC).unwrap_or_else(|| {
        let i64_ty = ctx.context.i64_type();
        let ptr_ty = ctx.context.i8_type().ptr_type(inkwell::AddressSpace::default());
        let fn_ty = ptr_ty.fn_type(&[i64_ty.into()], false);
        ctx.module.add_function(ffi_names::MALLOC, fn_ty, None)
    });
    
    let memcpy = ctx.module.get_function(ffi_names::MEMCPY).unwrap_or_else(|| {
        let ptr_ty = ctx.context.i8_type().ptr_type(inkwell::AddressSpace::default());
        let i64_ty = ctx.context.i64_type();
        let fn_ty = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false);
        ctx.module.add_function(ffi_names::MEMCPY, fn_ty, None)
    });
    
    // Get lengths of both strings
    let len1 = ctx.builder
        .build_call(strlen, &[str1_ptr.into()], "len1")
        .ok()?
        .try_as_basic_value()
        .left()?
        .into_int_value();
    let len2 = ctx.builder
        .build_call(strlen, &[str2_ptr.into()], "len2")
        .ok()?
        .try_as_basic_value()
        .left()?
        .into_int_value();
    
    // Allocate len1 + len2 + 1 bytes
    let total_len = ctx.builder.build_int_add(len1, len2, "total").ok()?;
    let size = ctx.builder.build_int_add(
        total_len,
        ctx.context.i64_type().const_int(1, false),
        "size"
    ).ok()?;
    
    let result_ptr = ctx.builder
        .build_call(malloc, &[size.into()], "concat")
        .ok()?
        .try_as_basic_value()
        .left()?
        .into_pointer_value();
    
    // Copy first string
    ctx.builder.build_call(memcpy, &[result_ptr.into(), str1_ptr.into(), len1.into()], "").ok()?;
    
    // Copy second string (including null terminator)
    let dest2 = unsafe {
        ctx.builder.build_in_bounds_gep(
            ctx.context.i8_type(),
            result_ptr,
            &[len1],
            "dest2"
        ).ok()?
    };
    let len2_plus_null = ctx.builder.build_int_add(
        len2,
        ctx.context.i64_type().const_int(1, false),
        "len2p1"
    ).ok()?;
    ctx.builder.build_call(memcpy, &[dest2.into(), str2_ptr.into(), len2_plus_null.into()], "").ok()?;
    
    Some(result_ptr.into())
}

/// Emit unary operation.
fn emit_unaryop<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    op: UnaryOp,
    val: BasicValueEnum<'ctx>,
) -> Option<BasicValueEnum<'ctx>> {
    match op {
        UnaryOp::Neg => {
            if val.is_int_value() {
                Some(ctx.builder.build_int_neg(val.into_int_value(), "neg").ok()?.into())
            } else if val.is_float_value() {
                Some(ctx.builder.build_float_neg(val.into_float_value(), "fneg").ok()?.into())
            } else {
                None
            }
        }
        UnaryOp::Not => {
            if val.is_int_value() {
                Some(ctx.builder.build_not(val.into_int_value(), "not").ok()?.into())
            } else {
                None
            }
        }
    }
}
