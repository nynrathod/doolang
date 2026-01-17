//! Decorator validation for struct fields
//!
//! Validates decorators like @email, @min, @max, @unique, @primary, @autoIncrement, @foreign
//! Ensures proper type compatibility and argument validation.

use super::types::SemanticError;
use crate::parser::ast::{AstNode, Decorator, TypeNode};

/// Known decorator names and their validation rules
#[derive(Debug, Clone, PartialEq)]
pub enum DecoratorKind {
    Email,         // @email - only on Str
    Url,           // @url - only on Str (URL validation)
    Min,           // @min(n) - Str (length), Int/Float (value)
    Max,           // @max(n) - Str (length), Int/Float (value)
    Foreign,       // @foreign(StructName) - only on Int (foreign key reference)
    Unique,        // @unique - any type (for DB)
    Primary,       // @primary - any type (for DB)
    AutoIncrement, // @autoIncrement - only on Int
    Auto,          // @auto - only on Int (alias for autoIncrement)
    Hash,          // @hash - only on Str (for password hashing)
    Optional,      // @optional - any type (HTTP)
    Default,       // @default(value) - any type (HTTP)
    Pattern,       // @pattern(regex) - only on Str (HTTP)
    WriteOnly,     // @writeOnly - field only in request, never in response
    ReadOnly,      // @readOnly - field only in response, never in request
    Internal,      // @internal - field neither in request nor response
    AutoTimestamp, // @autoTimestamp - struct-level decorator for createdAt/updatedAt
    Redirect,      // @redirect - when returning this field, do HTTP 302 redirect (requires @url)
    Unknown(String),
}

