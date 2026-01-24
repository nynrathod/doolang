//! Helper Functions
//! String conversion, memory allocation, and utility functions.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

/// Convert C string to Rust String
pub fn c_to_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe {
        CStr::from_ptr(ptr).to_string_lossy().to_string()
    }
}

/// Convert Rust string to C string (libc allocated)
pub fn string_to_c(s: &str) -> *const c_char {
    unsafe {
        let len = s.len();
        let ptr = libc::malloc(len + 1) as *mut u8;
        if ptr.is_null() {
            return std::ptr::null();
        }
        std::ptr::copy_nonoverlapping(s.as_ptr(), ptr, len);
        *ptr.add(len) = 0;
        ptr as *const c_char
    }
}

/// Convert Rust String to RC-compatible C string
/// Layout: [rc:i32][len:i32][data...][0]
/// Returns pointer to data (base + 8)
pub fn string_to_rc_str(s: &str) -> *const c_char {
    unsafe {
        let len = s.len();
        let total_size = len + 1 + 8;
        let alloc_size = (total_size + 15) & !15;
        
        let ptr = libc::malloc(alloc_size) as *mut u8;
        if ptr.is_null() {
            return std::ptr::null();
        }
        
        std::ptr::write_bytes(ptr, 0, alloc_size);
        
        // Write RC header
        *(ptr as *mut i32) = 1;  // refcount
        *(ptr.add(4) as *mut i32) = len as i32;  // length
        
        // Write data
        let data_ptr = ptr.add(8);
        std::ptr::copy_nonoverlapping(s.as_ptr(), data_ptr, len);
        *data_ptr.add(len) = 0;  // null terminator
        
        data_ptr as *const c_char
    }
}

/// Parse query string into HashMap
pub fn parse_query(query: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            map.insert(
                urlencoding::decode(key).unwrap_or_default().to_string(),
                urlencoding::decode(value).unwrap_or_default().to_string(),
            );
        }
    }
    map
}

/// Thread-local storage for current request path (for RFC 7807 instance field)
thread_local! {
    static CURRENT_REQUEST_PATH: std::cell::RefCell<String> = std::cell::RefCell::new("/".to_string());
    static LAST_ERROR_STATUS: std::cell::Cell<i32> = std::cell::Cell::new(0);
    static LAST_ERROR_JSON: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
}

pub fn set_current_request_path(path: &str) {
    CURRENT_REQUEST_PATH.with(|p| *p.borrow_mut() = path.to_string());
}

pub fn get_current_request_path() -> String {
    CURRENT_REQUEST_PATH.with(|p| p.borrow().clone())
}

pub fn set_last_error(status: i32, json: String) {
    LAST_ERROR_STATUS.with(|s| s.set(status));
    LAST_ERROR_JSON.with(|j| *j.borrow_mut() = json);
}

pub fn get_last_error_status() -> i32 {
    LAST_ERROR_STATUS.with(|s| s.get())
}

pub fn get_last_error_json() -> String {
    LAST_ERROR_JSON.with(|j| j.borrow().clone())
}

pub fn clear_last_error() {
    set_last_error(0, String::new());
}
