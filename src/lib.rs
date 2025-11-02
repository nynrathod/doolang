// Doo Compiler Library
// Exports all compiler modules for testing and external use

pub mod analyzer;
pub mod codegen;
pub mod compiler;
pub mod diagnostics;
pub mod lexar;
pub mod limits;
pub mod mir;
pub mod parser;

// Re-export commonly used types
pub use analyzer::SemanticAnalyzer;
pub use codegen::core::CodeGen;
pub use lexar::lexer::lex;
pub use lexar::token::{Token, TokenType};
pub use mir::builder::MirBuilder;
pub use parser::ast::AstNode;
pub use parser::Parser;
