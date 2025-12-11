//! HTTP-specific MIR instructions for phases 6-8
//!
//! This module defines MIR instructions for HTTP handler operations:
//! - JSON parsing and serialization (Phase 6)
//! - Validation decorators (Phase 7)
//! - Type-safe parameters and error mapping (Phase 8)

use std::collections::HashMap;

/// HTTP Handler MIR Instructions (Phases 6-8)
#[derive(Debug, Clone, PartialEq)]
pub enum HttpMirInstr {
    /// Phase 6: Parse JSON from request body into struct
    /// parse_json(dest, body, struct_name, validators)
    /// Calls: doohttp_parse_json_struct(body, struct_name, validator_spec)
    ParseJsonStruct {
        dest: String,
        body: String,
        struct_name: String,
        validator_spec: String, // "field1:email;field2:min8|max100"
    },

    /// Phase 6: Validate struct field against decorators
    /// validate_field(field_name, field_value, struct_name, decorators)
    /// Calls: doohttp_validate_field_value(field_value, field_name, decorators)
    ValidateStructField {
        field_name: String,
        field_value: String,
        struct_name: String,
        decorator_spec: String, // "email|min8|max100"
    },

    /// Phase 6: Serialize struct to JSON
    /// serialize_to_json(dest, source, struct_name)
    /// Calls: doohttp_serialize_struct(source_ptr, struct_name)
    SerializeStruct {
        dest: String,
        source: String,
        struct_name: String,
    },

    /// Phase 8: Extract path parameter with type conversion
    /// extract_param_typed(dest, param_name, param_type)
    /// Calls: doohttp_extract_param_typed(request, param_name, type_name)
    ExtractPathParam {
        dest: String,
        param_name: String,
        param_type: String, // Int, Float, Bool, Str, UUID
    },

    /// Phase 8: Parse query parameters into struct
    /// parse_query_struct(dest, query_string, struct_name)
    /// Calls: doohttp_parse_query_struct(query_string, struct_name, defaults)
    ParseQueryStruct {
        dest: String,
        query_string: String,
        struct_name: String,
        defaults_spec: String, // "field1:value1;field2:value2"
    },

    /// Phase 8: Map error enum variant to HTTP status code
    /// error_to_status(dest, error_enum, variant_name)
    /// Calls: doohttp_error_to_status(error_type, variant)
    ErrorToStatus {
        dest: String,
        error_enum: String,
        variant_name: String,
    },

    /// Phase 8: Get error message from enum variant
    /// error_message(dest, error_enum, variant_name)
    /// Calls: doohttp_error_message(error_type, variant)
    ErrorMessage {
        dest: String,
        error_enum: String,
        variant_name: String,
    },

    /// Phase 4-5: Response header chaining
    /// response_header(dest, source, header_name, header_value)
    ResponseHeader {
        dest: String,
        source: String,
        header_name: String,
        header_value: String,
    },

    /// Phase 4-5: Response status setter
    /// response_status(dest, source, status_code)
    ResponseStatus {
        dest: String,
        source: String,
        status_code: u32,
    },

    /// Auto status code detection from return type
    /// auto_status(dest, return_type, status_or_error)
    /// 200 for simple return
    /// 201 for tuple with Int status
    /// error status for error enums
    AutoStatusCode {
        dest: String,
        return_type: String,
        value: String,
    },
}

impl HttpMirInstr {
    /// Get string representation for debugging
    pub fn to_debug_string(&self) -> String {
        match self {
            HttpMirInstr::ParseJsonStruct {
                dest,
                body,
                struct_name,
                validator_spec,
            } => {
                format!(
                    "parse_json_struct({}, {}, {}, {})",
                    dest, body, struct_name, validator_spec
                )
            }
            HttpMirInstr::ValidateStructField {
                field_name,
                field_value,
                struct_name,
                decorator_spec,
            } => {
                format!(
                    "validate_field({}, {}, {}, {})",
                    field_name, field_value, struct_name, decorator_spec
                )
            }
            HttpMirInstr::SerializeStruct {
                dest,
                source,
                struct_name,
            } => {
                format!("serialize_struct({}, {}, {})", dest, source, struct_name)
            }
            HttpMirInstr::ExtractPathParam {
                dest,
                param_name,
                param_type,
            } => {
                format!(
                    "extract_param_typed({}, {}, {})",
                    dest, param_name, param_type
                )
            }
            HttpMirInstr::ParseQueryStruct {
                dest,
                query_string,
                struct_name,
                defaults_spec,
            } => {
                format!(
                    "parse_query_struct({}, {}, {}, {})",
                    dest, query_string, struct_name, defaults_spec
                )
            }
            HttpMirInstr::ErrorToStatus {
                dest,
                error_enum,
                variant_name,
            } => {
                format!(
                    "error_to_status({}, {}, {})",
                    dest, error_enum, variant_name
                )
            }
            HttpMirInstr::ErrorMessage {
                dest,
                error_enum,
                variant_name,
            } => {
                format!("error_message({}, {}, {})", dest, error_enum, variant_name)
            }
            HttpMirInstr::ResponseHeader {
                dest,
                source,
                header_name,
                header_value,
            } => {
                format!(
                    "response_header({}, {}, {}, {})",
                    dest, source, header_name, header_value
                )
            }
            HttpMirInstr::ResponseStatus {
                dest,
                source,
                status_code,
            } => {
                format!("response_status({}, {}, {})", dest, source, status_code)
            }
            HttpMirInstr::AutoStatusCode {
                dest,
                return_type,
                value,
            } => {
                format!("auto_status({}, {}, {})", dest, return_type, value)
            }
        }
    }
}

