//! JSON Parsing — type-specific parse functions for Doo's JSON FFI.
//!
//! Provides typed JSON parsing for scalars (Int, Float, Bool, Str),
//! typed arrays ([Int], [Float], [Bool], [Str]),
//! typed maps ({K: V} for all key/value type combinations),
//! struct/enum helpers (field extraction, variant parsing),
//! and the parse-once/get-field-many optimization API.
//!
//! All functions use catch_unwind for panic safety at FFI boundaries.
//! Type mismatches set RFC 7807 errors via the thread-local error state.

use doo_ffi_core::ffi_debug;
use doo_ffi_core::helpers::c_to_string_lossy;
use doo_ffi_core::memory::{
    doo_alloc, doo_alloc_empty_string, doo_alloc_string, MIN_ALLOCATION_SIZE,
};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::{
    alloc_empty_array, alloc_empty_map, check_json_size, json_value_type_name,
    set_parse_error_rfc7807,
};

// ============================================================================
// Type-Specific JSON Parse Functions
// ============================================================================

/// Parse JSON to Int
/// Handles both JSON numbers (123) and JSON strings containing numbers ("123")
/// Uses direct from_str::<i64> first (no Value tree allocation) for performance.
#[no_mangle]
pub extern "C" fn doo_json_parse_int(json_str: *const c_char) -> i64 {
    ffi_debug!("JSON", "doo_json_parse_int called, json_str={:?}", json_str);
    if json_str.is_null() {
        ffi_debug!("JSON", "doo_json_parse_int: null input");
        return 0;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = c_to_string_lossy(json_str);
        if check_json_size(&s).is_err() {
            return 0;
        }
        ffi_debug!("JSON", "doo_json_parse_int: input='{}'", s);
        // Fast path: direct parse (no Value tree)
        if let Ok(i) = serde_json::from_str::<i64>(&s) {
            ffi_debug!("JSON", "doo_json_parse_int: fast-path result={}", i);
            return i;
        }
        // Fallback: try string-containing-number ("123")
        if let Ok(sv) = serde_json::from_str::<String>(&s) {
            if let Ok(i) = sv.parse::<i64>() {
                ffi_debug!("JSON", "doo_json_parse_int: string-fallback result={}", i);
                return i;
            }
        }
        ffi_debug!("JSON", "doo_json_parse_int: fallback to 0");
        0
    }))
    .unwrap_or(0)
}

/// Parse JSON to Float
/// Handles both JSON numbers (1.5) and JSON strings containing numbers ("1.5")
/// Uses direct from_str::<f64> first (no Value tree allocation) for performance.
#[no_mangle]
pub extern "C" fn doo_json_parse_float(json_str: *const c_char) -> f64 {
    if json_str.is_null() {
        return 0.0;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = c_to_string_lossy(json_str);
        if check_json_size(&s).is_err() {
            return 0.0;
        }
        // Fast path: direct parse (no Value tree)
        if let Ok(f) = serde_json::from_str::<f64>(&s) {
            return f;
        }
        // Fallback: try string-containing-number ("1.5")
        if let Ok(sv) = serde_json::from_str::<String>(&s) {
            if let Ok(f) = sv.parse::<f64>() {
                return f;
            }
        }
        0.0
    }))
    .unwrap_or(0.0)
}

/// Parse JSON to Bool
/// Handles both JSON booleans (true) and JSON strings containing booleans ("true")
/// Uses direct from_str::<bool> first (no Value tree allocation) for performance.
#[no_mangle]
pub extern "C" fn doo_json_parse_bool(json_str: *const c_char) -> i32 {
    if json_str.is_null() {
        return 0i32;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = c_to_string_lossy(json_str);
        if check_json_size(&s).is_err() {
            return 0i32;
        }
        // Fast path: direct parse (no Value tree)
        if let Ok(b) = serde_json::from_str::<bool>(&s) {
            return b as i32;
        }
        // Fallback: try string-containing-boolean ("true")
        if let Ok(sv) = serde_json::from_str::<String>(&s) {
            if let Ok(b) = sv.parse::<bool>() {
                return b as i32;
            }
        }
        0i32
    }))
    .unwrap_or(0i32)
}

/// Parse JSON to Str (returns C string pointer)
/// Returns empty string if parsing fails or null input, never returns null
/// OWNERSHIP: Caller owns the returned string.
#[no_mangle]
pub extern "C" fn doo_json_parse_str(json_str: *const c_char) -> *mut c_char {
    if json_str.is_null() {
        return doo_alloc_empty_string();
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = c_to_string_lossy(json_str);
        if check_json_size(&s).is_err() {
            return doo_alloc_empty_string();
        }
        // Direct string parse (no Value tree)
        match serde_json::from_str::<String>(&s) {
            Ok(result) => doo_alloc_string(&result),
            Err(_) => doo_alloc_empty_string(),
        }
    }))
    .unwrap_or_else(|_| doo_alloc_empty_string())
}

// ============================================================================
// Array Helper Functions (for codegen-driven struct/enum array parsing)
// ============================================================================

