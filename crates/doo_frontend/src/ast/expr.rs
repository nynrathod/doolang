//! Expression AST nodes.
//!
//! All expressions that produce values.

use super::TypeExpr;
use crate::lexer::TokenKind;
use doo_core::Span;

/// An expression that produces a value.
#[derive(Debug, Clone)]
pub struct Expr {
    /// The expression kind.
    pub kind: ExprKind,
    /// Source location.
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Whether this expression ends with `}` (block-like, no semicolon needed).
    pub fn ends_with_brace(&self) -> bool {
        matches!(
            self.kind,
            ExprKind::IfExpr { .. }
                | ExprKind::Match { .. }
                | ExprKind::Block(_, _)
                | ExprKind::StructLit { .. }
                | ExprKind::RouteBlock { .. }
                | ExprKind::GoSpawn { .. }
                | ExprKind::ScopeBlock { .. }
        )
    }
}

/// Expression variants.
#[derive(Debug, Clone)]
pub enum ExprKind {
    // === Literals ===
    /// Integer literal: `123`
    IntLit(i64),
    /// Float literal: `3.14`
    FloatLit(f64),
    /// Boolean literal: `true`, `false`
    BoolLit(bool),
    /// String literal: `"hello"`
    StrLit(String),
    /// String interpolation: `"Hello ${name}"`
    /// Stores alternating parts: [str, expr, str, expr, str, ...]
    StringInterpolation(Vec<StringPart>),
    /// Nil literal: `nil`
    Nil,

    // === Compound Literals ===
    /// Array literal: `[1, 2, 3]`
    ArrayLit(Vec<Expr>),
    /// Map literal: `{a: 1, b: 2}`
    MapLit(Vec<(Expr, Expr)>),
    /// Object/struct literal: `{name: "foo", age: 30}`
    ObjectLit(Vec<(String, Expr)>),
    /// Tuple literal: `(1, "a", true)`
    TupleLit(Vec<Expr>),
    /// Spread expression: `...arr`
    Spread(Box<Expr>),

    // === Identifiers ===
    /// Variable reference: `foo`
    Ident(String),

    // === Operations ===
    /// Binary operation: `a + b`
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
    },
    /// Unary operation: `!x`, `-y`
    Unary { op: UnaryOp, expr: Box<Expr> },

    // === Access ===
    /// Field access: `obj.field`
    Field { object: Box<Expr>, field: String },
    /// Index access: `arr[0]`
    Index { object: Box<Expr>, index: Box<Expr> },

    // === Calls ===
    /// Function call: `foo(a, b)`
    Call { func: Box<Expr>, args: Vec<Expr> },
    /// Method call: `obj.method(args)`
    MethodCall {
        object: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },

    // === Control Flow Expressions ===
    /// Conditional expression: `if cond { a } else { b }`
    IfExpr {
        condition: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Option<Box<Expr>>,
    },
    /// Ternary expression: `cond ? a : b`
    Ternary {
        condition: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },
    /// Match expression
    Match {
        values: Vec<Expr>,
        arms: Vec<MatchArm>,
    },
    /// Block expression: `{ stmts; expr }`
    Block(Vec<super::Stmt>, Option<Box<Expr>>),

    // === Range ===
    /// Range: `1..10` or `1..=10`
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
        inclusive: bool,
    },

    // === Error Handling ===
    /// Ok wrapper: `Ok(value)`
    Ok(Vec<Expr>),
    /// Err wrapper: `Err(error)`
    Err(Box<Expr>),
    /// Try operator: `expr?`
    Try(Box<Expr>),
    /// Unwrap or panic: `expr! "message"`
    UnwrapOrPanic { expr: Box<Expr>, message: Box<Expr> },

    // === Struct/Enum Construction ===
    /// Struct literal: `User { name: "foo" }`
    StructLit {
        name: String,
        fields: Vec<(String, Expr)>,
    },
    /// Enum variant: `Status.Active` or `Result.Ok(value)`
    EnumVariant {
        enum_name: String,
        variant: String,
        payload: Vec<Expr>,
    },

    // === Cast ===
    /// Type cast: `x as Int`
    Cast { expr: Box<Expr>, target: TypeExpr },

    // === Closure ===
    /// Closure: `(x) => x + 1`
    Closure {
        params: Vec<(String, Option<TypeExpr>)>,
        body: Box<Expr>,
        return_type: Option<TypeExpr>,
        error_type: Option<TypeExpr>,
    },

    // === HTTP Route Block ===
    /// Route block: `{ get("/path", Handler), post("/path", Handler) }`
    /// Used in app.group() for inline route definitions
    RouteBlock { routes: Vec<Expr> },

    // === Async & Concurrency ===
    /// Await expression: `await expr`
    Await(Box<Expr>),
    /// Go spawn: `go { ... }` — spawns a concurrent task, returns TaskHandle
    GoSpawn { body: Box<Expr> },
    /// Structured scope: `scope { ... }` — all inner `go` blocks joined before exit
    ScopeBlock { body: Vec<super::Stmt> },
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    // Comparison
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    In,
    // Logical
    And,
    Or,
    // Bitwise
    BitAnd,
    BitOr,
    // Null coalescing
    NullCoalesce,
}