impl DecoratorKind {
    pub fn from_name(name: &str) -> Self {
        match name {
            "email" => DecoratorKind::Email,
            "url" => DecoratorKind::Url,
            "min" => DecoratorKind::Min,
            "max" => DecoratorKind::Max,
            "foreign" => DecoratorKind::Foreign,
            "unique" => DecoratorKind::Unique,
            "primary" => DecoratorKind::Primary,
            "autoIncrement" => DecoratorKind::AutoIncrement,
            "auto" => DecoratorKind::Auto,
            "hash" => DecoratorKind::Hash,
            "optional" => DecoratorKind::Optional,
            "default" => DecoratorKind::Default,
            "pattern" => DecoratorKind::Pattern,
            "writeOnly" => DecoratorKind::WriteOnly,
            "readOnly" => DecoratorKind::ReadOnly,
            "internal" => DecoratorKind::Internal,
            "autoTimestamp" => DecoratorKind::AutoTimestamp,
            "redirect" => DecoratorKind::Redirect,
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
        TypeNode::Optional(inner) => {
            matches!(**inner, TypeNode::String | TypeNode::Int | TypeNode::Float)
        }
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

        DecoratorKind::Url => {
            // @url only valid on Str
            if !is_string_type(field_type) {
                return Err(SemanticError::InvalidDecoratorType {
                    decorator: "url".to_string(),
                    field: field_name.to_string(),
                    struct_name: struct_name.to_string(),
                    expected_type: "Str".to_string(),
                    found_type: field_type.to_string(),
                });
            }
            // @url takes no arguments
            if !decorator.args.is_empty() {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "url".to_string(),
                    field: field_name.to_string(),
                    message: "url decorator takes no arguments".to_string(),
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

        DecoratorKind::Foreign => {
            // @foreign(StructName) only valid on Int
            if !is_int_type(field_type) {
                return Err(SemanticError::InvalidDecoratorType {
                    decorator: "foreign".to_string(),
                    field: field_name.to_string(),
                    struct_name: struct_name.to_string(),
                    expected_type: "Int".to_string(),
                    found_type: field_type.to_string(),
                });
            }
            // Must have exactly 1 identifier argument (the referenced struct name)
            if decorator.args.len() != 1 {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "foreign".to_string(),
                    field: field_name.to_string(),
                    message:
                        "foreign decorator requires exactly 1 argument: the referenced struct name"
                            .to_string(),
                });
            }
            // Argument must be an identifier (struct name)
            match &decorator.args[0] {
                AstNode::Identifier(_) => {}
                _ => {
                    return Err(SemanticError::InvalidDecoratorArgs {
                        decorator: "foreign".to_string(),
                        field: field_name.to_string(),
                        message: "foreign decorator argument must be a struct name".to_string(),
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

        DecoratorKind::Auto => {
            // @auto only valid on Int
            if !is_int_type(field_type) {
                return Err(SemanticError::InvalidDecoratorType {
                    decorator: "auto".to_string(),
                    field: field_name.to_string(),
                    struct_name: struct_name.to_string(),
                    expected_type: "Int".to_string(),
                    found_type: field_type.to_string(),
                });
            }

            if !decorator.args.is_empty() {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "auto".to_string(),
                    field: field_name.to_string(),
                    message: "auto decorator takes no arguments".to_string(),
                });
            }
        }

        DecoratorKind::Hash => {
            // @hash only valid on Str
            if !is_string_type(field_type) {
                return Err(SemanticError::InvalidDecoratorType {
                    decorator: "hash".to_string(),
                    field: field_name.to_string(),
                    struct_name: struct_name.to_string(),
                    expected_type: "Str".to_string(),
                    found_type: field_type.to_string(),
                });
            }

            if !decorator.args.is_empty() {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "hash".to_string(),
                    field: field_name.to_string(),
                    message: "hash decorator takes no arguments".to_string(),
                });
            }
        }

        DecoratorKind::Optional => {
            // @optional valid on any type, takes no arguments
            if !decorator.args.is_empty() {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "optional".to_string(),
                    field: field_name.to_string(),
                    message: "optional decorator takes no arguments".to_string(),
                });
            }
        }

        DecoratorKind::Default => {
            // @default(value) valid on any type, requires 1 argument
            if decorator.args.len() != 1 {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "default".to_string(),
                    field: field_name.to_string(),
                    message: "default decorator requires exactly 1 argument".to_string(),
                });
            }
            // Argument can be any literal type or enum variant
            match &decorator.args[0] {
                AstNode::StringLiteral(_)
                | AstNode::NumberLiteral(_)
                | AstNode::FloatLiteral(_)
                | AstNode::BoolLiteral(_)
                | AstNode::EnumVariant { .. } => {}
                _ => {
                    return Err(SemanticError::InvalidDecoratorArgs {
                        decorator: "default".to_string(),
                        field: field_name.to_string(),
                        message: "default decorator argument must be a literal or enum variant"
                            .to_string(),
                    });
                }
            }
        }

        DecoratorKind::Pattern => {
            // @pattern(regex) only valid on Str
            if !is_string_type(field_type) {
                return Err(SemanticError::InvalidDecoratorType {
                    decorator: "pattern".to_string(),
                    field: field_name.to_string(),
                    struct_name: struct_name.to_string(),
                    expected_type: "Str".to_string(),
                    found_type: field_type.to_string(),
                });
            }
            // Must have exactly 1 string argument
            if decorator.args.len() != 1 {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "pattern".to_string(),
                    field: field_name.to_string(),
                    message: "pattern decorator requires exactly 1 string argument".to_string(),
                });
            }
            match &decorator.args[0] {
                AstNode::StringLiteral(_) => {}
                _ => {
                    return Err(SemanticError::InvalidDecoratorArgs {
                        decorator: "pattern".to_string(),
                        field: field_name.to_string(),
                        message: "pattern decorator argument must be a string literal".to_string(),
                    });
                }
            }
        }

        DecoratorKind::WriteOnly => {
            // @writeOnly valid on any type, takes no arguments
            // Field is in request only, never in response
            if !decorator.args.is_empty() {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "writeOnly".to_string(),
                    field: field_name.to_string(),
                    message: "writeOnly decorator takes no arguments".to_string(),
                });
            }
        }

        DecoratorKind::ReadOnly => {
            // @readOnly valid on any type, takes no arguments
            // Field is in response only, never in request
            if !decorator.args.is_empty() {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "readOnly".to_string(),
                    field: field_name.to_string(),
                    message: "readOnly decorator takes no arguments".to_string(),
                });
            }
        }

        DecoratorKind::Internal => {
            // @internal valid on any type, takes no arguments
            // Field is neither in request nor response (backend only)
            if !decorator.args.is_empty() {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "internal".to_string(),
                    field: field_name.to_string(),
                    message: "internal decorator takes no arguments".to_string(),
                });
            }
        }

        DecoratorKind::AutoTimestamp => {
            // @autoTimestamp is a struct-level decorator
            // It should not be on a field
            return Err(SemanticError::InvalidDecoratorArgs {
                decorator: "autoTimestamp".to_string(),
                field: field_name.to_string(),
                message: "autoTimestamp is a struct-level decorator, not a field decorator. Use it like: struct MyStruct @autoTimestamp { ... }".to_string(),
            });
        }

        DecoratorKind::Redirect => {
            // @redirect only valid on Str (will be checked for @url in combination validation)
            if !is_string_type(field_type) {
                return Err(SemanticError::InvalidDecoratorType {
                    decorator: "redirect".to_string(),
                    field: field_name.to_string(),
                    struct_name: struct_name.to_string(),
                    expected_type: "Str".to_string(),
                    found_type: field_type.to_string(),
                });
            }
            // @redirect takes no arguments
            if !decorator.args.is_empty() {
                return Err(SemanticError::InvalidDecoratorArgs {
                    decorator: "redirect".to_string(),
                    field: field_name.to_string(),
                    message: "redirect decorator takes no arguments".to_string(),
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

/// Validate decorator combinations on a field
/// Checks for conflicting decorators that cannot be used together
pub fn validate_decorator_combinations(
    decorators: &[Decorator],
    is_optional: bool,
    field_name: &str,
    struct_name: &str,
) -> Result<(), SemanticError> {
    let decorator_kinds: Vec<DecoratorKind> = decorators
        .iter()
        .map(|d| DecoratorKind::from_name(&d.name))
        .collect();

    // Check for @internal + @writeOnly conflict
    let has_internal = decorator_kinds.contains(&DecoratorKind::Internal);
    let has_writeOnly = decorator_kinds.contains(&DecoratorKind::WriteOnly);
    let has_readOnly = decorator_kinds.contains(&DecoratorKind::ReadOnly);
    let has_auto = decorator_kinds.contains(&DecoratorKind::Auto)
        || decorator_kinds.contains(&DecoratorKind::AutoIncrement);

    if has_internal && has_writeOnly {
        return Err(SemanticError::DecoratorConflict {
            decorator1: "internal".to_string(),
            decorator2: "writeOnly".to_string(),
            field: field_name.to_string(),
            struct_name: struct_name.to_string(),
            reason: "@writeOnly requires field in request, but @internal excludes it from both request and response".to_string(),
        });
    }

    // Check for @internal + ? conflict
    if has_internal && is_optional {
        return Err(SemanticError::InvalidOptionalDecorator {
            decorator: "internal".to_string(),
            field: field_name.to_string(),
            struct_name: struct_name.to_string(),
            reason: "optional marker '?' is meaningless for internal fields that are never exposed"
                .to_string(),
        });
    }

    // Check for @auto + @writeOnly conflict
    if has_auto && has_writeOnly {
        return Err(SemanticError::DecoratorConflict {
            decorator1: "auto".to_string(),
            decorator2: "writeOnly".to_string(),
            field: field_name.to_string(),
            struct_name: struct_name.to_string(),
            reason: "@auto fields are server-generated and cannot accept input from requests"
                .to_string(),
        });
    }

    // Check for @writeOnly + @readonly conflict (contradictory)
    if has_writeOnly && has_readOnly {
        return Err(SemanticError::DecoratorConflict {
            decorator1: "writeOnly".to_string(),
            decorator2: "readOnly".to_string(),
            field: field_name.to_string(),
            struct_name: struct_name.to_string(),
            reason: "field cannot be both request-only and response-only".to_string(),
        });
    }

    // Check for @redirect without @url
    let has_redirect = decorator_kinds.contains(&DecoratorKind::Redirect);
    let has_url = decorator_kinds.contains(&DecoratorKind::Url);
    if has_redirect && !has_url {
        return Err(SemanticError::DecoratorConflict {
            decorator1: "redirect".to_string(),
            decorator2: "url".to_string(),
            field: field_name.to_string(),
            struct_name: struct_name.to_string(),
            reason: "@redirect requires @url on the same field to ensure valid redirect URLs".to_string(),
        });
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