/// Get the number of elements in a JSON array.
/// Returns 0 for null input, parse errors, or non-array JSON.
#[no_mangle]
pub extern "C" fn doo_json_array_count(json_str: *const c_char) -> i64 {
    if json_str.is_null() {
        return 0;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = c_to_string_lossy(json_str);
        if check_json_size(&s).is_err() {
            return 0;
        }
        match serde_json::from_str::<serde_json::Value>(&s) {
            Ok(serde_json::Value::Array(arr)) => {
                ffi_debug!("FFI", "doo_json_array_count: {} elements", arr.len());
                arr.len() as i64
            }
            _ => {
                ffi_debug!("FFI", "doo_json_array_count: not an array -> 0");
                0
            }
        }
    }))
    .unwrap_or(0)
}

/// Get the JSON string representation of an element at a given index in a JSON array.
/// Returns null for out-of-bounds, null input, or non-array JSON.
/// OWNERSHIP: Caller owns the returned string.
#[no_mangle]
pub extern "C" fn doo_json_array_get_element(json_str: *const c_char, index: i64) -> *mut c_char {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = c_to_string_lossy(json_str);
        if check_json_size(&s).is_err() {
            return std::ptr::null_mut();
        }
        match serde_json::from_str::<serde_json::Value>(&s) {
            Ok(serde_json::Value::Array(arr)) => {
                if index < 0 || (index as usize) >= arr.len() {
                    ffi_debug!(
                        "FFI",
                        "doo_json_array_get_element: index {} out of bounds (len={})",
                        index,
                        arr.len()
                    );
                    return std::ptr::null_mut();
                }
                let elem = &arr[index as usize];
                let elem_str = elem.to_string();
                ffi_debug!(
                    "FFI",
                    "doo_json_array_get_element[{}]: {}",
                    index,
                    &elem_str[..elem_str.len().min(100)]
                );
                doo_alloc_string(&elem_str)
            }
            _ => {
                ffi_debug!("FFI", "doo_json_array_get_element: not an array -> null");
                std::ptr::null_mut()
            }
        }
    }))
    .unwrap_or(std::ptr::null_mut())
}

// ============================================================================
// Type-Specific Array Parse Functions
// ============================================================================

/// Parse JSON array to [Int]
/// Layout: [Len: i64][Cap: i64][elements...]
/// Returns pointer to data section (after 16-byte header)
/// OWNERSHIP: Caller owns the returned array.
/// NEVER returns null - returns empty array on error
/// Sets parse error if any element has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_array_int(json_str: *const c_char) -> *mut i64 {
    ffi_debug!("FFI", "doo_json_parse_array_int: ENTER");

    if json_str.is_null() {
        ffi_debug!("FFI", "doo_json_parse_array_int: null input -> empty array");
        let result = alloc_empty_array() as *mut i64;
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_int: returning empty={:p}",
            result
        );
        return result;
    }

    let s = c_to_string_lossy(json_str);
    ffi_debug!("FFI", "doo_json_parse_array_int: input={:?}", s);

    if check_json_size(&s).is_err() {
        return alloc_empty_array() as *mut i64;
    }

    // Parse JSON and validate ALL elements are integers
    let arr = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Array(arr)) => arr,
        _ => {
            ffi_debug!("FFI", "doo_json_parse_array_int: not an array -> empty");
            return alloc_empty_array() as *mut i64;
        }
    };

    // Validate all elements and collect
    let mut elements = Vec::with_capacity(arr.len());
    for (i, elem) in arr.iter().enumerate() {
        match elem.as_i64() {
            Some(v) => elements.push(v),
            None => {
                // Type mismatch - set error and return empty array
                let received = json_value_type_name(elem);
                set_parse_error_rfc7807(&format!("[{}]", i), "Int", received);
                ffi_debug!(
                    "FFI",
                    "doo_json_parse_array_int: type mismatch at [{}], expected Int, got {}",
                    i,
                    received
                );
                return alloc_empty_array() as *mut i64;
            }
        }
    }

    ffi_debug!(
        "FFI",
        "doo_json_parse_array_int: parsed {} elements: {:?}",
        elements.len(),
        elements
    );

    let data_size = elements.len() * std::mem::size_of::<i64>();
    let total_size = 16 + data_size; // 16-byte header
    let alloc_size = total_size.max(MIN_ALLOCATION_SIZE);
    ffi_debug!(
        "FFI",
        "doo_json_parse_array_int: allocating {} bytes (data={}, total={}, min={})",
        alloc_size,
        data_size,
        total_size,
        MIN_ALLOCATION_SIZE
    );

    let ptr = doo_alloc(alloc_size);
    if ptr.is_null() {
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_int: alloc failed -> empty array"
        );
        return alloc_empty_array() as *mut i64;
    }
    ffi_debug!("FFI", "doo_json_parse_array_int: got header_ptr={:p}", ptr);

    unsafe {
        // Header: [len: i64][cap: i64]
        *(ptr as *mut i64) = elements.len() as i64; // Length at offset 0
        *(ptr.add(8) as *mut i64) = elements.len() as i64; // Capacity at offset 8

        let data_ptr = ptr.add(16) as *mut i64; // Data at offset 16
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_int: data_ptr={:p}, writing {} elements",
            data_ptr,
            elements.len()
        );

        for (i, val) in elements.iter().enumerate() {
            *data_ptr.add(i) = *val;
        }

        // Verify header is accessible
        let verify_len = *(data_ptr.offset(-2) as *const i64);
        let verify_cap = *(data_ptr.offset(-1) as *const i64);
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_int: RETURN data_ptr={:p}, verified len={}, cap={}",
            data_ptr,
            verify_len,
            verify_cap
        );

        data_ptr
    }
}

