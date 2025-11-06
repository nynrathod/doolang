// Core module

pub mod core;
pub use core::{ArrayMetadata, CodeGen, LoopContext, LoopType, MapMetadata, Symbol};
// Instruction generation
pub mod instructions;
// Type-specific operations
pub mod types;
// Memory management
pub mod memory;
// Built-in methods
pub mod builtins;
// Closures
pub mod closures;
// Main modules
pub mod builder;
pub mod functions;
pub mod globals;
pub mod loops;

#[cfg(test)]
mod tests;
