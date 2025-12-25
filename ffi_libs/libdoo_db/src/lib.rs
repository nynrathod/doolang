use once_cell::sync::OnceCell;
use serde_json::json;
use std::env;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Once;
use tokio::runtime::Runtime;
use tokio::sync::Notify;
use tokio_postgres::{Client, NoTls};

static INIT: Once = Once::new();

fn load_env() {
    INIT.call_once(|| {
        // Try to load .env file, ignore if it doesn't exist
        let _ = dotenvy::dotenv();
    });
}

// Link to centralized runtime functions
extern "C" {
    #[allow(dead_code)]
    fn dooruntime_db_error(code: *const c_char, message: *const c_char) -> *mut c_char;
    #[allow(dead_code)]
    fn dooruntime_db_error_rfc7807(
        code: *const c_char,
        message: *const c_char,
        instance: *const c_char,
        field: *const c_char,
    ) -> *mut c_char;
    #[allow(dead_code)]
    fn dooruntime_free_string(ptr: *mut c_char);
}

#[repr(C)]
pub struct DooResult {
    tag: i32,                     // 0 = Ok, 1 = Err
    value: *mut std::ffi::c_void, // pointer to data or error struct
}

#[repr(C)]
pub struct DooDbError {
    code: i32,
    message: *mut c_char,
}

#[repr(C)]
pub struct DooDatabase {
    connection_type: *mut c_char,
    connected: i32, // 0 = false, 1 = true
}

static RUNTIME: OnceCell<Runtime> = OnceCell::new();
static CLIENT: OnceCell<Arc<Client>> = OnceCell::new();
static GLOBAL_DB: OnceCell<Arc<Client>> = OnceCell::new();
static SHUTDOWN_SIGNAL: OnceCell<Arc<Notify>> = OnceCell::new();
static IS_CONNECTED: AtomicBool = AtomicBool::new(false);

fn runtime() -> Result<&'static Runtime, String> {
    RUNTIME.get_or_try_init(|| Runtime::new().map_err(|e| format!("Failed to create runtime: {e}")))
}

