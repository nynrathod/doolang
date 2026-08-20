//! Call instruction handlers — generic FFI call dispatch.
//!
//! All function calls (both internal and @extern) flow through a single
//! generic path. The compiler never matches on framework-specific symbol
//! names.

pub mod call_ffi;
pub mod call_utils;

use crate::context::CodegenContext;
use crate::instructions::InstructionHandler;
use crate::utils::operand_to_value;
use doo_core::doo_debug;
use doo_mir::sym::resolve;
use doo_mir::{MirInstr, MirInstrKind};
use inkwell::module::Linkage;
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::BasicValueEnum;

/// Call instruction handler.
pub struct CallHandler;

impl<'ctx> InstructionHandler<'ctx> for CallHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(instr.kind, MirInstrKind::Call { .. })
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::Call { dest, func, args } => {
                let func_name = resolve(*func);
                doo_debug!("codegen-Call", "emitting call {}", func_name);

                if call_ffi::is_runtime_symbol(&func_name)
                    || ctx.ffi_type_signatures.contains_key(&func_name)
                {
                    let dest_str = dest.map(|s| resolve(s));
                    return call_ffi::emit_ffi_call(ctx, dest_str.as_deref(), &func_name, args);
                }

                // Resolve arguments first so we know their LLVM types
                let mut arg_vals = Vec::new();
                for arg in args {
                    if let Some(v) = operand_to_value(ctx, arg) {
                        arg_vals.push(v);
                    } else {
                        doo_debug!(
                            "codegen-Call",
                            "failed to resolve argument for {}",
                            func_name
                        );
                        return None;
                    }
                }

                // Try to get the function, or declare it if it's missing (e.g., stdlib function)
                let func_val = match ctx.get_function(&func_name) {
                    Some(f) => f,
                    None => {
                        doo_debug!(
                            "codegen-Call",
                            "declaring external {}",
                            func_name
                        );

                        // Infer parameter types from the arguments we just resolved
                        let param_types: Vec<BasicMetadataTypeEnum> =
                            arg_vals.iter().map(|v| v.get_type().into()).collect();

                        // Infer return type: if dest is Some, assume i64, else void
                        let fn_type = if dest.is_some() {
                            ctx.i64_type().fn_type(&param_types, false)
                        } else {
                            ctx.context.void_type().fn_type(&param_types, false)
                        };

                        ctx.module
                            .add_function(&func_name, fn_type, Some(Linkage::External))
                    }
                };

                let call_args: Vec<_> = arg_vals.iter().map(|v| (*v).into()).collect();
                let call_site = ctx.builder.build_call(func_val, &call_args, "call").ok()?;

                // FIX: Use .basic() instead of .left() for inkwell LLVM 22 compatibility
                if let Some(result) = call_site.try_as_basic_value().basic() {
                    if let Some(dest_name) = dest {
                        ctx.set_temp(&resolve(*dest_name), result);
                    }
                    Some(result)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// Handles method calls. Since THIR resolved the method to a function,
/// this just delegates to the standard Call path with `self` as the first arg.
pub struct MethodCallHandler;

impl<'ctx> InstructionHandler<'ctx> for MethodCallHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(instr.kind, MirInstrKind::MethodCall { .. })
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::MethodCall {
                dest,
                receiver,
                method,
                args,
                ..
            } => {
                let func_name = resolve(*method);
                doo_debug!("codegen-MethodCall", "emitting method {}", func_name);

                if call_ffi::is_runtime_symbol(&func_name)
                    || ctx.ffi_type_signatures.contains_key(&func_name)
                {
                    let mut ffi_args = vec![receiver.clone()];
                    ffi_args.extend(args.iter().cloned());
                    let dest_str = dest.map(|s| resolve(s));
                    return call_ffi::emit_ffi_call(
                        ctx,
                        dest_str.as_deref(),
                        &func_name,
                        &ffi_args,
                    );
                }

                let recv_val = operand_to_value(ctx, receiver)?;

                let mut all_args = vec![recv_val];
                for arg in args {
                    if let Some(v) = operand_to_value(ctx, arg) {
                        all_args.push(v);
                    } else {
                        doo_debug!(
                            "codegen-MethodCall",
                            "failed to resolve argument for {}",
                            func_name
                        );
                        return None;
                    }
                }

                // Try to get the function, or declare it if it's missing
                let func_val = match ctx
                    .get_function(&func_name)
                    .or_else(|| ctx.module.get_function(&func_name))
                {
                    Some(f) => f,
                    None => {
                        doo_debug!(
                            "codegen-MethodCall",
                            "declaring external {}",
                            func_name
                        );
                        let param_types: Vec<BasicMetadataTypeEnum> =
                            all_args.iter().map(|v| v.get_type().into()).collect();
                        let fn_type = if dest.is_some() {
                            ctx.i64_type().fn_type(&param_types, false)
                        } else {
                            ctx.context.void_type().fn_type(&param_types, false)
                        };
                        ctx.module
                            .add_function(&func_name, fn_type, Some(Linkage::External))
                    }
                };

                let call_args: Vec<_> = all_args.iter().map(|v| (*v).into()).collect();
                let call_site = ctx
                    .builder
                    .build_call(func_val, &call_args, "method_call")
                    .ok()?;

                // FIX: Use .basic() instead of .left() for inkwell LLVM 22 compatibility
                if let Some(result) = call_site.try_as_basic_value().basic() {
                    if let Some(dest_name) = dest {
                        ctx.set_temp(&resolve(*dest_name), result);
                    }
                    Some(result)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// Handles @extern calls. Delegates to call_ffi.rs.
pub struct FfiCallHandler;

impl<'ctx> InstructionHandler<'ctx> for FfiCallHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(instr.kind, MirInstrKind::FfiCall { .. })
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::FfiCall {
                dest, symbol, args, ..
            } => {
                let dest_str = dest.map(|s| resolve(s));
                let symbol_str = resolve(*symbol);
                doo_debug!("codegen-FfiCall", "emitting {}", symbol_str);

                call_ffi::emit_ffi_call(
                    ctx,
                    dest_str.as_deref(),
                    &symbol_str,
                    args,
                )
            }
            _ => None,
        }
    }
}
