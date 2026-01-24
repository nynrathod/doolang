//! Decorator Validation
//!
//! Validates decorators on struct fields for type compatibility and constraints.
//!
//! ## Supported Decorators:
//!
//! - Validation: `@email`, `@url`, `@min(n)`, `@max(n)`, `@pattern(regex)`
//! - Database: `@primary`, `@autoIncrement`, `@auto`, `@unique`, `@foreign(Struct)`
//! - Security: `@hash` (password hashing)
//! - Visibility: `@readOnly`, `@writeOnly`, `@internal`
//! - Default: `@default(value)`, `@optional`
//! - Timestamp: `@autoTimestamp` (struct-level)
//! - HTTP: `@redirect`

use doo_core::types::{TypeKind, TypeRegistry, TypeId, builtin};
use doo_frontend::ast::Decorator;

/// Known decorator kinds with validation rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecoratorKind {
    Email,         // @email - Str only
    Url,           // @url - Str only
    Min,           // @min(n) - Str (length), Int/Float (value)
    Max,           // @max(n) - Str (length), Int/Float (value)
    Foreign,       // @foreign(StructName) - Int only
    Unique,        // @unique - any type (DB)
    Primary,       // @primary - any type (DB)
    AutoIncrement, // @autoIncrement - Int only
    Auto,          // @auto - Int only (alias)
    Hash,          // @hash - Str only (password)
    Optional,      // @optional - any type (HTTP)
    Default,       // @default(value) - any type (HTTP)
    Pattern,       // @pattern(regex) - Str only (HTTP)
    WriteOnly,     // @writeOnly - request only
    ReadOnly,      // @readOnly - response only
    Internal,      // @internal - neither request nor response
    AutoTimestamp, // @autoTimestamp - struct-level
    Redirect,      // @redirect - requires @url
    Unknown(String),
}

impl DecoratorKind {
    /// Parse decorator kind from name.
    pub fn from_name(name: &str) -> Self {
        match name {
            "email" => Self::Email,
            "url" => Self::Url,
            "min" => Self::Min,
            "max" => Self::Max,
            "foreign" => Self::Foreign,
            "unique" => Self::Unique,
            "primary" => Self::Primary,
            "autoIncrement" => Self::AutoIncrement,
            "auto" => Self::Auto,
            "hash" => Self::Hash,
            "optional" => Self::Optional,
            "default" => Self::Default,
            "pattern" => Self::Pattern,
            "writeOnly" => Self::WriteOnly,
            "readOnly" => Self::ReadOnly,
            "internal" => Self::Internal,
            "autoTimestamp" => Self::AutoTimestamp,
            "redirect" => Self::Redirect,
            other => Self::Unknown(other.to_string()),
        }
    }
}

/// Decorator validation error.
#[derive(Debug, Clone)]
pub enum DecoratorError {
    /// Decorator applied to wrong type.
    InvalidType {
        decorator: String,
        field: String,
        struct_name: String,
        expected: String,
        found: String,
    },
    /// Invalid arguments to decorator.
    InvalidArgs {
        decorator: String,
        field: String,
        message: String,
    },
    /// Unknown decorator.
    Unknown {
        decorator: String,
        field: String,
        struct_name: String,
    },
    /// Conflicting decorators.
    Conflict {
        decorator1: String,
        decorator2: String,
        field: String,
        struct_name: String,
        reason: String,
    },
    /// Invalid optional with decorator.
    InvalidOptional {
        decorator: String,
        field: String,
        struct_name: String,
        reason: String,
    },
}

impl std::fmt::Display for DecoratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidType { decorator, field, struct_name, expected, found } => {
                write!(f, "@{} on {}.{} requires type {}, found {}", 
                    decorator, struct_name, field, expected, found)
            }
            Self::InvalidArgs { decorator, field, message } => {
                write!(f, "@{} on {}: {}", decorator, field, message)
            }
            Self::Unknown { decorator, field, struct_name } => {
                write!(f, "Unknown decorator @{} on {}.{}", decorator, struct_name, field)
            }
            Self::Conflict { decorator1, decorator2, field, struct_name, reason } => {
                write!(f, "@{} and @{} conflict on {}.{}: {}", 
                    decorator1, decorator2, struct_name, field, reason)
            }
            Self::InvalidOptional { decorator, field, struct_name, reason } => {
                write!(f, "@{} on optional field {}.{}: {}", decorator, struct_name, field, reason)
            }
        }
    }
}

