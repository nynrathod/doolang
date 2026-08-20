//! Instruction Handlers
//!
//! Per-category instruction handlers for modular codegen.

pub mod arithmetic;
pub mod arrays;
pub mod async_ops;
pub mod calls;
pub mod casts;
pub mod closures;
pub mod composites;
pub mod control_flow;
pub mod enums;
pub mod maps;
pub mod memory;

use doo_core::doo_debug;

use crate::context::CodegenContext;
use doo_mir::MirInstr;
use inkwell::values::BasicValueEnum;

/// Instruction handler trait.
///
/// Each handler is responsible for a category of MIR instructions.
pub trait InstructionHandler<'ctx> {
    /// Check if this handler can process the instruction.
    fn handles(&self, instr: &MirInstr) -> bool;

    /// Emit LLVM IR for the instruction.
    /// Returns Some(value) if the instruction produces a value.
    fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>>;
}

/// Instruction dispatcher.
///
/// Routes instructions to the appropriate handler.
pub struct InstructionDispatcher<'ctx> {
    handlers: Vec<Box<dyn InstructionHandler<'ctx> + 'ctx>>,
}

impl<'ctx> InstructionDispatcher<'ctx> {
    /// Create a new dispatcher with all default handlers.
    pub fn new() -> Self {
        Self {
            handlers: vec![
                Box::new(arithmetic::ArithmeticHandler),
                Box::new(memory::MemoryHandler),
                Box::new(control_flow::ControlFlowHandler),
                Box::new(arrays::ArrayHandler),
                Box::new(maps::MapHandler),
                Box::new(composites::CompositeHandler),
                Box::new(calls::CallHandler),
                Box::new(calls::MethodCallHandler),
                Box::new(calls::FfiCallHandler),
                Box::new(enums::EnumHandler),
                Box::new(closures::ClosureHandler),
                Box::new(casts::CastHandler),
                Box::new(async_ops::AsyncOpsHandler),
            ],
        }
    }

    /// Emit LLVM IR for an instruction.
    pub fn emit(
        &self,
        ctx: &mut CodegenContext<'ctx>,
        instr: &MirInstr,
    ) -> Option<BasicValueEnum<'ctx>> {
        doo_debug!("codegen-dispatch", "instruction {:?}", instr.kind);

        for handler in &self.handlers {
            if handler.handles(instr) {
                return handler.emit(ctx, instr);
            }
        }

        doo_debug!(
            "codegen-dispatch",
            "no handler for {:?}",
            instr.kind
        );
        None
    }
}

impl<'ctx> Default for InstructionDispatcher<'ctx> {
    fn default() -> Self {
        Self::new()
    }
}
