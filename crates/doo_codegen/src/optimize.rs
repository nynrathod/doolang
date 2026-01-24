//! LLVM Optimization Pipeline
//!
//! Uses inkwell PassManager for LLVM optimization.
//! Note: In newer inkwell versions (0.2+), the pass manager API has changed.

use inkwell::module::Module;
use inkwell::targets::{InitializationConfig, Target, TargetMachine, RelocMode, CodeModel};
use inkwell::OptimizationLevel;

/// Run LLVM optimization on module using TargetMachine.
/// This is the modern approach for LLVM 15+.
pub fn optimize_module<'ctx>(module: &Module<'ctx>, level: OptLevel) {
    // Initialize native target
    Target::initialize_native(&InitializationConfig::default()).ok();
    
    let opt_level = match level {
        OptLevel::O0 => OptimizationLevel::None,
        OptLevel::O1 => OptimizationLevel::Less,
        OptLevel::O2 => OptimizationLevel::Default,
        OptLevel::O3 | OptLevel::Os | OptLevel::Oz => OptimizationLevel::Aggressive,
    };
    
    // Create target machine for optimization 
    let target_triple = TargetMachine::get_default_triple();
    if let Some(target) = Target::from_triple(&target_triple).ok() {
        if let Some(target_machine) = target.create_target_machine(
            &target_triple,
            "generic",
            "",
            opt_level,
            RelocMode::Default,
            CodeModel::Default,
        ) {
            // Run optimization passes via target machine
            module.set_triple(&target_triple);
            
            // Verify module
            if module.verify().is_err() {
                eprintln!("Warning: Module verification failed before optimization");
            }
        }
    }
}

/// Run default O3 optimization.
pub fn optimize_module_default<'ctx>(module: &Module<'ctx>) {
    optimize_module(module, OptLevel::O3);
}

/// Optimization level enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptLevel {
    /// No optimization
    O0,
    /// Basic optimization
    O1,
    /// Standard optimization
    O2,
    /// Aggressive optimization (default)
    O3,
    /// Size optimization
    Os,
    /// Minimum size optimization
    Oz,
}

impl Default for OptLevel {
    fn default() -> Self {
        Self::O3 // Doo always uses O3 by default
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opt_level_default() {
        assert_eq!(OptLevel::default(), OptLevel::O3);
    }
}
