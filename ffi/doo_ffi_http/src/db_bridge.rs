//! Database Bridge — Runtime symbol resolution for database FFI
//!
//! Resolves doo_db symbols at runtime using two strategies:
//! 1. Current process lookup (static linking — all symbols in one binary)
//! 2. External library loading (dynamic linking — separate .dll/.so files)
//!
//! Also contains database execution helpers and SQL generation utilities.

use std::ffi::c_void;
use std::os::raw::c_char;
use std::sync::OnceLock;

use doo_ffi_core::ffi_debug;

use crate::helpers::string_to_c;
use crate::metadata::StructMetadata;

// =============================================================================
// DATABASE SYMBOL RESOLUTION — supports static and dynamic linking
// =============================================================================

/// Cached database function pointers
struct DbSymbols {
    is_connected: unsafe extern "C" fn() -> bool,
    execute_sql: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    query_with_params: unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_void,
}

/// Cached resolved DB symbols
static DB_SYMBOLS: OnceLock<Option<DbSymbols>> = OnceLock::new();

/// Get resolved DB function pointers, trying current process first then external libraries.
fn get_db_symbols() -> Option<&'static DbSymbols> {
    DB_SYMBOLS
        .get_or_init(|| {
            // Strategy 1: Find symbols in current process (static linking)
            if let Some(syms) = resolve_from_current_process() {
                ffi_debug!("HTTP", "DB symbols resolved from current process (static linking)");
                return Some(syms);
            }

            // Strategy 2: Load from external library file (dynamic linking fallback)
            if let Some(syms) = resolve_from_external_library() {
                ffi_debug!("HTTP", "DB symbols resolved from external library (dynamic linking)");
                return Some(syms);
            }

            ffi_debug!("HTTP", "Warning: Could not resolve doo_db symbols");
            None
        })
        .as_ref()
}

/// Try to find DB symbols in the current process (static linking case).
/// First checks the FFI bridge registry, then falls back to OS-level resolution.
fn resolve_from_current_process() -> Option<DbSymbols> {
    unsafe {
        let is_connected = find_symbol_in_process(b"doo_db_is_connected\0")?;
        let execute_sql = find_symbol_in_process(b"doo_db_execute_sql\0")?;
        let query_with_params = find_symbol_in_process(b"doo_db_query_with_params\0")?;

        Some(DbSymbols {
            is_connected: std::mem::transmute(is_connected),
            execute_sql: std::mem::transmute(execute_sql),
            query_with_params: std::mem::transmute(query_with_params),
        })
    }
}

/// Find a symbol by name — FFI bridge first, then OS-level resolution.
///
/// Priority order:
/// 1. FFI bridge registry (doo_ffi_core::ffi_bridge) — works for static linking
/// 2. OS-level resolution (dlsym/GetProcAddress) — works for dynamic linking
fn find_symbol_in_process(name: &[u8]) -> Option<*mut c_void> {
    // Strip trailing null byte for bridge lookup
    let name_str = std::str::from_utf8(
        &name[..name.len().saturating_sub(1)]
    ).unwrap_or("");

    // First: check FFI bridge registry (works for static linking on all platforms)
    if let Some(ptr) = doo_ffi_core::ffi_bridge::resolve(name_str) {
        return Some(ptr as *mut c_void);
    }

    // Fallback: OS-level symbol resolution
    find_symbol_os_level(name)
}

/// Find a symbol using OS-level APIs (dlsym on Unix, GetProcAddress on Windows).
#[cfg(unix)]
fn find_symbol_os_level(name: &[u8]) -> Option<*mut c_void> {
    let addr = unsafe {
        libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr() as *const c_char)
    };
    if addr.is_null() {
        None
    } else {
        Some(addr)
    }
}

#[cfg(windows)]
fn find_symbol_os_level(name: &[u8]) -> Option<*mut c_void> {
    // Enumerate all loaded modules and search for the symbol
    extern "system" {
        fn GetProcAddress(
            hModule: *mut c_void,
            lpProcName: *const i8,
        ) -> *mut c_void;
        fn GetCurrentProcess() -> *mut c_void;
        fn K32EnumProcessModules(
            hProcess: *mut c_void,
            lphModule: *mut *mut c_void,
            cb: u32,
            lpcbNeeded: *mut u32,
        ) -> i32;
    }

    unsafe {
        let sym = name.as_ptr() as *const i8;
        let process = GetCurrentProcess();
        let mut modules: [*mut c_void; 512] = [std::ptr::null_mut(); 512];
        let mut needed: u32 = 0;
        let ok = K32EnumProcessModules(
            process,
            modules.as_mut_ptr(),
            (modules.len() * std::mem::size_of::<*mut c_void>()) as u32,
            &mut needed,
        );

        if ok != 0 {
            let count = (needed as usize) / std::mem::size_of::<*mut c_void>();
            for i in 0..count.min(modules.len()) {
                if !modules[i].is_null() {
                    let addr = GetProcAddress(modules[i], sym);
                    if !addr.is_null() {
                        return Some(addr);
                    }
                }
            }
        }
    }

    None
}

