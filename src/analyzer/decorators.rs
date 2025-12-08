//! Decorator validation for struct fields
//!
//! Validates decorators like @email, @min, @max, @required, @enum, @unique, @primary, @autoIncrement
//! Ensures proper type compatibility and argument validation.

use crate::parser::ast::{AstNode, Decorator, TypeNode};
use super::types::SemanticError;

/// Known decorator names and their validation rules
#[derive(Debug, Clone, PartialEq)]
pub enum DecoratorKind {
    Email,         // @email - only on Str
    Required,      // @required - any type
    Min,           // @min(n) - Str (length), Int/Float (value)
    Max,           // @max(n) - Str (length), Int/Float (value)
    Enum,          // @enum("a", "b", "c") - only on Str
    Unique,        // @unique - any type (for DB)
    Primary,       // @primary - any type (for DB)
    AutoIncrement, // @autoIncrement - only on Int
    Unknown(String),
}

impl DecoratorKind {
    pub fn from_name(name: &str) -> Self {
        match name {
            "email" => DecoratorKind::Email,
            "required" => DecoratorKind::Required,
            "min" => DecoratorKind::Min,
            "max" => DecoratorKind::Max,
            "enum" => DecoratorKind::Enum,
            "unique" => DecoratorKind::Unique,
            "primary" => DecoratorKind::Primary,
            "autoIncrement" => DecoratorKind::AutoIncrement,
            other => DecoratorKind::Unknown(other.to_string()),
        }
    }
}

/// Check if type is Str or Optional<Str>
fn is_string_type(ty: &TypeNode) -> bool {
    match ty {
        TypeNode::String => true,
        TypeNode::Optional(inner) => matches!(**inner, TypeNode::String),
        _ => false,
    }
}

/// Check if type is Int or Optional<Int>
fn is_int_type(ty: &TypeNode) -> bool {
    match ty {
        TypeNode::Int => true,
        TypeNode::Optional(inner) => matches!(**inner, TypeNode::Int),
        _ => false,
    }
}

/// Check if type is Str, Int, Float or Optional versions
fn is_string_or_numeric_type(ty: &TypeNode) -> bool {
    match ty {
        TypeNode::String | TypeNode::Int | TypeNode::Float => true,
        TypeNode::Optional(inner) => matches!(**inner, TypeNode::String | TypeNode::Int | TypeNode::Float),
        _ => false,
    }
}