/// Parse JSON array to [Float]
/// OWNERSHIP: Caller owns the returned array.
/// NEVER returns null - returns empty array on error
/// Sets parse error if any element has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_array_float(json_str: *const c_char) -> *mut f64 {
    if json_str.is_null() {
        return alloc_empty_array() as *mut f64;
    }
    let s = c_to_string_lossy(json_str);
    if check_json_size(&s).is_err() {
        return alloc_empty_array() as *mut f64;
    }

    // Parse JSON and validate ALL elements are numbers
    let arr = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Array(arr)) => arr,
        _ => return alloc_empty_array() as *mut f64,
    };

    // Validate all elements and collect
    let mut elements = Vec::with_capacity(arr.len());
    for (i, elem) in arr.iter().enumerate() {
        match elem.as_f64() {
            Some(v) => elements.push(v),
            None => {
                // Type mismatch - set error and return empty array
                let received = json_value_type_name(elem);
                set_parse_error_rfc7807(&format!("[{}]", i), "Float", received);
                return alloc_empty_array() as *mut f64;
            }
        }
    }

    let data_size = elements.len() * std::mem::size_of::<f64>();
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_array() as *mut f64;
    }

    unsafe {
        // Header: [len: i64][cap: i64]
        *(ptr as *mut i64) = elements.len() as i64;
        *(ptr.add(8) as *mut i64) = elements.len() as i64;

        let data_ptr = ptr.add(16) as *mut f64;
        for (i, val) in elements.iter().enumerate() {
            *data_ptr.add(i) = *val;
        }
        data_ptr
    }
}

/// Parse JSON array to [Bool]
/// OWNERSHIP: Caller owns the returned array.
/// Sets parse error if any element has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_array_bool(json_str: *const c_char) -> *mut u8 {
    ffi_debug!("FFI", "doo_json_parse_array_bool: ENTER");

    if json_str.is_null() {
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_bool: null input -> empty array"
        );
        return alloc_empty_array() as *mut u8;
    }
    let s = c_to_string_lossy(json_str);
    ffi_debug!("FFI", "doo_json_parse_array_bool: input={:?}", s);

    if check_json_size(&s).is_err() {
        return alloc_empty_array() as *mut u8;
    }

    // Parse JSON and validate ALL elements are booleans
    let arr = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Array(arr)) => arr,
        _ => return alloc_empty_array() as *mut u8,
    };

    // Validate all elements and collect
    let mut elements = Vec::with_capacity(arr.len());
    for (i, elem) in arr.iter().enumerate() {
        match elem.as_bool() {
            Some(b) => elements.push(if b { 1u8 } else { 0u8 }),
            None => {
                // Type mismatch - set error and return empty array
                let received = json_value_type_name(elem);
                set_parse_error_rfc7807(&format!("[{}]", i), "Bool", received);
                ffi_debug!(
                    "FFI",
                    "doo_json_parse_array_bool: type mismatch at [{}], expected Bool, got {}",
                    i,
                    received
                );
                return alloc_empty_array() as *mut u8;
            }
        }
    }

    ffi_debug!(
        "FFI",
        "doo_json_parse_array_bool: parsed {} elements",
        elements.len()
    );

    // Doo uses i1 (1 byte) for bools in arrays
    let data_size = elements.len() * std::mem::size_of::<u8>();
    let total_size = 16 + data_size; // 16-byte header
    let alloc_size = total_size.max(MIN_ALLOCATION_SIZE);
    ffi_debug!(
        "FFI",
        "doo_json_parse_array_bool: allocating {} bytes",
        alloc_size
    );

    let ptr = doo_alloc(alloc_size);
    if ptr.is_null() {
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_bool: alloc failed -> empty array"
        );
        return alloc_empty_array() as *mut u8;
    }

    unsafe {
        // Header: [len: i64][cap: i64]
        *(ptr as *mut i64) = elements.len() as i64;
        *(ptr.add(8) as *mut i64) = elements.len() as i64;

        let data_ptr = ptr.add(16) as *mut u8;
        for (i, val) in elements.iter().enumerate() {
            *data_ptr.add(i) = *val;
        }
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_bool: RETURN data_ptr={:p}, len={}",
            data_ptr,
            elements.len()
        );
        data_ptr
    }
}

