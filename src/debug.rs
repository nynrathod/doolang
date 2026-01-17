//! Compiler debug utilities for Doo
//!
//! This module provides debug macros for the compiler itself (analyzer, MIR, codegen).
//! Enable with `DOO_COMPILER_DEBUG=1` environment variable.
//!
//! Usage:
//! ```
//! doo_compiler_debug!("Analyzer", "Analyzing function: {}", name);
//! doo_mir_debug!("Lowering function: {}", name);
//! doo_codegen_debug!("Generating LLVM for: {}", node_type);
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// Cached debug flag - checked once at startup
static DEBUG_ENABLED: OnceLock<bool> = OnceLock::new();

/// Check if compiler debug is enabled (cached after first check)
#[inline]
pub fn is_compiler_debug_enabled() -> bool {
    *DEBUG_ENABLED
        .get_or_init(|| std::env::var("DOO_COMPILER_DEBUG").is_ok() || cfg!(debug_assertions))
}

/// Central debug macro for compiler components
///
/// Args:
/// - $component: Component name (e.g., "Analyzer", "MIR", "Codegen")
/// - $($arg:tt)*: Format string and arguments
#[macro_export]
macro_rules! doo_compiler_debug {
    ($component:expr, $($arg:tt)*) => {
        if $crate::debug::is_compiler_debug_enabled() {
            eprintln!("[DOO::COMPILER::{}] {}", $component, format!($($arg)*));
        }
    };
}

/// Debug macro for analyzer operations
#[macro_export]
macro_rules! doo_analyzer_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Analyzer", $($arg)*);
    };
}

/// Debug macro for MIR lowering operations
#[macro_export]
macro_rules! doo_mir_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("MIR", $($arg)*);
    };
}

/// Debug macro for codegen operations
#[macro_export]
macro_rules! doo_codegen_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Codegen", $($arg)*);
    };
}

/// Debug macro for parser operations
#[macro_export]
macro_rules! doo_parser_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Parser", $($arg)*);
    };
}

/// Debug macro for type checking operations
#[macro_export]
macro_rules! doo_type_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Type", $($arg)*);
    };
}

/// Debug macro for import/module resolution
#[macro_export]
macro_rules! doo_import_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Import", $($arg)*);
    };
}

/// Debug macro for memory/RC operations in codegen
#[macro_export]
macro_rules! doo_rc_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("RC", $($arg)*);
    };
}

/// Debug macro for FFI code generation
#[macro_export]
macro_rules! doo_ffi_codegen_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("FFI", $($arg)*);
    };
}

/// Trace macro for entering a function/phase (more verbose)
#[macro_export]
macro_rules! doo_trace_enter {
    ($phase:expr) => {
        if $crate::debug::is_compiler_debug_enabled() {
            eprintln!("[DOO::TRACE] >> ENTER {}", $phase);
        }
    };
    ($phase:expr, $($arg:tt)*) => {
        if $crate::debug::is_compiler_debug_enabled() {
            eprintln!("[DOO::TRACE] >> ENTER {} ({})", $phase, format!($($arg)*));
        }
    };
}

/// Trace macro for exiting a function/phase
#[macro_export]
macro_rules! doo_trace_exit {
    ($phase:expr) => {
        if $crate::debug::is_compiler_debug_enabled() {
            eprintln!("[DOO::TRACE] << EXIT  {}", $phase);
        }
    };
    ($phase:expr, $($arg:tt)*) => {
        if $crate::debug::is_compiler_debug_enabled() {
            eprintln!("[DOO::TRACE] << EXIT  {} ({})", $phase, format!($($arg)*));
        }
    };
}

/// Debug macro for struct operations (declaration, field access, instantiation)
#[macro_export]
macro_rules! doo_struct_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Struct", $($arg)*);
    };
}

/// Debug macro for loop operations (for, while, break, continue)
#[macro_export]
macro_rules! doo_loop_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Loop", $($arg)*);
    };
}

/// Debug macro for method resolution and calls
#[macro_export]
macro_rules! doo_method_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Method", $($arg)*);
    };
}

/// Debug macro for decorator processing
#[macro_export]
macro_rules! doo_decorator_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Decorator", $($arg)*);
    };
}

/// Debug macro for error handling (Result, try/catch, ?)
#[macro_export]
macro_rules! doo_error_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Error", $($arg)*);
    };
}

/// Debug macro for block/scope operations
#[macro_export]
macro_rules! doo_block_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Block", $($arg)*);
    };
}

