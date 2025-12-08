#![allow(dead_code)]

use crate::lexer::token::TokenType;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum TypeNode {
    Float,
    Int,
    String,
    Bool,
    Nil, // Polymorphic nil/null value - compatible with any pointer/optional type
    Array(Box<TypeNode>),
    Map(Box<TypeNode>, Box<TypeNode>),
    Tuple(Vec<TypeNode>),
    Void,
    Struct(String, HashMap<String, TypeNode>), // Name and fields map
    Enum(String, HashMap<String, Option<TypeNode>>), // Name and variants map
    Optional(Box<TypeNode>),                   // T? - optional type
    Range(Box<TypeNode>, Box<TypeNode>, bool),
    TypeRef(String),
    Function(Vec<TypeNode>, Box<TypeNode>),
    Result(Box<TypeNode>, Box<TypeNode>), // Result(OkType, ErrType)
    Builtin(String),                      // Builtin types like "json", "file"
    Any,   // Dynamic type - compatible with any type (used for JSON.parse)
    Error, // Generic error type - can hold any error value (used for ! Error syntax)
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
            TypeNode::Nil => "Nil".to_string(),
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
            TypeNode::Optional(inner) => format!("Optional({})", inner.format_type_string()),
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
            TypeNode::Builtin(name) => format!("Builtin({})", name),
            TypeNode::Any => "Any".to_string(),
            TypeNode::Error => "Error".to_string(),
        }
    }

    /// Check if a name is public (starts with uppercase)
    pub fn is_public_name(name: &str) -> bool {
        name.chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false)
    }
}

/// Represents a struct field with full metadata
#[derive(Debug, Clone)]
pub struct StructField {
    pub name: String,
    pub field_type: TypeNode,
    pub is_public: bool,   // Determined by PascalCase vs camelCase
    pub is_optional: bool, // Has ? suffix
    pub default_value: Option<Box<AstNode>>, // Default value expression
    pub decorators: Vec<Decorator>, // @email, @unique, etc.
}

/// Represents an enum variant
#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: String,
    pub payload: Option<TypeNode>, // None for unit variants, Some(T) for variants with data
}

/// Represents a decorator/annotation
#[derive(Debug, Clone)]
pub struct Decorator {
    pub name: String,
    pub args: Vec<AstNode>, // Decorator arguments like @min(8)
}

impl StructField {
    /// Create a new struct field with default settings
    pub fn new(name: String, field_type: TypeNode) -> Self {
        let is_public = TypeNode::is_public_name(&name);
        StructField {
            name,
            field_type,
            is_public,
            is_optional: false,
            default_value: None,
            decorators: Vec::new(),
        }
    }

    /// Set this field as optional
    pub fn with_optional(mut self, optional: bool) -> Self {
        self.is_optional = optional;
        self
    }

    /// Add a default value
    pub fn with_default(mut self, default: Box<AstNode>) -> Self {
        self.default_value = Some(default);
        self
    }

    /// Add a decorator
    pub fn with_decorator(mut self, decorator: Decorator) -> Self {
        self.decorators.push(decorator);
        self
    }
}

impl EnumVariant {
    /// Create a new enum variant
    pub fn new(name: String, payload: Option<TypeNode>) -> Self {
        EnumVariant { name, payload }
    }
}

impl Decorator {
    /// Create a new decorator
    pub fn new(name: String, args: Vec<AstNode>) -> Self {
        Decorator { name, args }
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
    NilLiteral,
    ArrayLiteral(Vec<AstNode>),
    MapLiteral(Vec<(AstNode, AstNode)>),
    SpreadElement(Box<AstNode>), // ...expr for spreading arrays/objects

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
        fields: Vec<StructField>,
        is_public: bool, // Determined by PascalCase vs camelCase
    },
    EnumDecl {
        name: String,
        variants: Vec<EnumVariant>,
        is_public: bool, // Determined by PascalCase vs camelCase
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
    FieldAssignment {
        object: Box<AstNode>,
        field: String,
        value: Box<AstNode>,
    },
    FunctionDecl {
        name: String,
        visibility: String,
        params: Vec<(String, Option<TypeNode>)>,
        return_type: Option<TypeNode>,
        error_type: Option<TypeNode>, // Error type after ! in function signature
        body: Vec<AstNode>,
        decorators: Vec<Decorator>,      // @ffi, @extern, etc.
        receiver_type: Option<String>, // For instance method declarations: fn TypeName.methodName(self)
        associated_type: Option<String>, // For both static and instance methods: the TypeName in fn TypeName.methodName
        is_expression: bool,             // true if function uses => syntax (expression function)
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

    // Closure: () => {} or (x: T) -> R ! E { }
    Closure {
        params: Vec<(String, Option<TypeNode>)>,
        body: Box<AstNode>,
        return_type: Option<TypeNode>,
        error_type: Option<TypeNode>, // Error type after ! for Result-returning closures
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
    UnwrapOrPanic {
        expr: Box<AstNode>,      // Expression that returns Result
        panic_msg: Box<AstNode>, // Panic message expression
    },
    ManualErrorExtract {
        expr: Box<AstNode>,  // Expression that returns Result
        ok_pattern: Pattern, // Pattern for Ok values (can be tuple)
        error_var: String,   // Variable name for error (or "_" to ignore)
    },

    // Struct and Enum operations
    StructLiteral {
        name: String,                        // Struct type name
        fields: Vec<(String, Box<AstNode>)>, // Field name and value pairs
    },
    FieldAccess {
        object: Box<AstNode>, // Object to access field from
        field: String,        // Field name
    },
    EnumVariant {
        enum_name: String,     // Enum type name
        variant: String,       // Variant name
        payload: Vec<AstNode>, // Arguments/payload (0 for unit variants, 1+ for data variants or function calls)
    },

    // Conditional expressions (inline if-else and ternary)
    ConditionalExpr {
        condition: Box<AstNode>,
        then_expr: Box<AstNode>,
        else_expr: Box<AstNode>,
    },

    // Block expression: { statements; final_expr }
    // Used for inline if-else with multiple statements before the result expression
    BlockExpr {
        statements: Vec<AstNode>, // Statements executed before the result (with semicolons)
        result: Box<AstNode>,     // Final expression value (no semicolon)
    },
    TernaryExpr {
        condition: Box<AstNode>,
        true_expr: Box<AstNode>,
        false_expr: Box<AstNode>,
    },

    // Match expression
    MatchExpr {
        values: Vec<AstNode>, // Empty for condition-based match, one or more for value-based match
        arms: Vec<MatchArm>,
    },
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: MatchPattern,
    pub guard: Option<Box<AstNode>>, // Optional guard condition (if expr)
    pub body: Box<AstNode>,
}

#[derive(Debug, Clone)]
pub enum MatchPattern {
    Literal(Box<AstNode>),   // 200, "OK", true, etc.
    Condition(Box<AstNode>), // age < 13, x > 100, etc.
    Wildcard,                // _
    EnumVariant {
        enum_name: String, // Status
        variant: String,   // Active
    },
    EnumVariantWithPayload {
        enum_name: String,     // HttpCode
        variant: String,       // Success
        bindings: Vec<String>, // code, msg (variables to bind payload to)
    },
    Tuple(Vec<MatchPattern>), // 1, "err", true => (tuple pattern without parens)
}
