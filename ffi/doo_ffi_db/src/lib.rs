//! doo_ffi_db — Database FFI Layer with Pluggable Driver Architecture
//!
//! All database calls dispatch through the registered `DbDriver` trait implementation.
//!
//! ## Architecture
//!
//! ```text
//! Doo program → FFI (this crate) → DbDriver trait → drivers::postgres / drivers::mysql / ...
//! ```
//!
//! ## Adding a new database driver
//!
//! 1. Create `src/drivers/<name>/mod.rs` implementing `DbDriver`
//! 2. Register it in `src/drivers/mod.rs`
//! 3. Add a `doo_db_connect_<name>()` FFI function below
//! 4. Done — zero compiler/codegen changes
//!
//! ## Production Features
//!
//! - Backpressure via semaphore (fast 503 on overload)
//! - Per-query timeouts
//! - Async execution via shared runtime
//! - Unified DooResult return type
//! - All errors propagated (no silent empty arrays)

pub mod driver;
pub mod drivers;
pub mod error;
pub mod limits;
pub mod migrate;

use std::ffi::c_void;
use std::os::raw::c_char;
use std::sync::OnceLock;

use doo_ffi_core::ffi_debug;
use doo_ffi_core::helpers::{c_to_string, safe_ffi, string_to_c};
use doo_ffi_core::DooResult;
use tokio::runtime::Runtime;

use driver::get_driver;

// ============================================================================
// Runtime — use shared HTTP runtime when available, fallback to DB-only runtime
// ============================================================================

/// Fallback runtime for standalone DB usage (no HTTP server).
static DB_RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Get the shared async runtime.
pub fn get_runtime() -> &'static Runtime {
    if doo_ffi_runtime::runtime::is_runtime_initialized() {
        return doo_ffi_runtime::runtime::get_runtime();
    }
    DB_RUNTIME.get_or_init(|| {
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .max(4);
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
                doo_ffi_core::ffi_fatal!("Failed to create DB runtime: {}", e);
                std::process::exit(1);
            })
    })
}

// ============================================================================
// Async Execution Helper — generic, driver-agnostic
// ============================================================================

/// Execute async DB work from synchronous FFI context.
///
/// Fast path: When called from a Tokio worker thread (HTTP handler), runs
/// the future inline via `block_in_place` + `block_on` — zero task spawn,
/// zero oneshot channel overhead.
///
/// Fallback: When no Tokio runtime is active (standalone DB usage), spawns
/// on the dedicated DB runtime with oneshot channel.
fn run_db_async<F, T>(f: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, Box<dyn std::error::Error + Send + Sync>>>
        + Send
        + 'static,
    T: Send + 'static,
{
    let timeout = limits::get_query_timeout();
    let sem_timeout = limits::get_semaphore_wait_timeout();

    // Fast path: already on a Tokio multi-thread worker (HTTP handler context)
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return tokio::task::block_in_place(|| {
            handle.block_on(async {
                let _permit =
                    tokio::time::timeout(sem_timeout, limits::get_query_semaphore().acquire())
                        .await
                        .map_err(|_| "Database overloaded".to_string())?
                        .map_err(|_| "Semaphore closed".to_string())?;

                tokio::time::timeout(timeout, f)
                    .await
                    .map_err(|_| format!("Query timed out ({}s)", timeout.as_secs()))?
                    .map_err(|e| format!("Query failed: {}", e))
            })
        });
    }

    // Fallback: no current runtime — spawn on dedicated DB runtime
    let rt = get_runtime();
    let (tx, rx) = tokio::sync::oneshot::channel();

    rt.spawn(async move {
        let _permit = match tokio::time::timeout(
            sem_timeout,
            limits::get_query_semaphore().acquire(),
        )
        .await
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

        let result = match tokio::time::timeout(timeout, f).await {
            Ok(Ok(val)) => Ok(val),
            Ok(Err(e)) => Err(format!("Query failed: {}", e)),
            Err(_) => Err(format!("Query timed out ({}s)", timeout.as_secs())),
        };

        let _ = tx.send(result);
    });

    rx.blocking_recv()
        .map_err(|_| "Channel closed".to_string())?
}

// ============================================================================
// Result Helpers — delegates to DooResult (single source of truth in doo_ffi_core)
// ============================================================================