fn string_to_c(s: String) -> *mut c_char {
    CString::new(s)
        .map(|c| c.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

fn c_to_string(s: *const c_char) -> Result<String, String> {
    if s.is_null() {
        return Err("Null pointer".to_string());
    }
    unsafe {
        // Use to_string_lossy to handle any invalid UTF-8 sequences
        Ok(CStr::from_ptr(s).to_string_lossy().into_owned())
    }
}

fn make_ok_void() -> *mut DooResult {
    Box::into_raw(Box::new(DooResult {
        tag: 0,
        value: std::ptr::null_mut(),
    }))
}

fn make_ok_database(connection_type: String, connected: bool) -> *mut DooResult {
    let db = Box::new(DooDatabase {
        connection_type: string_to_c(connection_type),
        connected: if connected { 1 } else { 0 },
    });
    Box::into_raw(Box::new(DooResult {
        tag: 0,
        value: Box::into_raw(db) as *mut _,
    }))
}

fn make_ok_string(s: String) -> *mut DooResult {
    Box::into_raw(Box::new(DooResult {
        tag: 0,
        value: string_to_c(s) as *mut _,
    }))
}

fn make_ok_int(n: i64) -> *mut DooResult {
    Box::into_raw(Box::new(DooResult {
        tag: 0,
        value: n as *mut _,
    }))
}

fn make_err(msg: String) -> *mut DooResult {
    make_err_with_code("INTERNAL_ERROR", "XX000", msg)
}

fn make_err_with_code(code: &str, pg_code: &str, msg: String) -> *mut DooResult {
    // Sanitize message to ensure valid UTF-8
    let safe_msg = msg
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect::<String>();

    let error_json = json!({
        "success": false,
        "error": {
            "code": code,
            "pg_code": pg_code,
            "message": safe_msg,
            "status": match code {
                "UNIQUE_VIOLATION" => 409,
                "NOT_NULL_VIOLATION" | "CHECK_VIOLATION" | "DATA_TYPE_MISMATCH" => 400,
                "TABLE_NOT_FOUND" | "COLUMN_NOT_FOUND" => 404,
                "CONNECTION_FAILED" | "INTERNAL_ERROR" | _ => 500,
            }
        }
    })
    .to_string();

    let err = Box::new(DooDbError {
        code: -1,
        message: string_to_c(error_json),
    });
    Box::into_raw(Box::new(DooResult {
        tag: 1,
        value: Box::into_raw(err) as *mut _,
    }))
}

fn make_err_unique_violation(field: &str) -> *mut DooResult {
    let msg = format!("Duplicate value for field: {}", field);
    make_err_with_code("UNIQUE_VIOLATION", "23505", msg)
}

fn make_err_connection_failed(msg: String) -> *mut DooResult {
    make_err_with_code("CONNECTION_FAILED", "08006", msg)
}

fn make_err_query_failed(msg: String) -> *mut DooResult {
    make_err_with_code("QUERY_FAILED", "42601", msg)
}

fn make_err_from_pg_error(e: &tokio_postgres::Error) -> *mut DooResult {
    let default_code = "XX000".to_string();
    let (code, msg) = if let Some(db_err) = e.as_db_error() {
        let c = db_err.code().code().to_string();
        let m = db_err.message();
        let d = db_err.detail().unwrap_or("");
        let h = db_err.hint().unwrap_or("");
        let full_msg = if !d.is_empty() || !h.is_empty() {
            format!("{} (Detail: {}, Hint: {})", m, d, h)
        } else {
            m.to_string()
        };
        (c, full_msg)
    } else {
        (default_code, e.to_string())
    };
    make_err_with_code("QUERY_FAILED", &code, msg)
}

fn get_client() -> Result<Arc<Client>, String> {
    CLIENT
        .get()
        .cloned()
        .ok_or_else(|| "Database not connected. Call doo_db_connect_postgres() first".to_string())
}

#[no_mangle]
pub extern "C" fn doo_db_connect_postgres() -> *mut DooResult {
    // Load .env file on first connection attempt
    load_env();

    if CLIENT.get().is_some() {
        return make_ok_database("postgres".to_string(), true);
    }

    let database_url = match env::var("DATABASE_URL") {
        Ok(v) => v,
        Err(_) => return make_err_connection_failed("DATABASE_URL not set".to_string()),
    };

    let rt = match runtime() {
        Ok(r) => r,
        Err(e) => return make_err(e),
    };

    let connect_res = rt.block_on(tokio_postgres::connect(&database_url, NoTls));
    let (client, connection) = match connect_res {
        Ok(v) => v,
        Err(e) => return make_err_connection_failed(format!("Failed to connect: {}", e)),
    };

    // Initialize shutdown signal
    let shutdown = Arc::new(Notify::new());
    SHUTDOWN_SIGNAL.set(shutdown.clone()).ok();
    IS_CONNECTED.store(true, Ordering::SeqCst);

    // Spawn the connection task with shutdown handling
    rt.spawn(async move {
        tokio::select! {
            result = connection => {
                if let Err(e) = result {
                    // Only print error if not a normal shutdown
                    if IS_CONNECTED.load(Ordering::SeqCst) {
                        eprintln!("Connection error: {}", e);
                    }
                }
            }
            _ = shutdown.notified() => {
                // Shutdown requested, exit cleanly
            }
        }
    });

    // Store in both CLIENT (for backward compatibility) and GLOBAL_DB
    let client_arc = Arc::new(client);
    CLIENT.set(client_arc.clone()).ok();

    // Store in global DB for get() access
    GLOBAL_DB.set(client_arc.clone()).ok();

    make_ok_database("postgres".to_string(), true)
}

/// Shutdown the database connection cleanly
/// This should be called before program exit to avoid segfaults
#[no_mangle]
pub extern "C" fn doo_db_shutdown() {
    // Mark as disconnected first to suppress any error messages
    IS_CONNECTED.store(false, Ordering::SeqCst);

    // Signal the connection task to shutdown
    // The notify will wake up the spawned task which will then exit
    if let Some(shutdown) = SHUTDOWN_SIGNAL.get() {
        shutdown.notify_one();
    }

    // Don't block here - the exit(0) call in generated code will handle termination
    // Blocking on the runtime can cause hangs if the connection task doesn't respond quickly
}

#[no_mangle]
pub extern "C" fn doo_db_table_exists(_db: *const c_char, table_name: *const c_char) -> i32 {
    let table = match c_to_string(table_name) {
        Ok(s) => s,
        Err(_) => return 0,
    };
    let client = match get_client() {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let rt = match runtime() {
        Ok(r) => r,
        Err(_) => return 0,
    };
    let query = "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = $1)";
    let exists = rt.block_on(async {
        match client.query_one(query, &[&table]).await {
            Ok(row) => row.get::<usize, bool>(0),
            Err(_) => false,
        }
    });
    if exists {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn doo_db_create_table(_db: *const c_char, sql: *const c_char) -> *mut DooResult {
    let sql = match c_to_string(sql) {
        Ok(s) => s,
        Err(e) => return make_err_query_failed(e),
    };
    let client = match get_client() {
        Ok(c) => c,
        Err(e) => return make_err_connection_failed(e),
    };
    let rt = match runtime() {
        Ok(r) => r,
        Err(e) => return make_err(e),
    };
    let res = rt.block_on(async { client.execute(sql.as_str(), &[]).await });
    match res {
        Ok(_) => make_ok_void(),
        Err(e) => make_err_from_pg_error(&e),
    }
}

#[no_mangle]
pub extern "C" fn doo_db_query(_db: *const c_char, sql: *const c_char) -> *mut DooResult {
    let sql = match c_to_string(sql) {
        Ok(s) => s,
        Err(e) => return make_err_query_failed(e),
    };
    let client = match get_client() {
        Ok(c) => c,
        Err(e) => return make_err_connection_failed(e),
    };
    let rt = match runtime() {
        Ok(r) => r,
        Err(e) => return make_err(e),
    };
    let res = rt.block_on(async { client.query(sql.as_str(), &[]).await });
    let rows = match res {
        Ok(r) => r,
        Err(e) => return make_err_from_pg_error(&e),
    };

    let mut array = Vec::new();
    for row in rows {
        let mut obj = serde_json::Map::new();
        for (i, col) in row.columns().iter().enumerate() {
            let name = col.name();
            let value: serde_json::Value = match col.type_().name() {
                "int4" => row
                    .get::<usize, Option<i32>>(i)
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::Null),
                "int8" => row
                    .get::<usize, Option<i64>>(i)
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::Null),
                "float4" => row
                    .get::<usize, Option<f32>>(i)
                    .map(|v| serde_json::Value::from(v as f64))
                    .unwrap_or(serde_json::Value::Null),
                "float8" => row
                    .get::<usize, Option<f64>>(i)
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::Null),
                "bool" => row
                    .get::<usize, Option<bool>>(i)
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::Null),
                _ => row
                    .get::<usize, Option<String>>(i)
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::Null),
            };
            obj.insert(name.to_string(), value);
        }
        array.push(serde_json::Value::Object(obj));
    }

    let json = serde_json::Value::Array(array).to_string();
    make_ok_string(json)
}

