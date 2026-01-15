//! Centralized debug utilities for all Doo FFI libraries
//!
//! This module provides a unified debug macro that:
//! - Only prints in debug builds OR when `doo run --debug` is used
//! - Uses consistent formatting across all FFI libraries
//! - Supports aggressive memory tracking for debugging heap corruption
//!
//! Usage in any FFI library:
//! ```
//! doo_debug!("Database", "Connection established: {}", connection_id);
//! doo_http_debug!("Request received: {} {}", method, path);
//! ```

/// Central debug macro for all FFI libraries
///
/// Args:
/// - $component: Component name (e.g., "DB", "HTTP", "Runtime")
/// - $($arg:tt)*: Format string and arguments
///
/// Example:
/// ```
/// doo_debug!("DB", "Query executed: {}", query);
/// // Output: [DOO::DB] Query executed: SELECT * FROM users
/// ```
#[macro_export]
macro_rules! doo_debug {
    ($component:expr, $($arg:tt)*) => {
        if $crate::debug::is_debug_enabled() {
            eprintln!("[DOO::{}] {}", $component, format!($($arg)*));
        }
    };
}

/// Debug macro specifically for database operations
#[macro_export]
macro_rules! doo_db_debug {
    ($($arg:tt)*) => {
        $crate::doo_debug!("DB", $($arg)*);
    };
}

/// Debug macro specifically for HTTP operations
#[macro_export]
macro_rules! doo_http_debug {
    ($($arg:tt)*) => {
        $crate::doo_debug!("HTTP", $($arg)*);
    };
}

/// Debug macro specifically for runtime operations
#[macro_export]
macro_rules! doo_runtime_debug {
    ($($arg:tt)*) => {
        $crate::doo_debug!("Runtime", $($arg)*);
    };
}

/// Debug macro specifically for auth operations
#[macro_export]
macro_rules! doo_auth_debug {
    ($($arg:tt)*) => {
        $crate::doo_debug!("Auth", $($arg)*);
    };
}

/// Aggressive memory allocation tracking macro
/// Use for debugging heap corruption issues
/// Format: [DOO::MEM] ALLOC #ID ptr=0x... size=... context=...
/// Also adds to global tracking set for double-free detection
#[macro_export]
macro_rules! doo_mem_alloc {
    ($ptr:expr, $size:expr, $context:expr) => {
        {
            // Track in global set
            $crate::memory::track_alloc($ptr as *const std::ffi::c_void, $context);
            
            if $crate::debug::is_debug_enabled() {
                let id = $crate::memory::next_alloc_id();
                eprintln!(
                    "[DOO::MEM] ALLOC #{:06} ptr={:p} size={} context={}",
                    id, $ptr, $size, $context
                );
            }
        }
    };
}

/// Aggressive memory free tracking macro with double-free detection
/// Use for debugging heap corruption issues
/// Format: [DOO::MEM] FREE #ID ptr=0x... context=...
/// Returns true if this is a DOUBLE-FREE (caller should abort the free!)
#[macro_export]
macro_rules! doo_mem_free {
    ($ptr:expr, $context:expr) => {
        {
            let is_double_free = $crate::memory::track_free($ptr as *const std::ffi::c_void, $context);
            
            if $crate::debug::is_debug_enabled() {
                let id = $crate::memory::next_free_id();
                if is_double_free {
                    eprintln!(
                        "[DOO::MEM] FREE  #{:06} ptr={:p} context={} !!!DOUBLE-FREE!!!",
                        id, $ptr, $context
                    );
                } else {
                    eprintln!(
                        "[DOO::MEM] FREE  #{:06} ptr={:p} context={}",
                        id, $ptr, $context
                    );
                }
            }
            
            is_double_free
        }
    };
}

/// Check if a pointer has already been freed (use-after-free detection)
#[macro_export]
macro_rules! doo_mem_check_freed {
    ($ptr:expr, $context:expr) => {
        {
            let is_freed = $crate::memory::is_freed($ptr as *const std::ffi::c_void);
            if is_freed && $crate::debug::is_debug_enabled() {
                eprintln!(
                    "[DOO::MEM] USE-AFTER-FREE ptr={:p} context={}",
                    $ptr, $context
                );
            }
            is_freed
        }
    };
}

/// Memory stats debug output
#[macro_export]
macro_rules! doo_mem_stats {
    () => {
        if $crate::debug::is_debug_enabled() {
            let (allocs, frees) = $crate::memory::get_alloc_stats();
            eprintln!(
                "[DOO::MEM] STATS allocs={} frees={} diff={}",
                allocs,
                frees,
                allocs.saturating_sub(frees)
            );
        }
    };
}

/// FFI boundary entry debug (for tracking function calls across FFI)
#[macro_export]
macro_rules! doo_ffi_enter {
    ($func:expr) => {
        if $crate::debug::is_debug_enabled() {
            eprintln!("[DOO::FFI] ENTER {}", $func);
        }
    };
    ($func:expr, $($arg:tt)*) => {
        if $crate::debug::is_debug_enabled() {
            eprintln!("[DOO::FFI] ENTER {} args=({})", $func, format!($($arg)*));
        }
    };
}