/// Parse JSON array to [Str]
/// Returns pointer to data section (after 16-byte header) containing raw C string pointers
/// OWNERSHIP: Caller owns the returned array and all strings in it.
/// Sets parse error if any element has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_array_str(json_str: *const c_char) -> *mut *mut c_char {
    ffi_debug!("FFI", "doo_json_parse_array_str: ENTER");

    if json_str.is_null() {
        ffi_debug!("FFI", "doo_json_parse_array_str: null input -> empty array");
        return alloc_empty_array() as *mut *mut c_char;
    }
    let s = c_to_string_lossy(json_str);
    ffi_debug!("FFI", "doo_json_parse_array_str: input={:?}", s);

    if check_json_size(&s).is_err() {
        return alloc_empty_array() as *mut *mut c_char;
    }

    // Parse JSON and validate ALL elements are strings
    let arr = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Array(arr)) => arr,
        _ => return alloc_empty_array() as *mut *mut c_char,
    };

    // Validate all elements and collect
    let mut elements = Vec::with_capacity(arr.len());
    for (i, elem) in arr.iter().enumerate() {
        match elem.as_str() {
            Some(s) => elements.push(s.to_string()),
            None => {
                // Type mismatch - set error and return empty array
                let received = json_value_type_name(elem);
                set_parse_error_rfc7807(&format!("[{}]", i), "Str", received);
                ffi_debug!(
                    "FFI",
                    "doo_json_parse_array_str: type mismatch at [{}], expected Str, got {}",
                    i,
                    received
                );
                return alloc_empty_array() as *mut *mut c_char;
            }
        }
    }

    ffi_debug!(
        "FFI",
        "doo_json_parse_array_str: parsed {} elements",
        elements.len()
    );

    let data_size = elements.len() * std::mem::size_of::<*mut c_char>();
    let total_size = 16 + data_size; // 16-byte header
    let alloc_size = total_size.max(MIN_ALLOCATION_SIZE);
    ffi_debug!(
        "FFI",
        "doo_json_parse_array_str: allocating {} bytes",
        alloc_size
    );

    let ptr = doo_alloc(alloc_size);
    if ptr.is_null() {
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_str: alloc failed -> empty array"
        );
        return alloc_empty_array() as *mut *mut c_char;
    }

    unsafe {
        // Header: [len: i64][cap: i64]
        *(ptr as *mut i64) = elements.len() as i64;
        *(ptr.add(8) as *mut i64) = elements.len() as i64;

        let data_ptr = ptr.add(16) as *mut *mut c_char;
        for (i, val) in elements.iter().enumerate() {
            // Use centralized string allocation - NOT CString::into_raw()!
            *data_ptr.add(i) = doo_alloc_string(val);
        }
        ffi_debug!(
            "FFI",
            "doo_json_parse_array_str: RETURN data_ptr={:p}, len={}",
            data_ptr,
            elements.len()
        );
        data_ptr
    }
}

// ============================================================================
// Map Parse Functions - {Str: *}
// ============================================================================

/// Parse JSON map {Str: Int}
/// Layout: [Len: i64][Cap: i64][entries...]
/// Returns pointer to data section (after 16-byte header)
/// OWNERSHIP: Caller owns the returned map and all keys.
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_str_int(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = c_to_string_lossy(json_str);
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    // Validate all values are integers
    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        match v.as_i64() {
            Some(i) => pairs.push((k.clone(), i)),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Int", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16; // ptr(8) + i64(8)
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len; // offset 0: length
        *(ptr.add(8) as *mut i64) = cap; // offset 8: capacity

        let data_ptr = ptr.add(16); // data starts after 16-byte header
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut *mut c_char) = doo_alloc_string(key);
            *(pair_ptr.add(8) as *mut i64) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Str: Float}
/// OWNERSHIP: Caller owns the returned map and all keys.
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_str_float(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = c_to_string_lossy(json_str);
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    // Validate all values are floats/numbers
    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        match v.as_f64() {
            Some(f) => pairs.push((k.clone(), f)),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Float", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut *mut c_char) = doo_alloc_string(key);
            *(pair_ptr.add(8) as *mut f64) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Str: Bool}
/// OWNERSHIP: Caller owns the returned map and all keys.
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_str_bool(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = c_to_string_lossy(json_str);
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        match v.as_bool() {
            Some(b) => pairs.push((k.clone(), if b { 1u8 } else { 0u8 })),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Bool", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    // Doo uses { ptr, i1 } which LLVM pads to 16 bytes
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut *mut c_char) = doo_alloc_string(key);
            // Store bool as u8 (i1) at offset 8
            *(pair_ptr.add(8) as *mut u8) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Str: Str}
/// OWNERSHIP: Caller owns the returned map and all keys/values.
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_str_str(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = c_to_string_lossy(json_str);
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        match v.as_str() {
            Some(s) => pairs.push((k.clone(), s.to_string())),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Str", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16; // ptr(8) + ptr(8)
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut *mut c_char) = doo_alloc_string(key);
            *(pair_ptr.add(8) as *mut *mut c_char) = doo_alloc_string(value);
        }
        data_ptr
    }
}

// ============================================================================
// Map Parse Functions - {Int: *}
// ============================================================================

/// Parse JSON map {Int: Int} - keys are stringified integers
/// OWNERSHIP: Caller owns the returned map.
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_int_int(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = c_to_string_lossy(json_str);
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let ki = match k.parse::<i64>() {
            Ok(ki) => ki,
            Err(_) => continue, // skip non-integer keys
        };
        match v.as_i64() {
            Some(vi) => pairs.push((ki, vi)),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Int", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16; // i64(8) + i64(8)
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut i64) = *key;
            *(pair_ptr.add(8) as *mut i64) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Int: Float}