#[no_mangle]
pub extern "C" fn doo_db_execute(_db: *const c_char, sql: *const c_char) -> *mut DooResult {
    let sql_str = match c_to_string(sql) {
        Ok(s) => s,
        Err(e) => return make_err_query_failed(format!("Invalid SQL string: {}", e)),
    };

    let client = match get_client() {
        Ok(c) => c,
        Err(e) => return make_err_connection_failed(e),
    };

    let rt = match runtime() {
        Ok(r) => r,
        Err(e) => return make_err(e),
    };

    match rt.block_on(async { client.execute(&sql_str, &[]).await }) {
        Ok(rows) => make_ok_int(rows as i64),
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("duplicate key") || err_str.contains("unique constraint") {
                make_err_unique_violation("unknown")
            } else {
                make_err_query_failed(format!("Execute failed: {}", err_str))
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn doo_db_insert(
    _db: *const c_char,
    sql: *const c_char,
    values_json: *const c_char,
) -> *mut DooResult {
    let sql_str = match c_to_string(sql) {
        Ok(s) => s,
        Err(e) => return make_err_query_failed(format!("Invalid SQL string: {}", e)),
    };
    let values_str = match c_to_string(values_json) {
        Ok(s) => s,
        Err(e) => return make_err_query_failed(format!("Invalid values JSON: {}", e)),
    };

    let client = match get_client() {
        Ok(c) => c,
        Err(e) => return make_err_connection_failed(e),
    };

    let rt = match runtime() {
        Ok(r) => r,
        Err(e) => return make_err(e),
    };

    match rt.block_on(async { client.execute(&sql_str, &[&values_str]).await }) {
        Ok(rows) => make_ok_int(rows as i64),
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("duplicate key") || err_str.contains("unique constraint") {
                make_err_unique_violation("unknown")
            } else {
                make_err_query_failed(format!("Insert failed: {}", err_str))
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn doo_db_query_one(_db: *const c_char, sql: *const c_char) -> *mut DooResult {
    let sql = match c_to_string(sql) {
        Ok(s) => s,
        Err(e) => return make_err_query_failed(e),
    };
    let client = match get_client() {
        Ok(c) => c,
        Err(e) => return make_err_connection_failed(e),
    };
    let rt = match runtime() {
        Ok(r) => r,
        Err(e) => return make_err(e),
    };

    let res = rt.block_on(async { client.query_one(sql.as_str(), &[]).await });
    let row = match res {
        Ok(r) => r,
        Err(e) => return make_err_query_failed(format!("Query failed: {}", e)),
    };

    let mut obj = serde_json::Map::new();
    for (i, col) in row.columns().iter().enumerate() {
        let name = col.name();
        let value: serde_json::Value = match col.type_().name() {
            "int4" => row
                .get::<usize, Option<i32>>(i)
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
            "int8" => row
                .get::<usize, Option<i64>>(i)
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
            "float4" => row
                .get::<usize, Option<f32>>(i)
                .map(|v| serde_json::Value::from(v as f64))
                .unwrap_or(serde_json::Value::Null),
            "float8" => row
                .get::<usize, Option<f64>>(i)
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
            "bool" => row
                .get::<usize, Option<bool>>(i)
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
            _ => row
                .get::<usize, Option<String>>(i)
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
        };
        obj.insert(name.to_string(), value);
    }

    let json = serde_json::Value::Object(obj).to_string();
    make_ok_string(json)
}

#[no_mangle]
pub extern "C" fn doo_db_insert_json(
    sql: *const c_char,
    values_json: *const c_char,
) -> *mut DooResult {
    let sql = match c_to_string(sql) {
        Ok(s) => s,
        Err(e) => return make_err_query_failed(e),
    };
    let values_json = match c_to_string(values_json) {
        Ok(s) => s,
        Err(e) => return make_err_query_failed(e),
    };

    let client = match get_client() {
        Ok(c) => c,
        Err(e) => return make_err_connection_failed(e),
    };
    let rt = match runtime() {
        Ok(r) => r,
        Err(e) => return make_err(e),
    };

    // Parse JSON values array
    let values: Vec<serde_json::Value> =
        match serde_json::from_str::<Vec<serde_json::Value>>(&values_json) {
            Ok(v) => v,
            Err(e) => return make_err(format!("Invalid JSON values: {e}")),
        };

    // Convert JSON values to owned types that can be passed to PostgreSQL
    let mut string_values: Vec<String> = Vec::new();
    let mut int32_values: Vec<i32> = Vec::new();
    let mut int64_values: Vec<i64> = Vec::new();
    let mut float_values: Vec<f64> = Vec::new();
    let mut bool_values: Vec<bool> = Vec::new();

    // Store parameter types and indices
    let mut param_types: Vec<&str> = Vec::new();
    let mut param_indices: Vec<(usize, usize)> = Vec::new(); // (vec_index, type_index)

    for (i, value) in values.iter().enumerate() {
        match value {
            serde_json::Value::String(s) => {
                string_values.push(s.clone());
                param_types.push("string");
                param_indices.push((i, string_values.len() - 1));
            }
            serde_json::Value::Number(n) => {
                if let Some(i_val) = n.as_i64() {
                    // Store as i32 if it fits, otherwise i64
                    if i_val >= i32::MIN as i64 && i_val <= i32::MAX as i64 {
                        int32_values.push(i_val as i32);
                        param_types.push("int32");
                        param_indices.push((i, int32_values.len() - 1));
                    } else {
                        int64_values.push(i_val);
                        param_types.push("int64");
                        param_indices.push((i, int64_values.len() - 1));
                    }
                } else if let Some(f_val) = n.as_f64() {
                    float_values.push(f_val);
                    param_types.push("float");
                    param_indices.push((i, float_values.len() - 1));
                } else {
                    return make_err(format!("Unsupported number type at index {}", i));
                }
            }
            serde_json::Value::Bool(b) => {
                bool_values.push(b.clone());
                param_types.push("bool");
                param_indices.push((i, bool_values.len() - 1));
            }
            serde_json::Value::Null => {
                // For null values, we'll use an empty string
                string_values.push(String::new());
                param_types.push("null");
                param_indices.push((i, string_values.len() - 1));
            }
            _ => {
                return make_err(format!("Unsupported JSON type at index {}: {:?}", i, value));
            }
        }
    }

    // Build params vector with references
    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();
    for (i, type_str) in param_types.iter().enumerate() {
        let (_, idx) = param_indices[i];
        match *type_str {
            "string" | "null" => params.push(&string_values[idx]),
            "int32" => params.push(&int32_values[idx]),
            "int64" => params.push(&int64_values[idx]),
            "float" => params.push(&float_values[idx]),
            "bool" => params.push(&bool_values[idx]),
            _ => {}
        }
    }

    let res = rt.block_on(async { client.query_one(sql.as_str(), &params[..]).await });

    match res {
        Ok(row) => {
            // Return the first column as ID
            if row.columns().is_empty() {
                return make_err("No columns returned".to_string());
            }
            // Try i32 first (SERIAL), then i64 (BIGSERIAL)
            let id: i64 = row
                .try_get::<_, i32>(0)
                .map(|v| v as i64)
                .or_else(|_| row.try_get::<_, i64>(0))
                .unwrap_or(0);
            make_ok_int(id)
        }
        Err(e) => {
            // Log the actual PostgreSQL error for debugging

            // Handle specific PostgreSQL errors
            if is_unique_violation(&e) {
                let field = extract_field_name(&e).unwrap_or_else(|| "unknown".to_string());
                return make_err_unique_violation(&field);
            }
            if is_not_null_violation(&e) {
                let field = extract_field_name(&e).unwrap_or_else(|| "unknown".to_string());
                return make_err_not_null_violation(&field);
            }

            // Check for other common PostgreSQL errors
            if let Some(db_err) = e.as_db_error() {
                let pg_code = db_err.code().code();

                // Return the actual database error message
                make_err(format!(
                    "Insert failed: {} (code: {})",
                    db_err.message(),
                    pg_code
                ))
            } else {
                make_err(format!("Insert failed: {}", e))
            }
        }
    }
}

/// Execute a parameterized query (SELECT with WHERE clause)
/// Returns single row as JSON
#[no_mangle]
pub extern "C" fn doo_db_query_one_param(
    _db: *const c_char,
    sql: *const c_char,
    param_value: *const c_char,
) -> *mut DooResult {
    let sql = match c_to_string(sql) {
        Ok(s) => s,
        Err(e) => return make_err(e),
    };
    let param = match c_to_string(param_value) {
        Ok(s) => s,
        Err(e) => return make_err(e),
    };
    let client = match get_client() {
        Ok(c) => c,
        Err(e) => return make_err(e),
    };
    let rt = match runtime() {
        Ok(r) => r,
        Err(e) => return make_err(e),
    };

    let res = rt.block_on(async { client.query_one(sql.as_str(), &[&param]).await });
    let row = match res {
        Ok(r) => r,
        Err(e) => return make_err_query_failed(format!("Query failed: {}", e)),
    };

    let mut obj = serde_json::Map::new();
    for (i, col) in row.columns().iter().enumerate() {
        let name = col.name();
        let value: serde_json::Value = match col.type_().name() {
            "int4" => row
                .get::<usize, Option<i32>>(i)
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
            "int8" => row
                .get::<usize, Option<i64>>(i)
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
            "float4" => row
                .get::<usize, Option<f32>>(i)
                .map(|v| serde_json::Value::from(v as f64))
                .unwrap_or(serde_json::Value::Null),
            "float8" => row
                .get::<usize, Option<f64>>(i)
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
            "bool" => row
                .get::<usize, Option<bool>>(i)
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
            _ => row
                .get::<usize, Option<String>>(i)
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
        };
        obj.insert(name.to_string(), value);
    }

    let json = serde_json::Value::Object(obj).to_string();
    make_ok_string(json)
}

/// Execute a parameterized query for list (SELECT)
/// Returns multiple rows as JSON array
#[no_mangle]
pub extern "C" fn doo_db_query_param(
    _db: *const c_char,
    sql: *const c_char,
    param: *const c_char,
) -> *mut DooResult {
    let sql = match c_to_string(sql) {
        Ok(s) => s,
        Err(e) => return make_err_query_failed(e),
    };
    let param = match c_to_string(param) {
        Ok(s) => s,
        Err(e) => return make_err_query_failed(e),
    };

    let client = match get_client() {
        Ok(c) => c,
        Err(e) => return make_err_connection_failed(e),
    };
    let rt = match runtime() {
        Ok(r) => r,
        Err(e) => return make_err(e),
    };

    let res = rt.block_on(async { client.query(sql.as_str(), &[&param]).await });
    let rows = match res {
        Ok(r) => r,
        Err(e) => return make_err_query_failed(format!("Query failed: {}", e)),
    };

    let mut json_rows = Vec::new();
    for row in rows {
        let mut obj = serde_json::Map::new();
        for (i, col) in row.columns().iter().enumerate() {
            let name = col.name();
            let value: serde_json::Value = match col.type_().name() {
                "int4" => row
                    .get::<usize, Option<i32>>(i)
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::Null),
                "int8" => row
                    .get::<usize, Option<i64>>(i)
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::Null),
                "float4" => row
                    .get::<usize, Option<f32>>(i)
                    .map(|v| serde_json::Value::from(v as f64))
                    .unwrap_or(serde_json::Value::Null),
                "float8" => row
                    .get::<usize, Option<f64>>(i)
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::Null),
                "bool" => row
                    .get::<usize, Option<bool>>(i)
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::Null),
                _ => row
                    .get::<usize, Option<String>>(i)
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::Null),
            };
            obj.insert(name.to_string(), value);
        }
        json_rows.push(serde_json::Value::Object(obj));
    }

    let json = serde_json::Value::Array(json_rows).to_string();
    make_ok_string(json)
}

/// Execute parameterized INSERT/UPDATE/DELETE and return affected rows
#[no_mangle]
pub extern "C" fn doo_db_execute_param(
    _db: *const c_char,
    sql: *const c_char,
    param: *const c_char,
) -> *mut DooResult {
    let sql = match c_to_string(sql) {
        Ok(s) => s,
        Err(e) => return make_err_query_failed(e),
    };
    let param = match c_to_string(param) {
        Ok(s) => s,
        Err(e) => return make_err_query_failed(e),
    };

    let client = match get_client() {
        Ok(c) => c,
        Err(e) => return make_err_connection_failed(e),
    };
    let rt = match runtime() {
        Ok(r) => r,
        Err(e) => return make_err(e),
    };
    let res = rt.block_on(async { client.execute(sql.as_str(), &[&param]).await });
    match res {
        Ok(rows) => make_ok_int(rows as i64),
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("duplicate key") || err_str.contains("unique constraint") {
                make_err_unique_violation("unknown")
            } else {
                make_err_query_failed(format!("Execute failed: {}", e))
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn doo_db_query_json(sql: *const c_char) -> *mut DooResult {
    let sql = match c_to_string(sql) {
        Ok(s) => s,
        Err(e) => return make_err_query_failed(e),
    };
    let client = match get_client() {
        Ok(c) => c,
        Err(e) => return make_err_connection_failed(e),
    };
    let rt = match runtime() {
        Ok(r) => r,
        Err(e) => return make_err(e),
    };

    let res = rt.block_on(async { client.query(sql.as_str(), &[]).await });
    let rows = match res {
        Ok(r) => r,
        Err(e) => return make_err_query_failed(format!("Query failed: {}", e)),
    };

    let mut json_rows = Vec::new();
    for row in rows {
        let mut obj = serde_json::Map::new();
        for (i, col) in row.columns().iter().enumerate() {
            let name = col.name();
            let value: serde_json::Value = match col.type_().name() {
                "int4" => row
                    .get::<usize, Option<i32>>(i)
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::Null),
                "int8" => row
                    .get::<usize, Option<i64>>(i)
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::Null),
                "float4" => row
                    .get::<usize, Option<f32>>(i)
                    .map(|v| serde_json::Value::from(v as f64))
                    .unwrap_or(serde_json::Value::Null),
                "float8" => row
                    .get::<usize, Option<f64>>(i)
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::Null),
                "bool" => row
                    .get::<usize, Option<bool>>(i)
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::Null),
                _ => row
                    .get::<usize, Option<String>>(i)
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::Null),
            };
            obj.insert(name.to_string(), value);
        }
        json_rows.push(serde_json::Value::Object(obj));
    }

    let json = serde_json::Value::Array(json_rows).to_string();
    make_ok_string(json)
}

