//! doo_ffi_file - Complete File System FFI Library
//!
//! Provides production-grade file operations matching Rust's std::fs:
//! - Basic I/O: read, write, append, delete, copy, move
//! - Directory: mkdir, rmdir, rmdir_all, list
//! - Metadata: exists, size, metadata (full struct)
//! - Lines: read_lines
//!
//! Design Principles:
//! - Single source of truth for all file operations
//! - Centralized error handling via FileError struct
//! - All fallible operations return DooResult with proper error payloads
//! - Memory managed via libc malloc (compatible with doo_ffi_core)

use std::ffi::{c_void, CStr};
use std::fs;
use std::io::Write;
use std::os::raw::c_char;
use std::path::Path;
use std::time::SystemTime;

use doo_ffi_core::{doo_alloc, doo_alloc_string, DooResult};

// ============================================================================
// Struct Definitions - Match std/File.doo exactly
// ============================================================================

/// FileError struct layout - matches Doo's FileError struct
/// Struct: { Message: Str }
#[repr(C)]
pub struct DooFileError {
    /// Error message (RC string pointer)
    pub message: *mut c_char,
}

/// FileMetadata struct layout - matches Doo's FileMetadata struct exactly
/// Field order MUST match std/File.doo declaration order
/// LLVM struct: %FileMetadata = type { i1, i1, i1, i64, i1, i64, i64, i64 }
/// LLVM uses natural alignment:
///   - i1 at offset 0, 1, 2 (each 1 byte)
///   - i64 at offset 8 (aligned, 5 bytes padding after i1s)
///   - i1 at offset 16
///   - i64 at offset 24 (aligned, 7 bytes padding after i1)
///   - i64 at offset 32, 40
/// Total: 48 bytes
#[repr(C, align(8))]
pub struct DooFileMetadata {
    pub is_file: u8,    // offset 0: IsFile (i1)
    pub is_dir: u8,     // offset 1: IsDir (i1)
    pub is_symlink: u8, // offset 2: IsSymlink (i1)
    pub _pad1: [u8; 5], // offset 3-7: padding for i64 alignment
    pub size: i64,      // offset 8: Size (i64)
    pub readonly: u8,   // offset 16: Readonly (i1)
    pub _pad2: [u8; 7], // offset 17-23: padding for i64 alignment
    pub created: i64,   // offset 24: Created (i64)
    pub modified: i64,  // offset 32: Modified (i64)
    pub accessed: i64,  // offset 40: Accessed (i64)
}

// ============================================================================
// Helper Functions - Single Source of Truth
// ============================================================================

/// Convert C string pointer to Rust String
#[inline]
fn c_to_string(s: *const c_char) -> Result<String, String> {
    if s.is_null() {
        return Err("Null pointer provided".to_string());
    }
    unsafe {
        CStr::from_ptr(s)
            .to_str()
            .map(|s| s.to_string())
            .map_err(|e| format!("Invalid UTF-8: {}", e))
    }
}

/// Allocate FileError struct with message
/// Uses simple C string (ownership transfers to caller)
#[inline]
fn alloc_file_error(message: &str) -> *mut DooFileError {
    unsafe {
        let err_ptr = doo_alloc(std::mem::size_of::<DooFileError>()) as *mut DooFileError;
        if err_ptr.is_null() {
            return std::ptr::null_mut();
        }
        // Use simple C string - codegen will handle conversion
        (*err_ptr).message = doo_alloc_string(message);
        err_ptr
    }
}

/// Create Ok result with string value
#[inline]
fn make_ok_string(s: &str) -> *mut DooResult {
    DooResult::ok_string(s).into_raw()
}

/// Create Ok result with no value (void operations)
#[inline]
fn make_ok_void() -> *mut DooResult {
    DooResult::ok_empty().into_raw()
}

/// Create Ok result with integer value
#[inline]
fn make_ok_int(n: i64) -> *mut DooResult {
    DooResult::ok(n as *mut c_void, 0).into_raw()
}

