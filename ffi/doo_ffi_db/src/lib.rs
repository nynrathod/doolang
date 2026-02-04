//! doo_ffi_db - Complete Database FFI Library
//!
//! Provides:
//! - Connection pooling with deadpool-postgres
//! - Raw SQL queries with/without parameters
//! - JSON result serialization
//! - Migrations and table metadata

mod json_utils;
mod pool;

use std::ffi::c_void;
use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::OnceLock;

use doo_ffi_core::DooResult;
use tokio::runtime::Runtime;

pub use json_utils::*;
pub use pool::*;

// Tokio runtime for async operations
static RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn get_runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime")
    })
}

// ============================================================================
// String Helpers
// ============================================================================

fn c_to_string(s: *const c_char) -> Result<String, String> {
    if s.is_null() {
        return Err("Null pointer".to_string());
    }
    unsafe {
        CStr::from_ptr(s)
            .to_str()
            .map(|s| s.to_string())
            .map_err(|e| format!("Invalid UTF-8: {}", e))
    }
}

fn string_to_c(s: &str) -> *const c_char {
    unsafe {
        let len = s.len();
        let ptr = libc::malloc(len + 1) as *mut u8;
        if ptr.is_null() {
            return std::ptr::null();
        }
        std::ptr::copy_nonoverlapping(s.as_ptr(), ptr, len);
        *ptr.add(len) = 0;
        ptr as *const c_char
    }
}

// ============================================================================
// CONNECTION
// ============================================================================

/// Database struct layout matching Doo's Database type
/// Fields: ConnectionType: Str, Connected: Bool
#[repr(C)]
struct DatabaseStruct {
    connection_type: *const c_char, // ptr to "postgres"
    connected: bool,                // i1 in LLVM
}

/// Result struct that matches LLVM codegen expectations: { i64 tag, i64 value }
/// tag = 0 for Ok, tag = 1 for Err
/// IMPORTANT: On Windows x64 MSVC, structs > 8 bytes are returned via sret (hidden pointer).
/// To avoid ABI mismatches between Rust and LLVM, we return a POINTER to a heap-allocated result
/// instead of returning the struct by value. The caller must free the result using doo_db_result_free.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SimpleResult {
    pub tag: i64,
    pub value: i64, // Pointer stored as i64 for ABI compatibility
}

/// Helper to create and return a heap-allocated SimpleResult for Ok case
fn simple_result_ok_ptr(value: *mut c_void) -> *mut SimpleResult {
    let result = Box::new(SimpleResult {
        tag: 0, // Ok
        value: value as i64,
    });
    eprintln!(
        "[DB_FFI] simple_result_ok: tag={}, value=0x{:x}",
        result.tag, result.value
    );
    Box::into_raw(result)
}

/// Helper to create and return a heap-allocated SimpleResult for Err case
fn simple_result_err_ptr(error_msg: &str) -> *mut SimpleResult {
    eprintln!("[DB_FFI] simple_result_err called with: {}", error_msg);
    let ptr = string_to_c(error_msg);
    eprintln!("[DB_FFI] string_to_c returned: {:?}", ptr);
    let result = Box::new(SimpleResult {
        tag: 1, // Err
        value: ptr as i64,
    });
    Box::into_raw(result)
}

/// Connect to PostgreSQL database using DATABASE_URL env var
/// Returns a POINTER to a heap-allocated Result struct to avoid Windows x64 ABI issues.
/// The caller must free the result using doo_db_result_free.
/// - Ok: tag=0, value=Database struct pointer
/// - Err: tag=1, value=error message string pointer
#[no_mangle]
pub extern "C" fn doo_db_connect_postgres() -> *mut SimpleResult {
    eprintln!("[DB_FFI] doo_db_connect_postgres called");
    let conn_str = match std::env::var("DATABASE_URL") {
        Ok(s) => {
            eprintln!("[DB_FFI] DATABASE_URL found: {}", s);
            s
        }
        Err(_) => {
            // For development/testing, create a mock database connection
            // In production, this should fail or use a default URL
            eprintln!("[DB_FFI] WARNING: DATABASE_URL not set, using mock connection");
            let db = create_database_struct("mock", true);
            return simple_result_ok_ptr(db);
        }
    };

    eprintln!("[DB_FFI] Creating tokio runtime...");
    let rt = get_runtime();
    eprintln!("[DB_FFI] Calling init_pool...");
    match rt.block_on(init_pool(&conn_str)) {
        Ok(_) => {
            eprintln!("[DB_FFI] Pool initialized successfully");
            let db = create_database_struct("postgres", true);
            simple_result_ok_ptr(db)
        }
        Err(e) => {
            eprintln!("[DB] Connection failed: {}", e);
            simple_result_err_ptr(&format!("Database connection failed: {}", e))
        }
    }
}

/// Get global database instance (returns mock if not connected)
/// Returns a POINTER to a heap-allocated Result struct.
#[no_mangle]
pub extern "C" fn doo_db_get_global() -> *mut SimpleResult {
    if is_pool_initialized() {
        let db = create_database_struct("postgres", true);
        simple_result_ok_ptr(db)
    } else {
        eprintln!("[DB] WARNING: No database connected, using mock");
        let db = create_database_struct("mock", false);
        simple_result_ok_ptr(db)
    }
}

