//! THIR Pattern Nodes
//!
//! Used in match arms and destructuring let bindings.

use doo_core::types::TypeId;
use doo_core::Span;

use crate::expr::ThirLiteral;

/// A THIR pattern with resolved types on bindings.
#[derive(Debug, Clone)]
pub struct ThirPattern {
    pub kind: ThirPatternKind,
    pub ty: Option<TypeId>,
    pub span: Span,
}

/// THIR pattern kinds.
#[derive(Debug, Clone)]
pub enum ThirPatternKind {
    /// `_`
    Wildcard,

    /// `name` or `mut name`
    Ident(String, bool),

    /// Literal pattern: `200`, `"ok"`
    Literal(ThirLiteral),

    /// Struct pattern: `Point { x, y: 10 }`
    Struct {
        name: String,
        fields: Vec<(String, ThirPattern)>,
    },

    /// Enum variant: `Status::Active` or `Result::Ok(val)`
    Enum {
        name: String,
        variant: String,
        payload: Option<Box<ThirPattern>>,
    },

    /// Tuple pattern: `(a, b, c)`
    Tuple(Vec<ThirPattern>),

    /// Array pattern: `[a, b, ..rest]`
    Array(Vec<ThirPattern>),

    /// Rest pattern: `..` or `..rest`
    Rest(Option<String>),

    /// Condition pattern: `x < 10`
    Condition(Box<crate::expr::ThirExpr>),
}
