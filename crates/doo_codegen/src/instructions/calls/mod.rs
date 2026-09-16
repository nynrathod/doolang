//! Call Instruction Handlers

pub mod call_ffi;
pub mod call_utils;
pub mod method_call;

pub use method_call::MethodCallHandler;

use crate::context::CodegenContext;
use crate::instructions::InstructionHandler;
use crate::utils::operand_to_value;
use doo_mir::sym::resolve;
use doo_mir::{MirInstr, MirInstrKind, MirOperand};
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue};
use inkwell::AddressSpace;

pub struct CallHandler;

impl<'ctx> InstructionHandler<'ctx> for CallHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(&instr.kind, MirInstrKind::Call { .. })
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::Call { dest, func, args } => {
                let func_name = resolve(*func);
                eprintln!("[CALL-DEBUG] func={:?} args={}", func_name, args.len());

                let func_val = get_or_declare_function(ctx, &func_name, args)?;
                eprintln!("[CALL-DEBUG] function OK");

                // Get param types from the function type
                let fn_type = func_val.get_type();
                let param_types: &[BasicMetadataTypeEnum<'ctx>] = &fn_type.get_param_types();

                let mut arg_values: Vec<BasicMetadataValueEnum<'ctx>> = Vec::new();
                for (i, arg) in args.iter().enumerate() {
                    let arg_val = operand_to_value(ctx, arg)?;
                    let casted = cast_arg(ctx, arg_val, param_types.get(i), i);
                    arg_values.push(casted);
                }

                let call_site =
                    ctx.builder
                        .build_call(func_val, &arg_values, &format!("call_{}", func_name));

                match call_site {
                    Ok(cs) => {
                        let result = cs.try_as_basic_value().basic();
                        match result {
                            Some(val) => {
                                if let Some(dest_name) = dest {
                                    ctx.set_temp(&resolve(*dest_name), val);
                                }
                                Some(val)
                            }
                            None => {
                                // Void function — store dummy for dest
                                if let Some(dest_name) = dest {
                                    let zero = ctx.context.i64_type().const_zero();
                                    ctx.set_temp(&resolve(*dest_name), zero.into());
                                }
                                None
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[CALL-DEBUG] build_call FAILED: {:?}", e);
                        None
                    }
                }
            }
            _ => None,
        }
    }
}

/// Get existing function from module, or declare it with correct signature.
fn get_or_declare_function<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    name: &str,
    args: &[MirOperand],
) -> Option<FunctionValue<'ctx>> {
    if let Some(f) = ctx.module.get_function(name) {
        return Some(f);
    }

    let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
    let i64_type = ctx.context.i64_type();
    let i32_type = ctx.context.i32_type();
    let void_type = ctx.context.void_type();

    eprintln!("[CALL-DEBUG] declaring new function: {}", name);

    // Use string literals to avoid missing constant issues
    let fn_type = match name {
        // Print functions (language level)
        "doo_print_str" => {
            eprintln!("[CALL-DEBUG]   signature: void(i8*)");
            void_type.fn_type(&[ptr_type.into()], false)
        }
        "doo_println" => {
            eprintln!("[CALL-DEBUG]   signature: void()");
            void_type.fn_type(&[], false)
        }

        // Memory
        "malloc" | "doo_alloc" => ptr_type.fn_type(&[i64_type.into()], false),
        "free" | "doo_free" => void_type.fn_type(&[ptr_type.into()], false),
        "realloc" | "doo_realloc" => ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false),

        // String
        "strlen" => i64_type.fn_type(&[ptr_type.into()], false),
        "memcpy" => void_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false),
        "memset" => void_type.fn_type(&[ptr_type.into(), i32_type.into(), i64_type.into()], false),
        "strcmp" => i32_type.fn_type(&[ptr_type.into(), ptr_type.into()], false),

        // I/O
        "printf" => i32_type.fn_type(&[ptr_type.into()], true),
        "fflush" => i32_type.fn_type(&[ptr_type.into()], false),
        "puts" => i32_type.fn_type(&[ptr_type.into()], false),
        "exit" => void_type.fn_type(&[i32_type.into()], false),

        // Default: generic i64(i64, i64, ...)
        _ => {
            eprintln!(
                "[CALL-DEBUG]   generic signature: i64({} params)",
                args.len()
            );
            let param_types: Vec<_> = (0..args.len()).map(|_| i64_type.into()).collect();
            i64_type.fn_type(&param_types, false)
        }
    };

    let func = ctx.module.add_function(name, fn_type, None);
    eprintln!("[CALL-DEBUG] declared OK");
    Some(func)
}

/// Cast an argument value to match the expected parameter type.
/// Handles: i64→ptr, ptr→i64, int widening/narrowing.
fn cast_arg<'ctx>(
    ctx: &mut CodegenContext<'ctx>,
    val: BasicValueEnum<'ctx>,
    ptype: Option<&BasicMetadataTypeEnum<'ctx>>,
    idx: usize,
) -> BasicMetadataValueEnum<'ctx> {
    let Some(ptype) = ptype else {
        return val.into();
    };

    // i64 → pointer (arrays stored as i64, functions expect pointer)
    if ptype.is_pointer_type() && val.is_int_value() {
        let result = ctx.builder.build_int_to_ptr(
            val.into_int_value(),
            ptype.into_pointer_type(),
            &format!("arg{}_i2p", idx),
        );
        if let Ok(p) = result {
            return p.into();
        }
    }

    // pointer → i64
    if ptype.is_int_type() && val.is_pointer_value() {
        let result = ctx.builder.build_ptr_to_int(
            val.into_pointer_value(),
            ptype.into_int_type(),
            &format!("arg{}_p2i", idx),
        );
        if let Ok(i) = result {
            return i.into();
        }
    }

    // Integer widening (i8/i32 → i64)
    if ptype.is_int_type() && val.is_int_value() {
        let src_bw = val.get_type().into_int_type().get_bit_width();
        let dst_bw = ptype.into_int_type().get_bit_width();
        if src_bw < dst_bw {
            if let Ok(e) = ctx.builder.build_int_z_extend(
                val.into_int_value(),
                ptype.into_int_type(),
                &format!("arg{}_zext", idx),
            ) {
                return e.into();
            }
        } else if src_bw > dst_bw {
            if let Ok(t) = ctx.builder.build_int_truncate(
                val.into_int_value(),
                ptype.into_int_type(),
                &format!("arg{}_trunc", idx),
            ) {
                return t.into();
            }
        }
    }

    val.into()
}