/// OWNERSHIP: Caller owns the returned map.
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_int_float(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = c_to_string_lossy(json_str);
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let ki = match k.parse::<i64>() {
            Ok(ki) => ki,
            Err(_) => continue,
        };
        match v.as_f64() {
            Some(vi) => pairs.push((ki, vi)),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Float", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut i64) = *key;
            *(pair_ptr.add(8) as *mut f64) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Int: Bool}
/// OWNERSHIP: Caller owns the returned map.
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_int_bool(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = c_to_string_lossy(json_str);
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let ki = match k.parse::<i64>() {
            Ok(ki) => ki,
            Err(_) => continue,
        };
        match v.as_bool() {
            Some(b) => pairs.push((ki, if b { 1u8 } else { 0u8 })),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Bool", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    // Doo uses { i64, i1 } which LLVM pads to 16 bytes
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut i64) = *key;
            // Store bool as u8 (i1) at offset 8
            *(pair_ptr.add(8) as *mut u8) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Int: Str}
/// OWNERSHIP: Caller owns the returned map and all string values.
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_int_str(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = c_to_string_lossy(json_str);
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let ki = match k.parse::<i64>() {
            Ok(ki) => ki,
            Err(_) => continue,
        };
        match v.as_str() {
            Some(sv) => pairs.push((ki, sv.to_string())),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Str", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut i64) = *key;
            *(pair_ptr.add(8) as *mut *mut c_char) = doo_alloc_string(value);
        }
        data_ptr
    }
}

// ============================================================================
// Map Parse Functions - {Float: *}
// ============================================================================

/// Parse JSON map {Float: Int}
/// OWNERSHIP: Caller owns the returned map.
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_float_int(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = c_to_string_lossy(json_str);
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let ki = match k.parse::<f64>() {
            Ok(ki) => ki,
            Err(_) => continue,
        };
        match v.as_i64() {
            Some(vi) => pairs.push((ki, vi)),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Int", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut f64) = *key;
            *(pair_ptr.add(8) as *mut i64) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Float: Float}
/// OWNERSHIP: Caller owns the returned map.
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_float_float(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = c_to_string_lossy(json_str);
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let ki = match k.parse::<f64>() {
            Ok(ki) => ki,
            Err(_) => continue,
        };
        match v.as_f64() {
            Some(vi) => pairs.push((ki, vi)),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Float", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut f64) = *key;
            *(pair_ptr.add(8) as *mut f64) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Float: Bool}
/// OWNERSHIP: Caller owns the returned map.
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_float_bool(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = c_to_string_lossy(json_str);
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let ki = match k.parse::<f64>() {
            Ok(ki) => ki,
            Err(_) => continue,
        };
        match v.as_bool() {
            Some(b) => pairs.push((ki, if b { 1u8 } else { 0u8 })),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Bool", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    // Doo uses { f64, i1 } which LLVM pads to 16 bytes
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut f64) = *key;
            // Store bool as u8 (i1) at offset 8
            *(pair_ptr.add(8) as *mut u8) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Float: Str}
/// OWNERSHIP: Caller owns the returned map and all string values.
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_float_str(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = c_to_string_lossy(json_str);
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let ki = match k.parse::<f64>() {
            Ok(ki) => ki,
            Err(_) => continue,
        };
        match v.as_str() {
            Some(sv) => pairs.push((ki, sv.to_string())),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Str", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut f64) = *key;
            *(pair_ptr.add(8) as *mut *mut c_char) = doo_alloc_string(value);
        }
        data_ptr
    }
}

// ============================================================================
// Map Parse Functions - {Bool: *}
// ============================================================================

/// Parse JSON map {Bool: Int} - keys are "true" or "false"
/// OWNERSHIP: Caller owns the returned map.
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_bool_int(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = c_to_string_lossy(json_str);
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let kb = match k.as_str() {
            "true" => 1u8,
            "false" => 0u8,
            _ => continue,
        };
        match v.as_i64() {
            Some(vi) => pairs.push((kb, vi)),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Int", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    // Doo uses { i1, i64 } which LLVM pads to 16 bytes
    // Key (i1) at offset 0, value (i64) at offset 8
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            // Store bool key as u8 (i1) at offset 0
            *(pair_ptr as *mut u8) = *key;
            // Store i64 value at offset 8 (due to alignment)
            *(pair_ptr.add(8) as *mut i64) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Bool: Float}
/// OWNERSHIP: Caller owns the returned map.
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_bool_float(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = c_to_string_lossy(json_str);
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let kb = match k.as_str() {
            "true" => 1u8,
            "false" => 0u8,
            _ => continue,
        };
        match v.as_f64() {
            Some(vi) => pairs.push((kb, vi)),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Float", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    // Doo uses { i1, f64 } which LLVM pads to 16 bytes
    // Key (i1) at offset 0, value (f64) at offset 8
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            // Store bool key as u8 (i1) at offset 0
            *(pair_ptr as *mut u8) = *key;
            // Store f64 value at offset 8 (due to alignment)
            *(pair_ptr.add(8) as *mut f64) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Bool: Bool}
/// OWNERSHIP: Caller owns the returned map.
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_bool_bool(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = c_to_string_lossy(json_str);
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let kb = match k.as_str() {
            "true" => 1u8,
            "false" => 0u8,
            _ => continue,
        };
        match v.as_bool() {
            Some(b) => pairs.push((kb, if b { 1u8 } else { 0u8 })),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Bool", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    // Doo uses { i1, i1 } which is 2 bytes (1 + 1, no padding between i1s)
    let pair_size = 2;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            // Store bool key as u8 (i1)
            *(pair_ptr as *mut u8) = *key;
            // Store bool value as u8 (i1) at offset 1 (next byte)
            *(pair_ptr.add(1) as *mut u8) = *value;
        }
        data_ptr
    }
}

/// Parse JSON map {Bool: Str}
/// OWNERSHIP: Caller owns the returned map and all string values.
/// Sets parse error if any value has wrong type
#[no_mangle]
pub extern "C" fn doo_json_parse_map_bool_str(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return alloc_empty_map() as *mut u8;
    }
    let s = c_to_string_lossy(json_str);
    if check_json_size(&s).is_err() {
        return alloc_empty_map() as *mut u8;
    }

    let obj = match serde_json::from_str::<serde_json::Value>(&s) {
        Ok(serde_json::Value::Object(obj)) => obj,
        _ => return alloc_empty_map() as *mut u8,
    };

    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj.iter() {
        let kb = match k.as_str() {
            "true" => 1u8,
            "false" => 0u8,
            _ => continue,
        };
        match v.as_str() {
            Some(sv) => pairs.push((kb, sv.to_string())),
            None => {
                let received = json_value_type_name(v);
                set_parse_error_rfc7807(&format!(".{}", k), "Str", received);
                return alloc_empty_map() as *mut u8;
            }
        }
    }

    // Doo uses { i1, ptr } which LLVM pads to 16 bytes
    // Key (i1) at offset 0, value (ptr) at offset 8
    let pair_size = 16;
    let data_size = pairs.len() * pair_size;
    let total_size = 16 + data_size; // 16-byte header
    let ptr = doo_alloc(total_size.max(MIN_ALLOCATION_SIZE));
    if ptr.is_null() {
        return alloc_empty_map() as *mut u8;
    }

    unsafe {
        let len = pairs.len() as i64;
        let cap = pairs.len() as i64;
        *(ptr as *mut i64) = len;
        *(ptr.add(8) as *mut i64) = cap;

        let data_ptr = ptr.add(16);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            // Store bool key as u8 (i1) at offset 0
            *(pair_ptr as *mut u8) = *key;
            // Store ptr value at offset 8 (due to alignment)
            *(pair_ptr.add(8) as *mut *mut c_char) = doo_alloc_string(value);
        }
        data_ptr
    }
}

