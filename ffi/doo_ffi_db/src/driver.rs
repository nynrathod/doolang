//! Database Driver Trait — Extensible driver architecture.
//!
//! Any database backend (PostgreSQL, MySQL, SQLite, etc.) implements `DbDriver`.
//! The base `doo_ffi_db` crate dispatches all FFI calls through the registered driver.
//!
//! ## Adding a new database driver
//!
//! 1. Create a new module under `src/drivers/<name>/` (e.g., `src/drivers/mysql/`)
//! 2. Implement `DbDriver` for your driver struct
//! 3. Add the driver deps to `Cargo.toml` (can use feature gates)
//! 4. Register from `src/drivers/mod.rs`
//! 5. Add a `doo_db_connect_<driver>` FFI function in `lib.rs`
//!
//! **Zero compiler changes required. Zero codegen changes required.**

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use crate::migrate::TableSchema;

/// Boxed future for async trait methods (Rust doesn't support `async fn` in traits natively).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Result type for driver operations.
pub type DriverResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Database driver trait — each backend implements this.
///
/// All async methods return `BoxFuture` since trait async fns require boxing.
/// The dispatch layer in `lib.rs` handles blocking via `run_db_async()`.
///
/// Return types are `String` (JSON) because FFI ultimately passes JSON strings.
/// Drivers handle their own row→JSON serialization for zero intermediate copies.
///
/// # Performance
///
/// - `BoxFuture` is heap-allocated once per query — negligible vs network I/O
/// - Driver gets full control over serialization (can avoid serde_json::Value)
/// - Semaphore/timeout enforcement is in the generic layer, not per-driver
pub trait DbDriver: Send + Sync + 'static {
    /// Driver name for logging (e.g., "postgres", "mysql", "sqlite").
    fn name(&self) -> &'static str;

    /// Check if pool/connection is initialized and ready for queries.
    fn is_connected(&self) -> bool;

    /// Execute a SELECT query, return JSON array string of rows.
    ///
    /// `params` are passed as `serde_json::Value` — the driver handles
    /// conversion to its native parameter types (e.g., `tokio_postgres::types::ToSql`).
    fn query(
        &self,
        sql: &str,
        params: &[serde_json::Value],
    ) -> BoxFuture<'_, DriverResult<String>>;

    /// Execute a mutating statement (INSERT/UPDATE/DELETE), return affected row count.
    fn execute(
        &self,
        sql: &str,
        params: &[serde_json::Value],
    ) -> BoxFuture<'_, DriverResult<u64>>;

    /// Query expecting exactly one row, return JSON object string.
    fn query_one(
        &self,
        sql: &str,
        params: &[serde_json::Value],
    ) -> BoxFuture<'_, DriverResult<String>>;

    /// Execute multiple queries in a single transaction.
    ///
    /// `queries_json` is a JSON array of `{ "sql": "...", "params": [...] }` objects.
    /// Returns JSON array of per-query results.
    /// On any error, the entire transaction must be rolled back.
    fn transaction(&self, queries_json: &str) -> BoxFuture<'_, DriverResult<String>>;

    /// Execute batch DDL/SQL (for migrations).
    fn batch_execute(&self, sql: &str) -> BoxFuture<'_, DriverResult<()>>;

    /// Auto-detect SELECT vs mutation, execute accordingly, return JSON result.
    ///
    /// For SELECT: returns JSON array of rows.
    /// For INSERT/UPDATE/DELETE with RETURNING: returns JSON array.
    /// For INSERT/UPDATE/DELETE without RETURNING: returns `{"affected_rows": N}`.
    ///
    /// Used by HTTP CRUD operations.
    fn execute_auto(
        &self,
        sql: &str,
        params: &[serde_json::Value],
    ) -> BoxFuture<'_, DriverResult<String>>;

    /// Generate CREATE TABLE DDL in this driver's SQL dialect.
    ///
    /// Each driver generates dialect-specific DDL:
    /// - PostgreSQL: `GENERATED ALWAYS AS IDENTITY`, `CREATE INDEX IF NOT EXISTS`
    /// - MySQL: `AUTO_INCREMENT`, `IF NOT EXISTS`
    /// - SQLite: `AUTOINCREMENT`, `IF NOT EXISTS`
    fn generate_create_table(&self, schema: &TableSchema) -> String;
}

// ============================================================================
// Driver Registry — OnceLock for zero-overhead after initialization
// ============================================================================

static DRIVER: OnceLock<Box<dyn DbDriver>> = OnceLock::new();

/// Register the active database driver.
///
/// Called by driver implementations during their connect function
/// (e.g., `PostgresDriver` registers itself in `doo_db_connect_postgres`).
///
/// Can only be called once — first driver wins. Returns Err if already registered.
pub fn register_driver(driver: Box<dyn DbDriver>) -> Result<(), &'static str> {
    DRIVER
        .set(driver)
        .map_err(|_| "Database driver already registered")
}

/// Get the active database driver.
///
/// Returns `None` if no driver has been registered (no `Database::Postgres()` etc. called).
pub fn get_driver() -> Option<&'static dyn DbDriver> {
    DRIVER.get().map(|b| b.as_ref())
}

/// Check if a driver has been registered.
pub fn is_driver_registered() -> bool {
    DRIVER.get().is_some()
}
