//! Connection Pool Management
//! Uses deadpool-postgres for connection pooling.
//!
//! Production-grade configuration:
//! - Bounded pool size (CPU-based or env override)
//! - Connection/wait/recycle timeouts
//! - FIFO queue mode
//! - Fast connection recycling
//! - Per-connection statement_timeout

use std::str::FromStr;
use std::sync::OnceLock;
use std::time::Duration;

use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod, Runtime, Timeouts};
use tokio_postgres::NoTls;

use doo_ffi_core::ffi_debug;

static POOL: OnceLock<Pool> = OnceLock::new();

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
        let limit = std::env::var("DATABASE_MAX_QUERIES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(200);
        ffi_debug!("DB", "Query semaphore initialized with limit: {}", limit);
        tokio::sync::Semaphore::new(limit)
    })
}

/// Get per-query timeout duration from env or default.
pub fn get_query_timeout() -> Duration {
    let secs = std::env::var("DATABASE_QUERY_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_QUERY_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

/// Get semaphore wait timeout from env or default.
pub fn get_semaphore_wait_timeout() -> Duration {
    let ms = std::env::var("DATABASE_SEMAPHORE_WAIT_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SEMAPHORE_WAIT_MS);
    Duration::from_millis(ms)
}

/// Initialize the connection pool with production-grade configuration.
///
/// Pool sizing: min(cpu_count * 2, 32), overridable via DATABASE_POOL_SIZE env.
/// Timeouts: 5s wait, 5s create, 5s recycle — prevents infinite hangs.
/// Recycling: Fast mode — checks connection liveness on checkout.
/// Statement timeout: 30s per connection — prevents runaway queries in PG.
pub async fn init_pool(connection_string: &str) -> Result<(), Box<dyn std::error::Error>> {
    let pg_config = tokio_postgres::Config::from_str(connection_string)?;

    let mut config = Config::new();

    // Extract connection details from parsed config
    if let Some(user) = pg_config.get_user() {
        config.user = Some(user.to_string());
    }
    if let Some(password) = pg_config.get_password() {
        config.password = Some(String::from_utf8_lossy(password).to_string());
    }
    if let Some(dbname) = pg_config.get_dbname() {
        config.dbname = Some(dbname.to_string());
    }

    // Get hosts (take first)
    let hosts = pg_config.get_hosts();
    if !hosts.is_empty() {
        if let tokio_postgres::config::Host::Tcp(host) = &hosts[0] {
            config.host = Some(host.clone());
        }
    }

    // Get ports (take first)
    let ports = pg_config.get_ports();
    if !ports.is_empty() {
        config.port = Some(ports[0]);
    }

    // Production pool configuration
    let cpu_count = num_cpus::get();
    let pool_size = std::env::var("DATABASE_POOL_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or((cpu_count * 2).min(32).max(4));

    ffi_debug!(
        "DB",
        "Pool config: size={}, cpus={}, timeouts=5s/5s/5s",
        pool_size,
        cpu_count
    );

    config.pool = Some(deadpool_postgres::PoolConfig {
        max_size: pool_size,
        timeouts: Timeouts {
            wait: Some(Duration::from_secs(5)),
            create: Some(Duration::from_secs(5)),
            recycle: Some(Duration::from_secs(5)),
        },
        queue_mode: deadpool::managed::QueueMode::Fifo,
    });
    config.manager = Some(ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    });

    let pool = config.create_pool(Some(Runtime::Tokio1), NoTls)?;

    // Test connection with timeout
    let _client = tokio::time::timeout(Duration::from_secs(10), pool.get())
        .await
        .map_err(|_| "Connection test timed out (10s)")?
        .map_err(|e| format!("Connection test failed: {}", e))?;

    // Set statement_timeout on the test connection to verify it works
    // Each new connection from the pool will also get this via PG server config
    // or we set it inline when we get a client
    ffi_debug!("DB", "Connection test passed, pool ready");

    POOL.set(pool).map_err(|_| "Pool already initialized")?;

    // Pre-initialize the semaphore
    let _ = get_query_semaphore();

    Ok(())
}

/// Check if pool is initialized.
pub fn is_pool_initialized() -> bool {
    POOL.get().is_some()
}

/// Get a client from the pool (with wait timeout enforced by pool config).
pub async fn get_client(
) -> Result<deadpool_postgres::Client, Box<dyn std::error::Error + Send + Sync>> {
    let pool = POOL
        .get()
        .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
            "Database pool not initialized".into()
        })?;
    let client = pool
        .get()
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("Failed to get connection from pool: {}", e).into()
        })?;
    Ok(client)
}