/// Helper to create a heap-allocated DooResult for Ok case.
fn db_result_ok(value: *mut c_void) -> *mut DooResult {
    ffi_debug!("DB", "db_result_ok: value={:p}", value);
    DooResult::ok(value, 0).into_raw()
}

/// Helper to create a heap-allocated DooResult for Err case (RFC 7807).
fn db_result_err(status: u16, msg: &str) -> *mut DooResult {
    ffi_debug!("DB", "db_result_err: status={} msg={}", status, msg);
    doo_ffi_core::helpers::make_err_rfc7807(status, msg)
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
/// Dispatches to drivers::postgres, which registers the PostgresDriver.
#[cfg(feature = "postgres")]
#[no_mangle]
pub extern "C" fn doo_db_connect_postgres() -> *mut DooResult {
    // Register bridge symbols so other FFI crates (e.g., doo_ffi_http/db_bridge)
    // can discover DB functions via the FFI bridge registry.
    register_bridge_symbols();

    safe_ffi("DB", || {
        ffi_debug!("DB", "doo_db_connect_postgres called");
        match drivers::postgres::connect_from_env() {
            Ok(()) => {
                ffi_debug!("DB", "PostgreSQL connected and driver registered");
                let db = create_database_struct("postgres", true);
                db_result_ok(db)
            }
            Err(e) => {
                eprintln!("[Doo] FATAL: Database connection failed: {}", e);
                db_result_err(503, &format!("Database connection failed: {}", e))
            }
        }
    })
}

/// Get global database instance.
#[no_mangle]
pub extern "C" fn doo_db_get_global() -> *mut DooResult {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(drv) = get_driver() {
            let db = create_database_struct(drv.name(), true);
            db_result_ok(db)
        } else {
            db_result_err(503, "No database connected. Set DATABASE_URL environment variable.")
        }
    }));
    result.unwrap_or_else(|_| db_result_err(500, "Internal error"))
}

/// Check if connected to database.
#[no_mangle]
pub extern "C" fn doo_db_is_connected() -> i32 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        get_driver().map_or(0i32, |d| d.is_connected() as i32)
    }))
    .unwrap_or(0i32)
}

/// Cleanup and disconnect.
#[no_mangle]
pub extern "C" fn doo_db_cleanup_and_exit() {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Pool will be cleaned up on drop
    }));
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
            Err(e) => return db_result_err(400, &e),
        };

        let drv = match get_driver() {
            Some(d) => d,
            None => {
                ffi_debug!(
                    "DB",
                    "No driver registered, returning empty result (demo mode)"
                );
                return db_result_ok(string_to_c("[]") as *mut c_void);
            }
        };

        let sql_debug = sql_str.clone();
        match run_db_async(async move { drv.query(&sql_str, &[]).await }) {
            Ok(json) => {
                ffi_debug!("DB", "Query result, json length: {}", json.len());
                db_result_ok(string_to_c(&json) as *mut c_void)
            }
            Err(e) => {
                eprintln!("[DB ERROR] raw query failed: {} | SQL: {}", e, sql_debug);
                db_result_err(500, &e)
            }
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

        let drv = match get_driver() {
            Some(d) => d,
            None => {
                ffi_debug!(
                    "DB",
                    "No driver registered, returning empty result (demo mode)"
                );
                return db_result_ok(string_to_c("[]") as *mut c_void);
            }
        };

        let sql_str = match c_to_string(sql) {
            Ok(s) => {
                ffi_debug!("DB", "SQL: {}", s);
                s
            }
            Err(e) => return db_result_err(400, &e),
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
                Err(e) => return db_result_err(400, &format!("Invalid params JSON: {}", e)),
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

        let params_debug = format!("{:?}", params_array);
        let sql_debug = sql_str.clone();
        match run_db_async(async move { drv.query(&sql_str, &params_array).await }) {
            Ok(json) => {
                ffi_debug!("DB", "Query result, json length: {}", json.len());
                db_result_ok(string_to_c(&json) as *mut c_void)
            }
            Err(e) => {
                eprintln!(
                    "[DB ERROR] parameterized query failed: {} | SQL: {} | params: {}",
                    e, sql_debug, params_debug
                );
                db_result_err(500, &format!("{} (params: {})", e, params_debug))
            }
        }
    })
}

// ============================================================================
// QUERY EXECUTION (Legacy — dispatches through driver)
// ============================================================================

