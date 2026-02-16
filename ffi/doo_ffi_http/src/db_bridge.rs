//! Database Bridge — Runtime dynamic loading for database FFI
//!
//! We load doo_db symbols at runtime to avoid static duplication between DLLs.
//! Each DLL would have its own copy of POOL static if we used Rust imports.
//! Runtime loading ensures we call into the SAME doo_db.dll that was initialized.
//!
//! Also contains database execution helpers and SQL generation utilities.

use std::ffi::c_void;
use std::os::raw::c_char;
use std::sync::OnceLock;

use libloading::{Library, Symbol};

use doo_ffi_core::ffi_debug;

use crate::helpers::string_to_c;
use crate::metadata::StructMetadata;

// =============================================================================
// RUNTIME DYNAMIC LOADING FOR DATABASE FFI
// =============================================================================

/// Cached handle to the doo_db library
static DB_LIB: OnceLock<Option<Library>> = OnceLock::new();

/// Get or load the doo_db library
pub(crate) fn get_db_lib() -> Option<&'static Library> {
    DB_LIB
        .get_or_init(|| {
            // Try platform-specific library names
            // IMPORTANT: Library name is doo_ffi_db (from Cargo.toml name = "doo_ffi_db")
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
                    ffi_debug!("HTTP", "Successfully loaded database library: {}", name);
                    return Some(lib);
                }
            }
            ffi_debug!(
                "HTTP",
                "Warning: Could not load doo_db library (tried: {:?})",
                names
            );
            None
        })
        .as_ref()
}

/// Check if database pool is initialized (calls doo_db at runtime)
pub(crate) fn is_pool_initialized() -> bool {
    let Some(lib) = get_db_lib() else {
        return false;
    };

    type FnType = unsafe extern "C" fn() -> bool;
    let func: Result<Symbol<FnType>, _> = unsafe { lib.get(b"doo_db_is_connected") };

    match func {
        Ok(f) => unsafe { f() },
        Err(_) => false,
    }
}

/// Execute SQL and return JSON result (calls doo_db at runtime)
pub(crate) fn call_db_execute_sql(sql: *const c_char) -> *mut c_void {
    let Some(lib) = get_db_lib() else {
        return std::ptr::null_mut();
    };

    type FnType = unsafe extern "C" fn(*const c_char) -> *mut c_void;
    let func: Result<Symbol<FnType>, _> = unsafe { lib.get(b"doo_db_execute_sql") };

    match func {
        Ok(f) => unsafe { f(sql) },
        Err(_) => std::ptr::null_mut(),
    }
}

/// Execute parameterized query (calls doo_db at runtime)
pub(crate) fn call_db_query_with_params(sql: *const c_char, params: *const c_char) -> *mut c_void {
    let Some(lib) = get_db_lib() else {
        return std::ptr::null_mut();
    };

    type FnType = unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_void;
    let func: Result<Symbol<FnType>, _> = unsafe { lib.get(b"doo_db_query_with_params") };

    match func {
        Ok(f) => unsafe { f(sql, params) },
        Err(_) => std::ptr::null_mut(),
    }
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