/// Try to load DB symbols from an external library file (dynamic linking fallback).
fn resolve_from_external_library() -> Option<DbSymbols> {
    use libloading::{Library, Symbol};

    #[cfg(target_os = "windows")]
    let names = [
        "doo_ffi_db.dll",
        "libdoo_ffi_db.dll",
        "doo_db.dll",
        "libdoo_db.dll",
    ];
    #[cfg(target_os = "linux")]
    let names = [
        "libdoo_ffi_db.so",
        "doo_ffi_db.so",
        "libdoo_db.so",
        "doo_db.so",
    ];
    #[cfg(target_os = "macos")]
    let names = [
        "libdoo_ffi_db.dylib",
        "doo_ffi_db.dylib",
        "libdoo_db.dylib",
        "doo_db.dylib",
    ];

    for name in &names {
        if let Ok(lib) = unsafe { Library::new(name) } {
            ffi_debug!("HTTP", "Loaded database library: {}", name);

            unsafe {
                type IsConnFn = unsafe extern "C" fn() -> bool;
                type ExecFn = unsafe extern "C" fn(*const c_char) -> *mut c_void;
                type QueryFn = unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_void;

                let is_connected: Result<Symbol<IsConnFn>, _> = lib.get(b"doo_db_is_connected");
                let execute_sql: Result<Symbol<ExecFn>, _> = lib.get(b"doo_db_execute_sql");
                let query_with_params: Result<Symbol<QueryFn>, _> =
                    lib.get(b"doo_db_query_with_params");

                if let (Ok(ic), Ok(es), Ok(qp)) = (is_connected, execute_sql, query_with_params) {
                    // Copy function pointers BEFORE forgetting the library
                    let ic_fn: IsConnFn = std::mem::transmute(*ic);
                    let es_fn: ExecFn = std::mem::transmute(*es);
                    let qp_fn: QueryFn = std::mem::transmute(*qp);
                    // Leak the library to keep it loaded for the process lifetime
                    std::mem::forget(lib);
                    return Some(DbSymbols {
                        is_connected: ic_fn,
                        execute_sql: es_fn,
                        query_with_params: qp_fn,
                    });
                }
            }
        }
    }

    None
}

/// Check if database pool is initialized (calls doo_db at runtime)
pub(crate) fn is_pool_initialized() -> bool {
    let Some(syms) = get_db_symbols() else {
        return false;
    };
    unsafe { (syms.is_connected)() }
}

/// Execute SQL and return JSON result (calls doo_db at runtime)
pub(crate) fn call_db_execute_sql(sql: *const c_char) -> *mut c_void {
    let Some(syms) = get_db_symbols() else {
        return std::ptr::null_mut();
    };
    unsafe { (syms.execute_sql)(sql) }
}

/// Execute parameterized query (calls doo_db at runtime)
pub(crate) fn call_db_query_with_params(sql: *const c_char, params: *const c_char) -> *mut c_void {
    let Some(syms) = get_db_symbols() else {
        return std::ptr::null_mut();
    };
    unsafe { (syms.query_with_params)(sql, params) }
}

// ============================================================================
// SQL GENERATION HELPERS
// ============================================================================

/// Convert PascalCase or camelCase to snake_case
/// Examples: "AuthorId" -> "author_id", "firstName" -> "first_name"
pub(crate) fn to_snake_case(name: &str) -> String {
    let mut result = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(ch.to_lowercase().next().unwrap_or(ch));
        } else {
            result.push(ch);
        }
    }
    result
}

/// Convert snake_case to PascalCase for JSON field names
/// Special case: "id" stays "id" (Doo convention)
pub(crate) fn to_pascal_case(s: &str) -> String {
    // Special case: "id" should stay lowercase
    if s == "id" {
        return "id".to_string();
    }

    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}