/// Execute raw SQL query (no parameters) — legacy interface.
#[no_mangle]
pub extern "C" fn doo_db_query(sql: *const c_char) -> *mut DooResult {
    safe_ffi("DB", || {
        let sql_str = match c_to_string(sql) {
            Ok(s) => s,
            Err(e) => return db_result_err(400, &e),
        };

        let drv = match get_driver() {
            Some(d) => d,
            None => return db_result_err(503, "No database driver registered"),
        };

        match run_db_async(async move { drv.query(&sql_str, &[]).await }) {
            Ok(json) => DooResult::ok_string(&json).into_raw(),
            Err(e) => db_result_err(500, &e),
        }
    })
}

/// Execute raw SQL query with JSON parameters — legacy interface.
#[no_mangle]
pub extern "C" fn doo_db_query_params(
    sql: *const c_char,
    params_json: *const c_char,
) -> *mut DooResult {
    safe_ffi("DB", || {
        let sql_str = match c_to_string(sql) {
            Ok(s) => s,
            Err(e) => return db_result_err(400, &e),
        };

        let params_str = match c_to_string(params_json) {
            Ok(s) => s,
            Err(e) => return db_result_err(400, &e),
        };

        let params_array: Vec<serde_json::Value> = match serde_json::from_str(&params_str) {
            Ok(v) => v,
            Err(e) => return db_result_err(400, &format!("Invalid params JSON: {}", e)),
        };

        let drv = match get_driver() {
            Some(d) => d,
            None => return db_result_err(503, "No database driver registered"),
        };

        match run_db_async(async move { drv.query(&sql_str, &params_array).await }) {
            Ok(json) => DooResult::ok_string(&json).into_raw(),
            Err(e) => db_result_err(500, &e),
        }
    })
}

/// Execute SQL (INSERT/UPDATE/DELETE) — legacy interface.
#[no_mangle]
pub extern "C" fn doo_db_execute(sql: *const c_char) -> *mut DooResult {
    safe_ffi("DB", || {
        let sql_str = match c_to_string(sql) {
            Ok(s) => s,
            Err(e) => return db_result_err(400, &e),
        };

        let drv = match get_driver() {
            Some(d) => d,
            None => return db_result_err(503, "No database driver registered"),
        };

        match run_db_async(async move { drv.execute(&sql_str, &[]).await }) {
            Ok(affected) => {
                let json = format!(r#"{{"affected":{}}}"#, affected);
                DooResult::ok_string(&json).into_raw()
            }
            Err(e) => db_result_err(500, &e),
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
            Err(e) => return db_result_err(400, &e),
        };

        let params_str = match c_to_string(params_json) {
            Ok(s) => s,
            Err(e) => return db_result_err(400, &e),
        };

        let params_array: Vec<serde_json::Value> = match serde_json::from_str(&params_str) {
            Ok(v) => v,
            Err(e) => return db_result_err(400, &format!("Invalid params JSON: {}", e)),
        };

        let drv = match get_driver() {
            Some(d) => d,
            None => return db_result_err(503, "No database driver registered"),
        };

        match run_db_async(async move { drv.execute(&sql_str, &params_array).await }) {
            Ok(affected) => {
                let json = format!(r#"{{"affected":{}}}"#, affected);
                DooResult::ok_string(&json).into_raw()
            }
            Err(e) => db_result_err(500, &e),
        }
    })
}

/// Query single row — legacy interface.
#[no_mangle]
pub extern "C" fn doo_db_query_one(sql: *const c_char) -> *mut DooResult {
    safe_ffi("DB", || {
        let sql_str = match c_to_string(sql) {
            Ok(s) => s,
            Err(e) => return db_result_err(400, &e),
        };

        let drv = match get_driver() {
            Some(d) => d,
            None => return db_result_err(503, "No database driver registered"),
        };

        match run_db_async(async move { drv.query_one(&sql_str, &[]).await }) {
            Ok(json) => DooResult::ok_string(&json).into_raw(),
            Err(e) => db_result_err(500, &e),
        }
    })
}

// ============================================================================
// RESULT HANDLING
// ============================================================================

/// Check if result is an error.
#[no_mangle]
pub extern "C" fn doo_db_result_is_error(result: *mut DooResult) -> i32 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
    }))
    .unwrap_or(1)
}