/// Debug macro for instruction generation
#[macro_export]
macro_rules! doo_instr_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Instr", $($arg)*);
    };
}

/// Debug macro for expression evaluation
#[macro_export]
macro_rules! doo_expr_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Expr", $($arg)*);
    };
}

/// Debug macro for warnings (non-fatal issues)
#[macro_export]
macro_rules! doo_warn {
    ($($arg:tt)*) => {
        if $crate::debug::is_compiler_debug_enabled() {
            eprintln!("[DOO::WARN] {}", format!($($arg)*));
        }
    };
}

/// Debug macro for errors (fatal issues during compilation)
#[macro_export]
macro_rules! doo_err {
    ($($arg:tt)*) => {
        eprintln!("[DOO::ERROR] {}", format!($($arg)*));
    };
}

/// Debug macro for validation operations
#[macro_export]
macro_rules! doo_validate_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Validate", $($arg)*);
    };
}

/// Debug macro for optimization passes
#[macro_export]
macro_rules! doo_opt_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Opt", $($arg)*);
    };
}

/// Debug macro for route/HTTP generation
#[macro_export]
macro_rules! doo_route_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Route", $($arg)*);
    };
}

/// Debug macro for database operations
#[macro_export]
macro_rules! doo_db_codegen_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("DB", $($arg)*);
    };
}

/// Debug macro for closure generation
#[macro_export]
macro_rules! doo_closure_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Closure", $($arg)*);
    };
}

/// Debug macro for linker operations
#[macro_export]
macro_rules! doo_linker_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Linker", $($arg)*);
    };
}

/// Assertion macro for compiler invariants - always logs if assertion fails
#[macro_export]
macro_rules! doo_assert {
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            eprintln!("[DOO::ASSERT FAILED] {}", format!($($arg)*));
            if cfg!(debug_assertions) {
                panic!("Compiler invariant violated: {}", format!($($arg)*));
            }
        }
    };
}

/// Debug macro for memory corruption detection (pointer validation)
#[macro_export]
macro_rules! doo_mem_check {
    ($ptr:expr, $context:expr) => {
        if $crate::debug::is_compiler_debug_enabled() {
            if $ptr.is_null() {
                eprintln!("[DOO::MEM::CHECK] WARN ptr=NULL context={}", $context);
            } else {
                let addr = $ptr as usize;
                if addr < 0x1000 {
                    eprintln!(
                        "[DOO::MEM::CHECK] CORRUPT ptr={:p} (too low) context={}",
                        $ptr, $context
                    );
                } else if addr % 8 != 0 {
                    eprintln!(
                        "[DOO::MEM::CHECK] MISALIGN ptr={:p} (alignment) context={}",
                        $ptr, $context
                    );
                } else {
                    eprintln!("[DOO::MEM::CHECK] OK ptr={:p} context={}", $ptr, $context);
                }
            }
        }
    };
}

/// Debug macro for LLVM value operations
#[macro_export]
macro_rules! doo_llvm_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("LLVM", $($arg)*);
    };
}

/// Debug macro for reference counting operations in codegen
#[macro_export]
macro_rules! doo_refcount_debug {
    ($op:expr, $ptr:expr, $context:expr) => {
        if $crate::debug::is_compiler_debug_enabled() {
            eprintln!(
                "[DOO::REFCOUNT] {} ptr={:?} context={}",
                $op, $ptr, $context
            );
        }
    };
}

/// Debug macro for string operations (creation, manipulation, freeing)
#[macro_export]
macro_rules! doo_string_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("String", $($arg)*);
    };
}

/// Debug macro for array/collection operations
#[macro_export]
macro_rules! doo_array_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Array", $($arg)*);
    };
}

/// Debug macro for HashMap operations
#[macro_export]
macro_rules! doo_hashmap_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("HashMap", $($arg)*);
    };
}

/// Debug macro for function call generation
#[macro_export]
macro_rules! doo_call_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Call", $($arg)*);
    };
}

/// Debug macro for variable/register operations
#[macro_export]
macro_rules! doo_var_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Var", $($arg)*);
    };
}

/// Debug macro for phi node generation
#[macro_export]
macro_rules! doo_phi_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Phi", $($arg)*);
    };
}

/// Debug macro for basic block operations
#[macro_export]
macro_rules! doo_bb_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("BasicBlock", $($arg)*);
    };
}

