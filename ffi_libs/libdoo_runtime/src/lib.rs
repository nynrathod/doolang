//! Doo Runtime Library
//! Contains JSON parsing/stringify functions and other runtime helpers

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

// Use libc malloc for C-compatible allocations
extern "C" {
    fn malloc(size: usize) -> *mut u8;
}

// ===== HASH FUNCTIONS =====

/// Simple hash function for strings
/// Returns a hash value (i32) for use as map index
#[no_mangle]
pub extern "C" fn hash_string(str_ptr: *const c_char) -> i32 {
    if str_ptr.is_null() {
        return 0;
    }

    unsafe {
        let c_str = CStr::from_ptr(str_ptr);
        let bytes = c_str.to_bytes();

        // Simple DJB2 hash algorithm
        let mut hash: u32 = 5381;
        for byte in bytes {
            hash = hash.wrapping_mul(33).wrapping_add(*byte as u32);
        }

        // Return as positive i32
        (hash & 0x7FFFFFFF) as i32
    }
}

// ===== JSON BUILTIN FUNCTIONS =====

/// Parse JSON string and return a pointer to the parsed data
/// Returns a heap-allocated string containing JSON data (simplified for now)
/// Format: ptr points to malloc-compatible allocation for compatibility with free()
#[no_mangle]
pub extern "C" fn json_parse(json_str: *const c_char) -> *mut c_char {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        // Allocate with C malloc so it can be freed with C free()
        let len = rust_str.len();
        let ptr = malloc(len + 1) as *mut c_char;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        // Copy string content
        std::ptr::copy_nonoverlapping(rust_str.as_ptr(), ptr as *mut u8, len);
        // Null terminate
        *ptr.add(len) = 0;

        ptr
    }
}

/// Parse JSON array string into a heap-allocated array structure
/// Layout: [RC: 4 bytes][Length: 4 bytes][elements...]
/// Returns pointer to the data section (after header)
#[no_mangle]
pub extern "C" fn json_parse_array_int(json_str: *const c_char) -> *mut i32 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        // Parse JSON array: "[1, 2, 3]"
        let trimmed = rust_str.trim();
        if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
            // Return empty array
            let ptr = malloc(8) as *mut i32;
            if ptr.is_null() {
                return std::ptr::null_mut();
            }
            *ptr = 1; // RC
            *ptr.add(1) = 0; // Length
            return ptr.add(2); // Return data pointer
        }

        let inner = &trimmed[1..trimmed.len() - 1];
        let elements: Vec<i32> = if inner.trim().is_empty() {
            Vec::new()
        } else {
            inner
                .split(',')
                .filter_map(|s| s.trim().parse::<i32>().ok())
                .collect()
        };

        // Allocate: 8 bytes header + elements
        let data_size = elements.len() * std::mem::size_of::<i32>();
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(8)) as *mut i32;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *ptr = 1; // RC
        *ptr.add(1) = elements.len() as i32; // Length

        let data_ptr = ptr.add(2);
        for (i, val) in elements.iter().enumerate() {
            *data_ptr.add(i) = *val;
        }

        data_ptr
    }
}

#[no_mangle]
pub extern "C" fn json_parse_array_float(json_str: *const c_char) -> *mut f64 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let trimmed = rust_str.trim();
        if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
            let ptr = malloc(16);
            if ptr.is_null() {
                return std::ptr::null_mut();
            }
            *(ptr as *mut i32) = 1; // RC
            *(ptr.add(4) as *mut i32) = 0; // Length
            return ptr.add(8) as *mut f64;
        }

        let inner = &trimmed[1..trimmed.len() - 1];
        let elements: Vec<f64> = if inner.trim().is_empty() {
            Vec::new()
        } else {
            inner
                .split(',')
                .filter_map(|s| s.trim().parse::<f64>().ok())
                .collect()
        };

        // Allocate: 8 bytes header + elements (8 bytes each)
        let data_size = elements.len() * std::mem::size_of::<f64>();
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1; // RC
        *(ptr.add(4) as *mut i32) = elements.len() as i32; // Length

        let data_ptr = ptr.add(8) as *mut f64;
        for (i, val) in elements.iter().enumerate() {
            *data_ptr.add(i) = *val;
        }

        data_ptr
    }
}

#[no_mangle]
pub extern "C" fn json_parse_array_bool(json_str: *const c_char) -> *mut i32 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let trimmed = rust_str.trim();
        if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
            let ptr = malloc(8) as *mut i32;
            if ptr.is_null() {
                return std::ptr::null_mut();
            }
            *ptr = 1; // RC
            *ptr.add(1) = 0; // Length
            return ptr.add(2);
        }

        let inner = &trimmed[1..trimmed.len() - 1];
        let elements: Vec<bool> = if inner.trim().is_empty() {
            Vec::new()
        } else {
            inner
                .split(',')
                .map(|s| {
                    let t = s.trim().to_lowercase();
                    t == "true" || t == "1"
                })
                .collect()
        };

        // Bool elements are i32 (4 bytes each)
        let data_size = elements.len() * std::mem::size_of::<i32>();
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(8)) as *mut i32;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *ptr = 1; // RC
        *ptr.add(1) = elements.len() as i32; // Length

        let data_ptr = ptr.add(2);
        for (i, val) in elements.iter().enumerate() {
            *data_ptr.add(i) = if *val { 1 } else { 0 };
        }

        data_ptr
    }
}