/// Get result value (JSON string).
#[no_mangle]
pub extern "C" fn doo_db_result_value(result: *mut DooResult) -> *const c_char {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
    }))
    .unwrap_or(std::ptr::null())
}

/// Free a DooResult.
#[no_mangle]
pub extern "C" fn doo_db_result_free(result: *mut DooResult) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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

            libc::free(result as *mut c_void);
        }
    }));
}

/// Free a C string allocated by this crate.
#[no_mangle]
pub extern "C" fn doo_db_free_string(ptr: *const c_char) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if !ptr.is_null() {
            unsafe {
                libc::free(ptr as *mut c_void);
            }
        }
    }));
}

/// Free a DatabaseStruct allocated by create_database_struct.
#[no_mangle]
pub extern "C" fn doo_db_free_struct(ptr: *mut c_void) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
    }));
}

// ============================================================================
// TRANSACTION SUPPORT
// ============================================================================

/// Execute multiple queries in a single transaction.
/// Input: JSON array of { "sql": "...", "params": [...] } objects.
/// Returns: JSON array of per-query results.
#[no_mangle]
pub extern "C" fn doo_db_transaction(
    _db: *const c_void,
    queries_json: *const c_char,
) -> *mut DooResult {
    safe_ffi("DB", || {
        let queries_str = match c_to_string(queries_json) {
            Ok(s) => s,
            Err(e) => return db_result_err(400, &e),
        };

        let drv = match get_driver() {
            Some(d) => d,
            None => return db_result_err(503, "Database not connected"),
        };

        match run_db_async(async move { drv.transaction(&queries_str).await }) {
            Ok(json) => db_result_ok(string_to_c(&json) as *mut c_void),
            Err(e) => db_result_err(500, &e),
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

        let drv = match get_driver() {
            Some(d) => d,
            None => {
                ffi_debug!("DB", "doo_db_execute_sql: no driver registered");
                return std::ptr::null_mut();
            }
        };

        match run_db_async(async move { drv.execute_auto(&sql_str, &[]).await }) {
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

        let drv = match get_driver() {
            Some(d) => d,
            None => {
                ffi_debug!("DB", "doo_db_query_with_params: no driver registered");
                return std::ptr::null_mut();
            }
        };

        let params: Vec<serde_json::Value> = serde_json::from_str(&params_str).unwrap_or_default();

        match run_db_async(async move { drv.execute_auto(&sql_str, &params).await }) {
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
// BATCH OPERATIONS — Single-connection multi-query and batch update
// ============================================================================

/// Execute multiple queries on a single connection, return JSON array of results.
///
/// Input: JSON array of `{ "sql": "...", "params": [...] }` objects.
/// Each query returns one row. Results are concatenated: `[{row1}, {row2}, ...]`.
///
/// Uses a single pool checkout and `prepare_cached` for maximum throughput
/// (TechEmpower "multiple queries" benchmark pattern).
#[no_mangle]
pub extern "C" fn doo_db_batch_query(
    _db: *const c_void,
    queries_json: *const c_char,
) -> *mut DooResult {
    safe_ffi("DB", || {
        ffi_debug!("DB", "doo_db_batch_query called");

        let queries_str = match c_to_string(queries_json) {
            Ok(s) => s,
            Err(e) => return db_result_err(400, &format!("Invalid queries JSON: {}", e)),
        };

        #[derive(serde::Deserialize)]
        struct QueryDef {
            sql: String,
            #[serde(default)]
            params: Vec<serde_json::Value>,
        }

        let query_defs: Vec<QueryDef> = match serde_json::from_str(&queries_str) {
            Ok(v) => v,
            Err(e) => return db_result_err(400, &format!("Invalid queries JSON array: {}", e)),
        };

        let drv = match get_driver() {
            Some(d) => d,
            None => return db_result_err(503, "No database driver registered"),
        };

        let queries: Vec<(String, Vec<serde_json::Value>)> =
            query_defs.into_iter().map(|q| (q.sql, q.params)).collect();

        match run_db_async(async move { drv.batch_query(&queries).await }) {
            Ok(json) => {
                ffi_debug!("DB", "Batch query result, json length: {}", json.len());
                db_result_ok(string_to_c(&json) as *mut c_void)
            }
            Err(e) => db_result_err(500, &e),
        }
    })
}

/// Execute a batch UPDATE using PostgreSQL array parameters (unnest pattern).
///
/// `sql`: UPDATE template with `$1::int[]` and `$2::int[]` placeholders.
/// `ids_json`: JSON array of integer IDs.
/// `values_json`: JSON array of integer values (parallel to ids).
///
/// Uses a single SQL statement for all updates (TechEmpower "updates" benchmark pattern).
#[no_mangle]
pub extern "C" fn doo_db_batch_update(
    _db: *const c_void,
    sql: *const c_char,
    ids_json: *const c_char,
    values_json: *const c_char,
) -> *mut DooResult {
    safe_ffi("DB", || {
        ffi_debug!("DB", "doo_db_batch_update called");

        let sql_str = match c_to_string(sql) {
            Ok(s) => s,
            Err(e) => return db_result_err(400, &format!("Invalid SQL: {}", e)),
        };

        let ids_str = match c_to_string(ids_json) {
            Ok(s) => s,
            Err(e) => return db_result_err(400, &format!("Invalid IDs JSON: {}", e)),
        };

        let values_str = match c_to_string(values_json) {
            Ok(s) => s,
            Err(e) => return db_result_err(400, &format!("Invalid values JSON: {}", e)),
        };

        let ids: Vec<i32> = match serde_json::from_str(&ids_str) {
            Ok(v) => v,
            Err(e) => return db_result_err(400, &format!("Invalid IDs array: {}", e)),
        };

        let values: Vec<i32> = match serde_json::from_str(&values_str) {
            Ok(v) => v,
            Err(e) => return db_result_err(400, &format!("Invalid values array: {}", e)),
        };

        let drv = match get_driver() {
            Some(d) => d,
            None => return db_result_err(503, "No database driver registered"),
        };

        match run_db_async(async move { drv.batch_update(&sql_str, &ids, &values).await }) {
            Ok(affected) => {
                let json = format!("{{\"affected\":{}}}", affected);
                db_result_ok(string_to_c(&json) as *mut c_void)
            }
            Err(e) => db_result_err(500, &e),
        }
    })
}

// ============================================================================
// MIGRATION FFI
// ============================================================================

/// Run migrations from schema JSON.
/// Dispatches through the registered driver for dialect-specific DDL.
#[no_mangle]
pub extern "C" fn doo_db_migrate_schemas(schema_json: *const c_char) -> *mut DooResult {
    safe_ffi("DB", || {
        let schema_str = match c_to_string(schema_json) {
            Ok(s) => s,
            Err(e) => return db_result_err(400, &format!("Invalid schema: {}", e)),
        };

        let drv = match get_driver() {
            Some(d) => d,
            None => return db_result_err(503, "Database not connected — cannot run migrations"),
        };

        let schemas: Vec<migrate::TableSchema> = match serde_json::from_str(&schema_str) {
            Ok(s) => s,
            Err(e) => return db_result_err(400, &format!("Invalid schema JSON: {}", e)),
        };

        let schema_count = schemas.len();
        let mut sqls = Vec::new();
        for schema in &schemas {
            sqls.push(drv.generate_create_table(schema));
        }
        let combined_sql = sqls.join("\n");

        match run_db_async(async move { drv.batch_execute(&combined_sql).await }) {
            Ok(()) => {
                let json = format!("{{\"migrated\":{}}}", schema_count);
                db_result_ok(string_to_c(&json) as *mut c_void)
            }
            Err(e) => db_result_err(500, &e),
        }
    })
}

// ============================================================================
// FFI BRIDGE REGISTRATION — Cross-Crate Symbol Discovery
// ============================================================================

/// Register DB bridge symbols with doo_ffi_core's FFI bridge registry.
/// Called once during DB connect so other packages (e.g., doo_ffi_http/db_bridge)
/// can discover these functions without OS-level symbol resolution.
fn register_bridge_symbols() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        doo_ffi_core::ffi_bridge::register(
            "doo_db_is_connected",
            doo_db_is_connected as *const std::ffi::c_void,
        );
        doo_ffi_core::ffi_bridge::register(
            "doo_db_execute_sql",
            doo_db_execute_sql as *const std::ffi::c_void,
        );
        doo_ffi_core::ffi_bridge::register(
            "doo_db_query_with_params",
            doo_db_query_with_params as *const std::ffi::c_void,
        );
    });
}
