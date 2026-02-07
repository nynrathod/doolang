//! Arithmetic Instruction Handler
//!
//! Handles: BinaryOp, UnaryOp

use super::InstructionHandler;
use crate::context::CodegenContext;
use doo_core::constants::ffi_names;
use doo_core::doo_debug;
use doo_mir::{BinaryOp, MirConst, MirInstr, MirInstrKind, MirOperand, UnaryOp};
use inkwell::values::BasicValueEnum;
use inkwell::FloatPredicate;
use inkwell::IntPredicate;

/// Arithmetic instruction handler.
pub struct ArithmeticHandler;

impl<'ctx> InstructionHandler<'ctx> for ArithmeticHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(
            instr.kind,
            MirInstrKind::BinaryOp { .. } | MirInstrKind::UnaryOp { .. }
        )
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::BinaryOp { dest, op, lhs, rhs } => {
                if std::env::var("DOO_DEBUG").is_ok() {
                    doo_debug!("CODEGEN", "BinaryOp: {} = {:?} {:?} {:?}",
                        dest, lhs, op, rhs
                    );
                }
                let lhs_val = operand_to_value(ctx, lhs);
                if lhs_val.is_none() && std::env::var("DOO_DEBUG").is_ok() {
                    doo_debug!("CODEGEN", "BinaryOp: lhs operand_to_value failed for {:?}",
                        lhs
                    );
                    return None;
                }
                let lhs_val = lhs_val?;

                let rhs_val = operand_to_value(ctx, rhs);
                if rhs_val.is_none() && std::env::var("DOO_DEBUG").is_ok() {
                    doo_debug!("CODEGEN", "BinaryOp: rhs operand_to_value failed for {:?}",
                        rhs
                    );
                    return None;
                }
                let rhs_val = rhs_val?;

                let result = emit_binop(ctx, *op, lhs_val, rhs_val);
                if result.is_none() && std::env::var("DOO_DEBUG").is_ok() {
                    doo_debug!("CODEGEN", "BinaryOp: emit_binop failed for {:?} {:?} {:?}",
                        lhs_val, op, rhs_val
                    );
                    return None;
                }
                let result = result?;
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
            // First try to get as a value (local variable, temp, etc.)
            if let Some(val) = ctx.get_value(name) {
                return Some(val);
            }
            // Fall back to function reference - convert function to pointer value
            if let Some(func) = ctx.get_function(name) {
                return Some(func.as_global_value().as_pointer_value().into());
            }
            None
        }
        MirOperand::FuncRef(name) => {
            // FuncRef is an explicit function reference - convert to pointer
            if let Some(func) = ctx.get_function(name) {
                return Some(func.as_global_value().as_pointer_value().into());
            }
            None
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
    // Handle string concatenation
    // Case 1: Both operands are pointers (strings)
    if matches!(op, BinaryOp::Concat)
        || (matches!(op, BinaryOp::Add) && lhs.is_pointer_value() && rhs.is_pointer_value())
    {
        return emit_string_concat(ctx, lhs, rhs);
    }

    // Case 2: String + Non-String (interpolation type conversion)
    if matches!(op, BinaryOp::Add | BinaryOp::Concat)
        && lhs.is_pointer_value()
        && !rhs.is_pointer_value()
    {
        let rhs_str = value_to_string(ctx, rhs)?;
        return emit_string_concat(ctx, lhs, rhs_str);
    }

    // Case 3: Non-String + String (interpolation type conversion)
    if matches!(op, BinaryOp::Add | BinaryOp::Concat)
        && !lhs.is_pointer_value()
        && rhs.is_pointer_value()
    {
        let lhs_str = value_to_string(ctx, lhs)?;
        return emit_string_concat(ctx, lhs_str, rhs);
    }

    // Handle pointer comparison with nil (null check)
    // Pointer == nil or Pointer != nil
    if matches!(op, BinaryOp::Eq | BinaryOp::Ne) && lhs.is_pointer_value() && rhs.is_int_value() {
        // rhs is probably nil (represented as i64 0)
        let lhs_ptr = lhs.into_pointer_value();
        let result = if matches!(op, BinaryOp::Eq) {
            ctx.builder.build_is_null(lhs_ptr, "ptr_eq_nil").ok()?
        } else {
            ctx.builder.build_is_not_null(lhs_ptr, "ptr_ne_nil").ok()?
        };
        return Some(result.into());
    }

    // Handle pointer comparison with nil (reversed operands)
    // nil == Pointer or nil != Pointer
    if matches!(op, BinaryOp::Eq | BinaryOp::Ne) && lhs.is_int_value() && rhs.is_pointer_value() {
        let rhs_ptr = rhs.into_pointer_value();
        let result = if matches!(op, BinaryOp::Eq) {
            ctx.builder.build_is_null(rhs_ptr, "nil_eq_ptr").ok()?
        } else {
            ctx.builder.build_is_not_null(rhs_ptr, "nil_ne_ptr").ok()?
        };
        return Some(result.into());
    }

    // Handle string comparison (both pointers) using strcmp
    // This is CRITICAL for error handling functions that compare strings
    if matches!(op, BinaryOp::Eq | BinaryOp::Ne) && lhs.is_pointer_value() && rhs.is_pointer_value()
    {
        let lhs_ptr = lhs.into_pointer_value();
        let rhs_ptr = rhs.into_pointer_value();

        // Debug: show which block we're emitting to
        if std::env::var("DOO_DEBUG").is_ok() {
            if let Some(bb) = ctx.builder.get_insert_block() {
                doo_debug!("CODEGEN", "strcmp emitting to block: {:?}",
                    bb.get_name().to_str()
                );
            }
        }

        // Get or declare strcmp
        let strcmp = ctx
            .module
            .get_function(ffi_names::STRCMP)
            .unwrap_or_else(|| {
                let i32_ty = ctx.context.i32_type();
                let ptr_ty = ctx.context.ptr_type(inkwell::AddressSpace::default());
                let fn_ty = i32_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
                ctx.module.add_function(ffi_names::STRCMP, fn_ty, None)
            });

        // Call strcmp
        let strcmp_result = ctx
            .builder
            .build_call(strcmp, &[lhs_ptr.into(), rhs_ptr.into()], "strcmp_result")
            .ok()?
            .try_as_basic_value()
            .basic()?
            .into_int_value();

        // strcmp returns 0 if equal
        let result = if matches!(op, BinaryOp::Eq) {
            ctx.builder
                .build_int_compare(
                    IntPredicate::EQ,
                    strcmp_result,
                    ctx.context.i32_type().const_zero(),
                    "str_eq",
                )
                .ok()?
        } else {
            ctx.builder
                .build_int_compare(
                    IntPredicate::NE,
                    strcmp_result,
                    ctx.context.i32_type().const_zero(),
                    "str_ne",
                )
                .ok()?
        };
        return Some(result.into());
    }

    // Handle enum comparison (both struct values - compare tags at index 0)
    if matches!(op, BinaryOp::Eq | BinaryOp::Ne)
        && lhs.is_struct_value()
        && rhs.is_struct_value()
    {
        let lhs_tag = ctx
            .builder
            .build_extract_value(lhs.into_struct_value(), 0, "lhs_tag")
            .ok()?
            .into_int_value();
        let rhs_tag = ctx
            .builder
            .build_extract_value(rhs.into_struct_value(), 0, "rhs_tag")
            .ok()?
            .into_int_value();
        let result = if matches!(op, BinaryOp::Eq) {
            ctx.builder
                .build_int_compare(IntPredicate::EQ, lhs_tag, rhs_tag, "enum_eq")
                .ok()?
        } else {
            ctx.builder
                .build_int_compare(IntPredicate::NE, lhs_tag, rhs_tag, "enum_ne")
                .ok()?
        };
        return Some(result.into());
    }

    if lhs.is_int_value() && rhs.is_int_value() {
        let lhs_int = lhs.into_int_value();
        let rhs_int = rhs.into_int_value();

        let result = match op {
            BinaryOp::Add => ctx
                .builder
                .build_int_add(lhs_int, rhs_int, "add")
                .ok()?
                .into(),
            BinaryOp::Sub => ctx
                .builder
                .build_int_sub(lhs_int, rhs_int, "sub")
                .ok()?
                .into(),
            BinaryOp::Mul => ctx
                .builder
                .build_int_mul(lhs_int, rhs_int, "mul")
                .ok()?
                .into(),
            BinaryOp::Div => ctx
                .builder
                .build_int_signed_div(lhs_int, rhs_int, "div")
                .ok()?
                .into(),
            BinaryOp::Mod => ctx
                .builder
                .build_int_signed_rem(lhs_int, rhs_int, "mod")
                .ok()?
                .into(),
            BinaryOp::Eq => ctx
                .builder
                .build_int_compare(IntPredicate::EQ, lhs_int, rhs_int, "eq")
                .ok()?
                .into(),
            BinaryOp::Ne => ctx
                .builder
                .build_int_compare(IntPredicate::NE, lhs_int, rhs_int, "ne")
                .ok()?
                .into(),
            BinaryOp::Lt => ctx
                .builder
                .build_int_compare(IntPredicate::SLT, lhs_int, rhs_int, "lt")
                .ok()?
                .into(),
            BinaryOp::Le => ctx
                .builder
                .build_int_compare(IntPredicate::SLE, lhs_int, rhs_int, "le")
                .ok()?
                .into(),
            BinaryOp::Gt => ctx
                .builder
                .build_int_compare(IntPredicate::SGT, lhs_int, rhs_int, "gt")
                .ok()?
                .into(),
            BinaryOp::Ge => ctx
                .builder
                .build_int_compare(IntPredicate::SGE, lhs_int, rhs_int, "ge")
                .ok()?
                .into(),
            BinaryOp::And => ctx.builder.build_and(lhs_int, rhs_int, "and").ok()?.into(),
            BinaryOp::Or => ctx.builder.build_or(lhs_int, rhs_int, "or").ok()?.into(),
            BinaryOp::Concat => return None, // Handled above
        };
        Some(result)
    } else if lhs.is_float_value() && rhs.is_float_value() {
        // Both floats: perform float operation
        let lhs_float = lhs.into_float_value();
        let rhs_float = rhs.into_float_value();
        emit_float_binop(ctx, op, lhs_float, rhs_float)
    } else if lhs.is_float_value() && rhs.is_int_value() {
        // Mixed: float op int → convert int to float, perform float op
        let lhs_float = lhs.into_float_value();
        let rhs_float = ctx
            .builder
            .build_signed_int_to_float(rhs.into_int_value(), ctx.context.f64_type(), "int_to_f64")
            .ok()?;

        emit_float_binop(ctx, op, lhs_float, rhs_float)
    } else if lhs.is_int_value() && rhs.is_float_value() {
        // Mixed: int op float → convert int to float, perform float op
        let lhs_float = ctx
            .builder
            .build_signed_int_to_float(lhs.into_int_value(), ctx.context.f64_type(), "int_to_f64")
            .ok()?;
        let rhs_float = rhs.into_float_value();

        emit_float_binop(ctx, op, lhs_float, rhs_float)
    } else {
        None
    }
}

