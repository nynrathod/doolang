#![allow(dead_code)]

use crate::lexer::token::TokenType;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum TypeNode {
    Float,
    Int,
    String,
    Bool,
    Array(Box<TypeNode>),
    Map(Box<TypeNode>, Box<TypeNode>),
    Tuple(Vec<TypeNode>),
    Void,
    Struct(String, HashMap<String, TypeNode>),
    Enum(String, HashMap<String, Option<TypeNode>>),
    Range(Box<TypeNode>, Box<TypeNode>, bool),
    TypeRef(String),
    Function(Vec<TypeNode>, Box<TypeNode>),
    Result(Box<TypeNode>, Box<TypeNode>), // Result(OkType, ErrType)
}

impl TypeNode {
    /// Format a TypeNode into a proper string representation for MIR
    /// This produces format like "Array(Int)", "Map(Str,Int)", etc.
    pub fn format_type_string(&self) -> String {
        match self {
            TypeNode::Float => "Float".to_string(),
            TypeNode::Int => "Int".to_string(),
            TypeNode::String => "Str".to_string(),
            TypeNode::Bool => "Bool".to_string(),
            TypeNode::Array(inner) => format!("Array({})", inner.format_type_string()),
            TypeNode::Map(key, value) => {
                format!(
                    "Map({},{})",
                    key.format_type_string(),
                    value.format_type_string()
                )
            }
            TypeNode::Tuple(types) => {
                let type_strs: Vec<String> = types.iter().map(|t| t.format_type_string()).collect();
                format!("Tuple({})", type_strs.join(","))
            }
            TypeNode::Void => "Void".to_string(),
            TypeNode::Struct(name, _) => format!("Struct({})", name),
            TypeNode::Enum(name, _) => format!("Enum({})", name),
            TypeNode::Range(_, _, _) => "Range".to_string(),
            TypeNode::TypeRef(name) => name.clone(),
            TypeNode::Function(params, ret) => {
                let param_strs: Vec<String> =
                    params.iter().map(|t| t.format_type_string()).collect();
                format!("Fn({})→{}", param_strs.join(","), ret.format_type_string())
            }
            TypeNode::Result(ok_type, err_type) => {
                format!(
                    "Result({},{})",
                    ok_type.format_type_string(),
                    err_type.format_type_string()
                )
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Identifier(String),
    Tuple(Vec<Pattern>),
    Wildcard,
}

#[derive(Debug, Clone)]
pub enum ImportItem {
    Symbol(String),                  // Single import: `Add`
    SymbolWithAlias(String, String), // Aliased import: `Add as mathAdd`
    Wildcard,                        // Wildcard import: `*`
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
        values: Vec<AstNode>,
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
    IncrementDecrement {
        variable: String,
        op: TokenType, // PlusPlus or MinusMinus
    },
    ElementAssignment {
        array: Box<AstNode>,
        index: Box<AstNode>,
        value: Box<AstNode>,
    },
    FunctionDecl {
        name: String,
        visibility: String,
        params: Vec<(String, Option<TypeNode>)>,
        return_type: Option<TypeNode>,
        error_type: Option<TypeNode>, // Error type after ! in function signature
        body: Vec<AstNode>,
    },
    FunctionCall {
        func: Box<AstNode>,
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
        body: Vec<AstNode>,
    },
    TupleLiteral(Vec<AstNode>),
    Range {
        start: Box<AstNode>,
        end: Box<AstNode>,
        inclusive: bool,
    },

    // Array/Map Element Access
    ElementAccess {
        array: Box<AstNode>,
        index: Box<AstNode>,
    },

    Import {
        path: Vec<String>,
        items: Vec<ImportItem>,
    },

    // Type cast variable
    Cast {
        expr: Box<AstNode>,
        target_type: TypeNode,
    },

    // Closure: () => {}
    Closure {
        params: Vec<(String, Option<TypeNode>)>,
        body: Box<AstNode>,
        return_type: Option<TypeNode>,
    },

    // Error handling constructs
    OkExpr {
        values: Vec<AstNode>, // Can be single or tuple of values
    },
    ErrExpr {
        value: Box<AstNode>, // Single error value
    },
    TryPropagate {
        expr: Box<AstNode>, // Expression with ? operator
    },
}