/// Create Ok result with metadata struct
#[inline]
fn make_ok_metadata(meta: DooFileMetadata) -> *mut DooResult {
    unsafe {
        let meta_ptr = doo_alloc(std::mem::size_of::<DooFileMetadata>()) as *mut DooFileMetadata;
        if meta_ptr.is_null() {
            return make_err("Failed to allocate metadata");
        }
        std::ptr::write(meta_ptr, meta);
        DooResult::ok(
            meta_ptr as *mut c_void,
            std::mem::size_of::<DooFileMetadata>() as u32,
        )
        .into_raw()
    }
}

/// Create Err result with FileError struct
#[inline]
fn make_err(message: &str) -> *mut DooResult {
    let err_ptr = alloc_file_error(message);
    DooResult::err(
        500,
        err_ptr as *mut c_void,
        std::mem::size_of::<DooFileError>() as u32,
    )
    .into_raw()
}

/// Convert SystemTime to Unix timestamp (seconds since epoch)
#[inline]
fn to_timestamp(time: Result<SystemTime, std::io::Error>) -> i64 {
    match time {
        Ok(t) => t
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        Err(_) => 0,
    }
}

// ============================================================================
// BASIC FILE I/O
// ============================================================================

/// Read entire file contents as string
/// Returns: Result<String, FileError>
#[no_mangle]
pub extern "C" fn doo_file_read(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    match fs::read_to_string(&path_str) {
        Ok(content) => make_ok_string(&content),
        Err(e) => make_err(&format!("Failed to read file '{}': {}", path_str, e)),
    }
}

/// Write string content to file (creates or overwrites)
/// Returns: Result<Void, FileError>
#[no_mangle]
pub extern "C" fn doo_file_write(path: *const c_char, content: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    let content_str = match c_to_string(content) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    match fs::write(&path_str, &content_str) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err(&format!("Failed to write file '{}': {}", path_str, e)),
    }
}

/// Append string content to file (creates if not exists)
/// Returns: Result<Void, FileError>
#[no_mangle]
pub extern "C" fn doo_file_append(path: *const c_char, content: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    let content_str = match c_to_string(content) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    let mut file = match fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path_str)
    {
        Ok(f) => f,
        Err(e) => {
            return make_err(&format!(
                "Failed to open file '{}' for append: {}",
                path_str, e
            ))
        }
    };

    match file.write_all(content_str.as_bytes()) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err(&format!("Failed to append to file '{}': {}", path_str, e)),
    }
}

/// Delete a file
/// Returns: Result<Void, FileError>
#[no_mangle]
pub extern "C" fn doo_file_delete(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    match fs::remove_file(&path_str) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err(&format!("Failed to delete file '{}': {}", path_str, e)),
    }
}

/// Copy a file from source to destination
/// Returns: Result<Void, FileError>
#[no_mangle]
pub extern "C" fn doo_file_copy(src: *const c_char, dst: *const c_char) -> *mut DooResult {
    let src_str = match c_to_string(src) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    let dst_str = match c_to_string(dst) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    match fs::copy(&src_str, &dst_str) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err(&format!(
            "Failed to copy '{}' to '{}': {}",
            src_str, dst_str, e
        )),
    }
}

/// Move/rename a file from source to destination
/// Returns: Result<Void, FileError>
#[no_mangle]
pub extern "C" fn doo_file_move(src: *const c_char, dst: *const c_char) -> *mut DooResult {
    let src_str = match c_to_string(src) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    let dst_str = match c_to_string(dst) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    match fs::rename(&src_str, &dst_str) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err(&format!(
            "Failed to move '{}' to '{}': {}",
            src_str, dst_str, e
        )),
    }
}

/// Read file contents (preserves line structure)
/// Returns: Result<String, FileError>
#[no_mangle]
pub extern "C" fn doo_file_read_lines(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    match fs::read_to_string(&path_str) {
        Ok(content) => make_ok_string(&content),
        Err(e) => make_err(&format!("Failed to read lines from '{}': {}", path_str, e)),
    }
}