/// Emit float binary operation (extracted for reuse).
fn emit_float_binop<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    op: BinaryOp,
    lhs_float: inkwell::values::FloatValue<'ctx>,
    rhs_float: inkwell::values::FloatValue<'ctx>,
) -> Option<BasicValueEnum<'ctx>> {
    let result = match op {
        BinaryOp::Add => ctx
            .builder
            .build_float_add(lhs_float, rhs_float, "fadd")
            .ok()?
            .into(),
        BinaryOp::Sub => ctx
            .builder
            .build_float_sub(lhs_float, rhs_float, "fsub")
            .ok()?
            .into(),
        BinaryOp::Mul => ctx
            .builder
            .build_float_mul(lhs_float, rhs_float, "fmul")
            .ok()?
            .into(),
        BinaryOp::Div => ctx
            .builder
            .build_float_div(lhs_float, rhs_float, "fdiv")
            .ok()?
            .into(),
        BinaryOp::Mod => {
            // Floating-point modulo using LLVM's frem instruction
            ctx.builder
                .build_float_rem(lhs_float, rhs_float, "fmod")
                .ok()?
                .into()
        }
        BinaryOp::Eq => ctx
            .builder
            .build_float_compare(FloatPredicate::OEQ, lhs_float, rhs_float, "feq")
            .ok()?
            .into(),
        BinaryOp::Ne => ctx
            .builder
            .build_float_compare(FloatPredicate::ONE, lhs_float, rhs_float, "fne")
            .ok()?
            .into(),
        BinaryOp::Lt => ctx
            .builder
            .build_float_compare(FloatPredicate::OLT, lhs_float, rhs_float, "flt")
            .ok()?
            .into(),
        BinaryOp::Le => ctx
            .builder
            .build_float_compare(FloatPredicate::OLE, lhs_float, rhs_float, "fle")
            .ok()?
            .into(),
        BinaryOp::Gt => ctx
            .builder
            .build_float_compare(FloatPredicate::OGT, lhs_float, rhs_float, "fgt")
            .ok()?
            .into(),
        BinaryOp::Ge => ctx
            .builder
            .build_float_compare(FloatPredicate::OGE, lhs_float, rhs_float, "fge")
            .ok()?
            .into(),
        _ => return None,
    };
    Some(result)
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
    let strlen = ctx
        .module
        .get_function(ffi_names::STRLEN)
        .unwrap_or_else(|| {
            let i64_ty = ctx.context.i64_type();
            let ptr_ty = ctx
                .context
                .i8_type()
                .ptr_type(inkwell::AddressSpace::default());
            let fn_ty = i64_ty.fn_type(&[ptr_ty.into()], false);
            ctx.module.add_function(ffi_names::STRLEN, fn_ty, None)
        });

    let malloc = ctx
        .module
        .get_function(ffi_names::MALLOC)
        .unwrap_or_else(|| {
            let i64_ty = ctx.context.i64_type();
            let ptr_ty = ctx
                .context
                .i8_type()
                .ptr_type(inkwell::AddressSpace::default());
            let fn_ty = ptr_ty.fn_type(&[i64_ty.into()], false);
            ctx.module.add_function(ffi_names::MALLOC, fn_ty, None)
        });

    let memcpy = ctx
        .module
        .get_function(ffi_names::MEMCPY)
        .unwrap_or_else(|| {
            let ptr_ty = ctx
                .context
                .i8_type()
                .ptr_type(inkwell::AddressSpace::default());
            let i64_ty = ctx.context.i64_type();
            let fn_ty = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false);
            ctx.module.add_function(ffi_names::MEMCPY, fn_ty, None)
        });

    // Get lengths of both strings
    let len1 = ctx
        .builder
        .build_call(strlen, &[str1_ptr.into()], "len1")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_int_value();
    let len2 = ctx
        .builder
        .build_call(strlen, &[str2_ptr.into()], "len2")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_int_value();

    // Allocate len1 + len2 + 1 bytes
    let total_len = ctx.builder.build_int_add(len1, len2, "total").ok()?;
    let size = ctx
        .builder
        .build_int_add(
            total_len,
            ctx.context.i64_type().const_int(1, false),
            "size",
        )
        .ok()?;

    let result_ptr = ctx
        .builder
        .build_call(malloc, &[size.into()], "concat")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_pointer_value();

    // Copy first string
    ctx.builder
        .build_call(
            memcpy,
            &[result_ptr.into(), str1_ptr.into(), len1.into()],
            "",
        )
        .ok()?;

    // Copy second string (including null terminator)
    let dest2 = unsafe {
        ctx.builder
            .build_in_bounds_gep(ctx.context.i8_type(), result_ptr, &[len1], "dest2")
            .ok()?
    };
    let len2_plus_null = ctx
        .builder
        .build_int_add(len2, ctx.context.i64_type().const_int(1, false), "len2p1")
        .ok()?;
    ctx.builder
        .build_call(
            memcpy,
            &[dest2.into(), str2_ptr.into(), len2_plus_null.into()],
            "",
        )
        .ok()?;

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
                Some(
                    ctx.builder
                        .build_int_neg(val.into_int_value(), "neg")
                        .ok()?
                        .into(),
                )
            } else if val.is_float_value() {
                Some(
                    ctx.builder
                        .build_float_neg(val.into_float_value(), "fneg")
                        .ok()?
                        .into(),
                )
            } else {
                None
            }
        }
        UnaryOp::Not => {
            if val.is_int_value() {
                Some(
                    ctx.builder
                        .build_not(val.into_int_value(), "not")
                        .ok()?
                        .into(),
                )
            } else {
                None
            }
        }
    }
}
/// Convert a value to a string for interpolation.
/// Uses snprintf to format integers and floats.
fn value_to_string<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
) -> Option<BasicValueEnum<'ctx>> {
    // If it's already a pointer (string), just return it
    if val.is_pointer_value() {
        return Some(val);
    }

    let snprintf = ctx
        .module
        .get_function(ffi_names::SNPRINTF)
        .unwrap_or_else(|| {
            let i32_ty = ctx.context.i32_type();
            let i64_ty = ctx.context.i64_type();
            let ptr_ty = ctx
                .context
                .i8_type()
                .ptr_type(inkwell::AddressSpace::default());
            let fn_ty = i32_ty.fn_type(&[ptr_ty.into(), i64_ty.into(), ptr_ty.into()], true);
            ctx.module.add_function(ffi_names::SNPRINTF, fn_ty, None)
        });

    let malloc = ctx
        .module
        .get_function(ffi_names::MALLOC)
        .unwrap_or_else(|| {
            let i64_ty = ctx.context.i64_type();
            let ptr_ty = ctx
                .context
                .i8_type()
                .ptr_type(inkwell::AddressSpace::default());
            let fn_ty = ptr_ty.fn_type(&[i64_ty.into()], false);
            ctx.module.add_function(ffi_names::MALLOC, fn_ty, None)
        });

    // Determine format string and value based on type
    let (fmt_str, args): (&str, Vec<BasicValueEnum>) = if val.is_int_value() {
        let int_val = val.into_int_value();
        // Check if it's a boolean (i1) or integer
        if int_val.get_type().get_bit_width() == 1 {
            // Boolean - need to convert to "true" or "false"
            // For now, just use %d which will output 0 or 1
            // TODO: Proper "true"/"false" string output
            ("%d", vec![val])
        } else {
            ("%lld", vec![val])
        }
    } else if val.is_float_value() {
        ("%g", vec![val]) // Use %g for more compact float representation
    } else {
        // Unknown type, try as integer
        ("%lld", vec![val])
    };

    let fmt = ctx.const_string(fmt_str);
    let null_buf = ctx
        .context
        .i8_type()
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    let zero_len = ctx.context.i64_type().const_zero();

    // Call snprintf(null, 0, fmt, val) to get required length
    use inkwell::values::BasicMetadataValueEnum;
    let mut call_args: Vec<BasicMetadataValueEnum> =
        vec![null_buf.into(), zero_len.into(), fmt.into()];
    call_args.extend(args.iter().map(|v| BasicMetadataValueEnum::from(*v)));

    let len_i32 = ctx
        .builder
        .build_call(snprintf, &call_args, "len")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_int_value();

    let len_i64 = ctx
        .builder
        .build_int_z_extend(len_i32, ctx.context.i64_type(), "len64")
        .ok()?;
    let size = ctx
        .builder
        .build_int_add(len_i64, ctx.context.i64_type().const_int(1, false), "size")
        .ok()?;

    // Allocate buffer
    let buf = ctx
        .builder
        .build_call(malloc, &[size.into()], "str_buf")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_pointer_value();

    // Format the value into the buffer
    let mut print_args: Vec<BasicMetadataValueEnum> = vec![buf.into(), size.into(), fmt.into()];
    print_args.extend(args.iter().map(|v| BasicMetadataValueEnum::from(*v)));

    ctx.builder.build_call(snprintf, &print_args, "fmt").ok()?;

    Some(buf.into())
}