fn create_database_struct(conn_type: &str, connected: bool) -> *mut c_void {
    unsafe {
        // Allocate Database struct: { ptr, i1 } aligned properly
        // In LLVM this is typically 16 bytes due to padding
        let ptr = libc::malloc(16) as *mut u8;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        // Store ConnectionType (ptr) at offset 0
        *(ptr as *mut *const c_char) = string_to_c(conn_type);
        // Store Connected (bool) at offset 8
        *(ptr.add(8) as *mut bool) = connected;
        ptr as *mut c_void
    }
}

/// Check if connected to database
#[no_mangle]
pub extern "C" fn doo_db_is_connected() -> bool {
    is_pool_initialized()
}

/// Cleanup and disconnect
#[no_mangle]
pub extern "C" fn doo_db_cleanup_and_exit() {
    // Pool will be cleaned up on drop
}

// ============================================================================
// RAW SQL (Returns SimpleResult by value for Result<Str, Error> types)
// ============================================================================

/// Execute raw SQL query - matches Database.raw(self, sql) -> Str !DatabaseError
/// First argument (_db) is the database handle (self), second is the SQL string
/// Returns pointer to heap-allocated SimpleResult (avoids Windows sret ABI issue)
#[no_mangle]
pub extern "C" fn doo_db_raw(_db: *const c_void, sql: *const c_char) -> *mut SimpleResult {
    eprintln!("[DB_FFI] doo_db_raw called");
    let sql_str = match c_to_string(sql) {
        Ok(s) => {
            eprintln!("[DB_FFI] SQL: {}", s);
            s
        }
        Err(e) => {
            eprintln!("[DB_FFI] Failed to convert SQL string: {}", e);
            return simple_result_err_ptr(&e);
        }
    };

    let rt = get_runtime();
    match rt.block_on(async {
        let client = get_client().await?;
        let rows = client.query(&sql_str, &[]).await?;
        Ok::<_, Box<dyn std::error::Error>>(rows_to_json(&rows))
    }) {
        Ok(json) => {
            eprintln!("[DB_FFI] Query succeeded, json length: {}", json.len());
            simple_result_ok_ptr(string_to_c(&json) as *mut c_void)
        }
        Err(e) => {
            eprintln!("[DB_FFI] Query failed: {}", e);
            simple_result_err_ptr(&format!("Query failed: {}", e))
        }
    }
}

/// Execute raw SQL query with parameters - matches Database.rawWithParams(self, sql, params) -> Str !DatabaseError
/// First arg (_db) is database handle, second is SQL, third is JSON params
/// Returns pointer to heap-allocated SimpleResult (avoids Windows sret ABI issue)
#[no_mangle]
pub extern "C" fn doo_db_raw_param(
    _db: *const c_void,
    sql: *const c_char,
    params_json: *const c_char,
) -> *mut SimpleResult {
    let sql_str = match c_to_string(sql) {
        Ok(s) => s,
        Err(e) => return simple_result_err_ptr(&e),
    };

    let params_str = match c_to_string(params_json) {
        Ok(s) => s,
        Err(e) => return simple_result_err_ptr(&e),
    };

    // Parse params - support both JSON array and simple string
    // If it's a JSON array like ["Bob", 25], parse it
    // If it's a simple string like "Bob", wrap it in an array
    let params_array: Vec<serde_json::Value> = if params_str.trim().starts_with('[') {
        match serde_json::from_str(&params_str) {
            Ok(v) => v,
            Err(e) => return simple_result_err_ptr(&format!("Invalid params JSON: {}", e)),
        }
    } else {
        // Single value - try to parse as number first, otherwise treat as string
        if let Ok(n) = params_str.parse::<i64>() {
            vec![serde_json::Value::Number(serde_json::Number::from(n))]
        } else if let Ok(f) = params_str.parse::<f64>() {
            vec![serde_json::json!(f)]
        } else {
            vec![serde_json::Value::String(params_str)]
        }
    };

    let rt = get_runtime();
    match rt.block_on(async {
        let client = get_client().await?;

        // Convert JSON values to boxed postgres params to preserve types
        // Use i32 for integers since PostgreSQL INT is 32-bit
        let boxed_params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = params_array
            .iter()
            .map(|v| -> Box<dyn tokio_postgres::types::ToSql + Sync + Send> {
                match v {
                    serde_json::Value::String(s) => Box::new(s.clone()),
                    serde_json::Value::Number(n) => {
                        // Try i32 first (for PostgreSQL INT/INT4), then i64 (BIGINT), then f64
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
            })
            .collect();

        // Build params slice from boxed values
        let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = boxed_params
            .iter()
            .map(|b| b.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client.query(&sql_str, &params[..]).await?;
        Ok::<_, Box<dyn std::error::Error>>(rows_to_json(&rows))
    }) {
        Ok(json) => simple_result_ok_ptr(string_to_c(&json) as *mut c_void),
        Err(e) => simple_result_err_ptr(&format!("Query failed: {}", e)),
    }
}

// ============================================================================
// QUERY EXECUTION (Legacy - returns *mut DooResult)
// ============================================================================

/// Execute raw SQL query (no parameters)
/// Returns: JSON array of rows
#[no_mangle]
pub extern "C" fn doo_db_query(sql: *const c_char) -> *mut DooResult {
    let sql_str = match c_to_string(sql) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };

    let rt = get_runtime();
    match rt.block_on(async {
        let client = get_client().await?;
        let rows = client.query(&sql_str, &[]).await?;
        Ok::<_, Box<dyn std::error::Error>>(rows_to_json(&rows))
    }) {
        Ok(json) => DooResult::ok_string(&json).into_raw(),
        Err(e) => DooResult::err_str(500, &format!("Query failed: {}", e)).into_raw(),
    }
}

/// Execute raw SQL query with JSON parameters
/// params_json should be a JSON array, e.g. ["value1", 123, true]
#[no_mangle]
pub extern "C" fn doo_db_query_params(
    sql: *const c_char,
    params_json: *const c_char,
) -> *mut DooResult {
    let sql_str = match c_to_string(sql) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };

    let params_str = match c_to_string(params_json) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };

    // Parse params JSON
    let params_array: Vec<serde_json::Value> = match serde_json::from_str(&params_str) {
        Ok(v) => v,
        Err(e) => {
            return DooResult::err_str(400, &format!("Invalid params JSON: {}", e)).into_raw()
        }
    };

    let rt = get_runtime();
    match rt.block_on(async {
        let client = get_client().await?;

        // Convert JSON values to postgres params (simplified - strings only for now)
        let param_refs: Vec<String> = params_array
            .iter()
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Null => "NULL".to_string(),
                _ => v.to_string(),
            })
            .collect();

        // Build params slice
        let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = param_refs
            .iter()
            .map(|s| s as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client.query(&sql_str, &params[..]).await?;
        Ok::<_, Box<dyn std::error::Error>>(rows_to_json(&rows))
    }) {
        Ok(json) => DooResult::ok_string(&json).into_raw(),
        Err(e) => DooResult::err_str(500, &format!("Query failed: {}", e)).into_raw(),
    }
}

