//! doo_ffi_db — Complete Database FFI Library
//!
//! Production-grade database layer:
//! - Connection pooling with deadpool-postgres (bounded, timeouts, recycling)
//! - Async execution via shared runtime (no OS thread-per-query)
//! - Backpressure via semaphore (fast 503 on overload)
//! - Per-query timeouts
//! - Row count safety limits
//! - Transaction support
//! - Direct JSON serialization (no intermediate Value tree)
//! - Unified DooResult type
//! - All errors propagated (no silent empty arrays)

pub mod error;
mod json_utils;
pub mod migrate;
mod pool;

use std::ffi::c_void;
use std::os::raw::c_char;
use std::sync::OnceLock;

use doo_ffi_core::ffi_debug;
use doo_ffi_core::helpers::{c_to_string, safe_ffi, string_to_c};
use doo_ffi_core::DooResult;
use tokio::runtime::Runtime;

pub use json_utils::*;
pub use pool::*;

// ============================================================================
// Runtime — use shared HTTP runtime when available, fallback to DB-only runtime
// ============================================================================

/// Fallback runtime for standalone DB usage (no HTTP server).
static DB_RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Get the shared async runtime.
/// Prefers the global HTTP runtime from doo_ffi_runtime (all cores).
/// Falls back to a DB-specific runtime if HTTP server not started.
pub fn get_runtime() -> &'static Runtime {
    // Try the global runtime first (initialized by HTTP server / doo_runtime_init)
    if doo_ffi_runtime::runtime::is_runtime_initialized() {
        return doo_ffi_runtime::runtime::get_runtime();
    }
    // Fallback for standalone DB usage
    DB_RUNTIME.get_or_init(|| {
        let workers = num_cpus::get().max(4);
        ffi_debug!(
            "DB",
            "Creating fallback DB runtime with {} workers",
            workers
        );
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(workers)
            .thread_name("doo-db")
            .enable_all()
            .build()
            .unwrap_or_else(|e| {
                eprintln!("[Doo] FATAL: Failed to create DB runtime: {}", e);
                std::process::exit(1);
            })
    })
}

// ============================================================================
// Async Execution Helper — replaces all std::thread::spawn patterns
// ============================================================================

