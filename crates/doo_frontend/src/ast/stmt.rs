//! Statement AST nodes.
//!
//! Statements that perform actions but don't produce values directly.

use doo_core::Span;
use super::{Expr, Pattern, TypeExpr};
use crate::lexer::TokenKind;

/// A statement.
#[derive(Debug, Clone)]
pub struct Stmt {
    /// Statement kind.
    pub kind: StmtKind,
    /// Source location.
    pub span: Span,
}

impl Stmt {
    pub fn new(kind: StmtKind, span: Span) -> Self {
        Self { kind, span }
    }

    pub fn span(&self) -> Span {
        self.span
    }
}

/// Statement variants.
#[derive(Debug, Clone)]
pub enum StmtKind {
    // === Declarations ===
    /// Variable declaration: `let x = 1` or `let mut x: Int = 1`
    Let {
        mutable: bool,
        pattern: Pattern,
        type_ann: Option<TypeExpr>,
        value: Expr,
    },

    // === Assignments ===
    /// Simple assignment: `x = 1`
    Assign {
        target: Pattern,
        value: Expr,
    },
    /// Compound assignment: `x += 1`
    CompoundAssign {
        target: Pattern,
        op: CompoundOp,
        value: Expr,
    },
    /// Increment/Decrement: `x++` or `x--`
    IncDec {
        variable: String,
        op: IncDecOp,
    },
    /// Element assignment: `arr[i] = x`
    ElementAssign {
        array: Expr,
        index: Expr,
        value: Expr,
    },
    /// Field assignment: `obj.field = x`
    FieldAssign {
        object: Expr,
        field: String,
        value: Expr,
    },

    // === Control Flow ===
    /// If statement: `if cond { ... } else { ... }`
    If {
        condition: Expr,
        then_block: Vec<Stmt>,
        else_branch: Option<ElseBranch>,
    },
    /// For loop: `for x in iter { ... }`
    For {
        pattern: Pattern,
        iterable: Option<Expr>,
        body: Vec<Stmt>,
    },
    /// Return statement: `return x`
    Return(Vec<Expr>),
    /// Break statement: `break`
    Break,
    /// Continue statement: `continue`
    Continue,

    // === Other ===
    /// Print statement: `print(x)`
    Print(Vec<Expr>),
    /// Expression statement: `foo()`
    Expr(Expr),
    /// Block: `{ ... }`
    Block(Vec<Stmt>),

    // === Error Handling ===
    /// Manual error extraction: `let value, err = result`
    ManualErrorExtract {
        expr: Expr,
        ok_pattern: Pattern,
        error_var: String,
    },
}

/// Else branch (either block or else-if chain).
#[derive(Debug, Clone)]
pub enum ElseBranch {
    /// `else { ... }`
    Block(Vec<Stmt>),
    /// `else if cond { ... }`
    ElseIf(Box<Stmt>),
}

/// Compound assignment operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompoundOp {
    Add,    // +=
    Sub,    // -=
    Mul,    // *=
    Div,    // /=
    Mod,    // %=
}

impl CompoundOp {
    pub fn from_token(kind: TokenKind) -> Option<Self> {
        match kind {
            TokenKind::PlusEq => Some(Self::Add),
            TokenKind::MinusEq => Some(Self::Sub),
            TokenKind::StarEq => Some(Self::Mul),
            TokenKind::SlashEq => Some(Self::Div),
            TokenKind::PercentEq => Some(Self::Mod),
            _ => None,
        }
    }
}

/// Increment/decrement operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncDecOp {
    Inc, // ++
    Dec, // --
}

impl IncDecOp {
    pub fn from_token(kind: TokenKind) -> Option<Self> {
        match kind {
            TokenKind::PlusPlus => Some(Self::Inc),
            TokenKind::MinusMinus => Some(Self::Dec),
            _ => None,
        }
    }
}