#[no_mangle]
pub extern "C" fn json_parse_array_str(json_str: *const c_char) -> *mut *mut c_char {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let trimmed = rust_str.trim();
        if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
            let ptr = malloc(16);
            if ptr.is_null() {
                return std::ptr::null_mut();
            }
            *(ptr as *mut i32) = 1; // RC
            *(ptr.add(4) as *mut i32) = 0; // Length
            return ptr.add(8) as *mut *mut c_char;
        }

        let inner = &trimmed[1..trimmed.len() - 1];

        let elements: Vec<String> = if inner.trim().is_empty() {
            Vec::new()
        } else {
            parse_json_string_array(inner)
        };

        let data_size = elements.len() * std::mem::size_of::<*mut c_char>();
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1; // RC
        *(ptr.add(4) as *mut i32) = elements.len() as i32; // Length

        let data_ptr = ptr.add(8) as *mut *mut c_char;
        for (i, val) in elements.iter().enumerate() {
            // Allocate heap string with RC header for each string
            let str_bytes = val.as_bytes();
            let str_total = 8 + str_bytes.len() + 1;
            let str_ptr = malloc(str_total);
            if str_ptr.is_null() {
                continue;
            }

            *(str_ptr as *mut i32) = 1; // RC
            *(str_ptr.add(4) as *mut i32) = str_bytes.len() as i32; // Length

            let str_data = str_ptr.add(8);
            std::ptr::copy_nonoverlapping(str_bytes.as_ptr(), str_data, str_bytes.len());
            *str_data.add(str_bytes.len()) = 0; // Null terminator

            *data_ptr.add(i) = str_data as *mut c_char;
        }

        data_ptr
    }
}

/// Helper to parse JSON string array elements
fn parse_json_string_array(inner: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut escape_next = false;

    for ch in inner.chars() {
        if escape_next {
            current.push(ch);
            escape_next = false;
            continue;
        }

        match ch {
            '\\' if in_string => {
                escape_next = true;
            }
            '"' => {
                if in_string {
                    result.push(current.clone());
                    current.clear();
                }
                in_string = !in_string;
            }
            ',' if !in_string => {
                // Element separator, ignore
            }
            _ if in_string => {
                current.push(ch);
            }
            _ => {
                // Whitespace outside string, ignore
            }
        }
    }

    result
}

/// Parse JSON map with string keys to Int values
/// Layout: [RC: 4][Length: 4][{key_ptr, value}...]
#[no_mangle]
pub extern "C" fn json_parse_map_str_int(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let pairs = parse_json_map_str_int(rust_str);

        // Each pair: ptr (8 bytes) + i32 (4 bytes) + padding (4 bytes) = 16 bytes
        let pair_size = 16;
        let data_size = pairs.len() * pair_size;
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1; // RC
        *(ptr.add(4) as *mut i32) = pairs.len() as i32; // Length

        let data_ptr = ptr.add(8);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);

            // Allocate key string with RC header
            let key_bytes = key.as_bytes();
            let key_total = 8 + key_bytes.len() + 1;
            let key_ptr = malloc(key_total);
            if key_ptr.is_null() {
                continue;
            }

            *(key_ptr as *mut i32) = 1; // RC
            *(key_ptr.add(4) as *mut i32) = key_bytes.len() as i32;
            let key_data = key_ptr.add(8);
            std::ptr::copy_nonoverlapping(key_bytes.as_ptr(), key_data, key_bytes.len());
            *key_data.add(key_bytes.len()) = 0;

            // Store key pointer and value
            *(pair_ptr as *mut *mut u8) = key_data;
            *(pair_ptr.add(8) as *mut i32) = *value;
        }

        data_ptr
    }
}

fn parse_json_map_str_int(json: &str) -> Vec<(String, i32)> {
    let mut result = Vec::new();
    let trimmed = json.trim();

    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return result;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return result;
    }

    let mut key = String::new();
    let mut value_str = String::new();
    let mut in_key = false;
    let mut in_value = false;
    let mut escape_next = false;

    for ch in inner.chars() {
        if escape_next {
            if in_key {
                key.push(ch);
            }
            escape_next = false;
            continue;
        }

        match ch {
            '\\' => escape_next = true,
            '"' => {
                if !in_key && !in_value {
                    in_key = true;
                } else if in_key {
                    in_key = false;
                }
            }
            ':' if !in_key => {
                in_value = true;
            }
            ',' if !in_key => {
                if let Ok(v) = value_str.trim().parse::<i32>() {
                    result.push((key.clone(), v));
                }
                key.clear();
                value_str.clear();
                in_value = false;
            }
            _ if in_key => key.push(ch),
            _ if in_value => value_str.push(ch),
            _ => {}
        }
    }

    // Don't forget the last pair
    if !key.is_empty() {
        if let Ok(v) = value_str.trim().parse::<i32>() {
            result.push((key, v));
        }
    }

    result
}

/// Parse JSON map with string keys to Float values
#[no_mangle]
pub extern "C" fn json_parse_map_str_float(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let pairs = parse_json_map_str_float(rust_str);

        let pair_size = 16;
        let data_size = pairs.len() * pair_size;
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1;
        *(ptr.add(4) as *mut i32) = pairs.len() as i32;

        let data_ptr = ptr.add(8);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);

            let key_bytes = key.as_bytes();
            let key_total = 8 + key_bytes.len() + 1;
            let key_ptr = malloc(key_total);
            if key_ptr.is_null() {
                continue;
            }

            *(key_ptr as *mut i32) = 1;
            *(key_ptr.add(4) as *mut i32) = key_bytes.len() as i32;
            let key_data = key_ptr.add(8);
            std::ptr::copy_nonoverlapping(key_bytes.as_ptr(), key_data, key_bytes.len());
            *key_data.add(key_bytes.len()) = 0;

            *(pair_ptr as *mut *mut u8) = key_data;
            *(pair_ptr.add(8) as *mut f64) = *value;
        }

        data_ptr
    }
}