impl std::error::Error for DecoratorError {}

/// Decorator validator.
pub struct DecoratorValidator<'a> {
    type_registry: &'a TypeRegistry,
}

impl<'a> DecoratorValidator<'a> {
    /// Create a new validator.
    pub fn new(type_registry: &'a TypeRegistry) -> Self {
        Self { type_registry }
    }

    /// Check if type is Str or Optional<Str>.
    fn is_string_type(&self, type_id: TypeId) -> bool {
        match self.type_registry.get(type_id).map(|t| &t.kind) {
            Some(TypeKind::Str) => true,
            Some(TypeKind::Optional { inner }) => self.is_string_type(*inner),
            _ => type_id == builtin::STR,
        }
    }

    /// Check if type is Int or Optional<Int>.
    fn is_int_type(&self, type_id: TypeId) -> bool {
        match self.type_registry.get(type_id).map(|t| &t.kind) {
            Some(TypeKind::Int) => true,
            Some(TypeKind::Optional { inner }) => self.is_int_type(*inner),
            _ => type_id == builtin::INT,
        }
    }

    /// Check if type is Str, Int, Float, or Optional versions.
    fn is_string_or_numeric_type(&self, type_id: TypeId) -> bool {
        match self.type_registry.get(type_id).map(|t| &t.kind) {
            Some(TypeKind::Str) | Some(TypeKind::Int) | Some(TypeKind::Float) => true,
            Some(TypeKind::Optional { inner }) => self.is_string_or_numeric_type(*inner),
            _ => type_id == builtin::STR || type_id == builtin::INT || type_id == builtin::FLOAT,
        }
    }