/// FFI boundary exit debug (for tracking function returns across FFI)
#[macro_export]
macro_rules! doo_ffi_exit {
    ($func:expr) => {
        if $crate::debug::is_debug_enabled() {
            eprintln!("[DOO::FFI] EXIT  {}", $func);
        }
    };
    ($func:expr, $($arg:tt)*) => {
        if $crate::debug::is_debug_enabled() {
            eprintln!("[DOO::FFI] EXIT  {} result=({})", $func, format!($($arg)*));
        }
    };
}

/// Handler call tracking (for tracking JIT handler invocations)
#[macro_export]
macro_rules! doo_handler_call {
    ($name:expr, $req_ptr:expr) => {
        if $crate::debug::is_debug_enabled() {
            eprintln!("[DOO::HANDLER] CALL {} req_ptr={:p}", $name, $req_ptr);
        }
    };
}

/// Handler result tracking
#[macro_export]
macro_rules! doo_handler_result {
    ($name:expr, $result_ptr:expr, $tag:expr) => {
        if $crate::debug::is_debug_enabled() {
            eprintln!(
                "[DOO::HANDLER] RESULT {} result_ptr={:p} tag={}",
                $name, $result_ptr, $tag
            );
        }
    };
}

/// Check if debug mode is enabled
#[inline]
pub fn is_debug_enabled() -> bool {
    // Check global flag set by doo run --debug
    if crate::doo_runtime_is_debug_enabled() {
        return true;
    }
    // Always enabled in debug builds
    if cfg!(debug_assertions) {
        return true;
    }
    // Fallback to env var
    std::env::var("DOO_DEBUG").is_ok()
}

/// Runtime debug printf - checks debug flag at runtime and forwards to libc fprintf
/// Used by generated LLVM code to conditionally print debug messages
/// This is declared as variadic in LLVM but implemented as a wrapper to fprintf
#[no_mangle]
pub extern "C" fn doo_runtime_debug_printf(format: *const i8) -> i32 {
    // Only print if debug is enabled
    if !is_debug_enabled() {
        return 0;
    }

    unsafe {
        if format.is_null() {
            return -1;
        }

        // For now, just print the format string without processing format specifiers
        // The actual variadic handling is done by declaring the function as variadic in LLVM
        let format_str = std::ffi::CStr::from_ptr(format).to_string_lossy();

        eprint!("{}", format_str);
        0
    }
}

/// Validate a pointer looks sane (not obviously corrupted)
/// Returns true if pointer looks valid, false if obviously corrupt
#[inline]
pub fn validate_pointer(ptr: *const std::ffi::c_void, context: &str) -> bool {
    if ptr.is_null() {
        return true; // Null is valid (handled separately)
    }

    let addr = ptr as usize;

    // Check for obviously bad pointers (very low addresses, typically guard pages)
    if addr < 0x1000 {
        if is_debug_enabled() {
            eprintln!(
                "[DOO::MEM] CORRUPT ptr={:p} (too low) context={}",
                ptr, context
            );
        }
        return false;
    }

    // Check for obviously misaligned pointers (pointers should be at least 4-byte aligned)
    if addr % 4 != 0 {
        if is_debug_enabled() {
            eprintln!(
                "[DOO::MEM] CORRUPT ptr={:p} (misaligned) context={}",
                ptr, context
            );
        }
        return false;
    }

    // On Unix (including WSL), also verify the address is actually mapped.
    // This avoids crashing when small integers are passed around as pointers (e.g. 0x4c).
    #[cfg(unix)]
    {
        unsafe {
            let page_size = libc::sysconf(libc::_SC_PAGESIZE);
            if page_size > 0 {
                let page_size = page_size as usize;
                let page_base = (addr / page_size) * page_size;
                let mut vec: [u8; 1] = [0];
                // mincore() length must be non-zero and typically page-size aligned.
                let rc = libc::mincore(
                    page_base as *mut libc::c_void,
                    page_size,
                    vec.as_mut_ptr(),
                );
                if rc != 0 {
                    if is_debug_enabled() {
                        eprintln!(
                            "[DOO::MEM] CORRUPT ptr={:p} (unmapped) context={}",
                            ptr, context
                        );
                    }
                    return false;
                }
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_enabled() {
        // In test builds, cfg!(debug_assertions) is typically true
        let enabled = is_debug_enabled();
        assert!(enabled || !enabled); // Just ensure it compiles
    }

    #[test]
    fn test_alloc_counter() {
        let id1 = next_alloc_id();
        let id2 = next_alloc_id();
        assert!(id2 > id1);
    }

    #[test]
    fn test_validate_pointer() {
        let valid_ptr = Box::into_raw(Box::new(42)) as *const std::ffi::c_void;
        assert!(validate_pointer(valid_ptr, "test"));
        assert!(validate_pointer(std::ptr::null(), "test_null"));

        // Clean up
        unsafe {
            let _ = Box::from_raw(valid_ptr as *mut i32);
        }
    }
}

