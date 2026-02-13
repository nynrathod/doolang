//! Control Flow Instruction Handler
//!
//! Handles control flow terminators (handled separately in builder)
//! This handler is a placeholder for any non-terminator control flow.

use super::InstructionHandler;
use crate::context::CodegenContext;
use crate::utils::operand_to_value;
use doo_mir::{MirInstr, MirInstrKind};
use inkwell::values::BasicValueEnum;

/// Control flow instruction handler.
pub struct ControlFlowHandler;

impl<'ctx> InstructionHandler<'ctx> for ControlFlowHandler {
    fn handles(&self, instr: &MirInstr) -> bool {
        matches!(&instr.kind, MirInstrKind::Panic { .. })
    }

    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        match &instr.kind {
            MirInstrKind::Panic { message } => {
                // Get message value (should be a string pointer)
                let msg_val = operand_to_value(ctx, message)?;

                // Emit panic: print message and abort
                emit_panic_with_value(ctx, msg_val);

                None
            }
            _ => None,
        }
    }
}

/// Emit panic code: print message and exit(1).
/// NOTE: Does NOT emit unreachable - that's handled by MirTerminator::Unreachable
fn emit_panic_with_value<'ctx>(ctx: &mut CodegenContext<'ctx>, message: BasicValueEnum<'ctx>) {
    // Get or declare printf
    let printf_type = ctx.i32_type().fn_type(&[ctx.ptr_type().into()], true);
    let printf = ctx
        .module
        .get_function("printf")
        .unwrap_or_else(|| ctx.module.add_function("printf", printf_type, None));

    // If message is a pointer, it might be an error struct (like FileError)
    // Error structs have Message as first field - extract it
    let msg_ptr = if message.is_pointer_value() {
        let ptr = message.into_pointer_value();
        // Load the first field (Message) from the error struct
        // FileError = { Message: *char } -> first field at offset 0
        let msg_field_ptr = unsafe {
            ctx.builder
                .build_gep(
                    ctx.ptr_type(), // Field type is a pointer (to char)
                    ptr,
                    &[ctx.context.i32_type().const_zero()],
                    "error_msg_ptr",
                )
                .ok()
        };
        match msg_field_ptr {
            Some(field_ptr) => {
                // Load the message pointer from the struct
                ctx.builder
                    .build_load(ctx.ptr_type(), field_ptr, "error_msg")
                    .map(|v| v.into())
                    .unwrap_or(message)
            }
            None => message,
        }
    } else {
        message
    };

    // Print panic message
    let panic_fmt = ctx.const_string("panic: %s\n");
    let _ = ctx
        .builder
        .build_call(printf, &[panic_fmt.into(), msg_ptr.into()], "print_panic");

    // Flush stdout to ensure panic message is visible before exit
    let fflush_type = ctx.i32_type().fn_type(&[ctx.ptr_type().into()], false);
    let fflush_fn = ctx
        .module
        .get_function("fflush")
        .unwrap_or_else(|| ctx.module.add_function("fflush", fflush_type, None));
    let null_ptr = ctx.ptr_type().const_null();
    let _ = ctx
        .builder
        .build_call(fflush_fn, &[null_ptr.into()], "flush_before_exit");

    // Get or declare exit
    let exit_type = ctx
        .context
        .void_type()
        .fn_type(&[ctx.i32_type().into()], false);
    let exit_fn = ctx
        .module
        .get_function("exit")
        .unwrap_or_else(|| ctx.module.add_function("exit", exit_type, None));

    // Exit with code 1
    let exit_code = ctx.i32_type().const_int(1, false);
    let _ = ctx
        .builder
        .build_call(exit_fn, &[exit_code.into()], "exit_on_panic");

    // Don't emit unreachable here - let MirTerminator::Unreachable handle it
}