fn parse_json_map_str_float(json: &str) -> Vec<(String, f64)> {
    let mut result = Vec::new();
    let trimmed = json.trim();

    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return result;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return result;
    }

    let mut key = String::new();
    let mut value_str = String::new();
    let mut in_key = false;
    let mut in_value = false;
    let mut escape_next = false;

    for ch in inner.chars() {
        if escape_next {
            if in_key {
                key.push(ch);
            }
            escape_next = false;
            continue;
        }

        match ch {
            '\\' => escape_next = true,
            '"' => {
                if !in_key && !in_value {
                    in_key = true;
                } else if in_key {
                    in_key = false;
                }
            }
            ':' if !in_key => in_value = true,
            ',' if !in_key => {
                if let Ok(v) = value_str.trim().parse::<f64>() {
                    result.push((key.clone(), v));
                }
                key.clear();
                value_str.clear();
                in_value = false;
            }
            _ if in_key => key.push(ch),
            _ if in_value => value_str.push(ch),
            _ => {}
        }
    }

    if !key.is_empty() {
        if let Ok(v) = value_str.trim().parse::<f64>() {
            result.push((key, v));
        }
    }

    result
}

/// Parse JSON map with string keys to Bool values
#[no_mangle]
pub extern "C" fn json_parse_map_str_bool(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let pairs = parse_json_map_str_bool(rust_str);

        let pair_size = 16;
        let data_size = pairs.len() * pair_size;
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1;
        *(ptr.add(4) as *mut i32) = pairs.len() as i32;

        let data_ptr = ptr.add(8);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);

            let key_bytes = key.as_bytes();
            let key_total = 8 + key_bytes.len() + 1;
            let key_ptr = malloc(key_total);
            if key_ptr.is_null() {
                continue;
            }

            *(key_ptr as *mut i32) = 1;
            *(key_ptr.add(4) as *mut i32) = key_bytes.len() as i32;
            let key_data = key_ptr.add(8);
            std::ptr::copy_nonoverlapping(key_bytes.as_ptr(), key_data, key_bytes.len());
            *key_data.add(key_bytes.len()) = 0;

            *(pair_ptr as *mut *mut u8) = key_data;
            *(pair_ptr.add(8) as *mut i32) = if *value { 1 } else { 0 };
        }

        data_ptr
    }
}

fn parse_json_map_str_bool(json: &str) -> Vec<(String, bool)> {
    let mut result = Vec::new();
    let trimmed = json.trim();

    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return result;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return result;
    }

    let mut key = String::new();
    let mut value_str = String::new();
    let mut in_key = false;
    let mut in_value = false;
    let mut escape_next = false;

    for ch in inner.chars() {
        if escape_next {
            if in_key {
                key.push(ch);
            }
            escape_next = false;
            continue;
        }

        match ch {
            '\\' => escape_next = true,
            '"' => {
                if !in_key && !in_value {
                    in_key = true;
                } else if in_key {
                    in_key = false;
                }
            }
            ':' if !in_key => in_value = true,
            ',' if !in_key => {
                let v = value_str.trim().to_lowercase();
                result.push((key.clone(), v == "true" || v == "1"));
                key.clear();
                value_str.clear();
                in_value = false;
            }
            _ if in_key => key.push(ch),
            _ if in_value => value_str.push(ch),
            _ => {}
        }
    }

    if !key.is_empty() {
        let v = value_str.trim().to_lowercase();
        result.push((key, v == "true" || v == "1"));
    }

    result
}

/// Parse JSON map with string keys to Str values
#[no_mangle]
pub extern "C" fn json_parse_map_str_str(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let pairs = parse_json_map_str_str(rust_str);

        let pair_size = 16;
        let data_size = pairs.len() * pair_size;
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1;
        *(ptr.add(4) as *mut i32) = pairs.len() as i32;

        let data_ptr = ptr.add(8);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);

            // Allocate key
            let key_bytes = key.as_bytes();
            let key_total = 8 + key_bytes.len() + 1;
            let key_ptr = malloc(key_total);
            if key_ptr.is_null() {
                continue;
            }
            *(key_ptr as *mut i32) = 1;
            *(key_ptr.add(4) as *mut i32) = key_bytes.len() as i32;
            let key_data = key_ptr.add(8);
            std::ptr::copy_nonoverlapping(key_bytes.as_ptr(), key_data, key_bytes.len());
            *key_data.add(key_bytes.len()) = 0;

            // Allocate value
            let val_bytes = value.as_bytes();
            let val_total = 8 + val_bytes.len() + 1;
            let val_ptr = malloc(val_total);
            if val_ptr.is_null() {
                continue;
            }
            *(val_ptr as *mut i32) = 1;
            *(val_ptr.add(4) as *mut i32) = val_bytes.len() as i32;
            let val_data = val_ptr.add(8);
            std::ptr::copy_nonoverlapping(val_bytes.as_ptr(), val_data, val_bytes.len());
            *val_data.add(val_bytes.len()) = 0;

            *(pair_ptr as *mut *mut u8) = key_data;
            *(pair_ptr.add(8) as *mut *mut u8) = val_data;
        }

        data_ptr
    }
}

fn parse_json_map_str_str(json: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let trimmed = json.trim();

    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return result;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return result;
    }

    let mut key = String::new();
    let mut value = String::new();
    let mut in_key = false;
    let mut in_value = false;
    let mut escape_next = false;
    let mut after_colon = false;

    for ch in inner.chars() {
        if escape_next {
            if in_key {
                key.push(ch);
            } else if in_value {
                value.push(ch);
            }
            escape_next = false;
            continue;
        }

        match ch {
            '\\' => escape_next = true,
            '"' => {
                if !in_key && !in_value && !after_colon {
                    in_key = true;
                } else if in_key {
                    in_key = false;
                } else if !in_value && after_colon {
                    in_value = true;
                } else if in_value {
                    in_value = false;
                }
            }
            ':' if !in_key && !in_value => {
                after_colon = true;
            }
            ',' if !in_key && !in_value => {
                result.push((key.clone(), value.clone()));
                key.clear();
                value.clear();
                after_colon = false;
            }
            _ if in_key => key.push(ch),
            _ if in_value => value.push(ch),
            _ => {}
        }
    }

    if !key.is_empty() {
        result.push((key, value));
    }

    result
}

