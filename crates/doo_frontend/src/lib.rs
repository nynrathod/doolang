//! # Doo Frontend
//!
//! Lexer, parser, and AST for the Doo language.
//!
//! ## Architecture
//!
//! - `lexer/` - Tokenizes source code into tokens with spans
//! - `ast/` - Abstract syntax tree definitions
//! - `parser/` - Parses tokens into AST

pub mod lexer;
pub mod ast;
pub mod parser;

pub use lexer::{Token, TokenKind, Lexer};
pub use ast::{Program, Item, Expr, ExprKind, Stmt, StmtKind};
pub use parser::Parser;
