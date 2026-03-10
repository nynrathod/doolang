//! Debug Info — DWARF/CodeView debug symbol generation via LLVM DIBuilder.
//!
//! Provides debug metadata so Doo programs can be debugged with gdb/lldb/VS Code.
//!
//! ## Architecture
//!
//! - `DebugInfo` struct wraps inkwell's `DebugInfoBuilder`
//! - Created per-module when debug symbols are requested (`--debug` flag)
//! - Tracks current file, compile unit, and function debug scopes
//! - Emits `DILocation` metadata for each instruction via `set_debug_location()`
//!
//! ## Usage in Codegen
//!
//! ```ignore
//! // Initialize debug info for a module:
//! let debug_info = DebugInfo::new(&module, "main.doo", "/project/src");
//!
//! // When entering a function:
//! debug_info.create_function(&ctx.context, "my_fn", 10);
//!
//! // Before each instruction:
//! debug_info.set_location(&ctx.builder, line, col);
//!
//! // Finalize at end of compilation:
//! debug_info.finalize();
//! ```

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::debug_info::{
    AsDIScope, DICompileUnit, DIFile, DIFlagsConstants, DIScope, DISubprogram, DWARFEmissionKind,
    DWARFSourceLanguage, DebugInfoBuilder,
};
use inkwell::module::Module;
use inkwell::values::FunctionValue;

/// DWARF debug info manager for a single compilation module.
///
/// Encapsulates all debug metadata state and provides a clean API
/// for the codegen layer to emit debug information.
pub struct DebugInfo<'ctx> {
    /// The LLVM debug info builder.
    pub builder: DebugInfoBuilder<'ctx>,
    /// The compile unit for this module.
    pub compile_unit: DICompileUnit<'ctx>,
    /// The primary source file.
    pub file: DIFile<'ctx>,
    /// Currently active debug scope (function or block).
    current_scope: Option<DIScope<'ctx>>,
    /// Currently active subprogram (for function-level debug info).
    current_subprogram: Option<DISubprogram<'ctx>>,
}

impl<'ctx> DebugInfo<'ctx> {
    /// Create debug info for a module.
    ///
    /// - `module`: The LLVM module to attach debug metadata to
    /// - `filename`: Source filename (e.g., "main.doo")
    /// - `directory`: Source directory (e.g., "/project/src")
    pub fn new(module: &Module<'ctx>, filename: &str, directory: &str) -> Self {
        let (builder, compile_unit) = module.create_debug_info_builder(
            /* allow_unresolved */ true,
            DWARFSourceLanguage::C, // Use C ABI for compatibility; register Doo later
            filename,
            directory,
            "doo", // producer
            false, // is_optimized (set true for release builds)
            "",    // compiler flags
            0,     // runtime version
            "",    // split name
            DWARFEmissionKind::Full,
            0,     // DWO id
            false, // split debug inlining
            false, // debug info for profiling
            "",    // sysroot
            "",    // SDK
        );

        let file = builder.create_file(filename, directory);

        Self {
            builder,
            compile_unit,
            file,
            current_scope: None,
            current_subprogram: None,
        }
    }

    /// Create debug info for a function.
    ///
    /// Must be called before emitting instructions for the function.
    /// Sets the current debug scope to this function's subprogram.
    pub fn create_function(
        &mut self,
        context: &'ctx Context,
        func: FunctionValue<'ctx>,
        name: &str,
        line: u32,
    ) {
        // Create a basic subroutine type (void function for now)
        let subroutine_type = self.builder.create_subroutine_type(
            self.file,
            /* return type */ None,
            /* param types */ &[],
            /* flags */ DIFlagsConstants::ZERO,
        );

        let subprogram = self.builder.create_function(
            self.file.as_debug_info_scope(),
            name,
            /* linkage_name */ Some(name),
            self.file,
            line,
            subroutine_type,
            /* is_local_to_unit */ true,
            /* is_definition */ true,
            /* scope_line */ line,
            /* flags */ DIFlagsConstants::ZERO,
            /* is_optimized */ false,
        );

        func.set_subprogram(subprogram);
        self.current_subprogram = Some(subprogram);
        self.current_scope = Some(subprogram.as_debug_info_scope());
    }

    /// Set the debug location for the next instruction(s).
    ///
    /// Should be called before each instruction emission with the source
    /// line/column from the MIR instruction's span.
    pub fn set_location(
        &self,
        _context: &'ctx Context,
        builder: &Builder<'ctx>,
        line: u32,
        col: u32,
    ) {
        if let Some(scope) = self.current_scope {
            let location = self
                .builder
                .create_debug_location(_context, line, col, scope, /* inlined_at */ None);
            builder.set_current_debug_location(location);
        }
    }

    /// Clear the current debug location (for synthesized code).
    pub fn clear_location(&self, builder: &Builder<'ctx>) {
        builder.unset_current_debug_location();
    }

    /// Finalize debug info. Must be called after all code generation is complete.
    ///
    /// This writes the final debug metadata to the LLVM module.
    pub fn finalize(&self) {
        self.builder.finalize();
    }
}