/// Parse JSON map with Int keys
#[no_mangle]
pub extern "C" fn json_parse_map_int_int(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let pairs = parse_json_map_int_int(rust_str);

        let pair_size = 8;
        let data_size = pairs.len() * pair_size;
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1;
        *(ptr.add(4) as *mut i32) = pairs.len() as i32;

        let data_ptr = ptr.add(8);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut i32) = *key;
            *(pair_ptr.add(4) as *mut i32) = *value;
        }

        data_ptr
    }
}

fn parse_json_map_int_int(json: &str) -> Vec<(i32, i32)> {
    let mut result = Vec::new();
    let trimmed = json.trim();

    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return result;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return result;
    }

    let mut key_str = String::new();
    let mut value_str = String::new();
    let mut in_key = false;
    let mut in_value = false;

    for ch in inner.chars() {
        match ch {
            '"' => {
                if !in_key && !in_value {
                    in_key = true;
                } else if in_key {
                    in_key = false;
                }
            }
            ':' if !in_key => in_value = true,
            ',' if !in_key => {
                if let (Ok(k), Ok(v)) = (
                    key_str.trim().parse::<i32>(),
                    value_str.trim().parse::<i32>(),
                ) {
                    result.push((k, v));
                }
                key_str.clear();
                value_str.clear();
                in_value = false;
            }
            _ if in_key => key_str.push(ch),
            _ if in_value && ch != ' ' => value_str.push(ch),
            _ => {}
        }
    }

    if !key_str.is_empty() {
        if let (Ok(k), Ok(v)) = (
            key_str.trim().parse::<i32>(),
            value_str.trim().parse::<i32>(),
        ) {
            result.push((k, v));
        }
    }

    result
}

#[no_mangle]
pub extern "C" fn json_parse_map_int_float(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let pairs = parse_json_map_int_float(rust_str);

        let pair_size = 16;
        let data_size = pairs.len() * pair_size;
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1;
        *(ptr.add(4) as *mut i32) = pairs.len() as i32;

        let data_ptr = ptr.add(8);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut i32) = *key;
            *(pair_ptr.add(8) as *mut f64) = *value;
        }

        data_ptr
    }
}

fn parse_json_map_int_float(json: &str) -> Vec<(i32, f64)> {
    let mut result = Vec::new();
    let trimmed = json.trim();

    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return result;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return result;
    }

    let mut key_str = String::new();
    let mut value_str = String::new();
    let mut in_key = false;
    let mut in_value = false;

    for ch in inner.chars() {
        match ch {
            '"' => {
                if !in_key && !in_value {
                    in_key = true;
                } else if in_key {
                    in_key = false;
                }
            }
            ':' if !in_key => in_value = true,
            ',' if !in_key => {
                if let (Ok(k), Ok(v)) = (
                    key_str.trim().parse::<i32>(),
                    value_str.trim().parse::<f64>(),
                ) {
                    result.push((k, v));
                }
                key_str.clear();
                value_str.clear();
                in_value = false;
            }
            _ if in_key => key_str.push(ch),
            _ if in_value && ch != ' ' => value_str.push(ch),
            _ => {}
        }
    }

    if !key_str.is_empty() {
        if let (Ok(k), Ok(v)) = (
            key_str.trim().parse::<i32>(),
            value_str.trim().parse::<f64>(),
        ) {
            result.push((k, v));
        }
    }

    result
}

#[no_mangle]
pub extern "C" fn json_parse_map_int_bool(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let pairs = parse_json_map_int_bool(rust_str);

        let pair_size = 8;
        let data_size = pairs.len() * pair_size;
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1;
        *(ptr.add(4) as *mut i32) = pairs.len() as i32;

        let data_ptr = ptr.add(8);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut i32) = *key;
            *(pair_ptr.add(4) as *mut i32) = if *value { 1 } else { 0 };
        }

        data_ptr
    }
}

fn parse_json_map_int_bool(json: &str) -> Vec<(i32, bool)> {
    let mut result = Vec::new();
    let trimmed = json.trim();

    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return result;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return result;
    }

    let mut key_str = String::new();
    let mut value_str = String::new();
    let mut in_key = false;
    let mut in_value = false;

    for ch in inner.chars() {
        match ch {
            '"' => {
                if !in_key && !in_value {
                    in_key = true;
                } else if in_key {
                    in_key = false;
                }
            }
            ':' if !in_key => in_value = true,
            ',' if !in_key => {
                if let Ok(k) = key_str.trim().parse::<i32>() {
                    let v = value_str.trim().to_lowercase();
                    result.push((k, v == "true" || v == "1"));
                }
                key_str.clear();
                value_str.clear();
                in_value = false;
            }
            _ if in_key => key_str.push(ch),
            _ if in_value && ch != ' ' => value_str.push(ch),
            _ => {}
        }
    }

    if !key_str.is_empty() {
        if let Ok(k) = key_str.trim().parse::<i32>() {
            let v = value_str.trim().to_lowercase();
            result.push((k, v == "true" || v == "1"));
        }
    }

    result
}

