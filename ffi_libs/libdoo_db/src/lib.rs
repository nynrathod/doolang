use once_cell::sync::OnceCell;
use serde_json::json;
use std::collections::HashSet;
use std::env;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Once;
use tokio::runtime::Runtime;
use tokio::sync::Notify;
use tokio_postgres::{Client, NoTls};

/// Thread-safe set to track freed DooResult pointers.
/// This prevents double-free by checking if a pointer was already freed.
/// NOTE: We use actual pointer tracking instead of sentinel values because
/// reading from freed memory (to check sentinel) is undefined behavior.
static FREED_RESULTS: OnceCell<Mutex<HashSet<usize>>> = OnceCell::new();

fn get_freed_results() -> &'static Mutex<HashSet<usize>> {
    FREED_RESULTS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Global counter for tracking malloc/free operations
static MALLOC_COUNTER: AtomicUsize = AtomicUsize::new(0);
static FREE_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Cross-platform stderr write helper
/// On Windows, libc::write expects u32 for count, on Unix it expects usize
#[inline]
unsafe fn stderr_write(msg: &[u8]) {
    #[cfg(windows)]
    {
        libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len() as u32);
    }
    #[cfg(not(windows))]
    {
        libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
    }
}

/// Tracked malloc wrapper that logs all allocations
unsafe fn tracked_malloc(size: usize, context: &str) -> *mut libc::c_void {
    let ptr = libc::malloc(size);
    let count = MALLOC_COUNTER.fetch_add(1, Ordering::SeqCst);

    if size > 10000 {
        let msg = format!(
            "[TRACKED_MALLOC #{:06}] context='{}', size={}, ptr={:p}\n",
            count, context, size, ptr
        );
        stderr_write(msg.as_bytes());
    }

    // Validate heap after large allocation
    if size > 5000 && !ptr.is_null() {
        let test = libc::malloc(64);
        if test.is_null() {
            let err_msg = format!(
                "[TRACKED_MALLOC] HEAP CORRUPTED after malloc! context='{}', size={}, ptr={:p}\n",
                context, size, ptr
            );
            stderr_write(err_msg.as_bytes());
        } else {
            libc::free(test);
        }
    }

    ptr
}

/// Tracked free wrapper that logs all deallocations
unsafe fn tracked_free(ptr: *mut libc::c_void, context: &str) {
    if ptr.is_null() {
        return;
    }

    let count = FREE_COUNTER.fetch_add(1, Ordering::SeqCst);

    let msg = format!(
        "[TRACKED_FREE #{:06}] context='{}', ptr={:p}\n",
        count, context, ptr
    );
    stderr_write(msg.as_bytes());

    // Validate heap before free
    let test = libc::malloc(32);
    if test.is_null() {
        let err_msg = format!(
            "[TRACKED_FREE] HEAP CORRUPTED before free! context='{}', ptr={:p}\n",
            context, ptr
        );
        stderr_write(err_msg.as_bytes());
    } else {
        libc::free(test);
    }

    libc::free(ptr);

    // Validate heap after free
    let test2 = libc::malloc(32);
    if test2.is_null() {
        let err_msg = format!(
            "[TRACKED_FREE] HEAP CORRUPTED after free! context='{}', ptr={:p}\n",
            context, ptr
        );
        stderr_write(err_msg.as_bytes());
    } else {
        libc::free(test2);
    }
}

/// Check if a pointer was already freed. Returns true if already freed.
fn mark_as_freed(ptr: *mut DooResult) -> bool {
    let addr = ptr as usize;
    let mut set = get_freed_results().lock().unwrap();

    // ATOMIC: insert() returns false if the value was already present
    // This prevents race conditions where multiple threads try to free the same address
    let was_newly_inserted = set.insert(addr);

    if !was_newly_inserted {
        // Address was already in the set - this is a double-free attempt!
        eprintln!(
            "[MARK_AS_FREED] Address {:p} (0x{:x}) ALREADY in freed set, returning true (skip free)",
            ptr, addr
        );
        return true;
    }

    eprintln!(
        "[MARK_AS_FREED] Address {:p} (0x{:x}) added to freed set, set size now {}",
        ptr,
        addr,
        set.len()
    );
    false
}

