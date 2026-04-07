//! PostgreSQL Driver for Doo
//!
//! Implements `crate::driver::DbDriver` for PostgreSQL using
//! `tokio-postgres` + `deadpool-postgres`.
//!
//! ## Features
//! - Connection pooling with deadpool (bounded, timeouts, recycling)
//! - Type-aware parameter binding via `prepare()` + PG type inference
//! - Direct JSON serialization (no intermediate serde_json::Value tree)
//! - PascalCase column name conversion
//! - PostgreSQL-dialect DDL generation for migrations

pub(crate) mod json_utils;
pub(crate) mod params;
pub(crate) mod pool;

use doo_ffi_core::ffi_debug;

use crate::driver::{BoxFuture, DbDriver, DriverResult};
use crate::limits::MAX_ROWS;
use crate::migrate::TableSchema;

pub use pool::{get_client, init_pool, is_pool_initialized};

// ============================================================================
// PostgreSQL Driver
// ============================================================================

/// PostgreSQL database driver.
///
/// Stateless struct — all state lives in the global connection pool (`OnceLock<Pool>`).
/// This is safe because the pool is thread-safe (`deadpool_postgres::Pool` is `Send + Sync`).
pub struct PostgresDriver;

impl DbDriver for PostgresDriver {
    fn name(&self) -> &'static str {
        "postgres"
    }

    fn is_connected(&self) -> bool {
        is_pool_initialized()
    }

    fn query(
        &self,
        sql: &str,
        params_vals: &[serde_json::Value],
    ) -> BoxFuture<'_, DriverResult<String>> {
        let sql = sql.to_owned();
        let params_vals = params_vals.to_vec();
        Box::pin(async move {
            let client = get_client().await.map_err(|e| format_pg_error(&*e))?;

            if params_vals.is_empty() {
                let rows = client
                    .query(&sql, &[])
                    .await
                    .map_err(|e| format_pg_error(&e))?;
                if rows.len() > MAX_ROWS {
                    return Err(
                        format!("Query returned {} rows (max {})", rows.len(), MAX_ROWS).into(),
                    );
                }
                Ok(json_utils::rows_to_json(&rows))
            } else {
                // Prepare to get PG-inferred param types, then adapt params
                let stmt = client
                    .prepare_cached(&sql)
                    .await
                    .map_err(|e| format_pg_error(&e))?;
                let pg_types = stmt.params();
                let boxed_params = params::json_values_to_pg_params_typed(&params_vals, pg_types);
                let param_refs = params::params_as_refs(&boxed_params);

                let rows = client
                    .query(&stmt, &param_refs[..])
                    .await
                    .map_err(|e| format_pg_error(&e))?;
                if rows.len() > MAX_ROWS {
                    return Err(
                        format!("Query returned {} rows (max {})", rows.len(), MAX_ROWS).into(),
                    );
                }
                Ok(json_utils::rows_to_json(&rows))
            }
        })
    }

    fn execute(
        &self,
        sql: &str,
        params_vals: &[serde_json::Value],
    ) -> BoxFuture<'_, DriverResult<u64>> {
        let sql = sql.to_owned();
        let params_vals = params_vals.to_vec();
        Box::pin(async move {
            let client = get_client().await.map_err(|e| format_pg_error(&*e))?;

            if params_vals.is_empty() {
                let affected = client
                    .execute(&sql, &[])
                    .await
                    .map_err(|e| format_pg_error(&e))?;
                Ok(affected)
            } else {
                let boxed_params = params::json_values_to_pg_params(&params_vals);
                let param_refs = params::params_as_refs(&boxed_params);
                let affected = client
                    .execute(&sql, &param_refs[..])
                    .await
                    .map_err(|e| format_pg_error(&e))?;
                Ok(affected)
            }
        })
    }

    fn query_one(
        &self,
        sql: &str,
        params_vals: &[serde_json::Value],
    ) -> BoxFuture<'_, DriverResult<String>> {
        let sql = sql.to_owned();
        let params_vals = params_vals.to_vec();
        Box::pin(async move {
            let client = get_client().await.map_err(|e| format_pg_error(&*e))?;

            if params_vals.is_empty() {
                let row = client
                    .query_one(&sql, &[])
                    .await
                    .map_err(|e| format_pg_error(&e))?;
                Ok(json_utils::row_to_json(&row))
            } else {
                let boxed_params = params::json_values_to_pg_params(&params_vals);
                let param_refs = params::params_as_refs(&boxed_params);
                let row = client
                    .query_one(&sql, &param_refs[..])
                    .await
                    .map_err(|e| format_pg_error(&e))?;
                Ok(json_utils::row_to_json(&row))
            }
        })
    }

    fn transaction(&self, queries_json: &str) -> BoxFuture<'_, DriverResult<String>> {
        let queries_json = queries_json.to_owned();
        Box::pin(async move {
            let queries: Vec<QueryDef> = serde_json::from_str(&queries_json)
                .map_err(|e| format!("Invalid transaction queries: {}", e))?;

            let mut client = get_client().await.map_err(|e| format_pg_error(&*e))?;
            let tx = client
                .transaction()
                .await
                .map_err(|e| format_pg_error(&e))?;

            let mut results = Vec::new();
            for q in &queries {
                let boxed_params = params::json_values_to_pg_params(&q.params);
                let param_refs = params::params_as_refs(&boxed_params);
                let rows = tx
                    .query(&q.sql, &param_refs[..])
                    .await
                    .map_err(|e| format_pg_error(&e))?;
                if rows.len() > MAX_ROWS {
                    return Err(
                        format!("Query returned {} rows (max {})", rows.len(), MAX_ROWS).into(),
                    );
                }
                results.push(json_utils::rows_to_json(&rows));
            }

            tx.commit().await.map_err(|e| format_pg_error(&e))?;
            Ok(serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string()))
        })
    }

    fn batch_execute(&self, sql: &str) -> BoxFuture<'_, DriverResult<()>> {
        let sql = sql.to_owned();
        Box::pin(async move {
            let client = get_client().await.map_err(|e| format_pg_error(&*e))?;
            client.batch_execute(&sql).await.map_err(|e| {
                let msg: Box<dyn std::error::Error + Send + Sync> =
                    format!("Batch execute failed: {}", format_pg_error(&e)).into();
                msg
            })?;
            Ok(())
        })
    }

    fn execute_auto(
        &self,
        sql: &str,
        params_vals: &[serde_json::Value],
    ) -> BoxFuture<'_, DriverResult<String>> {
        let sql = sql.to_owned();
        let params_vals = params_vals.to_vec();
        Box::pin(async move {
            let client = get_client().await.map_err(|e| format_pg_error(&*e))?;

            if params_vals.is_empty() {
                if is_mutating_sql(&sql) {
                    let count = client
                        .execute(&sql, &[])
                        .await
                        .map_err(|e| format_pg_error(&e))?;
                    Ok(format!("{{\"affected_rows\":{}}}", count))
                } else {
                    let rows = client
                        .query(&sql, &[])
                        .await
                        .map_err(|e| format_pg_error(&e))?;
                    if rows.len() > MAX_ROWS {
                        return Err(format!(
                            "Query returned {} rows (max {})",
                            rows.len(),
                            MAX_ROWS
                        )
                        .into());
                    }
                    Ok(json_utils::rows_to_json(&rows))
                }
            } else {
                let stmt = client
                    .prepare_cached(&sql)
                    .await
                    .map_err(|e| format_pg_error(&e))?;
                let pg_types = stmt.params();
                let boxed_params = params::json_values_to_pg_params_typed(&params_vals, pg_types);
                let param_refs = params::params_as_refs(&boxed_params);

                if is_mutating_sql(&sql) {
                    if has_returning(&sql) {
                        let rows = client
                            .query(&stmt, &param_refs[..])
                            .await
                            .map_err(|e| format_pg_error(&e))?;
                        Ok(json_utils::rows_to_json(&rows))
                    } else {
                        let count = client
                            .execute(&stmt, &param_refs[..])
                            .await
                            .map_err(|e| format_pg_error(&e))?;
                        Ok(format!("{{\"affected_rows\":{}}}", count))
                    }
                } else {
                    let rows = client
                        .query(&stmt, &param_refs[..])
                        .await
                        .map_err(|e| format_pg_error(&e))?;
                    if rows.len() > MAX_ROWS {
                        return Err(format!(
                            "Query returned {} rows (max {})",
                            rows.len(),
                            MAX_ROWS
                        )
                        .into());
                    }
                    Ok(json_utils::rows_to_json(&rows))
                }
            }
        })
    }

    /// Pipeline multiple queries on a SINGLE connection.
    /// Each query returns exactly one row (e.g., TechEmpower's multiple-queries test).
    /// Uses `prepare_cached` for statement reuse + single pool checkout.
    fn batch_query(
        &self,
        queries: &[(String, Vec<serde_json::Value>)],
    ) -> BoxFuture<'_, DriverResult<String>> {
        let queries = queries.to_vec();
        Box::pin(async move {
            let client = get_client().await.map_err(|e| format_pg_error(&*e))?;
            let mut buf = String::with_capacity(queries.len() * 64);
            buf.push('[');
            for (qi, (sql, params_vals)) in queries.iter().enumerate() {
                if qi > 0 {
                    buf.push(',');
                }
                if params_vals.is_empty() {
                    let row = client
                        .query_one(&**sql, &[])
                        .await
                        .map_err(|e| format_pg_error(&e))?;
                    buf.push_str(&json_utils::row_to_json(&row));
                } else {
                    let stmt = client
                        .prepare_cached(sql)
                        .await
                        .map_err(|e| format_pg_error(&e))?;
                    let pg_types = stmt.params();
                    let boxed_params =
                        params::json_values_to_pg_params_typed(params_vals, pg_types);
                    let param_refs = params::params_as_refs(&boxed_params);
                    let row = client
                        .query_one(&stmt, &param_refs[..])
                        .await
                        .map_err(|e| format_pg_error(&e))?;
                    buf.push_str(&json_utils::row_to_json(&row));
                }
            }
            buf.push(']');
            Ok(buf)
        })
    }

    /// Batch UPDATE using PostgreSQL's `unnest` — single statement for all updates.
    /// `sql` should use `unnest($1::int[], $2::int[])` syntax.
    /// `ids` and `values` are parallel arrays of IDs and new random numbers.
    fn batch_update(
        &self,
        sql: &str,
        ids: &[i32],
        values: &[i32],
    ) -> BoxFuture<'_, DriverResult<u64>> {
        let sql = sql.to_owned();
        let ids = ids.to_vec();
        let values = values.to_vec();
        Box::pin(async move {
            let client = get_client().await.map_err(|e| format_pg_error(&*e))?;
            let affected = client
                .execute(&sql, &[&ids, &values])
                .await
                .map_err(|e| format_pg_error(&e))?;
            Ok(affected)
        })
    }

    fn generate_create_table(&self, schema: &TableSchema) -> String {
        let mut sql = format!("CREATE TABLE IF NOT EXISTS {} (\n", schema.name);

        let mut col_defs = Vec::new();
        for col in &schema.columns {
            let mut def = format!("  {} {}", col.name, col.sql_type);
            if col.primary_key {
                def.push_str(" PRIMARY KEY");
            }
            if col.auto_increment {
                // PostgreSQL 10+ identity columns
                def.push_str(" GENERATED ALWAYS AS IDENTITY");
            }
            if !col.nullable {
                def.push_str(" NOT NULL");
            }
            if col.unique && !col.primary_key {
                def.push_str(" UNIQUE");
            }
            if let Some(default) = &col.default {
                def.push_str(&format!(" DEFAULT {}", default));
            }
            col_defs.push(def);
        }

        // Foreign key constraints
        for fk in &schema.foreign_keys {
            col_defs.push(format!(
                "  FOREIGN KEY ({}) REFERENCES {}({})",
                fk.column, fk.ref_table, fk.ref_column
            ));
        }

        sql.push_str(&col_defs.join(",\n"));
        sql.push_str("\n);\n");

        // Indexes
        for idx in &schema.indexes {
            let unique = if idx.unique { "UNIQUE " } else { "" };
            sql.push_str(&format!(
                "CREATE {}INDEX IF NOT EXISTS {} ON {} ({});\n",
                unique,
                idx.name,
                schema.name,
                idx.columns.join(", ")
            ));
        }

        sql
    }
}