/// Execute SQL (INSERT/UPDATE/DELETE) - no result rows
#[no_mangle]
pub extern "C" fn doo_db_execute(sql: *const c_char) -> *mut DooResult {
    let sql_str = match c_to_string(sql) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };

    let rt = get_runtime();
    match rt.block_on(async {
        let client = get_client().await?;
        let affected = client.execute(&sql_str, &[]).await?;
        Ok::<_, Box<dyn std::error::Error>>(affected)
    }) {
        Ok(affected) => {
            let json = format!(r#"{{"affected":{}}}"#, affected);
            DooResult::ok_string(&json).into_raw()
        }
        Err(e) => DooResult::err_str(500, &format!("Execute failed: {}", e)).into_raw(),
    }
}

/// Execute SQL with parameters
#[no_mangle]
pub extern "C" fn doo_db_execute_params(
    sql: *const c_char,
    params_json: *const c_char,
) -> *mut DooResult {
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

    let rt = get_runtime();
    match rt.block_on(async {
        let client = get_client().await?;

        let param_refs: Vec<String> = params_array
            .iter()
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Null => "NULL".to_string(),
                _ => v.to_string(),
            })
            .collect();

        let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = param_refs
            .iter()
            .map(|s| s as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let affected = client.execute(&sql_str, &params[..]).await?;
        Ok::<_, Box<dyn std::error::Error>>(affected)
    }) {
        Ok(affected) => {
            let json = format!(r#"{{"affected":{}}}"#, affected);
            DooResult::ok_string(&json).into_raw()
        }
        Err(e) => DooResult::err_str(500, &format!("Execute failed: {}", e)).into_raw(),
    }
}

/// Query single row
#[no_mangle]
pub extern "C" fn doo_db_query_one(sql: *const c_char) -> *mut DooResult {
    let sql_str = match c_to_string(sql) {
        Ok(s) => s,
        Err(e) => return DooResult::err_str(400, &e).into_raw(),
    };

    let rt = get_runtime();
    match rt.block_on(async {
        let client = get_client().await?;
        let row = client.query_one(&sql_str, &[]).await?;
        Ok::<_, Box<dyn std::error::Error>>(row_to_json(&row))
    }) {
        Ok(json) => DooResult::ok_string(&json).into_raw(),
        Err(e) => DooResult::err_str(500, &format!("Query failed: {}", e)).into_raw(),
    }
}

// ============================================================================
// RESULT HANDLING
// ============================================================================

/// Check if result is an error
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

/// Get result value (JSON string)
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

/// Free a result
#[no_mangle]
pub extern "C" fn doo_db_result_free(result: *mut DooResult) {
    if result.is_null() {
        return;
    }
    unsafe {
        let res = Box::from_raw(result);
        if !res.data.is_null() {
            libc::free(res.data);
        }
    }
}
