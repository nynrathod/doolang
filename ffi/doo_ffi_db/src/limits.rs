//! Generic concurrency limits and timeouts for database operations.
//!
//! These are driver-agnostic — all backends share the same
//! backpressure and timeout configuration.

use std::sync::OnceLock;
use std::time::Duration;

use doo_ffi_core::ffi_debug;

/// Query semaphore — bounds total in-flight DB queries to prevent overload.
/// When the pool is full + slow, excess requests get fast 503 instead of queuing.
static QUERY_SEMAPHORE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();

/// Default query timeout in seconds (individual query execution).
const DEFAULT_QUERY_TIMEOUT_SECS: u64 = 30;

/// Default semaphore wait timeout in milliseconds.
const DEFAULT_SEMAPHORE_WAIT_MS: u64 = 100;

/// Maximum rows returned by a single query (OOM protection).
pub const MAX_ROWS: usize = 10_000;

/// Get the query concurrency semaphore.
pub fn get_query_semaphore() -> &'static tokio::sync::Semaphore {
    QUERY_SEMAPHORE.get_or_init(|| {
        let limit = std::env::var(doo_ffi_core::constants::ENV_DATABASE_MAX_QUERIES)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(200);
        ffi_debug!("DB", "Query semaphore initialized with limit: {}", limit);
        tokio::sync::Semaphore::new(limit)
    })
}

/// Get per-query timeout duration from env or default.
pub fn get_query_timeout() -> Duration {
    let secs = std::env::var(doo_ffi_core::constants::ENV_DATABASE_QUERY_TIMEOUT_SECS)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_QUERY_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

/// Get semaphore wait timeout from env or default.
pub fn get_semaphore_wait_timeout() -> Duration {
    let ms = std::env::var(doo_ffi_core::constants::ENV_DATABASE_SEMAPHORE_WAIT_MS)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SEMAPHORE_WAIT_MS);
    Duration::from_millis(ms)
}
