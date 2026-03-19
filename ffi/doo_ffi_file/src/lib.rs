//! doo_ffi_file - Production-Grade File System FFI Library
//!
//! Cloud-grade file operations with full security hardening for DooCloud.
//!
//! Security Features:
//! - Path sandboxing: all paths resolved relative to BASE_DIR
//! - Path traversal prevention: canonicalization + containment check
//! - Symlink protection: uses symlink_metadata where needed
//! - File size limits: prevents OOM via large file reads
//! - Directory entry limits: prevents OOM on listing
//! - Concurrent file operation limit: prevents FD exhaustion
//! - Windows ADS/UNC blocking: prevents escape via NTFS streams/network paths
//! - Panic safety: catch_unwind on all FFI boundaries
//! - Atomic writes: temp file + rename pattern for data integrity
//! - Restrictive permissions: 0o600 for files, 0o700 for dirs on Unix
//!
//! Memory Model:
//! - All allocations via libc::malloc (compatible with doo_ffi_core)
//! - DooResult allocated via libc::malloc in into_raw()
//! - Consistent allocator: libc::malloc for alloc, libc::free for free

use std::ffi::c_void;
use std::fs;
use std::io::Write;
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::SystemTime;

use doo_ffi_core::helpers::{c_to_string, make_ok_int, make_ok_string, make_ok_void, safe_ffi};
use doo_ffi_core::{doo_alloc, doo_alloc_string, DooResult};

// ============================================================================
// Configuration Constants - Single Source of Truth
// ============================================================================

/// Maximum file size for read operations (100 MB)
const MAX_FILE_READ_SIZE: u64 = 100 * 1024 * 1024;

/// Maximum file size for write operations (100 MB)
const MAX_FILE_WRITE_SIZE: usize = 100 * 1024 * 1024;

/// Maximum directory entries returned by list_dir
const MAX_DIR_ENTRIES: usize = 10_000;

/// Maximum concurrent file operations (prevents FD exhaustion)
const MAX_CONCURRENT_FILE_OPS: usize = 500;

/// Maximum path length in characters
const MAX_PATH_LENGTH: usize = 4096;

// ============================================================================
// Global State - Thread-Safe
// ============================================================================

/// Base directory for sandboxed file operations.
/// Set once during init, immutable after.
/// When not set, uses CWD as base (local dev mode).
static BASE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Active file operation counter for concurrency limiting
static ACTIVE_FILE_OPS: AtomicUsize = AtomicUsize::new(0);

// ============================================================================
// Struct Definitions - Match std/File.doo exactly
// ============================================================================

/// FileError struct layout - matches Doo's FileError struct
/// Struct: { Message: Str }
#[repr(C)]
pub struct DooFileError {
    /// Error message (C string pointer allocated via doo_alloc_string)
    pub message: *mut c_char,
}

/// FileMetadata struct layout - matches Doo's FileMetadata struct exactly
/// Field order MUST match std/File.doo declaration order
/// LLVM struct: %FileMetadata = type { i1, i1, i1, i64, i1, i64, i64, i64 }
/// P06 field reordering: i64 fields first (alignment 8), then i8 fields (alignment 1).
/// Matches LLVM struct layout: { i64, i64, i64, i64, i8, i8, i8, i8 }
/// Total: 40 bytes (32 bytes for i64s + 4 bytes for i8s + 4 padding)
#[repr(C)]
pub struct DooFileMetadata {
    pub size: i64,      // P06 physical 0: Size (i64)
    pub created: i64,   // P06 physical 1: Created (i64)
    pub modified: i64,  // P06 physical 2: Modified (i64)
    pub accessed: i64,  // P06 physical 3: Accessed (i64)
    pub is_file: u8,    // P06 physical 4: IsFile (i8)
    pub is_dir: u8,     // P06 physical 5: IsDir (i8)
    pub is_symlink: u8, // P06 physical 6: IsSymlink (i8)
    pub readonly: u8,   // P06 physical 7: Readonly (i8)
}

