//! PostgreSQL Connection Pool
//!
//! Uses `deadpool-postgres` for production-grade connection pooling.
//! Configuration via environment variables.

use std::str::FromStr;
use std::sync::OnceLock;
use std::time::Duration;

use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod, Runtime, Timeouts};

use doo_ffi_core::ffi_debug;

static POOL: OnceLock<Pool> = OnceLock::new();

/// Initialize the connection pool with production-grade configuration.
///
/// Pool sizing: min(cpu_count * 2, 32), overridable via DATABASE_POOL_SIZE env.
/// Timeouts: 5s wait, 5s create, 5s recycle — prevents infinite hangs.
/// Recycling: Fast mode — checks connection liveness on checkout.
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
            ffi_debug!("DB", "Parsed host: {}", host);
        }
    }

    // Get ports (take first)
    let ports = pg_config.get_ports();
    if !ports.is_empty() {
        config.port = Some(ports[0]);
        ffi_debug!("DB", "Parsed port: {}", ports[0]);
    }

    // Production pool configuration
    let cpu_count = num_cpus::get();
    let pool_size = std::env::var(doo_ffi_core::constants::ENV_DATABASE_POOL_SIZE)
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

    // Build TLS connector for Cloud SQL (requires encrypted connections)
    // Uses system/webpki root CAs to verify Cloud SQL server certificate
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let rustls_config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let tls = tokio_postgres_rustls::MakeRustlsConnect::new(rustls_config);

    ffi_debug!("DB", "TLS connector configured (rustls + webpki roots)");

    let pool = config.create_pool(Some(Runtime::Tokio1), tls)?;

    // Test connection with timeout and set timezone to UTC
    let test_client = tokio::time::timeout(Duration::from_secs(10), pool.get())
        .await
        .map_err(|_| "Connection test timed out (10s)")?
        .map_err(|e| {
            // Walk the full error chain for debugging
            let mut msg = format!("Connection test failed: {}", e);
            let mut source = std::error::Error::source(&e);
            while let Some(cause) = source {
                msg.push_str(&format!(" | Caused by: {}", cause));
                source = std::error::Error::source(cause);
            }
            msg
        })?;

    // Ensure UTC timezone for consistent timestamp handling
    test_client
        .simple_query("SET timezone = 'UTC'")
        .await
        .map_err(|e| format!("Failed to set timezone: {}", e))?;

    ffi_debug!("DB", "Connection test passed, pool ready (timezone=UTC)");

    POOL.set(pool).map_err(|_| "Pool already initialized")?;

    Ok(())
}

/// Check if pool is initialized.
pub fn is_pool_initialized() -> bool {
    POOL.get().is_some()
}

/// Get a client from the pool.
/// Sets timezone to UTC on each checkout to ensure consistent timestamp handling.
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
    // Ensure UTC timezone — prevents TIMESTAMP columns from using server's local timezone.
    // This is idempotent (no-op if already UTC) and cheap (~0.1ms per SET).
    let _ = client.simple_query("SET timezone = 'UTC'").await;
    Ok(client)
}