/// HTTP Handler context for codegen
#[derive(Debug, Clone)]
pub struct HttpHandlerContext {
    pub handler_name: String,
    pub is_route_handler: bool,
    pub has_request_param: bool,
    pub request_param_name: Option<String>,
    pub input_struct_name: Option<String>,
    pub output_struct_name: Option<String>,
    pub error_type: Option<String>,
    pub path_params: Vec<(String, String)>, // (name, type)
    pub query_params: Option<String>,       // struct name
    pub middleware_chain: Vec<String>,      // middleware function names
    pub decorators_by_field: HashMap<String, Vec<String>>, // field -> decorators
}

impl Default for HttpHandlerContext {
    fn default() -> Self {
        HttpHandlerContext {
            handler_name: String::new(),
            is_route_handler: false,
            has_request_param: false,
            request_param_name: None,
            input_struct_name: None,
            output_struct_name: None,
            error_type: None,
            path_params: Vec::new(),
            query_params: None,
            middleware_chain: Vec::new(),
            decorators_by_field: HashMap::new(),
        }
    }
}

/// Validator spec builder for constructing validation strings
pub struct ValidatorSpecBuilder {
    specs: HashMap<String, Vec<String>>,
}

impl ValidatorSpecBuilder {
    pub fn new() -> Self {
        ValidatorSpecBuilder {
            specs: HashMap::new(),
        }
    }

    pub fn add_field(&mut self, field_name: &str) {
        self.specs.insert(field_name.to_string(), Vec::new());
    }

    pub fn add_validator(&mut self, field_name: &str, validator: String) {
        self.specs
            .entry(field_name.to_string())
            .or_insert_with(Vec::new)
            .push(validator);
    }

    pub fn add_email(&mut self, field_name: &str) {
        self.add_validator(field_name, "email".to_string());
    }

    pub fn add_min(&mut self, field_name: &str, min: i64) {
        self.add_validator(field_name, format!("min{}", min));
    }

    pub fn add_max(&mut self, field_name: &str, max: i64) {
        self.add_validator(field_name, format!("max{}", max));
    }

    pub fn add_enum(&mut self, field_name: &str, values: Vec<&str>) {
        let enum_str = format!("enum:{}", values.join("|"));
        self.add_validator(field_name, enum_str);
    }

    pub fn add_pattern(&mut self, field_name: &str, pattern: &str) {
        self.add_validator(field_name, format!("pattern:{}", pattern));
    }

    pub fn build(&self) -> String {
        self.specs
            .iter()
            .map(|(field, validators)| {
                if validators.is_empty() {
                    field.clone()
                } else {
                    format!("{}:{}", field, validators.join("|"))
                }
            })
            .collect::<Vec<_>>()
            .join(";")
    }
}

impl Default for ValidatorSpecBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validator_spec_builder() {
        let mut builder = ValidatorSpecBuilder::new();
        builder.add_field("email");
        builder.add_email("email");
        builder.add_field("password");
        builder.add_min("password", 8);
        builder.add_max("password", 100);

        let spec = builder.build();
        assert!(spec.contains("email:email"));
        assert!(spec.contains("password:min8|max100"));
    }

    #[test]
    fn test_http_mir_instr_debug() {
        let instr = HttpMirInstr::ParseJsonStruct {
            dest: "user".to_string(),
            body: "req_body".to_string(),
            struct_name: "User".to_string(),
            validator_spec: "email:email;password:min8|max100".to_string(),
        };

        let debug_str = instr.to_debug_string();
        assert!(debug_str.contains("parse_json_struct"));
        assert!(debug_str.contains("User"));
    }

    #[test]
    fn test_http_handler_context_default() {
        let ctx = HttpHandlerContext::default();
        assert_eq!(ctx.handler_name, "");
        assert!(!ctx.is_route_handler);
        assert!(!ctx.has_request_param);
        assert!(ctx.path_params.is_empty());
    }
}
