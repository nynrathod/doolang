//! Arithmetic Instruction Handler
//!
//! Handles: BinaryOp, UnaryOp

use super::InstructionHandler;
use crate::context::CodegenContext;
use crate::utils::null_coerce_str;
use doo_core::constants::ffi_names;
use doo_core::doo_debug;
use doo_mir::sym::resolve;
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
                if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {}
                let lhs_val = operand_to_value(ctx, lhs);
                if lhs_val.is_none()
                    && std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok()
                {
                    return None;
                }
                let lhs_val = lhs_val?;

                let rhs_val = operand_to_value(ctx, rhs);
                if rhs_val.is_none()
                    && std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok()
                {
                    return None;
                }
                let rhs_val = rhs_val?;

                let result = emit_binop(ctx, *op, lhs_val, rhs_val);
                if result.is_none()
                    && std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok()
                {
                    return None;
                }
                let result = result?;
                ctx.set_temp(&resolve(*dest), result);
                Some(result)
            }

            MirInstrKind::UnaryOp { dest, op, operand } => {
                let val = operand_to_value(ctx, operand)?;
                let result = emit_unaryop(ctx, *op, val)?;
                ctx.set_temp(&resolve(*dest), result);
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
            let name_str = resolve(*name);
            // First try to get as a value (local variable, temp, etc.)
            if let Some(val) = ctx.get_value(&name_str) {
                return Some(val);
            }
            // Fall back to function reference - convert function to pointer value
            if let Some(func) = ctx.get_function(&name_str) {
                return Some(func.as_global_value().as_pointer_value().into());
            }
            None
        }
        MirOperand::FuncRef(name) => {
            let name_str = resolve(*name);
            // FuncRef is an explicit function reference - convert to pointer
            if let Some(func) = ctx.get_function(&name_str) {
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
    // Handle nil coalescing: a ?? b
    // If a is nil (null pointer or zero), return b; otherwise return a.
    if op == BinaryOp::NullCoalesce {
        // Same type: use select for efficiency
        if lhs.get_type() == rhs.get_type() {
            let is_nil = if lhs.is_pointer_value() {
                let ptr = lhs.into_pointer_value();
                let is_null = ctx.builder.build_is_null(ptr, "coalesce_nullptr").ok()?;
                // Also check for boxed nil: pointer may be valid but point to i64 0
                let loaded = ctx
                    .builder
                    .build_load(ctx.i64_type(), ptr, "coalesce_load_chk")
                    .ok()?;
                let is_boxed_nil = ctx
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        loaded.into_int_value(),
                        ctx.i64_type().const_zero(),
                        "coalesce_boxed_nil",
                    )
                    .ok()?;
                ctx.builder
                    .build_or(is_null, is_boxed_nil, "coalesce_is_nil")
                    .ok()?
            } else if lhs.is_int_value() {
                let lhs_int = lhs.into_int_value();
                let zero = lhs_int.get_type().const_zero();
                ctx.builder
                    .build_int_compare(IntPredicate::EQ, lhs_int, zero, "coalesce_nil")
                    .ok()?
            } else if lhs.is_struct_value() {
                let lhs_struct = lhs.into_struct_value();
                let first = ctx
                    .builder
                    .build_extract_value(lhs_struct, 0, "coalesce_tag")
                    .ok()?;
                if first.is_int_value() {
                    let tag = first.into_int_value();
                    let zero = tag.get_type().const_zero();
                    ctx.builder
                        .build_int_compare(IntPredicate::EQ, tag, zero, "coalesce_nil")
                        .ok()?
                } else {
                    return Some(lhs); // can't determine nil-ness
                }
            } else {
                return Some(lhs); // unknown type
            };
            return Some(
                ctx.builder
                    .build_select(is_nil, rhs, lhs, "coalesce_result")
                    .ok()?,
            );
        }

        // Type mismatch: need branching with phi.
        // Determine nil-ness of lhs.
        let is_nil = if lhs.is_pointer_value() {
            let ptr = lhs.into_pointer_value();
            let is_null = ctx.builder.build_is_null(ptr, "coalesce_nullptr").ok()?;
            // Also check for boxed nil: the pointer may be valid (heap-allocated)
            // but point to an i64 0 (from `Ok nil` wrapping a raw nil constant).
            // Load the first 8 bytes as i64; if zero, treat as nil.
            let loaded = ctx
                .builder
                .build_load(ctx.i64_type(), ptr, "coalesce_load_nullchk")
                .ok()?;
            let is_boxed_nil = ctx
                .builder
                .build_int_compare(
                    IntPredicate::EQ,
                    loaded.into_int_value(),
                    ctx.i64_type().const_zero(),
                    "coalesce_boxed_nil",
                )
                .ok()?;
            ctx.builder
                .build_or(is_null, is_boxed_nil, "coalesce_is_nil")
                .ok()?
        } else if lhs.is_int_value() {
            let lhs_int = lhs.into_int_value();
            let zero = lhs_int.get_type().const_zero();
            ctx.builder
                .build_int_compare(IntPredicate::EQ, lhs_int, zero, "coalesce_nil")
                .ok()?
        } else if lhs.is_struct_value() {
            let lhs_struct = lhs.into_struct_value();
            let first = ctx
                .builder
                .build_extract_value(lhs_struct, 0, "coalesce_tag")
                .ok()?;
            if first.is_int_value() {
                let tag = first.into_int_value();
                let zero = tag.get_type().const_zero();
                ctx.builder
                    .build_int_compare(IntPredicate::EQ, tag, zero, "coalesce_nil")
                    .ok()?
            } else {
                return Some(lhs);
            }
        } else {
            return Some(lhs);
        };

        let current_fn = ctx.current_function()?;
        let then_bb = ctx.context.append_basic_block(current_fn, "coalesce_nil");
        let else_bb = ctx.context.append_basic_block(current_fn, "coalesce_val");
        let merge_bb = ctx.context.append_basic_block(current_fn, "coalesce_merge");

        ctx.builder
            .build_conditional_branch(is_nil, then_bb, else_bb)
            .ok()?;

        // Build a null value of the target type for phi compatibility
        let null_of_rhs_type: BasicValueEnum<'ctx> = match rhs.get_type() {
            inkwell::types::BasicTypeEnum::PointerType(pt) => pt.const_null().into(),
            inkwell::types::BasicTypeEnum::IntType(it) => it.const_zero().into(),
            inkwell::types::BasicTypeEnum::StructType(st) => st.const_zero().into(),
            _ => rhs, // fallback
        };

        // Then block: lhs is nil, use rhs directly
        ctx.builder.position_at_end(then_bb);
        ctx.builder.build_unconditional_branch(merge_bb).ok()?;

        // Else block: lhs is valid (non-nil) — convert to rhs type if needed
        ctx.builder.position_at_end(else_bb);
        let else_val = if lhs.get_type() == rhs.get_type() {
            lhs
        } else if lhs.is_pointer_value() && rhs.is_struct_value() {
            // Load struct value from pointer to match struct value type
            if let inkwell::types::BasicTypeEnum::StructType(st) = rhs.get_type() {
                ctx.builder
                    .build_load(st, lhs.into_pointer_value(), "coalesce_load")
                    .ok()?
            } else {
                lhs
            }
        } else if lhs.is_struct_value() && rhs.is_pointer_value() {
            // LHS is an Optional/Result struct — extract the payload pointer (field 1)
            let lhs_struct = lhs.into_struct_value();
            if let Ok(payload) = ctx
                .builder
                .build_extract_value(lhs_struct, 1, "coalesce_payload")
            {
                if payload.is_pointer_value() {
                    payload
                } else {
                    // Fallback: alloca + store
                    let alloca = ctx
                        .builder
                        .build_alloca(lhs_struct.get_type(), "coalesce_alloca")
                        .ok()?;
                    ctx.builder.build_store(alloca, lhs).ok()?;
                    alloca.into()
                }
            } else {
                lhs
            }
        } else {
            // Types differ (e.g. nil i64 vs ptr) — this path unreachable at runtime,
            // but LLVM needs a type-compatible value for the phi.
            null_of_rhs_type
        };
        ctx.builder.build_unconditional_branch(merge_bb).ok()?;

        // Merge block: phi uses rhs type (the dominant type)
        ctx.builder.position_at_end(merge_bb);
        let phi = ctx.builder.build_phi(rhs.get_type(), "coalesce_phi").ok()?;
        phi.add_incoming(&[(&rhs, then_bb), (&else_val, else_bb)]);

        return Some(phi.as_basic_value());
    }

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
        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
            if let Some(bb) = ctx.builder.get_insert_block() {}
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

        // Null-coerce both operands: LLVM infers nonnull+dereferenceable(1) on
        // strcmp args because it recognizes the function name. If either operand
        // is null (e.g., from a try-expression error path or uninitialized var),
        // the nonnull annotation triggers UB-based miscompilation.
        let safe_lhs = null_coerce_str(ctx, lhs_ptr);
        let safe_rhs = null_coerce_str(ctx, rhs_ptr);

        // Call strcmp
        let strcmp_result = ctx
            .builder
            .build_call(strcmp, &[safe_lhs.into(), safe_rhs.into()], "strcmp_result")
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
    if matches!(op, BinaryOp::Eq | BinaryOp::Ne) && lhs.is_struct_value() && rhs.is_struct_value() {
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
        let mut lhs_int = lhs.into_int_value();
        let mut rhs_int = rhs.into_int_value();

        // Normalize int widths: when comparing i64 vs i1 (e.g., Bool variable vs Bool literal),
        // zero-extend the smaller operand to match the larger one.
        let lhs_width = lhs_int.get_type().get_bit_width();
        let rhs_width = rhs_int.get_type().get_bit_width();
        if lhs_width != rhs_width {
            if lhs_width > rhs_width {
                rhs_int = ctx
                    .builder
                    .build_int_z_extend(rhs_int, lhs_int.get_type(), "zext")
                    .ok()?;
            } else {
                lhs_int = ctx
                    .builder
                    .build_int_z_extend(lhs_int, rhs_int.get_type(), "zext")
                    .ok()?;
            }
        }

        let result = match op {
            BinaryOp::Add => {
                emit_overflow_guard(ctx, lhs_int, rhs_int, "add");
                ctx.builder
                    .build_int_add(lhs_int, rhs_int, "add")
                    .ok()?
                    .into()
            }
            BinaryOp::Sub => {
                emit_overflow_guard(ctx, lhs_int, rhs_int, "sub");
                ctx.builder
                    .build_int_sub(lhs_int, rhs_int, "sub")
                    .ok()?
                    .into()
            }
            BinaryOp::Mul => {
                emit_overflow_guard(ctx, lhs_int, rhs_int, "mul");
                ctx.builder
                    .build_int_mul(lhs_int, rhs_int, "mul")
                    .ok()?
                    .into()
            }
            BinaryOp::Div => {
                emit_div_zero_guard(ctx, rhs_int, "division");
                ctx.builder
                    .build_int_signed_div(lhs_int, rhs_int, "div")
                    .ok()?
                    .into()
            }
            BinaryOp::Mod => {
                emit_div_zero_guard(ctx, rhs_int, "modulo");
                ctx.builder
                    .build_int_signed_rem(lhs_int, rhs_int, "mod")
                    .ok()?
                    .into()
            }
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
            BinaryOp::BitXor => ctx.builder.build_xor(lhs_int, rhs_int, "xor").ok()?.into(),
            BinaryOp::Concat => return None,       // Handled above
            BinaryOp::NullCoalesce => return None, // Handled above
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
/// SAFETY: Both operands are null-coerced to empty strings before use,
/// preventing undefined behavior when LLVM infers nonnull on strlen args.
fn emit_string_concat<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    lhs: BasicValueEnum<'ctx>,
    rhs: BasicValueEnum<'ctx>,
) -> Option<BasicValueEnum<'ctx>> {
    // Null-coerce both string pointers to prevent UB: if a string pointer
    // is null (e.g., from MapGet not-found default), replace with empty string.
    // This is critical because LLVM infers nonnull+dereferenceable(1) on strlen
    // arguments, and a null input would be UB that the optimizer can exploit.
    let str1_ptr = null_coerce_str(ctx, lhs.into_pointer_value());
    let str2_ptr = null_coerce_str(ctx, rhs.into_pointer_value());

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
            .build_gep(ctx.context.i8_type(), result_ptr, &[len1], "dest2")
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
                let int_val = val.into_int_value();
                let int_type = int_val.get_type();
                // Logical NOT for all integer widths: value == 0
                // Works correctly for Bool (i8) and any method returning
                // bool-like values in wider types (e.g. isEmpty returning i32)
                let zero = int_type.const_zero();
                let cmp = ctx
                    .builder
                    .build_int_compare(inkwell::IntPredicate::EQ, int_val, zero, "lnot")
                    .ok()?;
                // Extend i1 result back to original type
                Some(
                    ctx.builder
                        .build_int_z_extend(cmp, int_type, "lnot_ext")
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
/// Uses snprintf for ints, doo_format_float (ryu) for floats.
fn value_to_string<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
) -> Option<BasicValueEnum<'ctx>> {
    // If it's already a pointer (string), just return it
    if val.is_pointer_value() {
        return Some(val);
    }

    // For floats, use doo_format_float (ryu) for clean shortest representation
    if val.is_float_value() {
        let ptr_ty = ctx
            .context
            .i8_type()
            .ptr_type(inkwell::AddressSpace::default());
        let f64_ty = ctx.f64_type();
        let format_fn = ctx
            .module
            .get_function("doo_format_float")
            .unwrap_or_else(|| {
                let fn_ty = ptr_ty.fn_type(&[f64_ty.into()], false);
                ctx.module
                    .add_function("doo_format_float", fn_ty, None)
            });
        let result = ctx
            .builder
            .build_call(format_fn, &[val.into()], "fmt_float")
            .ok()?
            .try_as_basic_value()
            .basic()?;
        return Some(result);
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

    // Determine format string and value based on type (floats handled above via ryu)
    let (fmt_str, args): (&str, Vec<BasicValueEnum>) = if val.is_int_value() {
        let int_val = val.into_int_value();
        // Check if it's a boolean (i1 or i8) or integer
        if int_val.get_type().get_bit_width() <= 8 {
            // Boolean (i8 for C ABI, or i1 from comparison) - output 0 or 1
            ("%d", vec![val])
        } else {
            ("%lld", vec![val])
        }
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

/// Emit integer overflow detection guard (debug mode only).
/// Uses LLVM's signed overflow intrinsics (llvm.sadd.with.overflow, etc.)
/// to detect overflow at runtime and abort with a diagnostic message.
/// Only active when DOO_OVERFLOW_CHECKS=1 or DOO_DEBUG is set.
fn emit_overflow_guard<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    lhs: inkwell::values::IntValue<'ctx>,
    rhs: inkwell::values::IntValue<'ctx>,
    op_name: &str,
) {
    // Only emit overflow checks if enabled via env var
    let checks_enabled = std::env::var("DOO_OVERFLOW_CHECKS").is_ok()
        || std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok();
    if !checks_enabled {
        return;
    }

    // Only check i64 operations (skip i1/i8/i32 comparisons)
    if lhs.get_type().get_bit_width() != 64 {
        return;
    }

    let current_fn = match ctx.builder.get_insert_block().and_then(|b| b.get_parent()) {
        Some(f) => f,
        None => return,
    };

    // Build the overflow intrinsic name based on operation
    let intrinsic_name = match op_name {
        "add" => "llvm.sadd.with.overflow.i64",
        "sub" => "llvm.ssub.with.overflow.i64",
        "mul" => "llvm.smul.with.overflow.i64",
        _ => return,
    };

    // Declare the intrinsic if not already declared
    let i64_type = ctx.context.i64_type();
    let i1_type = ctx.context.bool_type();
    let overflow_result_type = ctx
        .context
        .struct_type(&[i64_type.into(), i1_type.into()], false);
    let intrinsic_fn_type =
        overflow_result_type.fn_type(&[i64_type.into(), i64_type.into()], false);

    let intrinsic_fn = ctx.module.get_function(intrinsic_name).unwrap_or_else(|| {
        ctx.module
            .add_function(intrinsic_name, intrinsic_fn_type, None)
    });

    // Call the intrinsic
    let result =
        match ctx
            .builder
            .build_call(intrinsic_fn, &[lhs.into(), rhs.into()], "overflow_check")
        {
            Ok(r) => r,
            Err(_) => return,
        };

    let result_val = match result.try_as_basic_value().basic() {
        Some(v) => v.into_struct_value(),
        None => return,
    };

    // Extract the overflow flag (index 1)
    let overflow_flag = match ctx
        .builder
        .build_extract_value(result_val, 1, "overflow_flag")
    {
        Ok(v) => v.into_int_value(),
        Err(_) => return,
    };

    let abort_bb = ctx.context.append_basic_block(current_fn, "overflow_abort");
    let cont_bb = ctx.context.append_basic_block(current_fn, "overflow_ok");

    let _ = ctx
        .builder
        .build_conditional_branch(overflow_flag, abort_bb, cont_bb);

    // Abort block: print error and exit(1)
    ctx.builder.position_at_end(abort_bb);
    let printf_type = ctx.context.i32_type().fn_type(
        &[ctx
            .context
            .i8_type()
            .ptr_type(inkwell::AddressSpace::default())
            .into()],
        true,
    );
    let printf = ctx
        .module
        .get_function(ffi_names::PRINTF)
        .unwrap_or_else(|| {
            ctx.module
                .add_function(ffi_names::PRINTF, printf_type, None)
        });

    let error_msg = ctx.const_string(&format!(
        "fatal: integer overflow in {} operation\n",
        op_name
    ));
    let _ = ctx
        .builder
        .build_call(printf, &[error_msg.into()], "print_overflow_err");

    let fflush_type = ctx.context.i32_type().fn_type(
        &[ctx
            .context
            .i8_type()
            .ptr_type(inkwell::AddressSpace::default())
            .into()],
        false,
    );
    let fflush = ctx
        .module
        .get_function(ffi_names::FFLUSH)
        .unwrap_or_else(|| {
            ctx.module
                .add_function(ffi_names::FFLUSH, fflush_type, None)
        });
    let null_ptr = ctx
        .context
        .i8_type()
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    let _ = ctx.builder.build_call(fflush, &[null_ptr.into()], "flush");

    let exit_type = ctx
        .context
        .void_type()
        .fn_type(&[ctx.context.i32_type().into()], false);
    let exit_fn = ctx
        .module
        .get_function(ffi_names::EXIT)
        .unwrap_or_else(|| ctx.module.add_function(ffi_names::EXIT, exit_type, None));
    let exit_code = ctx.context.i32_type().const_int(1, false);
    let _ = ctx
        .builder
        .build_call(exit_fn, &[exit_code.into()], "exit_overflow");
    let _ = ctx.builder.build_unreachable();

    ctx.builder.position_at_end(cont_bb);
}

/// Emit a runtime guard that aborts with an error message if `divisor` is zero.
/// Inserts a conditional branch: if divisor == 0 → print error + exit(1), else continue.
fn emit_div_zero_guard<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    divisor: inkwell::values::IntValue<'ctx>,
    op_name: &str,
) {
    let current_fn = match ctx.builder.get_insert_block().and_then(|b| b.get_parent()) {
        Some(f) => f,
        None => return,
    };

    let zero = divisor.get_type().const_zero();
    let is_zero = ctx
        .builder
        .build_int_compare(IntPredicate::EQ, divisor, zero, "div_zero_check")
        .unwrap_or_else(|_| return ctx.context.bool_type().const_zero());

    let abort_bb = ctx.context.append_basic_block(current_fn, "div_zero_abort");
    let cont_bb = ctx.context.append_basic_block(current_fn, "div_zero_ok");

    let _ = ctx
        .builder
        .build_conditional_branch(is_zero, abort_bb, cont_bb);

    // Abort block: print error and exit
    ctx.builder.position_at_end(abort_bb);
    let printf_type = ctx.context.i32_type().fn_type(
        &[ctx
            .context
            .i8_type()
            .ptr_type(inkwell::AddressSpace::default())
            .into()],
        true,
    );
    let printf = ctx
        .module
        .get_function(ffi_names::PRINTF)
        .unwrap_or_else(|| {
            ctx.module
                .add_function(ffi_names::PRINTF, printf_type, None)
        });

    let error_msg = ctx.const_string(&format!("fatal: {} by zero\n", op_name));
    let _ = ctx
        .builder
        .build_call(printf, &[error_msg.into()], "print_div_zero_err");

    // Flush
    let fflush_type = ctx.context.i32_type().fn_type(
        &[ctx
            .context
            .i8_type()
            .ptr_type(inkwell::AddressSpace::default())
            .into()],
        false,
    );
    let fflush = ctx
        .module
        .get_function(ffi_names::FFLUSH)
        .unwrap_or_else(|| {
            ctx.module
                .add_function(ffi_names::FFLUSH, fflush_type, None)
        });
    let null_ptr = ctx
        .context
        .i8_type()
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    let _ = ctx.builder.build_call(fflush, &[null_ptr.into()], "flush");

    // Exit(1)
    let exit_type = ctx
        .context
        .void_type()
        .fn_type(&[ctx.context.i32_type().into()], false);
    let exit_fn = ctx
        .module
        .get_function(ffi_names::EXIT)
        .unwrap_or_else(|| ctx.module.add_function(ffi_names::EXIT, exit_type, None));
    let exit_code = ctx.context.i32_type().const_int(1, false);
    let _ = ctx
        .builder
        .build_call(exit_fn, &[exit_code.into()], "exit_div_zero");
    let _ = ctx.builder.build_unreachable();

    // Continue block: safe to proceed
    ctx.builder.position_at_end(cont_bb);
}
