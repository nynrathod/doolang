//! HTTP-specific decorator extraction for validation and type-safety
//!
//! This module extends the base decorator system with HTTP-specific functionality
//! for auto JSON parsing, validation, and type-safe parameter extraction.

use crate::parser::ast::{AstNode, Decorator, TypeNode};

/// HTTP-specific decorator info extracted during semantic analysis
#[derive(Debug, Clone)]
pub struct HttpDecoratorInfo {
    pub field_name: String,
    pub field_type: String,
    pub decorators: Vec<DecoratorSpec>,
    pub is_required: bool,
    pub is_optional: bool,
    pub has_default: bool,
    pub default_value: Option<String>,
}

/// Individual decorator specification for validation
#[derive(Debug, Clone, PartialEq)]
pub enum DecoratorSpec {
    Email,
    Min(i64),
    Max(i64),
    Pattern(String),
    Enum(Vec<String>),
    Required,
    Optional,
    Default(String),
}

impl DecoratorSpec {
    pub fn to_string(&self) -> String {
        match self {
            DecoratorSpec::Email => "email".to_string(),
            DecoratorSpec::Min(n) => format!("min{}", n),
            DecoratorSpec::Max(n) => format!("max{}", n),
            DecoratorSpec::Pattern(p) => format!("pattern:{}", p),
            DecoratorSpec::Enum(values) => format!("enum:{}", values.join("|")),
            DecoratorSpec::Required => "required".to_string(),
            DecoratorSpec::Optional => "optional".to_string(),
            DecoratorSpec::Default(v) => format!("default:{}", v),
        }
    }
}

/// Extract HTTP decorators from struct field
pub fn extract_http_decorators(
    decorators: &[Decorator],
    field_name: &str,
    field_type: &TypeNode,
) -> HttpDecoratorInfo {
    let mut specs = Vec::new();
    let mut is_required = true;
    let mut is_optional = false;
    let mut has_default = false;
    let mut default_value = None;

    for dec in decorators {
        match dec.name.as_str() {
            "email" => {
                specs.push(DecoratorSpec::Email);
            }
            "min" => {
                if let Some(arg) = dec.args.first() {
                    if let AstNode::NumberLiteral(n) = arg {
                        specs.push(DecoratorSpec::Min(*n as i64));
                    }
                }
            }
            "max" => {
                if let Some(arg) = dec.args.first() {
                    if let AstNode::NumberLiteral(n) = arg {
                        specs.push(DecoratorSpec::Max(*n as i64));
                    }
                }
            }
            "pattern" => {
                if let Some(arg) = dec.args.first() {
                    if let AstNode::StringLiteral(p) = arg {
                        specs.push(DecoratorSpec::Pattern(p.clone()));
                    }
                }
            }
            "enum" => {
                let values: Vec<String> = dec
                    .args
                    .iter()
                    .filter_map(|arg| {
                        if let AstNode::StringLiteral(s) = arg {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                if !values.is_empty() {
                    specs.push(DecoratorSpec::Enum(values));
                }
            }
            "required" => {
                specs.push(DecoratorSpec::Required);
                is_required = true;
            }
            "optional" => {
                specs.push(DecoratorSpec::Optional);
                is_optional = true;
                is_required = false;
            }
            "default" => {
                if let Some(arg) = dec.args.first() {
                    if let AstNode::StringLiteral(val) = arg {
                        specs.push(DecoratorSpec::Default(val.clone()));
                        has_default = true;
                        default_value = Some(val.clone());
                        is_required = false;
                    }
                }
            }
            _ => {} // Ignore unknown decorators
        }
    }

    HttpDecoratorInfo {
        field_name: field_name.to_string(),
        field_type: format!("{:?}", field_type),
        decorators: specs,
        is_required,
        is_optional,
        has_default,
        default_value,
    }
}

/// Build validator specification string for FFI
/// Format: "field1:email;field2:min8|max100;field3:enum:a|b|c;..."
pub fn build_validator_spec(field_info: &HttpDecoratorInfo) -> String {
    if field_info.decorators.is_empty() {
        return String::new();
    }

    let specs: Vec<String> = field_info
        .decorators
        .iter()
        .map(|spec| spec.to_string())
        .collect();

    format!("{}:{}", field_info.field_name, specs.join("|"))
}

/// Build complete validator specification for a struct
pub fn build_struct_validators(struct_fields: &[HttpDecoratorInfo]) -> String {
    struct_fields
        .iter()
        .filter_map(|field| {
            let spec = build_validator_spec(field);
            if spec.is_empty() {
                None
            } else {
                Some(spec)
            }
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Extract type-safe parameter info for path/query params
#[derive(Debug, Clone)]
pub struct ParameterInfo {
    pub name: String,
    pub param_type: String,
    pub is_required: bool,
    pub default_value: Option<String>,
    pub validators: Vec<DecoratorSpec>,
}

impl ParameterInfo {
    /// Determine if parameter needs type conversion
    pub fn needs_type_conversion(&self) -> bool {
        matches!(self.param_type.as_str(), "Int" | "Float" | "Bool" | "UUID")
    }

    /// Get LLVM type for parameter
    pub fn get_llvm_type(&self) -> &'static str {
        match self.param_type.as_str() {
            "Int" => "i64",
            "Float" => "f64",
            "Bool" => "i32",
            "UUID" => "ptr",
            _ => "ptr", // Default to pointer for strings and complex types
        }
    }
}

/// Extract path/query parameter info from function signature
pub fn extract_parameter_info(param_name: &str, param_type: &str) -> ParameterInfo {
    ParameterInfo {
        name: param_name.to_string(),
        param_type: param_type.to_string(),
        is_required: true,
        default_value: None,
        validators: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decorator_spec_to_string() {
        assert_eq!(DecoratorSpec::Email.to_string(), "email");
        assert_eq!(DecoratorSpec::Min(8).to_string(), "min8");
        assert_eq!(DecoratorSpec::Max(100).to_string(), "max100");
        assert_eq!(
            DecoratorSpec::Pattern("^[0-9]+$".to_string()).to_string(),
            "pattern:^[0-9]+$"
        );
        assert_eq!(
            DecoratorSpec::Enum(vec!["a".to_string(), "b".to_string()]).to_string(),
            "enum:a|b"
        );
    }

    #[test]
    fn test_parameter_type_conversion() {
        let param = ParameterInfo {
            name: "id".to_string(),
            param_type: "Int".to_string(),
            is_required: true,
            default_value: None,
            validators: Vec::new(),
        };

        assert!(param.needs_type_conversion());
        assert_eq!(param.get_llvm_type(), "i64");
    }

    #[test]
    fn test_build_validator_spec() {
        let info = HttpDecoratorInfo {
            field_name: "email".to_string(),
            field_type: "String".to_string(),
            decorators: vec![DecoratorSpec::Email, DecoratorSpec::Required],
            is_required: true,
            is_optional: false,
            has_default: false,
            default_value: None,
        };

        let spec = build_validator_spec(&info);
        assert!(spec.contains("email:"));
        assert!(spec.contains("email"));
        assert!(spec.contains("required"));
    }
}