// ============================================================================
// File Operation Guard - Concurrency Limiter
// ============================================================================

/// RAII guard for tracking active file operations.
struct FileOpGuard;

impl FileOpGuard {
    /// Try to acquire a file operation slot.
    fn acquire() -> Result<Self, String> {
        let current = ACTIVE_FILE_OPS.fetch_add(1, Ordering::Relaxed);
        if current >= MAX_CONCURRENT_FILE_OPS {
            ACTIVE_FILE_OPS.fetch_sub(1, Ordering::Relaxed);
            return Err(format!(
                "Too many concurrent file operations (limit: {})",
                MAX_CONCURRENT_FILE_OPS
            ));
        }
        Ok(FileOpGuard)
    }
}

impl Drop for FileOpGuard {
    fn drop(&mut self) {
        ACTIVE_FILE_OPS.fetch_sub(1, Ordering::Relaxed);
    }
}

// ============================================================================
// Path Validation - Single Source of Truth for Security
// ============================================================================

/// Get the base directory. Falls back to CWD if not explicitly set.
fn get_base_dir() -> PathBuf {
    if let Some(base) = BASE_DIR.get() {
        base.clone()
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }
}

/// Returns true if doo_file_init() was called (explicit sandbox).
/// When false, sandbox checks are skipped — allows access to any valid path.
#[inline]
fn is_sandboxed() -> bool {
    BASE_DIR.get().is_some()
}

/// Validate path length
#[inline]
fn validate_path_length(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("Empty path provided".to_string());
    }
    if path.len() > MAX_PATH_LENGTH {
        return Err(format!(
            "Path too long ({} chars, max {})",
            path.len(),
            MAX_PATH_LENGTH
        ));
    }
    Ok(())
}

/// Validate path doesn't contain dangerous patterns.
fn validate_path_safety(path: &str) -> Result<(), String> {
    if path.contains('\0') {
        return Err("Path contains null byte".to_string());
    }
    if path.starts_with("\\\\") || path.starts_with("//") {
        return Err("Network/UNC paths are not allowed".to_string());
    }
    #[cfg(windows)]
    {
        let after_drive = if path.len() >= 2
            && path.as_bytes()[0].is_ascii_alphabetic()
            && path.as_bytes()[1] == b':'
        {
            &path[2..]
        } else {
            path
        };
        if after_drive.contains(':') {
            return Err("Alternate data streams (NTFS ADS) are not allowed".to_string());
        }
    }
    Ok(())
}

/// Resolve and validate a path for existing files/directories.
fn resolve_safe_path(user_path: &str) -> Result<PathBuf, String> {
    validate_path_length(user_path)?;
    validate_path_safety(user_path)?;

    let base = get_base_dir();
    let resolved = if Path::new(user_path).is_absolute() {
        PathBuf::from(user_path)
    } else {
        base.join(user_path)
    };

    let canonical = fs::canonicalize(&resolved)
        .map_err(|e| format!("Path resolution failed for '{}': {}", user_path, e))?;

    // Only enforce sandbox when doo_file_init() was explicitly called
    if is_sandboxed() {
        let canonical_base = fs::canonicalize(&base).unwrap_or_else(|_| base.clone());

        if !canonical.starts_with(&canonical_base) {
            return Err(format!(
                "Access denied: path '{}' is outside the allowed directory",
                user_path
            ));
        }
    }

    Ok(canonical)
}