/// Generate CREATE TABLE SQL from struct metadata
/// Uses snake_case for column names (PostgreSQL convention)
pub(crate) fn generate_create_table_sql(table_name: &str, metadata: &StructMetadata) -> String {
    let mut columns = Vec::new();

    for field in &metadata.fields {
        // Convert field name to snake_case for PostgreSQL convention
        let col_name = to_snake_case(&field.name);

        // Smart fallback: if field is named "id" with Int type, make it SERIAL PRIMARY KEY
        // This is needed because decorators are not passed from codegen
        let is_id_field = col_name == "id" && field.field_type == "Int";

        if is_id_field {
            columns.push(format!("  {} SERIAL PRIMARY KEY", col_name));
            continue;
        }

        // Map Doo types to PostgreSQL types
        let sql_type = match field.field_type.as_str() {
            "Int" => "INTEGER",
            "Float" => "REAL",
            "Bool" => "BOOLEAN",
            "Str" | "String" => "TEXT",
            _ => "TEXT", // Default to TEXT for unknown types
        };

        let mut col_def = format!("  {} {}", col_name, sql_type);

        // Check decorators (if they ever get passed)
        for dec in &field.decorators {
            if dec == "primary" || dec == "@primary" {
                col_def.push_str(" PRIMARY KEY");
            }
            if dec == "auto" || dec == "@auto" {
                col_def = format!("  {} SERIAL PRIMARY KEY", col_name);
            }
            if dec == "unique" || dec == "@unique" {
                col_def.push_str(" UNIQUE");
            }
            if dec.starts_with("default(") || dec.starts_with("@default(") {
                // Extract default value
                let start = dec.find('(').unwrap_or(0) + 1;
                let end = dec.rfind(')').unwrap_or(dec.len());
                let default_val = &dec[start..end];
                // Quote strings, leave numbers/booleans as-is
                if default_val == "true" || default_val == "false" {
                    col_def.push_str(&format!(" DEFAULT {}", default_val));
                } else if default_val.parse::<i64>().is_ok() || default_val.parse::<f64>().is_ok() {
                    col_def.push_str(&format!(" DEFAULT {}", default_val));
                } else {
                    // String value - remove surrounding quotes if present
                    let clean_val = default_val.trim_matches('"').trim_matches('\'');
                    col_def.push_str(&format!(" DEFAULT '{}'", clean_val));
                }
            }
        }

        columns.push(col_def);
    }

    format!(
        "CREATE TABLE IF NOT EXISTS {} (\n{}\n)",
        table_name,
        columns.join(",\n")
    )
}

// ============================================================================
// DATABASE EXECUTION HELPERS - Using FFI calls to doo_db.dll
// ============================================================================
// CRITICAL: All database operations must call FFI functions in doo_db.dll
// to ensure we use the correct shared POOL state.

/// Execute SQL query and return JSON result via FFI
pub(crate) fn execute_db_query(sql: &str) -> Result<String, String> {
    if !is_pool_initialized() {
        return Err("Database not connected".to_string());
    }

    let sql_c = unsafe {
        let len = sql.len();
        let ptr = libc::malloc(len + 1) as *mut u8;
        if ptr.is_null() {
            return Err("Memory allocation failed".to_string());
        }
        std::ptr::copy_nonoverlapping(sql.as_ptr(), ptr, len);
        *ptr.add(len) = 0;
        ptr as *const c_char
    };

    let result = call_db_execute_sql(sql_c);
    unsafe {
        libc::free(sql_c as *mut c_void);
    }

    if result.is_null() {
        return Err("Query execution failed".to_string());
    }

    let json = unsafe {
        let c_str = std::ffi::CStr::from_ptr(result as *const c_char);
        let s = c_str.to_string_lossy().into_owned();
        libc::free(result);
        s
    };

    Ok(json)
}

