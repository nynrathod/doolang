//! PostgreSQL Connection Pool
//!
//! Uses `deadpool-postgres` for production-grade connection pooling.
//! Configuration via environment variables.
//!
//! TLS behavior driven by `sslmode` in DATABASE_URL (standard PostgreSQL parameter):
//!   - `disable`     → no TLS (local dev)
//!   - `require`     → TLS encryption, skip CA verification (default — works with any cloud)
//!   - `verify-full` → TLS + verify server CA against Mozilla root CAs
//!
//! Defaults to `require` when sslmode is absent — encrypted but no CA check.
//! This works with Cloud SQL, AWS RDS, Azure, Supabase, Neon, self-hosted, etc.

use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod, Runtime, Timeouts};

use doo_ffi_core::ffi_debug;

static POOL: OnceLock<Pool> = OnceLock::new();

/// Extract sslmode from a connection string.
/// Checks both parsed tokio-postgres config and raw query string.
fn parse_sslmode(connection_string: &str) -> String {
    // tokio-postgres parses sslmode from the connection string and exposes it
    // via get_ssl_mode(), but the enum doesn't cover all pg modes.
    // Parse from the raw URL query params for accuracy.
    if let Some(query_start) = connection_string.find('?') {
        let query = &connection_string[query_start + 1..];
        for param in query.split('&') {
            if let Some((key, value)) = param.split_once('=') {
                if key.eq_ignore_ascii_case("sslmode") {
                    return value.to_lowercase();
                }
            }
        }
    }
    // Check key-value format (e.g., "host=/path sslmode=disable user=x ...")
    for part in connection_string.split_whitespace() {
        if let Some((key, value)) = part.split_once('=') {
            if key.eq_ignore_ascii_case("sslmode") {
                return value.to_lowercase();
            }
        }
    }
    // Default: require (encrypted, no CA verification) — works with any cloud platform
    "require".to_string()
}

/// A TLS certificate verifier that accepts any server certificate.
/// Equivalent to PostgreSQL `sslmode=require` — encrypts traffic without CA verification.
/// This is the standard mode for cloud-managed databases (Cloud SQL, RDS, Azure, etc.)
/// which use private/internal CAs not in public trust stores.
#[derive(Debug)]
struct NoVerifier;

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Build a rustls ClientConfig based on sslmode.
fn build_tls_config(sslmode: &str) -> rustls::ClientConfig {
    match sslmode {
        "verify-full" | "verify-ca" => {
            // Full CA verification using Mozilla's trusted root certificates
            let mut root_store = rustls::RootCertStore::empty();
            root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            rustls::ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth()
        }
        _ => {
            // require / prefer — encrypt but skip CA verification
            // Works with ANY cloud platform's private CA (Cloud SQL, RDS, Azure, etc.)
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoVerifier))
                .with_no_client_auth()
        }
    }
}

/// Initialize the connection pool with production-grade configuration.
///
/// Pool sizing: min(cpu_count * 2, 32), overridable via DATABASE_POOL_SIZE env.
/// Timeouts: 5s wait, 5s create, 5s recycle — prevents infinite hangs.
/// Recycling: Fast mode — checks connection liveness on checkout.
pub async fn init_pool(connection_string: &str) -> Result<(), Box<dyn std::error::Error>> {
    let sslmode = parse_sslmode(connection_string);
    ffi_debug!("DB", "sslmode={}", sslmode);

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

    // Get hosts (take first) — supports both TCP and Unix socket connections
    let hosts = pg_config.get_hosts();
    if !hosts.is_empty() {
        match &hosts[0] {
            tokio_postgres::config::Host::Tcp(host) => {
                config.host = Some(host.clone());
                ffi_debug!("DB", "Parsed host: {}", host);
            }
            #[cfg(unix)]
            tokio_postgres::config::Host::Unix(path) => {
                config.host = Some(path.display().to_string());
                ffi_debug!("DB", "Parsed Unix socket path: {}", path.display());
            }
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

    if sslmode == "disable" {
        // No TLS — local development only
        ffi_debug!("DB", "TLS disabled (sslmode=disable)");
        let pool = config.create_pool(Some(Runtime::Tokio1), tokio_postgres::NoTls)?;
        return test_and_store_pool(pool).await;
    }

    // TLS mode (require, verify-full, etc.)
    let rustls_config = build_tls_config(&sslmode);
    let tls = tokio_postgres_rustls::MakeRustlsConnect::new(rustls_config);
    ffi_debug!("DB", "TLS connector configured (sslmode={})", sslmode);

    let pool = config.create_pool(Some(Runtime::Tokio1), tls)?;
    test_and_store_pool(pool).await
}

/// Test the pool connection and store it in the global static.
async fn test_and_store_pool(pool: Pool) -> Result<(), Box<dyn std::error::Error>> {
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
