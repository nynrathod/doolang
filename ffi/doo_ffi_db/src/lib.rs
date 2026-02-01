//! doo_ffi_db - Complete Database FFI Library
//!
//! Provides:
//! - Connection pooling with deadpool-postgres
//! - Raw SQL queries with/without parameters
//! - JSON result serialization
//! - Migrations and table metadata

mod pool;
mod json_utils;

use std::ffi::CStr;
use std::ffi::c_void;
use std::os::raw::c_char;
use std::sync::OnceLock;

use tokio::runtime::Runtime;
use doo_ffi_core::DooResult;

pub use pool::*;
pub use json_utils::*;

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

/// Connect to PostgreSQL database using DATABASE_URL env var
/// Returns a pointer to a Database struct (not DooResult) for compatibility
#[no_mangle]
pub extern "C" fn doo_db_connect_postgres() -> *mut c_void {
    let conn_str = match std::env::var("DATABASE_URL") {
        Ok(s) => s,
        Err(_) => {
            // For development/testing, create a mock database connection
            // In production, this should fail or use a default URL
            eprintln!("[DB] WARNING: DATABASE_URL not set, using mock connection");
            return create_database_struct("mock", false);
        }
    };
    
    let rt = get_runtime();
    match rt.block_on(init_pool(&conn_str)) {
        Ok(_) => create_database_struct("postgres", true),
        Err(e) => {
            eprintln!("[DB] Connection failed: {}", e);
            create_database_struct("postgres", false)
        }
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
// QUERY EXECUTION
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
pub extern "C" fn doo_db_query_params(sql: *const c_char, params_json: *const c_char) -> *mut DooResult {
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
        Err(e) => return DooResult::err_str(400, &format!("Invalid params JSON: {}", e)).into_raw(),
    };
    
    let rt = get_runtime();
    match rt.block_on(async {
        let client = get_client().await?;
        
        // Convert JSON values to postgres params (simplified - strings only for now)
        let param_refs: Vec<String> = params_array.iter().map(|v| {
            match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Null => "NULL".to_string(),
                _ => v.to_string(),
            }
        }).collect();
        
        // Build params slice
        let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = 
            param_refs.iter().map(|s| s as &(dyn tokio_postgres::types::ToSql + Sync)).collect();
        
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
pub extern "C" fn doo_db_execute_params(sql: *const c_char, params_json: *const c_char) -> *mut DooResult {
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
        Err(e) => return DooResult::err_str(400, &format!("Invalid params JSON: {}", e)).into_raw(),
    };
    
    let rt = get_runtime();
    match rt.block_on(async {
        let client = get_client().await?;
        
        let param_refs: Vec<String> = params_array.iter().map(|v| {
            match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Null => "NULL".to_string(),
                _ => v.to_string(),
            }
        }).collect();
        
        let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = 
            param_refs.iter().map(|s| s as &(dyn tokio_postgres::types::ToSql + Sync)).collect();
        
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
    unsafe { if (*result).is_err() { 1 } else { 0 } }
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
