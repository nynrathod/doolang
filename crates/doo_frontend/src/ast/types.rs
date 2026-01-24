//! Type expression AST nodes.
//!
//! Type annotations as they appear in source code.

use doo_core::Span;

/// A type expression (as written in source).
#[derive(Debug, Clone)]
pub struct TypeExpr {
    pub kind: TypeExprKind,
    pub span: Span,
}

impl TypeExpr {
    pub fn new(kind: TypeExprKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// Type expression variants.
#[derive(Debug, Clone)]
pub enum TypeExprKind {
    /// Simple type name: `Int`, `Str`, `User`
    Named(String),
    /// Array type: `[Int]`
    Array(Box<TypeExpr>),
    /// Map type: `{Str: Int}`
    Map(Box<TypeExpr>, Box<TypeExpr>),
    /// Tuple type: `(Int, Str, Bool)`
    Tuple(Vec<TypeExpr>),
    /// Optional type: `Int?`
    Optional(Box<TypeExpr>),
    /// Result type: `Result<Int, Error>`
    Result(Box<TypeExpr>, Box<TypeExpr>),
    /// Function type: `(Int, Int) -> Int`
    Function {
        params: Vec<TypeExpr>,
        returns: Box<TypeExpr>,
    },
    /// Range type (inferred from usage)
    Range(Box<TypeExpr>),
    /// Any type (dynamic)
    Any,
    /// Void type (no value)
    Void,
    /// Error type placeholder
    Error,
}

impl TypeExpr {
    /// Create a named type.
    pub fn named(name: impl Into<String>, span: Span) -> Self {
        Self::new(TypeExprKind::Named(name.into()), span)
    }

    /// Create an array type.
    pub fn array(element: TypeExpr, span: Span) -> Self {
        Self::new(TypeExprKind::Array(Box::new(element)), span)
    }

    /// Create an optional type.
    pub fn optional(inner: TypeExpr, span: Span) -> Self {
        Self::new(TypeExprKind::Optional(Box::new(inner)), span)
    }

    /// Create void type.
    pub fn void(span: Span) -> Self {
        Self::new(TypeExprKind::Void, span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_expr_creation() {
        let span = Span::dummy();
        let int_type = TypeExpr::named("Int", span);
        assert!(matches!(&int_type.kind, TypeExprKind::Named(s) if s == "Int"));

        let arr_type = TypeExpr::array(int_type, span);
        assert!(matches!(&arr_type.kind, TypeExprKind::Array(_)));
    }
}
