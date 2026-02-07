//! Cast Instruction Handler
//!
//! Handles: Cast (type conversions)

use super::InstructionHandler;
use crate::context::CodegenContext;
use doo_core::types::builtin;
use doo_mir::{MirConst, MirInstr, MirInstrKind, MirOperand};
use inkwell::values::BasicValueEnum;

/// Cast/conversion instruction handler.
pub struct CastHandler;

impl<'ctx> InstructionHandler<'ctx> for CastHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(instr.kind, MirInstrKind::Cast { .. })
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::Cast {
                dest,
                value,
                to_type,
            } => {
                let source_val = operand_to_value(ctx, value)?;
                let target_type_id = doo_core::types::TypeId::from(*to_type);

                let result: Option<BasicValueEnum<'ctx>> = if target_type_id == builtin::STR {
                    emit_string_from_value(ctx, source_val)
                } else if source_val.is_pointer_value() {
                    // String (or Ptr) -> Int/Float via FFI
                    // Assuming source is String for now if it's a pointer and not array/map/etc
                    // Ideally we check type_id of source but we only have operand.
                    // Operand type check is done in Analysis. Here we blindly cast.

                    use doo_core::constants::ffi_names;
                    if target_type_id == builtin::INT {
                        let cast_fn = ctx
                            .module
                            .get_function(ffi_names::DOO_CAST_STR_TO_INT)
                            .unwrap_or_else(|| {
                                let i64_ty = ctx.context.i64_type();
                                let ptr_ty = ctx
                                    .context
                                    .i8_type()
                                    .ptr_type(inkwell::AddressSpace::default());
                                let fn_ty = i64_ty.fn_type(&[ptr_ty.into()], false);
                                ctx.module
                                    .add_function(ffi_names::DOO_CAST_STR_TO_INT, fn_ty, None)
                            });

                        Some(
                            ctx.builder
                                .build_call(cast_fn, &[source_val.into()], "cast_str_int")
                                .ok()?
                                .try_as_basic_value()
                                .basic()?
                                .into(),
                        )
                    } else if target_type_id == builtin::FLOAT {
                        // Use centralized FFI function for str->float conversion
                        let cast_fn = ctx
                            .module
                            .get_function(ffi_names::DOO_CAST_STR_TO_FLOAT)
                            .unwrap_or_else(|| {
                                let f64_ty = ctx.context.f64_type();
                                let ptr_ty = ctx
                                    .context
                                    .i8_type()
                                    .ptr_type(inkwell::AddressSpace::default());
                                let fn_ty = f64_ty.fn_type(&[ptr_ty.into()], false);
                                ctx.module.add_function(
                                    ffi_names::DOO_CAST_STR_TO_FLOAT,
                                    fn_ty,
                                    None,
                                )
                            });
                        Some(
                            ctx.builder
                                .build_call(cast_fn, &[source_val.into()], "cast_str_float")
                                .ok()?
                                .try_as_basic_value()
                                .basic()?
                                .into(),
                        )
                    } else {
                        // Cast ptr -> ptr (e.g. any -> string, array -> any, etc)
                        // Just bitcast/pointercast
                        let target_llvm_ty = ctx.get_llvm_type(target_type_id);
                        if target_llvm_ty.is_pointer_type() {
                            Some(
                                ctx.builder
                                    .build_pointer_cast(
                                        source_val.into_pointer_value(),
                                        target_llvm_ty.into_pointer_type(),
                                        "ptr_cast",
                                    )
                                    .ok()?
                                    .into(),
                            )
                        } else {
                            Some(source_val) // No-op if types match or unknown
                        }
                    }
                } else if source_val.is_int_value() {
                    let int_val = source_val.into_int_value();
                    let source_bits = int_val.get_type().get_bit_width();

                    if target_type_id == builtin::FLOAT {
                        // Int/Bool -> Float: convert to double
                        // For bool (i1), zero-extend first to avoid sign issues
                        if source_bits == 1 {
                            // Bool -> Float: zero-extend to i64 first, then convert
                            let extended = ctx
                                .builder
                                .build_int_z_extend(int_val, ctx.i64_type(), "zext")
                                .ok()?;
                            Some(
                                ctx.builder
                                    .build_signed_int_to_float(extended, ctx.f64_type(), "cast")
                                    .ok()?
                                    .into(),
                            )
                        } else {
                            Some(
                                ctx.builder
                                    .build_signed_int_to_float(int_val, ctx.f64_type(), "cast")
                                    .ok()?
                                    .into(),
                            )
                        }
                    } else if target_type_id == builtin::BOOL {
                        // Int -> Bool: compare != 0
                        let zero = ctx.const_i64(0);
                        Some(
                            ctx.builder
                                .build_int_compare(inkwell::IntPredicate::NE, int_val, zero, "cast")
                                .ok()?
                                .into(),
                        )
                    } else if target_type_id == builtin::INT {
                        // Int/Bool -> Int: use zero-extend for bool, sign-extend for others
                        let target_bits = 64; // i64

                        if source_bits == 1 {
                            // Bool -> Int: MUST zero-extend (true=1, false=0)
                            Some(
                                ctx.builder
                                    .build_int_z_extend(int_val, ctx.i64_type(), "bool_to_int")
                                    .ok()?
                                    .into(),
                            )
                        } else if target_bits > source_bits {
                            Some(
                                ctx.builder
                                    .build_int_s_extend(int_val, ctx.i64_type(), "cast")
                                    .ok()?
                                    .into(),
                            )
                        } else if target_bits < source_bits {
                            Some(
                                ctx.builder
                                    .build_int_truncate(int_val, ctx.i64_type(), "cast")
                                    .ok()?
                                    .into(),
                            )
                        } else {
                            Some(source_val)
                        }
                    } else {
                        // Other int -> int conversions
                        Some(source_val)
                    }
                } else if source_val.is_float_value() {
                    let float_val = source_val.into_float_value();

                    if target_type_id == builtin::INT {
                        // Float -> Int: double to signed int
                        Some(
                            ctx.builder
                                .build_float_to_signed_int(float_val, ctx.i64_type(), "cast")
                                .ok()?
                                .into(),
                        )
                    } else if target_type_id == builtin::BOOL {
                        // Float -> Bool: compare != 0.0
                        let zero = ctx.f64_type().const_float(0.0);
                        Some(
                            ctx.builder
                                .build_float_compare(
                                    inkwell::FloatPredicate::ONE,
                                    float_val,
                                    zero,
                                    "cast",
                                )
                                .ok()?
                                .into(),
                        )
                    } else {
                        // Float -> Float: extend/truncate
                        Some(source_val)
                    }
                } else {
                    // Other types - pass through
                    Some(source_val)
                };

                if let Some(res) = result {
                    ctx.set_temp(dest, res);
                    Some(res)
                } else {
                    None
                }
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

