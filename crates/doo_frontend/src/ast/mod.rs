//! Abstract Syntax Tree for Doo.
//!
//! The AST represents the syntactic structure of Doo programs after parsing.
//! All nodes are typed with spans for precise error reporting.

mod decl;
mod expr;
mod pattern;
mod stmt;
mod types;

pub use decl::*;
pub use expr::*;
pub use pattern::*;
pub use stmt::*;
pub use types::*;

use doo_core::Span;

/// A complete program (module) with all its items.
#[derive(Debug, Clone)]
pub struct Program {
    /// All top-level items in the program.
    pub items: Vec<Item>,
    /// Span covering the entire file.
    pub span: Span,
}

impl Program {
    pub fn new(items: Vec<Item>, span: Span) -> Self {
        Self { items, span }
    }
}

/// Top-level items in a program.
#[derive(Debug, Clone)]
pub enum Item {
    /// Compile-time constant declaration
    Const(ConstDecl),
    /// Runtime global variable declaration (OnceLock semantics)
    Static(StaticDecl),
    /// Function declaration
    Function(FunctionDecl),
    /// Struct declaration
    Struct(StructDecl),
    /// Enum declaration
    Enum(EnumDecl),
    /// Interface declaration
    Interface(InterfaceDecl),
    /// Import statement
    Import(ImportDecl),
    /// RBAC policy block
    Policy(PolicyDecl),
    /// Standalone statement (for scripting)
    Statement(Stmt),
}

impl Item {
    pub fn span(&self) -> Span {
        match self {
            Self::Const(c) => c.span,
            Self::Static(s) => s.span,
            Self::Function(f) => f.span,
            Self::Struct(s) => s.span,
            Self::Enum(e) => e.span,
            Self::Interface(i) => i.span,
            Self::Import(i) => i.span,
            Self::Policy(p) => p.span,
            Self::Statement(s) => s.span(),
        }
    }
}
