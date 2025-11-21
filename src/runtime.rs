use std::ffi::{CStr, CString};
use std::fs;
use std::io::{Read, Write};
use std::os::raw::c_char;
use std::path::Path;

/// Simple runtime panic function for error handling
#[no_mangle]
pub extern "C" fn panic_runtime(msg: *const u8) {
    if msg.is_null() {
        eprintln!("Runtime error: panic");
        std::process::exit(1);
    }

    // Convert C string to Rust string
    unsafe {
        let c_str = std::ffi::CStr::from_ptr(msg as *const i8);
        if let Ok(msg_str) = c_str.to_str() {
            eprintln!("{}", msg_str);
        } else {
            eprintln!("Runtime error: invalid UTF-8 in panic message");
        }
    }

    std::process::exit(1);
}

// ===== JSON BUILTIN FUNCTIONS =====

/// Parse JSON string and return a pointer to the parsed data
/// Returns a heap-allocated string containing JSON data (simplified for now)
/// Format: ptr points to RC-counted string with JSON content
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

        // For now, return the same string (placeholder for actual JSON parsing)
        // In a real implementation, this would parse JSON and return a structured format
        match CString::new(rust_str) {
            Ok(result) => result.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }
}

/// Stringify data to JSON format
/// Takes any string representation and converts to JSON string
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

        // For now, wrap the string in JSON format
        // In a real implementation, this would properly escape and format
        let json_output = format!("\"{}\"", rust_str.replace("\"", "\\\""));

        match CString::new(json_output) {
            Ok(result) => result.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }
}

// ===== FILE I/O FUNCTIONS =====

/// Read entire file contents as a string
/// Returns pointer to heap-allocated string, or null on error
#[no_mangle]
pub extern "C" fn file_read(path: *const c_char) -> *mut c_char {
    if path.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(path);
        let path_str = match c_str.to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        match fs::read_to_string(path_str) {
            Ok(content) => match CString::new(content) {
                Ok(result) => result.into_raw(),
                Err(_) => std::ptr::null_mut(),
            },
            Err(_) => std::ptr::null_mut(),
        }
    }
}

/// Write content to file (creates or overwrites)
/// Returns 0 on success, -1 on error
#[no_mangle]
pub extern "C" fn file_write(path: *const c_char, content: *const c_char) -> i32 {
    if path.is_null() || content.is_null() {
        return -1;
    }

    unsafe {
        let path_cstr = CStr::from_ptr(path);
        let content_cstr = CStr::from_ptr(content);

        let path_str = match path_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return -1,
        };

        let content_str = match content_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return -1,
        };

        match fs::write(path_str, content_str) {
            Ok(_) => 0,
            Err(_) => -1,
        }
    }
}

/// Append content to file
/// Returns 0 on success, -1 on error
#[no_mangle]
pub extern "C" fn file_append(path: *const c_char, content: *const c_char) -> i32 {
    if path.is_null() || content.is_null() {
        return -1;
    }

    unsafe {
        let path_cstr = CStr::from_ptr(path);
        let content_cstr = CStr::from_ptr(content);

        let path_str = match path_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return -1,
        };

        let content_str = match content_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return -1,
        };

        match fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path_str)
        {
            Ok(mut file) => match file.write_all(content_str.as_bytes()) {
                Ok(_) => 0,
                Err(_) => -1,
            },
            Err(_) => -1,
        }
    }
}

/// Check if file or directory exists
/// Returns 1 if exists, 0 if not
#[no_mangle]
pub extern "C" fn file_exists(path: *const c_char) -> i32 {
    if path.is_null() {
        return 0;
    }

    unsafe {
        let c_str = CStr::from_ptr(path);
        let path_str = match c_str.to_str() {
            Ok(s) => s,
            Err(_) => return 0,
        };

        if Path::new(path_str).exists() {
            1
        } else {
            0
        }
    }
}

/// Delete a file
/// Returns 0 on success, -1 on error
#[no_mangle]
pub extern "C" fn file_delete(path: *const c_char) -> i32 {
    if path.is_null() {
        return -1;
    }

    unsafe {
        let c_str = CStr::from_ptr(path);
        let path_str = match c_str.to_str() {
            Ok(s) => s,
            Err(_) => return -1,
        };

        match fs::remove_file(path_str) {
            Ok(_) => 0,
            Err(_) => -1,
        }
    }
}

/// Create a directory
/// Returns 0 on success, -1 on error
#[no_mangle]
pub extern "C" fn file_mkdir(path: *const c_char) -> i32 {
    if path.is_null() {
        return -1;
    }

    unsafe {
        let c_str = CStr::from_ptr(path);
        let path_str = match c_str.to_str() {
            Ok(s) => s,
            Err(_) => return -1,
        };

        match fs::create_dir_all(path_str) {
            Ok(_) => 0,
            Err(_) => -1,
        }
    }
}

/// List files in a directory
/// Returns a specially formatted string with filenames separated by newlines
/// Format: "file1.txt\nfile2.txt\ndir1\n"
/// Returns null on error
#[no_mangle]
pub extern "C" fn file_list(path: *const c_char) -> *mut c_char {
    if path.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(path);
        let path_str = match c_str.to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        match fs::read_dir(path_str) {
            Ok(entries) => {
                let mut result = String::new();
                for entry in entries {
                    if let Ok(entry) = entry {
                        if let Ok(name) = entry.file_name().into_string() {
                            result.push_str(&name);
                            result.push('\n');
                        }
                    }
                }

                match CString::new(result) {
                    Ok(cstring) => cstring.into_raw(),
                    Err(_) => std::ptr::null_mut(),
                }
            }
            Err(_) => std::ptr::null_mut(),
        }
    }
}

/// Read file as lines (array of strings)
/// Returns a specially formatted string with lines separated by a delimiter
/// Format: "line1\x1Eline2\x1Eline3\x1E" (using ASCII Record Separator)
/// Returns null on error
#[no_mangle]
pub extern "C" fn file_read_lines(path: *const c_char) -> *mut c_char {
    if path.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let c_str = CStr::from_ptr(path);
        let path_str = match c_str.to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        match fs::read_to_string(path_str) {
            Ok(content) => {
                // Split by newlines and join with record separator
                let lines: Vec<&str> = content.lines().collect();
                let result = lines.join("\x1E");

                match CString::new(result) {
                    Ok(cstring) => cstring.into_raw(),
                    Err(_) => std::ptr::null_mut(),
                }
            }
            Err(_) => std::ptr::null_mut(),
        }
    }
}