// ============================================================================
// Connection — reads DATABASE_URL and registers the driver
// ============================================================================

/// Connect to PostgreSQL and register the driver.
///
/// Reads `DATABASE_URL` from environment. Errors if not set or connection fails.
/// On success, registers `PostgresDriver` as the active database driver so
/// all subsequent `doo_db_*` calls route through PostgreSQL.
pub fn connect_from_env() -> Result<(), String> {
    let conn_str = match std::env::var(doo_ffi_core::constants::ENV_DATABASE_URL) {
        Ok(s) => {
            ffi_debug!("DB", "DATABASE_URL found");
            s
        }
        Err(_) => {
            return Err("DATABASE_URL environment variable is not set. Cannot start without a database.".to_string());
        }
    };

    // Get runtime from parent crate
    let rt = crate::get_runtime();
    match rt.block_on(pool::init_pool(&conn_str)) {
        Ok(_) => {
            ffi_debug!("DB", "Pool initialized successfully");
            // Register the driver for all subsequent DB operations
            crate::driver::register_driver(Box::new(PostgresDriver)).map_err(|e| e.to_string())?;
            Ok(())
        }
        Err(e) => {
            ffi_debug!("DB", "Connection failed: {}", e);
            Err(format!("Database connection failed: {}", e))
        }
    }
}

