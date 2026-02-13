//! HIR Type Definitions
//!
//! All HIR node types - expressions, statements, items.
//! Every node carries a TypeId for type information (initially unknown).

use doo_core::{
    types::{builtin, TypeId},
    Span,
};
use serde::{Deserialize, Serialize};

// ============================================================================
// Core Types
// ============================================================================

/// Ownership state for a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ownership {
    /// Value is owned by this variable.
    Owned,
    /// Value has been moved to another location.
    Moved,
    /// Value is borrowed (immutably or mutably).
    Borrowed { mutable: bool },
    /// Value will be cloned when used.
    Clone,
}

impl Default for Ownership {
    fn default() -> Self {
        Self::Owned
    }
}

/// Constant value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConstValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Nil,
}

impl ConstValue {
    /// Get the TypeId for this constant value.
    pub fn type_id(&self) -> TypeId {
        match self {
            Self::Int(_) => builtin::INT,
            Self::Float(_) => builtin::FLOAT,
            Self::Bool(_) => builtin::BOOL,
            Self::Str(_) => builtin::STR,
            Self::Nil => builtin::VOID,
        }
    }
}

// ============================================================================
// Expressions
// ============================================================================

/// HIR expression with type annotation.
#[derive(Debug, Clone)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub type_id: Option<TypeId>,
    pub span: Span,
}

impl HirExpr {
    pub fn new(kind: HirExprKind, span: Span) -> Self {
        Self {
            kind,
            type_id: None,
            span,
        }
    }

    pub fn with_type(kind: HirExprKind, type_id: TypeId, span: Span) -> Self {
        Self {
            kind,
            type_id: Some(type_id),
            span,
        }
    }
}

/// HIR expression kinds (desugared).
#[derive(Debug, Clone)]
pub enum HirExprKind {
    // === Literals ===
    /// Constant value.
    Const(ConstValue),

    // === Variables ===
    /// Local variable reference.
    Local {
        name: String,
    },
    /// Function/global reference.
    Global {
        name: String,
    },

    // === Operations ===
    /// Binary operation (desugared from all compound assignments).
    BinOp {
        op: HirBinOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },
    /// Unary operation.
    UnaryOp {
        op: HirUnaryOp,
        operand: Box<HirExpr>,
    },

    // === Calls ===
    /// Function call.
    Call {
        func: Box<HirExpr>,
        args: Vec<HirExpr>,
    },
    /// Method call.
    MethodCall {
        receiver: Box<HirExpr>,
        method: String,
        args: Vec<HirExpr>,
    },

    // === Access ===
    /// Field access: `obj.field`
    Field {
        object: Box<HirExpr>,
        field: String,
    },
    /// Index access: `arr[idx]`
    Index {
        object: Box<HirExpr>,
        index: Box<HirExpr>,
    },

    // === Compound Literals ===
    /// Array construction.
    Array(Vec<HirExpr>),

    Map(Vec<(HirExpr, HirExpr)>),
    /// Tuple construction.
    Tuple(Vec<HirExpr>),
    Struct {
        name: String,
        fields: Vec<(String, HirExpr)>,
    },

    EnumVariant {
        enum_name: String,
        variant: String,
        payload: Vec<HirExpr>,
    },

    // === Spread ===
    /// Spread operator: `...expr`
    Spread(Box<HirExpr>),

    // === Control Flow ===
    /// Conditional expression.
    If {
        condition: Box<HirExpr>,
        then_expr: Box<HirExpr>,
        else_expr: Option<Box<HirExpr>>,
    },
    Block {
        stmts: Vec<HirStmt>,
        expr: Option<Box<HirExpr>>,
    },

    Match {
        values: Vec<HirExpr>,
        arms: Vec<HirMatchArm>,
    },

    // === Range (desugared from `..` and `..=`) ===
    /// Range construction.
    Range {
        start: Box<HirExpr>,
        end: Box<HirExpr>,
        inclusive: bool,
    },

    // === Error Handling ===
    /// Result::Ok wrapper.
    Ok(Box<HirExpr>),
    /// Result::Err wrapper.
    Err(Box<HirExpr>),
    /// Try operator (propagate error).
    Try(Box<HirExpr>),

    UnwrapOrPanic {
        expr: Box<HirExpr>,
        message: Box<HirExpr>,
    },

    // === Route Block (for app.group() inline routes) ===
    /// Collection of route definitions.
    RouteBlock {
        routes: Vec<HirExpr>,
    },

    // === Ownership Annotations (filled by analysis) ===
    /// Move value.
    Move(Box<HirExpr>),
    /// Borrow value.
    Borrow {
        expr: Box<HirExpr>,
        mutable: bool,
    },
    /// Clone value (inserted by ownership analysis).
    Clone(Box<HirExpr>),

    // === Closure ===
    Closure {
        params: Vec<(String, Option<TypeId>)>,
        body: Box<HirExpr>,
    },

    // === Cast ===
    /// Type cast expression.
    Cast {
        value: Box<HirExpr>,
        to_type: TypeId,
    },

    // === Async & Concurrency ===
    /// Await expression: `await expr`
    Await(Box<HirExpr>),
    /// Go spawn: `go { ... }`
    Spawn { body: Box<HirExpr> },
    /// Structured scope: `scope { ... }`
    ScopeBlock { stmts: Vec<HirStmt> },
}

/// Binary operators (simplified set).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HirBinOp {
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
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HirUnaryOp {
    Neg, // -x
    Not, // !x
}

