// Doo Compiler Library
// Exports all compiler modules for testing and external use

#[macro_export]
macro_rules! doo_debug {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) || std::env::var("DOO_DEBUG").is_ok() {
             eprintln!("[COMPILER] {}", format!($($arg)*));
        }
    }
}

pub mod analyzer;
pub mod codegen;
pub mod compiler;
pub mod debug;
pub mod diagnostics;
pub mod lexer;
pub mod limits;
pub mod mir;
pub mod parser;
pub mod path_resolver;
pub mod runtime;

// Re-export commonly used types
pub use analyzer::SemanticAnalyzer;
pub use codegen::core::CodeGen;
pub use lexer::lexer::lex;
pub use lexer::token::{Token, TokenType};
pub use mir::builder::MirBuilder;
pub use parser::ast::AstNode;
pub use parser::Parser;
