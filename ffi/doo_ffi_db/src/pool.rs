//! Connection Pool Management
//! Uses deadpool-postgres for connection pooling.

use std::sync::OnceLock;
use std::str::FromStr;

use deadpool_postgres::{Config, Pool, Runtime, Manager};
use tokio_postgres::NoTls;

static POOL: OnceLock<Pool> = OnceLock::new();

/// Initialize the connection pool
pub async fn init_pool(connection_string: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Parse connection string
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
    
    // Create pool
    let pool = config.create_pool(Some(Runtime::Tokio1), NoTls)?;
    
    // Test connection
    let _client = pool.get().await?;
    
    POOL.set(pool).map_err(|_| "Pool already initialized")?;
    
    Ok(())
}

/// Check if pool is initialized
pub fn is_pool_initialized() -> bool {
    POOL.get().is_some()
}

/// Get a client from the pool
pub async fn get_client() -> Result<deadpool_postgres::Client, Box<dyn std::error::Error>> {
    let pool = POOL.get().ok_or("Database pool not initialized")?;
    let client = pool.get().await?;
    Ok(client)
}