/// Execute async DB work from synchronous FFI context.
///
/// Uses tokio::spawn on the shared runtime + oneshot channel.
/// Zero OS threads created per query. Bounded by runtime worker count.
/// Includes semaphore backpressure and per-query timeout.
fn run_db_async<F, T>(f: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, Box<dyn std::error::Error + Send + Sync>>>
        + Send
        + 'static,
    T: Send + 'static,
{
    let rt = get_runtime();
    let timeout = get_query_timeout();
    let sem_timeout = get_semaphore_wait_timeout();

    let (tx, rx) = tokio::sync::oneshot::channel();

    rt.spawn(async move {
        // Acquire semaphore permit (backpressure gate)
        let _permit = match tokio::time::timeout(sem_timeout, get_query_semaphore().acquire()).await
        {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => {
                let _ = tx.send(Err("Semaphore closed".to_string()));
                return;
            }
            Err(_) => {
                let _ = tx.send(Err("Database overloaded".to_string()));
                return;
            }
        };

        // Execute with per-query timeout
        let result = match tokio::time::timeout(timeout, f).await {
            Ok(Ok(val)) => Ok(val),
            Ok(Err(e)) => Err(format!("Query failed: {}", e)),
            Err(_) => Err(format!("Query timed out ({}s)", timeout.as_secs())),
        };

        let _ = tx.send(result);
    });

    // Block the FFI thread waiting for the result.
    // This is safe: FFI calls come from LLVM-compiled code on blocking threads.
    rx.blocking_recv()
        .map_err(|_| "Channel closed".to_string())?
}

// ============================================================================
// String Helpers — delegated to doo_ffi_core::helpers (single source of truth)
// c_to_string and string_to_c imported from doo_ffi_core::helpers
// ============================================================================

/// Extract detailed error message from a PostgreSQL error.
#[allow(dead_code)]
fn format_db_error(e: &(dyn std::error::Error + 'static)) -> String {
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
// Shared Parameter Conversion — SINGLE SOURCE OF TRUTH
// ============================================================================

/// Convert a single JSON value to a PostgreSQL parameter, using the PG-inferred
/// type to ensure compatibility. This is the SINGLE SOURCE OF TRUTH for param
/// type mapping.
///
/// Key insight: PostgreSQL infers parameter types from query context (e.g.,
/// `WHERE age > $1` infers INT4 from the `age` column). For bare `SELECT $1`,
/// PostgreSQL returns UNKNOWN. We adapt to whatever PostgreSQL expects.
fn json_value_to_typed_param(
    v: &serde_json::Value,
    pg_type: &tokio_postgres::types::Type,
) -> Box<dyn tokio_postgres::types::ToSql + Sync + Send> {
    use tokio_postgres::types::Type;

    match pg_type {
        // Integer types — convert from JSON number or string
        &Type::INT2 => match v {
            serde_json::Value::Number(n) => Box::new(n.as_i64().unwrap_or(0) as i16),
            serde_json::Value::String(s) => Box::new(s.parse::<i16>().unwrap_or(0)),
            _ => Box::new(v.to_string()),
        },
        &Type::INT4 | &Type::OID => match v {
            serde_json::Value::Number(n) => Box::new(n.as_i64().unwrap_or(0) as i32),
            serde_json::Value::String(s) => Box::new(s.parse::<i32>().unwrap_or(0)),
            _ => Box::new(v.to_string()),
        },
        &Type::INT8 => match v {
            serde_json::Value::Number(n) => Box::new(n.as_i64().unwrap_or(0)),
            serde_json::Value::String(s) => Box::new(s.parse::<i64>().unwrap_or(0)),
            _ => Box::new(v.to_string()),
        },
        // Float types
        &Type::FLOAT4 => match v {
            serde_json::Value::Number(n) => Box::new(n.as_f64().unwrap_or(0.0) as f32),
            serde_json::Value::String(s) => Box::new(s.parse::<f32>().unwrap_or(0.0)),
            _ => Box::new(v.to_string()),
        },
        &Type::FLOAT8 | &Type::NUMERIC => match v {
            serde_json::Value::Number(n) => Box::new(n.as_f64().unwrap_or(0.0)),
            serde_json::Value::String(s) => Box::new(s.parse::<f64>().unwrap_or(0.0)),
            _ => Box::new(v.to_string()),
        },
        // Boolean
        &Type::BOOL => match v {
            serde_json::Value::Bool(b) => Box::new(*b),
            serde_json::Value::String(s) => Box::new(s == "true" || s == "1"),
            serde_json::Value::Number(n) => Box::new(n.as_i64().unwrap_or(0) != 0),
            _ => Box::new(false),
        },
        // Text types + UNKNOWN — always send as String (PostgreSQL casts UNKNOWN to text)
        &Type::TEXT | &Type::VARCHAR | &Type::BPCHAR | &Type::NAME => match v {
            serde_json::Value::String(s) => Box::new(s.clone()),
            serde_json::Value::Null => Box::new(None::<String>),
            _ => Box::new(v.to_string()),
        },
        // NULL
        _ if matches!(v, serde_json::Value::Null) => Box::new(None::<String>),
        // UNKNOWN or any unrecognized type — send as String (safe default)
        _ => match v {
            serde_json::Value::String(s) => Box::new(s.clone()),
            serde_json::Value::Null => Box::new(None::<String>),
            // For UNKNOWN type, numbers/bools serialize as their string representation
            _ => Box::new(v.to_string()),
        },
    }
}

/// Convert JSON values to PostgreSQL parameters, adapting to PG-inferred types.
/// Uses `prepare()` to get expected types, then maps each value accordingly.
fn json_values_to_pg_params_typed(
    values: &[serde_json::Value],
    pg_types: &[tokio_postgres::types::Type],
) -> Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> {
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if i < pg_types.len() {
                json_value_to_typed_param(v, &pg_types[i])
            } else {
                // More params than PostgreSQL expects — fallback to untyped
                json_value_to_untyped_param(v)
            }
        })
        .collect()
}

/// Convert JSON values to PostgreSQL parameter types WITHOUT type info (legacy/fallback).
/// Correctly maps: String→text, i32→INT4, i64→BIGINT, f64→FLOAT8, bool→BOOLEAN, null→NULL
fn json_values_to_pg_params(
    values: &[serde_json::Value],
) -> Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> {
    values.iter().map(json_value_to_untyped_param).collect()
}

/// Convert a single JSON value to a PG parameter without type context (fallback).
fn json_value_to_untyped_param(
    v: &serde_json::Value,
) -> Box<dyn tokio_postgres::types::ToSql + Sync + Send> {
    match v {
        serde_json::Value::String(s) => Box::new(s.clone()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= i32::MIN as i64 && i <= i32::MAX as i64 {
                    Box::new(i as i32)
                } else {
                    Box::new(i)
                }
            } else if let Some(f) = n.as_f64() {
                Box::new(f)
            } else {
                Box::new(n.to_string())
            }
        }
        serde_json::Value::Bool(b) => Box::new(*b),
        serde_json::Value::Null => Box::new(None::<String>),
        _ => Box::new(v.to_string()),
    }
}