// ============================================================================
// Struct/Enum JSON Parse Helpers
// ============================================================================

/// Extract a field from a JSON object by name, returning it as a JSON string
/// This allows recursive parsing of complex types
/// OWNERSHIP: Caller owns the returned string.
/// Returns empty JSON string "{}" if field not found or invalid JSON (never returns null)
#[no_mangle]
pub extern "C" fn doo_json_get_field(
    json_str: *const c_char,
    field_name: *const c_char,
) -> *mut c_char {
    ffi_debug!(
        "JSON",
        "doo_json_get_field called, json_str={:?}, field_name={:?}",
        json_str,
        field_name
    );
    if json_str.is_null() || field_name.is_null() {
        ffi_debug!("JSON", "doo_json_get_field: null input");
        return doo_alloc_string("{}");
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = c_to_string_lossy(json_str);
        let field = c_to_string_lossy(field_name);
        if check_json_size(&s).is_err() {
            return doo_alloc_string("null");
        }
        ffi_debug!(
            "JSON",
            "doo_json_get_field: json='{}', field='{}'",
            s,
            field
        );

        let result = serde_json::from_str::<serde_json::Value>(&s)
            .ok()
            .and_then(|v| {
                v.as_object()
                    .and_then(|obj| json_object_get_field(obj, &field).cloned())
            })
            .and_then(|field_val| serde_json::to_string(&field_val).ok())
            .map(|s| doo_alloc_string(&s))
            .unwrap_or_else(|| doo_alloc_string("null"));
        ffi_debug!("JSON", "doo_json_get_field: returning {:?}", result);
        result
    }))
    .unwrap_or_else(|_| doo_alloc_string("null"))
}

/// Get enum variant name from JSON
/// JSON could be either:
/// - A string: "VariantName" (for unit variants)
/// - An object: {"VariantName": payload} (for variants with data)
/// Returns the variant name as a C string
/// OWNERSHIP: Caller owns the returned string.
/// Returns empty string if invalid JSON or not a variant (never returns null)
/// NOTE: Normalizes variant names to PascalCase for consistent matching
#[no_mangle]
pub extern "C" fn doo_json_get_variant_name(json_str: *const c_char) -> *mut c_char {
    if json_str.is_null() {
        return doo_alloc_string("");
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = c_to_string_lossy(json_str);
        serde_json::from_str::<serde_json::Value>(&s)
            .ok()
            .and_then(|v| match v {
                serde_json::Value::String(s) => Some(s),
                serde_json::Value::Object(obj) if obj.len() == 1 => obj.keys().next().cloned(),
                _ => None,
            })
            .map(|s| {
                let normalized = if !s.is_empty() {
                    let mut chars = s.chars();
                    match chars.next() {
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                        None => s,
                    }
                } else {
                    s
                };
                doo_alloc_string(&normalized)
            })
            .unwrap_or_else(|| doo_alloc_string(""))
    }))
    .unwrap_or_else(|_| doo_alloc_string(""))
}

