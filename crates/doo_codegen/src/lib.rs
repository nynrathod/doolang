//! # Doo Codegen
//!
//! LLVM code generation from MIR.
//!
//! ## Architecture
//!
//! - `context` - CodegenContext with LLVM module, builder, types
//! - `types` - Doo type to LLVM type mapping (single source of truth)
//! - `instructions` - Per-category instruction handlers
//! - `memory` - Memory management (alloca, load, store, drop)
//! - `optimize` - LLVM optimization passes
//!
//! ## Multi-File Support
//!
//! - `ModuleLinker` - Links multiple LLVM modules together
//! - `CrossModuleResolver` - Resolves cross-module function references
//! - `ExternalFunction` - Metadata for external function declarations

pub mod builder;
pub mod builtins;
pub mod context;
pub mod instructions;
pub mod layout;
pub mod linker;
pub mod memory;
pub mod optimize;
pub mod packages;
pub mod types;
pub mod utils;

pub use builder::CodegenBuilder;
pub use context::{CodegenContext, ExternalFunction};
pub use linker::{CrossModuleResolver, LinkError, ModuleLinker};
pub use optimize::{
    optimize_module, optimize_module_default, optimize_module_none, optimize_module_size,
    optimize_module_with_config, OptLevel, OptimizationConfig,
};
