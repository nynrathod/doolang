//! Map Builder Functions
//!
//! Used by codegen to convert object literals `{ key: value, ... }` into
//! HashMap<String, String> compatible with doo_map_get_str and FFI config parsing.
//! This is the single source of truth for building maps from Doo object literals.

use std::collections::HashMap;
use std::ffi::{c_void, CStr};
use std::os::raw::c_char;

use doo_ffi_core::{ffi_safe_ptr, ffi_safe_void};

use crate::helpers::{c_to_string, string_to_c};

// ============================================================================
// MAP BUILDER FUNCTIONS
// ============================================================================

/// Create a new empty map (HashMap<String, String>)
#[no_mangle]
pub extern "C" fn doo_map_new() -> *mut c_void {
    ffi_safe_ptr!({
        let map: HashMap<String, String> = HashMap::new();
        let boxed = Box::new(map);
        Box::into_raw(boxed) as *mut c_void
    })
}

/// Set a string key-value pair in the map
#[no_mangle]
pub extern "C" fn doo_map_set(map: *mut c_void, key: *const c_char, value: *const c_char) {
    ffi_safe_void!({
        if map.is_null() || key.is_null() {
            return;
        }
        unsafe {
            let map = &mut *(map as *mut HashMap<String, String>);
            let k = CStr::from_ptr(key).to_string_lossy().to_string();
            let v = if value.is_null() {
                String::new()
            } else {
                CStr::from_ptr(value).to_string_lossy().to_string()
            };
            map.insert(k, v);
        }
    });
}

/// Set a string array value as comma-separated string in the map.
/// arr_data points to the Doo array data area (header is 16 bytes before).
/// Array layout: [i64 len, i64 cap, ptr elem0, ptr elem1, ...]
#[no_mangle]
pub extern "C" fn doo_map_set_str_array(
    map: *mut c_void,
    key: *const c_char,
    arr_data: *const c_void,
) {
    ffi_safe_void!({
        if map.is_null() || key.is_null() || arr_data.is_null() {
            return;
        }
        unsafe {
            let map = &mut *(map as *mut HashMap<String, String>);
            let k = CStr::from_ptr(key).to_string_lossy().to_string();

            // Array data pointer is at offset +16 from header
            // Header: [len: i64 at -16, cap: i64 at -8]
            let len_ptr = (arr_data as *const u8).sub(16) as *const i64;
            let len = *len_ptr as usize;

            let data_ptr = arr_data as *const *const c_char;
            let mut values = Vec::new();
            for i in 0..len {
                let elem = *data_ptr.add(i);
                if !elem.is_null() {
                    values.push(CStr::from_ptr(elem).to_string_lossy().to_string());
                }
            }
            map.insert(k, values.join(","));
        }
    });
}

// ============================================================================
// MAP HELPERS — used by middleware_ffi, request, etc.
// ============================================================================

/// Get a string value from a HashMap<String, String> by key
pub(crate) fn doo_map_get_str(map_ptr: *const c_void, key: &str) -> *const c_char {
    if map_ptr.is_null() {
        return std::ptr::null();
    }
    unsafe {
        let map = &*(map_ptr as *const HashMap<String, String>);
        match map.get(key) {
            Some(v) => string_to_c(v),
            None => std::ptr::null(),
        }
    }
}

pub(crate) fn parse_json_i64_or_default(ptr: *const c_char, default: i64) -> i64 {
    if ptr.is_null() {
        return default;
    }
    let s = c_to_string(ptr);
    // Handle JSON-encoded numbers (might be quoted or unquoted)
    let trimmed = s.trim().trim_matches('"');
    trimmed.parse::<i64>().unwrap_or(default)
}

pub(crate) fn parse_json_string_or_default(ptr: *const c_char, default: &str) -> String {
    if ptr.is_null() {
        return default.to_string();
    }
    let s = c_to_string(ptr);
    // Remove JSON quotes if present
    let trimmed = s.trim().trim_matches('"');
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn parse_json_bool_or_default(ptr: *const c_char, default: bool) -> bool {
    if ptr.is_null() {
        return default;
    }
    let s = c_to_string(ptr).trim().to_lowercase();
    match s.as_str() {
        "true" | "\"true\"" | "1" => true,
        "false" | "\"false\"" | "0" => false,
        _ => default,
    }
}
