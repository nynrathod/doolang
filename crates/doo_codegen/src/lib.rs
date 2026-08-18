//! # Doo Codegen
//!
//! LLVM code generation for the Doo compiler.
//!
//! Translates MIR (Mid-level IR) into LLVM IR, then into native machine code.
//! The compiler emits generic @extern calls only — no builtins/ directory,
//! no framework-specific codegen. Method calls resolve through the type system.

pub mod builder;
pub mod context;
pub mod debug_info;
pub mod fat_string;
pub mod instructions;
pub mod layout;
pub mod linker;
pub mod memory;
pub mod optimize;
pub mod types;
pub mod utils;

pub use builder::{CodegenBuilder, OwnedCodegenBuilder};
pub use context::CodegenContext;
pub use debug_info::DebugInfo;
pub use instructions::InstructionDispatcher;
pub use linker::{BinaryLinker, CrossModuleResolver, LinkError, ModuleLinker};
pub use optimize::{optimize_module, OptLevel, OptimizationConfig};
