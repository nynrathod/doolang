//! Control Flow Instruction Handler
//!
//! Handles control flow terminators (handled separately in builder)
//! This handler is a placeholder for any non-terminator control flow.

use inkwell::values::BasicValueEnum;
use doo_mir::MirInstr;
use crate::context::CodegenContext;
use super::InstructionHandler;

/// Control flow instruction handler.
pub struct ControlFlowHandler;

impl<'ctx> InstructionHandler<'ctx> for ControlFlowHandler {
    fn handles(&self, _instr: &MirInstr) -> bool {
        // Control flow (terminators) handled directly in builder
        false
    }

    fn emit(
        &self,
        _ctx: &mut CodegenContext<'ctx>,
        _instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        None
    }
}