#[no_mangle]
pub extern "C" fn json_parse_map_int_str(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let pairs = parse_json_map_int_str(rust_str);

        let pair_size = 16;
        let data_size = pairs.len() * pair_size;
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1;
        *(ptr.add(4) as *mut i32) = pairs.len() as i32;

        let data_ptr = ptr.add(8);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);

            let val_bytes = value.as_bytes();
            let val_total = 8 + val_bytes.len() + 1;
            let val_ptr = malloc(val_total);
            if val_ptr.is_null() {
                continue;
            }
            *(val_ptr as *mut i32) = 1;
            *(val_ptr.add(4) as *mut i32) = val_bytes.len() as i32;
            let val_data = val_ptr.add(8);
            std::ptr::copy_nonoverlapping(val_bytes.as_ptr(), val_data, val_bytes.len());
            *val_data.add(val_bytes.len()) = 0;

            *(pair_ptr as *mut i32) = *key;
            *(pair_ptr.add(8) as *mut *mut u8) = val_data;
        }

        data_ptr
    }
}

fn parse_json_map_int_str(json: &str) -> Vec<(i32, String)> {
    let mut result = Vec::new();
    let trimmed = json.trim();

    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return result;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return result;
    }

    let mut key_str = String::new();
    let mut value = String::new();
    let mut in_key = false;
    let mut in_value = false;
    let mut after_colon = false;
    let mut escape_next = false;

    for ch in inner.chars() {
        if escape_next {
            if in_value {
                value.push(ch);
            }
            escape_next = false;
            continue;
        }

        match ch {
            '\\' => escape_next = true,
            '"' => {
                if !in_key && !in_value && !after_colon {
                    in_key = true;
                } else if in_key {
                    in_key = false;
                } else if !in_value && after_colon {
                    in_value = true;
                } else if in_value {
                    in_value = false;
                }
            }
            ':' if !in_key && !in_value => after_colon = true,
            ',' if !in_key && !in_value => {
                if let Ok(k) = key_str.trim().parse::<i32>() {
                    result.push((k, value.clone()));
                }
                key_str.clear();
                value.clear();
                after_colon = false;
            }
            _ if in_key => key_str.push(ch),
            _ if in_value => value.push(ch),
            _ => {}
        }
    }

    if !key_str.is_empty() {
        if let Ok(k) = key_str.trim().parse::<i32>() {
            result.push((k, value));
        }
    }

    result
}

// Float key maps
#[no_mangle]
pub extern "C" fn json_parse_map_float_int(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let pairs = parse_json_map_float_int(rust_str);

        let pair_size = 16;
        let data_size = pairs.len() * pair_size;
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1;
        *(ptr.add(4) as *mut i32) = pairs.len() as i32;

        let data_ptr = ptr.add(8);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut f64) = *key;
            *(pair_ptr.add(8) as *mut i32) = *value;
        }

        data_ptr
    }
}

fn parse_json_map_float_int(json: &str) -> Vec<(f64, i32)> {
    let mut result = Vec::new();
    let trimmed = json.trim();

    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return result;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return result;
    }

    let mut key_str = String::new();
    let mut value_str = String::new();
    let mut in_key = false;
    let mut in_value = false;

    for ch in inner.chars() {
        match ch {
            '"' => {
                if !in_key && !in_value {
                    in_key = true;
                } else if in_key {
                    in_key = false;
                }
            }
            ':' if !in_key => in_value = true,
            ',' if !in_key => {
                if let (Ok(k), Ok(v)) = (
                    key_str.trim().parse::<f64>(),
                    value_str.trim().parse::<i32>(),
                ) {
                    result.push((k, v));
                }
                key_str.clear();
                value_str.clear();
                in_value = false;
            }
            _ if in_key => key_str.push(ch),
            _ if in_value && ch != ' ' => value_str.push(ch),
            _ => {}
        }
    }

    if !key_str.is_empty() {
        if let (Ok(k), Ok(v)) = (
            key_str.trim().parse::<f64>(),
            value_str.trim().parse::<i32>(),
        ) {
            result.push((k, v));
        }
    }

    result
}

#[no_mangle]
pub extern "C" fn json_parse_map_float_float(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let pairs = parse_json_map_float_float(rust_str);

        let pair_size = 16;
        let data_size = pairs.len() * pair_size;
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1;
        *(ptr.add(4) as *mut i32) = pairs.len() as i32;

        let data_ptr = ptr.add(8);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut f64) = *key;
            *(pair_ptr.add(8) as *mut f64) = *value;
        }

        data_ptr
    }
}

fn parse_json_map_float_float(json: &str) -> Vec<(f64, f64)> {
    let mut result = Vec::new();
    let trimmed = json.trim();

    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return result;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return result;
    }

    let mut key_str = String::new();
    let mut value_str = String::new();
    let mut in_key = false;
    let mut in_value = false;

    for ch in inner.chars() {
        match ch {
            '"' => {
                if !in_key && !in_value {
                    in_key = true;
                } else if in_key {
                    in_key = false;
                }
            }
            ':' if !in_key => in_value = true,
            ',' if !in_key => {
                if let (Ok(k), Ok(v)) = (
                    key_str.trim().parse::<f64>(),
                    value_str.trim().parse::<f64>(),
                ) {
                    result.push((k, v));
                }
                key_str.clear();
                value_str.clear();
                in_value = false;
            }
            _ if in_key => key_str.push(ch),
            _ if in_value && ch != ' ' => value_str.push(ch),
            _ => {}
        }
    }

    if !key_str.is_empty() {
        if let (Ok(k), Ok(v)) = (
            key_str.trim().parse::<f64>(),
            value_str.trim().parse::<f64>(),
        ) {
            result.push((k, v));
        }
    }

    result
}

#[no_mangle]
pub extern "C" fn json_parse_map_float_bool(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let pairs = parse_json_map_float_bool(rust_str);

        let pair_size = 16;
        let data_size = pairs.len() * pair_size;
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1;
        *(ptr.add(4) as *mut i32) = pairs.len() as i32;

        let data_ptr = ptr.add(8);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut f64) = *key;
            *(pair_ptr.add(8) as *mut i32) = if *value { 1 } else { 0 };
        }

        data_ptr
    }
}

