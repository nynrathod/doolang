//! THIR Expression Nodes
//!
//! Every expression explicitly carries its resolved `TypeId`.

use doo_core::types::TypeId;
use doo_core::Span;

use crate::pattern::ThirPattern;
use crate::stmt::ThirStmt;
use crate::types::ImplResolution;
use crate::types::ThirCapture;

/// A THIR expression with resolved type.
#[derive(Debug, Clone)]
pub struct ThirExpr {
    pub kind: ThirExprKind,
    pub ty: TypeId,
    pub span: Span,
}

impl ThirExpr {
    pub fn new(kind: ThirExprKind, ty: TypeId, span: Span) -> Self {
        Self { kind, ty, span }
    }
}

/// THIR expression kinds (fully desugared and resolved).
#[derive(Debug, Clone)]
pub enum ThirExprKind {
    // === Literals ===
    Literal(ThirLiteral),

    // === Variables ===
    Var(String),

    // === Operations ===
    Binary {
        op: ThirBinOp,
        lhs: Box<ThirExpr>,
        rhs: Box<ThirExpr>,
    },
    Unary {
        op: ThirUnOp,
        expr: Box<ThirExpr>,
    },

    // === Calls ===
    Call {
        func: Box<ThirExpr>,
        args: Vec<ThirExpr>,
    },
    MethodCall {
        receiver: Box<ThirExpr>,
        method: String,
        resolved_impl: ImplResolution,
        args: Vec<ThirExpr>,
    },

    // === Access ===
    FieldAccess {
        object: Box<ThirExpr>,
        field: String,
        field_idx: usize,
    },
    Index {
        object: Box<ThirExpr>,
        index: Box<ThirExpr>,
    },

    // === Compound Literals ===
    ArrayLiteral(Vec<ThirExpr>),
    MapLiteral(Vec<(ThirExpr, ThirExpr)>),
    StructLiteral {
        name: String,
        fields: Vec<(String, ThirExpr)>,
    },
    EnumVariant {
        enum_name: String,
        variant: String,
        payload: Vec<ThirExpr>,
    },
    Tuple(Vec<ThirExpr>),
    Spread(Box<ThirExpr>),

    // === Control Flow ===
    If {
        cond: Box<ThirExpr>,
        then: Box<ThirExpr>,
        else_: Option<Box<ThirExpr>>,
    },
    Match {
        expr: Box<ThirExpr>,
        arms: Vec<ThirArm>,
    },
    Block(Vec<ThirStmt>, Option<Box<ThirExpr>>),
    Range {
        start: Option<Box<ThirExpr>>,
        end: Option<Box<ThirExpr>>,
        inclusive: bool,
    },

    // === Error Handling ===
    Ok(Box<ThirExpr>),
    Err(Box<ThirExpr>),
    Try(Box<ThirExpr>),
    UnwrapOrPanic {
        expr: Box<ThirExpr>,
        message: Box<ThirExpr>,
    },

    // === Ownership Annotations ===
    Move(Box<ThirExpr>),
    Borrow {
        expr: Box<ThirExpr>,
        mutable: bool,
    },
    Clone(Box<ThirExpr>),

    // === Closures & Casts ===
    Closure {
        params: Vec<(String, TypeId)>,
        body: Box<ThirExpr>,
        captures: Vec<ThirCapture>,
    },
    Cast {
        value: Box<ThirExpr>,
        to_type: TypeId,
    },

    // === Async & Concurrency ===
    Async(Box<ThirExpr>),
    Await(Box<ThirExpr>),
    Spawn(Box<ThirExpr>),
    ScopeBlock {
        stmts: Vec<ThirStmt>,
    },
}

/// Literal values.
#[derive(Debug, Clone)]
pub enum ThirLiteral {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Null,
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThirBinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThirUnOp {
    Neg,
    Not,
}

/// A match arm.
#[derive(Debug, Clone)]
pub struct ThirArm {
    pub pattern: ThirPattern,
    pub guard: Option<ThirExpr>,
    pub body: ThirExpr,
}
