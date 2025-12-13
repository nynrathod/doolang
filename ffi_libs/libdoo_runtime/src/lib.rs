use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::ffi::{CStr, CString};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FieldDecorator {
    name: String,
    args: Vec<String>,
}

/// Structured validation error for RFC 7807 responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub field_name: String,
    pub rule: String,
    pub message: String,
    pub value: String,
}

/// Thread-local storage for the last validation error (for HTTP context)
thread_local! {
    static LAST_VALIDATION_ERROR: RefCell<Option<ValidationError>> = RefCell::new(None);
}

fn set_validation_error(error: ValidationError) {
    LAST_VALIDATION_ERROR.with(|cell| {
        *cell.borrow_mut() = Some(error);
    });
}

fn clear_validation_error() {
    LAST_VALIDATION_ERROR.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

/// Get the last validation error as JSON string
/// Returns null if no error, or JSON string with error details
#[no_mangle]
pub extern "C" fn dooruntime_get_last_validation_error() -> *mut libc::c_char {
    LAST_VALIDATION_ERROR.with(|cell| {
        if let Some(error) = cell.borrow().as_ref() {
            if let Ok(json) = serde_json::to_string(error) {
                if let Ok(c_str) = CString::new(json) {
                    return c_str.into_raw();
                }
            }
        }
        std::ptr::null_mut()
    })
}

/// Clear the last validation error
#[no_mangle]
pub extern "C" fn dooruntime_clear_validation_error() {
    clear_validation_error();
}

/// Validate a single field with its decorators
/// Returns error message as C string, or null if validation passes
/// Also stores structured error in thread-local for HTTP RFC 7807 responses
#[no_mangle]
pub extern "C" fn dooruntime_validate_field(
    field_name: *const libc::c_char,
    field_type: *const libc::c_char,
    value: *const libc::c_char,
    decorators_json: *const libc::c_char,
) -> *const libc::c_char {
    if field_name.is_null() || field_type.is_null() || value.is_null() || decorators_json.is_null()
    {
        return std::ptr::null();
    }

    let field_name_str = unsafe { CStr::from_ptr(field_name).to_string_lossy().to_string() };
    let field_type_str = unsafe { CStr::from_ptr(field_type).to_string_lossy().to_string() };
    let value_str = unsafe { CStr::from_ptr(value).to_string_lossy().to_string() };
    let decorators_str = unsafe {
        CStr::from_ptr(decorators_json)
            .to_string_lossy()
            .to_string()
    };

    // Parse decorators JSON
    let decorators: Vec<FieldDecorator> = match serde_json::from_str(&decorators_str) {
        Ok(d) => d,
        Err(_) => return std::ptr::null(), // Invalid JSON, no validation
    };

    // Clear any previous validation error
    clear_validation_error();

    // Validate each decorator
    match validate_field_decorators(&field_name_str, &field_type_str, &value_str, &decorators) {
        Ok(_) => std::ptr::null(), // No error
        Err((err_msg, rule, message)) => {
            // Store structured error for HTTP context
            let validation_error = ValidationError {
                field_name: field_name_str.clone(),
                rule,
                message: message.clone(),
                value: value_str.clone(),
            };
            set_validation_error(validation_error);

            // Return simple error message as C string (backward compatibility)
            match CString::new(err_msg) {
                Ok(c_str) => c_str.into_raw(),
                Err(_) => std::ptr::null(),
            }
        }
    }
}

/// Free a string allocated by this library
#[no_mangle]
pub extern "C" fn dooruntime_free_string(ptr: *mut libc::c_char) {
    if !ptr.is_null() {
        unsafe {
            let _ = CString::from_raw(ptr);
        }
    }
}

fn validate_field_decorators(
    field_name: &str,
    field_type: &str,
    value: &str,
    decorators: &[FieldDecorator],
) -> Result<(), (String, String, String)> {
    for decorator in decorators {
        match decorator.name.as_str() {
            "email" => {
                if field_type != "Str" {
                    return Err((
                        format!(
                            "Field '{}' has @email decorator but type is {}, expected Str",
                            field_name, field_type
                        ),
                        "email".to_string(),
                        "Email decorator requires String type".to_string(),
                    ));
                }
                // Email validation: must contain @ and . with chars before/after @
                let parts: Vec<&str> = value.split('@').collect();
                if parts.len() != 2 || parts[0].is_empty() || !parts[1].contains('.') {
                    return Err((
                        format!(
                            "Field '{}': '{}' is not a valid email address",
                            field_name, value
                        ),
                        "email".to_string(),
                        "Invalid email format".to_string(),
                    ));
                }
            }
            "min" => {
                if let Some(min_arg) = decorator.args.first() {
                    if field_type == "Str" {
                        // For strings, min is length
                        if let Ok(min_len) = min_arg.parse::<usize>() {
                            if value.len() < min_len {
                                return Err((
                                    format!(
                                        "Field '{}': must have at least {} characters (got {})",
                                        field_name,
                                        min_len,
                                        value.len()
                                    ),
                                    format!("min:{}", min_len),
                                    format!("Must be at least {} characters", min_len),
                                ));
                            }
                        }
                    } else if field_type == "Int" {
                        // For Int, min is numeric value
                        if let Ok(min_val) = min_arg.parse::<i64>() {
                            if let Ok(val) = value.parse::<i64>() {
                                if val < min_val {
                                    return Err((
                                        format!(
                                            "Field '{}': value {} is below minimum {}",
                                            field_name, val, min_val
                                        ),
                                        format!("min:{}", min_val),
                                        format!("Must be at least {}", min_val),
                                    ));
                                }
                            }
                        }
                    } else if field_type == "Float" {
                        // For Float, min is numeric value
                        if let Ok(min_val) = min_arg.parse::<f64>() {
                            if let Ok(val) = value.parse::<f64>() {
                                if val < min_val {
                                    return Err((
                                        format!(
                                            "Field '{}': value {} is below minimum {}",
                                            field_name, val, min_val
                                        ),
                                        format!("min:{}", min_val),
                                        format!("Must be at least {}", min_val),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            "max" => {
                if let Some(max_arg) = decorator.args.first() {
                    if field_type == "Str" {
                        // For strings, max is length
                        if let Ok(max_len) = max_arg.parse::<usize>() {
                            if value.len() > max_len {
                                return Err((
                                    format!(
                                        "Field '{}': must have at most {} characters (got {})",
                                        field_name,
                                        max_len,
                                        value.len()
                                    ),
                                    format!("max:{}", max_len),
                                    format!("Maximum {} characters allowed", max_len),
                                ));
                            }
                        }
                    } else if field_type == "Int" {
                        // For Int, max is numeric value
                        if let Ok(max_val) = max_arg.parse::<i64>() {
                            if let Ok(val) = value.parse::<i64>() {
                                if val > max_val {
                                    return Err((
                                        format!(
                                            "Field '{}': value {} exceeds maximum {}",
                                            field_name, val, max_val
                                        ),
                                        format!("max:{}", max_val),
                                        format!("Maximum {} allowed", max_val),
                                    ));
                                }
                            }
                        }
                    } else if field_type == "Float" {
                        // For Float, max is numeric value
                        if let Ok(max_val) = max_arg.parse::<f64>() {
                            if let Ok(val) = value.parse::<f64>() {
                                if val > max_val {
                                    return Err((
                                        format!(
                                            "Field '{}': value {} exceeds maximum {}",
                                            field_name, val, max_val
                                        ),
                                        format!("max:{}", max_val),
                                        format!("Maximum {} allowed", max_val),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            "enum" => {
                if field_type != "Str" {
                    return Err((
                        format!(
                            "Field '{}' has @enum decorator but type is {}, expected Str",
                            field_name, field_type
                        ),
                        "enum".to_string(),
                        "Enum decorator requires String type".to_string(),
                    ));
                }
                // Check if value is in the allowed enum list
                if !decorator.args.contains(&value.to_string()) {
                    return Err((
                        format!(
                            "Field '{}': '{}' is not a valid option. Must be one of: {}",
                            field_name,
                            value,
                            decorator.args.join(", ")
                        ),
                        format!("enum({})", decorator.args.join("|")),
                        format!("Must be one of: {}", decorator.args.join(", ")),
                    ));
                }
            }
            "required" => {
                if value.is_empty() {
                    return Err((
                        format!("Field '{}' is required and cannot be empty", field_name),
                        "required".to_string(),
                        "This field is required".to_string(),
                    ));
                }
            }
            "optional" => {
                // Always valid - just marks field as optional
            }
            "unique" => {
                // @unique is DB-specific and must be validated at DB layer
                // Runtime can't check uniqueness without DB query
                // This is a no-op here; DB FFI will handle it
            }
            _ => {
                // Unknown decorator - ignore
            }
        }
    }
    Ok(())
}