fn parse_json_map_float_bool(json: &str) -> Vec<(f64, bool)> {
    let mut result = Vec::new();
    let trimmed = json.trim();

    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return result;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return result;
    }

    let mut key_str = String::new();
    let mut value_str = String::new();
    let mut in_key = false;
    let mut in_value = false;

    for ch in inner.chars() {
        match ch {
            '"' => {
                if !in_key && !in_value {
                    in_key = true;
                } else if in_key {
                    in_key = false;
                }
            }
            ':' if !in_key => in_value = true,
            ',' if !in_key => {
                if let Ok(k) = key_str.trim().parse::<f64>() {
                    let v = value_str.trim().to_lowercase();
                    result.push((k, v == "true" || v == "1"));
                }
                key_str.clear();
                value_str.clear();
                in_value = false;
            }
            _ if in_key => key_str.push(ch),
            _ if in_value && ch != ' ' => value_str.push(ch),
            _ => {}
        }
    }

    if !key_str.is_empty() {
        if let Ok(k) = key_str.trim().parse::<f64>() {
            let v = value_str.trim().to_lowercase();
            result.push((k, v == "true" || v == "1"));
        }
    }

    result
}

#[no_mangle]
pub extern "C" fn json_parse_map_float_str(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let pairs = parse_json_map_float_str(rust_str);

        let pair_size = 16;
        let data_size = pairs.len() * pair_size;
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1;
        *(ptr.add(4) as *mut i32) = pairs.len() as i32;

        let data_ptr = ptr.add(8);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);

            let val_bytes = value.as_bytes();
            let val_total = 8 + val_bytes.len() + 1;
            let val_ptr = malloc(val_total);
            if val_ptr.is_null() {
                continue;
            }
            *(val_ptr as *mut i32) = 1;
            *(val_ptr.add(4) as *mut i32) = val_bytes.len() as i32;
            let val_data = val_ptr.add(8);
            std::ptr::copy_nonoverlapping(val_bytes.as_ptr(), val_data, val_bytes.len());
            *val_data.add(val_bytes.len()) = 0;

            *(pair_ptr as *mut f64) = *key;
            *(pair_ptr.add(8) as *mut *mut u8) = val_data;
        }

        data_ptr
    }
}

fn parse_json_map_float_str(json: &str) -> Vec<(f64, String)> {
    let mut result = Vec::new();
    let trimmed = json.trim();

    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return result;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return result;
    }

    let mut key_str = String::new();
    let mut value = String::new();
    let mut in_key = false;
    let mut in_value = false;
    let mut after_colon = false;
    let mut escape_next = false;

    for ch in inner.chars() {
        if escape_next {
            if in_value {
                value.push(ch);
            }
            escape_next = false;
            continue;
        }

        match ch {
            '\\' => escape_next = true,
            '"' => {
                if !in_key && !in_value && !after_colon {
                    in_key = true;
                } else if in_key {
                    in_key = false;
                } else if !in_value && after_colon {
                    in_value = true;
                } else if in_value {
                    in_value = false;
                }
            }
            ':' if !in_key && !in_value => after_colon = true,
            ',' if !in_key && !in_value => {
                if let Ok(k) = key_str.trim().parse::<f64>() {
                    result.push((k, value.clone()));
                }
                key_str.clear();
                value.clear();
                after_colon = false;
            }
            _ if in_key => key_str.push(ch),
            _ if in_value => value.push(ch),
            _ => {}
        }
    }

    if !key_str.is_empty() {
        if let Ok(k) = key_str.trim().parse::<f64>() {
            result.push((k, value));
        }
    }

    result
}

// Bool key maps
#[no_mangle]
pub extern "C" fn json_parse_map_bool_int(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let pairs = parse_json_map_bool_int(rust_str);

        let pair_size = 8;
        let data_size = pairs.len() * pair_size;
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1;
        *(ptr.add(4) as *mut i32) = pairs.len() as i32;

        let data_ptr = ptr.add(8);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut i32) = if *key { 1 } else { 0 };
            *(pair_ptr.add(4) as *mut i32) = *value;
        }

        data_ptr
    }
}

fn parse_json_map_bool_int(json: &str) -> Vec<(bool, i32)> {
    let mut result = Vec::new();
    let trimmed = json.trim();

    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return result;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return result;
    }

    let mut key_str = String::new();
    let mut value_str = String::new();
    let mut in_key = false;
    let mut in_value = false;

    for ch in inner.chars() {
        match ch {
            '"' => {
                if !in_key && !in_value {
                    in_key = true;
                } else if in_key {
                    in_key = false;
                }
            }
            ':' if !in_key => in_value = true,
            ',' if !in_key => {
                let k = key_str.trim().to_lowercase();
                if let Ok(v) = value_str.trim().parse::<i32>() {
                    result.push((k == "true" || k == "1", v));
                }
                key_str.clear();
                value_str.clear();
                in_value = false;
            }
            _ if in_key => key_str.push(ch),
            _ if in_value && ch != ' ' => value_str.push(ch),
            _ => {}
        }
    }

    if !key_str.is_empty() {
        let k = key_str.trim().to_lowercase();
        if let Ok(v) = value_str.trim().parse::<i32>() {
            result.push((k == "true" || k == "1", v));
        }
    }

    result
}

#[no_mangle]
pub extern "C" fn json_parse_map_bool_float(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let pairs = parse_json_map_bool_float(rust_str);

        let pair_size = 16;
        let data_size = pairs.len() * pair_size;
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1;
        *(ptr.add(4) as *mut i32) = pairs.len() as i32;

        let data_ptr = ptr.add(8);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut i32) = if *key { 1 } else { 0 };
            *(pair_ptr.add(8) as *mut f64) = *value;
        }

        data_ptr
    }
}

