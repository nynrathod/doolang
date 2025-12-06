use std::ffi::{CStr, CString};
use std::fs;
use std::io::Write;
use std::os::raw::c_char;
use std::path::Path;
use std::time::SystemTime;

/// Doo Result struct layout: { i32 tag, void* value }
/// tag = 0 for Ok, tag = 1 for Err
/// value = pointer to actual data (string, int, etc.)
#[repr(C)]
pub struct DooResult {
    tag: i32,
    value: *mut std::ffi::c_void,
}

/// File error struct layout - matches Doo's FileError struct
#[repr(C)]
pub struct DooFileError {
    message: *mut c_char,
}

/// File metadata struct layout
#[repr(C)]
pub struct DooFileMetadata {
    isFile: i32,
    isDir: i32,
    isSymlink: i32,
    size: i64,
    readonly: i32,
    created: i64,
    modified: i64,
    accessed: i64,
}

/// Free a Result struct allocated by this library
#[no_mangle]
pub extern "C" fn doo_free_result(result: *mut DooResult) {
    if !result.is_null() {
        unsafe {
            let _ = Box::from_raw(result);
        }
    }
}

/// Free a string allocated by this library
#[no_mangle]
pub extern "C" fn doo_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            let _ = CString::from_raw(s);
        }
    }
}

/// Helper to convert Rust String to C string pointer
fn string_to_c(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(cs) => cs.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Helper to convert C string to Rust String
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

/// Create a Result struct with Ok value (string) - returns heap pointer
fn make_ok_string(s: String) -> *mut DooResult {
    Box::into_raw(Box::new(DooResult {
        tag: 0,
        value: string_to_c(s) as *mut std::ffi::c_void,
    }))
}

/// Create a Result struct with Err value (string) - returns heap pointer
fn make_err_string(s: String) -> *mut DooResult {
    Box::into_raw(Box::new(DooResult {
        tag: 1,
        value: string_to_c(s) as *mut std::ffi::c_void,
    }))
}

/// Create a Result struct with Err value (FileError struct) - returns heap pointer
fn make_err_file_error(message: String) -> *mut DooResult {
    let error_struct = Box::new(DooFileError {
        message: string_to_c(message),
    });
    Box::into_raw(Box::new(DooResult {
        tag: 1,
        value: Box::into_raw(error_struct) as *mut std::ffi::c_void,
    }))
}

/// Create a Result struct with Ok value (int) - returns heap pointer
fn make_ok_int(n: i64) -> *mut DooResult {
    Box::into_raw(Box::new(DooResult {
        tag: 0,
        value: n as *mut std::ffi::c_void,
    }))
}

/// Create a Result struct with Ok value (void) - returns heap pointer
fn make_ok_void() -> *mut DooResult {
    Box::into_raw(Box::new(DooResult {
        tag: 0,
        value: std::ptr::null_mut(),
    }))
}

/// Create a Result struct with Ok value (metadata) - returns heap pointer
fn make_ok_metadata(metadata: DooFileMetadata) -> *mut DooResult {
    Box::into_raw(Box::new(DooResult {
        tag: 0,
        value: Box::into_raw(Box::new(metadata)) as *mut std::ffi::c_void,
    }))
}

/// Read entire file content as string
/// Returns: Pointer to Result<String, FileError>
#[no_mangle]
pub extern "C" fn doo_file_read(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err_file_error(e),
    };

    match fs::read_to_string(&path_str) {
        Ok(content) => make_ok_string(content),
        Err(e) => make_err_file_error(format!("Failed to read file: {}", e)),
    }
}

/// Write content to file (overwrites existing)
/// Returns: Pointer to Result<Void, FileError>
#[no_mangle]
pub extern "C" fn doo_file_write(path: *const c_char, content: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err_file_error(e),
    };

    let content_str = match c_to_string(content) {
        Ok(s) => s,
        Err(e) => return make_err_file_error(e),
    };

    match fs::write(&path_str, content_str) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err_file_error(format!("Failed to write file: {}", e)),
    }
}

/// Append content to file
/// Returns: Pointer to Result<Void, FileError>
#[no_mangle]
pub extern "C" fn doo_file_append(path: *const c_char, content: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err_file_error(e),
    };

    let content_str = match c_to_string(content) {
        Ok(s) => s,
        Err(e) => return make_err_file_error(e),
    };

    match fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path_str)
    {
        Ok(mut file) => match file.write_all(content_str.as_bytes()) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err_file_error(format!("Failed to append to file: {}", e)),
        },
        Err(e) => make_err_file_error(format!("Failed to open file for append: {}", e)),
    }
}

/// Check if file exists
/// Returns: Bool (not a Result - this operation can't fail)
#[no_mangle]
pub extern "C" fn doo_file_exists(path: *const c_char) -> i32 {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(_) => return 0,
    };

    if Path::new(&path_str).exists() {
        1
    } else {
        0
    }
}

/// Delete a file
/// Returns: Pointer to Result<Void, FileError>
#[no_mangle]
pub extern "C" fn doo_file_delete(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err_file_error(e),
    };

    match fs::remove_file(&path_str) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err_file_error(format!("Failed to delete file: {}", e)),
    }
}

/// Get file size in bytes
/// Returns: Pointer to Result<Int, FileError>
#[no_mangle]
pub extern "C" fn doo_file_size(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err_file_error(e),
    };

    match fs::metadata(&path_str) {
        Ok(metadata) => make_ok_int(metadata.len() as i64),
        Err(e) => make_err_file_error(format!("Failed to get file size: {}", e)),
    }
}

