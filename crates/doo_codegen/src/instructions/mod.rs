//! Instruction Handlers
//!
//! Per-category instruction handlers for modular codegen.

pub mod arithmetic;
pub mod memory;
pub mod control_flow;
// pub mod collections; // Deprecated
pub mod arrays;
pub mod maps;
pub mod composites;
pub mod calls;
pub mod enums;
pub mod closures;
pub mod casts;
pub mod async_ops;

use doo_core::doo_debug;

use inkwell::values::BasicValueEnum;
use doo_mir::MirInstr;
use crate::context::CodegenContext;

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
                // Box::new(collections::CollectionHandler), // Split into arrays/maps/composites
                Box::new(arrays::ArrayHandler),
                Box::new(maps::MapHandler),
                Box::new(composites::CompositeHandler),
                Box::new(calls::CallHandler),
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
        for handler in &self.handlers {
            if handler.handles(instr) {
                if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                    doo_debug!("CODEGEN", "    -> Handled by handler");
                }
                return handler.emit(ctx, instr);
            }
        }
        // Unknown instruction - emit nothing
        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
            doo_debug!("CODEGEN", "    -> NO HANDLER for instruction");
        }
        None
    }
}

impl<'ctx> Default for InstructionDispatcher<'ctx> {
    fn default() -> Self {
        Self::new()
    }
}