/// Resolve and validate a path for file/directory creation.
fn resolve_safe_path_for_create(user_path: &str) -> Result<PathBuf, String> {
    validate_path_length(user_path)?;
    validate_path_safety(user_path)?;

    let base = get_base_dir();
    let resolved = if Path::new(user_path).is_absolute() {
        PathBuf::from(user_path)
    } else {
        base.join(user_path)
    };

    let parent = resolved
        .parent()
        .ok_or_else(|| format!("Invalid path (no parent): '{}'", user_path))?;

    let canonical_parent = if parent.exists() {
        fs::canonicalize(parent)
            .map_err(|e| format!("Parent directory resolution failed: {}", e))?
    } else {
        let mut ancestor = parent.to_path_buf();
        loop {
            if ancestor.exists() {
                break fs::canonicalize(&ancestor)
                    .map_err(|e| format!("Ancestor directory resolution failed: {}", e))?;
            }
            if !ancestor.pop() {
                return Err("No valid ancestor directory found".to_string());
            }
        }
    };

    // Only enforce sandbox when doo_file_init() was explicitly called
    if is_sandboxed() {
        let canonical_base = fs::canonicalize(&base).unwrap_or_else(|_| base.clone());

        if !canonical_parent.starts_with(&canonical_base) {
            return Err(format!(
                "Access denied: path '{}' would be outside the allowed directory",
                user_path
            ));
        }
    }

    let filename = resolved
        .file_name()
        .ok_or_else(|| format!("Invalid path (no filename): '{}'", user_path))?;

    Ok(canonical_parent.join(filename))
}

/// Resolve a path for mkdir_all
fn resolve_safe_path_for_mkdir(user_path: &str) -> Result<PathBuf, String> {
    validate_path_length(user_path)?;
    validate_path_safety(user_path)?;

    let base = get_base_dir();
    let resolved = if Path::new(user_path).is_absolute() {
        PathBuf::from(user_path)
    } else {
        base.join(user_path)
    };

    // Only enforce sandbox when doo_file_init() was explicitly called
    if is_sandboxed() {
        let mut ancestor = resolved.clone();
        let canonical_ancestor = loop {
            if ancestor.exists() {
                break fs::canonicalize(&ancestor)
                    .map_err(|e| format!("Ancestor resolution failed: {}", e))?;
            }
            if !ancestor.pop() {
                return Err("No valid ancestor directory found".to_string());
            }
        };

        let canonical_base = fs::canonicalize(&base).unwrap_or_else(|_| base.clone());

        if !canonical_ancestor.starts_with(&canonical_base) {
            return Err(format!(
                "Access denied: path '{}' would be outside the allowed directory",
                user_path
            ));
        }
    }

    Ok(resolved)
}

// ============================================================================
// Helper Functions
// c_to_string, make_ok_string, make_ok_void, make_ok_int, safe_ffi
// all imported from doo_ffi_core::helpers (single source of truth)
// ============================================================================

/// Allocate FileError struct with message
#[inline]
fn alloc_file_error(message: &str) -> *mut DooFileError {
    unsafe {
        let err_ptr = doo_alloc(std::mem::size_of::<DooFileError>()) as *mut DooFileError;
        if err_ptr.is_null() {
            return std::ptr::null_mut();
        }
        (*err_ptr).message = doo_alloc_string(message);
        err_ptr
    }
}

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

#[inline]
fn make_err(message: &str) -> *mut DooResult {
    doo_ffi_core::helpers::make_err_rfc7807(500, message)
}

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

// safe_ffi imported from doo_ffi_core::helpers (single source of truth)

// ============================================================================
// Platform-Specific Helpers
// ============================================================================

#[cfg(unix)]
fn create_file_with_permissions(path: &Path, content: &str) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(content.as_bytes())
}

#[cfg(not(unix))]
fn create_file_with_permissions(path: &Path, content: &str) -> std::io::Result<()> {
    fs::write(path, content)
}

#[cfg(unix)]
fn create_dir_with_permissions(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn create_dir_with_permissions(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)
}

#[cfg(unix)]
fn append_file_with_permissions(path: &Path, content: &str) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(content.as_bytes())
}

#[cfg(not(unix))]
fn append_file_with_permissions(path: &Path, content: &str) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(content.as_bytes())
}