fn parse_json_map_bool_float(json: &str) -> Vec<(bool, f64)> {
    let mut result = Vec::new();
    let trimmed = json.trim();

    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return result;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return result;
    }

    let mut key_str = String::new();
    let mut value_str = String::new();
    let mut in_key = false;
    let mut in_value = false;

    for ch in inner.chars() {
        match ch {
            '"' => {
                if !in_key && !in_value {
                    in_key = true;
                } else if in_key {
                    in_key = false;
                }
            }
            ':' if !in_key => in_value = true,
            ',' if !in_key => {
                let k = key_str.trim().to_lowercase();
                if let Ok(v) = value_str.trim().parse::<f64>() {
                    result.push((k == "true" || k == "1", v));
                }
                key_str.clear();
                value_str.clear();
                in_value = false;
            }
            _ if in_key => key_str.push(ch),
            _ if in_value && ch != ' ' => value_str.push(ch),
            _ => {}
        }
    }

    if !key_str.is_empty() {
        let k = key_str.trim().to_lowercase();
        if let Ok(v) = value_str.trim().parse::<f64>() {
            result.push((k == "true" || k == "1", v));
        }
    }

    result
}

#[no_mangle]
pub extern "C" fn json_parse_map_bool_bool(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let pairs = parse_json_map_bool_bool(rust_str);

        let pair_size = 8;
        let data_size = pairs.len() * pair_size;
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1;
        *(ptr.add(4) as *mut i32) = pairs.len() as i32;

        let data_ptr = ptr.add(8);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);
            *(pair_ptr as *mut i32) = if *key { 1 } else { 0 };
            *(pair_ptr.add(4) as *mut i32) = if *value { 1 } else { 0 };
        }

        data_ptr
    }
}

fn parse_json_map_bool_bool(json: &str) -> Vec<(bool, bool)> {
    let mut result = Vec::new();
    let trimmed = json.trim();

    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return result;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return result;
    }

    let mut key_str = String::new();
    let mut value_str = String::new();
    let mut in_key = false;
    let mut in_value = false;

    for ch in inner.chars() {
        match ch {
            '"' => {
                if !in_key && !in_value {
                    in_key = true;
                } else if in_key {
                    in_key = false;
                }
            }
            ':' if !in_key => in_value = true,
            ',' if !in_key => {
                let k = key_str.trim().to_lowercase();
                let v = value_str.trim().to_lowercase();
                result.push((k == "true" || k == "1", v == "true" || v == "1"));
                key_str.clear();
                value_str.clear();
                in_value = false;
            }
            _ if in_key => key_str.push(ch),
            _ if in_value && ch != ' ' => value_str.push(ch),
            _ => {}
        }
    }

    if !key_str.is_empty() {
        let k = key_str.trim().to_lowercase();
        let v = value_str.trim().to_lowercase();
        result.push((k == "true" || k == "1", v == "true" || v == "1"));
    }

    result
}

#[no_mangle]
pub extern "C" fn json_parse_map_bool_str(json_str: *const c_char) -> *mut u8 {
    if json_str.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(json_str);
        let rust_str = match c_str.to_str() {
            Ok(s) => s.trim(),
            Err(_) => return std::ptr::null_mut(),
        };

        let pairs = parse_json_map_bool_str(rust_str);

        let pair_size = 16;
        let data_size = pairs.len() * pair_size;
        let total_size = 8 + data_size;
        let ptr = malloc(total_size.max(16));
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        *(ptr as *mut i32) = 1;
        *(ptr.add(4) as *mut i32) = pairs.len() as i32;

        let data_ptr = ptr.add(8);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let pair_ptr = data_ptr.add(i * pair_size);

            let val_bytes = value.as_bytes();
            let val_total = 8 + val_bytes.len() + 1;
            let val_ptr = malloc(val_total);
            if val_ptr.is_null() {
                continue;
            }
            *(val_ptr as *mut i32) = 1;
            *(val_ptr.add(4) as *mut i32) = val_bytes.len() as i32;
            let val_data = val_ptr.add(8);
            std::ptr::copy_nonoverlapping(val_bytes.as_ptr(), val_data, val_bytes.len());
            *val_data.add(val_bytes.len()) = 0;

            *(pair_ptr as *mut i32) = if *key { 1 } else { 0 };
            *(pair_ptr.add(8) as *mut *mut u8) = val_data;
        }

        data_ptr
    }
}

fn parse_json_map_bool_str(json: &str) -> Vec<(bool, String)> {
    let mut result = Vec::new();
    let trimmed = json.trim();

    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return result;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return result;
    }

    let mut key_str = String::new();
    let mut value = String::new();
    let mut in_key = false;
    let mut in_value = false;
    let mut after_colon = false;
    let mut escape_next = false;

    for ch in inner.chars() {
        if escape_next {
            if in_value {
                value.push(ch);
            }
            escape_next = false;
            continue;
        }

        match ch {
            '\\' => escape_next = true,
            '"' => {
                if !in_key && !in_value && !after_colon {
                    in_key = true;
                } else if in_key {
                    in_key = false;
                } else if !in_value && after_colon {
                    in_value = true;
                } else if in_value {
                    in_value = false;
                }
            }
            ':' if !in_key && !in_value => after_colon = true,
            ',' if !in_key && !in_value => {
                let k = key_str.trim().to_lowercase();
                result.push((k == "true" || k == "1", value.clone()));
                key_str.clear();
                value.clear();
                after_colon = false;
            }
            _ if in_key => key_str.push(ch),
            _ if in_value => value.push(ch),
            _ => {}
        }
    }

    if !key_str.is_empty() {
        let k = key_str.trim().to_lowercase();
        result.push((k == "true" || k == "1", value));
    }

    result
}

// JSON field extraction helpers for struct parsing

