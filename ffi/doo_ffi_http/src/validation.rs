//! Schema Validation
//!
//! Validates request data against struct schemas using centralized metadata.
//! Handles decorator validation (@email, @min, @max) and enum variant validation.

use std::collections::HashMap;

use doo_ffi_core::FieldError;

use crate::error::*;
use crate::metadata::{get_enum_variants, get_struct_metadata};
use crate::router::get_frozen_routes;

// ============================================================================
// VALIDATION FUNCTIONS
// ============================================================================

/// Validate an item against its struct schema using centralized metadata.
/// Returns Ok(()) if valid, Err(error_json) if validation fails.
pub(crate) fn validate_item_against_schema(
    item: &serde_json::Value,
    resource_name: &str,
    path: &str,
) -> Result<(), String> {
    // Look up struct metadata for this resource (frozen registry — no lock)
    let struct_name = {
        let registry = get_frozen_routes();
        registry
            .crud_configs
            .iter()
            .find(|c| c.base_path.trim_start_matches('/') == resource_name)
            .map(|c| c.resource_struct.clone())
    };

    let struct_name = match struct_name {
        Some(name) => name,
        None => return Ok(()), // No schema registered, skip validation
    };

    // Get struct metadata
    let struct_meta = match get_struct_metadata(&struct_name) {
        Some(meta) => meta,
        None => return Ok(()), // No metadata, skip validation
    };

    let obj = match item.as_object() {
        Some(o) => o,
        None => {
            return Err(Rfc7807Error::bad_request("Expected JSON object")
                .with_instance(path)
                .to_json())
        }
    };

    // Validate each field against its type and decorators
    for field_meta in &struct_meta.fields {
        if let Some(value) = obj.get(&field_meta.name) {
            // Check if field type is an enum
            if let Some(variants) = get_enum_variants(&field_meta.field_type) {
                // Validate enum value - case-insensitive matching
                if let Some(str_val) = value.as_str() {
                    let str_val_lower = str_val.to_lowercase();
                    if !variants.iter().any(|v| v.to_lowercase() == str_val_lower) {
                        let mut fields = HashMap::new();
                        fields.insert(
                            field_meta.name.clone(),
                            FieldError::new(
                                &field_meta.name,
                                format!("Must be one of: {}", variants.join(", ")),
                            )
                            .with_rule(format!("enum:{}", variants.join("|")))
                            .with_value(str_val),
                        );
                        return Err(Rfc7807Error::validation_error(fields)
                            .with_instance(path)
                            .to_json());
                    }
                }
            }

            // Validate JSON value type matches Doo field type
            if let Err(e) = validate_field_type(&field_meta.field_type, &field_meta.name, value, path) {
                return Err(e);
            }

            // Validate decorators (e.g., @email, @min, @max)
            for decorator in &field_meta.decorators {
                if let Err(e) = validate_decorator(decorator, &field_meta.name, value, path) {
                    return Err(e);
                }
            }
        }
    }

    Ok(())
}

/// Validate that a JSON value's type matches the Doo field type.
/// Returns Err with RFC 7807 validation error on mismatch.
fn validate_field_type(
    doo_type: &str,
    field_name: &str,
    value: &serde_json::Value,
    path: &str,
) -> Result<(), String> {
    let type_ok = match doo_type {
        "Str" => value.is_string(),
        "Int" => value.is_number() && value.as_f64().map_or(false, |n| n == n as i64 as f64),
        "Float" => value.is_number(),
        "Bool" => value.is_boolean(),
        // Complex types (structs, arrays, enums, maps, optionals) — accept objects/arrays/strings
        _ => true,
    };
    if !type_ok {
        let mut fields = HashMap::new();
        fields.insert(
            field_name.to_string(),
            FieldError::new(field_name, format!("Expected type '{}'", doo_type))
                .with_rule(format!("type:{}", doo_type.to_lowercase()))
                .with_value(&value.to_string()),
        );
        return Err(Rfc7807Error::validation_error(fields)
            .with_instance(path)
            .to_json());
    }
    Ok(())
}

/// Decorators that indicate a field is filled by the server and not required in the request body.
const SERVER_FILLED_DECORATORS: &[&str] = &["auto", "owner"];

/// Check that all required struct fields are present in the request body.
/// Fields decorated with @auto or @owner are server-filled and are skipped.
/// Returns `Err(message)` for the first missing required field.
pub(crate) fn validate_required_fields(
    item: &serde_json::Value,
    resource_name: &str,
    _path: &str,
) -> Result<(), String> {
    let struct_name = {
        let registry = get_frozen_routes();
        registry
            .crud_configs
            .iter()
            .find(|c| c.base_path.trim_start_matches('/') == resource_name)
            .map(|c| c.resource_struct.clone())
    };
    let struct_name = match struct_name {
        Some(name) => name,
        None => return Ok(()), // No schema registered, skip
    };
    let struct_meta = match get_struct_metadata(&struct_name) {
        Some(meta) => meta,
        None => return Ok(()), // No metadata, skip
    };
    let obj = match item.as_object() {
        Some(o) => o,
        None => return Err("Expected JSON object".to_string()),
    };
    for field_meta in &struct_meta.fields {
        let is_server_filled = field_meta
            .decorators
            .iter()
            .any(|d| SERVER_FILLED_DECORATORS.contains(&d.as_str()));
        if !is_server_filled {
            let name_lower = field_meta.name.to_lowercase();
            let present = obj.contains_key(&field_meta.name)
                || obj.keys().any(|k| k.to_lowercase() == name_lower);
            if !present {
                return Err(format!("Missing required field: '{}'", field_meta.name));
            }
        }
    }
    Ok(())
}