/// Debug macro for branching/control flow
#[macro_export]
macro_rules! doo_branch_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Branch", $($arg)*);
    };
}

/// Debug macro for return operations
#[macro_export]
macro_rules! doo_return_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Return", $($arg)*);
    };
}

/// Debug macro for allocation operations
#[macro_export]
macro_rules! doo_alloc_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Alloc", $($arg)*);
    };
}

/// Debug macro for free/deallocation operations
#[macro_export]
macro_rules! doo_free_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Free", $($arg)*);
    };
}

/// Debug macro for GEP (GetElementPtr) operations
#[macro_export]
macro_rules! doo_gep_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("GEP", $($arg)*);
    };
}

/// Debug macro for load operations
#[macro_export]
macro_rules! doo_load_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Load", $($arg)*);
    };
}

/// Debug macro for store operations
#[macro_export]
macro_rules! doo_store_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Store", $($arg)*);
    };
}

/// Debug macro for cast operations
#[macro_export]
macro_rules! doo_cast_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Cast", $($arg)*);
    };
}

/// Debug macro for binary operations
#[macro_export]
macro_rules! doo_binop_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("BinOp", $($arg)*);
    };
}

/// Debug macro for comparison operations
#[macro_export]
macro_rules! doo_cmp_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Cmp", $($arg)*);
    };
}

/// Debug macro for pattern matching
#[macro_export]
macro_rules! doo_match_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Match", $($arg)*);
    };
}

/// Debug macro for module/namespace operations
#[macro_export]
macro_rules! doo_module_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Module", $($arg)*);
    };
}

/// Debug macro for symbol resolution
#[macro_export]
macro_rules! doo_symbol_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Symbol", $($arg)*);
    };
}

/// Debug macro for lifetime/scope tracking
#[macro_export]
macro_rules! doo_lifetime_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Lifetime", $($arg)*);
    };
}

/// Memory region tracking for corruption detection
#[macro_export]
macro_rules! doo_mem_region {
    ($op:expr, $ptr:expr, $size:expr, $context:expr) => {
        if $crate::debug::is_compiler_debug_enabled() {
            eprintln!(
                "[DOO::MEM::REGION] {} ptr={:p} size={} context={}",
                $op, $ptr, $size, $context
            );
        }
    };
}

/// Stack frame tracking for debugging call stacks
#[macro_export]
macro_rules! doo_frame_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Frame", $($arg)*);
    };
}

/// Detailed function entry tracking with parameters
#[macro_export]
macro_rules! doo_func_enter {
    ($name:expr) => {
        if $crate::debug::is_compiler_debug_enabled() {
            eprintln!("[DOO::FUNC] >> ENTER {}", $name);
        }
    };
    ($name:expr, $($arg:tt)*) => {
        if $crate::debug::is_compiler_debug_enabled() {
            eprintln!("[DOO::FUNC] >> ENTER {} params=({})", $name, format!($($arg)*));
        }
    };
}

/// Detailed function exit tracking with return value
#[macro_export]
macro_rules! doo_func_exit {
    ($name:expr) => {
        if $crate::debug::is_compiler_debug_enabled() {
            eprintln!("[DOO::FUNC] << EXIT  {}", $name);
        }
    };
    ($name:expr, $($arg:tt)*) => {
        if $crate::debug::is_compiler_debug_enabled() {
            eprintln!("[DOO::FUNC] << EXIT  {} return=({})", $name, format!($($arg)*));
        }
    };
}

/// Type coercion/conversion tracking
#[macro_export]
macro_rules! doo_coerce_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Coerce", $($arg)*);
    };
}

/// Metadata extraction/processing debug
#[macro_export]
macro_rules! doo_metadata_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Metadata", $($arg)*);
    };
}

/// Constant folding/evaluation debug
#[macro_export]
macro_rules! doo_const_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Const", $($arg)*);
    };
}

/// Inline/optimization decision tracking
#[macro_export]
macro_rules! doo_inline_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Inline", $($arg)*);
    };
}

/// Generic trait/interface resolution
#[macro_export]
macro_rules! doo_trait_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Trait", $($arg)*);
    };
}

/// Panic/abort generation tracking
#[macro_export]
macro_rules! doo_panic_debug {
    ($($arg:tt)*) => {
        $crate::doo_compiler_debug!("Panic", $($arg)*);
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_check() {
        // Just ensure it compiles and doesn't panic
        let _ = is_compiler_debug_enabled();
    }
}