// ============================================================================
// DIRECTORY OPERATIONS
// ============================================================================

/// Create directory (and parent directories if needed - matches Doo std behavior)
/// Returns: Result<Void, FileError>
#[no_mangle]
pub extern "C" fn doo_file_mkdir(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    // Use create_dir_all to match Doo std behavior (creates parents)
    match fs::create_dir_all(&path_str) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err(&format!("Failed to create directory '{}': {}", path_str, e)),
    }
}

/// Create directory and all parent directories
/// Returns: Result<Void, FileError>
#[no_mangle]
pub extern "C" fn doo_file_mkdir_all(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    match fs::create_dir_all(&path_str) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err(&format!(
            "Failed to create directories '{}': {}",
            path_str, e
        )),
    }
}

/// Remove empty directory
/// Returns: Result<Void, FileError>
#[no_mangle]
pub extern "C" fn doo_file_rmdir(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    match fs::remove_dir(&path_str) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err(&format!("Failed to remove directory '{}': {}", path_str, e)),
    }
}

/// Remove directory and all contents recursively
/// Returns: Result<Void, FileError>
#[no_mangle]
pub extern "C" fn doo_file_rmdir_all(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    match fs::remove_dir_all(&path_str) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err(&format!(
            "Failed to remove directory tree '{}': {}",
            path_str, e
        )),
    }
}

/// List directory contents (comma-separated string)
/// Returns: Result<String, FileError>
#[no_mangle]
pub extern "C" fn doo_file_list_dir(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    match fs::read_dir(&path_str) {
        Ok(entries) => {
            let names: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect();
            make_ok_string(&names.join(","))
        }
        Err(e) => make_err(&format!("Failed to list directory '{}': {}", path_str, e)),
    }
}

// ============================================================================
// FILE METADATA OPERATIONS
// ============================================================================

/// Check if path exists
/// Returns: Bool (i32: 0 = false, 1 = true) - never fails
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

/// Get file size in bytes
/// Returns: Result<Int, FileError>
#[no_mangle]
pub extern "C" fn doo_file_size(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    match fs::metadata(&path_str) {
        Ok(meta) => make_ok_int(meta.len() as i64),
        Err(e) => make_err(&format!("Failed to get size of '{}': {}", path_str, e)),
    }
}

/// Get comprehensive file/directory metadata
/// Returns: Result<FileMetadata, FileError>
#[no_mangle]
pub extern "C" fn doo_file_metadata(path: *const c_char) -> *mut DooResult {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(e) => return make_err(&e),
    };

    match fs::metadata(&path_str) {
        Ok(meta) => {
            let file_meta = DooFileMetadata {
                is_file: if meta.is_file() { 1 } else { 0 },
                is_dir: if meta.is_dir() { 1 } else { 0 },
                is_symlink: if meta.file_type().is_symlink() { 1 } else { 0 },
                _pad1: [0; 5],
                size: meta.len() as i64,
                readonly: if meta.permissions().readonly() { 1 } else { 0 },
                _pad2: [0; 7],
                created: to_timestamp(meta.created()),
                modified: to_timestamp(meta.modified()),
                accessed: to_timestamp(meta.accessed()),
            };
            make_ok_metadata(file_meta)
        }
        Err(e) => make_err(&format!("Failed to get metadata for '{}': {}", path_str, e)),
    }
}

/// Check if path is a file
/// Returns: Bool (i32: 0 = false, 1 = true) - never fails
#[no_mangle]
pub extern "C" fn doo_file_is_file(path: *const c_char) -> i32 {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(_) => return 0,
    };

    if Path::new(&path_str).is_file() {
        1
    } else {
        0
    }
}

