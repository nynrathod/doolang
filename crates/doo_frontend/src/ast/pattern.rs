//! Pattern AST nodes.
//!
//! Patterns for destructuring in let bindings and match expressions.

use doo_core::Span;

/// A pattern for destructuring.
#[derive(Debug, Clone)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

impl Pattern {
    pub fn new(kind: PatternKind, span: Span) -> Self {
        Self { kind, span }
    }

    pub fn ident(name: String, span: Span) -> Self {
        Self::new(PatternKind::Ident(name), span)
    }

    pub fn wildcard(span: Span) -> Self {
        Self::new(PatternKind::Wildcard, span)
    }
}

/// Pattern variants.
#[derive(Debug, Clone)]
pub enum PatternKind {
    /// Identifier binding: `x`
    Ident(String),
    /// Tuple destructuring: `(a, b, c)`
    Tuple(Vec<Pattern>),
    /// Wildcard: `_`
    Wildcard,
    /// Index pattern for array assignment: `arr[i]`
    Index {
        object: Box<Pattern>,
        index: Box<crate::ast::expr::Expr>,
    },
    /// Field pattern for field assignment: `obj.field` or `self.Users`
    Field { object: Box<Pattern>, field: String },

    /// OR pattern: `A | B`
    Or(Vec<Pattern>),

    /// `@` binding: `name @ Pattern`
    Bind { name: String, pattern: Box<Pattern> },
}