/// Remove a pointer from the freed set (called when address is reused for new allocation)
fn unmark_freed(ptr: *mut DooResult) {
    let addr = ptr as usize;
    if let Ok(mut set) = get_freed_results().lock() {
        let was_present = set.remove(&addr);
        if was_present {
            eprintln!(
                "[UNMARK_FREED] Address {:p} (0x{:x}) removed from freed set (reused by malloc)",
                ptr, addr
            );
        }
    }
}

static INIT: Once = Once::new();

fn load_env() {
    INIT.call_once(|| {
        // 1. Identify project root from finding the main .doo file in args
        let args: Vec<String> = env::args().collect();
        let mut loaded = false;

        for arg in &args {
            if arg.ends_with(".doo") {
                let path = std::path::Path::new(arg);
                if path.exists() {
                    // This is likely the entry file. Use its parent as root.
                    if let Some(parent) = path.parent() {
                        let env_path = parent.join(".env");
                        if env_path.exists() {
                            if dotenvy::from_path(&env_path).is_ok() {
                                loaded = true;
                                break;
                            }
                        }
                    }
                }
            }
        }

        // 2. Fallback to current directory .env if not found relative to .doo file
        if !loaded {
            let _ = dotenvy::from_filename(".env");
        }
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
    // Ownership-aware string freeing
    fn dooruntime_free_rc_string(ptr: *const c_char);
    // Runtime allocator for LLVM-managed memory
    fn dooruntime_malloc(size: usize) -> *mut libc::c_void;
    fn dooruntime_strdup(s: *const c_char) -> *mut c_char;
    fn dooruntime_free(ptr: *mut libc::c_void);
}

// Result type for FFI returns with ownership tracking
// tag: 0 = Ok, 1 = Err
// owner: 0 = LLVM (RC), 1 = FFI (libc), 2 = Rust (Box)
#[repr(C)]
pub struct DooResult {
    tag: i32,
    value: *mut std::ffi::c_void,
    owner: u8, // Owner enum: 0=LLVM, 1=FFI, 2=Rust
}

/// Owner enum constants for DooResult
pub mod owner {
    pub const LLVM: u8 = 0;
    pub const FFI: u8 = 1;
    pub const RUST: u8 = 2;
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

/// Convert Rust String to C string using RC layout expected by the compiler/runtime.
/// Layout: [rc:i32][len:i32][data...][0]
/// Returns pointer to data (base + 8).
fn string_to_c(s: String) -> *mut c_char {
    unsafe {
        let bytes = s.as_bytes();
        let len = bytes.len();

        // total_size = header(8) + data(len) + null(1)
        let total_size = len + 1 + 8;
        let alloc_size = (total_size + 15) & !15; // Align to 16 bytes

        let ptr = dooruntime_malloc(alloc_size) as *mut u8;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        // Zero memory for safety
        std::ptr::write_bytes(ptr, 0, alloc_size);

        // RC header
        *(ptr as *mut i32) = 1; // RC = 1
        *(ptr.add(4) as *mut i32) = len as i32; // Length

        // Copy bytes after header
        let data_ptr = ptr.add(8);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data_ptr, len);
        *data_ptr.add(len) = 0;

        data_ptr as *mut c_char
    }
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
    unsafe {
        let size = std::mem::size_of::<DooResult>();
        let ptr = tracked_malloc(size, "make_ok_db") as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        // Unmark in case this address was previously freed and is being reused
        unmark_freed(ptr);
        (*ptr).tag = 0;
        (*ptr).value = std::ptr::null_mut();
        (*ptr).owner = owner::FFI;
        ptr
    }
}

fn make_ok_database(connection_type: String, connected: bool) -> *mut DooResult {
    unsafe {
        // Allocate DooDatabase using libc malloc
        let db_size = std::mem::size_of::<DooDatabase>();
        let db = libc::malloc(db_size) as *mut DooDatabase;
        if db.is_null() {
            return std::ptr::null_mut();
        }
        (*db).connection_type = string_to_c(connection_type);
        (*db).connected = if connected { 1 } else { 0 };

        // Allocate DooResult using libc malloc
        let size = std::mem::size_of::<DooResult>();
        let ptr = tracked_malloc(size, "make_ok_int") as *mut DooResult;
        if ptr.is_null() {
            libc::free(db as *mut libc::c_void);
            return std::ptr::null_mut();
        }
        (*ptr).tag = 0;
        (*ptr).value = db as *mut _;
        (*ptr).owner = owner::FFI;
        ptr
    }
}

fn make_ok_string(s: String) -> *mut DooResult {
    unsafe {
        // AGGRESSIVE DEBUG: Log before allocation
        let s_len = s.len();
        if s_len > 10000 {
            let msg = format!(
                "[MAKE_OK_STRING] About to create DooResult for {} byte string\n",
                s_len
            );
            stderr_write(msg.as_bytes());
        }

        let size = std::mem::size_of::<DooResult>();
        let ptr = tracked_malloc(size, "make_ok_void") as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }

        // Unmark in case this address was previously freed and is being reused
        unmark_freed(ptr);

        // AGGRESSIVE DEBUG: Before string_to_c
        if s_len > 10000 {
            let msg = b"[MAKE_OK_STRING] Calling string_to_c for large string\n";
            stderr_write(msg);
        }

        let value_ptr = string_to_c(s);

        // AGGRESSIVE DEBUG: After string_to_c
        if s_len > 10000 {
            let msg = format!("[MAKE_OK_STRING] string_to_c returned {:p}\n", value_ptr);
            stderr_write(msg.as_bytes());
        }

        if value_ptr.is_null() {
            let msg = b"[MAKE_OK_STRING] ERROR: string_to_c returned null!\n";
            stderr_write(msg);
            libc::free(ptr as *mut libc::c_void);
            return std::ptr::null_mut();
        }

        (*ptr).tag = 0;
        (*ptr).value = value_ptr as *mut _;
        (*ptr).owner = owner::FFI;
        eprintln!(
            "[MAKE_OK_STRING] Created DooResult at {:p}, value at {:p}, owner=FFI (libc malloc)",
            ptr, value_ptr
        );
        ptr
    }
}

fn make_ok_int(n: i64) -> *mut DooResult {
    unsafe {
        let size = std::mem::size_of::<DooResult>();
        let ptr = tracked_malloc(size, "make_ok_string") as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        // Unmark in case this address was previously freed and is being reused
        unmark_freed(ptr);
        (*ptr).tag = 0;
        (*ptr).value = n as *mut std::ffi::c_void;
        (*ptr).owner = owner::FFI;
        ptr
    }
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

    unsafe {
        // Allocate DooDbError using libc allocator (consistent with rest of file)
        let err_size = std::mem::size_of::<DooDbError>();
        let err = tracked_malloc(err_size, "make_err_with_code_error") as *mut DooDbError;
        if err.is_null() {
            return std::ptr::null_mut();
        }
        (*err).code = -1;
        (*err).message = string_to_c(error_json);

        // Allocate DooResult using libc allocator
        let result_size = std::mem::size_of::<DooResult>();
        let ptr = tracked_malloc(result_size, "make_err_with_code_result") as *mut DooResult;
        if ptr.is_null() {
            tracked_free(err as *mut libc::c_void, "make_err_with_code_error_cleanup");
            return std::ptr::null_mut();
        }
        (*ptr).tag = 1;
        (*ptr).value = err as *mut _;
        (*ptr).owner = owner::FFI;
        ptr
    }
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
    eprintln!("[DOO_DB_FREE_STRING] Freeing string at {:p}", ptr);
    unsafe {
        // Strings are RC layout (data pointer), so free via runtime helper.
        dooruntime_free_rc_string(ptr as *const c_char);
    }
}

#[no_mangle]
pub extern "C" fn doo_db_result_free(ptr: *mut DooResult) {
    if ptr.is_null() {
        return;
    }

    // AGGRESSIVE DEBUG: Capture backtrace/caller info
    unsafe {
        let msg = format!("[DOO_DB_RESULT_FREE] ===== ENTRY ===== ptr={:p}\n", ptr);
        stderr_write(msg.as_bytes());
    }

    // CRITICAL FIX: Read struct fields FIRST, then check/mark as freed atomically.
    // This prevents a race condition where:
    // 1. Thread A checks mark_as_freed (returns false), about to read struct
    // 2. Thread B allocates same address (unmark_freed), uses it, frees it
    // 3. Thread A reads from now-freed memory → use-after-free!
    //
    // By reading fields first while we know ptr is valid (caller's responsibility),
    // we avoid reading from potentially freed memory later.
    let (owner, tag, value) = unsafe {
        let res = &*ptr;
        (res.owner, res.tag, res.value)
    };

    unsafe {
        let msg = format!(
            "[DOO_DB_RESULT_FREE] Read fields: owner={}, tag={}, value={:p}\n",
            owner, tag, value
        );
        stderr_write(msg.as_bytes());
    }

    // Now atomically check if already freed
    if mark_as_freed(ptr) {
        // Already freed - this is a double-free attempt, skip it
        unsafe {
            let msg = format!(
                "[DOO_DB_RESULT_FREE] !!!!! DOUBLE-FREE DETECTED !!!!! ptr={:p} was already freed!\n",
                ptr
            );
            stderr_write(msg.as_bytes());
        }
        eprintln!(
            "[DOO_DB_RESULT_FREE] SKIPPED - already freed (tracked in set) at {:p}",
            ptr
        );
        return;
    }

    eprintln!(
        "[DOO_DB_RESULT_FREE] Freeing result at {:p}, owner: {}, tag: {}",
        ptr, owner, tag
    );

    // IMPORTANT: DooResult is allocated with libc::malloc in this crate.
    // Using Box::from_raw here can cause allocator mismatch and heap corruption.
    unsafe {
        match owner {
            owner::LLVM => {
                // LLVM allocated - RC handles cleanup, don't free value
                // CRITICAL: If owner is LLVM, the result was allocated by FFI but
                // ownership was transferred to LLVM RC. The RC will free the string value.
                // We only free the DooResult wrapper here, not the string.
                eprintln!(
                    "[DOO_DB_RESULT_FREE] Owner is LLVM - freeing wrapper only, RC handles string"
                );

                // AGGRESSIVE DEBUG: Validate heap before free
                let test_alloc = libc::malloc(32);
                if test_alloc.is_null() {
                    let msg = b"[DOO_DB_RESULT_FREE] HEAP CORRUPTED before free (LLVM owner)!\n";
                    stderr_write(msg);
                } else {
                    libc::free(test_alloc);
                }

                eprintln!("[DOO_DB_RESULT_FREE] About to free wrapper at {:p}", ptr);
                tracked_free(ptr as *mut libc::c_void, "DooResult_LLVM_owner");

                // AGGRESSIVE DEBUG: Validate heap after free
                let test_alloc2 = libc::malloc(32);
                if test_alloc2.is_null() {
                    let msg = b"[DOO_DB_RESULT_FREE] HEAP CORRUPTED after free (LLVM owner)!\n";
                    stderr_write(msg);
                } else {
                    libc::free(test_alloc2);
                }
            }
            owner::FFI => {
                // FFI allocated the DooResult wrapper and value.
                // Key insight:
                // - Error values (tag != 0): These are COPIED into the HTTP error response,
                //   so we MUST free them here to prevent leaks.
                // - OK values (tag == 0): The compiler EXTRACTS the value pointer directly
                //   and stores it in LLVM-managed memory. The value is still in use,
                //   so we must NOT free it here - LLVM RC will free it later.

                if tag != 0 && !value.is_null() {
                    // Error value - DooDbError - FREE IT (it was copied to response)
                    eprintln!(
                        "[DOO_DB_RESULT_FREE] Owner is FFI, ERROR value - freeing error at {:p}",
                        value
                    );
                    let err_ptr = value as *mut DooDbError;
                    if !err_ptr.is_null() {
                        if !(*err_ptr).message.is_null() {
                            eprintln!(
                                "[DOO_DB_RESULT_FREE] Freeing error message at {:p}",
                                (*err_ptr).message
                            );
                            // Error messages are allocated as RC strings (data pointer).
                            // Free via centralized runtime helper to avoid invalid free/heap corruption.
                            dooruntime_free_rc_string((*err_ptr).message as *const c_char);
                        }
                        libc::free(err_ptr as *mut libc::c_void);
                    }
                } else {
                    // OK value - DON'T free it (compiler extracted and owns it via LLVM RC)
                    eprintln!(
                        "[DOO_DB_RESULT_FREE] Owner is FFI, OK value - NOT freeing (owned by LLVM RC)"
                    );
                }

                // Free ONLY the result wrapper
                eprintln!(
                    "[DOO_DB_RESULT_FREE] Freeing DooResult wrapper at {:p}",
                    ptr
                );

                // AGGRESSIVE DEBUG: Validate heap before free
                let test_alloc = libc::malloc(32);
                if test_alloc.is_null() {
                    let msg = b"[DOO_DB_RESULT_FREE] HEAP CORRUPTED before free (FFI owner)!\n";
                    stderr_write(msg);
                } else {
                    libc::free(test_alloc);
                }

                let msg = format!(
                    "[DOO_DB_RESULT_FREE] About to free FFI wrapper at {:p} (owner={}, tag={}, value={:p})\n",
                    ptr, owner, tag, value
                );
                stderr_write(msg.as_bytes());

                tracked_free(ptr as *mut libc::c_void, "DooResult_FFI_owner");

                // AGGRESSIVE DEBUG: Validate heap after free
                let test_alloc2 = libc::malloc(32);
                if test_alloc2.is_null() {
                    let msg = b"[DOO_DB_RESULT_FREE] HEAP CORRUPTED after free (FFI owner)!\n";
                    stderr_write(msg);
                } else {
                    libc::free(test_alloc2);
                    let msg = b"[DOO_DB_RESULT_FREE] Heap OK after free (FFI owner)\n";
                    stderr_write(msg);
                }
            }
            owner::RUST => {
                // Rust Box allocated - shouldn't happen in normal flow
                // But if it does, we can't safely free it here without knowing the allocator
                // Just free the wrapper with libc::free (might leak, but safer than double-free)

                // AGGRESSIVE DEBUG: Validate heap before free
                let test_alloc = libc::malloc(32);
                if test_alloc.is_null() {
                    let msg = b"[DOO_DB_RESULT_FREE] HEAP CORRUPTED before free (RUST owner)!\n";
                    stderr_write(msg);
                } else {
                    libc::free(test_alloc);
                }

                tracked_free(ptr as *mut libc::c_void, "DooResult_RUST_owner");

                // AGGRESSIVE DEBUG: Validate heap after free
                let test_alloc2 = libc::malloc(32);
                if test_alloc2.is_null() {
                    let msg = b"[DOO_DB_RESULT_FREE] HEAP CORRUPTED after free (RUST owner)!\n";
                    stderr_write(msg);
                } else {
                    libc::free(test_alloc2);
                }
            }
            _ => {
                // Unknown owner - default to FFI behavior, free wrapper only
                eprintln!(
                    "[DOO_DB_RESULT_FREE] Unknown owner {}, freeing wrapper only at {:p}",
                    owner, ptr
                );

                // AGGRESSIVE DEBUG: Validate heap before free
                let test_alloc = libc::malloc(32);
                if test_alloc.is_null() {
                    let msg = b"[DOO_DB_RESULT_FREE] HEAP CORRUPTED before free (unknown owner)!\n";
                    stderr_write(msg);
                } else {
                    libc::free(test_alloc);
                }

                tracked_free(ptr as *mut libc::c_void, "DooResult_unknown_owner");

                // AGGRESSIVE DEBUG: Validate heap after free
                let test_alloc2 = libc::malloc(32);
                if test_alloc2.is_null() {
                    let msg = b"[DOO_DB_RESULT_FREE] HEAP CORRUPTED after free (unknown owner)!\n";
                    stderr_write(msg);
                } else {
                    libc::free(test_alloc2);
                }
            }
        }
    } // end unsafe
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

        let row_count = array.len();
        let json = serde_json::Value::Array(array).to_string();
        eprintln!(
            "[DOO_DB_RAW] Query returned {} rows, JSON length: {}",
            row_count,
            json.len()
        );
        if json.len() < 500 {
            eprintln!("[DOO_DB_RAW] Full JSON: {}", json);
        } else {
            eprintln!(
                "[DOO_DB_RAW] JSON preview (first 200 chars): {}",
                &json[..200]
            );
        }
        eprintln!(
            "[DOO_DB_RAW] JSON first 50 bytes as hex: {:02x?}",
            &json.as_bytes()[..json.len().min(50)]
        );
        let result = make_ok_string(json.clone());
        eprintln!(
            "[DOO_DB_RAW] make_ok_string returned result at {:p}",
            result
        );
        if !result.is_null() {
            unsafe {
                eprintln!(
                    "[DOO_DB_RAW] Verifying result: tag={}, value={:p}, owner={}",
                    (*result).tag,
                    (*result).value,
                    (*result).owner
                );
                let verify_str = CStr::from_ptr((*result).value as *const c_char);
                eprintln!(
                    "[DOO_DB_RAW] Value string length: {}, first 50 chars: {}",
                    verify_str.to_bytes().len(),
                    verify_str
                        .to_str()
                        .unwrap_or("invalid")
                        .chars()
                        .take(50)
                        .collect::<String>()
                );
            }
        }
        eprintln!(
            "[DOO_DB_RAW] Created DooResult at {:p}, owner=LLVM (RC-managed via runtime allocator)",
            result
        );
        result
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

    eprintln!("[DOO_DB_RAW_PARAM] SQL: {}", sql_str);
    eprintln!("[DOO_DB_RAW_PARAM] Params: {}", params_str);

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
            Ok(r) => {
                eprintln!("[DOO_DB_RAW_PARAM] Query returned {} rows", r.len());
                r
            }
            Err(e) => {
                eprintln!("[DOO_DB_RAW_PARAM] Query error: {}", e);
                return make_err_query_failed(format!("Query failed: {}", e));
            }
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

        let row_count = array.len();
        let json = serde_json::Value::Array(array).to_string();
        eprintln!(
            "[DOO_DB_RAW_PARAM] Returning {} rows, JSON length: {}",
            row_count,
            json.len()
        );
        if json.len() < 500 {
            eprintln!("[DOO_DB_RAW_PARAM] Full JSON: {}", json);
        }
        let result = make_ok_string(json);
        eprintln!(
            "[DOO_DB_RAW_PARAM] Created DooResult at {:p}, owner=LLVM (RC-managed via runtime allocator)",
            result
        );
        result
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
            libc::free(field_c as *mut libc::c_void);
            libc::free(value_c as *mut libc::c_void);
            libc::free(table_c as *mut libc::c_void);
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
    unsafe {
        // Allocate DooDbError using libc::malloc
        let err_size = std::mem::size_of::<DooDbError>();
        let err = tracked_malloc(err_size, "make_err_not_null") as *mut DooDbError;
        if err.is_null() {
            return std::ptr::null_mut();
        }
        (*err).code = 2;
        (*err).message = string_to_c(format!("Field '{}' cannot be null", field));

        // Allocate DooResult using libc::malloc
        let result_size = std::mem::size_of::<DooResult>();
        let ptr = tracked_malloc(result_size, "make_err_not_null_result") as *mut DooResult;
        if ptr.is_null() {
            libc::free(err as *mut libc::c_void);
            return std::ptr::null_mut();
        }
        (*ptr).tag = 1;
        (*ptr).value = err as *mut _;
        (*ptr).owner = owner::FFI;
        ptr
    }
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

/// CRITICAL: Extract string from DooResult IMMEDIATELY before RC operations
///
/// ROOT CAUSE OF MEMORY CORRUPTION:
/// When db.raw() returns a DooResult with owner::LLVM, the LLVM compiler's RC system
/// wraps the string pointer in an RC header. However, when the RC count goes to zero,
/// it frees the string. Meanwhile, array_to_json_with_metadata receives an RC-wrapped
/// pointer that has already been freed, causing use-after-free corruption.
///
/// OWNERSHIP MODEL VIOLATION:
/// - FFI allocates string with libc::malloc and sets owner::LLVM
/// - LLVM RC treats this as RC-managed memory (WRONG!)
/// - RC frees the malloc'd string prematurely
/// - Result: Corrupted data when array_to_json tries to read it
///
/// THE PROPER FIX (COMPILER CHANGE REQUIRED):
/// The compiler must generate code that calls this function IMMEDIATELY after
/// doo_db_raw() or doo_db_rawWithParams(), BEFORE any RC operations:
///
///   let result = doo_db_raw(sql);  // Returns DooResult with malloc'd string
///   let safe_string = doo_db_extract_string_from_result(result);  // Copy and free result
///   // Now safe_string is a new malloc'd copy, safe from RC corruption
///   // Pass safe_string to array_to_json_with_metadata
///
/// This function creates a NEW malloc'd copy of the string and frees the original
/// DooResult, preventing the RC system from corrupting the data.
///
/// TEMPORARY WORKAROUND:
/// Until compiler is fixed, avoid using db.raw() for array returns in handlers.
/// Use CRUD endpoints or simple queries that return single values.
#[no_mangle]
pub extern "C" fn doo_db_extract_string_from_result(result: *mut DooResult) -> *mut c_char {
    if result.is_null() {
        eprintln!("[EXTRACT_STRING] Result is null");
        return string_to_c("[]".to_string());
    }

    unsafe {
        let res = &*result;
        eprintln!(
            "[EXTRACT_STRING] Extracting from DooResult at {:p}, tag={}, owner={}, value={:p}",
            result, res.tag, res.owner, res.value
        );

        // Check if this is an error
        if res.tag != 0 {
            eprintln!("[EXTRACT_STRING] Result is error, returning empty");
            return string_to_c("[]".to_string());
        }

        // Check if value is null
        if res.value.is_null() {
            eprintln!("[EXTRACT_STRING] Value is null, returning empty");
            return string_to_c("[]".to_string());
        }

        // Extract the string pointer
        let string_ptr = res.value as *mut c_char;
        eprintln!("[EXTRACT_STRING] Reading from string_ptr={:p}", string_ptr);

        // CRITICAL: Make a COPY of the string using string_to_c which allocates new memory
        // This creates a new malloc'd string that won't be affected by the original being freed
        let original_str = match CStr::from_ptr(string_ptr).to_str() {
            Ok(s) => {
                eprintln!(
                    "[EXTRACT_STRING] String read OK, length={}, first 100 chars: {}",
                    s.len(),
                    s.chars().take(100).collect::<String>()
                );
                s
            }
            Err(e) => {
                eprintln!("[EXTRACT_STRING] Failed to read string: {}", e);
                eprintln!(
                    "[EXTRACT_STRING] First 20 bytes as hex: {:02x?}",
                    std::slice::from_raw_parts(string_ptr as *const u8, 20)
                );
                return string_to_c("[]".to_string());
            }
        };

        eprintln!(
            "[EXTRACT_STRING] Copying string, length={}, first 50 bytes hex: {:02x?}",
            original_str.len(),
            &original_str.as_bytes()[..original_str.len().min(50)]
        );

        // Create a NEW copy of the string
        let new_string_ptr = string_to_c(original_str.to_string());

        eprintln!(
            "[EXTRACT_STRING] Created new string at {:p}",
            new_string_ptr
        );

        // Verify the new string
        let verify = CStr::from_ptr(new_string_ptr);
        eprintln!(
            "[EXTRACT_STRING] Verify new string: length={}, first 50 chars: {}",
            verify.to_bytes().len(),
            verify
                .to_str()
                .unwrap_or("invalid")
                .chars()
                .take(50)
                .collect::<String>()
        );

        // Now we can safely free the original DooResult
        // The string inside will be freed, but we have our copy
        eprintln!("[EXTRACT_STRING] Freeing original DooResult");
        doo_db_result_free(result);

        eprintln!(
            "[EXTRACT_STRING] ##### ABOUT TO RETURN new_string_ptr={:p} #####",
            new_string_ptr
        );
        use std::io::Write;
        let _ = std::io::stderr().flush();
        eprintln!("[EXTRACT_STRING] ##### RETURNING NOW #####");
        let _ = std::io::stderr().flush();
        new_string_ptr
    }
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
