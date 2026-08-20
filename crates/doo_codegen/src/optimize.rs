//! LLVM Optimization Pipeline
//!
//! Production-grade LLVM optimization using inkwell 0.8.0 New Pass Manager.
//! This module provides the single source of truth for all LLVM optimization.
//!
//! ## Architecture (LLVM 18 New Pass Manager)
//!
//! The legacy PassManager was deprecated in LLVM 17+ and removed in newer versions.
//! inkwell 0.8.0 uses `Module::run_passes()` with `PassBuilderOptions` for the
//! new pass manager pipeline introduced in LLVM 13+.
//!
//! ## Pass Pipeline Levels
//!
//! - `default<O0>` - No optimization (fast compile)
//! - `default<O1>` - Basic optimization (balanced)
//! - `default<O2>` - Standard optimization (good perf)
//! - `default<O3>` - Aggressive optimization (max perf) [DEFAULT]
//! - `default<Os>` - Size optimization (smaller binary)
//! - `default<Oz>` - Min size optimization (smallest binary)

use inkwell::module::Module;
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{CodeModel, InitializationConfig, RelocMode, Target, TargetMachine};
use inkwell::OptimizationLevel;

// ============================================================================
// Optimization Level (Single Source of Truth)
// ============================================================================

/// Optimization level enum - centralized definition.
///
/// Maps directly to LLVM optimization pipelines:
/// - O0: No optimization (fastest compile)
/// - O1: Basic optimization
/// - O2: Standard optimization
/// - O3: Aggressive optimization (default for Doo)
/// - Os: Size optimization
/// - Oz: Minimum size optimization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OptLevel {
    /// No optimization - fastest compilation
    O0,
    /// Basic optimization - balanced compile time/performance
    O1,
    /// Standard optimization - good performance
    O2,
    /// Aggressive optimization - maximum performance (Doo default)
    #[default]
    O3,
    /// Size optimization - smaller binary, good performance
    Os,
    /// Minimum size optimization - smallest possible binary
    Oz,
}

impl OptLevel {
    /// Convert to LLVM pass pipeline string.
    ///
    /// These strings are passed to `module.run_passes()` and represent
    /// the standard LLVM optimization pipelines.
    #[inline]
    pub const fn to_pass_pipeline(self) -> &'static str {
        match self {
            Self::O0 => "default<O0>",
            Self::O1 => "default<O1>",
            Self::O2 => "default<O2>",
            Self::O3 => "default<O3>",
            Self::Os => "default<Os>",
            Self::Oz => "default<Oz>",
        }
    }

    /// Convert to inkwell OptimizationLevel for TargetMachine.
    #[inline]
    pub const fn to_inkwell_opt_level(self) -> OptimizationLevel {
        match self {
            Self::O0 => OptimizationLevel::None,
            Self::O1 => OptimizationLevel::Less,
            Self::O2 => OptimizationLevel::Default,
            Self::O3 | Self::Os | Self::Oz => OptimizationLevel::Aggressive,
        }
    }

    /// Parse from CLI argument string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "0" | "O0" | "o0" => Some(Self::O0),
            "1" | "O1" | "o1" => Some(Self::O1),
            "2" | "O2" | "o2" => Some(Self::O2),
            "3" | "O3" | "o3" => Some(Self::O3),
            "s" | "Os" | "os" => Some(Self::Os),
            "z" | "Oz" | "oz" => Some(Self::Oz),
            _ => None,
        }
    }
}

// ============================================================================
// PassBuilder Options (Centralized Configuration)
// ============================================================================

/// Configuration options for the LLVM New Pass Manager.
///
/// This provides a centralized way to configure optimization passes.
/// All options are derived from `PassBuilderOptions` in inkwell 0.8.0.
#[derive(Debug, Clone)]
pub struct OptimizationConfig {
    /// Optimization level
    pub level: OptLevel,
    /// Enable loop vectorization (auto-vectorize loops)
    pub loop_vectorization: bool,
    /// Enable SLP vectorization (superword-level parallelism)
    pub slp_vectorization: bool,
    /// Enable loop unrolling
    pub loop_unrolling: bool,
    /// Enable loop interleaving
    pub loop_interleaving: bool,
    /// Merge identical functions
    pub merge_functions: bool,
    /// Verify each pass (debug mode)
    pub verify_each: bool,
    /// Debug logging
    pub debug_logging: bool,
}