/// Get enum variant payload from JSON
/// JSON should be: {"VariantName": payload}
/// Returns the payload as a JSON string for recursive parsing
/// OWNERSHIP: Caller owns the returned string.
/// Returns "null" JSON if invalid input (never returns null ptr)
#[no_mangle]
pub extern "C" fn doo_json_get_variant_payload(json_str: *const c_char) -> *mut c_char {
    if json_str.is_null() {
        return doo_alloc_string("null");
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = c_to_string_lossy(json_str);
        serde_json::from_str::<serde_json::Value>(&s)
            .ok()
            .and_then(|v| match v {
                serde_json::Value::Object(obj) if obj.len() == 1 => obj.values().next().cloned(),
                _ => None,
            })
            .and_then(|payload| serde_json::to_string(&payload).ok())
            .map(|s| doo_alloc_string(&s))
            .unwrap_or_else(|| doo_alloc_string("null"))
    }))
    .unwrap_or_else(|_| doo_alloc_string("null"))
}

/// Check if JSON represents a unit variant (plain string like "VariantName")
/// Returns 1 if unit variant, 0 otherwise
#[no_mangle]
pub extern "C" fn doo_json_is_unit_variant(json_str: *const c_char) -> i32 {
    if json_str.is_null() {
        return 0;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = c_to_string_lossy(json_str);
        serde_json::from_str::<serde_json::Value>(&s)
            .ok()
            .map(|v| if v.is_string() { 1 } else { 0 })
            .unwrap_or(0)
    }))
    .unwrap_or(0)
}

// ============================================================================
// Parse-Once / Get-Field-Many API (Performance Optimization)
// ============================================================================
// Problem: doo_json_get_field re-parses full JSON per field (N times for N fields).
// Solution: parse once, cache the Value, extract fields from the cached parse.
//
// Usage:
//   obj = doo_json_parse_object(json_str)   // parse ONCE
//   f1  = doo_json_object_get_field(obj, "name")  // no re-parse
//   f2  = doo_json_object_get_field(obj, "age")   // no re-parse
//   doo_json_object_free(obj)                // release cached parse

/// Parse a JSON string into an opaque object handle (parse once).
/// Returns an opaque pointer to a cached serde_json::Value.
/// OWNERSHIP: Caller must call doo_json_object_free when done.
/// Returns null if input is null or invalid JSON.
#[no_mangle]
pub extern "C" fn doo_json_parse_object(json_str: *const c_char) -> *mut std::ffi::c_void {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }
    catch_unwind(AssertUnwindSafe(|| {
        let s = c_to_string_lossy(json_str);
        if check_json_size(&s).is_err() {
            return std::ptr::null_mut();
        }
        match serde_json::from_str::<serde_json::Value>(&s) {
            Ok(v) => Box::into_raw(Box::new(v)) as *mut std::ffi::c_void,
            Err(_) => std::ptr::null_mut(),
        }
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Extract a field from a cached JSON parse result (no re-parse).
/// Returns the field value as a JSON string for type-specific parsing.
/// OWNERSHIP: Caller owns the returned string.
/// Returns "null" if field not found (never returns null ptr).
#[no_mangle]
pub extern "C" fn doo_json_object_get_field(
    obj: *mut std::ffi::c_void,
    field_name: *const c_char,
) -> *mut c_char {
    if obj.is_null() || field_name.is_null() {
        return doo_alloc_string("null");
    }
    catch_unwind(AssertUnwindSafe(|| {
        let value = unsafe { &*(obj as *const serde_json::Value) };
        let field = c_to_string_lossy(field_name);
        value
            .as_object()
            .and_then(|o| json_object_get_field(o, &field))
            .and_then(|v| serde_json::to_string(v).ok())
            .map(|s| doo_alloc_string(&s))
            .unwrap_or_else(|| doo_alloc_string("null"))
    }))
    .unwrap_or_else(|_| doo_alloc_string("null"))
}

/// Check if a cached JSON object has a specific field.
/// Returns 1 if field exists, 0 otherwise.
#[no_mangle]
pub extern "C" fn doo_json_object_has_field(
    obj: *mut std::ffi::c_void,
    field_name: *const c_char,
) -> i32 {
    if obj.is_null() || field_name.is_null() {
        return 0;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let value = unsafe { &*(obj as *const serde_json::Value) };
        let field = c_to_string_lossy(field_name);
        value
            .as_object()
            .map(|o| {
                if json_object_get_field(o, &field).is_some() {
                    1
                } else {
                    0
                }
            })
            .unwrap_or(0)
    }))
    .unwrap_or(0)
}

/// Free a cached JSON parse result.
/// OWNERSHIP: Takes ownership and drops the cached Value.
#[no_mangle]
pub extern "C" fn doo_json_object_free(obj: *mut std::ffi::c_void) {
    if !obj.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
            let _ = Box::from_raw(obj as *mut serde_json::Value);
        }));
    }
}