fn emit_string_from_value<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
) -> Option<BasicValueEnum<'ctx>> {
    use doo_core::constants::ffi_names;

    // Check if it's already a string (pointer)
    // Note: This is weak checking, assumes pointers cast to str are already strings
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

    let (fmt_str, args) = if val.is_int_value() {
        ("%lld", vec![val])
    } else if val.is_float_value() {
        ("%f", vec![val])
    } else {
        // Bool or other? Bool is often i1 which is int value.
        // If TypeId check was here we could do "true"/"false".
        // But here we rely on LLVM value type.
        // Assuming int format for now.
        ("%lld", vec![val])
    };

    let fmt = ctx.const_string(fmt_str);
    let null_buf = ctx
        .context
        .i8_type()
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    let zero_len = ctx.context.i64_type().const_zero();

    // Call snprintf(null, 0, fmt, val) to get length
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
        .build_int_z_extend(len_i32, ctx.i64_type(), "len64")
        .ok()?;
    let size = ctx
        .builder
        .build_int_add(len_i64, ctx.i64_type().const_int(1, false), "size")
        .ok()?;

    // Malloc
    let buf = ctx
        .builder
        .build_call(malloc, &[size.into()], "buf")
        .ok()?
        .try_as_basic_value()
        .basic()?
        .into_pointer_value();

    // Actual format
    let mut print_args: Vec<BasicMetadataValueEnum> = vec![buf.into(), size.into(), fmt.into()];
    print_args.extend(args.iter().map(|v| BasicMetadataValueEnum::from(*v)));

    ctx.builder.build_call(snprintf, &print_args, "fmt").ok()?;

    Some(buf.into())
}