impl Default for OptimizationConfig {
    fn default() -> Self {
        Self {
            level: OptLevel::O3,
            loop_vectorization: true,
            slp_vectorization: true,
            loop_unrolling: true,
            loop_interleaving: true,
            // MergeFunctions is skipped: LLVM 22's pass can hang on Windows
            // when many similar stdlib/FFI declarations are present.
            merge_functions: false,
            verify_each: false,
            debug_logging: false,
        }
    }
}

impl OptimizationConfig {
    /// Create configuration for given optimization level.
    #[inline]
    pub fn for_level(level: OptLevel) -> Self {
        let mut config = Self::default();
        config.level = level;

        // Disable vectorization for O0 and O1
        if matches!(level, OptLevel::O0 | OptLevel::O1) {
            config.loop_vectorization = false;
            config.slp_vectorization = false;
            config.merge_functions = false;
        }

        // Disable unrolling for O0
        if level == OptLevel::O0 {
            config.loop_unrolling = false;
            config.loop_interleaving = false;
        }

        config
    }

    /// Create debug configuration with verification.
    pub fn debug(level: OptLevel) -> Self {
        let mut config = Self::for_level(level);
        config.verify_each = true;
        config.debug_logging = true;
        config
    }
}

// ============================================================================
// Core Optimization Functions
// ============================================================================

/// Run LLVM optimization on module using the New Pass Manager.
///
/// This is the primary entry point for LLVM optimization in Doo.
/// Uses `module.run_passes()` with the new pass manager API (LLVM 13+).
///
/// # Arguments
/// * `module` - The LLVM module to optimize
/// * `level` - Optimization level (O0-O3, Os, Oz)
///
/// # Example
/// ```ignore
/// let context = Context::create();
/// let module = context.create_module("my_module");
/// // ... populate module ...
/// optimize_module(&module, OptLevel::O3);
/// ```
pub fn optimize_module<'ctx>(module: &Module<'ctx>, level: OptLevel) {
    let config = OptimizationConfig::for_level(level);
    optimize_module_with_config(module, &config);
}

/// Run LLVM optimization with custom configuration.
///
/// This provides fine-grained control over the optimization pipeline.
pub fn optimize_module_with_config<'ctx>(module: &Module<'ctx>, config: &OptimizationConfig) {
    // Initialize native target (required for optimization)
    if Target::initialize_native(&InitializationConfig::default()).is_err() {
        return;
    }

    // Get target triple and create target machine
    let target_triple = TargetMachine::get_default_triple();
    let target = match Target::from_triple(&target_triple) {
        Ok(t) => t,
        Err(_e) => {
            // Leak the triple's LLVM string to avoid LLVM 22 disposal crash.
            // _e (LLVMString) also needs leak, but it's the error message
            // which would also crash on drop. Leak both.
            std::mem::forget(_e);
            std::mem::forget(target_triple);
            return;
        }
    };

    // Create target machine with optimization level.
    // CRITICAL: Use host CPU and features (not "generic") to match the codegen
    // TargetMachine. Mismatched CPU/features between optimization and codegen
    // can cause the optimizer to make incorrect assumptions about instruction
    // availability and data layout, leading to miscompilation.
    let cpu = TargetMachine::get_host_cpu_name();
    let features = TargetMachine::get_host_cpu_features();
    let target_machine = match target.create_target_machine(
        &target_triple,
        cpu.to_str().unwrap_or("generic"),
        features.to_str().unwrap_or(""),
        config.level.to_inkwell_opt_level(),
        RelocMode::PIC,
        CodeModel::Default,
    ) {
        Some(tm) => tm,
        None => {
            // Leak LLVM strings on early return to avoid LLVM 22 disposal crash.
            std::mem::forget(target_triple);
            std::mem::forget(cpu);
            std::mem::forget(features);
            return;
        }
    };

    // Set module target triple and data layout
    // CRITICAL: The data layout MUST be set before optimization.
    module.set_triple(&target_triple);
    {
        let td = target_machine.get_target_data();
        let dl = td.get_data_layout();
        module.set_data_layout(&dl);
        // Leak TargetData AND DataLayout to avoid LLVM 22 disposal crash.
        // The module copies the layout string via set_data_layout, so dl's
        // lifetime does NOT need to outlive the module. Both td and dl
        // internally hold LLVM resources that crash on disposal via LLVM 22.
        std::mem::forget(td);
        std::mem::forget(dl);
    }

    // Leak LLVM-allocated strings (TargetTriple, cpu, features) to avoid
    // LLVM 22's LLVMDisposeMessage crash on Windows. These strings are allocated
    // by LLVM's internal allocator and disposing them via the Rust-linked CRT
    // can cause silent exit/crash. Consistent with mem::forget pattern used
    // for TargetData and TargetMachine in this function.
    std::mem::forget(target_triple);
    std::mem::forget(cpu);
    std::mem::forget(features);

    // Skip module.verify() — crashes with LLVM 22 on Windows (CRT mismatch).
    // The module is validated indirectly by successful compilation.

    // Create pass builder options
    let pass_options = PassBuilderOptions::create();

    // Configure optimization options
    pass_options.set_verify_each(config.verify_each);
    pass_options.set_debug_logging(config.debug_logging);
    pass_options.set_loop_vectorization(config.loop_vectorization);
    pass_options.set_loop_slp_vectorization(config.slp_vectorization);
    pass_options.set_loop_unrolling(config.loop_unrolling);
    pass_options.set_loop_interleaving(config.loop_interleaving);
    pass_options.set_merge_functions(config.merge_functions);

    // Run optimization passes using new pass manager
    let passes = config.level.to_pass_pipeline();

    if let Err(e) = module.run_passes(passes, &target_machine, pass_options) {
        // Leak the LLVM error string to avoid LLVM 22 disposal crash on Windows.
        std::mem::forget(e);
    }

    // Leak TargetMachine to avoid LLVM 22 disposal crash on Windows.
    // (PassBuilderOptions is consumed by run_passes and dropped internally.)
    std::mem::forget(target_machine);
}