#[no_mangle]
pub extern "C" fn doo_db_query_one_json(sql: *const c_char) -> *mut DooResult {
    let sql = match c_to_string(sql) {
        Ok(s) => s,
        Err(e) => return make_err_query_failed(e),
    };
    let client = match get_client() {
        Ok(c) => c,
        Err(e) => return make_err_connection_failed(e),
    };
    let rt = match runtime() {
        Ok(r) => r,
        Err(e) => return make_err(e),
    };

    let res = rt.block_on(async { client.query_one(sql.as_str(), &[]).await });
    let row = match res {
        Ok(r) => r,
        Err(e) => return make_err_query_failed(format!("Query failed: {}", e)),
    };

    let mut obj = serde_json::Map::new();
    for (i, col) in row.columns().iter().enumerate() {
        let name = col.name();
        let value: serde_json::Value = match col.type_().name() {
            "int4" => row
                .get::<usize, Option<i32>>(i)
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
            "int8" => row
                .get::<usize, Option<i64>>(i)
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
            "float4" => row
                .get::<usize, Option<f32>>(i)
                .map(|v| serde_json::Value::from(v as f64))
                .unwrap_or(serde_json::Value::Null),
            "float8" => row
                .get::<usize, Option<f64>>(i)
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
            "bool" => row
                .get::<usize, Option<bool>>(i)
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
            _ => row
                .get::<usize, Option<String>>(i)
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null),
        };
        obj.insert(name.to_string(), value);
    }

    let json = serde_json::Value::Object(obj).to_string();
    make_ok_string(json)
}