/// Map PostgreSQL SQLSTATE error code to DbError.
pub fn db_error_from_pg_code(code: &str) -> crate::error::DbError {
    use crate::error::DbError;
    match code {
        "23505" => DbError::UniqueViolation,
        "23503" => DbError::ForeignKeyViolation,
        "23502" => DbError::NotNullViolation,
        "23514" => DbError::CheckViolation,
        "42601" | "42000" => DbError::InvalidSql,
        "42P01" => DbError::TableNotFound,
        "42703" => DbError::ColumnNotFound,
        "42804" | "42846" => DbError::DataTypeMismatch,
        "40001" | "40P01" => DbError::TransactionFailed,
        "57014" => DbError::QueryTimeout,
        "08000" | "08003" | "08006" => DbError::ConnectionFailed,
        _ => DbError::QueryFailed,
    }
}

/// Extract detailed error message from a PostgreSQL error.
pub fn format_pg_error(e: &(dyn std::error::Error + 'static)) -> String {
    if let Some(pg_err) = e.downcast_ref::<tokio_postgres::Error>() {
        if let Some(db_err) = pg_err.as_db_error() {
            let msg = db_err.message();
            let code = db_err.code().code();
            let detail = db_err.detail().unwrap_or("");
            let hint = db_err.hint().unwrap_or("");

            let mut full_msg = format!("[{}] {}", code, msg);
            if !detail.is_empty() {
                full_msg.push_str(&format!(" (Detail: {})", detail));
            }
            if !hint.is_empty() {
                full_msg.push_str(&format!(" (Hint: {})", hint));
            }
            return full_msg;
        }
    }
    e.to_string()
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Transaction query definition.
#[derive(serde::Deserialize)]
struct QueryDef {
    sql: String,
    #[serde(default)]
    params: Vec<serde_json::Value>,
}

/// Check if SQL is a mutating statement.
fn is_mutating_sql(sql: &str) -> bool {
    let trimmed = sql.trim_start().as_bytes();
    if trimmed.len() < 4 {
        return false;
    }
    let prefix: [u8; 4] = [
        trimmed[0].to_ascii_uppercase(),
        trimmed[1].to_ascii_uppercase(),
        trimmed[2].to_ascii_uppercase(),
        trimmed[3].to_ascii_uppercase(),
    ];
    matches!(
        &prefix,
        b"INSE" | b"UPDA" | b"DELE" | b"CREA" | b"ALTE" | b"DROP" | b"TRUN"
    )
}

/// Check if SQL contains RETURNING clause.
fn has_returning(sql: &str) -> bool {
    sql.as_bytes()
        .windows(9)
        .any(|w| w.eq_ignore_ascii_case(b"RETURNING"))
}