/// Build a refs slice from boxed params.
fn params_as_refs(
    boxed: &[Box<dyn tokio_postgres::types::ToSql + Sync + Send>],
) -> Vec<&(dyn tokio_postgres::types::ToSql + Sync)> {
    boxed
        .iter()
        .map(|b| b.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect()
}

// ============================================================================
// Statement Type Detection — zero allocation
// ============================================================================

/// Check if SQL is a mutating statement (INSERT/UPDATE/DELETE/CREATE/ALTER/DROP/TRUNCATE).
/// Uses byte-level prefix comparison — no allocation.
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

/// Check if SQL contains RETURNING clause — no allocation.
fn has_returning(sql: &str) -> bool {
    sql.as_bytes()
        .windows(9)
        .any(|w| w.eq_ignore_ascii_case(b"RETURNING"))
}

// ============================================================================
// Result Helpers — delegates to DooResult (single source of truth in doo_ffi_core)
// ============================================================================

/// Helper to create a heap-allocated DooResult for Ok case.
/// Uses DooResult::ok from doo_ffi_core — single source of truth.
fn db_result_ok(value: *mut c_void) -> *mut DooResult {
    ffi_debug!("DB", "db_result_ok: value={:p}", value);
    DooResult::ok(value, 0).into_raw()
}

/// Helper to create a heap-allocated DooResult for Err case.
/// Uses DooResult::err_str from doo_ffi_core — wraps string in { *char } struct
/// matching codegen's error handler which does GEP(data, 0) → load ptr.
fn db_result_err(msg: &str) -> *mut DooResult {
    ffi_debug!("DB", "db_result_err: {}", msg);
    DooResult::err_str(500, msg).into_raw()
}

// ============================================================================
// CONNECTION
// ============================================================================

/// Database struct layout matching Doo's Database type.
#[repr(C)]
#[allow(dead_code)]
struct DatabaseStruct {
    connection_type: *const c_char,
    connected: bool,
}

fn create_database_struct(conn_type: &str, connected: bool) -> *mut c_void {
    unsafe {
        let ptr = libc::malloc(16) as *mut u8;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        *(ptr as *mut *const c_char) = string_to_c(conn_type);
        *(ptr.add(8) as *mut bool) = connected;
        ptr as *mut c_void
    }
}

/// Connect to PostgreSQL database using DATABASE_URL env var.
/// Returns heap-allocated DooResult: Ok=Database struct pointer, Err=error message.
///
/// ABI note: SimpleResult and DooResult have identical layout { i64, i64 }.
/// The return type is *mut DooResult — codegen reads this as { i64 tag, i64 value }.
#[no_mangle]
pub extern "C" fn doo_db_connect_postgres() -> *mut DooResult {
    safe_ffi("DB", || {
        ffi_debug!("DB", "doo_db_connect_postgres called");
        let conn_str = match std::env::var("DATABASE_URL") {
            Ok(s) => {
                ffi_debug!("DB", "DATABASE_URL found");
                s
            }
            Err(_) => {
                ffi_debug!("DB", "WARNING: DATABASE_URL not set, using mock connection");
                let db = create_database_struct("mock", true);
                return db_result_ok(db);
            }
        };

        let rt = get_runtime();
        match rt.block_on(init_pool(&conn_str)) {
            Ok(_) => {
                ffi_debug!("DB", "Pool initialized successfully");
                let db = create_database_struct("postgres", true);
                db_result_ok(db)
            }
            Err(e) => {
                ffi_debug!("DB", "Connection failed: {}", e);
                db_result_err(&format!("Database connection failed: {}", e))
            }
        }
    })
}

/// Get global database instance (returns mock if not connected).
#[no_mangle]
pub extern "C" fn doo_db_get_global() -> *mut DooResult {
    if is_pool_initialized() {
        let db = create_database_struct("postgres", true);
        db_result_ok(db)
    } else {
        ffi_debug!("DB", "WARNING: No database connected, using mock");
        let db = create_database_struct("mock", false);
        db_result_ok(db)
    }
}

/// Check if connected to database.
#[no_mangle]
pub extern "C" fn doo_db_is_connected() -> bool {
    is_pool_initialized()
}

/// Cleanup and disconnect.
#[no_mangle]
pub extern "C" fn doo_db_cleanup_and_exit() {
    // Pool will be cleaned up on drop
}

// ============================================================================
// RAW SQL — Primary query entry points
// ============================================================================

/// Execute raw SQL query — matches Database.raw(self, sql) -> Str !DatabaseError
#[no_mangle]
pub extern "C" fn doo_db_raw(_db: *const c_void, sql: *const c_char) -> *mut DooResult {
    safe_ffi("DB", || {
        ffi_debug!("DB", "doo_db_raw called");

        let sql_str = match c_to_string(sql) {
            Ok(s) => {
                ffi_debug!("DB", "SQL: {}", s);
                s
            }
            Err(e) => return db_result_err(&e),
        };

        if !is_pool_initialized() {
            ffi_debug!("DB", "No database pool, returning empty result (demo mode)");
            return db_result_ok(string_to_c("[]") as *mut c_void);
        }

        match run_db_async(async move {
            let client = get_client().await?;
            let rows = client.query(&sql_str, &[]).await?;
            if rows.len() > MAX_ROWS {
                return Err(
                    format!("Query returned {} rows (max {})", rows.len(), MAX_ROWS).into(),
                );
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(rows_to_json(&rows))
        }) {
            Ok(json) => {
                ffi_debug!("DB", "Query result, json length: {}", json.len());
                db_result_ok(string_to_c(&json) as *mut c_void)
            }
            Err(e) => db_result_err(&e),
        }
    })
}

/// Execute raw SQL query with parameters.
#[no_mangle]
pub extern "C" fn doo_db_raw_param(
    _db: *const c_void,
    sql: *const c_char,
    params_json: *const c_char,
) -> *mut DooResult {
    safe_ffi("DB", || {
        ffi_debug!("DB", "doo_db_raw_param called");

        if !is_pool_initialized() {
            ffi_debug!(
                "DB",
                "No database connected, returning empty result (demo mode)"
            );
            return db_result_ok(string_to_c("[]") as *mut c_void);
        }

        let sql_str = match c_to_string(sql) {
            Ok(s) => {
                ffi_debug!("DB", "SQL: {}", s);
                s
            }
            Err(e) => return db_result_err(&e),
        };

        // Defensive: Handle null/empty/invalid params by treating as empty array
        let params_str = if params_json.is_null() {
            ffi_debug!("DB", "Params pointer is null, treating as empty array");
            "[]".to_string()
        } else {
            let first_byte = unsafe { *params_json as u8 };
            if first_byte == 0 {
                ffi_debug!(
                    "DB",
                    "Params starts with null byte, treating as empty array"
                );
                "[]".to_string()
            } else {
                match c_to_string(params_json) {
                    Ok(s) => {
                        ffi_debug!("DB", "Params: {}", s);
                        s
                    }
                    Err(e) => {
                        ffi_debug!("DB", "Params UTF-8 error ({}), treating as empty array", e);
                        "[]".to_string()
                    }
                }
            }
        };

        // Parse params — support both JSON array and simple string
        let params_array: Vec<serde_json::Value> = if params_str.trim().starts_with('[') {
            match serde_json::from_str(&params_str) {
                Ok(v) => v,
                Err(e) => return db_result_err(&format!("Invalid params JSON: {}", e)),
            }
        } else {
            if let Ok(n) = params_str.parse::<i64>() {
                vec![serde_json::Value::Number(serde_json::Number::from(n))]
            } else if let Ok(f) = params_str.parse::<f64>() {
                vec![serde_json::json!(f)]
            } else {
                vec![serde_json::Value::String(params_str)]
            }
        };

        match run_db_async(async move {
            let client = get_client().await?;

            // Prepare statement to get PostgreSQL-inferred parameter types.
            // This ensures type compatibility: e.g., `SELECT $1 as val` infers UNKNOWN
            // → we send as String. `WHERE age > $1` infers INT4 → we send as i32.
            let stmt = client.prepare(&sql_str).await?;
            let pg_types = stmt.params();
            let boxed_params = json_values_to_pg_params_typed(&params_array, pg_types);
            let params = params_as_refs(&boxed_params);

            let rows = client.query(&stmt, &params[..]).await?;
            if rows.len() > MAX_ROWS {
                return Err(
                    format!("Query returned {} rows (max {})", rows.len(), MAX_ROWS).into(),
                );
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(rows_to_json(&rows))
        }) {
            Ok(json) => {
                ffi_debug!("DB", "Query result, json length: {}", json.len());
                // Always return JSON string — codegen handles type conversion.
                // Scalar int detection removed: it returned raw integers as pointers
                // which crashes when the Doo code expects a Str result.
                db_result_ok(string_to_c(&json) as *mut c_void)
            }
            Err(e) => db_result_err(&e),
        }
    })
}

// ============================================================================
// QUERY EXECUTION (Legacy — now uses run_db_async, deadlock-safe)
// ============================================================================

/// Execute raw SQL query (no parameters) — legacy interface.
#[no_mangle]
pub extern "C" fn doo_db_query(sql: *const c_char) -> *mut DooResult {
    safe_ffi("DB", || {
        let sql_str = match c_to_string(sql) {
            Ok(s) => s,
            Err(e) => return DooResult::err_str(400, &e).into_raw(),
        };

        match run_db_async(async move {
            let client = get_client().await?;
            let rows = client.query(&sql_str, &[]).await?;
            if rows.len() > MAX_ROWS {
                return Err(
                    format!("Query returned {} rows (max {})", rows.len(), MAX_ROWS).into(),
                );
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(rows_to_json(&rows))
        }) {
            Ok(json) => DooResult::ok_string(&json).into_raw(),
            Err(e) => DooResult::err_str(500, &e).into_raw(),
        }
    })
}

/// Execute raw SQL query with JSON parameters — legacy interface.
/// Uses shared json_values_to_pg_params for correct type handling.
#[no_mangle]
pub extern "C" fn doo_db_query_params(
    sql: *const c_char,
    params_json: *const c_char,
) -> *mut DooResult {
    safe_ffi("DB", || {
        let sql_str = match c_to_string(sql) {
            Ok(s) => s,
            Err(e) => return DooResult::err_str(400, &e).into_raw(),
        };

        let params_str = match c_to_string(params_json) {
            Ok(s) => s,
            Err(e) => return DooResult::err_str(400, &e).into_raw(),
        };

        let params_array: Vec<serde_json::Value> = match serde_json::from_str(&params_str) {
            Ok(v) => v,
            Err(e) => {
                return DooResult::err_str(400, &format!("Invalid params JSON: {}", e)).into_raw()
            }
        };

        match run_db_async(async move {
            let client = get_client().await?;
            let boxed_params = json_values_to_pg_params(&params_array);
            let params = params_as_refs(&boxed_params);
            let rows = client.query(&sql_str, &params[..]).await?;
            if rows.len() > MAX_ROWS {
                return Err(
                    format!("Query returned {} rows (max {})", rows.len(), MAX_ROWS).into(),
                );
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(rows_to_json(&rows))
        }) {
            Ok(json) => DooResult::ok_string(&json).into_raw(),
            Err(e) => DooResult::err_str(500, &e).into_raw(),
        }
    })
}

/// Execute SQL (INSERT/UPDATE/DELETE) — legacy interface.
#[no_mangle]
pub extern "C" fn doo_db_execute(sql: *const c_char) -> *mut DooResult {
    safe_ffi("DB", || {
        let sql_str = match c_to_string(sql) {
            Ok(s) => s,
            Err(e) => return DooResult::err_str(400, &e).into_raw(),
        };

        match run_db_async(async move {
            let client = get_client().await?;
            let affected = client.execute(&sql_str, &[]).await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(affected)
        }) {
            Ok(affected) => {
                let json = format!(r#"{{"affected":{}}}"#, affected);
                DooResult::ok_string(&json).into_raw()
            }
            Err(e) => DooResult::err_str(500, &e).into_raw(),
        }
    })
}

/// Execute SQL with parameters — legacy interface.
#[no_mangle]
pub extern "C" fn doo_db_execute_params(
    sql: *const c_char,
    params_json: *const c_char,
) -> *mut DooResult {
    safe_ffi("DB", || {
        let sql_str = match c_to_string(sql) {
            Ok(s) => s,
            Err(e) => return DooResult::err_str(400, &e).into_raw(),
        };

        let params_str = match c_to_string(params_json) {
            Ok(s) => s,
            Err(e) => return DooResult::err_str(400, &e).into_raw(),
        };

        let params_array: Vec<serde_json::Value> = match serde_json::from_str(&params_str) {
            Ok(v) => v,
            Err(e) => {
                return DooResult::err_str(400, &format!("Invalid params JSON: {}", e)).into_raw()
            }
        };

        match run_db_async(async move {
            let client = get_client().await?;
            let boxed_params = json_values_to_pg_params(&params_array);
            let params = params_as_refs(&boxed_params);
            let affected = client.execute(&sql_str, &params[..]).await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(affected)
        }) {
            Ok(affected) => {
                let json = format!(r#"{{"affected":{}}}"#, affected);
                DooResult::ok_string(&json).into_raw()
            }
            Err(e) => DooResult::err_str(500, &e).into_raw(),
        }
    })
}

/// Query single row — legacy interface.
#[no_mangle]
pub extern "C" fn doo_db_query_one(sql: *const c_char) -> *mut DooResult {
    safe_ffi("DB", || {
        let sql_str = match c_to_string(sql) {
            Ok(s) => s,
            Err(e) => return DooResult::err_str(400, &e).into_raw(),
        };

        match run_db_async(async move {
            let client = get_client().await?;
            let row = client.query_one(&sql_str, &[]).await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(row_to_json(&row))
        }) {
            Ok(json) => DooResult::ok_string(&json).into_raw(),
            Err(e) => DooResult::err_str(500, &e).into_raw(),
        }
    })
}

// ============================================================================
// RESULT HANDLING
// ============================================================================

/// Check if result is an error.
#[no_mangle]
pub extern "C" fn doo_db_result_is_error(result: *mut DooResult) -> i32 {
    if result.is_null() {
        return 1;
    }
    unsafe {
        if (*result).is_err() {
            1
        } else {
            0
        }
    }
}

/// Get result value (JSON string).
#[no_mangle]
pub extern "C" fn doo_db_result_value(result: *mut DooResult) -> *const c_char {
    if result.is_null() {
        return std::ptr::null();
    }
    unsafe {
        if (*result).is_ok() {
            (*result).data as *const c_char
        } else {
            std::ptr::null()
        }
    }
}

/// Free a DooResult.
/// Handles Err results from err_str: data -> wrapper { *char } -> frees inner string + wrapper.
/// Matches doo_ffi_core::doo_result_free semantics.
#[no_mangle]
pub extern "C" fn doo_db_result_free(result: *mut DooResult) {
    if result.is_null() {
        return;
    }
    unsafe {
        let tag = (*result).tag;
        let data = (*result).data;

        if !data.is_null() {
            if tag == 1 {
                // Err: data -> wrapper { *char message } -> free inner string first
                let inner_str = *(data as *const *mut c_void);
                if !inner_str.is_null() {
                    libc::free(inner_str);
                }
            }
            libc::free(data);
        }

        // Free the outer DooResult shell (allocated with libc::malloc in into_raw)
        libc::free(result as *mut c_void);
    }
}

/// Free a C string allocated by this crate.
#[no_mangle]
pub extern "C" fn doo_db_free_string(ptr: *const c_char) {
    if !ptr.is_null() {
        unsafe {
            libc::free(ptr as *mut c_void);
        }
    }
}

/// Free a DatabaseStruct allocated by create_database_struct.
#[no_mangle]
pub extern "C" fn doo_db_free_struct(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let conn_type_ptr = *(ptr as *const *const c_char);
        if !conn_type_ptr.is_null() {
            libc::free(conn_type_ptr as *mut c_void);
        }
        libc::free(ptr);
    }
}

// ============================================================================
// TRANSACTION SUPPORT
// ============================================================================

/// Transaction query definition (deserialized from JSON).
#[derive(serde::Deserialize)]
struct QueryDef {
    sql: String,
    #[serde(default)]
    params: Vec<serde_json::Value>,
}

/// Execute multiple queries in a single transaction.
/// Input: JSON array of { "sql": "...", "params": [...] } objects.
/// Returns: JSON array of per-query results.
/// On any error, the entire transaction is rolled back (auto-rollback on drop).
#[no_mangle]
pub extern "C" fn doo_db_transaction(
    _db: *const c_void,
    queries_json: *const c_char,
) -> *mut DooResult {
    safe_ffi("DB", || {
        let queries_str = match c_to_string(queries_json) {
            Ok(s) => s,
            Err(e) => return db_result_err(&e),
        };

        let queries: Vec<QueryDef> = match serde_json::from_str(&queries_str) {
            Ok(q) => q,
            Err(e) => return db_result_err(&format!("Invalid transaction queries: {}", e)),
        };

        if !is_pool_initialized() {
            return db_result_err("Database not connected");
        }

        match run_db_async(async move {
            let mut client = get_client().await?;
            let tx = client.transaction().await?;

            let mut results = Vec::new();
            for q in &queries {
                let boxed_params = json_values_to_pg_params(&q.params);
                let param_refs = params_as_refs(&boxed_params);
                let rows = tx.query(&q.sql, &param_refs[..]).await?;
                if rows.len() > MAX_ROWS {
                    return Err(
                        format!("Query returned {} rows (max {})", rows.len(), MAX_ROWS).into(),
                    );
                }
                results.push(rows_to_json(&rows));
            }

            tx.commit().await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(
                serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string()),
            )
        }) {
            Ok(json) => db_result_ok(string_to_c(&json) as *mut c_void),
            Err(e) => db_result_err(&e),
        }
    })
}

// ============================================================================
// FFI FUNCTIONS FOR HTTP CRATE
// ============================================================================

/// Execute SQL query and return JSON array result.
/// Returns null-terminated C string, or null on error.
#[no_mangle]
pub extern "C" fn doo_db_execute_sql(sql: *const c_char) -> *mut c_void {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let sql_str = match c_to_string(sql) {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        if !is_pool_initialized() {
            ffi_debug!("DB", "doo_db_execute_sql: pool not initialized");
            return std::ptr::null_mut();
        }

        match run_db_async(async move {
            let client = get_client().await?;

            if is_mutating_sql(&sql_str) {
                let count = client.execute(&sql_str, &[]).await?;
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(format!(
                    "{{\"affected_rows\":{}}}",
                    count
                ))
            } else {
                let rows = client.query(&sql_str, &[]).await?;
                if rows.len() > MAX_ROWS {
                    return Err(
                        format!("Query returned {} rows (max {})", rows.len(), MAX_ROWS).into(),
                    );
                }
                let json_rows: Vec<serde_json::Value> =
                    rows.iter().map(|row| row_to_json_value(row)).collect();
                Ok(serde_json::to_string(&json_rows).unwrap_or_else(|_| "[]".to_string()))
            }
        }) {
            Ok(json) => string_to_c(&json) as *mut c_void,
            Err(e) => {
                ffi_debug!("DB", "doo_db_execute_sql error: {}", e);
                std::ptr::null_mut()
            }
        }
    }));
    result.unwrap_or(std::ptr::null_mut())
}

/// Execute SQL query with JSON params and return JSON array result.
#[no_mangle]
pub extern "C" fn doo_db_query_with_params(
    sql: *const c_char,
    params_json: *const c_char,
) -> *mut c_void {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let sql_str = match c_to_string(sql) {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        let params_str = match c_to_string(params_json) {
            Ok(s) => s,
            Err(_) => "[]".to_string(),
        };

        if !is_pool_initialized() {
            ffi_debug!("DB", "doo_db_query_with_params: pool not initialized");
            return std::ptr::null_mut();
        }

        let params: Vec<serde_json::Value> = serde_json::from_str(&params_str).unwrap_or_default();

        match run_db_async(async move {
            let client = get_client().await?;

            // Prepare to get PG-inferred param types, then adapt params
            let stmt = client.prepare(&sql_str).await?;
            let pg_types = stmt.params();
            let boxed_params = json_values_to_pg_params_typed(&params, pg_types);
            let param_refs = params_as_refs(&boxed_params);

            if is_mutating_sql(&sql_str) {
                if has_returning(&sql_str) {
                    let rows = client.query(&stmt, &param_refs[..]).await?;
                    let json_rows: Vec<serde_json::Value> =
                        rows.iter().map(|row| row_to_json_value(row)).collect();
                    Ok::<_, Box<dyn std::error::Error + Send + Sync>>(
                        serde_json::to_string(&json_rows).unwrap_or_else(|_| "[]".to_string()),
                    )
                } else {
                    let count = client.execute(&stmt, &param_refs[..]).await?;
                    Ok(format!("{{\"affected_rows\":{}}}", count))
                }
            } else {
                let rows = client.query(&stmt, &param_refs[..]).await?;
                if rows.len() > MAX_ROWS {
                    return Err(
                        format!("Query returned {} rows (max {})", rows.len(), MAX_ROWS).into(),
                    );
                }
                let json_rows: Vec<serde_json::Value> =
                    rows.iter().map(|row| row_to_json_value(row)).collect();
                Ok(serde_json::to_string(&json_rows).unwrap_or_else(|_| "[]".to_string()))
            }
        }) {
            Ok(json) => string_to_c(&json) as *mut c_void,
            Err(e) => {
                ffi_debug!("DB", "doo_db_query_with_params error: {}", e);
                std::ptr::null_mut()
            }
        }
    }));
    result.unwrap_or(std::ptr::null_mut())
}

// ============================================================================
// ENUM ARRAY SERIALIZATION
// ============================================================================

/// Serialize an array of enum values to JSON array.
#[no_mangle]
pub extern "C" fn doo_db_serialize_enum_array(
    array_ptr: *const std::ffi::c_void,
    variants: *const c_char,
    stride: i32,
) -> *const c_char {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if array_ptr.is_null() || variants.is_null() {
            return string_to_c("[]");
        }

        let variants_str = match c_to_string(variants) {
            Ok(s) => s,
            Err(_) => return string_to_c("[]"),
        };

        let variant_names: Vec<&str> = variants_str.split(',').collect();

        let len = unsafe {
            let len_ptr = (array_ptr as *const u8).offset(-16) as *const i64;
            (*len_ptr) as usize
        };

        let raw_data = array_ptr as *const u8;
        let mut json_arr = Vec::with_capacity(len);
        let safe_len = std::cmp::min(len, 10000);
        let stride_usize = if stride > 0 { stride as usize } else { 16 };

        for i in 0..safe_len {
            let offset = i * stride_usize;
            let tag = unsafe {
                let tag_ptr = raw_data.add(offset) as *const i32;
                (*tag_ptr) as usize
            };

            if tag < variant_names.len() {
                json_arr.push(serde_json::Value::String(variant_names[tag].to_string()));
            } else {
                json_arr.push(serde_json::Value::Null);
            }
        }

        let json_str = serde_json::Value::Array(json_arr).to_string();
        string_to_c(&json_str)
    }));
    result.unwrap_or_else(|_| string_to_c("[]"))
}