#[no_mangle]
pub extern "C" fn doo_db_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let _ = CString::from_raw(ptr);
    }
}

#[no_mangle]
pub extern "C" fn doo_db_result_free(ptr: *mut DooResult) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let res = Box::from_raw(ptr);
        // Only free error values - Ok values might be integers cast to pointers
        // or strings that will be freed separately
        if res.tag != 0 && !res.value.is_null() {
            let _ = Box::from_raw(res.value as *mut DooDbError);
        }
    }
}

#[no_mangle]
pub extern "C" fn doo_db_is_error(ptr: *mut DooResult) -> i32 {
    if ptr.is_null() {
        return 1;
    }
    unsafe {
        let res = &*ptr;
        if res.tag != 0 {
            1
        } else {
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn doo_db_get_error_message(ptr: *mut DooResult) -> *mut c_char {
    if ptr.is_null() {
        return string_to_c("Null result pointer".to_string());
    }
    unsafe {
        let res = &*ptr;
        if res.tag != 0 && !res.value.is_null() {
            let err = &*(res.value as *mut DooDbError);
            // Return a copy of the error message
            if !err.message.is_null() {
                match c_to_string(err.message) {
                    Ok(s) => string_to_c(s),
                    Err(_) => string_to_c("Error reading message".to_string()),
                }
            } else {
                string_to_c("Unknown error".to_string())
            }
        } else {
            string_to_c("No error".to_string())
        }
    }
}

/// Execute raw SQL - returns JSON for SELECT, empty string for others
/// This is a convenience method that auto-detects query type
#[no_mangle]
pub extern "C" fn doo_db_raw(_db: *const c_char, sql: *const c_char) -> *mut DooResult {
    let sql_str = match c_to_string(sql) {
        Ok(s) => s.trim().to_string(),
        Err(e) => return make_err_query_failed(e),
    };

    let client = match get_client() {
        Ok(c) => c,
        Err(e) => return make_err_connection_failed(e),
    };

    let rt = match runtime() {
        Ok(r) => r,
        Err(e) => return make_err(e),
    };

    // Detect if this is a SELECT query
    let sql_upper = sql_str.to_uppercase();
    let is_select = sql_upper.starts_with("SELECT") || sql_upper.starts_with("WITH");

    if is_select {
        // Execute as SELECT and return JSON
        let res = rt.block_on(async { client.query(&sql_str, &[]).await });
        let rows = match res {
            Ok(r) => r,
            Err(e) => return make_err_query_failed(format!("Query failed: {}", e)),
        };

        let mut array = Vec::new();
        for row in rows {
            let mut obj = serde_json::Map::new();
            for (i, col) in row.columns().iter().enumerate() {
                let name = col.name();
                let value: serde_json::Value = match col.type_().name() {
                    "int4" => row
                        .get::<usize, Option<i32>>(i)
                        .map(serde_json::Value::from)
                        .unwrap_or(serde_json::Value::Null),
                    "int8" => row
                        .get::<usize, Option<i64>>(i)
                        .map(serde_json::Value::from)
                        .unwrap_or(serde_json::Value::Null),
                    "float4" => row
                        .get::<usize, Option<f32>>(i)
                        .map(|v| serde_json::Value::from(v as f64))
                        .unwrap_or(serde_json::Value::Null),
                    "float8" => row
                        .get::<usize, Option<f64>>(i)
                        .map(serde_json::Value::from)
                        .unwrap_or(serde_json::Value::Null),
                    "bool" => row
                        .get::<usize, Option<bool>>(i)
                        .map(serde_json::Value::from)
                        .unwrap_or(serde_json::Value::Null),
                    _ => row
                        .get::<usize, Option<String>>(i)
                        .map(serde_json::Value::from)
                        .unwrap_or(serde_json::Value::Null),
                };
                obj.insert(name.to_string(), value);
            }
            array.push(serde_json::Value::Object(obj));
        }

        let json = serde_json::Value::Array(array).to_string();
        make_ok_string(json)
    } else {
        // Execute as INSERT/UPDATE/DELETE/CREATE/DROP/ALTER
        let res = rt.block_on(async { client.execute(&sql_str, &[]).await });
        match res {
            Ok(_) => make_ok_string(String::new()),
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("duplicate key") || err_str.contains("unique constraint") {
                    return make_err_unique_violation("unknown");
                } else {
                    // Extract PostgreSQL error code if available
                    let pg_code = if let Some(db_err) = e.as_db_error() {
                        db_err.code().code().to_string()
                    } else {
                        "UNKNOWN".to_string()
                    };
                    return make_err_query_failed(format!(
                        "SQL: {} | Error: {} | PG Code: {}",
                        sql_str, e, pg_code
                    ));
                }
            }
        }
    }
}

/// Execute raw SQL with parameter - returns JSON for SELECT, empty string for others
#[no_mangle]
pub extern "C" fn doo_db_raw_param(
    _db: *const c_char,
    sql: *const c_char,
    params_json: *const c_char,
) -> *mut DooResult {
    let sql_str = match c_to_string(sql) {
        Ok(s) => s.trim().to_string(),
        Err(e) => return make_err_query_failed(e),
    };

    let params_str = match c_to_string(params_json) {
        Ok(s) => s.trim().to_string(),
        Err(e) => return make_err_query_failed(e),
    };

    let client = match get_client() {
        Ok(c) => c,
        Err(e) => return make_err_connection_failed(e),
    };
    let rt = match runtime() {
        Ok(r) => r,
        Err(e) => return make_err(e),
    };

    // Parse params as JSON - can be a single value, array, or object
    let params_value: serde_json::Value = match serde_json::from_str(&params_str) {
        Ok(v) => v,
        Err(_) => {
            // If not valid JSON, try to parse as number first, then fall back to string
            if let Ok(i) = params_str.parse::<i64>() {
                serde_json::Value::Number(serde_json::Number::from(i))
            } else if let Ok(f) = params_str.parse::<f64>() {
                serde_json::Value::Number(
                    serde_json::Number::from_f64(f).unwrap_or(serde_json::Number::from(0)),
                )
            } else if params_str == "true" || params_str == "false" {
                serde_json::Value::Bool(params_str == "true")
            } else {
                serde_json::Value::String(params_str.clone())
            }
        }
    };

    // Convert JSON params to PostgreSQL parameters
    // Supports: primitives, arrays (for ANY/ALL), and objects (as JSONB)
    let pg_params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync>> = match &params_value {
        serde_json::Value::Array(arr) => {
            // If it's an array, each element becomes a separate $1, $2, $3... parameter
            arr.iter()
                .map(|v| -> Box<dyn tokio_postgres::types::ToSql + Sync> {
                    match v {
                        serde_json::Value::String(s) => Box::new(s.clone()),
                        serde_json::Value::Number(n) => {
                            if let Some(i) = n.as_i64() {
                                // Use i32 for integers to match postgres INT4
                                Box::new(i as i32)
                            } else if let Some(f) = n.as_f64() {
                                Box::new(f)
                            } else {
                                Box::new(n.to_string())
                            }
                        }
                        serde_json::Value::Bool(b) => Box::new(*b),
                        serde_json::Value::Null => Box::new(None::<String>),
                        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                            // Nested arrays/objects as JSONB
                            Box::new(serde_json::to_string(v).unwrap_or_default())
                        }
                    }
                })
                .collect()
        }
        serde_json::Value::String(s) => vec![Box::new(s.clone())],
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                // Use i32 for integers to match postgres INT4
                vec![Box::new(i as i32)]
            } else if let Some(f) = n.as_f64() {
                vec![Box::new(f)]
            } else {
                vec![Box::new(n.to_string())]
            }
        }
        serde_json::Value::Bool(b) => vec![Box::new(*b)],
        serde_json::Value::Object(_) => {
            // Struct/object as JSONB string
            vec![Box::new(params_str.clone())]
        }
        serde_json::Value::Null => vec![Box::new(None::<String>)],
    };

    // Detect if this is a SELECT query
    let sql_upper = sql_str.to_uppercase();
    let is_select = sql_upper.starts_with("SELECT") || sql_upper.starts_with("WITH");

    if is_select {
        // Execute as SELECT and return JSON
        let pg_params_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            pg_params.iter().map(|p| p.as_ref()).collect();

        let res = rt.block_on(async { client.query(&sql_str, &pg_params_refs[..]).await });
        let rows = match res {
            Ok(r) => r,
            Err(e) => return make_err_query_failed(format!("Query failed: {}", e)),
        };

        let mut array = Vec::new();
        for row in rows {
            let mut obj = serde_json::Map::new();
            for (i, col) in row.columns().iter().enumerate() {
                let name = col.name();
                let value: serde_json::Value = match col.type_().name() {
                    "int4" => row
                        .get::<usize, Option<i32>>(i)
                        .map(serde_json::Value::from)
                        .unwrap_or(serde_json::Value::Null),
                    "int8" => row
                        .get::<usize, Option<i64>>(i)
                        .map(serde_json::Value::from)
                        .unwrap_or(serde_json::Value::Null),
                    "float4" => row
                        .get::<usize, Option<f32>>(i)
                        .map(|v| serde_json::Value::from(v as f64))
                        .unwrap_or(serde_json::Value::Null),
                    "float8" => row
                        .get::<usize, Option<f64>>(i)
                        .map(serde_json::Value::from)
                        .unwrap_or(serde_json::Value::Null),
                    "bool" => row
                        .get::<usize, Option<bool>>(i)
                        .map(serde_json::Value::from)
                        .unwrap_or(serde_json::Value::Null),
                    _ => row
                        .get::<usize, Option<String>>(i)
                        .map(serde_json::Value::from)
                        .unwrap_or(serde_json::Value::Null),
                };
                obj.insert(name.to_string(), value);
            }
            array.push(serde_json::Value::Object(obj));
        }

        let json = serde_json::Value::Array(array).to_string();
        make_ok_string(json)
    } else {
        // Execute as INSERT/UPDATE/DELETE/CREATE/DROP/ALTER
        let pg_params_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            pg_params.iter().map(|p| p.as_ref()).collect();

        let res = rt.block_on(async { client.execute(&sql_str, &pg_params_refs[..]).await });
        match res {
            Ok(_) => make_ok_string(String::new()),
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("duplicate key") || err_str.contains("unique constraint") {
                    return make_err_unique_violation("unknown");
                } else {
                    // Extract PostgreSQL error code if available
                    let pg_code = if let Some(db_err) = e.as_db_error() {
                        db_err.code().code().to_string()
                    } else {
                        "UNKNOWN".to_string()
                    };
                    return make_err_query_failed(format!(
                        "SQL: {} | Params: {} | Error: {} | PG Code: {}",
                        sql_str, params_str, e, pg_code
                    ));
                }
            }
        }
    }
}

