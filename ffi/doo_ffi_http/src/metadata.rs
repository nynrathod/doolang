//! Struct/Enum Metadata Registry
//!
//! Single Source of Truth for struct/enum metadata used by auth, crud, and handlers.
//! The compiler emits calls to register metadata, which is then used at runtime
//! for validation, serialization, and response filtering.

use std::collections::HashMap;
use std::os::raw::c_char;
use std::sync::{Mutex as StdMutex, OnceLock};

use doo_ffi_core::ffi_safe_void;

use crate::helpers::c_to_string;

// ============================================================================
// GLOBAL STRUCT/ENUM METADATA REGISTRY
// ============================================================================

/// Global struct metadata registry - stores field info for structs
static STRUCT_REGISTRY: OnceLock<StdMutex<HashMap<String, StructMetadata>>> = OnceLock::new();

/// Global enum metadata registry - stores variants for enums
static ENUM_REGISTRY: OnceLock<StdMutex<HashMap<String, Vec<String>>>> = OnceLock::new();

pub(crate) fn get_struct_registry() -> &'static StdMutex<HashMap<String, StructMetadata>> {
    STRUCT_REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()))
}

pub(crate) fn get_enum_registry() -> &'static StdMutex<HashMap<String, Vec<String>>> {
    ENUM_REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// Struct metadata for runtime validation
#[derive(Clone, Debug)]
pub struct StructMetadata {
    pub name: String,
    pub fields: Vec<FieldMetadata>,
}

/// Field metadata for runtime validation
#[derive(Clone, Debug)]
pub struct FieldMetadata {
    pub name: String,
    pub field_type: String,
    pub decorators: Vec<String>, // e.g., ["email", "min(3)", "max(50)"]
}

/// Register struct metadata from the compiler.
/// Called by codegen when processing structs used with auth/crud/handlers.
#[no_mangle]
pub extern "C" fn doo_http_register_struct_metadata(
    struct_name: *const c_char,
    metadata_json: *const c_char,
) {
    ffi_safe_void!({
        let name = c_to_string(struct_name);
        let json_str = c_to_string(metadata_json);

        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_str) {
            let mut fields = Vec::new();

            if let Some(fields_arr) = parsed.get("fields").and_then(|v| v.as_array()) {
                for field in fields_arr {
                    if let (Some(fname), Some(ftype)) = (
                        field
                            .get("name")
                            .and_then(|v: &serde_json::Value| v.as_str()),
                        field
                            .get("type")
                            .and_then(|v: &serde_json::Value| v.as_str()),
                    ) {
                        let decorators: Vec<String> = field
                            .get("decorators")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default();

                        fields.push(FieldMetadata {
                            name: fname.to_string(),
                            field_type: ftype.to_string(),
                            decorators,
                        });
                    }
                }
            }

            let mut registry = get_struct_registry()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            registry.insert(name.clone(), StructMetadata { name, fields });
        }
    });
}

/// Register enum metadata from the compiler.
#[no_mangle]
pub extern "C" fn doo_http_register_enum_metadata(
    enum_name: *const c_char,
    variants_json: *const c_char,
) {
    ffi_safe_void!({
        let name = c_to_string(enum_name);
        let json_str = c_to_string(variants_json);

        if let Ok(parsed) = serde_json::from_str::<Vec<String>>(&json_str) {
            let mut registry = get_enum_registry()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            registry.insert(name, parsed);
        }
    });
}

/// Get struct metadata by name (used by auth/crud handlers)
pub(crate) fn get_struct_metadata(name: &str) -> Option<StructMetadata> {
    let registry = get_struct_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    registry.get(name).cloned()
}

/// Normalize a field name to a canonical form for comparison.
/// Strips underscores and lowercases, so "InternalId" and "internal_id" both become "internalid".
fn normalize_field_name(name: &str) -> String {
    name.chars()
        .filter(|c| *c != '_')
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Check if two field names refer to the same field, regardless of casing convention.
/// Handles PascalCase ("InternalId") == snake_case ("internal_id") == camelCase ("internalId").
pub(crate) fn field_names_match(a: &str, b: &str) -> bool {
    normalize_field_name(a) == normalize_field_name(b)
}

/// Check if a field should be included in response output.
/// Fields with @writeOnly or @internal decorators are EXCLUDED from responses.
pub(crate) fn should_include_in_response(struct_name: &str, field_name: &str) -> bool {
    if let Some(meta) = get_struct_metadata(struct_name) {
        for field in &meta.fields {
            if field_names_match(&field.name, field_name) {
                // @writeOnly: accepted in request, hidden from response
                // @internal: hidden from both request and response
                if field
                    .decorators
                    .iter()
                    .any(|d| d == "writeOnly" || d == "internal")
                {
                    return false;
                }
                return true;
            }
        }
    }
    true // Unknown struct/field — include by default
}

/// Check if a field should be accepted from request input.
/// Fields with @readOnly or @internal decorators are IGNORED from requests.
pub(crate) fn should_accept_from_request(struct_name: &str, field_name: &str) -> bool {
    if let Some(meta) = get_struct_metadata(struct_name) {
        for field in &meta.fields {
            if field_names_match(&field.name, field_name) {
                // @readOnly: visible in response, rejected from request
                // @internal: hidden from both request and response
                if field
                    .decorators
                    .iter()
                    .any(|d| d == "readOnly" || d == "internal")
                {
                    return false;
                }
                return true;
            }
        }
    }
    true // Unknown struct/field — accept by default
}

/// Filter response JSON to remove fields with @writeOnly or @internal decorators.
/// Works for both individual objects and arrays of objects.
pub(crate) fn filter_response_fields(
    value: &serde_json::Value,
    struct_name: &str,
) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut filtered = serde_json::Map::new();
            for (k, v) in map {
                if should_include_in_response(struct_name, k) {
                    filtered.insert(k.clone(), v.clone());
                }
            }
            serde_json::Value::Object(filtered)
        }
        serde_json::Value::Array(arr) => {
            let filtered: Vec<serde_json::Value> = arr
                .iter()
                .map(|item| filter_response_fields(item, struct_name))
                .collect();
            serde_json::Value::Array(filtered)
        }
        _ => value.clone(),
    }
}

/// Get enum variants by name (used for validation)
pub(crate) fn get_enum_variants(name: &str) -> Option<Vec<String>> {
    let registry = get_enum_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    registry.get(name).cloned()
}
