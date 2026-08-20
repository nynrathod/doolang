//! Cast Instruction Handler
//!
//! Handles: Cast (type conversions)

use super::InstructionHandler;
use crate::context::CodegenContext;
use doo_core::types::builtin;
use doo_core::TypeKind;
use doo_mir::sym::resolve;
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
                    // Check if source is actually a Str — only then pass pointer through.
                    // Non-string pointers (Enum, Struct, Array, Map) must NOT be passed
                    // through as C strings — they'd print "<null>" or garbage.
                    let source_type = match value {
                        MirOperand::Local(name) | MirOperand::Temp(name) => {
                            ctx.get_variable_type(&resolve(*name))
                        }
                        _ => None,
                    };

                    let is_str = source_type.map_or(false, |tid| {
                        ctx.get_type_kind(tid)
                            .map_or(false, |k| matches!(k, TypeKind::Str))
                    });

                    if is_str {
                        // String pointer → pass through directly
                        Some(source_val)
                    } else if source_val.is_int_value() {
                        // Int/Bool → convert to string
                        emit_string_from_value(ctx, source_val)
                    } else if source_val.is_float_value() {
                        // Float → convert to string
                        emit_string_from_value(ctx, source_val)
                    } else if source_val.is_pointer_value() {
                        // Non-string pointer (Enum, Struct, Array, Map)
                        // Format as type name instead of passing raw pointer through.
                        let type_name = source_type
                            .and_then(|tid| ctx.get_type_kind(tid))
                            .map(|k| k.kind_name().to_string())
                            .unwrap_or_else(|| "<value>".to_string());
                        Some(ctx.const_string(&type_name).into())
                    } else {
                        emit_string_from_value(ctx, source_val)
                    }
                } else if source_val.is_pointer_value() {
                    // String (or Ptr) -> Int/Float via FFI
                    // Assuming source is String for now if it's a pointer and not array/map/etc
                    // Ideally we check type_id of source but we only have operand.
                    // Operand type check is done in Analysis. Here we blindly cast.

                    use doo_core::constants::ffi_names;
                    if target_type_id == builtin::INT {
                        let cast_fn = ctx
                            .module
                            .get_function("doo_cast_str_to_int")
                            .unwrap_or_else(|| {
                                let i64_ty = ctx.context.i64_type();
                                let ptr_ty = ctx
                                    .context
                                    .i8_type()
                                    .ptr_type(inkwell::AddressSpace::default());
                                let fn_ty = i64_ty.fn_type(&[ptr_ty.into()], false);
                                ctx.module.add_function("doo_cast_str_to_int", fn_ty, None)
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
                            .get_function("doo_cast_str_to_float")
                            .unwrap_or_else(|| {
                                let f64_ty = ctx.context.f64_type();
                                let ptr_ty = ctx
                                    .context
                                    .i8_type()
                                    .ptr_type(inkwell::AddressSpace::default());
                                let fn_ty = f64_ty.fn_type(&[ptr_ty.into()], false);
                                ctx.module
                                    .add_function("doo_cast_str_to_float", fn_ty, None)
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
                    ctx.set_temp(&resolve(*dest), res);
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

fn emit_string_from_value<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
) -> Option<BasicValueEnum<'ctx>> {
    use doo_core::constants::ffi_names;

    // Str (pointer) → pass through — already a string
    if val.is_pointer_value() {
        return Some(val);
    }

    // Int → call doo_int_to_str (Rust)
    if val.is_int_value() {
        let int_val = val.into_int_value();
        let bit_width = int_val.get_type().get_bit_width();

        // Bool (i1) → call doo_bool_to_str
        if bit_width == 1 {
            let bool_fn = ctx
                .module
                .get_function(ffi_names::DOO_BOOL_TO_STR)
                .unwrap_or_else(|| {
                    let ptr_type = ctx.ptr_type();
                    let i32_type = ctx.i32_type();
                    let fn_type = ptr_type.fn_type(&[i32_type.into()], false);
                    ctx.module
                        .add_function(ffi_names::DOO_BOOL_TO_STR, fn_type, None)
                });
            // Extend i1 to i32 for the FFI call
            let extended = ctx
                .builder
                .build_int_z_extend(int_val, ctx.i32_type(), "bool_ext")
                .ok()?;
            let result = ctx
                .builder
                .build_call(bool_fn, &[extended.into()], "bool_to_str")
                .ok()?
                .try_as_basic_value()
                .basic()?;
            return Some(result);
        }

        // Int → call doo_int_to_str
        let int_fn = ctx
            .module
            .get_function(ffi_names::DOO_INT_TO_STR)
            .unwrap_or_else(|| {
                let ptr_type = ctx.ptr_type();
                let i64_type = ctx.i64_type();
                let fn_type = ptr_type.fn_type(&[i64_type.into()], false);
                ctx.module
                    .add_function(ffi_names::DOO_INT_TO_STR, fn_type, None)
            });
        // Ensure int is i64
        let int_64 = if bit_width < 64 {
            ctx.builder
                .build_int_z_extend(int_val, ctx.i64_type(), "int_ext")
                .ok()?
        } else {
            int_val
        };
        let result = ctx
            .builder
            .build_call(int_fn, &[int_64.into()], "int_to_str")
            .ok()?
            .try_as_basic_value()
            .basic()?;
        return Some(result);
    }

    // Float → call doo_float_to_str (Rust)
    if val.is_float_value() {
        let float_val = val.into_float_value();
        // Ensure it's f64
        let f64_val = if float_val.get_type() != ctx.f64_type() {
            ctx.builder
                .build_float_ext(float_val, ctx.f64_type(), "fext")
                .ok()?
        } else {
            float_val
        };
        let float_fn = ctx
            .module
            .get_function(ffi_names::DOO_FLOAT_TO_STR)
            .unwrap_or_else(|| {
                let ptr_type = ctx.ptr_type();
                let f64_type = ctx.f64_type();
                let fn_type = ptr_type.fn_type(&[f64_type.into()], false);
                ctx.module
                    .add_function(ffi_names::DOO_FLOAT_TO_STR, fn_type, None)
            });
        let result = ctx
            .builder
            .build_call(float_fn, &[f64_val.into()], "float_to_str")
            .ok()?
            .try_as_basic_value()
            .basic()?;
        return Some(result);
    }

    // Null/nil → call doo_null_to_str
    let null_fn = ctx
        .module
        .get_function(ffi_names::DOO_NULL_TO_STR)
        .unwrap_or_else(|| {
            let ptr_type = ctx.ptr_type();
            let fn_type = ptr_type.fn_type(&[], false);
            ctx.module
                .add_function(ffi_names::DOO_NULL_TO_STR, fn_type, None)
        });
    let result = ctx
        .builder
        .build_call(null_fn, &[], "null_to_str")
        .ok()?
        .try_as_basic_value()
        .basic()?;
    Some(result)
}