/// Atomic write: write to temp file then rename for data integrity
fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    let tmp_path = path.with_extension("doo_tmp");
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        fs::write(&tmp_path, content)?;
    }
    match fs::rename(&tmp_path, path) {
        Ok(_) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp_path);
            Err(e)
        }
    }
}

// ============================================================================
// INITIALIZATION
// ============================================================================

/// Initialize the file system sandbox.
#[no_mangle]
pub extern "C" fn doo_file_init(base_dir: *const c_char) -> *mut DooResult {
    safe_ffi("File", || {
        let dir_str = match c_to_string(base_dir) {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let canonical = match fs::canonicalize(&dir_str) {
            Ok(p) => p,
            Err(e) => return make_err(&format!("Base directory '{}' invalid: {}", dir_str, e)),
        };
        if !canonical.is_dir() {
            return make_err(&format!("Base path '{}' must be a directory", dir_str));
        }
        match BASE_DIR.set(canonical) {
            Ok(_) => make_ok_void(),
            Err(_) => make_err("File system already initialized"),
        }
    })
}

// ============================================================================
// BASIC FILE I/O
// ============================================================================

/// Read entire file contents as string. Enforces file size limit.
#[no_mangle]
pub extern "C" fn doo_file_read(path: *const c_char) -> *mut DooResult {
    safe_ffi("File", || {
        let path_str = match c_to_string(path) {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let safe_path = match resolve_safe_path(&path_str) {
            Ok(p) => p,
            Err(e) => return make_err(&e),
        };
        let _guard = match FileOpGuard::acquire() {
            Ok(g) => g,
            Err(e) => return make_err(&e),
        };
        match fs::metadata(&safe_path) {
            Ok(meta) => {
                if meta.len() > MAX_FILE_READ_SIZE {
                    return make_err(&format!(
                        "File too large: {} bytes (max {} MB)",
                        meta.len(),
                        MAX_FILE_READ_SIZE / (1024 * 1024)
                    ));
                }
            }
            Err(e) => return make_err(&format!("Failed to stat '{}': {}", path_str, e)),
        }
        match fs::read_to_string(&safe_path) {
            Ok(content) => make_ok_string(&content),
            Err(e) => make_err(&format!("Failed to read file '{}': {}", path_str, e)),
        }
    })
}

/// Write string content to file. Uses atomic write for data integrity.
#[no_mangle]
pub extern "C" fn doo_file_write(path: *const c_char, content: *const c_char) -> *mut DooResult {
    safe_ffi("File", || {
        let path_str = match c_to_string(path) {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let content_str = match c_to_string(content) {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        if content_str.len() > MAX_FILE_WRITE_SIZE {
            return make_err(&format!(
                "Content too large: {} bytes (max {} MB)",
                content_str.len(),
                MAX_FILE_WRITE_SIZE / (1024 * 1024)
            ));
        }
        let safe_path = match resolve_safe_path_for_create(&path_str) {
            Ok(p) => p,
            Err(e) => return make_err(&e),
        };
        let _guard = match FileOpGuard::acquire() {
            Ok(g) => g,
            Err(e) => return make_err(&e),
        };
        match atomic_write(&safe_path, &content_str) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err(&format!("Failed to write file '{}': {}", path_str, e)),
        }
    })
}

/// Append string content to file (creates if not exists).
#[no_mangle]
pub extern "C" fn doo_file_append(path: *const c_char, content: *const c_char) -> *mut DooResult {
    safe_ffi("File", || {
        let path_str = match c_to_string(path) {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let content_str = match c_to_string(content) {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        if content_str.len() > MAX_FILE_WRITE_SIZE {
            return make_err(&format!(
                "Content too large: {} bytes (max {} MB)",
                content_str.len(),
                MAX_FILE_WRITE_SIZE
            ));
        }
        let safe_path =
            resolve_safe_path(&path_str).or_else(|_| resolve_safe_path_for_create(&path_str));
        let safe_path = match safe_path {
            Ok(p) => p,
            Err(e) => return make_err(&e),
        };
        let _guard = match FileOpGuard::acquire() {
            Ok(g) => g,
            Err(e) => return make_err(&e),
        };
        match append_file_with_permissions(&safe_path, &content_str) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err(&format!("Failed to append to file '{}': {}", path_str, e)),
        }
    })
}

/// Delete a file.
#[no_mangle]
pub extern "C" fn doo_file_delete(path: *const c_char) -> *mut DooResult {
    safe_ffi("File", || {
        let path_str = match c_to_string(path) {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let safe_path = match resolve_safe_path(&path_str) {
            Ok(p) => p,
            Err(e) => return make_err(&e),
        };
        let _guard = match FileOpGuard::acquire() {
            Ok(g) => g,
            Err(e) => return make_err(&e),
        };
        match fs::remove_file(&safe_path) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err(&format!("Failed to delete file '{}': {}", path_str, e)),
        }
    })
}

/// Copy a file from source to destination.
#[no_mangle]
pub extern "C" fn doo_file_copy(src: *const c_char, dst: *const c_char) -> *mut DooResult {
    safe_ffi("File", || {
        let src_str = match c_to_string(src) {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let dst_str = match c_to_string(dst) {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let safe_src = match resolve_safe_path(&src_str) {
            Ok(p) => p,
            Err(e) => return make_err(&e),
        };
        let safe_dst = match resolve_safe_path_for_create(&dst_str) {
            Ok(p) => p,
            Err(e) => return make_err(&e),
        };
        match fs::metadata(&safe_src) {
            Ok(meta) => {
                if meta.len() > MAX_FILE_READ_SIZE {
                    return make_err(&format!(
                        "Source file too large for copy: {} bytes (max {} MB)",
                        meta.len(),
                        MAX_FILE_READ_SIZE / (1024 * 1024)
                    ));
                }
            }
            Err(e) => return make_err(&format!("Failed to stat source '{}': {}", src_str, e)),
        }
        let _guard = match FileOpGuard::acquire() {
            Ok(g) => g,
            Err(e) => return make_err(&e),
        };
        match fs::copy(&safe_src, &safe_dst) {
            Ok(_) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(&safe_dst, fs::Permissions::from_mode(0o600));
                }
                make_ok_void()
            }
            Err(e) => make_err(&format!(
                "Failed to copy '{}' to '{}': {}",
                src_str, dst_str, e
            )),
        }
    })
}

/// Move/rename a file from source to destination.
#[no_mangle]
pub extern "C" fn doo_file_move(src: *const c_char, dst: *const c_char) -> *mut DooResult {
    safe_ffi("File", || {
        let src_str = match c_to_string(src) {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let dst_str = match c_to_string(dst) {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let safe_src = match resolve_safe_path(&src_str) {
            Ok(p) => p,
            Err(e) => return make_err(&e),
        };
        let safe_dst = match resolve_safe_path_for_create(&dst_str) {
            Ok(p) => p,
            Err(e) => return make_err(&e),
        };
        let _guard = match FileOpGuard::acquire() {
            Ok(g) => g,
            Err(e) => return make_err(&e),
        };
        match fs::rename(&safe_src, &safe_dst) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err(&format!(
                "Failed to move '{}' to '{}': {}",
                src_str, dst_str, e
            )),
        }
    })
}

/// Read file contents (same as doo_file_read - single source of truth).
#[no_mangle]
pub extern "C" fn doo_file_read_lines(path: *const c_char) -> *mut DooResult {
    doo_file_read(path)
}

// ============================================================================
// DIRECTORY OPERATIONS
// ============================================================================

/// Create directory (and parents). Restrictive permissions on Unix.
#[no_mangle]
pub extern "C" fn doo_file_mkdir(path: *const c_char) -> *mut DooResult {
    safe_ffi("File", || {
        let path_str = match c_to_string(path) {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let safe_path = match resolve_safe_path_for_mkdir(&path_str) {
            Ok(p) => p,
            Err(e) => return make_err(&e),
        };
        let _guard = match FileOpGuard::acquire() {
            Ok(g) => g,
            Err(e) => return make_err(&e),
        };
        match create_dir_with_permissions(&safe_path) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err(&format!("Failed to create directory '{}': {}", path_str, e)),
        }
    })
}

/// Create directory and all parents (delegates to mkdir).
#[no_mangle]
pub extern "C" fn doo_file_mkdir_all(path: *const c_char) -> *mut DooResult {
    doo_file_mkdir(path)
}

/// Remove empty directory.
#[no_mangle]
pub extern "C" fn doo_file_rmdir(path: *const c_char) -> *mut DooResult {
    safe_ffi("File", || {
        let path_str = match c_to_string(path) {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let safe_path = match resolve_safe_path(&path_str) {
            Ok(p) => p,
            Err(e) => return make_err(&e),
        };
        let _guard = match FileOpGuard::acquire() {
            Ok(g) => g,
            Err(e) => return make_err(&e),
        };
        match fs::remove_dir(&safe_path) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err(&format!("Failed to remove directory '{}': {}", path_str, e)),
        }
    })
}

/// Remove directory and all contents recursively.
/// Cannot delete the base directory itself.
#[no_mangle]
pub extern "C" fn doo_file_rmdir_all(path: *const c_char) -> *mut DooResult {
    safe_ffi("File", || {
        let path_str = match c_to_string(path) {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let safe_path = match resolve_safe_path(&path_str) {
            Ok(p) => p,
            Err(e) => return make_err(&e),
        };
        let base = get_base_dir();
        let canonical_base = fs::canonicalize(&base).unwrap_or_else(|_| base.clone());
        if safe_path == canonical_base {
            return make_err("Cannot delete the base directory");
        }
        let _guard = match FileOpGuard::acquire() {
            Ok(g) => g,
            Err(e) => return make_err(&e),
        };
        match fs::remove_dir_all(&safe_path) {
            Ok(_) => make_ok_void(),
            Err(e) => make_err(&format!(
                "Failed to remove directory tree '{}': {}",
                path_str, e
            )),
        }
    })
}

/// List directory contents (comma-separated). Limited to MAX_DIR_ENTRIES.
#[no_mangle]
pub extern "C" fn doo_file_list_dir(path: *const c_char) -> *mut DooResult {
    safe_ffi("File", || {
        let path_str = match c_to_string(path) {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let safe_path = match resolve_safe_path(&path_str) {
            Ok(p) => p,
            Err(e) => return make_err(&e),
        };
        let _guard = match FileOpGuard::acquire() {
            Ok(g) => g,
            Err(e) => return make_err(&e),
        };
        match fs::read_dir(&safe_path) {
            Ok(entries) => {
                let names: Vec<String> = entries
                    .take(MAX_DIR_ENTRIES)
                    .filter_map(|e| e.ok())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect();
                make_ok_string(&names.join(","))
            }
            Err(e) => make_err(&format!("Failed to list directory '{}': {}", path_str, e)),
        }
    })
}

// ============================================================================
// FILE METADATA OPERATIONS
// ============================================================================

/// Check if path exists. Returns i32 (0=false, 1=true).
#[no_mangle]
pub extern "C" fn doo_file_exists(path: *const c_char) -> i32 {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let path_str = match c_to_string(path) {
            Ok(s) => s,
            Err(_) => return 0,
        };
        let safe_path = match resolve_safe_path(&path_str) {
            Ok(p) => p,
            Err(_) => match resolve_safe_path_for_create(&path_str) {
                Ok(p) => {
                    if p.exists() {
                        return 1;
                    }
                    return 0;
                }
                Err(_) => return 0,
            },
        };
        if safe_path.exists() {
            1
        } else {
            0
        }
    })) {
        Ok(r) => r,
        Err(_) => 0,
    }
}

/// Get file size in bytes.
#[no_mangle]
pub extern "C" fn doo_file_size(path: *const c_char) -> *mut DooResult {
    safe_ffi("File", || {
        let path_str = match c_to_string(path) {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let safe_path = match resolve_safe_path(&path_str) {
            Ok(p) => p,
            Err(e) => return make_err(&e),
        };
        match fs::metadata(&safe_path) {
            Ok(meta) => make_ok_int(meta.len() as i64),
            Err(e) => make_err(&format!("Failed to get size of '{}': {}", path_str, e)),
        }
    })
}

/// Get comprehensive file/directory metadata.
/// Uses symlink_metadata for correct symlink detection.
#[no_mangle]
pub extern "C" fn doo_file_metadata(path: *const c_char) -> *mut DooResult {
    safe_ffi("File", || {
        let path_str = match c_to_string(path) {
            Ok(s) => s,
            Err(e) => return make_err(&e),
        };
        let safe_path = match resolve_safe_path(&path_str) {
            Ok(p) => p,
            Err(e) => return make_err(&e),
        };
        // Use symlink_metadata for correct symlink detection
        let is_symlink = match fs::symlink_metadata(&safe_path) {
            Ok(m) => m.file_type().is_symlink(),
            Err(_) => false,
        };
        match fs::metadata(&safe_path) {
            Ok(meta) => {
                let file_meta = DooFileMetadata {
                    size: meta.len() as i64,
                    created: to_timestamp(meta.created()),
                    modified: to_timestamp(meta.modified()),
                    accessed: to_timestamp(meta.accessed()),
                    is_file: if meta.is_file() { 1 } else { 0 },
                    is_dir: if meta.is_dir() { 1 } else { 0 },
                    is_symlink: if is_symlink { 1 } else { 0 },
                    readonly: if meta.permissions().readonly() { 1 } else { 0 },
                };
                make_ok_metadata(file_meta)
            }
            Err(e) => make_err(&format!("Failed to get metadata for '{}': {}", path_str, e)),
        }
    })
}

/// Check if path is a file. Returns i32 (0=false, 1=true).
#[no_mangle]
pub extern "C" fn doo_file_is_file(path: *const c_char) -> i32 {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let path_str = match c_to_string(path) {
            Ok(s) => s,
            Err(_) => return 0,
        };
        let safe_path = match resolve_safe_path(&path_str) {
            Ok(p) => p,
            Err(_) => return 0,
        };
        if safe_path.is_file() {
            1
        } else {
            0
        }
    })) {
        Ok(result) => result,
        Err(_) => 0,
    }
}

/// Check if path is a directory. Returns i32 (0=false, 1=true).
#[no_mangle]
pub extern "C" fn doo_file_is_dir(path: *const c_char) -> i32 {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let path_str = match c_to_string(path) {
            Ok(s) => s,
            Err(_) => return 0,
        };
        let safe_path = match resolve_safe_path(&path_str) {
            Ok(p) => p,
            Err(_) => return 0,
        };
        if safe_path.is_dir() {
            1
        } else {
            0
        }
    })) {
        Ok(result) => result,
        Err(_) => 0,
    }
}

