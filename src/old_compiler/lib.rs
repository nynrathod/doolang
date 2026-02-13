// Doo Compiler Library
// LEGACY MODULES DISABLED - Using new crates-based compiler
// See: crates/doo_driver, crates/doo_frontend, crates/doo_codegen, etc.

#[macro_export]
macro_rules! doo_debug {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) || std::env::var("DOO_DEBUG").is_ok() {
             eprintln!("[COMPILER] {}", format!($($arg)*));
        }
    }
}

// LEGACY MODULES - DISABLED
// These modules are kept for reference but are no longer used.
// The new compiler lives in the crates/ directory.
// pub mod analyzer;  // -> crates/doo_analysis
// pub mod codegen;   // -> crates/doo_codegen
// pub mod compiler;  // -> crates/doo_driver/src/compile.rs
// pub mod lexer;     // -> crates/doo_frontend
// pub mod mir;       // -> crates/doo_mir
// pub mod parser;    // -> crates/doo_frontend

// Keep only essential runtime/utility modules
// pub mod debug;
// pub mod diagnostics;  // -> crates/doo_diagnostics
// pub mod limits;
// pub mod path_resolver;
// pub mod runtime;