/// Create a directory (and parent directories if needed)
/// Returns: Pointer to Result<Void, FileError>
#[no_mangle]
pub extern "C" fn doo_file_mkdir(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err_file_error(e),
    };

    match fs::create_dir_all(&path_str) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err_file_error(format!("Failed to create directory: {}", e)),
    }
}

/// Check if path is a directory
/// Returns: Bool (not a Result)
#[no_mangle]
pub extern "C" fn doo_file_is_dir(path: *const c_char) -> i32 {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(_) => return 0,
    };

    match fs::metadata(&path_str) {
        Ok(metadata) => {
            if metadata.is_dir() {
                1
            } else {
                0
            }
        }
        Err(_) => 0,
    }
}

/// Get comprehensive file/directory metadata
/// Returns: Pointer to Result<DooFileMetadata, FileError>
#[no_mangle]
pub extern "C" fn doo_file_metadata(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err_file_error(e),
    };

    match fs::metadata(&path_str) {
        Ok(metadata) => {
            // Helper to convert SystemTime to Unix timestamp
            let to_timestamp = |time: Result<SystemTime, std::io::Error>| -> i64 {
                match time {
                    Ok(t) => match t.duration_since(SystemTime::UNIX_EPOCH) {
                        Ok(duration) => duration.as_secs() as i64,
                        Err(_) => 0,
                    },
                    Err(_) => 0,
                }
            };

            let file_metadata = DooFileMetadata {
                isFile: if metadata.is_file() { 1 } else { 0 },
                isDir: if metadata.is_dir() { 1 } else { 0 },
                isSymlink: if metadata.file_type().is_symlink() {
                    1
                } else {
                    0
                },
                size: metadata.len() as i64,
                readonly: if metadata.permissions().readonly() {
                    1
                } else {
                    0
                },
                created: to_timestamp(metadata.created()),
                modified: to_timestamp(metadata.modified()),
                accessed: to_timestamp(metadata.accessed()),
            };

            make_ok_metadata(file_metadata)
        }
        Err(e) => make_err_file_error(format!("Failed to get file metadata: {}", e)),
    }
}

/// List directory contents (comma-separated)
/// Returns: Pointer to Result<String, FileError>
#[no_mangle]
pub extern "C" fn doo_file_list_dir(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err_file_error(e),
    };

    match fs::read_dir(&path_str) {
        Ok(entries) => {
            let mut names = Vec::new();
            for entry in entries {
                match entry {
                    Ok(e) => {
                        if let Some(name) = e.file_name().to_str() {
                            names.push(name.to_string());
                        }
                    }
                    Err(_) => continue,
                }
            }
            make_ok_string(names.join(","))
        }
        Err(e) => make_err_file_error(format!("Failed to list directory: {}", e)),
    }
}

/// Copy a file
/// Returns: Pointer to Result<Void, FileError>
#[no_mangle]
pub extern "C" fn doo_file_copy(src: *const c_char, dst: *const c_char) -> *mut DooResult {
    let src_str = match c_to_string(src) {
        Ok(s) => s,
        Err(e) => return make_err_file_error(e),
    };

    let dst_str = match c_to_string(dst) {
        Ok(s) => s,
        Err(e) => return make_err_file_error(e),
    };

    match fs::copy(&src_str, &dst_str) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err_file_error(format!("Failed to copy file: {}", e)),
    }
}

/// Move/rename a file
/// Returns: Pointer to Result<Void, FileError>
#[no_mangle]
pub extern "C" fn doo_file_move(src: *const c_char, dst: *const c_char) -> *mut DooResult {
    let src_str = match c_to_string(src) {
        Ok(s) => s,
        Err(e) => return make_err_file_error(e),
    };

    let dst_str = match c_to_string(dst) {
        Ok(s) => s,
        Err(e) => return make_err_file_error(e),
    };

    match fs::rename(&src_str, &dst_str) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err_file_error(format!("Failed to move file: {}", e)),
    }
}

/// Read file as lines (newline-separated)
/// Returns: Pointer to Result<String, FileError>
#[no_mangle]
pub extern "C" fn doo_file_read_lines(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err_file_error(e),
    };

    match fs::read_to_string(&path_str) {
        Ok(content) => make_ok_string(content),
        Err(e) => make_err_file_error(format!("Failed to read file lines: {}", e)),
    }
}

/// Remove a directory (must be empty)
/// Returns: Pointer to Result<Void, FileError>
#[no_mangle]
pub extern "C" fn doo_file_rmdir(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err_file_error(e),
    };

    match fs::remove_dir(&path_str) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err_file_error(format!("Failed to remove directory: {}", e)),
    }
}

/// Remove a directory and all its contents recursively
/// Returns: Pointer to Result<Void, FileError>
#[no_mangle]
pub extern "C" fn doo_file_rmdir_all(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err_file_error(e),
    };

    match fs::remove_dir_all(&path_str) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err_file_error(format!("Failed to remove directory recursively: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn test_file_operations() {
        let test_file = CString::new("test_doo_file.txt").unwrap();
        let content = CString::new("Hello, Doo!").unwrap();

        // Write
        let result = doo_file_write(test_file.as_ptr(), content.as_ptr());
        assert!(!result.is_null());
        unsafe {
            assert_eq!((*result).tag, 0); // Ok
            doo_free_result(result);
        }

        // Exists
        assert_eq!(doo_file_exists(test_file.as_ptr()), 1);

        // Read
        let read_result = doo_file_read(test_file.as_ptr());
        assert!(!read_result.is_null());
        unsafe {
            assert_eq!((*read_result).tag, 0); // Ok
            doo_free_result(read_result);
        }

        // Cleanup
        let del_result = doo_file_delete(test_file.as_ptr());
        if !del_result.is_null() {
            unsafe {
                doo_free_result(del_result);
            }
        }
    }
}
