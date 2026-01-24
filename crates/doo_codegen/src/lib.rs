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

pub mod context;
pub mod types;
pub mod instructions;
pub mod memory;
pub mod optimize;
pub mod builder;
pub mod builtins;
pub mod layout;

pub use context::CodegenContext;
pub use builder::CodegenBuilder;
pub use optimize::{optimize_module, OptLevel};
