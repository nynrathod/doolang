use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Global allocation counter for tracking memory operations
static ALLOC_COUNTER: AtomicU64 = AtomicU64::new(0);
static FREE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Global set to track all freed pointers - helps detect use-after-free and double-free
/// Key: pointer address
static FREED_POINTERS: std::sync::LazyLock<Mutex<HashSet<usize>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));

/// Global set to track all allocated pointers - helps detect free of unallocated memory
static ALLOCATED_POINTERS: std::sync::LazyLock<Mutex<HashSet<usize>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));

/// Maximum number of tracked pointers (to prevent memory bloat)
const MAX_TRACKED_POINTERS: usize = 100_000;

#[inline]
fn tracking_enabled() -> bool {
    true
}

/// Get next allocation ID (monotonically increasing)
pub fn next_alloc_id() -> u64 {
    ALLOC_COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// Get next free ID (monotonically increasing)
pub fn next_free_id() -> u64 {
    FREE_COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// Get current allocation stats (alloc_count, free_count)
pub fn get_alloc_stats() -> (u64, u64) {
    (
        ALLOC_COUNTER.load(Ordering::SeqCst),
        FREE_COUNTER.load(Ordering::SeqCst),
    )
}

/// Track a pointer as allocated - returns false if already allocated (double-alloc)
pub fn track_alloc(ptr: *const std::ffi::c_void, context: &str) -> bool {
    if !tracking_enabled() {
        return true;
    }
    if ptr.is_null() {
        return true;
    }
    let addr = ptr as usize;
    let alloc_id = next_alloc_id();

    if let Ok(mut set) = ALLOCATED_POINTERS.lock() {
        // Remove from freed set (it's now re-allocated)
        if let Ok(mut freed) = FREED_POINTERS.lock() {
            if freed.remove(&addr) && crate::debug::is_debug_enabled() {
                eprintln!(
                    "[DOO::MEM] REALLOC  #{:06} ptr={:p} context={} (was freed, now reallocated)",
                    alloc_id, ptr, context
                );
            }
        }

        // Prevent memory bloat
        if set.len() >= MAX_TRACKED_POINTERS {
            set.clear();
        }

        if !set.insert(addr) {
            if crate::debug::is_debug_enabled() {
                eprintln!("[DOO::MEM] DOUBLE-ALLOC ptr={:p} context={}", ptr, context);
            }
            return false;
        } else {
            // Log successful allocation
            if crate::debug::is_debug_enabled() {
                eprintln!(
                    "[DOO::MEM] ALLOC   #{:06} ptr={:p} context={}",
                    alloc_id, ptr, context
                );
            }
        }
    }
    true
}

/// Track a pointer as freed - returns true if this is a DOUBLE-FREE
pub fn track_free(ptr: *const std::ffi::c_void, context: &str) -> bool {
    if !tracking_enabled() {
        return false;
    }
    if ptr.is_null() {
        return false;
    }
    let addr = ptr as usize;

    // Check if this was a valid allocation
    let was_allocd = if let Ok(mut set) = ALLOCATED_POINTERS.lock() {
        set.remove(&addr)
    } else {
        true // assume valid if we can't check
    };

    if !was_allocd {
        if crate::debug::is_debug_enabled() {
            eprintln!(
                "[DOO::MEM] FREE-OF-UNALLOCATED ptr={:p} context={}",
                ptr, context
            );
        }
    }

    if let Ok(mut set) = FREED_POINTERS.lock() {
        // Prevent memory bloat
        if set.len() >= MAX_TRACKED_POINTERS {
            set.clear();
        }

        if !set.insert(addr) {
            if crate::debug::is_debug_enabled() {
                eprintln!("[DOO::MEM] DOUBLE-FREE ptr={:p} context={}", ptr, context);
            }
            return true;
        }
    }
    false
}

/// Check if a pointer was already freed (use-after-free detection)
pub fn is_freed(ptr: *const std::ffi::c_void) -> bool {
    if !tracking_enabled() {
        return false;
    }
    if ptr.is_null() {
        return false;
    }
    let addr = ptr as usize;

    if let Ok(set) = FREED_POINTERS.lock() {
        let was_freed = set.contains(&addr);
        if was_freed && crate::debug::is_debug_enabled() {
            eprintln!(
                "[DOO::MEM] USE-AFTER-FREE detected! ptr={:p} was already freed",
                ptr
            );
        }
        return was_freed;
    }
    false
}

/// Heap canary value for detecting buffer overflows
pub const HEAP_CANARY: u64 = 0xDEADBEEFCAFEBABE;

/// Validate heap canary at a given pointer offset
#[inline]
pub fn validate_heap_canary(ptr: *const u64, context: &str) -> bool {
    if ptr.is_null() {
        return true;
    }

    unsafe {
        let canary = *ptr;
        if canary != HEAP_CANARY {
            if crate::debug::is_debug_enabled() {
                eprintln!(
                    "[DOO::MEM] HEAP-CORRUPTION canary={:016x} expected={:016x} ptr={:p} context={}",
                    canary, HEAP_CANARY, ptr, context
                );
            }
            return false;
        }
    }
    true
}

/// Validate a pointer looks sane (not obviously corrupted)
/// Returns true if pointer looks valid, false if obviously corrupt
#[inline]
pub fn validate_pointer(ptr: *const std::ffi::c_void, context: &str) -> bool {
    if ptr.is_null() {
        return true;
    }

    let addr = ptr as usize;

    if addr < 0x1000 {
        if crate::debug::is_debug_enabled() {
            eprintln!(
                "[DOO::MEM] CORRUPT ptr={:p} (too low) context={}",
                ptr, context
            );
        }
        return false;
    }

    if addr % 4 != 0 {
        if crate::debug::is_debug_enabled() {
            eprintln!(
                "[DOO::MEM] CORRUPT ptr={:p} (misaligned) context={}",
                ptr, context
            );
        }
        return false;
    }

    #[cfg(unix)]
    {
        unsafe {
            let page_size = libc::sysconf(libc::_SC_PAGESIZE);
            if page_size > 0 {
                let page_size = page_size as usize;
                let page_base = (addr / page_size) * page_size;
                let mut vec: [u8; 1] = [0];
                #[cfg(target_os = "macos")]
                let rc = libc::mincore(
                    page_base as *mut libc::c_void,
                    page_size,
                    vec.as_mut_ptr() as *mut i8,
                );
                #[cfg(not(target_os = "macos"))]
                let rc = libc::mincore(
                    page_base as *mut libc::c_void,
                    page_size,
                    vec.as_mut_ptr(),
                );
                if rc != 0 {
                    if crate::debug::is_debug_enabled() {
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