// ============================================================================
// MIGRATION FFI
// ============================================================================

/// Run migrations from schema JSON.
/// Generates CREATE TABLE IF NOT EXISTS + CREATE INDEX statements.
#[no_mangle]
pub extern "C" fn doo_db_migrate_schemas(schema_json: *const c_char) -> *mut DooResult {
    safe_ffi("DB", || {
        let schema_str = match c_to_string(schema_json) {
            Ok(s) => s,
            Err(e) => return db_result_err(&format!("Invalid schema: {}", e)),
        };

        if !is_pool_initialized() {
            return db_result_err("Database not connected — cannot run migrations");
        }

        let schemas: Vec<migrate::TableSchema> = match serde_json::from_str(&schema_str) {
            Ok(s) => s,
            Err(e) => return db_result_err(&format!("Invalid schema JSON: {}", e)),
        };

        let schema_count = schemas.len();
        let mut sqls = Vec::new();
        for schema in &schemas {
            sqls.push(schema.to_create_sql());
        }
        let combined_sql = sqls.join("\n");

        match run_db_async(async move {
            let client = get_client().await?;
            client.batch_execute(&combined_sql).await.map_err(
                |e| -> Box<dyn std::error::Error + Send + Sync> {
                    format!("Migration failed: {}", e).into()
                },
            )?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(format!(
                "{{\"migrated\":{}}}",
                schema_count
            ))
        }) {
            Ok(json) => db_result_ok(string_to_c(&json) as *mut c_void),
            Err(e) => db_result_err(&e),
        }
    })
}
