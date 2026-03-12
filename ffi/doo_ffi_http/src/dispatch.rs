//! Handler Dispatch
//!
//! Registration of handler function pointers and associated metadata
//! (parameter types, struct layouts, enum variants, return type).

use std::collections::HashMap;
use std::os::raw::c_char;

use doo_ffi_core::ffi_safe_void;

use crate::helpers::c_to_string;
use crate::router::get_routes;
use crate::types::*;

// ============================================================================
// HANDLER REGISTRATION
// ============================================================================

#[no_mangle]
pub extern "C" fn doo_http_register_handler(name: *const c_char, handler: DooHandlerFn) {
    ffi_safe_void!({
        let name_str = c_to_string(name);
        let routes = get_routes();
        let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());
        registry.register_handler(&name_str, handler);
    });
}

#[no_mangle]
pub extern "C" fn doo_http_register_handler_with_metadata(
    name: *const c_char,
    handler: DooHandlerFn,
    metadata_json: *const c_char,
) {
    ffi_safe_void!({
        let name_str = c_to_string(name);
        let routes = get_routes();
        let mut registry = routes.lock().unwrap_or_else(|e| e.into_inner());

        // Parse metadata JSON properly
        let metadata = if metadata_json.is_null() {
            HandlerMetadata::default()
        } else {
            let json_str = c_to_string(metadata_json);
            parse_handler_metadata(&json_str).unwrap_or_default()
        };

        registry.register_handler_with_metadata(&name_str, handler, metadata);
    });
}

/// Parse handler metadata JSON into HandlerMetadata struct
pub(crate) fn parse_handler_metadata(json_str: &str) -> Option<HandlerMetadata> {
    let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let obj = parsed.as_object()?;

    // Extract param_types array
    let param_types = obj
        .get("param_types")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // Extract struct_layouts map
    let struct_layouts = obj
        .get("struct_layouts")
        .and_then(|v| v.as_object())
        .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    // Extract enum_variants map
    let enum_variants = obj
        .get("enum_variants")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| {
                    v.as_array().map(|arr| {
                        let variants: Vec<String> = arr
                            .iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect();
                        (k.clone(), variants)
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Extract return_type
    let return_type = obj
        .get("return_type")
        .and_then(|v: &serde_json::Value| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Void".to_string());

    Some(HandlerMetadata {
        param_types,
        return_type,
        struct_decorators: HashMap::new(),
        struct_layouts,
        enum_variants,
    })
}
