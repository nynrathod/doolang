//! Call instruction handlers — generic FFI call dispatch.
//!
//! All function calls (both internal and @extern) flow through a single
//! generic path. The compiler never matches on framework-specific symbol
//! names.

pub mod call_ffi;
pub mod call_print;
pub mod call_utils;

use crate::context::CodegenContext;
use crate::instructions::InstructionHandler;
use doo_mir::sym::resolve;
use doo_mir::{MirInstr, MirInstrKind};
use inkwell::values::BasicValueEnum;

/// Call instruction handler.
///
/// Handles direct function calls and FFI calls. Method calls on built-in
/// types are dispatched separately via `builtins::dispatch_method`.
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
                let dest_str = dest.map(|s| resolve(s));
                let func_str = resolve(*func);

                if func_str == "print" {
                    return call_print::emit_print(ctx, args);
                }

                call_ffi::emit_ffi_call(ctx, dest_str.as_deref(), &func_str, args)
            }
            _ => None,
        }
    }
}
