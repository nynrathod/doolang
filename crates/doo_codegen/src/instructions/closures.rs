//! Closure Instruction Handler
//!
//! Handles: ClosureCreate, ClosureCall

use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;
use doo_mir::{MirInstr, MirInstrKind, MirOperand, MirConst};
use crate::context::CodegenContext;
use super::InstructionHandler;

/// Closure instruction handler.
pub struct ClosureHandler;

impl<'ctx> InstructionHandler<'ctx> for ClosureHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(instr.kind,
            MirInstrKind::ClosureCreate { .. } |
            MirInstrKind::ClosureCall { .. }
        )
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::ClosureCreate { dest, func, captures } => {
                // Closure layout: { ptr fn_ptr, ptr env }
                // fn_ptr: pointer to the function
                // env: pointer to captured environment struct
                
                let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
                let closure_type = ctx.context.struct_type(
                    &[ptr_type.into(), ptr_type.into()],
                    false,
                );
                
                // Allocate closure struct
                let alloca = ctx.builder.build_alloca(closure_type, dest).ok()?;
                
                // Get function pointer
                if let Some(func_val) = ctx.get_function(func) {
                    let fn_ptr = func_val.as_global_value().as_pointer_value();
                    
                    // Store fn_ptr
                    if let Ok(fn_ptr_slot) = ctx.builder.build_struct_gep(closure_type, alloca, 0, "fn_ptr") {
                        ctx.builder.build_store(fn_ptr_slot, fn_ptr).ok();
                    }
                }
                
                // Create environment struct with captures
                if !captures.is_empty() {
                    // Allocate environment
                    let env_size = captures.len() as u64 * 8; // 8 bytes per capture (i64)
                    let env_type = ctx.context.i64_type().array_type(captures.len() as u32);
                    let env_alloca = ctx.builder.build_alloca(env_type, "env").ok()?;
                    
                    // Store each capture
                    for (i, cap) in captures.iter().enumerate() {
                        if let Some(cap_val) = operand_to_value(ctx, cap) {
                            let indices = [
                                ctx.const_i64(0).into(),
                                ctx.const_i64(i as i64).into(),
                            ];
                            if let Ok(cap_ptr) = unsafe { ctx.builder.build_gep(env_type, env_alloca, &indices, "cap_ptr") } {
                                // Store capture value
                                if cap_val.is_int_value() {
                                    ctx.builder.build_store(cap_ptr, cap_val).ok();
                                }
                            }
                        }
                    }
                    
                    // Store env pointer in closure
                    if let Ok(env_slot) = ctx.builder.build_struct_gep(closure_type, alloca, 1, "env_ptr") {
                        ctx.builder.build_store(env_slot, env_alloca).ok();
                    }
                } else {
                    // No captures - store null env
                    let null_ptr = ptr_type.const_null();
                    if let Ok(env_slot) = ctx.builder.build_struct_gep(closure_type, alloca, 1, "env_ptr") {
                        ctx.builder.build_store(env_slot, null_ptr).ok();
                    }
                }
                
                ctx.set_temp(dest, alloca.into());
                Some(alloca.into())
            }

            MirInstrKind::ClosureCall { dest, closure, args } => {
                // Extract fn_ptr from closure and call with (env, ...args)
                if let Some(closure_val) = operand_to_value(ctx, closure) {
                    if closure_val.is_pointer_value() {
                        let closure_ptr = closure_val.into_pointer_value();
                        
                        let ptr_type = ctx.context.i8_type().ptr_type(AddressSpace::default());
                        let closure_type = ctx.context.struct_type(
                            &[ptr_type.into(), ptr_type.into()],
                            false,
                        );
                        
                        // Get fn_ptr
                        if let Ok(fn_ptr_slot) = ctx.builder.build_struct_gep(closure_type, closure_ptr, 0, "fn_ptr") {
                            if let Ok(fn_ptr) = ctx.builder.build_load(ptr_type, fn_ptr_slot, "fn") {
                                // Get env_ptr
                                if let Ok(env_slot) = ctx.builder.build_struct_gep(closure_type, closure_ptr, 1, "env_ptr") {
                                    if let Ok(env_ptr) = ctx.builder.build_load(ptr_type, env_slot, "env") {
                                        // Build args: env, ...user_args
                                        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = vec![env_ptr.into()];
                                        for arg in args {
                                            if let Some(v) = operand_to_value(ctx, arg) {
                                                call_args.push(v.into());
                                            }
                                        }
                                        
                                        // Build indirect call
                                        // For now, just set dest to 0 (full impl needs function type)
                                        if let Some(dest_name) = dest {
                                            let result = ctx.const_i64(0);
                                            ctx.set_temp(dest_name, result.into());
                                            return Some(result.into());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                None
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