/// Extract an integer field from JSON object
#[no_mangle]
pub extern "C" fn json_get_int(json_str: *const c_char, field_name: *const c_char) -> i32 {
    if json_str.is_null() || field_name.is_null() {
        return 0;
    }

    unsafe {
        let json_cstr = CStr::from_ptr(json_str);
        let field_cstr = CStr::from_ptr(field_name);

        let json = match json_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        };
        let field = match field_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        };

        let pattern = format!("\"{}\":", field);
        if let Some(pos) = json.find(&pattern) {
            let after_colon = &json[pos + pattern.len()..];
            let trimmed = after_colon.trim_start();
            let mut num_str = String::new();
            for ch in trimmed.chars() {
                if ch.is_ascii_digit() || ch == '-' {
                    num_str.push(ch);
                } else if !num_str.is_empty() {
                    break;
                }
            }
            return num_str.parse::<i32>().unwrap_or(0);
        }
        0
    }
}

/// Extract a float field from JSON object
#[no_mangle]
pub extern "C" fn json_get_float(json_str: *const c_char, field_name: *const c_char) -> f64 {
    if json_str.is_null() || field_name.is_null() {
        return 0.0;
    }

    unsafe {
        let json_cstr = CStr::from_ptr(json_str);
        let field_cstr = CStr::from_ptr(field_name);

        let json = match json_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return 0.0,
        };
        let field = match field_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return 0.0,
        };

        let pattern = format!("\"{}\":", field);
        if let Some(pos) = json.find(&pattern) {
            let after_colon = &json[pos + pattern.len()..];
            let trimmed = after_colon.trim_start();
            let mut num_str = String::new();
            for ch in trimmed.chars() {
                if ch.is_ascii_digit()
                    || ch == '-'
                    || ch == '.'
                    || ch == 'e'
                    || ch == 'E'
                    || ch == '+'
                {
                    num_str.push(ch);
                } else if !num_str.is_empty() {
                    break;
                }
            }
            return num_str.parse::<f64>().unwrap_or(0.0);
        }
        0.0
    }
}

/// Extract a boolean field from JSON object
#[no_mangle]
pub extern "C" fn json_get_bool(json_str: *const c_char, field_name: *const c_char) -> i32 {
    if json_str.is_null() || field_name.is_null() {
        return 0;
    }

    unsafe {
        let json_cstr = CStr::from_ptr(json_str);
        let field_cstr = CStr::from_ptr(field_name);

        let json = match json_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        };
        let field = match field_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        };

        let pattern = format!("\"{}\":", field);
        if let Some(pos) = json.find(&pattern) {
            let after_colon = &json[pos + pattern.len()..];
            let trimmed = after_colon.trim_start();
            if trimmed.starts_with("true") || trimmed.starts_with("1") {
                return 1;
            }
        }
        0
    }
}

/// Extract a string field from JSON object
/// Returns heap-allocated string with RC header
#[no_mangle]
pub extern "C" fn json_get_str(json_str: *const c_char, field_name: *const c_char) -> *mut c_char {
    if json_str.is_null() || field_name.is_null() {
        unsafe {
            let ptr = malloc(9);
            if ptr.is_null() {
                return std::ptr::null_mut();
            }
            *(ptr as *mut i32) = 1; // RC
            *(ptr.add(4) as *mut i32) = 0; // Length
            *ptr.add(8) = 0; // Null terminator
            return ptr.add(8) as *mut c_char;
        }
    }

    unsafe {
        let json_cstr = CStr::from_ptr(json_str);
        let field_cstr = CStr::from_ptr(field_name);

        let json = match json_cstr.to_str() {
            Ok(s) => s,
            Err(_) => {
                let ptr = malloc(9);
                if ptr.is_null() {
                    return std::ptr::null_mut();
                }
                *(ptr as *mut i32) = 1;
                *(ptr.add(4) as *mut i32) = 0;
                *ptr.add(8) = 0;
                return ptr.add(8) as *mut c_char;
            }
        };
        let field = match field_cstr.to_str() {
            Ok(s) => s,
            Err(_) => {
                let ptr = malloc(9);
                if ptr.is_null() {
                    return std::ptr::null_mut();
                }
                *(ptr as *mut i32) = 1;
                *(ptr.add(4) as *mut i32) = 0;
                *ptr.add(8) = 0;
                return ptr.add(8) as *mut c_char;
            }
        };

        let pattern = format!("\"{}\":", field);
        let value = if let Some(pos) = json.find(&pattern) {
            let after_colon = &json[pos + pattern.len()..];
            let trimmed = after_colon.trim_start();
            if trimmed.starts_with('"') {
                let mut result = String::new();
                let mut chars = trimmed[1..].chars();
                let mut escape_next = false;
                while let Some(ch) = chars.next() {
                    if escape_next {
                        result.push(ch);
                        escape_next = false;
                    } else if ch == '\\' {
                        escape_next = true;
                    } else if ch == '"' {
                        break;
                    } else {
                        result.push(ch);
                    }
                }
                result
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Allocate heap string with RC header
        let bytes = value.as_bytes();
        let total = 8 + bytes.len() + 1;
        let ptr = malloc(total);
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        *(ptr as *mut i32) = 1; // RC
        *(ptr.add(4) as *mut i32) = bytes.len() as i32; // Length
        let data = ptr.add(8);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
        *data.add(bytes.len()) = 0; // Null terminator
        data as *mut c_char
    }
}

/// Stringify data to JSON format
#[no_mangle]
pub extern "C" fn json_stringify(data: *const c_char) -> *mut c_char {
    if data.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(data);
        let rust_str = match c_str.to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        let json_output = format!("\"{}\"", rust_str.replace("\"", "\\\""));

        match CString::new(json_output) {
            Ok(result) => result.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }
}