// ============================================================================
// DB VALIDATION HELPERS
// ============================================================================

/// Check if a value already exists in a table column (for @unique validation)
/// Returns 1 if exists, 0 if not exists, -1 on error
#[no_mangle]
pub extern "C" fn doo_db_check_unique(
    table: *const c_char,
    column: *const c_char,
    value: *const c_char,
    exclude_id: i64, // -1 means don't exclude any, used for updates
) -> i32 {
    let table = match c_to_string(table) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let column = match c_to_string(column) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let value = match c_to_string(value) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let client = match get_client() {
        Ok(c) => c,
        Err(_) => return -1,
    };
    let rt = match runtime() {
        Ok(r) => r,
        Err(_) => return -1,
    };

    let sql = if exclude_id >= 0 {
        format!(
            "SELECT COUNT(*) FROM {} WHERE {} = $1 AND id != $2",
            table, column
        )
    } else {
        format!("SELECT COUNT(*) FROM {} WHERE {} = $1", table, column)
    };

    let res = if exclude_id >= 0 {
        rt.block_on(async { client.query_one(&sql, &[&value, &exclude_id]).await })
    } else {
        rt.block_on(async { client.query_one(&sql, &[&value]).await })
    };

    match res {
        Ok(row) => {
            let count: i64 = row.get(0);
            if count > 0 {
                1 // Exists
            } else {
                0 // Not exists
            }
        }
        Err(_) => -1, // Error
    }
}