/// Get file modification time as Unix timestamp. Returns -1 on error.
#[no_mangle]
pub extern "C" fn doo_file_modified_time(path: *const c_char) -> i64 {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let path_str = match c_to_string(path) {
            Ok(s) => s,
            Err(_) => return -1i64,
        };
        let safe_path = match resolve_safe_path(&path_str) {
            Ok(p) => p,
            Err(_) => return -1i64,
        };
        fs::metadata(&safe_path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(-1i64)
    })) {
        Ok(result) => result,
        Err(_) => -1,
    }
}

// ============================================================================
// MEMORY MANAGEMENT
// ============================================================================

/// Free a DooResult. All allocations use libc::malloc, all frees use libc::free.
#[no_mangle]
pub extern "C" fn doo_file_free_result(result: *mut DooResult) {
    if result.is_null() {
        return;
    }
    unsafe {
        let tag = (*result).tag;
        let data = (*result).data;
        if !data.is_null() {
            if tag == 1 {
                let inner_str = *(data as *const *mut c_void);
                if !inner_str.is_null() {
                    libc::free(inner_str);
                }
            }
            libc::free(data as *mut c_void);
        }
        libc::free(result as *mut c_void);
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
        let write_result = doo_file_write(path.as_ptr(), content.as_ptr());
        assert!(!write_result.is_null());
        unsafe {
            assert_eq!((*write_result).tag, 0);
            doo_file_free_result(write_result);
        }
        assert_eq!(doo_file_exists(path.as_ptr()), 1);
        let read_result = doo_file_read(path.as_ptr());
        assert!(!read_result.is_null());
        unsafe {
            assert_eq!((*read_result).tag, 0);
            doo_file_free_result(read_result);
        }
        let del_result = doo_file_delete(path.as_ptr());
        if !del_result.is_null() {
            unsafe {
                doo_file_free_result(del_result);
            }
        }
    }

    #[test]
    fn test_file_metadata_symlink_detection() {
        let path = CString::new("test_metadata_sym.txt").unwrap();
        let content = CString::new("Metadata symlink test").unwrap();
        let write_result = doo_file_write(path.as_ptr(), content.as_ptr());
        unsafe {
            doo_file_free_result(write_result);
        }
        let meta_result = doo_file_metadata(path.as_ptr());
        assert!(!meta_result.is_null());
        unsafe {
            assert_eq!((*meta_result).tag, 0);
            let meta_ptr = (*meta_result).data as *const DooFileMetadata;
            assert!(!meta_ptr.is_null());
            assert_eq!((*meta_ptr).is_file, 1);
            assert_eq!((*meta_ptr).is_dir, 0);
            assert_eq!((*meta_ptr).is_symlink, 0);
            doo_file_free_result(meta_result);
        }
        let del_result = doo_file_delete(path.as_ptr());
        if !del_result.is_null() {
            unsafe {
                doo_file_free_result(del_result);
            }
        }
    }

    #[test]
    fn test_path_validation() {
        assert!(validate_path_safety("file\0.txt").is_err());
        assert!(validate_path_safety("\\\\server\\share").is_err());
        assert!(validate_path_safety("//server/share").is_err());
        assert!(validate_path_length(&"a".repeat(MAX_PATH_LENGTH + 1)).is_err());
        assert!(validate_path_length("").is_err());
    }

    #[test]
    fn test_file_move() {
        let src = CString::new("test_move_src.txt").unwrap();
        let dst = CString::new("test_move_dst.txt").unwrap();
        let content = CString::new("Move test").unwrap();
        let write_result = doo_file_write(src.as_ptr(), content.as_ptr());
        unsafe {
            doo_file_free_result(write_result);
        }
        let move_result = doo_file_move(src.as_ptr(), dst.as_ptr());
        assert!(!move_result.is_null());
        unsafe {
            assert_eq!((*move_result).tag, 0);
            doo_file_free_result(move_result);
        }
        assert_eq!(doo_file_exists(src.as_ptr()), 0);
        assert_eq!(doo_file_exists(dst.as_ptr()), 1);
        let del_result = doo_file_delete(dst.as_ptr());
        if !del_result.is_null() {
            unsafe {
                doo_file_free_result(del_result);
            }
        }
    }

    #[test]
    fn test_concurrent_guard() {
        let guard = FileOpGuard::acquire();
        assert!(guard.is_ok());
        drop(guard);
    }
}