// ============================================================================
// Typed Field Extraction — Zero Re-Serialization
// ============================================================================
// These extract primitive values directly from the cached parse without
// converting back to JSON strings. Used by codegen for struct-from-JSON.

/// Case-insensitive JSON field lookup: try exact match first, then case-insensitive fallback.
/// Handles PascalCase struct fields matching snake_case JSON keys (e.g., "ExitCode" → "exit_code").
/// Normalizes by lowercasing AND stripping underscores so `exit_code` == `exitcode` == `ExitCode`.
fn json_object_get_field<'a>(
    obj: &'a serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Option<&'a serde_json::Value> {
    // Exact match first (fast path)
    if let Some(v) = obj.get(field) {
        return Some(v);
    }
    // Normalized fallback: lowercase + strip underscores/hyphens
    let normalized = field.to_lowercase().replace('_', "").replace('-', "");
    obj.iter()
        .find(|(k, _)| k.to_lowercase().replace('_', "").replace('-', "") == normalized)
        .map(|(_, v)| v)
}

/// Extract an integer field from a cached JSON object.
/// Returns the value directly (zero re-serialization). Returns 0 if missing/wrong type.
#[no_mangle]
pub extern "C" fn doo_json_object_get_int(
    obj: *mut std::ffi::c_void,
    field_name: *const c_char,
) -> i64 {
    if obj.is_null() || field_name.is_null() {
        return 0;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let value = unsafe { &*(obj as *const serde_json::Value) };
        let field = c_to_string_lossy(field_name);
        value
            .as_object()
            .and_then(|o| json_object_get_field(o, &field))
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
    }))
    .unwrap_or(0)
}

/// Extract a float field from a cached JSON object.
/// Returns the value directly. Returns 0.0 if missing/wrong type.
#[no_mangle]
pub extern "C" fn doo_json_object_get_float(
    obj: *mut std::ffi::c_void,
    field_name: *const c_char,
) -> f64 {
    if obj.is_null() || field_name.is_null() {
        return 0.0;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let value = unsafe { &*(obj as *const serde_json::Value) };
        let field = c_to_string_lossy(field_name);
        value
            .as_object()
            .and_then(|o| json_object_get_field(o, &field))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    }))
    .unwrap_or(0.0)
}

/// Extract a boolean field from a cached JSON object.
/// Returns 1 for true, 0 for false/missing/wrong type.
#[no_mangle]
pub extern "C" fn doo_json_object_get_bool(
    obj: *mut std::ffi::c_void,
    field_name: *const c_char,
) -> i32 {
    if obj.is_null() || field_name.is_null() {
        return 0;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let value = unsafe { &*(obj as *const serde_json::Value) };
        let field = c_to_string_lossy(field_name);
        value
            .as_object()
            .and_then(|o| json_object_get_field(o, &field))
            .and_then(|v| v.as_bool())
            .map(|b| if b { 1 } else { 0 })
            .unwrap_or(0)
    }))
    .unwrap_or(0)
}

/// Extract a string field from a cached JSON object.
/// Returns a newly allocated C string. Caller must free.
/// Returns empty string "" if missing/wrong type (never null).
#[no_mangle]
pub extern "C" fn doo_json_object_get_str(
    obj: *mut std::ffi::c_void,
    field_name: *const c_char,
) -> *mut c_char {
    if obj.is_null() || field_name.is_null() {
        return doo_alloc_string("");
    }
    catch_unwind(AssertUnwindSafe(|| {
        let value = unsafe { &*(obj as *const serde_json::Value) };
        let field = c_to_string_lossy(field_name);
        value
            .as_object()
            .and_then(|o| json_object_get_field(o, &field))
            .and_then(|v| v.as_str())
            .map(|s| doo_alloc_string(s))
            .unwrap_or_else(|| doo_alloc_string(""))
    }))
    .unwrap_or_else(|_| doo_alloc_string(""))
}

/// Extract a nested object/array field from a cached JSON object as a JSON string.
/// For composite types (structs, arrays, maps) that need further parsing.
/// Returns "null" if missing. Caller must free.
#[no_mangle]
pub extern "C" fn doo_json_object_get_json(
    obj: *mut std::ffi::c_void,
    field_name: *const c_char,
) -> *mut c_char {
    if obj.is_null() || field_name.is_null() {
        return doo_alloc_string("null");
    }
    catch_unwind(AssertUnwindSafe(|| {
        let value = unsafe { &*(obj as *const serde_json::Value) };
        let field = c_to_string_lossy(field_name);
        value
            .as_object()
            .and_then(|o| json_object_get_field(o, &field))
            .and_then(|v| serde_json::to_string(v).ok())
            .map(|s| doo_alloc_string(&s))
            .unwrap_or_else(|| doo_alloc_string("null"))
    }))
    .unwrap_or_else(|_| doo_alloc_string("null"))
}
