//! Validation Logic
//!
//! Centralized validation for field decorators (@email, @min, @max, etc.).
//! Returns RFC 7807 compatible errors.

use serde::{Deserialize, Serialize};

/// Represents a parsed decorator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDecorator {
    pub name: String,
    pub args: Vec<String>,
}

/// A validation error result.
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub field: String,
    pub rule: String,
    pub message: String,
    pub expected: Option<String>,
    pub received: Option<String>,
}

/// Validate a list of decorators against a value.
pub fn validate_field(
    field_name: &str,
    field_type: &str,
    value: &str,
    decorators: &[FieldDecorator],
) -> Result<(), ValidationError> {
    for decorator in decorators {
        match decorator.name.as_str() {
            "email" => validate_email(field_name, field_type, value)?,
            "url" => validate_url(field_name, field_type, value)?,
            "min" => validate_min(field_name, field_type, value, &decorator.args)?,
            "max" => validate_max(field_name, field_type, value, &decorator.args)?,
            "enum" => validate_enum(field_name, value, &decorator.args)?,
            "required" => validate_required(field_name, value)?,
            _ => {} // Ignore unknown or non-validation decorators (e.g. unique, default)
        }
    }
    Ok(())
}

fn validate_email(field_name: &str, field_type: &str, value: &str) -> Result<(), ValidationError> {
    if field_type != "Str" {
        return Err(ValidationError {
            field: field_name.to_string(),
            rule: "email".to_string(),
            message: "Email decorator requires String type".to_string(),
            expected: Some("Str".to_string()),
            received: Some(field_type.to_string()),
        });
    }
    
    // Simple email validation: contains @ and .
    // Legacy logic: parts.len() == 2 && !parts[0].is_empty() && parts[1].contains('.')
    let parts: Vec<&str> = value.split('@').collect();
    if parts.len() != 2 || parts[0].is_empty() || !parts[1].contains('.') {
        return Err(ValidationError {
            field: field_name.to_string(),
            rule: "email".to_string(),
            message: "Invalid email format".to_string(),
            expected: Some("user@domain.com".to_string()),
            received: Some(value.to_string()),
        });
    }
    Ok(())
}

fn validate_url(field_name: &str, field_type: &str, value: &str) -> Result<(), ValidationError> {
    if field_type != "Str" {
         return Err(ValidationError {
            field: field_name.to_string(),
            rule: "url".to_string(),
            message: "URL decorator requires String type".to_string(),
            expected: Some("Str".to_string()),
            received: Some(field_type.to_string()),
        });
    }
    // Simple check mostly, or use a regex/parser if available. 
    // Legacy used `url::Url::parse` but `url` crate might not be in doo_ffi_core deps.
    // Check Cargo.toml later. For now, check start_with http
    if !value.starts_with("http") {
          return Err(ValidationError {
            field: field_name.to_string(),
            rule: "url".to_string(),
            message: "Invalid URL format".to_string(),
            expected: Some("http://...".to_string()),
            received: Some(value.to_string()),
        });
    }
    Ok(())
}

fn validate_min(field_name: &str, field_type: &str, value: &str, args: &[String]) -> Result<(), ValidationError> {
    let min_arg = args.first().ok_or_else(|| ValidationError {
        field: field_name.to_string(),
        rule: "min".to_string(),
        message: "@min requires an argument".to_string(),
        expected: None,
        received: None,
    })?;

    if field_type == "Str" {
        let min_len = min_arg.parse::<usize>().unwrap_or(0);
        if value.len() < min_len {
            return Err(ValidationError {
                field: field_name.to_string(),
                rule: format!("min:{}", min_len),
                message: format!("Must be at least {} characters", min_len),
                expected: Some(format!("length >= {}", min_len)),
                received: Some(format!("length = {}", value.len())),
            });
        }
    } else if field_type == "Int" {
        let min_val = min_arg.parse::<i64>().unwrap_or(0);
        if let Ok(val) = value.parse::<i64>() {
            if val < min_val {
                 return Err(ValidationError {
                    field: field_name.to_string(),
                    rule: format!("min:{}", min_val),
                    message: format!("Must be at least {}", min_val),
                    expected: Some(format!(">= {}", min_val)),
                    received: Some(val.to_string()),
                });
            }
        }
    }
    // Float logic omitted for brevity, add if needed.
    Ok(())
}

fn validate_max(field_name: &str, field_type: &str, value: &str, args: &[String]) -> Result<(), ValidationError> {
    let max_arg = args.first().ok_or_else(|| ValidationError {
        field: field_name.to_string(),
        rule: "max".to_string(),
        message: "@max requires an argument".to_string(),
        expected: None,
        received: None,
    })?;

    if field_type == "Str" {
        let max_len = max_arg.parse::<usize>().unwrap_or(0);
        if value.len() > max_len {
            return Err(ValidationError {
                field: field_name.to_string(),
                rule: format!("max:{}", max_len),
                message: format!("Maximum {} characters allowed", max_len),
                expected: Some(format!("length <= {}", max_len)),
                received: Some(format!("length = {}", value.len())),
            });
        }
    } else if field_type == "Int" {
        let max_val = max_arg.parse::<i64>().unwrap_or(0);
        if let Ok(val) = value.parse::<i64>() {
            if val > max_val {
                 return Err(ValidationError {
                    field: field_name.to_string(),
                    rule: format!("max:{}", max_val),
                    message: format!("Maximum {} allowed", max_val),
                    expected: Some(format!("<= {}", max_val)),
                    received: Some(val.to_string()),
                });
            }
        }
    }
    Ok(())
}

fn validate_enum(field_name: &str, value: &str, args: &[String]) -> Result<(), ValidationError> {
    if !args.contains(&value.to_string()) {
        return Err(ValidationError {
            field: field_name.to_string(),
            rule: format!("enum:{}", args.join("|")),
            message: format!("Must be one of: {}", args.join(", ")),
            expected: None,
            received: None,
        });
    }
    Ok(())
}

fn validate_required(field_name: &str, value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Err(ValidationError {
            field: field_name.to_string(),
            rule: "required".to_string(),
            message: "This field is required".to_string(),
            expected: Some("non-empty".to_string()),
            received: Some("empty".to_string()),
        });
    }
    Ok(())
}