/// Validate unique constraints for multiple fields
/// fields_json: JSON array of {field: "name", value: "john", table: "users"}
/// Returns JSON with validation result
#[no_mangle]
pub extern "C" fn doo_db_validate_unique_constraints(
    fields_json: *const c_char,
    exclude_id: i64,
) -> *mut c_char {
    let fields_json_str = match c_to_string(fields_json) {
        Ok(s) => s,
        Err(e) => return string_to_c(json!({"error": e}).to_string()),
    };

    let fields: Vec<serde_json::Value> = match serde_json::from_str(&fields_json_str) {
        Ok(f) => f,
        Err(e) => return string_to_c(json!({"error": format!("Invalid JSON: {}", e)}).to_string()),
    };

    for field_obj in fields {
        let field = field_obj
            .get("field")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let value = field_obj
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let table = field_obj
            .get("table")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if field.is_empty() || table.is_empty() {
            continue;
        }

        let field_c = string_to_c(field.to_string());
        let value_c = string_to_c(value.to_string());
        let table_c = string_to_c(table.to_string());

        let exists = doo_db_check_unique(table_c, field_c, value_c, exclude_id);

        unsafe {
            let _ = CString::from_raw(field_c);
            let _ = CString::from_raw(value_c);
            let _ = CString::from_raw(table_c);
        }

        if exists == 1 {
            return string_to_c(
                json!({
                    "success": false,
                    "error": {
                        "code": "UNIQUE_VIOLATION",
                        "field": field,
                        "message": format!("A record with this {} already exists", field)
                    }
                })
                .to_string(),
            );
        } else if exists == -1 {
            return string_to_c(
                json!({
                    "success": false,
                    "error": {
                        "code": "INTERNAL_ERROR",
                        "message": "Failed to check unique constraint"
                    }
                })
                .to_string(),
            );
        }
    }

    string_to_c(json!({"success": true}).to_string())
}

// Helper functions to detect PostgreSQL errors

fn is_unique_violation(err: &tokio_postgres::Error) -> bool {
    if let Some(db_err) = err.as_db_error() {
        db_err.code().code() == "23505"
    } else {
        false
    }
}

fn is_not_null_violation(err: &tokio_postgres::Error) -> bool {
    if let Some(db_err) = err.as_db_error() {
        db_err.code().code() == "23502"
    } else {
        false
    }
}

fn extract_field_name(err: &tokio_postgres::Error) -> Option<String> {
    err.as_db_error().and_then(|db_err| {
        db_err
            .column()
            .map(|s| s.to_string())
            .or_else(|| db_err.constraint().map(|s| s.to_string()))
    })
}

fn make_err_not_null_violation(field: &str) -> *mut DooResult {
    let err = Box::new(DooDbError {
        code: 2,
        message: string_to_c(format!("Field '{}' cannot be null", field)),
    });
    Box::into_raw(Box::new(DooResult {
        tag: 1,
        value: Box::into_raw(err) as *mut _,
    }))
}

