//! THIR Statement Nodes

use doo_core::types::TypeId;
use doo_core::Span;

use crate::expr::ThirExpr;

/// A THIR statement.
#[derive(Debug, Clone)]
pub struct ThirStmt {
    pub kind: ThirStmtKind,
    pub span: Span,
}

/// THIR statement kinds.
#[derive(Debug, Clone)]
pub enum ThirStmtKind {
    /// Variable declaration.
    Let {
        name: String,
        ty: TypeId,
        value: ThirExpr,
        mutable: bool,
    },

    /// Compile-time constant declaration.
    Const {
        name: String,
        ty: TypeId,
        value: ThirExpr,
    },

    /// Expression statement.
    Expr(ThirExpr),

    /// Assignment.
    Assign {
        target: ThirExpr,
        value: ThirExpr,
    },

    /// Return statement.
    Return(Option<ThirExpr>),

    /// Break with optional value (for loop expressions).
    Break(Option<ThirExpr>),
    Continue,

    /// While loop (for-loops are fully desugared to this in HIR).
    While {
        cond: ThirExpr,
        body: Vec<ThirStmt>,
        increment: Vec<ThirStmt>,
    },

    /// Infinite loop.
    Loop {
        body: Vec<ThirStmt>,
    },

    /// Go spawn: `go { ... }`
    Go {
        expr: ThirExpr,
    },

    /// Structured scope: `scope { ... }`
    Scope {
        stmts: Vec<ThirStmt>,
    },

    /// Drop (inserted by ownership analysis later, but present in THIR).
    Drop {
        name: String,
        ty: TypeId,
    },

    /// Tuple unpacking declaration.
    TupleLet {
        names: Vec<String>,
        type_ids: Vec<TypeId>,
        value: ThirExpr,
        mutable: bool,
    },

    /// Manual error extraction: `let val ?? err = expr`
    ManualErrorExtract {
        ok_names: Vec<String>,
        error_name: String,
        expr: ThirExpr,
    },
}