/// Validate a single decorator against a field type
pub fn validate_decorator(
    decorator: &Decorator,
    field_type: &TypeNode,
    field_name: &str,
    struct_name: &str,
) -> Result<(), SemanticError> {
    let kind = DecoratorKind::from_name(&decorator.name);
    
    match kind {
        DecoratorKind::Email => {
            // @email only valid on Str
            if !is_string_type(field_type) {
                return Err(SemanticError::InvalidDecoratorType {
                    decorator: "email".to_string(),
                    field: field_name.to_string(),
                    struct_name: struct_name.to_string(),
                    expected_type: "Str".to_string(),
                    found_type: field_type.to_string(),
                });
            }
            // @email takes no arguments
            if !decorator.args.is_empty() {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "email".to_string(),
                    field: field_name.to_string(),
                    message: "email decorator takes no arguments".to_string(),
                });
            }
        }
        
        DecoratorKind::Required => {
            // @required valid on any type, takes no arguments
            if !decorator.args.is_empty() {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "required".to_string(),
                    field: field_name.to_string(),
                    message: "required decorator takes no arguments".to_string(),
                });
            }
        }
        
        DecoratorKind::Min => {
            // @min(n) valid on Str (length), Int, Float
            if !is_string_or_numeric_type(field_type) {
                return Err(SemanticError::InvalidDecoratorType {
                    decorator: "min".to_string(),
                    field: field_name.to_string(),
                    struct_name: struct_name.to_string(),
                    expected_type: "Str, Int, or Float".to_string(),
                    found_type: field_type.to_string(),
                });
            }
            // Must have exactly 1 numeric argument
            if decorator.args.len() != 1 {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "min".to_string(),
                    field: field_name.to_string(),
                    message: "min decorator requires exactly 1 numeric argument".to_string(),
                });
            }
            // Validate argument is a number
            match &decorator.args[0] {
                AstNode::NumberLiteral(_) | AstNode::FloatLiteral(_) => {}
                _ => {
                    return Err(SemanticError::InvalidDecoratorArgs {
                        decorator: "min".to_string(),
                        field: field_name.to_string(),
                        message: "min decorator argument must be a number".to_string(),
                    });
                }
            }
        }
        
        DecoratorKind::Max => {
            // @max(n) valid on Str (length), Int, Float
            if !is_string_or_numeric_type(field_type) {
                return Err(SemanticError::InvalidDecoratorType {
                    decorator: "max".to_string(),
                    field: field_name.to_string(),
                    struct_name: struct_name.to_string(),
                    expected_type: "Str, Int, or Float".to_string(),
                    found_type: field_type.to_string(),
                });
            }
            // Must have exactly 1 numeric argument
            if decorator.args.len() != 1 {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "max".to_string(),
                    field: field_name.to_string(),
                    message: "max decorator requires exactly 1 numeric argument".to_string(),
                });
            }
            match &decorator.args[0] {
                AstNode::NumberLiteral(_) | AstNode::FloatLiteral(_) => {}
                _ => {
                    return Err(SemanticError::InvalidDecoratorArgs {
                        decorator: "max".to_string(),
                        field: field_name.to_string(),
                        message: "max decorator argument must be a number".to_string(),
                    });
                }
            }
        }
        
        DecoratorKind::Enum => {
            // @enum("a", "b") only valid on Str
            if !is_string_type(field_type) {
                return Err(SemanticError::InvalidDecoratorType {
                    decorator: "enum".to_string(),
                    field: field_name.to_string(),
                    struct_name: struct_name.to_string(),
                    expected_type: "Str".to_string(),
                    found_type: field_type.to_string(),
                });
            }
            // Must have at least 1 string argument
            if decorator.args.is_empty() {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "enum".to_string(),
                    field: field_name.to_string(),
                    message: "enum decorator requires at least 1 string argument".to_string(),
                });
            }
            // All arguments must be strings
            for arg in &decorator.args {
                if !matches!(arg, AstNode::StringLiteral(_)) {
                    return Err(SemanticError::InvalidDecoratorArgs {
                        decorator: "enum".to_string(),
                        field: field_name.to_string(),
                        message: "enum decorator arguments must be string literals".to_string(),
                    });
                }
            }
        }
        
        DecoratorKind::Unique => {
            // @unique valid on any type, takes no arguments
            if !decorator.args.is_empty() {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "unique".to_string(),
                    field: field_name.to_string(),
                    message: "unique decorator takes no arguments".to_string(),
                });
            }
        }
        
        DecoratorKind::Primary => {
            // @primary valid on any type, takes no arguments
            if !decorator.args.is_empty() {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "primary".to_string(),
                    field: field_name.to_string(),
                    message: "primary decorator takes no arguments".to_string(),
                });
            }
        }
        
        DecoratorKind::AutoIncrement => {
            // @autoIncrement only valid on Int
            if !is_int_type(field_type) {
                return Err(SemanticError::InvalidDecoratorType {
                    decorator: "autoIncrement".to_string(),
                    field: field_name.to_string(),
                    struct_name: struct_name.to_string(),
                    expected_type: "Int".to_string(),
                    found_type: field_type.to_string(),
                });
            }
            if !decorator.args.is_empty() {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "autoIncrement".to_string(),
                    field: field_name.to_string(),
                    message: "autoIncrement decorator takes no arguments".to_string(),
                });
            }
        }
        
        DecoratorKind::Unknown(name) => {
            return Err(SemanticError::UnknownDecorator {
                decorator: name,
                field: field_name.to_string(),
                struct_name: struct_name.to_string(),
            });
        }
    }
    
    Ok(())
}

/// Validate all decorators on a struct field
pub fn validate_field_decorators(
    decorators: &[Decorator],
    field_type: &TypeNode,
    field_name: &str,
    struct_name: &str,
) -> Result<(), SemanticError> {
    for decorator in decorators {
        validate_decorator(decorator, field_type, field_name, struct_name)?;
    }
    Ok(())
}
