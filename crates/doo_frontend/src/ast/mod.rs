//! Abstract Syntax Tree (AST) top-level module for Doo.
//!
//! The AST represents the syntactic structure of Doo programs after parsing.
//! All nodes carry span information for precise error reporting.

pub mod decl;
pub mod expr;
pub mod pattern;
pub mod stmt;
pub mod types;

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
    /// RBAC policy block (Framework domain - slated for removal in Audit Phase 2)
    Policy(PolicyDecl),
    /// Impl block for struct methods
    Impl(ImplDecl),
    /// Standalone statement (for scripting mode)
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
            Self::Impl(i) => i.span,
            Self::Statement(s) => s.span,
        }
    }

    /// Whether this top-level item requires a trailing semicolon.
    /// Items ending with `}` (functions, structs, enums, etc.) do not need `;`.
    /// For standalone statements, delegates to StmtKind::needs_semicolon()
    /// so that block-ending statements (if, for) also don't require `;`.
    pub fn needs_semicolon(&self) -> bool {
        match self {
            Self::Const(_) | Self::Static(_) | Self::Import(_) => true,
            Self::Statement(stmt) => stmt.kind.needs_semicolon(),
            Self::Function(_)
            | Self::Struct(_)
            | Self::Enum(_)
            | Self::Interface(_)
            | Self::Policy(_)
            | Self::Impl(_) => false,
        }
    }
}
