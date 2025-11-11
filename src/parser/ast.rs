#![allow(dead_code)]

use crate::lexer::token::TokenType;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum TypeNode {
    Float,
    Int,
    String,
    Bool,
    Array(Box<TypeNode>),              // Array<Int>, Array<String>
    Map(Box<TypeNode>, Box<TypeNode>), // Map<String, Int>
    Tuple(Vec<TypeNode>),
    Void,
    Struct(String, HashMap<String, TypeNode>), // StructName -> field types
    Enum(String, HashMap<String, Option<TypeNode>>),
    Range(Box<TypeNode>, Box<TypeNode>, bool),
    TypeRef(String),
    Function(Vec<TypeNode>, Box<TypeNode>), // (params, return_type)
}

#[derive(Debug, Clone)]
pub enum ImportItem {
    /// Single import: `Add`
    Symbol(String),
    /// Aliased import: `Add as mathAdd`
    SymbolWithAlias(String, String), // (name, alias)
    /// Wildcard import: `*`
    Wildcard,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Identifier(String),
    Tuple(Vec<Pattern>),
    Wildcard,
}

#[derive(Debug, Clone)]
pub enum AstNode {
    Program(Vec<AstNode>),
    NumberLiteral(i32),
    FloatLiteral(f64),
    Identifier(String),
    StringLiteral(String),
    BoolLiteral(bool),
    ArrayLiteral(Vec<AstNode>),
    MapLiteral(Vec<(AstNode, AstNode)>),
    UnaryExpr {
        op: TokenType,
        expr: Box<AstNode>,
    },

    // 1+2 || a+2
    BinaryExpr {
        left: Box<AstNode>,
        op: TokenType,
        right: Box<AstNode>,
    },

    LetDecl {
        mutable: bool,
        type_annotation: Option<TypeNode>,
        pattern: Pattern,
        value: Box<AstNode>,
        is_ref_counted: Option<bool>,
    },

    StructDecl {
        name: String,
        fields: Vec<(String, TypeNode)>,
    },

    EnumDecl {
        name: String,
        variants: Vec<(String, Option<TypeNode>)>,
    },

    ConditionalStmt {
        condition: Box<AstNode>,
        then_block: Vec<AstNode>,
        else_branch: Option<Box<AstNode>>,
    },
    Block(Vec<AstNode>),
    Return {
        values: Vec<AstNode>, // multiple expressions can be returned
    },
    Print {
        exprs: Vec<AstNode>,
    },
    Break,
    Continue,

    Assignment {
        pattern: Pattern,
        value: Box<AstNode>,
    },

    CompoundAssignment {
        pattern: Pattern,
        op: TokenType, // PlusEq, MinusEq, StarEq, SlashEq
        value: Box<AstNode>,
    },

    FunctionDecl {
        name: String,
        visibility: String,
        params: Vec<(String, Option<TypeNode>)>,
        return_type: Option<TypeNode>,
        body: Vec<AstNode>,
    },
    FunctionCall {
        func: Box<AstNode>, // usually an Identifier node
        args: Vec<AstNode>,
    },
    MethodCall {
        object: Box<AstNode>,
        method: String,
        args: Vec<AstNode>,
    },

    ForLoopStmt {
        pattern: Pattern,
        iterable: Option<Box<AstNode>>,
        body: Vec<AstNode>, // keep Vec (block already returns Vec)
    },

    TupleLiteral(Vec<AstNode>),

    Range {
        start: Box<AstNode>,
        end: Box<AstNode>,
        inclusive: bool,
    },

    // --- Array/Map Element Access ---
    ElementAccess {
        array: Box<AstNode>,
        index: Box<AstNode>,
    },

    // --- Module Import ---
    Import {
        path: Vec<String>,      // e.g. ["core", "math"]
        items: Vec<ImportItem>, // Multiple imports with optional aliases, or wildcard
    },

    // --- Type Casting ---
    Cast {
        expr: Box<AstNode>,
        target_type: TypeNode,
    },

    // --- Closure ---
    Closure {
        params: Vec<(String, Option<TypeNode>)>, // parameter name and optional type
        body: Box<AstNode>,                      // closure body (expression or block)
        return_type: Option<TypeNode>,           // optional explicit return type
    },
}