/// Check if path is a directory
/// Returns: Bool (i32: 0 = false, 1 = true) - never fails
#[no_mangle]
pub extern "C" fn doo_file_is_dir(path: *const c_char) -> i32 {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(_) => return 0,
    };

    if Path::new(&path_str).is_dir() {
        1
    } else {
        0
    }
}

/// Get file modification time as Unix timestamp
/// Returns: i64 (-1 on error)
#[no_mangle]
pub extern "C" fn doo_file_modified_time(path: *const c_char) -> i64 {
    let path_str = match c_to_string(path) {
        Ok(s) => s,
        Err(_) => return -1,
    };

    fs::metadata(&path_str)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(-1)
}

// ============================================================================
// MEMORY MANAGEMENT
// ============================================================================

/// Free a DooResult (must be called by Doo runtime after use)
#[no_mangle]
pub extern "C" fn doo_file_free_result(result: *mut DooResult) {
    if result.is_null() {
        return;
    }
    unsafe {
        let res = Box::from_raw(result);
        if !res.data.is_null() {
            libc::free(res.data as *mut c_void);
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn test_file_write_read() {
        let path = CString::new("test_doo_ffi_file.txt").unwrap();
        let content = CString::new("Hello from Doo FFI!").unwrap();

        // Write
        let write_result = doo_file_write(path.as_ptr(), content.as_ptr());
        assert!(!write_result.is_null());
        unsafe {
            assert_eq!((*write_result).tag, 0); // Ok
            doo_file_free_result(write_result);
        }

        // Exists
        assert_eq!(doo_file_exists(path.as_ptr()), 1);

        // Read
        let read_result = doo_file_read(path.as_ptr());
        assert!(!read_result.is_null());
        unsafe {
            assert_eq!((*read_result).tag, 0); // Ok
            doo_file_free_result(read_result);
        }

        // Cleanup
        let del_result = doo_file_delete(path.as_ptr());
        if !del_result.is_null() {
            unsafe {
                doo_file_free_result(del_result);
            }
        }
    }

    #[test]
    fn test_file_metadata() {
        let path = CString::new("test_metadata.txt").unwrap();
        let content = CString::new("Metadata test content").unwrap();

        // Create file
        let write_result = doo_file_write(path.as_ptr(), content.as_ptr());
        unsafe {
            doo_file_free_result(write_result);
        }

        // Get metadata
        let meta_result = doo_file_metadata(path.as_ptr());
        assert!(!meta_result.is_null());
        unsafe {
            assert_eq!((*meta_result).tag, 0); // Ok
            let meta_ptr = (*meta_result).data as *const DooFileMetadata;
            assert!(!meta_ptr.is_null());
            assert_eq!((*meta_ptr).is_file, 1);
            assert_eq!((*meta_ptr).is_dir, 0);
            doo_file_free_result(meta_result);
        }

        // Cleanup
        let del_result = doo_file_delete(path.as_ptr());
        if !del_result.is_null() {
            unsafe {
                doo_file_free_result(del_result);
            }
        }
    }

    #[test]
    fn test_file_move() {
        let src = CString::new("test_move_src.txt").unwrap();
        let dst = CString::new("test_move_dst.txt").unwrap();
        let content = CString::new("Move test").unwrap();

        // Create source file
        let write_result = doo_file_write(src.as_ptr(), content.as_ptr());
        unsafe {
            doo_file_free_result(write_result);
        }

        // Move file
        let move_result = doo_file_move(src.as_ptr(), dst.as_ptr());
        assert!(!move_result.is_null());
        unsafe {
            assert_eq!((*move_result).tag, 0); // Ok
            doo_file_free_result(move_result);
        }

        // Verify source doesn't exist
        assert_eq!(doo_file_exists(src.as_ptr()), 0);

        // Verify destination exists
        assert_eq!(doo_file_exists(dst.as_ptr()), 1);

        // Cleanup
        let del_result = doo_file_delete(dst.as_ptr());
        if !del_result.is_null() {
            unsafe {
                doo_file_free_result(del_result);
            }
        }
    }
}
