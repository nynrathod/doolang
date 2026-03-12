//! Memory Leak Tracker
//!
//! Runtime allocation tracking for detecting memory leaks in Doo programs.
//! Activated by environment variable `DOO_LEAK_CHECK=1`.
//!
//! When enabled:
//! - Every `doo_alloc` call is recorded with size
//! - Every `doo_free` call removes the record
//! - At process exit, any remaining allocations are reported as leaks
//!
//! Zero overhead when disabled (env var not set).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

// ============================================================================
// Global State
// ============================================================================

/// Whether leak tracking is active (checked once at init, cached)
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Whether we've initialized (prevents double-init)
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Total allocations made
static TOTAL_ALLOCS: AtomicU64 = AtomicU64::new(0);

/// Total frees made
static TOTAL_FREES: AtomicU64 = AtomicU64::new(0);

/// Total bytes allocated (cumulative)
static TOTAL_BYTES_ALLOCATED: AtomicU64 = AtomicU64::new(0);

/// Total bytes freed (cumulative)
static TOTAL_BYTES_FREED: AtomicU64 = AtomicU64::new(0);

/// Live allocation records: address -> size
static LIVE_ALLOCATIONS: Mutex<Option<HashMap<usize, usize>>> = Mutex::new(None);

// ============================================================================
// Initialization
// ============================================================================

/// Initialize the leak tracker. Called once at first allocation.
/// Checks DOO_LEAK_CHECK env var and registers atexit handler.
///
/// Hot path: single Relaxed load (~0.3ns, compiles to MOV).
/// Cold path (first call only): env var check + atexit registration.
#[inline(always)]
pub fn init() {
    // Fast path: already initialized — single relaxed load, no memory barrier
    if INITIALIZED.load(Ordering::Relaxed) {
        return;
    }
    init_cold();
}

/// Cold path for initialization — called exactly once.
#[cold]
#[inline(never)]
fn init_cold() {
    // Atomic swap to prevent double-init in concurrent scenarios
    if INITIALIZED.swap(true, Ordering::SeqCst) {
        return;
    }

    let enabled = std::env::var("DOO_LEAK_CHECK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"))
        .unwrap_or(false);

    if enabled {
        ENABLED.store(true, Ordering::SeqCst);

        // Initialize the allocation map
        if let Ok(mut map) = LIVE_ALLOCATIONS.lock() {
            *map = Some(HashMap::with_capacity(1024));
        }

        // Register atexit handler to report leaks
        unsafe {
            libc::atexit(report_leaks_at_exit);
        }

        eprintln!("[DOO_LEAK_CHECK] Memory leak tracking enabled");
    }
}

/// Check if tracking is enabled (fast path — single atomic load)
#[inline(always)]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

// ============================================================================
// Tracking
// ============================================================================

/// Record an allocation. Called from doo_alloc when tracking is enabled.
#[inline]
pub fn track_alloc(ptr: *mut u8, size: usize) {
    if ptr.is_null() || !is_enabled() {
        return;
    }

    TOTAL_ALLOCS.fetch_add(1, Ordering::Relaxed);
    TOTAL_BYTES_ALLOCATED.fetch_add(size as u64, Ordering::Relaxed);

    if let Ok(mut guard) = LIVE_ALLOCATIONS.lock() {
        if let Some(map) = guard.as_mut() {
            map.insert(ptr as usize, size);
        }
    }
}

/// Record a free. Called from doo_free when tracking is enabled.
#[inline]
pub fn track_free(ptr: *mut u8) {
    if ptr.is_null() || !is_enabled() {
        return;
    }

    TOTAL_FREES.fetch_add(1, Ordering::Relaxed);

    if let Ok(mut guard) = LIVE_ALLOCATIONS.lock() {
        if let Some(map) = guard.as_mut() {
            if let Some(size) = map.remove(&(ptr as usize)) {
                TOTAL_BYTES_FREED.fetch_add(size as u64, Ordering::Relaxed);
            } else {
                // Free of unknown pointer — possible double-free or free of non-doo memory
                eprintln!(
                    "[DOO_LEAK_CHECK] WARNING: free of untracked pointer {:p}",
                    ptr
                );
            }
        }
    }
}

// ============================================================================
// Reporting
// ============================================================================

/// Called at process exit via atexit(). Reports any leaked memory.
extern "C" fn report_leaks_at_exit() {
    let total_allocs = TOTAL_ALLOCS.load(Ordering::SeqCst);
    let total_frees = TOTAL_FREES.load(Ordering::SeqCst);
    let total_bytes_allocated = TOTAL_BYTES_ALLOCATED.load(Ordering::SeqCst);
    let total_bytes_freed = TOTAL_BYTES_FREED.load(Ordering::SeqCst);

    let (leak_count, leaked_bytes, details) = if let Ok(guard) = LIVE_ALLOCATIONS.lock() {
        if let Some(map) = guard.as_ref() {
            let count = map.len();
            let bytes: usize = map.values().sum();

            // Collect leak details (up to 50 for display)
            let mut details: Vec<(usize, usize)> =
                map.iter().map(|(&addr, &size)| (addr, size)).collect();
            details.sort_by(|a, b| b.1.cmp(&a.1)); // Sort by size descending
            details.truncate(50);

            (count, bytes, details)
        } else {
            (0, 0, vec![])
        }
    } else {
        (0, 0, vec![])
    };

    eprintln!();
    eprintln!("==================================================================");
    eprintln!("  DOO MEMORY LEAK CHECK REPORT");
    eprintln!("==================================================================");
    eprintln!();
    eprintln!("  Total allocations:    {}", total_allocs);
    eprintln!("  Total frees:          {}", total_frees);
    eprintln!("  Total bytes allocated: {} bytes", total_bytes_allocated);
    eprintln!("  Total bytes freed:     {} bytes", total_bytes_freed);
    eprintln!();

    if leak_count == 0 {
        eprintln!("  RESULT: NO LEAKS DETECTED ✓");
        eprintln!("  All {} allocation(s) were properly freed.", total_allocs);
    } else {
        eprintln!(
            "  RESULT: LEAKED {} allocation(s), {} bytes total",
            leak_count, leaked_bytes
        );
        eprintln!();

        if !details.is_empty() {
            eprintln!("  Leaked allocations (largest first):");
            for (addr, size) in &details {
                eprintln!("    - {:#x}: {} bytes", addr, size);
            }
            if leak_count > details.len() {
                eprintln!("    ... and {} more", leak_count - details.len());
            }
        }
    }

    eprintln!();
    eprintln!("==================================================================");

    // Exit with non-zero if leaks detected (for CI automation)
    if leak_count > 0 {
        // Use _exit to avoid recursive atexit handlers
        unsafe { libc::_exit(77) }; // 77 = special exit code for leak detected
    }
}

// ============================================================================
// FFI exports (for compiler/codegen integration)
// ============================================================================

/// Initialize the leak tracker from Doo runtime.
/// Called automatically on first allocation, but can be called explicitly.
#[no_mangle]
pub extern "C" fn doo_leak_tracker_init() {
    init();
}

/// Get current leak count (for programmatic checks).
#[no_mangle]
pub extern "C" fn doo_leak_tracker_count() -> i64 {
    if !is_enabled() {
        return 0;
    }
    if let Ok(guard) = LIVE_ALLOCATIONS.lock() {
        if let Some(map) = guard.as_ref() {
            return map.len() as i64;
        }
    }
    0
}