/// Get the global database instance
/// This allows handlers to access the database without explicit parameter passing
#[no_mangle]
pub extern "C" fn doo_db_get_global() -> *mut DooResult {
    match GLOBAL_DB.get() {
        Some(_db_mutex) => {
            // Database was initialized, return success
            make_ok_database("postgres".to_string(), true)
        }
        None => {
            // Database not initialized yet
            make_err_connection_failed(
                "Database not initialized. Call Database::postgres() first".to_string(),
            )
        }
    }
}

/// Query database and return JSON array for typed deserialization
/// This is identical to doo_db_raw but explicitly indicates array return
/// The compiler will deserialize the JSON array into typed structs
#[no_mangle]
pub extern "C" fn doo_db_query_array(_db: *const c_char, sql: *const c_char) -> *mut DooResult {
    // For SELECT queries, doo_db_raw already returns JSON arrays
    // Just delegate to it
    doo_db_raw(_db, sql)
}

/// Force a clean program exit
/// This is needed because the Tokio runtime stored in a static OnceCell
/// doesn't shut down cleanly, causing the program to hang or crash on exit.
/// Calling this function will immediately exit with code 0.
#[no_mangle]
pub extern "C" fn doo_db_cleanup_and_exit() {
    std::process::exit(0);
}

// ============================================================================
// ARRAY SERIALIZATION FOR RAW PARAMS
// ============================================================================

// Array layout in memory:
// [RC: 4 bytes][LEN: 4 bytes][DATA...]
// The array_ptr passed from compiler points to DATA, not the header!
// So we need to read RC at offset -8, LEN at offset -4

#[no_mangle]
pub extern "C" fn doo_db_serialize_array(
    array_ptr: *const std::ffi::c_void,
    elem_type: *const std::ffi::c_char,
) -> *const std::ffi::c_char {
    if array_ptr.is_null() || elem_type.is_null() {
        return string_to_c("[]".to_string());
    }

    let elem_type_str = match c_to_string(elem_type) {
        Ok(s) => s,
        Err(_) => return string_to_c("[]".to_string()),
    };

    // Read length from offset -4 (4 bytes before data pointer)
    let len = unsafe {
        let len_ptr = (array_ptr as *const u8).offset(-4) as *const i32;
        *len_ptr as usize
    };

    // The data starts at array_ptr
    let raw_data = array_ptr as *const u8;

    let mut json_arr = Vec::with_capacity(len);

    // Safety limit to prevent massive loop on bad pointers
    let safe_len = std::cmp::min(len, 10000);

    for i in 0..safe_len {
        match elem_type_str.as_str() {
            "Int" | "Enum" => {
                // Ints are i64 (actually i32 in Doo)
                let val_ptr = unsafe { (raw_data as *const i32).add(i) };
                let val = unsafe { *val_ptr };
                json_arr.push(serde_json::Value::Number(val.into()));
            }
            "Float" => {
                // Floats are f64
                let val_ptr = unsafe { (raw_data as *const f64).add(i) };
                let val = unsafe { *val_ptr };
                let num = serde_json::Number::from_f64(val).unwrap_or(serde_json::Number::from(0));
                json_arr.push(serde_json::Value::Number(num));
            }
            "Bool" => {
                // Bools are i32 (per compiler implementation)
                let val_ptr = unsafe { (raw_data as *const i32).add(i) };
                let val = unsafe { *val_ptr };
                json_arr.push(serde_json::Value::Bool(val != 0));
            }
            "Str" => {
                // Elements are *const c_char (string pointers)
                let ptr_ptr = unsafe { (raw_data as *const *const c_char).add(i) };
                let str_ptr = unsafe { *ptr_ptr };

                if str_ptr.is_null() {
                    json_arr.push(serde_json::Value::Null);
                } else {
                    if let Ok(s) = unsafe { CStr::from_ptr(str_ptr).to_str() } {
                        json_arr.push(serde_json::Value::String(s.to_string()));
                    } else {
                        let s = unsafe { CStr::from_ptr(str_ptr).to_string_lossy().to_string() };
                        json_arr.push(serde_json::Value::String(s));
                    }
                }
            }
            _ => {
                // Fallback for unknown types (e.g. unknown structs)
                json_arr.push(serde_json::Value::Null);
            }
        }
    }

    let json_str = serde_json::Value::Array(json_arr).to_string();
    string_to_c(json_str)
}

#[no_mangle]
pub extern "C" fn doo_db_serialize_enum_array(
    array_ptr: *const std::ffi::c_void,
    variants: *const std::ffi::c_char,
    stride: i32,
) -> *const std::ffi::c_char {
    if array_ptr.is_null() || variants.is_null() {
        return string_to_c("[]".to_string());
    }

    let variants_str = match c_to_string(variants) {
        Ok(s) => s,
        Err(_) => "Int".to_string(), // Fallback?
    };

    let variant_names: Vec<&str> = variants_str.split(',').collect();

    // Read length from offset -4 (4 bytes before data pointer)
    let len = unsafe {
        let len_ptr = (array_ptr as *const u8).offset(-4) as *const i32;
        *len_ptr as usize
    };

    // The data starts at array_ptr
    let raw_data = array_ptr as *const u8;

    let mut json_arr = Vec::with_capacity(len);
    let safe_len = std::cmp::min(len, 10000);

    let stride_usize = if stride > 0 { stride as usize } else { 8 }; // Default 8 if 0?

    for i in 0..safe_len {
        let offset = i * stride_usize;
        // Enum layout: { i32 tag, ... }
        // We read first 4 bytes as i32 tag.
        let tag_ptr = unsafe { raw_data.add(offset) as *const i32 };
        let tag = unsafe { *tag_ptr } as usize;

        if tag < variant_names.len() {
            json_arr.push(serde_json::Value::String(variant_names[tag].to_string()));
        } else {
            // Invalid tag, push null or "Unknown"
            json_arr.push(serde_json::Value::Null);
        }
    }

    let json_str = serde_json::Value::Array(json_arr).to_string();
    string_to_c(json_str)
}
