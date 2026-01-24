//! doo_ffi_file - Complete File System FFI Library
//!
//! Provides:
//! - Basic I/O: read, write, append, delete, copy
//! - Directory: mkdir, rmdir, list
//! - Metadata: exists, size, is_file, is_dir

use std::ffi::CStr;
use std::os::raw::c_char;
use std::path::Path;
use std::fs;

use doo_ffi_core::DooResult;

// ============================================================================
// String Helpers
// ============================================================================

fn c_to_string(s: *const c_char) -> Result<String, String> {
    if s.is_null() {
        return Err("Null pointer".to_string());
    }
    unsafe {
        CStr::from_ptr(s)
            .to_str()
            .map(|s| s.to_string())
            .map_err(|e| format!("Invalid UTF-8: {}", e))
    }
}

// ============================================================================
// BASIC FILE I/O
// ============================================================================

/// Read entire file contents as string
#[no_mangle]
pub extern "C" fn doo_file_read(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };
    
    match fs::read_to_string(&path_str) {
        Ok(content) => DooResult::ok_string(&content).into_raw(),
        Err(e) => DooResult::err_str(404, &format!("Failed to read file: {}", e)).into_raw(),
    }
}

/// Write string content to file (overwrites if exists)
#[no_mangle]
pub extern "C" fn doo_file_write(path: *const c_char, content: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };
    
    let content_str = match c_to_string(content) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };
    
    match fs::write(&path_str, &content_str) {
        Ok(_) => DooResult::ok_empty().into_raw(),
        Err(e) => DooResult::err_str(500, &format!("Failed to write file: {}", e)).into_raw(),
    }
}

/// Append string content to file
#[no_mangle]
pub extern "C" fn doo_file_append(path: *const c_char, content: *const c_char) -> *mut DooResult {
    use std::io::Write;
    
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };
    
    let content_str = match c_to_string(content) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };
    
    let mut file = match fs::OpenOptions::new().create(true).append(true).open(&path_str) {
        Ok(f) => f,
        Err(e) => return DooResult::err_str(500, &format!("Failed to open file: {}", e)).into_raw(),
    };
    
    match file.write_all(content_str.as_bytes()) {
        Ok(_) => DooResult::ok_empty().into_raw(),
        Err(e) => DooResult::err_str(500, &format!("Failed to append: {}", e)).into_raw(),
    }
}

/// Delete a file
#[no_mangle]
pub extern "C" fn doo_file_delete(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };
    
    match fs::remove_file(&path_str) {
        Ok(_) => DooResult::ok_empty().into_raw(),
        Err(e) => DooResult::err_str(500, &format!("Failed to delete: {}", e)).into_raw(),
    }
}

/// Copy a file
#[no_mangle]
pub extern "C" fn doo_file_copy(src: *const c_char, dst: *const c_char) -> *mut DooResult {
    let src_str = match c_to_string(src) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };
    
    let dst_str = match c_to_string(dst) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };
    
    match fs::copy(&src_str, &dst_str) {
        Ok(_) => DooResult::ok_empty().into_raw(),
        Err(e) => DooResult::err_str(500, &format!("Failed to copy: {}", e)).into_raw(),
    }
}

// ============================================================================
// DIRECTORY OPERATIONS
// ============================================================================

/// Create directory
#[no_mangle]
pub extern "C" fn doo_file_mkdir(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };
    
    match fs::create_dir(&path_str) {
        Ok(_) => DooResult::ok_empty().into_raw(),
        Err(e) => DooResult::err_str(500, &format!("Failed to create directory: {}", e)).into_raw(),
    }
}

/// Create directory and all parent directories
#[no_mangle]
pub extern "C" fn doo_file_mkdir_all(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };
    
    match fs::create_dir_all(&path_str) {
        Ok(_) => DooResult::ok_empty().into_raw(),
        Err(e) => DooResult::err_str(500, &format!("Failed to create directories: {}", e)).into_raw(),
    }
}

/// Remove empty directory
#[no_mangle]
pub extern "C" fn doo_file_rmdir(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };
    
    match fs::remove_dir(&path_str) {
        Ok(_) => DooResult::ok_empty().into_raw(),
        Err(e) => DooResult::err_str(500, &format!("Failed to remove directory: {}", e)).into_raw(),
    }
}

/// Remove directory and all contents
#[no_mangle]
pub extern "C" fn doo_file_rmdir_all(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };
    
    match fs::remove_dir_all(&path_str) {
        Ok(_) => DooResult::ok_empty().into_raw(),
        Err(e) => DooResult::err_str(500, &format!("Failed to remove directory tree: {}", e)).into_raw(),
    }
}

/// List directory contents as JSON array
#[no_mangle]
pub extern "C" fn doo_file_list_dir(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };
    
    let entries: Vec<String> = match fs::read_dir(&path_str) {
        Ok(dir) => dir
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect(),
        Err(e) => return DooResult::err_str(500, &format!("Failed to read directory: {}", e)).into_raw(),
    };
    
    let json = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string());
    DooResult::ok_string(&json).into_raw()
}

// ============================================================================
// FILE METADATA
// ============================================================================

/// Check if path exists
#[no_mangle]
pub extern "C" fn doo_file_exists(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };
    
    let exists = Path::new(&path_str).exists();
    DooResult::ok((exists as i32) as *mut std::ffi::c_void, 0).into_raw()
}

/// Get file size in bytes
#[no_mangle]
pub extern "C" fn doo_file_size(path: *const c_char) -> i64 {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    
    fs::metadata(&path_str)
        .map(|m| m.len() as i64)
        .unwrap_or(-1)
}

/// Check if path is a file
#[no_mangle]
pub extern "C" fn doo_file_is_file(path: *const c_char) -> bool {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(_) => return false,
    };
    Path::new(&path_str).is_file()
}

/// Check if path is a directory
#[no_mangle]
pub extern "C" fn doo_file_is_dir(path: *const c_char) -> bool {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(_) => return false,
    };
    Path::new(&path_str).is_dir()
}

/// Get file modification time as Unix timestamp
#[no_mangle]
pub extern "C" fn doo_file_modified_time(path: *const c_char) -> i64 {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    
    fs::metadata(&path_str)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(-1)
}

// ============================================================================
// MEMORY MANAGEMENT
// ============================================================================

/// Free a result (delegates to doo_ffi_core)
#[no_mangle]
pub extern "C" fn doo_free_result(result: *mut DooResult) {
    if result.is_null() {
        return;
    }
    unsafe {
        let res = Box::from_raw(result);
        if !res.data.is_null() {
            libc::free(res.data);
        }
    }
}