impl std::fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Add => write!(f, "+"),
            Self::Sub => write!(f, "-"),
            Self::Mul => write!(f, "*"),
            Self::Div => write!(f, "/"),
            Self::Mod => write!(f, "%"),
            Self::Eq => write!(f, "=="),
            Self::NotEq => write!(f, "!="),
            Self::Lt => write!(f, "<"),
            Self::Gt => write!(f, ">"),
            Self::LtEq => write!(f, "<="),
            Self::GtEq => write!(f, ">="),
            Self::In => write!(f, "in"),
            Self::And => write!(f, "&&"),
            Self::Or => write!(f, "||"),
            Self::BitAnd => write!(f, "&"),
            Self::BitOr => write!(f, "|"),
            Self::NullCoalesce => write!(f, "??"),
        }
    }
}

impl BinaryOp {
    /// Get precedence (higher = binds tighter).
    pub fn precedence(&self) -> u8 {
        match self {
            Self::Or => 1,
            Self::And => 2,
            Self::Eq | Self::NotEq => 3,
            Self::Lt | Self::Gt | Self::LtEq | Self::GtEq | Self::In => 4,
            Self::NullCoalesce => 5,
            Self::BitOr => 6,
            Self::BitAnd => 7,
            Self::Add | Self::Sub => 8,
            Self::Mul | Self::Div | Self::Mod => 9,
        }
    }

    /// Create from token kind.
    pub fn from_token(kind: TokenKind) -> Option<Self> {
        match kind {
            TokenKind::Plus => Some(Self::Add),
            TokenKind::Minus => Some(Self::Sub),
            TokenKind::Star => Some(Self::Mul),
            TokenKind::Slash => Some(Self::Div),
            TokenKind::Percent => Some(Self::Mod),
            TokenKind::EqEq => Some(Self::Eq),
            TokenKind::NotEq => Some(Self::NotEq),
            TokenKind::Lt => Some(Self::Lt),
            TokenKind::Gt => Some(Self::Gt),
            TokenKind::LtEq => Some(Self::LtEq),
            TokenKind::GtEq => Some(Self::GtEq),
            TokenKind::In => Some(Self::In),
            TokenKind::AndAnd => Some(Self::And),
            TokenKind::OrOr => Some(Self::Or),
            TokenKind::And => Some(Self::BitAnd),
            TokenKind::Or => Some(Self::BitOr),
            TokenKind::QuestionQuestion => Some(Self::NullCoalesce),
            _ => None,
        }
    }
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// Negation: `-x`
    Neg,
    /// Logical not: `!x`
    Not,
}

impl std::fmt::Display for UnaryOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Neg => write!(f, "-"),
            Self::Not => write!(f, "!"),
        }
    }
}

impl UnaryOp {
    pub fn from_token(kind: TokenKind) -> Option<Self> {
        match kind {
            TokenKind::Minus => Some(Self::Neg),
            TokenKind::Bang => Some(Self::Not),
            _ => None,
        }
    }
}

/// A match arm.
#[derive(Debug, Clone)]
pub struct MatchArm {
    /// Pattern to match.
    pub pattern: MatchPattern,
    /// Optional guard: `if cond`
    pub guard: Option<Expr>,
    /// Body expression.
    pub body: Expr,
    /// Span.
    pub span: Span,
}

/// Patterns in match expressions.
#[derive(Debug, Clone)]
pub enum MatchPattern {
    /// Literal pattern: `200`, `"ok"`
    Literal(Box<Expr>),
    /// Condition pattern: `x < 10`
    Condition(Box<Expr>),
    /// Wildcard: `_`
    Wildcard,
    /// Enum variant: `Status.Active`
    EnumVariant { enum_name: String, variant: String },
    /// Enum variant with payload binding: `Result.Ok(value)`
    EnumVariantPayload {
        enum_name: String,
        variant: String,
        bindings: Vec<String>,
    },
    /// Tuple pattern: `(a, b, c)`
    Tuple(Vec<MatchPattern>),
}

/// Part of a string interpolation.
#[derive(Debug, Clone)]
pub enum StringPart {
    /// Literal string piece: `"Hello "`
    Literal(String),
    /// Interpolated expression: `${name}`
    Expr(Box<Expr>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binary_op_precedence() {
        assert!(BinaryOp::Mul.precedence() > BinaryOp::Add.precedence());
        assert!(BinaryOp::And.precedence() > BinaryOp::Or.precedence());
    }

    #[test]
    fn test_binary_op_from_token() {
        assert_eq!(BinaryOp::from_token(TokenKind::Plus), Some(BinaryOp::Add));
        assert_eq!(BinaryOp::from_token(TokenKind::EqEq), Some(BinaryOp::Eq));
        assert_eq!(BinaryOp::from_token(TokenKind::Ident), None);
    }
}
