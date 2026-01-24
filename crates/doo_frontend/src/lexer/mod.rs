//! # Lexer Module
//!
//! Tokenizes Doo source code into a stream of tokens with span information.

mod token;
mod lexer;

pub use token::{Token, TokenKind};
pub use lexer::Lexer;