/// Execute SQL statement (INSERT/UPDATE/DELETE) via FFI
pub(crate) fn execute_db_statement(sql: &str) -> Result<u64, String> {
    if !is_pool_initialized() {
        return Err("Database not connected".to_string());
    }

    let sql_c = unsafe {
        let len = sql.len();
        let ptr = libc::malloc(len + 1) as *mut u8;
        if ptr.is_null() {
            return Err("Memory allocation failed".to_string());
        }
        std::ptr::copy_nonoverlapping(sql.as_ptr(), ptr, len);
        *ptr.add(len) = 0;
        ptr as *const c_char
    };

    let result = call_db_execute_sql(sql_c);
    unsafe {
        libc::free(sql_c as *mut c_void);
    }

    if result.is_null() {
        return Err("Statement execution failed".to_string());
    }

    let json = unsafe {
        let c_str = std::ffi::CStr::from_ptr(result as *const c_char);
        let s = c_str.to_string_lossy().into_owned();
        libc::free(result);
        s
    };

    // Parse {"affected_rows": N} response
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
        if let Some(n) = v.get("affected_rows").and_then(|v| v.as_u64()) {
            return Ok(n);
        }
    }

    Ok(0)
}

/// Execute parameterized query with a single string param and return JSON result
pub(crate) fn execute_db_query_with_string_param(sql: &str, param: &str) -> Result<String, String> {
    if !is_pool_initialized() {
        return Err("Database not connected".to_string());
    }

    let params_json = serde_json::to_string(&vec![param]).unwrap_or_else(|_| "[]".to_string());

    let sql_c = string_to_c(sql);
    let params_c = string_to_c(&params_json);

    let result = call_db_query_with_params(sql_c, params_c);

    unsafe {
        libc::free(sql_c as *mut c_void);
        libc::free(params_c as *mut c_void);
    }

    if result.is_null() {
        return Err("Query execution failed".to_string());
    }

    let json = unsafe {
        let c_str = std::ffi::CStr::from_ptr(result as *const c_char);
        let s = c_str.to_string_lossy().into_owned();
        libc::free(result);
        s
    };

    Ok(json)
}

/// Execute INSERT with JSON values as parameters and return the result
pub(crate) fn execute_db_insert(sql: &str, values: &[serde_json::Value]) -> Result<String, String> {
    if !is_pool_initialized() {
        return Err("Database not connected".to_string());
    }

    let params_json = serde_json::to_string(&values).unwrap_or_else(|_| "[]".to_string());

    let sql_c = string_to_c(sql);
    let params_c = string_to_c(&params_json);

    let result = call_db_query_with_params(sql_c, params_c);

    unsafe {
        libc::free(sql_c as *mut c_void);
        libc::free(params_c as *mut c_void);
    }

    if result.is_null() {
        return Err("Insert execution failed".to_string());
    }

    let json = unsafe {
        let c_str = std::ffi::CStr::from_ptr(result as *const c_char);
        let s = c_str.to_string_lossy().into_owned();
        libc::free(result);
        s
    };

    Ok(json)
}

/// Execute parameterized query by ID and return JSON result
pub(crate) fn execute_db_query_by_id(sql: &str, id: i32) -> Result<String, String> {
    if !is_pool_initialized() {
        return Err("Database not connected".to_string());
    }

    let params_json = serde_json::to_string(&vec![id]).unwrap_or_else(|_| "[]".to_string());

    let sql_c = string_to_c(sql);
    let params_c = string_to_c(&params_json);

    let result = call_db_query_with_params(sql_c, params_c);

    unsafe {
        libc::free(sql_c as *mut c_void);
        libc::free(params_c as *mut c_void);
    }

    if result.is_null() {
        return Err("Query execution failed".to_string());
    }

    let json = unsafe {
        let c_str = std::ffi::CStr::from_ptr(result as *const c_char);
        let s = c_str.to_string_lossy().into_owned();
        libc::free(result);
        s
    };

    Ok(json)
}

/// Execute delete by ID
pub(crate) fn execute_db_delete_by_id(sql: &str, id: i32) -> Result<u64, String> {
    if !is_pool_initialized() {
        return Err("Database not connected".to_string());
    }

    let params_json = serde_json::to_string(&vec![id]).unwrap_or_else(|_| "[]".to_string());

    let sql_c = string_to_c(sql);
    let params_c = string_to_c(&params_json);

    let result = call_db_query_with_params(sql_c, params_c);

    unsafe {
        libc::free(sql_c as *mut c_void);
        libc::free(params_c as *mut c_void);
    }

    if result.is_null() {
        return Err("Delete execution failed".to_string());
    }

    let json = unsafe {
        let c_str = std::ffi::CStr::from_ptr(result as *const c_char);
        let s = c_str.to_string_lossy().into_owned();
        libc::free(result);
        s
    };

    // Parse {"affected_rows": N} response
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
        if let Some(n) = v.get("affected_rows").and_then(|v| v.as_u64()) {
            return Ok(n);
        }
    }

    Ok(0)
}