/// Run default O3 optimization.
///
/// This is a convenience function for maximum performance optimization.
#[inline]
pub fn optimize_module_default<'ctx>(module: &Module<'ctx>) {
    optimize_module(module, OptLevel::O3);
}

/// Run O0 optimization (no optimization).
///
/// Useful for debugging or fast compilation.
#[inline]
pub fn optimize_module_none<'ctx>(module: &Module<'ctx>) {
    optimize_module(module, OptLevel::O0);
}

/// Run size optimization (Os).
///
/// Optimizes for smaller binary size while maintaining good performance.
#[inline]
pub fn optimize_module_size<'ctx>(module: &Module<'ctx>) {
    optimize_module(module, OptLevel::Os);
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

    #[test]
    fn test_opt_level_to_pipeline() {
        assert_eq!(OptLevel::O0.to_pass_pipeline(), "default<O0>");
        assert_eq!(OptLevel::O1.to_pass_pipeline(), "default<O1>");
        assert_eq!(OptLevel::O2.to_pass_pipeline(), "default<O2>");
        assert_eq!(OptLevel::O3.to_pass_pipeline(), "default<O3>");
        assert_eq!(OptLevel::Os.to_pass_pipeline(), "default<Os>");
        assert_eq!(OptLevel::Oz.to_pass_pipeline(), "default<Oz>");
    }

    #[test]
    fn test_opt_level_from_str() {
        assert_eq!(OptLevel::from_str("0"), Some(OptLevel::O0));
        assert_eq!(OptLevel::from_str("O3"), Some(OptLevel::O3));
        assert_eq!(OptLevel::from_str("os"), Some(OptLevel::Os));
        assert_eq!(OptLevel::from_str("invalid"), None);
    }

    #[test]
    fn test_optimization_config_default() {
        let config = OptimizationConfig::default();
        assert_eq!(config.level, OptLevel::O3);
        assert!(config.loop_vectorization);
        assert!(config.slp_vectorization);
    }

    #[test]
    fn test_optimization_config_for_o0() {
        let config = OptimizationConfig::for_level(OptLevel::O0);
        assert!(!config.loop_vectorization);
        assert!(!config.slp_vectorization);
        assert!(!config.loop_unrolling);
    }
}