/// Validate an item against a struct by name (bypasses CRUD route lookup).
/// Used by auth signup to validate enum fields (e.g. Role) against registered variants.
pub(crate) fn validate_item_against_struct(
    item: &serde_json::Value,
    struct_name: &str,
    path: &str,
) -> Result<(), String> {
    let struct_meta = match get_struct_metadata(struct_name) {
        Some(meta) => meta,
        None => return Ok(()),
    };
    let obj = match item.as_object() {
        Some(o) => o,
        None => return Ok(()),
    };
    for field_meta in &struct_meta.fields {
        let field_name_lower = field_meta.name.to_lowercase();
        let value = obj.get(&field_meta.name).or_else(|| {
            obj.iter()
                .find(|(k, _)| k.to_lowercase() == field_name_lower)
                .map(|(_, v)| v)
        });

        if let Some(value) = value {
            if let Some(variants) = get_enum_variants(&field_meta.field_type) {
                if let Some(str_val) = value.as_str() {
                    let str_val_lower = str_val.to_lowercase();
                    if !variants.iter().any(|v| v.to_lowercase() == str_val_lower) {
                        return Err(format!(
                            "Invalid value '{}' for field '{}'. Must be one of: {}",
                            str_val,
                            field_meta.name,
                            variants.join(", ")
                        ));
                    }
                }
            }
            for decorator in &field_meta.decorators {
                if let Err(e) = validate_decorator(decorator, &field_meta.name, value, path) {
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}
pub(crate) fn validate_decorator(
    decorator: &str,
    field_name: &str,
    value: &serde_json::Value,
    path: &str,
) -> Result<(), String> {
    if decorator == "email" {
        if let Some(s) = value.as_str() {
            if !s.contains('@') || !s.contains('.') {
                let mut fields = HashMap::new();
                fields.insert(
                    field_name.to_string(),
                    FieldError::new(field_name, "Invalid email format")
                        .with_rule("email")
                        .with_value(s),
                );
                return Err(Rfc7807Error::validation_error(fields)
                    .with_instance(path)
                    .to_json());
            }
        }
    } else if decorator.starts_with("min(") && decorator.ends_with(')') {
        let min_str = &decorator[4..decorator.len() - 1];
        if let Ok(min_val) = min_str.parse::<i64>() {
            if let Some(s) = value.as_str() {
                if (s.len() as i64) < min_val {
                    let mut fields = HashMap::new();
                    fields.insert(
                        field_name.to_string(),
                        FieldError::new(
                            field_name,
                            format!("Must be at least {} characters", min_val),
                        )
                        .with_rule(format!("min:{}", min_val))
                        .with_value(s),
                    );
                    return Err(Rfc7807Error::validation_error(fields)
                        .with_instance(path)
                        .to_json());
                }
            } else if let Some(n) = value.as_i64() {
                if n < min_val {
                    let mut fields = HashMap::new();
                    fields.insert(
                        field_name.to_string(),
                        FieldError::new(field_name, format!("Must be at least {}", min_val))
                            .with_rule(format!("min:{}", min_val))
                            .with_value(n.to_string()),
                    );
                    return Err(Rfc7807Error::validation_error(fields)
                        .with_instance(path)
                        .to_json());
                }
            }
        }
    } else if decorator.starts_with("max(") && decorator.ends_with(')') {
        let max_str = &decorator[4..decorator.len() - 1];
        if let Ok(max_val) = max_str.parse::<i64>() {
            if let Some(s) = value.as_str() {
                if (s.len() as i64) > max_val {
                    let mut fields = HashMap::new();
                    fields.insert(
                        field_name.to_string(),
                        FieldError::new(
                            field_name,
                            format!("Maximum {} characters allowed", max_val),
                        )
                        .with_rule(format!("max:{}", max_val))
                        .with_value(s),
                    );
                    return Err(Rfc7807Error::validation_error(fields)
                        .with_instance(path)
                        .to_json());
                }
            } else if let Some(n) = value.as_i64() {
                if n > max_val {
                    let mut fields = HashMap::new();
                    fields.insert(
                        field_name.to_string(),
                        FieldError::new(field_name, format!("Maximum {} allowed", max_val))
                            .with_rule(format!("max:{}", max_val))
                            .with_value(n.to_string()),
                    );
                    return Err(Rfc7807Error::validation_error(fields)
                        .with_instance(path)
                        .to_json());
                }
            }
        }
    }

    Ok(())
}