    /// Get type name for error messages.
    fn type_name(&self, type_id: TypeId) -> String {
        self.type_registry
            .get(type_id)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| format!("Type#{}", type_id.0))
    }

    /// Validate a single decorator.
    pub fn validate_decorator(
        &self,
        decorator: &Decorator,
        field_type_id: TypeId,
        field_name: &str,
        struct_name: &str,
    ) -> Result<(), DecoratorError> {
        let kind = DecoratorKind::from_name(&decorator.name);

        match kind {
            DecoratorKind::Email => {
                if !self.is_string_type(field_type_id) {
                    return Err(DecoratorError::InvalidType {
                        decorator: "email".to_string(),
                        field: field_name.to_string(),
                        struct_name: struct_name.to_string(),
                        expected: "Str".to_string(),
                        found: self.type_name(field_type_id),
                    });
                }
                if !decorator.args.is_empty() {
                    return Err(DecoratorError::InvalidArgs {
                        decorator: "email".to_string(),
                        field: field_name.to_string(),
                        message: "takes no arguments".to_string(),
                    });
                }
            }

            DecoratorKind::Url => {
                if !self.is_string_type(field_type_id) {
                    return Err(DecoratorError::InvalidType {
                        decorator: "url".to_string(),
                        field: field_name.to_string(),
                        struct_name: struct_name.to_string(),
                        expected: "Str".to_string(),
                        found: self.type_name(field_type_id),
                    });
                }
                if !decorator.args.is_empty() {
                    return Err(DecoratorError::InvalidArgs {
                        decorator: "url".to_string(),
                        field: field_name.to_string(),
                        message: "takes no arguments".to_string(),
                    });
                }
            }

            DecoratorKind::Min | DecoratorKind::Max => {
                let name = if matches!(kind, DecoratorKind::Min) { "min" } else { "max" };
                if !self.is_string_or_numeric_type(field_type_id) {
                    return Err(DecoratorError::InvalidType {
                        decorator: name.to_string(),
                        field: field_name.to_string(),
                        struct_name: struct_name.to_string(),
                        expected: "Str, Int, or Float".to_string(),
                        found: self.type_name(field_type_id),
                    });
                }
                if decorator.args.len() != 1 {
                    return Err(DecoratorError::InvalidArgs {
                        decorator: name.to_string(),
                        field: field_name.to_string(),
                        message: "requires exactly 1 numeric argument".to_string(),
                    });
                }
            }

            DecoratorKind::Foreign => {
                if !self.is_int_type(field_type_id) {
                    return Err(DecoratorError::InvalidType {
                        decorator: "foreign".to_string(),
                        field: field_name.to_string(),
                        struct_name: struct_name.to_string(),
                        expected: "Int".to_string(),
                        found: self.type_name(field_type_id),
                    });
                }
                if decorator.args.len() != 1 {
                    return Err(DecoratorError::InvalidArgs {
                        decorator: "foreign".to_string(),
                        field: field_name.to_string(),
                        message: "requires exactly 1 argument: the referenced struct name".to_string(),
                    });
                }
            }

            DecoratorKind::Unique | DecoratorKind::Primary | DecoratorKind::Optional => {
                let name = match kind {
                    DecoratorKind::Unique => "unique",
                    DecoratorKind::Primary => "primary",
                    _ => "optional",
                };
                if !decorator.args.is_empty() {
                    return Err(DecoratorError::InvalidArgs {
                        decorator: name.to_string(),
                        field: field_name.to_string(),
                        message: "takes no arguments".to_string(),
                    });
                }
            }

            DecoratorKind::AutoIncrement | DecoratorKind::Auto => {
                let name = if matches!(kind, DecoratorKind::AutoIncrement) { "autoIncrement" } else { "auto" };
                if !self.is_int_type(field_type_id) {
                    return Err(DecoratorError::InvalidType {
                        decorator: name.to_string(),
                        field: field_name.to_string(),
                        struct_name: struct_name.to_string(),
                        expected: "Int".to_string(),
                        found: self.type_name(field_type_id),
                    });
                }
                if !decorator.args.is_empty() {
                    return Err(DecoratorError::InvalidArgs {
                        decorator: name.to_string(),
                        field: field_name.to_string(),
                        message: "takes no arguments".to_string(),
                    });
                }
            }

            DecoratorKind::Hash => {
                if !self.is_string_type(field_type_id) {
                    return Err(DecoratorError::InvalidType {
                        decorator: "hash".to_string(),
                        field: field_name.to_string(),
                        struct_name: struct_name.to_string(),
                        expected: "Str".to_string(),
                        found: self.type_name(field_type_id),
                    });
                }
                if !decorator.args.is_empty() {
                    return Err(DecoratorError::InvalidArgs {
                        decorator: "hash".to_string(),
                        field: field_name.to_string(),
                        message: "takes no arguments".to_string(),
                    });
                }
            }

            DecoratorKind::Default => {
                if decorator.args.len() != 1 {
                    return Err(DecoratorError::InvalidArgs {
                        decorator: "default".to_string(),
                        field: field_name.to_string(),
                        message: "requires exactly 1 argument".to_string(),
                    });
                }
            }

            DecoratorKind::Pattern => {
                if !self.is_string_type(field_type_id) {
                    return Err(DecoratorError::InvalidType {
                        decorator: "pattern".to_string(),
                        field: field_name.to_string(),
                        struct_name: struct_name.to_string(),
                        expected: "Str".to_string(),
                        found: self.type_name(field_type_id),
                    });
                }
                if decorator.args.len() != 1 {
                    return Err(DecoratorError::InvalidArgs {
                        decorator: "pattern".to_string(),
                        field: field_name.to_string(),
                        message: "requires exactly 1 string argument".to_string(),
                    });
                }
            }

            DecoratorKind::WriteOnly | DecoratorKind::ReadOnly | DecoratorKind::Internal => {
                let name = match kind {
                    DecoratorKind::WriteOnly => "writeOnly",
                    DecoratorKind::ReadOnly => "readOnly",
                    _ => "internal",
                };
                if !decorator.args.is_empty() {
                    return Err(DecoratorError::InvalidArgs {
                        decorator: name.to_string(),
                        field: field_name.to_string(),
                        message: "takes no arguments".to_string(),
                    });
                }
            }

            DecoratorKind::AutoTimestamp => {
                return Err(DecoratorError::InvalidArgs {
                    decorator: "autoTimestamp".to_string(),
                    field: field_name.to_string(),
                    message: "is a struct-level decorator, not a field decorator".to_string(),
                });
            }

            DecoratorKind::Redirect => {
                if !self.is_string_type(field_type_id) {
                    return Err(DecoratorError::InvalidType {
                        decorator: "redirect".to_string(),
                        field: field_name.to_string(),
                        struct_name: struct_name.to_string(),
                        expected: "Str".to_string(),
                        found: self.type_name(field_type_id),
                    });
                }
                if !decorator.args.is_empty() {
                    return Err(DecoratorError::InvalidArgs {
                        decorator: "redirect".to_string(),
                        field: field_name.to_string(),
                        message: "takes no arguments".to_string(),
                    });
                }
            }

            DecoratorKind::Unknown(name) => {
                return Err(DecoratorError::Unknown {
                    decorator: name,
                    field: field_name.to_string(),
                    struct_name: struct_name.to_string(),
                });
            }
        }

        Ok(())
    }

    /// Validate decorator combinations on a field.
    pub fn validate_combinations(
        &self,
        decorators: &[Decorator],
        is_optional: bool,
        field_name: &str,
        struct_name: &str,
    ) -> Result<(), DecoratorError> {
        let kinds: Vec<DecoratorKind> = decorators
            .iter()
            .map(|d| DecoratorKind::from_name(&d.name))
            .collect();

        let has_internal = kinds.contains(&DecoratorKind::Internal);
        let has_write_only = kinds.contains(&DecoratorKind::WriteOnly);
        let has_read_only = kinds.contains(&DecoratorKind::ReadOnly);
        let has_auto = kinds.contains(&DecoratorKind::Auto) || kinds.contains(&DecoratorKind::AutoIncrement);
        let has_redirect = kinds.contains(&DecoratorKind::Redirect);
        let has_url = kinds.contains(&DecoratorKind::Url);

        // @internal + @writeOnly conflict
        if has_internal && has_write_only {
            return Err(DecoratorError::Conflict {
                decorator1: "internal".to_string(),
                decorator2: "writeOnly".to_string(),
                field: field_name.to_string(),
                struct_name: struct_name.to_string(),
                reason: "@writeOnly requires field in request, but @internal excludes it".to_string(),
            });
        }

        // @internal + ? conflict
        if has_internal && is_optional {
            return Err(DecoratorError::InvalidOptional {
                decorator: "internal".to_string(),
                field: field_name.to_string(),
                struct_name: struct_name.to_string(),
                reason: "optional marker '?' is meaningless for internal fields".to_string(),
            });
        }

        // @auto + @writeOnly conflict
        if has_auto && has_write_only {
            return Err(DecoratorError::Conflict {
                decorator1: "auto".to_string(),
                decorator2: "writeOnly".to_string(),
                field: field_name.to_string(),
                struct_name: struct_name.to_string(),
                reason: "@auto fields are server-generated and cannot accept input".to_string(),
            });
        }

        // @writeOnly + @readOnly conflict
        if has_write_only && has_read_only {
            return Err(DecoratorError::Conflict {
                decorator1: "writeOnly".to_string(),
                decorator2: "readOnly".to_string(),
                field: field_name.to_string(),
                struct_name: struct_name.to_string(),
                reason: "field cannot be both request-only and response-only".to_string(),
            });
        }

        // @redirect requires @url
        if has_redirect && !has_url {
            return Err(DecoratorError::Conflict {
                decorator1: "redirect".to_string(),
                decorator2: "url".to_string(),
                field: field_name.to_string(),
                struct_name: struct_name.to_string(),
                reason: "@redirect requires @url on the same field".to_string(),
            });
        }

        Ok(())
    }

    /// Validate all decorators on a field.
    pub fn validate_field(
        &self,
        decorators: &[Decorator],
        field_type_id: TypeId,
        is_optional: bool,
        field_name: &str,
        struct_name: &str,
    ) -> Result<(), DecoratorError> {
        // Validate each decorator individually
        for decorator in decorators {
            self.validate_decorator(decorator, field_type_id, field_name, struct_name)?;
        }

        // Validate combinations
        self.validate_combinations(decorators, is_optional, field_name, struct_name)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decorator_kind_from_name() {
        assert_eq!(DecoratorKind::from_name("email"), DecoratorKind::Email);
        assert_eq!(DecoratorKind::from_name("min"), DecoratorKind::Min);
        assert!(matches!(DecoratorKind::from_name("foo"), DecoratorKind::Unknown(_)));
    }
}