// ============================================================================
// Statements
// ============================================================================

/// HIR statement.
#[derive(Debug, Clone)]
pub struct HirStmt {
    pub kind: HirStmtKind,
    pub span: Span,
}

impl HirStmt {
    pub fn new(kind: HirStmtKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// Statement kinds (desugared).
#[derive(Debug, Clone)]
pub enum HirStmtKind {
    /// Variable declaration.
    Let {
        name: String,
        type_id: Option<TypeId>,
        value: HirExpr,
        mutable: bool,
        ownership: Ownership,
    },

    /// Tuple unpacking declaration (desugared from `let a, b, c = tuple_expr`).
    /// Each name gets bound to the corresponding tuple element.
    TupleLet {
        names: Vec<String>,
        type_ids: Vec<Option<TypeId>>,
        value: HirExpr,
        mutable: bool,
    },

    /// Assignment (desugared from compound assignments).
    Assign { target: HirExpr, value: HirExpr },

    /// Expression statement.
    Expr(HirExpr),

    /// Return.
    Return(Vec<HirExpr>),

    /// Break.
    Break,

    /// Continue.
    Continue,

    /// Drop (inserted by ownership analysis).
    Drop { name: String },

    /// If statement (desugared from if expressions when used as statement).
    If {
        condition: HirExpr,
        then_block: Vec<HirStmt>,
        else_block: Option<Vec<HirStmt>>,
    },

    /// While loop (for-loops are desugared to this).
    While {
        condition: HirExpr,
        body: Vec<HirStmt>,
    },

    ManualErrorExtract {
        ok_names: Vec<String>,
        error_name: String,
        expr: HirExpr,
    },
}

#[derive(Debug, Clone)]
pub struct HirMatchArm {
    pub pattern: HirMatchPattern,
    pub guard: Option<HirExpr>,
    pub body: HirExpr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum HirMatchPattern {
    Literal(Box<HirExpr>),
    Condition(Box<HirExpr>),
    Wildcard,
    EnumVariant {
        enum_name: String,
        variant: String,
    },
    EnumVariantPayload {
        enum_name: String,
        variant: String,
        bindings: Vec<String>,
    },
    Tuple(Vec<HirMatchPattern>),
}

// ============================================================================
// Items (Top-Level Declarations)
// ============================================================================

/// HIR program.
#[derive(Debug, Clone)]
pub struct HirProgram {
    pub items: Vec<HirItem>,
    pub span: Span,
}

/// Top-level items.
#[derive(Debug, Clone)]
pub enum HirItem {
    Function(HirFunction),
    Struct(HirStruct),
    Enum(HirEnum),
    Import(HirImport),
}

/// Function definition.
#[derive(Debug, Clone)]
pub struct HirFunction {
    pub name: String,
    pub params: Vec<HirParam>,
    pub return_type: Option<TypeId>,
    pub error_type: Option<TypeId>,
    pub body: Vec<HirStmt>,
    pub decorators: Vec<HirDecorator>,
    pub is_async: bool,
    pub span: Span,
}

/// Function parameter.
#[derive(Debug, Clone)]
pub struct HirParam {
    pub name: String,
    pub type_id: Option<TypeId>,
    pub span: Span,
}

/// Struct definition.
#[derive(Debug, Clone)]
pub struct HirStruct {
    pub name: String,
    pub fields: Vec<HirField>,
    pub decorators: Vec<HirDecorator>,
    pub span: Span,
}

/// Struct field.
#[derive(Debug, Clone)]
pub struct HirField {
    pub name: String,
    pub type_id: Option<TypeId>,
    pub is_public: bool,
    pub is_optional: bool,
    pub default: Option<HirExpr>,
    pub decorators: Vec<HirDecorator>,
    pub span: Span,
}

/// Enum definition.
#[derive(Debug, Clone)]
pub struct HirEnum {
    pub name: String,
    pub variants: Vec<HirVariant>,
    pub span: Span,
}

/// Enum variant.
#[derive(Debug, Clone)]
pub struct HirVariant {
    pub name: String,
    pub payload: Option<TypeId>,
    pub span: Span,
}

/// Import.
#[derive(Debug, Clone)]
pub struct HirImport {
    pub path: Vec<String>,
    pub items: Vec<HirImportItem>,
    pub span: Span,
}

/// Import item.
#[derive(Debug, Clone)]
pub enum HirImportItem {
    Symbol(String),
    Alias { name: String, alias: String },
    Wildcard,
}

/// Decorator.
#[derive(Debug, Clone)]
pub struct HirDecorator {
    pub name: String,
    pub args: Vec<HirExpr>,
    pub span: Span,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ownership_default() {
        assert_eq!(Ownership::default(), Ownership::Owned);
    }

    #[test]
    fn test_const_value() {
        let c = ConstValue::Int(42);
        assert_eq!(c, ConstValue::Int(42));
    }

    #[test]
    fn test_hir_expr_new() {
        let span = Span::dummy();
        let expr = HirExpr::new(HirExprKind::Const(ConstValue::Int(1)), span);
        assert!(expr.type_id.is_none());
    }

    #[test]
    fn test_hir_expr_with_type() {
        let span = Span::dummy();
        let expr = HirExpr::with_type(
            HirExprKind::Const(ConstValue::Bool(true)),
            builtin::BOOL,
            span,
        );
        assert_eq!(expr.type_id, Some(builtin::BOOL));
    }
}
