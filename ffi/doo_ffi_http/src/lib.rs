//! doo_ffi_http - Complete HTTP FFI Library
//!
//! Provides all HTTP functionality for Doo applications:
//! - Route registration (GET, POST, PUT, DELETE, PATCH)
//! - Middleware (JWT, CORS, Rate Limiting)
//! - RFC 7807 error responses
//! - Request helpers (params, query, headers)
//! - Server lifecycle

mod error;
mod helpers;
mod middleware;
mod router;
mod server;
mod types;
pub mod ws;

use std::collections::HashMap;
use std::ffi::{c_void, CStr};
use std::os::raw::c_char;

use doo_ffi_core::constants::{MIDDLEWARE_CORS, MIDDLEWARE_JWT, MIDDLEWARE_RATELIMIT};
use doo_ffi_core::ffi_debug;

pub use error::*;
pub use helpers::*;
pub use middleware::*;
pub use router::*;
pub use types::*;

// =============================================================================
// RUNTIME DYNAMIC LOADING FOR DATABASE FFI
// =============================================================================
// We load doo_db symbols at runtime to avoid static duplication between DLLs.
// Each DLL would have its own copy of POOL static if we used Rust imports.
// Runtime loading ensures we call into the SAME doo_db.dll that was initialized.

use libloading::{Library, Symbol};
use std::sync::OnceLock;

/// Cached handle to the doo_db library
static DB_LIB: OnceLock<Option<Library>> = OnceLock::new();

/// Get or load the doo_db library
fn get_db_lib() -> Option<&'static Library> {
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
fn is_pool_initialized() -> bool {
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
fn call_db_execute_sql(sql: *const c_char) -> *mut std::ffi::c_void {
    let Some(lib) = get_db_lib() else {
        return std::ptr::null_mut();
    };

    type FnType = unsafe extern "C" fn(*const c_char) -> *mut std::ffi::c_void;
    let func: Result<Symbol<FnType>, _> = unsafe { lib.get(b"doo_db_execute_sql") };

    match func {
        Ok(f) => unsafe { f(sql) },
        Err(_) => std::ptr::null_mut(),
    }
}

/// Execute parameterized query (calls doo_db at runtime)
fn call_db_query_with_params(sql: *const c_char, params: *const c_char) -> *mut std::ffi::c_void {
    let Some(lib) = get_db_lib() else {
        return std::ptr::null_mut();
    };

    type FnType = unsafe extern "C" fn(*const c_char, *const c_char) -> *mut std::ffi::c_void;
    let func: Result<Symbol<FnType>, _> = unsafe { lib.get(b"doo_db_query_with_params") };

    match func {
        Ok(f) => unsafe { f(sql, params) },
        Err(_) => std::ptr::null_mut(),
    }
}

// ============================================================================
// SERVER LIFECYCLE
// ============================================================================

/// Global server instance pointer — accessible to handler wrappers that need `app: Server`.
static GLOBAL_SERVER_PTR: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

/// Get the global server instance pointer. Called by codegen-generated handler
/// wrappers when a user handler has `app: Server` parameter.
#[no_mangle]
pub extern "C" fn doo_http_get_server_instance() -> *const c_void {
    GLOBAL_SERVER_PTR
        .get()
        .map(|p| *p as *const c_void)
        .unwrap_or(std::ptr::null())
}

#[no_mangle]
pub extern "C" fn doo_http_server_new(host_port: *const c_char) -> *mut c_void {
    let host_port_str = if host_port.is_null() {
        ":3000".to_string()
    } else {
        c_to_string(host_port)
    };

    // Parse host:port
    let (host, port) = if let Some(colon) = host_port_str.rfind(':') {
        let h = &host_port_str[..colon];
        let p: i64 = host_port_str[colon + 1..].parse().unwrap_or(3000);
        (if h.is_empty() { "127.0.0.1" } else { h }.to_string(), p)
    } else {
        ("127.0.0.1".to_string(), 3000i64)
    };

    // Allocate server struct matching LLVM's %Server = type { i64, ptr }
    // Layout: Port (i64) at offset 0, Host (ptr) at offset 8
    unsafe {
        let ptr = libc::malloc(16) as *mut u8;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        // Store Port as i64 at offset 0
        *(ptr as *mut i64) = port;
        // Store Host as ptr at offset 8
        *(ptr.add(8) as *mut *const c_char) = string_to_c(&host);
        ptr as *mut c_void
    }
}

#[no_mangle]
pub extern "C" fn doo_http_listen(server_ptr: *const c_void) -> *mut DooResult {
    // Store global server pointer for handler wrappers with `app: Server` param
    let _ = GLOBAL_SERVER_PTR.set(server_ptr as usize);

    let (host, port) = if server_ptr.is_null() {
        ("0.0.0.0".to_string(), 3000u16)
    } else {
        unsafe {
            // Server struct: { i64 Port, ptr Host }
            let port = *(server_ptr as *const i64) as u16;
            let host_ptr = *((server_ptr as *const u8).add(8) as *const *const c_char);
            (c_to_string(host_ptr), port)
        }
    };

    match server::start_server(&host, port) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err_http(500, &e),
    }
}

// ============================================================================
// ROUTE REGISTRATION
// ============================================================================

#[no_mangle]
pub extern "C" fn doo_http_get(
    _server: *const c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    register_route("GET", path, handler_name)
}

#[no_mangle]
pub extern "C" fn doo_http_get_fn(
    _server: *const c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    register_route_fn("GET", path, handler)
}

#[no_mangle]
pub extern "C" fn doo_http_post(
    _server: *const c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    register_route("POST", path, handler_name)
}

#[no_mangle]
pub extern "C" fn doo_http_post_fn(
    _server: *const c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    register_route_fn("POST", path, handler)
}

#[no_mangle]
pub extern "C" fn doo_http_put(
    _server: *const c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    register_route("PUT", path, handler_name)
}

#[no_mangle]
pub extern "C" fn doo_http_put_fn(
    _server: *const c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    register_route_fn("PUT", path, handler)
}

#[no_mangle]
pub extern "C" fn doo_http_delete(
    _server: *const c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    register_route("DELETE", path, handler_name)
}

#[no_mangle]
pub extern "C" fn doo_http_delete_fn(
    _server: *const c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    register_route_fn("DELETE", path, handler)
}

#[no_mangle]
pub extern "C" fn doo_http_patch(
    _server: *const c_void,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    register_route("PATCH", path, handler_name)
}

#[no_mangle]
pub extern "C" fn doo_http_patch_fn(
    _server: *const c_void,
    path: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    register_route_fn("PATCH", path, handler)
}

fn register_route(
    method: &str,
    path: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    let handler_str = c_to_string(handler_name);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.register_by_name(method, &path_str, &handler_str);
    make_ok_void()
}

fn register_route_fn(method: &str, path: *const c_char, handler: DooHandlerFn) -> *mut DooResult {
    let path_str = c_to_string(path);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.register(method, &path_str, handler);
    make_ok_void()
}

// ============================================================================
// ROUTE REGISTRATION WITH MIDDLEWARE - Single Source of Truth
// ============================================================================
// All *_with_middleware functions use this centralized helper

/// Centralized helper: Register route with middleware names (comma-separated) and function pointer handler
fn register_route_with_middleware_fn(
    method: &str,
    path: *const c_char,
    middleware_names: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    let middleware_str = c_to_string(middleware_names);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();

    // Parse middleware names (comma-separated)
    let middleware_list: Vec<String> = if middleware_str.is_empty() {
        vec![]
    } else {
        middleware_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    // Lookup middleware functions and auto-register built-ins
    let mut middleware_fns = Vec::new();
    for mw_name in middleware_list {
        // Auto-register built-in middleware if referenced
        if mw_name == MIDDLEWARE_JWT && !registry.middleware_handlers.contains_key(MIDDLEWARE_JWT) {
            registry
                .middleware_handlers
                .insert(MIDDLEWARE_JWT.to_string(), jwt_middleware_handler);
        }
        if mw_name == MIDDLEWARE_CORS && !registry.middleware_handlers.contains_key(MIDDLEWARE_CORS)
        {
            registry
                .middleware_handlers
                .insert(MIDDLEWARE_CORS.to_string(), cors_middleware_handler);
        }
        if mw_name == MIDDLEWARE_RATELIMIT
            && !registry
                .middleware_handlers
                .contains_key(MIDDLEWARE_RATELIMIT)
        {
            registry.middleware_handlers.insert(
                MIDDLEWARE_RATELIMIT.to_string(),
                ratelimit_middleware_handler,
            );
        }

        if let Some(mw_fn) = registry.middleware_handlers.get(&mw_name).copied() {
            middleware_fns.push(mw_fn);
        }
    }

    registry.register_with_middleware(method, &path_str, handler, middleware_fns);
    make_ok_void()
}

/// Centralized helper: Register route with middleware names (comma-separated) and handler name
fn register_route_with_middleware(
    method: &str,
    path: *const c_char,
    middleware_names: *const c_char,
    handler_name: *const c_char,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    let middleware_str = c_to_string(middleware_names);
    let handler_str = c_to_string(handler_name);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();

    // Parse middleware names (comma-separated)
    let middleware_list: Vec<String> = if middleware_str.is_empty() {
        vec![]
    } else {
        middleware_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    // Lookup middleware functions and auto-register built-ins
    let mut middleware_fns = Vec::new();
    for mw_name in middleware_list {
        // Auto-register built-in middleware if referenced
        if mw_name == MIDDLEWARE_JWT && !registry.middleware_handlers.contains_key(MIDDLEWARE_JWT) {
            registry
                .middleware_handlers
                .insert(MIDDLEWARE_JWT.to_string(), jwt_middleware_handler);
        }
        if mw_name == MIDDLEWARE_CORS && !registry.middleware_handlers.contains_key(MIDDLEWARE_CORS)
        {
            registry
                .middleware_handlers
                .insert(MIDDLEWARE_CORS.to_string(), cors_middleware_handler);
        }
        if mw_name == MIDDLEWARE_RATELIMIT
            && !registry
                .middleware_handlers
                .contains_key(MIDDLEWARE_RATELIMIT)
        {
            registry.middleware_handlers.insert(
                MIDDLEWARE_RATELIMIT.to_string(),
                ratelimit_middleware_handler,
            );
        }

        if let Some(mw_fn) = registry.middleware_handlers.get(&mw_name).copied() {
            middleware_fns.push(mw_fn);
        }
    }

    registry.register_by_name_with_middleware(method, &path_str, &handler_str, middleware_fns);
    make_ok_void()
}

#[no_mangle]
pub extern "C" fn doo_http_get_with_middleware(
    _server: *const c_void,
    path: *const c_char,
    middleware_names: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    register_route_with_middleware_fn("GET", path, middleware_names, handler)
}

#[no_mangle]
pub extern "C" fn doo_http_post_with_middleware(
    _server: *const c_void,
    path: *const c_char,
    middleware_names: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    register_route_with_middleware_fn("POST", path, middleware_names, handler)
}

#[no_mangle]
pub extern "C" fn doo_http_put_with_middleware(
    _server: *const c_void,
    path: *const c_char,
    middleware_names: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    register_route_with_middleware_fn("PUT", path, middleware_names, handler)
}

#[no_mangle]
pub extern "C" fn doo_http_delete_with_middleware(
    _server: *const c_void,
    path: *const c_char,
    middleware_names: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    register_route_with_middleware_fn("DELETE", path, middleware_names, handler)
}

#[no_mangle]
pub extern "C" fn doo_http_patch_with_middleware(
    _server: *const c_void,
    path: *const c_char,
    middleware_names: *const c_char,
    handler: DooHandlerFn,
) -> *mut DooResult {
    register_route_with_middleware_fn("PATCH", path, middleware_names, handler)
}

// ============================================================================
// GLOBAL STRUCT/ENUM METADATA REGISTRY
// ============================================================================
// Single Source of Truth for struct/enum metadata used by auth, crud, and handlers.
// The compiler emits calls to register metadata, which is then used at runtime.

use std::sync::Mutex as StdMutex;

/// Global struct metadata registry - stores field info for structs
static STRUCT_REGISTRY: std::sync::OnceLock<StdMutex<HashMap<String, StructMetadata>>> =
    std::sync::OnceLock::new();

/// Global enum metadata registry - stores variants for enums
static ENUM_REGISTRY: std::sync::OnceLock<StdMutex<HashMap<String, Vec<String>>>> =
    std::sync::OnceLock::new();

fn get_struct_registry() -> &'static StdMutex<HashMap<String, StructMetadata>> {
    STRUCT_REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn get_enum_registry() -> &'static StdMutex<HashMap<String, Vec<String>>> {
    ENUM_REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// Struct metadata for runtime validation
#[derive(Clone, Debug)]
pub struct StructMetadata {
    pub name: String,
    pub fields: Vec<FieldMetadata>,
}

/// Field metadata for runtime validation
#[derive(Clone, Debug)]
pub struct FieldMetadata {
    pub name: String,
    pub field_type: String,
    pub decorators: Vec<String>, // e.g., ["email", "min(3)", "max(50)"]
}

/// Register struct metadata from the compiler.
/// Called by codegen when processing structs used with auth/crud/handlers.
#[no_mangle]
pub extern "C" fn doo_http_register_struct_metadata(
    struct_name: *const c_char,
    metadata_json: *const c_char,
) {
    let name = c_to_string(struct_name);
    let json_str = c_to_string(metadata_json);

    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_str) {
        let mut fields = Vec::new();

        if let Some(fields_arr) = parsed.get("fields").and_then(|v| v.as_array()) {
            for field in fields_arr {
                if let (Some(fname), Some(ftype)) = (
                    field.get("name").and_then(|v| v.as_str()),
                    field.get("type").and_then(|v| v.as_str()),
                ) {
                    let decorators: Vec<String> = field
                        .get("decorators")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();

                    fields.push(FieldMetadata {
                        name: fname.to_string(),
                        field_type: ftype.to_string(),
                        decorators,
                    });
                }
            }
        }

        let mut registry = get_struct_registry().lock().unwrap();
        registry.insert(name.clone(), StructMetadata { name, fields });
    }
}

/// Register enum metadata from the compiler.
#[no_mangle]
pub extern "C" fn doo_http_register_enum_metadata(
    enum_name: *const c_char,
    variants_json: *const c_char,
) {
    let name = c_to_string(enum_name);
    let json_str = c_to_string(variants_json);

    if let Ok(parsed) = serde_json::from_str::<Vec<String>>(&json_str) {
        let mut registry = get_enum_registry().lock().unwrap();
        registry.insert(name, parsed);
    }
}

/// Get struct metadata by name (used by auth/crud handlers)
fn get_struct_metadata(name: &str) -> Option<StructMetadata> {
    let registry = get_struct_registry().lock().unwrap();
    registry.get(name).cloned()
}

/// Get enum variants by name (used for validation)
fn get_enum_variants(name: &str) -> Option<Vec<String>> {
    let registry = get_enum_registry().lock().unwrap();
    registry.get(name).cloned()
}

// ============================================================================
// DATABASE-BACKED CRUD HELPERS
// ============================================================================

/// Convert PascalCase or camelCase to snake_case
/// Examples: "AuthorId" -> "author_id", "firstName" -> "first_name"
fn to_snake_case(name: &str) -> String {
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

/// Generate CREATE TABLE SQL from struct metadata
/// Uses snake_case for column names (PostgreSQL convention)
fn generate_create_table_sql(table_name: &str, metadata: &StructMetadata) -> String {
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
fn execute_db_query(sql: &str) -> Result<String, String> {
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
        libc::free(sql_c as *mut std::ffi::c_void);
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
fn execute_db_statement(sql: &str) -> Result<u64, String> {
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
        libc::free(sql_c as *mut std::ffi::c_void);
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

/// Helper to create C string from Rust string
fn string_to_c_local(s: &str) -> *const c_char {
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

/// Execute parameterized query with a single string param and return JSON result
fn execute_db_query_with_string_param(sql: &str, param: &str) -> Result<String, String> {
    if !is_pool_initialized() {
        return Err("Database not connected".to_string());
    }

    let params_json = serde_json::to_string(&vec![param]).unwrap_or_else(|_| "[]".to_string());

    let sql_c = string_to_c_local(sql);
    let params_c = string_to_c_local(&params_json);

    let result = call_db_query_with_params(sql_c, params_c);

    unsafe {
        libc::free(sql_c as *mut std::ffi::c_void);
        libc::free(params_c as *mut std::ffi::c_void);
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
fn execute_db_insert(sql: &str, values: &[serde_json::Value]) -> Result<String, String> {
    if !is_pool_initialized() {
        return Err("Database not connected".to_string());
    }

    let params_json = serde_json::to_string(&values).unwrap_or_else(|_| "[]".to_string());

    let sql_c = string_to_c_local(sql);
    let params_c = string_to_c_local(&params_json);

    let result = call_db_query_with_params(sql_c, params_c);

    unsafe {
        libc::free(sql_c as *mut std::ffi::c_void);
        libc::free(params_c as *mut std::ffi::c_void);
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
fn execute_db_query_by_id(sql: &str, id: i32) -> Result<String, String> {
    if !is_pool_initialized() {
        return Err("Database not connected".to_string());
    }

    let params_json = serde_json::to_string(&vec![id]).unwrap_or_else(|_| "[]".to_string());

    let sql_c = string_to_c_local(sql);
    let params_c = string_to_c_local(&params_json);

    let result = call_db_query_with_params(sql_c, params_c);

    unsafe {
        libc::free(sql_c as *mut std::ffi::c_void);
        libc::free(params_c as *mut std::ffi::c_void);
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
fn execute_db_delete_by_id(sql: &str, id: i32) -> Result<u64, String> {
    if !is_pool_initialized() {
        return Err("Database not connected".to_string());
    }

    let params_json = serde_json::to_string(&vec![id]).unwrap_or_else(|_| "[]".to_string());

    let sql_c = string_to_c_local(sql);
    let params_c = string_to_c_local(&params_json);

    let result = call_db_query_with_params(sql_c, params_c);

    unsafe {
        libc::free(sql_c as *mut std::ffi::c_void);
        libc::free(params_c as *mut std::ffi::c_void);
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

/// Convert snake_case to PascalCase for JSON field names
/// Special case: "id" stays "id" (Doo convention)
fn to_pascal_case(s: &str) -> String {
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

/// Map error enum variant name to HTTP status code.
/// This is the centralized mapping for all error enums.
/// Uses common HTTP error naming conventions (Unauthorized -> 401, Forbidden -> 403, etc.)
#[no_mangle]
pub extern "C" fn doohttp_error_variant_to_status(
    enum_name: *const c_char,
    variant_name: *const c_char,
    variant_index: i32,
) -> i32 {
    let _enum_name = c_to_string(enum_name);
    let variant_str = c_to_string(variant_name);

    // Common error status mappings based on variant name
    match variant_str.as_str() {
        "Unauthorized" => 401,
        "Forbidden" => 403,
        "NotFound" => 404,
        "MethodNotAllowed" => 405,
        "Conflict" => 409,
        "ValidationError" => 422,
        "TooManyRequests" => 429,
        "InternalError" | "ServerError" => 500,
        "BadRequest" => 400,
        "NotImplemented" => 501,
        "ServiceUnavailable" => 503,
        // Default to 500 + variant_index for unknown variants
        _ => 500 + variant_index,
    }
}

/// Build RFC 7807 error JSON from status code and title
#[no_mangle]
pub extern "C" fn doohttp_build_rfc7807_error(status: i32, title: *const c_char) -> *const c_char {
    let title_str = c_to_string(title);

    // Build RFC 7807 compliant error JSON using centralized source
    let err = Rfc7807Error::new(status as u16, &title_str);
    string_to_c(&err.to_json())
}

/// Format an error message string as JSON with {"error": "message"} format.
/// This is used by generated wrapper code to format database/FFI errors for HTTP responses.
#[no_mangle]
pub extern "C" fn doohttp_format_error_as_json(error_msg: *const c_char) -> *const c_char {
    let msg = if error_msg.is_null() {
        "Unknown error".to_string()
    } else {
        c_to_string(error_msg)
    };

    // Escape any special JSON characters in the message
    let escaped = msg
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");

    let error_json = format!(r#"{{"error":"{}"}}"#, escaped);
    string_to_c(&error_json)
}

/// Validate an item against its struct schema using centralized metadata.
/// Returns Ok(()) if valid, Err(error_json) if validation fails.
fn validate_item_against_schema(
    item: &serde_json::Value,
    resource_name: &str,
    path: &str,
) -> Result<(), String> {
    // Look up struct metadata for this resource
    // The CRUD config stores struct name (e.g., "Task"), need to find it
    let struct_name = {
        let routes = get_routes();
        let registry = routes.lock().unwrap();
        registry
            .crud_configs
            .iter()
            .find(|c| c.base_path.trim_start_matches('/') == resource_name)
            .map(|c| c.resource_struct.clone())
    };

    let struct_name = match struct_name {
        Some(name) => name,
        None => return Ok(()), // No schema registered, skip validation
    };

    // Get struct metadata
    let struct_meta = match get_struct_metadata(&struct_name) {
        Some(meta) => meta,
        None => return Ok(()), // No metadata, skip validation
    };

    let obj = match item.as_object() {
        Some(o) => o,
        None => {
            return Err(Rfc7807Error::bad_request("Expected JSON object")
                .with_instance(path)
                .to_json())
        }
    };

    // Validate each field against its type and decorators
    for field_meta in &struct_meta.fields {
        if let Some(value) = obj.get(&field_meta.name) {
            // Check if field type is an enum
            if let Some(variants) = get_enum_variants(&field_meta.field_type) {
                // Validate enum value - case-insensitive matching
                if let Some(str_val) = value.as_str() {
                    let str_val_lower = str_val.to_lowercase();
                    if !variants.iter().any(|v| v.to_lowercase() == str_val_lower) {
                        let mut fields = std::collections::HashMap::new();
                        fields.insert(
                            field_meta.name.clone(),
                            doo_ffi_core::FieldError::new(
                                &field_meta.name,
                                format!("Must be one of: {}", variants.join(", ")),
                            )
                            .with_rule(format!("enum:{}", variants.join("|")))
                            .with_value(str_val),
                        );
                        return Err(Rfc7807Error::validation_error(fields)
                            .with_instance(path)
                            .to_json());
                    }
                }
            }

            // Validate decorators (e.g., @email, @min, @max)
            for decorator in &field_meta.decorators {
                if let Err(e) = validate_decorator(decorator, &field_meta.name, value, path) {
                    return Err(e);
                }
            }
        }
    }

    Ok(())
}

/// Validate a single decorator constraint on a field value.
fn validate_decorator(
    decorator: &str,
    field_name: &str,
    value: &serde_json::Value,
    path: &str,
) -> Result<(), String> {
    if decorator == "email" {
        if let Some(s) = value.as_str() {
            if !s.contains('@') || !s.contains('.') {
                let mut fields = std::collections::HashMap::new();
                fields.insert(
                    field_name.to_string(),
                    doo_ffi_core::FieldError::new(field_name, "Invalid email format")
                        .with_rule("email")
                        .with_value(s),
                );
                return Err(Rfc7807Error::validation_error(fields)
                    .with_instance(path)
                    .to_json());
            }
        }
    } else if decorator.starts_with("min(") && decorator.ends_with(')') {
        let min_str = &decorator[4..decorator.len() - 1];
        if let Ok(min_val) = min_str.parse::<i64>() {
            if let Some(s) = value.as_str() {
                if (s.len() as i64) < min_val {
                    let mut fields = std::collections::HashMap::new();
                    fields.insert(
                        field_name.to_string(),
                        doo_ffi_core::FieldError::new(
                            field_name,
                            format!("Must be at least {} characters", min_val),
                        )
                        .with_rule(format!("min:{}", min_val))
                        .with_value(s),
                    );
                    return Err(Rfc7807Error::validation_error(fields)
                        .with_instance(path)
                        .to_json());
                }
            } else if let Some(n) = value.as_i64() {
                if n < min_val {
                    let mut fields = std::collections::HashMap::new();
                    fields.insert(
                        field_name.to_string(),
                        doo_ffi_core::FieldError::new(
                            field_name,
                            format!("Must be at least {}", min_val),
                        )
                        .with_rule(format!("min:{}", min_val))
                        .with_value(n.to_string()),
                    );
                    return Err(Rfc7807Error::validation_error(fields)
                        .with_instance(path)
                        .to_json());
                }
            }
        }
    } else if decorator.starts_with("max(") && decorator.ends_with(')') {
        let max_str = &decorator[4..decorator.len() - 1];
        if let Ok(max_val) = max_str.parse::<i64>() {
            if let Some(s) = value.as_str() {
                if (s.len() as i64) > max_val {
                    let mut fields = std::collections::HashMap::new();
                    fields.insert(
                        field_name.to_string(),
                        doo_ffi_core::FieldError::new(
                            field_name,
                            format!("Maximum {} characters allowed", max_val),
                        )
                        .with_rule(format!("max:{}", max_val))
                        .with_value(s),
                    );
                    return Err(Rfc7807Error::validation_error(fields)
                        .with_instance(path)
                        .to_json());
                }
            } else if let Some(n) = value.as_i64() {
                if n > max_val {
                    let mut fields = std::collections::HashMap::new();
                    fields.insert(
                        field_name.to_string(),
                        doo_ffi_core::FieldError::new(
                            field_name,
                            format!("Maximum {} allowed", max_val),
                        )
                        .with_rule(format!("max:{}", max_val))
                        .with_value(n.to_string()),
                    );
                    return Err(Rfc7807Error::validation_error(fields)
                        .with_instance(path)
                        .to_json());
                }
            }
        }
    }

    Ok(())
}

// ============================================================================
// AUTH AND CRUD HELPERS
// ============================================================================

/// In-memory user store for auth (fallback when no database connected)
static AUTH_USERS: std::sync::OnceLock<StdMutex<HashMap<String, AuthUser>>> =
    std::sync::OnceLock::new();

/// Counter for generating user IDs (in-memory; production would use DB auto-increment)
static AUTH_USER_ID_COUNTER: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);

/// Store which auth table has been created in the database
static AUTH_DB_TABLE: std::sync::OnceLock<StdMutex<Option<String>>> = std::sync::OnceLock::new();

fn get_auth_users() -> &'static StdMutex<HashMap<String, AuthUser>> {
    AUTH_USERS.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn get_auth_db_table() -> &'static StdMutex<Option<String>> {
    AUTH_DB_TABLE.get_or_init(|| StdMutex::new(None))
}

/// Check if auth is using the database
fn is_auth_db_backed() -> bool {
    if !is_pool_initialized() {
        return false;
    }
    let table = get_auth_db_table().lock().unwrap();
    table.is_some()
}

/// Get the auth table name (e.g., "users")
fn get_auth_table_name() -> Option<String> {
    let table = get_auth_db_table().lock().unwrap();
    table.clone()
}

/// Basic email format validation
/// Returns true if email has valid format: something@something.something
fn is_valid_email(email: &str) -> bool {
    // Basic validation: must have exactly one @, non-empty parts, and at least one . after @
    let parts: Vec<&str> = email.split('@').collect();
    if parts.len() != 2 {
        return false;
    }
    let local = parts[0];
    let domain = parts[1];

    // Local part must be non-empty
    if local.is_empty() {
        return false;
    }

    // Domain must have at least one dot and non-empty parts
    if !domain.contains('.') {
        return false;
    }

    let domain_parts: Vec<&str> = domain.split('.').collect();
    if domain_parts.iter().any(|p| p.is_empty()) {
        return false;
    }

    true
}

/// Generic auth user that stores all fields from the user's struct
#[derive(Clone)]
struct AuthUser {
    id: i64,
    email: String,
    password_hash: String,
    /// Additional fields from the user's struct (stored as JSON)
    extra_fields: serde_json::Value,
}

/// Signup handler - registers a new user
/// Generic: works with any struct that has email and password fields, plus any additional fields
extern "C" fn auth_signup_handler(req: *const DooRequest) -> *mut DooResult {
    ffi_debug!("AUTH", "auth_signup_handler called");

    if req.is_null() {
        ffi_debug!("AUTH", "Error: Invalid request (null)");
        return make_err_http(400, "Invalid request");
    }

    let body = unsafe { c_to_string((*req).body) };
    ffi_debug!("AUTH", "Request body: {}", body);

    // Parse full JSON body - supports any fields the user defines
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&body);
    let json = match parsed {
        Ok(serde_json::Value::Object(obj)) => obj,
        Ok(_) => {
            ffi_debug!("AUTH", "Error: Request body is not a JSON object");
            return make_err_http(400, "Request body must be a JSON object");
        }
        Err(e) => {
            ffi_debug!("AUTH", "Error: Invalid JSON body: {}", e);
            return make_err_http(400, "Invalid JSON body");
        }
    };

    // Extract email (required, case-insensitive field lookup)
    let email = json
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("email"))
        .and_then(|(_, v)| v.as_str())
        .map(|s| s.to_string());

    let email = match email {
        Some(e) => e,
        None => {
            ffi_debug!("AUTH", "Error: Missing 'email' field");
            return make_err_http(400, "Missing 'email' field");
        }
    };

    // Validate email format
    if !is_valid_email(&email) {
        ffi_debug!("AUTH", "Error: Invalid email format: {}", email);
        return make_err_http(
            400,
            "Invalid email format. Email must be in format: name@domain.tld",
        );
    }

    // Extract password (required, case-insensitive field lookup)
    let password = json
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("password"))
        .and_then(|(_, v)| v.as_str());

    let password = match password {
        Some(p) => p,
        None => {
            ffi_debug!("AUTH", "Error: Missing 'password' field");
            return make_err_http(400, "Missing 'password' field");
        }
    };

    // Hash password using bcrypt
    let password_hash = match bcrypt::hash(password, 8) {
        Ok(h) => h,
        Err(_) => return make_err_http(500, "Failed to hash password"),
    };

    // Build extra_fields: all fields except email and password
    let mut extra_fields = serde_json::Map::new();
    for (key, value) in json.iter() {
        if !key.eq_ignore_ascii_case("email") && !key.eq_ignore_ascii_case("password") {
            extra_fields.insert(key.clone(), value.clone());
        }
    }

    // Try database-backed auth first
    if is_auth_db_backed() {
        if let Some(table_name) = get_auth_table_name() {
            ffi_debug!("AUTH", "Using database-backed auth, table: {}", table_name);

            // Check if user already exists
            let check_sql = format!("SELECT id FROM {} WHERE email = $1", table_name);
            match execute_db_query_with_string_param(&check_sql, &email) {
                Ok(json) => {
                    let rows: Vec<serde_json::Value> =
                        serde_json::from_str(&json).unwrap_or_default();
                    if !rows.is_empty() {
                        ffi_debug!("AUTH", "Error: User already exists in DB: {}", email);
                        return make_err_http(409, "User already exists");
                    }
                }
                Err(e) => {
                    ffi_debug!("AUTH", "DB error checking existing user: {}", e);
                }
            }

            // Build dynamic INSERT based on extra_fields from request
            // Required: email, password; Optional: all other fields from the struct
            let mut columns = vec!["email".to_string(), "password".to_string()];
            let mut values: Vec<serde_json::Value> = vec![
                serde_json::json!(email),
                serde_json::json!(password_hash.clone()),
            ];
            let mut placeholders: Vec<String> = vec!["$1".to_string(), "$2".to_string()];
            let mut idx = 3;

            // Add extra fields dynamically
            for (key, value) in extra_fields.iter() {
                let col_name = to_snake_case(key);
                columns.push(col_name);
                placeholders.push(format!("${}", idx));
                values.push(value.clone());
                idx += 1;
            }

            let insert_sql = format!(
                "INSERT INTO {} ({}) VALUES ({}) RETURNING *",
                table_name,
                columns.join(", "),
                placeholders.join(", ")
            );

            ffi_debug!("AUTH", "Inserting user with SQL: {}", insert_sql);
            ffi_debug!("AUTH", "Values: {:?}", values);

            match execute_db_insert(&insert_sql, &values) {
                Ok(json) => {
                    ffi_debug!("AUTH", "Insert result: {}", json);
                    let rows: Vec<serde_json::Value> =
                        serde_json::from_str(&json).unwrap_or_default();
                    if let Some(user_row) = rows.into_iter().next() {
                        let user_id = user_row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);

                        // Generate JWT token with user_id in claims
                        let token = generate_jwt_token(&email, user_id);

                        // Build response with all fields from the database row (except password)
                        let mut response_data = serde_json::json!({
                            "token": token,
                        });
                        if let (Some(obj), Some(row_obj)) =
                            (response_data.as_object_mut(), user_row.as_object())
                        {
                            for (k, v) in row_obj {
                                if k != "password" {
                                    obj.insert(k.clone(), v.clone());
                                }
                            }
                        }

                        let response = serde_json::json!({ "data": response_data }).to_string();
                        ffi_debug!("AUTH", "Signup success (DB): {}", response);
                        return make_ok_json(&response);
                    } else {
                        ffi_debug!("AUTH", "DB insert returned no rows, falling back");
                        // Fall through to in-memory
                    }
                }
                Err(e) => {
                    ffi_debug!("AUTH", "DB insert error: {}", e);
                    // Check for unique constraint violation
                    if e.contains("duplicate key") || e.contains("unique constraint") {
                        return make_err_http(409, "User already exists");
                    }
                    return make_err_http(500, &format!("Database error: {}", e));
                }
            }
        }
    }

    // Fallback to in-memory auth
    ffi_debug!("AUTH", "Using in-memory auth fallback");

    // Check if user already exists (in-memory check)
    {
        let users = get_auth_users().lock().unwrap();
        if users.contains_key(&email) {
            ffi_debug!("AUTH", "Error: User already exists: {}", email);
            return make_err_http(409, "User already exists");
        }
    }

    // Generate user ID (in-memory counter)
    let user_id = AUTH_USER_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    // Store user with all fields
    {
        let mut users = get_auth_users().lock().unwrap();
        users.insert(
            email.clone(),
            AuthUser {
                id: user_id,
                email: email.clone(),
                password_hash,
                extra_fields: serde_json::Value::Object(extra_fields.clone()),
            },
        );
        ffi_debug!(
            "AUTH",
            "User stored in memory: {} (id={}), total users: {}",
            email,
            user_id,
            users.len()
        );
    }

    // Generate JWT token with user_id in claims
    let token = generate_jwt_token(&email, user_id as i64);
    ffi_debug!("AUTH", "JWT token generated for: {}", email);

    // Build response with id, email and any extra fields (but not password)
    let mut response_data = serde_json::json!({
        "token": token,
        "email": email,
        "id": user_id,
    });
    if let Some(obj) = response_data.as_object_mut() {
        for (k, v) in extra_fields {
            obj.insert(k, v);
        }
    }
    let response = serde_json::json!({ "data": response_data }).to_string();
    ffi_debug!("AUTH", "Signup success response: {}", response);
    make_ok_json(&response)
}

/// Login handler - authenticates a user and returns JWT
/// Generic: works with any struct that has email and password fields, plus returns any extra fields
extern "C" fn auth_login_handler(req: *const DooRequest) -> *mut DooResult {
    if req.is_null() {
        return make_err_http(400, "Invalid request");
    }

    let body = unsafe { c_to_string((*req).body) };
    ffi_debug!("AUTH", "Login request body: {}", body);

    // Parse full JSON body - supports any fields the user defines
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&body);
    let json = match parsed {
        Ok(serde_json::Value::Object(obj)) => obj,
        Ok(_) => return make_err_http(400, "Request body must be a JSON object"),
        Err(_) => return make_err_http(400, "Invalid JSON body"),
    };

    // Extract email (required, case-insensitive field lookup)
    let email = json
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("email"))
        .and_then(|(_, v)| v.as_str())
        .map(|s| s.to_string());

    let email = match email {
        Some(e) => e,
        None => return make_err_http(400, "Missing 'email' field"),
    };

    // Extract password (required, case-insensitive field lookup)
    let password = json
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("password"))
        .and_then(|(_, v)| v.as_str());

    let password = match password {
        Some(p) => p,
        None => return make_err_http(400, "Missing 'password' field"),
    };

    // Try database-backed auth first
    if is_auth_db_backed() {
        if let Some(table_name) = get_auth_table_name() {
            ffi_debug!(
                "AUTH",
                "Login: Using database-backed auth, table: {}",
                table_name
            );

            // Query ALL fields from user table dynamically
            let query_sql = format!("SELECT * FROM {} WHERE email = $1", table_name);
            match execute_db_query_with_string_param(&query_sql, &email) {
                Ok(json_result) => {
                    let rows: Vec<serde_json::Value> =
                        serde_json::from_str(&json_result).unwrap_or_default();

                    if let Some(user_row) = rows.into_iter().next() {
                        // Case-insensitive lookup for password field (could be "password" or "Password")
                        let stored_hash = user_row
                            .as_object()
                            .and_then(|obj| {
                                obj.iter()
                                    .find(|(k, _)| k.eq_ignore_ascii_case("password"))
                                    .and_then(|(_, v)| v.as_str())
                            })
                            .unwrap_or("");

                        // Verify password
                        match bcrypt::verify(password, stored_hash) {
                            Ok(true) => {
                                ffi_debug!(
                                    "AUTH",
                                    "Login success (DB): Password verified for: {}",
                                    email
                                );

                                // Extract user_id and generate JWT token
                                let user_id =
                                    user_row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                                let token = generate_jwt_token(&email, user_id);

                                // Build response with all fields from DB (except password)
                                let mut response_data = serde_json::json!({
                                    "token": token,
                                });
                                if let (Some(obj), Some(row_obj)) =
                                    (response_data.as_object_mut(), user_row.as_object())
                                {
                                    for (k, v) in row_obj {
                                        // Exclude password field (case-insensitive)
                                        if !k.eq_ignore_ascii_case("password") {
                                            obj.insert(k.clone(), v.clone());
                                        }
                                    }
                                }

                                let response =
                                    serde_json::json!({ "data": response_data }).to_string();
                                return make_ok_json(&response);
                            }
                            _ => {
                                ffi_debug!(
                                    "AUTH",
                                    "Login failed (DB): Invalid password for: {}",
                                    email
                                );
                                return make_err_http(401, "Invalid email or password");
                            }
                        }
                    } else {
                        ffi_debug!("AUTH", "Login failed (DB): User not found: {}", email);
                        return make_err_http(401, "Invalid email or password");
                    }
                }
                Err(e) => {
                    ffi_debug!("AUTH", "DB error during login: {}", e);
                    return make_err_http(500, &format!("Database error: {}", e));
                }
            }
        }
    }

    // Fallback to in-memory auth
    ffi_debug!("AUTH", "Login: Using in-memory auth fallback");

    // Lookup user
    let user = {
        let users = get_auth_users().lock().unwrap();
        ffi_debug!(
            "AUTH",
            "Login lookup for: {} (total users in store: {})",
            email,
            users.len()
        );
        users.get(&email).cloned()
    };

    let user = match user {
        Some(u) => u,
        None => {
            ffi_debug!("AUTH", "Login failed: User not found: {}", email);
            return make_err_http(401, "Invalid email or password");
        }
    };

    // Verify password
    match bcrypt::verify(password, &user.password_hash) {
        Ok(true) => {
            ffi_debug!("AUTH", "Login success: Password verified for: {}", email);
        }
        _ => {
            ffi_debug!("AUTH", "Login failed: Invalid password for: {}", email);
            return make_err_http(401, "Invalid email or password");
        }
    }

    // Generate JWT token with user_id in claims
    let token = generate_jwt_token(&email, user.id as i64);

    // Build response with id, email, token, and any extra fields stored during signup
    let mut response_data = serde_json::json!({
        "token": token,
        "email": email,
        "id": user.id,
    });
    if let Some(obj) = response_data.as_object_mut() {
        if let serde_json::Value::Object(extras) = &user.extra_fields {
            for (k, v) in extras {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
    let response = serde_json::json!({ "data": response_data }).to_string();
    make_ok_json(&response)
}

/// Generate a JWT token for the given subject and user ID
fn generate_jwt_token(sub: &str, user_id: i64) -> String {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde::{Deserialize, Serialize};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Serialize, Deserialize)]
    struct Claims {
        sub: String,
        user_id: i64,
        exp: usize,
        iat: usize,
    }

    let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "test-secret".to_string());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as usize)
        .unwrap_or(0);

    let claims = Claims {
        sub: sub.to_string(),
        user_id,
        exp: now + 86400, // 24 hours
        iat: now,
    };

    let key = EncodingKey::from_secret(secret.as_bytes());
    encode(&Header::new(Algorithm::HS256), &claims, &key)
        .unwrap_or_else(|_| "invalid-token".to_string())
}

/// Set up authentication routes for a user struct.
/// Creates /signup and /login endpoints that handle user registration and authentication.
/// If database is connected, creates the users table and uses DB-backed auth.
#[no_mangle]
pub extern "C" fn doo_http_auth(
    _server: *const c_void,
    signup_path: *const c_char,
    login_path: *const c_char,
    user_struct_name: *const c_char,
    _db: *const c_void,
) -> *mut DooResult {
    let signup_str = c_to_string(signup_path);
    let login_str = c_to_string(login_path);
    let struct_name = c_to_string(user_struct_name);

    ffi_debug!(
        "HTTP",
        "Auth configured: signup={}, login={}, struct={}",
        signup_str,
        login_str,
        struct_name
    );

    // Table name for users (lowercase plural)
    let table_name = "users";

    // Try to create users table in database if connected
    if is_pool_initialized() {
        ffi_debug!("HTTP", "Database connected, setting up DB-backed auth");

        // Get struct metadata to generate CREATE TABLE
        if let Some(metadata) = get_struct_metadata(&struct_name) {
            let create_sql = generate_create_table_sql(table_name, &metadata);
            ffi_debug!("HTTP", "CREATE TABLE SQL for users:\n{}", create_sql);

            match execute_db_statement(&create_sql) {
                Ok(_) => {
                    ffi_debug!(
                        "HTTP",
                        "Users table '{}' created/verified successfully",
                        table_name
                    );
                    // Register auth as DB-backed
                    let mut auth_table = get_auth_db_table().lock().unwrap();
                    *auth_table = Some(table_name.to_string());
                }
                Err(e) => {
                    ffi_debug!("HTTP", "Warning: Failed to create users table: {}", e);
                    // Fall back to in-memory auth
                }
            }
        } else {
            ffi_debug!(
                "HTTP",
                "Warning: No metadata found for struct '{}', using in-memory auth",
                struct_name
            );
        }
    } else {
        ffi_debug!("HTTP", "No database connection, using in-memory auth");
    }

    // Register auth routes
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();

    // Register signup POST route
    registry.register("POST", &signup_str, auth_signup_handler);

    // Register login POST route
    registry.register("POST", &login_str, auth_login_handler);

    // Store auth configuration for reference
    registry.auth_config = Some(AuthConfig {
        signup_path: signup_str,
        login_path: login_str,
        user_struct: struct_name,
    });

    make_ok_void()
}

// ============================================================================
// CRUD HELPERS
// ============================================================================

/// In-memory store fallback for CRUD resources (used when DB not connected)
static CRUD_STORES: std::sync::OnceLock<StdMutex<HashMap<String, CrudStore>>> =
    std::sync::OnceLock::new();

/// Store which resources have been configured for DB-backed CRUD
static CRUD_DB_TABLES: std::sync::OnceLock<StdMutex<HashMap<String, String>>> =
    std::sync::OnceLock::new();

fn get_crud_stores() -> &'static StdMutex<HashMap<String, CrudStore>> {
    CRUD_STORES.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn get_crud_db_tables() -> &'static StdMutex<HashMap<String, String>> {
    CRUD_DB_TABLES.get_or_init(|| StdMutex::new(HashMap::new()))
}

struct CrudStore {
    items: Vec<serde_json::Value>,
    next_id: i64,
}

impl CrudStore {
    fn new() -> Self {
        Self {
            items: Vec::new(),
            next_id: 1,
        }
    }
}

/// Check if a resource is using database-backed CRUD
fn is_db_backed_crud(resource: &str) -> bool {
    if !is_pool_initialized() {
        return false;
    }
    let tables = get_crud_db_tables().lock().unwrap();
    tables.contains_key(resource)
}

/// Get the struct name for a CRUD resource
fn get_crud_struct_name(resource: &str) -> Option<String> {
    let tables = get_crud_db_tables().lock().unwrap();
    tables.get(resource).cloned()
}

/// Extract resource name from CRUD path.
/// Examples:
///   "/api/posts" -> "posts"
///   "/api/posts/1" -> "posts"
///   "/posts" -> "posts"
///   "/posts/1" -> "posts"
fn extract_crud_resource(path: &str) -> String {
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();

    // For paths like /api/posts or /api/posts/1, find the resource name
    // The resource is the segment before any numeric ID
    // Look for the last non-numeric segment (excluding common prefixes like "api")

    for seg in segments.iter().rev() {
        let s = *seg;
        // Skip numeric IDs
        if s.parse::<i64>().is_ok() {
            continue;
        }
        // Skip common API prefixes and empty segments
        if s == "api" || s == "v1" || s == "v2" || s.is_empty() {
            continue;
        }
        return s.to_string();
    }

    // Fallback: return empty string
    String::new()
}

/// Create CRUD handler that returns all items
fn make_crud_list_handler(resource: String) -> DooHandlerFn {
    // Since we can't capture, we'll use a single handler that looks up resource from path
    crud_list_handler
}

extern "C" fn crud_list_handler(req: *const DooRequest) -> *mut DooResult {
    if req.is_null() {
        return make_err_http(400, "Invalid request");
    }

    let path = unsafe { c_to_string((*req).path) };
    // Extract resource name from path (e.g., "/api/posts" -> "posts")
    let resource = extract_crud_resource(&path);

    // Try database-backed CRUD first
    if is_db_backed_crud(&resource) {
        let sql = format!("SELECT * FROM {}", resource);
        match execute_db_query(&sql) {
            Ok(json) => {
                let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap_or_default();
                let response = serde_json::to_string(&serde_json::json!({ "data": items }))
                    .unwrap_or_else(|_| r#"{"data":[]}"#.to_string());
                return make_ok_json(&response);
            }
            Err(e) => {
                ffi_debug!("CRUD", "DB list error: {}", e);
                return make_err_http(500, &format!("Query failed: {}", e));
            }
        }
    }

    // Fallback to in-memory store
    let stores = get_crud_stores().lock().unwrap();
    let items = match stores.get(&resource) {
        Some(store) => store.items.clone(),
        None => Vec::new(),
    };

    let response = serde_json::to_string(&serde_json::json!({ "data": items }))
        .unwrap_or_else(|_| r#"{"data":[]}"#.to_string());
    make_ok_json(&response)
}

extern "C" fn crud_create_handler(req: *const DooRequest) -> *mut DooResult {
    if req.is_null() {
        return make_err_http(400, "Invalid request");
    }

    let path = unsafe { c_to_string((*req).path) };
    let body = unsafe { c_to_string((*req).body) };

    // Debug: Print received body to help diagnose issues
    ffi_debug!(
        "CRUD",
        "POST {} - body length: {}, body: {:?}",
        path,
        body.len(),
        &body[..body.len().min(200)]
    );

    // Extract resource name from path (e.g., "/api/posts" -> "posts")
    let resource = extract_crud_resource(&path);

    // Parse body JSON
    let item: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            ffi_debug!("CRUD", "JSON parse error: {:?}", e);
            return make_err_http(400, "Invalid JSON body");
        }
    };

    // Validate fields using centralized struct/enum metadata
    if let Err(validation_error) = validate_item_against_schema(&item, &resource, &path) {
        return make_err_http(422, &validation_error);
    }

    // Try database-backed CRUD first
    if is_db_backed_crud(&resource) {
        if let Some(obj) = item.as_object() {
            // Build INSERT statement from struct metadata
            let struct_name = get_crud_struct_name(&resource);
            if let Some(meta) = struct_name.and_then(|n| get_struct_metadata(&n)) {
                let mut columns = Vec::new();
                let mut values = Vec::new();
                let mut placeholders = Vec::new();
                let mut idx = 1;

                for field in &meta.fields {
                    // Skip id field (auto-generated)
                    if field.name.to_lowercase() == "id" {
                        continue;
                    }
                    let col_name = to_snake_case(&field.name);
                    if let Some(val) = obj.get(&field.name).or_else(|| obj.get(&col_name)) {
                        columns.push(col_name);
                        placeholders.push(format!("${}", idx));
                        values.push(val.clone());
                        idx += 1;
                    }
                }

                if !columns.is_empty() {
                    let sql = format!(
                        "INSERT INTO {} ({}) VALUES ({}) RETURNING *",
                        resource,
                        columns.join(", "),
                        placeholders.join(", ")
                    );
                    ffi_debug!("CRUD", "DB INSERT SQL: {}", sql);
                    ffi_debug!("CRUD", "DB INSERT values: {:?}", values);

                    // Execute with parameters
                    match execute_db_insert(&sql, &values) {
                        Ok(json) => {
                            let items: Vec<serde_json::Value> =
                                serde_json::from_str(&json).unwrap_or_default();
                            let created = items.into_iter().next().unwrap_or(serde_json::json!({}));
                            let response =
                                serde_json::to_string(&serde_json::json!({ "data": created }))
                                    .unwrap_or_else(|_| r#"{"data":{}}"#.to_string());
                            return make_ok_json(&response);
                        }
                        Err(e) => {
                            ffi_debug!("CRUD", "DB insert error: {}", e);
                            return make_err_http(500, &format!("Insert failed: {}", e));
                        }
                    }
                }
            }
        }
    }

    // Fallback to in-memory store
    let mut item = item;
    let mut stores = get_crud_stores().lock().unwrap();
    let store = stores
        .entry(resource.clone())
        .or_insert_with(CrudStore::new);

    // Add ID to item
    if let Some(obj) = item.as_object_mut() {
        obj.insert("id".to_string(), serde_json::json!(store.next_id));
    }
    store.next_id += 1;
    store.items.push(item.clone());

    let response = serde_json::to_string(&serde_json::json!({ "data": item }))
        .unwrap_or_else(|_| r#"{"data":{}}"#.to_string());
    make_ok_json(&response)
}

extern "C" fn crud_get_handler(req: *const DooRequest) -> *mut DooResult {
    if req.is_null() {
        return make_err_http(400, "Invalid request");
    }

    let path = unsafe { c_to_string((*req).path) };
    // params is now JSON string
    let params_json = unsafe { (*req).params as *const c_char };
    let params: serde_json::Value = if !params_json.is_null() {
        let s = unsafe { std::ffi::CStr::from_ptr(params_json).to_string_lossy() };
        serde_json::from_str(&s).unwrap_or_default()
    } else {
        serde_json::json!({})
    };

    // Extract resource from path (e.g., "/api/posts/1" -> "posts")
    let resource = extract_crud_resource(&path);

    // Extract ID from params or path
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let id: i64 = params
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            // Get last numeric segment from path
            parts.iter().rev().find_map(|s| s.parse().ok()).unwrap_or(0)
        });

    // Try database-backed CRUD first
    if is_db_backed_crud(&resource) {
        let sql = format!("SELECT * FROM {} WHERE id = $1", resource);
        match execute_db_query_by_id(&sql, id as i32) {
            Ok(json) => {
                let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap_or_default();
                if let Some(item) = items.into_iter().next() {
                    let response = serde_json::to_string(&serde_json::json!({ "data": item }))
                        .unwrap_or_else(|_| r#"{"data":{}}"#.to_string());
                    return make_ok_json(&response);
                } else {
                    return make_err_http(404, "Resource not found");
                }
            }
            Err(e) => {
                ffi_debug!("CRUD", "DB get error: {}", e);
                return make_err_http(500, &format!("Query failed: {}", e));
            }
        }
    }

    // Fallback to in-memory store
    let stores = get_crud_stores().lock().unwrap();
    let item = stores
        .get(&resource)
        .and_then(|store| {
            store
                .items
                .iter()
                .find(|i| i.get("id").and_then(|v| v.as_i64()) == Some(id))
        })
        .cloned();

    match item {
        Some(i) => {
            let response = serde_json::to_string(&serde_json::json!({ "data": i }))
                .unwrap_or_else(|_| r#"{"data":{}}"#.to_string());
            make_ok_json(&response)
        }
        None => make_err_http(404, "Resource not found"),
    }
}

extern "C" fn crud_update_handler(req: *const DooRequest) -> *mut DooResult {
    if req.is_null() {
        return make_err_http(400, "Invalid request");
    }

    let path = unsafe { c_to_string((*req).path) };
    let body = unsafe { c_to_string((*req).body) };
    // params is now JSON string
    let params_json = unsafe { (*req).params as *const c_char };
    let params: serde_json::Value = if !params_json.is_null() {
        let s = unsafe { std::ffi::CStr::from_ptr(params_json).to_string_lossy() };
        serde_json::from_str(&s).unwrap_or_default()
    } else {
        serde_json::json!({})
    };

    // Extract resource from path (e.g., "/api/posts/1" -> "posts")
    let resource = extract_crud_resource(&path);

    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let id: i64 = params
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            // Get last numeric segment from path
            parts.iter().rev().find_map(|s| s.parse().ok()).unwrap_or(0)
        });

    let updates: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return make_err_http(400, "Invalid JSON body"),
    };

    // Try database-backed CRUD first
    if is_db_backed_crud(&resource) {
        if let Some(obj) = updates.as_object() {
            let struct_name = get_crud_struct_name(&resource);
            if let Some(meta) = struct_name.and_then(|n| get_struct_metadata(&n)) {
                let mut set_clauses = Vec::new();
                let mut values: Vec<serde_json::Value> = Vec::new();
                let mut idx = 1;

                for field in &meta.fields {
                    if field.name.to_lowercase() == "id" {
                        continue;
                    }
                    let col_name = to_snake_case(&field.name);
                    if let Some(val) = obj.get(&field.name).or_else(|| obj.get(&col_name)) {
                        set_clauses.push(format!("{} = ${}", col_name, idx));
                        values.push(val.clone());
                        idx += 1;
                    }
                }

                if !set_clauses.is_empty() {
                    // Add id as the last parameter
                    values.push(serde_json::json!(id));
                    let sql = format!(
                        "UPDATE {} SET {} WHERE id = ${} RETURNING *",
                        resource,
                        set_clauses.join(", "),
                        idx
                    );
                    ffi_debug!("CRUD", "DB UPDATE SQL: {}", sql);

                    match execute_db_insert(&sql, &values) {
                        Ok(json) => {
                            let items: Vec<serde_json::Value> =
                                serde_json::from_str(&json).unwrap_or_default();
                            if let Some(updated) = items.into_iter().next() {
                                let response =
                                    serde_json::to_string(&serde_json::json!({ "data": updated }))
                                        .unwrap_or_else(|_| r#"{"data":{}}"#.to_string());
                                return make_ok_json(&response);
                            } else {
                                return make_err_http(404, "Resource not found");
                            }
                        }
                        Err(e) => {
                            ffi_debug!("CRUD", "DB update error: {}", e);
                            return make_err_http(500, &format!("Update failed: {}", e));
                        }
                    }
                }
            }
        }
    }

    // Fallback to in-memory store
    let mut stores = get_crud_stores().lock().unwrap();
    let item = stores.get_mut(&resource).and_then(|store| {
        store
            .items
            .iter_mut()
            .find(|i| i.get("id").and_then(|v| v.as_i64()) == Some(id))
    });

    match item {
        Some(i) => {
            if let (Some(existing), Some(new)) = (i.as_object_mut(), updates.as_object()) {
                for (k, v) in new {
                    existing.insert(k.clone(), v.clone());
                }
            }
            let response = serde_json::to_string(&serde_json::json!({ "data": i }))
                .unwrap_or_else(|_| r#"{"data":{}}"#.to_string());
            make_ok_json(&response)
        }
        None => make_err_http(404, "Resource not found"),
    }
}

extern "C" fn crud_delete_handler(req: *const DooRequest) -> *mut DooResult {
    if req.is_null() {
        return make_err_http(400, "Invalid request");
    }

    let path = unsafe { c_to_string((*req).path) };
    // params is now JSON string
    let params_json = unsafe { (*req).params as *const c_char };
    let params: serde_json::Value = if !params_json.is_null() {
        let s = unsafe { std::ffi::CStr::from_ptr(params_json).to_string_lossy() };
        serde_json::from_str(&s).unwrap_or_default()
    } else {
        serde_json::json!({})
    };

    // Extract resource from path (e.g., "/api/posts/1" -> "posts")
    let resource = extract_crud_resource(&path);

    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let id: i64 = params
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            // Get last numeric segment from path
            parts.iter().rev().find_map(|s| s.parse().ok()).unwrap_or(0)
        });

    // Try database-backed CRUD first
    if is_db_backed_crud(&resource) {
        let sql = format!("DELETE FROM {} WHERE id = $1", resource);
        match execute_db_delete_by_id(&sql, id as i32) {
            Ok(affected) => {
                if affected > 0 {
                    return make_ok_json(r#"{"data":{"deleted":true}}"#);
                } else {
                    return make_err_http(404, "Resource not found");
                }
            }
            Err(e) => {
                ffi_debug!("CRUD", "DB delete error: {}", e);
                return make_err_http(500, &format!("Delete failed: {}", e));
            }
        }
    }

    // Fallback to in-memory store
    let mut stores = get_crud_stores().lock().unwrap();
    let removed = stores
        .get_mut(&resource)
        .map(|store| {
            let before = store.items.len();
            store
                .items
                .retain(|i| i.get("id").and_then(|v| v.as_i64()) != Some(id));
            before != store.items.len()
        })
        .unwrap_or(false);

    if removed {
        make_ok_json(r#"{"data":{"deleted":true}}"#)
    } else {
        make_err_http(404, "Resource not found")
    }
}

/// Set up CRUD routes for a resource struct.
/// Creates GET, POST, PUT, DELETE endpoints for the resource.
/// If database is connected, creates the table and uses DB-backed CRUD.
#[no_mangle]
pub extern "C" fn doo_http_crud(
    _server: *const c_void,
    base_path: *const c_char,
    resource_struct_name: *const c_char,
    _db: *const c_void,
) -> *mut DooResult {
    let base_str = c_to_string(base_path);
    let struct_name = c_to_string(resource_struct_name);

    ffi_debug!(
        "HTTP",
        "CRUD configured: base={}, struct={}",
        base_str,
        struct_name
    );

    // Extract resource name (e.g., "posts" from "/api/posts")
    let resource_key = extract_crud_resource(&base_str);
    ffi_debug!("HTTP", "CRUD resource key: {}", resource_key);

    // Try to create table in database if connected
    if is_pool_initialized() {
        ffi_debug!(
            "HTTP",
            "Database connected, setting up DB-backed CRUD for {}",
            resource_key
        );

        // Get struct metadata to generate CREATE TABLE
        if let Some(metadata) = get_struct_metadata(&struct_name) {
            let create_sql = generate_create_table_sql(&resource_key, &metadata);
            ffi_debug!("HTTP", "CREATE TABLE SQL:\n{}", create_sql);

            match execute_db_statement(&create_sql) {
                Ok(_) => {
                    ffi_debug!(
                        "HTTP",
                        "Table '{}' created/verified successfully",
                        resource_key
                    );
                    // Register this resource as DB-backed
                    let mut tables = get_crud_db_tables().lock().unwrap();
                    tables.insert(resource_key.clone(), struct_name.clone());
                }
                Err(e) => {
                    ffi_debug!(
                        "HTTP",
                        "Warning: Failed to create table '{}': {}",
                        resource_key,
                        e
                    );
                    // Fall back to in-memory store
                }
            }
        } else {
            ffi_debug!(
                "HTTP",
                "Warning: No metadata found for struct '{}', using in-memory store",
                struct_name
            );
        }
    } else {
        ffi_debug!("HTTP", "No database connection, using in-memory CRUD store");
    }

    // Initialize in-memory store as fallback
    {
        let mut stores = get_crud_stores().lock().unwrap();
        stores
            .entry(resource_key.clone())
            .or_insert_with(CrudStore::new);
    }

    // Register CRUD routes
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();

    // GET /resource - list all
    registry.register("GET", &base_str, crud_list_handler);

    // POST /resource - create
    registry.register("POST", &base_str, crud_create_handler);

    // GET /resource/{id} - get one (matchit uses {param} syntax, not :param)
    let get_one_path = format!("{}/{{id}}", base_str);
    registry.register("GET", &get_one_path, crud_get_handler);

    // PUT /resource/{id} - update
    registry.register("PUT", &get_one_path, crud_update_handler);

    // DELETE /resource/{id} - delete
    registry.register("DELETE", &get_one_path, crud_delete_handler);

    // Store configuration for reference
    registry.crud_configs.push(CrudConfig {
        base_path: base_str,
        resource_struct: struct_name,
    });

    make_ok_void()
}

// ============================================================================
// MIDDLEWARE
// ============================================================================

/// Register a middleware function pointer for global use
/// The middleware parameter is a function pointer (DooMiddlewareFn) despite the type signature
/// The compiler passes the wrapper function pointer directly
#[no_mangle]
pub extern "C" fn doo_http_use(
    server: *const c_void,
    middleware_fn: DooMiddlewareFn,
) -> *const c_void {
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();

    // Add the middleware function directly to global middleware
    registry.add_middleware(middleware_fn);

    server
}

/// Register a user-defined middleware function by name
/// Called by the compiler to register middleware before route registration
#[no_mangle]
pub extern "C" fn doo_http_register_middleware(
    name: *const c_char,
    middleware_fn: DooMiddlewareFn,
) {
    let name_str = c_to_string(name);
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.middleware_handlers.insert(name_str, middleware_fn);
}

#[no_mangle]
pub extern "C" fn doo_http_jwt() -> *const c_char {
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    if !registry.middleware_handlers.contains_key(MIDDLEWARE_JWT) {
        registry
            .middleware_handlers
            .insert(MIDDLEWARE_JWT.to_string(), jwt_middleware_handler);
    }
    string_to_c(MIDDLEWARE_JWT)
}

#[no_mangle]
pub extern "C" fn doo_http_cors(server: *mut c_void) -> *mut c_void {
    let config = CorsConfig::default();
    *get_cors_config().lock().unwrap() = Some(config);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    if !registry.middleware_handlers.contains_key(MIDDLEWARE_CORS) {
        registry
            .middleware_handlers
            .insert(MIDDLEWARE_CORS.to_string(), cors_middleware_handler);
    }
    registry.add_middleware(cors_middleware_handler);
    server
}

#[no_mangle]
pub extern "C" fn doo_http_cors_custom(server: *mut c_void, options: *mut c_void) -> *mut c_void {
    // Parse options map for CORS configuration
    let config = if options.is_null() {
        CorsConfig::default()
    } else {
        // Parse origins - comma-separated string
        let origins_ptr = doo_map_get_str(options, "origins");
        let origins = if origins_ptr.is_null() {
            vec!["*".to_string()]
        } else {
            parse_json_string_or_default(origins_ptr, "*")
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        };

        // Parse methods - comma-separated string
        let methods_ptr = doo_map_get_str(options, "methods");
        let methods = if methods_ptr.is_null() {
            vec!["GET", "POST", "PUT", "DELETE", "OPTIONS", "PATCH"]
                .into_iter()
                .map(String::from)
                .collect()
        } else {
            parse_json_string_or_default(methods_ptr, "GET,POST,PUT,DELETE,OPTIONS,PATCH")
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        };

        // Parse headers - comma-separated string
        let headers_ptr = doo_map_get_str(options, "headers");
        let headers = if headers_ptr.is_null() {
            vec!["Content-Type", "Authorization"]
                .into_iter()
                .map(String::from)
                .collect()
        } else {
            parse_json_string_or_default(headers_ptr, "Content-Type,Authorization")
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        };

        // Parse credentials - boolean
        let credentials_ptr = doo_map_get_str(options, "credentials");
        let credentials = parse_json_bool_or_default(credentials_ptr, false);

        // Parse max_age - integer (seconds)
        let max_age_ptr = doo_map_get_str(options, "max_age");
        let max_age = if max_age_ptr.is_null() {
            None
        } else {
            let val = parse_json_i64_or_default(max_age_ptr, 0);
            if val > 0 {
                Some(val as i32)
            } else {
                None
            }
        };

        CorsConfig {
            origins,
            methods,
            headers,
            credentials,
            max_age,
        }
    };

    *get_cors_config().lock().unwrap() = Some(config);

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    if !registry.middleware_handlers.contains_key(MIDDLEWARE_CORS) {
        registry
            .middleware_handlers
            .insert(MIDDLEWARE_CORS.to_string(), cors_middleware_handler);
    }
    registry.add_middleware(cors_middleware_handler);
    server
}

#[no_mangle]
pub extern "C" fn doo_http_ratelimit(server: *mut c_void) -> *mut c_void {
    let config = RateLimitConfig::default();
    *get_ratelimit_config().lock().unwrap() = Some(config);

    // Clear state for fresh start
    get_ratelimit_state().lock().unwrap().clear();

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    if !registry
        .middleware_handlers
        .contains_key(MIDDLEWARE_RATELIMIT)
    {
        registry.middleware_handlers.insert(
            MIDDLEWARE_RATELIMIT.to_string(),
            ratelimit_middleware_handler,
        );
    }
    registry.add_middleware(ratelimit_middleware_handler);
    server
}

#[no_mangle]
pub extern "C" fn doo_http_ratelimit_custom(
    server: *mut c_void,
    options: *mut c_void,
) -> *mut c_void {
    // Parse options map for rate limit configuration
    let config = if options.is_null() {
        RateLimitConfig::default()
    } else {
        let max = parse_json_i64_or_default(doo_map_get_str(options, "max"), 100);
        let window = parse_json_i64_or_default(doo_map_get_str(options, "window"), 60);
        let per_str = parse_json_string_or_default(doo_map_get_str(options, "per"), "ip");

        RateLimitConfig {
            max: if max > 0 { max as u32 } else { 100 },
            window: if window > 0 { window as u64 } else { 60 },
            per: per_str,
        }
    };

    *get_ratelimit_config().lock().unwrap() = Some(config);

    // Clear state for fresh start
    get_ratelimit_state().lock().unwrap().clear();

    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    if !registry
        .middleware_handlers
        .contains_key(MIDDLEWARE_RATELIMIT)
    {
        registry.middleware_handlers.insert(
            MIDDLEWARE_RATELIMIT.to_string(),
            ratelimit_middleware_handler,
        );
    }
    registry.add_middleware(ratelimit_middleware_handler);
    server
}

#[no_mangle]
pub extern "C" fn doo_http_group(
    _server: *const c_void,
    _prefix: *const c_char,
    _handler: extern "C" fn(),
) -> *mut DooResult {
    // Groups handled at compile-time, no-op at runtime
    make_ok_void()
}

// ============================================================================
// REQUEST HELPERS
// ============================================================================

#[no_mangle]
pub extern "C" fn doo_http_req_query(req: *const DooRequest, key: *const c_char) -> *const c_char {
    if req.is_null() {
        return std::ptr::null();
    }
    unsafe {
        let query_map = (*req).query as *const HashMap<String, String>;
        if query_map.is_null() {
            return std::ptr::null();
        }
        let key_str = c_to_string(key);
        (*query_map)
            .get(&key_str)
            .map(|v| string_to_c(v))
            .unwrap_or(std::ptr::null())
    }
}

#[no_mangle]
pub extern "C" fn doo_http_req_param(req: *const DooRequest, key: *const c_char) -> *const c_char {
    if req.is_null() {
        return std::ptr::null();
    }
    unsafe {
        // params is now stored as a JSON string pointer, not HashMap
        let params_json = (*req).params as *const c_char;
        if params_json.is_null() {
            return std::ptr::null();
        }
        let key_str = c_to_string(key);
        let json_str = match std::ffi::CStr::from_ptr(params_json).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null(),
        };
        // Parse JSON and extract field
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) {
            if let Some(v) = value.get(&key_str) {
                if let Some(s) = v.as_str() {
                    return string_to_c(s);
                }
            }
        }
        std::ptr::null()
    }
}

#[no_mangle]
pub extern "C" fn doo_http_req_header(req: *const DooRequest, key: *const c_char) -> *const c_char {
    if req.is_null() {
        return std::ptr::null();
    }
    unsafe {
        let headers_map = (*req).headers as *const HashMap<String, String>;
        if headers_map.is_null() {
            return std::ptr::null();
        }
        let key_str = c_to_string(key).to_lowercase();
        (*headers_map)
            .get(&key_str)
            .map(|v| string_to_c(v))
            .unwrap_or(string_to_c(""))
    }
}

#[no_mangle]
pub extern "C" fn doohttp_extract_param_int(
    req: *const DooRequest,
    param_name: *const c_char,
) -> i64 {
    clear_last_error();
    if req.is_null() || param_name.is_null() {
        return 0;
    }
    let value_ptr = doo_http_req_param(req, param_name);
    if value_ptr.is_null() {
        return 0;
    }
    let value_str = c_to_string(value_ptr);
    match value_str.parse::<i64>() {
        Ok(v) => v,
        Err(_) => {
            let param_name_str = c_to_string(param_name);
            let path = get_current_request_path();
            let param = ParameterError::new(&param_name_str)
                .with_expected("Int")
                .with_received(&value_str);
            let err = Rfc7807Error::invalid_path_param(&path, param);
            set_last_error(400, err.to_json());
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn doohttp_extract_param_float(
    req: *const DooRequest,
    param_name: *const c_char,
) -> f64 {
    clear_last_error();
    if req.is_null() || param_name.is_null() {
        return 0.0;
    }
    let value_ptr = doo_http_req_param(req, param_name);
    if value_ptr.is_null() {
        return 0.0;
    }
    let value_str = c_to_string(value_ptr);
    match value_str.parse::<f64>() {
        Ok(v) => v,
        Err(_) => {
            let param_name_str = c_to_string(param_name);
            let path = get_current_request_path();
            let param = ParameterError::new(&param_name_str)
                .with_expected("Float")
                .with_received(&value_str);
            let err = Rfc7807Error::invalid_path_param(&path, param);
            set_last_error(400, err.to_json());
            0.0
        }
    }
}

/// Extract typed path parameter from request with type validation
/// Returns: converted value as C string (caller must free), or null on error
#[no_mangle]
pub extern "C" fn doohttp_extract_param_typed(
    req: *const DooRequest,
    param_name: *const c_char,
    param_type: *const c_char,
) -> *const c_char {
    clear_last_error();
    if req.is_null() || param_name.is_null() || param_type.is_null() {
        return std::ptr::null();
    }

    let param_name_str = c_to_string(param_name);
    let param_type_str = c_to_string(param_type);
    let value_ptr = doo_http_req_param(req, param_name);

    if value_ptr.is_null() {
        let path = get_current_request_path();
        let param = ParameterError::new(&param_name_str)
            .with_message(format!("Path parameter '{}' is required", param_name_str));
        let err = Rfc7807Error::missing_path_param(&path, param);
        set_last_error(400, err.to_json());
        return std::ptr::null();
    }

    let value = c_to_string(value_ptr);

    // Type conversion validation
    match param_type_str.as_str() {
        "Int" => {
            if value.parse::<i64>().is_ok() {
                string_to_c(&value)
            } else {
                let path = get_current_request_path();
                let param = ParameterError::new(&param_name_str)
                    .with_expected("Int")
                    .with_received(&value);
                let err = Rfc7807Error::invalid_path_param(&path, param);
                set_last_error(400, err.to_json());
                std::ptr::null()
            }
        }
        "Float" => {
            if value.parse::<f64>().is_ok() {
                string_to_c(&value)
            } else {
                let path = get_current_request_path();
                let param = ParameterError::new(&param_name_str)
                    .with_expected("Float")
                    .with_received(&value);
                let err = Rfc7807Error::invalid_path_param(&path, param);
                set_last_error(400, err.to_json());
                std::ptr::null()
            }
        }
        "Bool" => {
            if value == "true" || value == "false" {
                string_to_c(&value)
            } else {
                let path = get_current_request_path();
                let param = ParameterError::new(&param_name_str)
                    .with_expected("Bool")
                    .with_received(&value);
                let err = Rfc7807Error::invalid_path_param(&path, param);
                set_last_error(400, err.to_json());
                std::ptr::null()
            }
        }
        _ => string_to_c(&value), // String or other types - return as-is
    }
}

// ============================================================================
// RFC 7807 ERROR FUNCTIONS
// ============================================================================

// ============================================================================
// MAP BUILDER FUNCTIONS
// Used by codegen to convert object literals `{ key: value, ... }` into
// HashMap<String, String> compatible with doo_map_get_str and FFI config parsing.
// This is the single source of truth for building maps from Doo object literals.
// ============================================================================

/// Create a new empty map (HashMap<String, String>)
#[no_mangle]
pub extern "C" fn doo_map_new() -> *mut c_void {
    let map: HashMap<String, String> = HashMap::new();
    let boxed = Box::new(map);
    Box::into_raw(boxed) as *mut c_void
}

/// Set a string key-value pair in the map
#[no_mangle]
pub extern "C" fn doo_map_set(map: *mut c_void, key: *const c_char, value: *const c_char) {
    if map.is_null() || key.is_null() {
        return;
    }
    unsafe {
        let map = &mut *(map as *mut HashMap<String, String>);
        let k = CStr::from_ptr(key).to_string_lossy().to_string();
        let v = if value.is_null() {
            String::new()
        } else {
            CStr::from_ptr(value).to_string_lossy().to_string()
        };
        map.insert(k, v);
    }
}

/// Set a string array value as comma-separated string in the map.
/// arr_data points to the Doo array data area (header is 16 bytes before).
/// Array layout: [i64 len, i64 cap, ptr elem0, ptr elem1, ...]
#[no_mangle]
pub extern "C" fn doo_map_set_str_array(
    map: *mut c_void,
    key: *const c_char,
    arr_data: *const c_void,
) {
    if map.is_null() || key.is_null() || arr_data.is_null() {
        return;
    }
    unsafe {
        let map = &mut *(map as *mut HashMap<String, String>);
        let k = CStr::from_ptr(key).to_string_lossy().to_string();

        // Array data pointer is at offset +16 from header
        // Header: [len: i64 at -16, cap: i64 at -8]
        let len_ptr = (arr_data as *const u8).sub(16) as *const i64;
        let len = *len_ptr as usize;

        let data_ptr = arr_data as *const *const c_char;
        let mut values = Vec::new();
        for i in 0..len {
            let elem = *data_ptr.add(i);
            if !elem.is_null() {
                values.push(CStr::from_ptr(elem).to_string_lossy().to_string());
            }
        }
        map.insert(k, values.join(","));
    }
}

// Helper for map value parsing (used by populate_struct)
fn doo_map_get_str(map_ptr: *const c_void, key: &str) -> *const c_char {
    if map_ptr.is_null() {
        return std::ptr::null();
    }
    unsafe {
        let map = &*(map_ptr as *const HashMap<String, String>);
        match map.get(key) {
            Some(v) => string_to_c(v),
            None => std::ptr::null(),
        }
    }
}

fn parse_json_i64_or_default(ptr: *const c_char, default: i64) -> i64 {
    if ptr.is_null() {
        return default;
    }
    let s = c_to_string(ptr);
    // Handle JSON-encoded numbers (might be quoted or unquoted)
    let trimmed = s.trim().trim_matches('"');
    trimmed.parse::<i64>().unwrap_or(default)
}

fn parse_json_string_or_default(ptr: *const c_char, default: &str) -> String {
    if ptr.is_null() {
        return default.to_string();
    }
    let s = c_to_string(ptr);
    // Remove JSON quotes if present
    let trimmed = s.trim().trim_matches('"');
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    }
}

fn parse_json_bool_or_default(ptr: *const c_char, default: bool) -> bool {
    if ptr.is_null() {
        return default;
    }
    let s = c_to_string(ptr).trim().to_lowercase();
    match s.as_str() {
        "true" | "\"true\"" | "1" => true,
        "false" | "\"false\"" | "0" => false,
        _ => default,
    }
}

/// Populate struct from request data with JSON parsing and validation
///
/// Parameters:
/// - request_ptr: Pointer to DooRequest
/// - struct_ptr: Pointer to allocated struct to populate
/// - source_type: 0=body (JSON), 1=params, 2=query
/// - handler_name: Name of handler (used to get metadata)
///
/// Returns: 0 on success, error code on failure
#[no_mangle]
pub extern "C" fn doohttp_populate_struct_from_request(
    request_ptr: *const c_void,
    struct_ptr: *mut c_void,
    source_type: i32,
    handler_name: *const c_char,
) -> i32 {
    clear_last_error();

    // Request pointer is required, but struct_ptr can be null for validation-only mode
    if request_ptr.is_null() {
        return -1;
    }

    if handler_name.is_null() {
        return 0; // No handler name, can't look up metadata
    }

    // Track if we're in validation-only mode (struct_ptr is null)
    let _validation_only = struct_ptr.is_null();

    let handler_name_str = c_to_string(handler_name);

    // Cast request to get fields
    let request = unsafe { &*(request_ptr as *const DooRequest) };
    let path_str = c_to_string(request.path);
    set_current_request_path(&path_str);

    // Get handler metadata from registry
    let routes = get_routes();
    let registry = routes.lock().unwrap();
    let metadata = registry.handler_metadata.get(&handler_name_str).cloned();
    drop(registry);

    let metadata = match metadata {
        Some(m) => m,
        None => return 0, // No metadata, skip validation
    };

    // Helper to coerce string values to typed JSON based on struct layout from metadata
    // Searches ALL struct layouts to find the field type (for multi-param handlers)
    fn coerce_string_to_typed_value(
        key: &str,
        value: &str,
        metadata: &HandlerMetadata,
    ) -> serde_json::Value {
        // Search ALL struct layouts to find the expected type for this field
        for (_struct_name, layout) in &metadata.struct_layouts {
            if let Some(fields) = layout.get("fields").and_then(|f| f.as_array()) {
                for field in fields {
                    if let Some(obj) = field.as_object() {
                        let field_name = obj.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let field_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("Str");
                        if field_name == key {
                            return match field_type {
                                "Int" => value
                                    .parse::<i64>()
                                    .map(serde_json::Value::from)
                                    .unwrap_or_else(|_| {
                                        serde_json::Value::String(value.to_string())
                                    }),
                                "Float" => value
                                    .parse::<f64>()
                                    .map(|f| serde_json::Value::from(f))
                                    .unwrap_or_else(|_| {
                                        serde_json::Value::String(value.to_string())
                                    }),
                                "Bool" => {
                                    let lower = value.to_lowercase();
                                    match lower.as_str() {
                                        "true" | "1" => serde_json::Value::Bool(true),
                                        "false" | "0" => serde_json::Value::Bool(false),
                                        _ => serde_json::Value::String(value.to_string()),
                                    }
                                }
                                _ => serde_json::Value::String(value.to_string()),
                            };
                        }
                    }
                }
            }
        }
        // Default: keep as string
        serde_json::Value::String(value.to_string())
    }

    // Build source_data by merging ALL sources for multi-param handler support
    // Priority: path params > query params > body (later sources don't override earlier)
    let mut source_data: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();

    // 1. Start with path params (highest priority for path-based fields like userId)
    // NOTE: params is stored as a JSON string (e.g., '{"authorId":"1"}') by the server,
    // NOT as a HashMap pointer. This is consistent with how codegen reads it via doo_json_get_field.
    if !request.params.is_null() {
        let params_json_ptr = request.params as *const c_char;
        let params_json_str = c_to_string(params_json_ptr);
        if !params_json_str.is_empty() {
            if let Ok(serde_json::Value::Object(params_obj)) =
                serde_json::from_str::<serde_json::Value>(&params_json_str)
            {
                for (k, v) in params_obj {
                    // Path params values are strings in JSON, coerce them to typed values
                    let value_str = v.as_str().unwrap_or_default();
                    source_data.insert(
                        k.clone(),
                        coerce_string_to_typed_value(&k, value_str, &metadata),
                    );
                }
            }
        }
    }

    // 2. Add query params (for GET requests with query strings)
    if !request.query.is_null() {
        let query_map = unsafe { &*(request.query as *const HashMap<String, String>) };
        for (k, v) in query_map.iter() {
            // Don't override path params
            if !source_data.contains_key(k) {
                source_data.insert(k.clone(), coerce_string_to_typed_value(k, v, &metadata));
            }
        }
    }

    // 3. Merge body JSON (for POST/PUT/PATCH with JSON body)
    if !request.body.is_null() {
        let body_str = c_to_string(request.body);
        if !body_str.is_empty() {
            if let Ok(serde_json::Value::Object(body_obj)) =
                serde_json::from_str::<serde_json::Value>(&body_str)
            {
                for (k, v) in body_obj {
                    // Don't override path params or query params
                    if !source_data.contains_key(&k) {
                        source_data.insert(k, v);
                    }
                }
            }
        }
    }

    // If no data at all and it's a GET/DELETE, that's fine (might have no params)
    // But for POST/PUT/PATCH, empty data when params expected is an error
    // This will be caught by validation below

    // Check if first param type is a special raw request type - skip validation
    let first_param = metadata.param_types.first().cloned().unwrap_or_default();
    if first_param == "Request" || first_param == "DooRequest" {
        return 0;
    }

    // Skip validation if no param types defined
    if metadata.param_types.is_empty() {
        return 0;
    }

    // Recursive validation helper
    fn validate_struct_fields(
        source_data: &serde_json::Map<String, serde_json::Value>,
        struct_name: &str,
        field_prefix: &str,
        metadata: &HandlerMetadata,
        field_errors: &mut HashMap<String, FieldError>,
    ) {
        let struct_layout = match metadata.struct_layouts.get(struct_name) {
            Some(layout) => layout,
            None => return, // Unknown struct, skip validation
        };

        let fields = match struct_layout.get("fields").and_then(|f| f.as_array()) {
            Some(f) => f,
            None => return,
        };

        for field in fields {
            let field_obj = match field.as_object() {
                Some(obj) => obj,
                None => continue,
            };

            let field_name = match field_obj.get("name").and_then(|v| v.as_str()) {
                Some(n) => n,
                None => continue,
            };

            let field_type = match field_obj.get("type").and_then(|v| v.as_str()) {
                Some(t) => t,
                None => continue,
            };

            // Build full field path for error messages
            let full_field_name = if field_prefix.is_empty() {
                field_name.to_string()
            } else {
                format!("{}.{}", field_prefix, field_name)
            };

            // Check if field is missing - Optional fields are allowed to be missing
            if !source_data.contains_key(field_name)
                && !(field_type.starts_with("Optional(") && field_type.ends_with(')'))
            {
                let err = FieldError::required();
                field_errors.insert(full_field_name, err);
                continue;
            }

            if let Some(value) = source_data.get(field_name) {
                // Validate array element types
                if field_type.starts_with("[") && field_type.ends_with("]") {
                    let elem_type = &field_type[1..field_type.len() - 1];

                    if let serde_json::Value::Array(arr) = value {
                        for (i, elem) in arr.iter().enumerate() {
                            let elem_valid = match elem_type {
                                "Int" => elem.is_i64() || elem.is_u64(),
                                "Float" => elem.is_f64(),
                                "Bool" => elem.is_boolean(),
                                "Str" | "String" => elem.is_string(),
                                _ => true, // Nested struct arrays - skip for now
                            };

                            if !elem_valid {
                                let received_type = match elem {
                                    serde_json::Value::Null => "null",
                                    serde_json::Value::Bool(_) => "Bool",
                                    serde_json::Value::Number(n) => {
                                        if n.is_i64() || n.is_u64() {
                                            "Int"
                                        } else {
                                            "Float"
                                        }
                                    }
                                    serde_json::Value::String(_) => "Str",
                                    serde_json::Value::Array(_) => "Array",
                                    serde_json::Value::Object(_) => "Object",
                                };

                                let elem_value_str = match elem {
                                    serde_json::Value::String(s) => s.clone(),
                                    serde_json::Value::Null => "null".to_string(),
                                    serde_json::Value::Bool(b) => b.to_string(),
                                    serde_json::Value::Number(n) => n.to_string(),
                                    _ => elem.to_string(),
                                };
                                let err = FieldError::type_mismatch(elem_type, received_type)
                                    .with_value(elem_value_str);
                                field_errors.entry(full_field_name.clone()).or_insert(err);
                            }
                        }
                    } else {
                        let received_type = match value {
                            serde_json::Value::Null => "null",
                            serde_json::Value::Bool(_) => "Bool",
                            serde_json::Value::Number(_) => "Number",
                            serde_json::Value::String(_) => "Str",
                            serde_json::Value::Object(_) => "Object",
                            serde_json::Value::Array(_) => "Array",
                        };
                        let value_str = match value {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Null => "null".to_string(),
                            serde_json::Value::Bool(b) => b.to_string(),
                            serde_json::Value::Number(n) => n.to_string(),
                            _ => value.to_string(),
                        };
                        let err = FieldError::type_mismatch(field_type, received_type)
                            .with_value(value_str);
                        field_errors.insert(full_field_name, err);
                    }
                } else {
                    // Check primitive types first
                    let is_primitive =
                        matches!(field_type, "Int" | "Float" | "Bool" | "Str" | "String");

                    if is_primitive {
                        let type_valid = match field_type {
                            "Int" => value.is_i64() || value.is_u64(),
                            "Float" => value.is_f64(),
                            "Bool" => value.is_boolean(),
                            "Str" | "String" => value.is_string(),
                            _ => true,
                        };

                        if !type_valid {
                            let received_type = match value {
                                serde_json::Value::Null => "null",
                                serde_json::Value::Bool(_) => "Bool",
                                serde_json::Value::Number(n) => {
                                    if n.is_i64() || n.is_u64() {
                                        "Int"
                                    } else {
                                        "Float"
                                    }
                                }
                                serde_json::Value::String(_) => "String",
                                serde_json::Value::Array(_) => "Array",
                                serde_json::Value::Object(_) => "Object",
                            };
                            // Get the raw value string for the response
                            let value_str = match value {
                                serde_json::Value::String(s) => s.clone(),
                                serde_json::Value::Null => "null".to_string(),
                                serde_json::Value::Bool(b) => b.to_string(),
                                serde_json::Value::Number(n) => n.to_string(),
                                _ => value.to_string(),
                            };
                            let err = FieldError::type_mismatch(field_type, received_type)
                                .with_value(value_str);
                            field_errors.insert(full_field_name, err);
                        }
                    } else if let Some(variants) = metadata.enum_variants.get(field_type) {
                        // Enum validation - case-insensitive matching
                        let valid = if let Some(s) = value.as_str() {
                            let s_lower = s.to_lowercase();
                            variants.iter().any(|v| v.to_lowercase() == s_lower)
                        } else {
                            false
                        };

                        if !valid {
                            let received_str = match value {
                                serde_json::Value::String(s) => s.clone(),
                                serde_json::Value::Null => "null".to_string(),
                                serde_json::Value::Bool(b) => b.to_string(),
                                serde_json::Value::Number(n) => n.to_string(),
                                serde_json::Value::Array(_) => "Array".to_string(),
                                serde_json::Value::Object(_) => "Object".to_string(),
                            };
                            let err =
                                FieldError::new(format!("Must be one of: {}", variants.join(", ")))
                                    .with_rule(format!("enum:{}", variants.join("|")))
                                    .with_value(received_str);
                            field_errors.insert(full_field_name, err);
                        }
                    } else if metadata.struct_layouts.contains_key(field_type) {
                        // Nested struct validation - recurse
                        if let serde_json::Value::Object(nested_obj) = value {
                            validate_struct_fields(
                                nested_obj,
                                field_type,
                                &full_field_name,
                                metadata,
                                field_errors,
                            );
                        } else {
                            let received_type = match value {
                                serde_json::Value::Null => "null",
                                serde_json::Value::Bool(_) => "Bool",
                                serde_json::Value::Number(_) => "Number",
                                serde_json::Value::String(_) => "Str",
                                serde_json::Value::Array(_) => "Array",
                                serde_json::Value::Object(_) => "Object",
                            };
                            let value_str = match value {
                                serde_json::Value::String(s) => s.clone(),
                                serde_json::Value::Null => "null".to_string(),
                                serde_json::Value::Bool(b) => b.to_string(),
                                serde_json::Value::Number(n) => n.to_string(),
                                _ => value.to_string(),
                            };
                            let err = FieldError::type_mismatch(field_type, received_type)
                                .with_value(value_str);
                            field_errors.insert(full_field_name, err);
                        }
                    }
                    // Unknown types are skipped
                }
            }
        }
    }

    // Validate fields for ALL param types (multi-param handler support)
    let mut field_errors: HashMap<String, FieldError> = HashMap::new();
    for param_type in &metadata.param_types {
        // Skip special/injected types — not sourced from request data
        if param_type == "Request" || param_type == "DooRequest" || param_type == "Server" {
            continue;
        }
        validate_struct_fields(&source_data, param_type, "", &metadata, &mut field_errors);
    }

    if !field_errors.is_empty() {
        // Convert to core FieldErrors in a HashMap preserving field names
        let core_fields: HashMap<String, doo_ffi_core::FieldError> = field_errors
            .into_iter()
            .map(|(field, err)| (field.clone(), err.to_core(&field)))
            .collect();

        // Determine detail based on error types
        let has_required = core_fields
            .values()
            .any(|e| e.error.as_deref() == Some("required"));
        let has_type_mismatch = core_fields
            .values()
            .any(|e| e.expected.is_some() && e.received.is_some() && e.rule.is_none());

        let (status, detail) = if has_required && !has_type_mismatch {
            (400u16, "Required field missing in request body")
        } else if has_type_mismatch && !has_required {
            (400, "Type mismatch in request body")
        } else {
            (400, "Request body parsing failed")
        };

        let err = doo_ffi_core::Rfc7807Error::new(status, detail)
            .with_instance(path_str)
            .with_fields(core_fields);
        set_last_error(status as i32, err.to_json());
        return status as i32;
    }

    // Always update request.body with the merged/typed JSON
    // so that codegen's body parsing works correctly for all sources
    if !source_data.is_empty() {
        let json_obj = serde_json::Value::Object(source_data);
        if let Ok(json_str) = serde_json::to_string(&json_obj) {
            let request_mut = unsafe { &mut *(request_ptr as *mut DooRequest) };
            request_mut.body = string_to_c(&json_str);
        }
    }

    0 // Success
}

#[no_mangle]
pub extern "C" fn doohttp_error_rfc7807(
    status: i32,
    detail: *const c_char,
    instance: *const c_char,
) -> *const c_char {
    let detail_str = c_to_string(detail);
    let instance_str = if instance.is_null() {
        get_current_request_path()
    } else {
        c_to_string(instance)
    };

    let err = match status {
        400 => bad_request(detail_str, instance_str),
        401 => unauthorized(detail_str, instance_str),
        403 => forbidden(detail_str, instance_str),
        404 => not_found(detail_str, instance_str),
        409 => conflict(detail_str, instance_str),
        429 => too_many_requests(detail_str, instance_str),
        _ => internal_error(detail_str, instance_str),
    };
    string_to_c(&err.to_json())
}

#[no_mangle]
pub extern "C" fn doohttp_error_rfc7807_auto_instance(
    status: i32,
    detail: *const c_char,
) -> *const c_char {
    doohttp_error_rfc7807(status, detail, std::ptr::null())
}

#[no_mangle]
pub extern "C" fn doohttp_last_error_status() -> i32 {
    get_last_error_status()
}

#[no_mangle]
pub extern "C" fn doohttp_last_error_json() -> *const c_char {
    string_to_c(&get_last_error_json())
}

/// Parse request body JSON and return a raw pointer suitable for passing to user function.
///
/// This function:
/// 1. Extracts the JSON body from the request
/// 2. Validates it against the handler's expected struct type
/// 3. Returns the raw JSON string pointer (user function receives this)
///
/// The user function's wrapper in codegen will handle the actual struct parsing.
/// This is a simpler approach that just validates and passes through.
///
/// Returns: Pointer to body JSON string, or null on error (check doohttp_last_error_*)
#[no_mangle]
pub extern "C" fn doohttp_get_validated_body(
    request_ptr: *const c_void,
    handler_name: *const c_char,
) -> *const c_char {
    clear_last_error();

    if request_ptr.is_null() {
        set_last_error(400, Rfc7807Error::bad_request("Null request").to_json());
        return std::ptr::null();
    }

    let handler_name_str = if handler_name.is_null() {
        return std::ptr::null();
    } else {
        c_to_string(handler_name)
    };

    let request = unsafe { &*(request_ptr as *const DooRequest) };
    let path_str = c_to_string(request.path);
    set_current_request_path(&path_str);

    // Get body
    if request.body.is_null() {
        set_last_error(
            400,
            bad_request("Missing request body", path_str.clone()).to_json(),
        );
        return std::ptr::null();
    }

    let body_str = c_to_string(request.body);
    if body_str.is_empty() {
        set_last_error(
            400,
            bad_request("Empty request body", path_str.clone()).to_json(),
        );
        return std::ptr::null();
    }

    // Validate body using populate_struct_from_request
    let validate_result = doohttp_populate_struct_from_request(
        request_ptr,
        std::ptr::null_mut(), // validation only
        0,                    // body
        handler_name,
    );

    if validate_result != 0 {
        // Error already set by populate_struct_from_request
        return std::ptr::null();
    }

    // Return the body string (the codegen's JSON parser will use this)
    string_to_c(&body_str)
}

#[no_mangle]
pub extern "C" fn doohttp_error_to_status(
    _error_type: *const c_char,
    variant: *const c_char,
) -> i32 {
    let variant_str = c_to_string(variant);
    match variant_str.as_str() {
        "NotFound" => 404,
        "InvalidInput" | "ValidationError" => 422,
        "Unauthorized" => 401,
        "Forbidden" => 403,
        "Conflict" | "AlreadyExists" => 409,
        "BadRequest" => 400,
        _ => 500,
    }
}

// ============================================================================
// HANDLER REGISTRATION
// ============================================================================

#[no_mangle]
pub extern "C" fn doo_http_register_handler(name: *const c_char, handler: DooHandlerFn) {
    let name_str = c_to_string(name);
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();
    registry.register_handler(&name_str, handler);
}

#[no_mangle]
pub extern "C" fn doo_http_register_handler_with_metadata(
    name: *const c_char,
    handler: DooHandlerFn,
    metadata_json: *const c_char,
) {
    let name_str = c_to_string(name);
    let routes = get_routes();
    let mut registry = routes.lock().unwrap();

    // Parse metadata JSON properly
    let metadata = if metadata_json.is_null() {
        HandlerMetadata::default()
    } else {
        let json_str = c_to_string(metadata_json);
        parse_handler_metadata(&json_str).unwrap_or_default()
    };

    registry.register_handler_with_metadata(&name_str, handler, metadata);
}

/// Parse handler metadata JSON into HandlerMetadata struct
fn parse_handler_metadata(json_str: &str) -> Option<HandlerMetadata> {
    let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let obj = parsed.as_object()?;

    // Extract param_types array
    let param_types = obj
        .get("param_types")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // Extract struct_layouts map
    let struct_layouts = obj
        .get("struct_layouts")
        .and_then(|v| v.as_object())
        .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    // Extract enum_variants map
    let enum_variants = obj
        .get("enum_variants")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| {
                    v.as_array().map(|arr| {
                        let variants: Vec<String> = arr
                            .iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect();
                        (k.clone(), variants)
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Extract return_type
    let return_type = obj
        .get("return_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Void".to_string());

    Some(HandlerMetadata {
        param_types,
        return_type,
        struct_decorators: HashMap::new(),
        struct_layouts,
        enum_variants,
    })
}

// ============================================================================
// RESPONSE HELPERS
// ============================================================================

#[no_mangle]
pub extern "C" fn doohttp_create_response_from_result(
    tag: i32,
    value_ptr: *const c_void,
    success_body_ptr: *const c_char,
) -> *mut DooResponse {
    unsafe {
        let response = libc::malloc(std::mem::size_of::<DooResponse>()) as *mut DooResponse;
        if response.is_null() {
            return std::ptr::null_mut();
        }

        if tag == 1 {
            // Error
            (*response).status = 500;
            (*response).body = if value_ptr.is_null() {
                string_to_c(r#"{"error":"Unknown error"}"#)
            } else {
                value_ptr as *const c_char
            };
        } else {
            // Success
            (*response).status = 200;
            (*response).body = success_body_ptr;
        }
        (*response).content_type = string_to_c(CONTENT_TYPE_JSON);
        response
    }
}

// ============================================================================
// STRUCT SERIALIZATION
// ============================================================================

/// Serialize a struct to JSON for HTTP response.
/// Takes struct pointer and handler name, looks up metadata to serialize.
#[no_mangle]
pub extern "C" fn doohttp_serialize_struct_to_json(
    struct_ptr: *const c_void,
    handler_name: *const c_char,
) -> *const c_char {
    if struct_ptr.is_null() || handler_name.is_null() {
        return string_to_c("{}");
    }

    let handler_name_str = c_to_string(handler_name);

    // Get handler metadata from registry
    let routes = get_routes();
    let registry = routes.lock().unwrap();
    let metadata = registry.handler_metadata.get(&handler_name_str).cloned();
    drop(registry);

    let metadata = match metadata {
        Some(m) => m,
        None => return string_to_c("{}"),
    };

    let return_type = &metadata.return_type;

    // If return type is a primitive, just return it as-is
    if return_type == "Str" {
        let s = c_to_string(struct_ptr as *const c_char);
        return string_to_c(&format!("\"{}\"", s.replace("\"", "\\\"")));
    }
    if return_type == "Int" {
        let i = unsafe { *(struct_ptr as *const i64) };
        return string_to_c(&i.to_string());
    }
    if return_type == "Float" {
        let f = unsafe { *(struct_ptr as *const f64) };
        return string_to_c(&f.to_string());
    }
    if return_type == "Bool" {
        let b = unsafe { *(struct_ptr as *const i8) != 0 };
        return string_to_c(if b { "true" } else { "false" });
    }
    if return_type == "Void" {
        return string_to_c("null");
    }

    // CRITICAL FIX: When return type is an array (e.g., [Post]) but the actual value
    // is a JSON string from db.raw(), detect this and pass through directly.
    // This happens when user writes: let result: [Post] = db.raw("SELECT ...")?;
    // The db.raw() returns a JSON string, not an in-memory struct array.
    if return_type.starts_with('[') && return_type.ends_with(']') {
        // Try to read as a C string first - if it's valid JSON, pass through
        if let Some(json_str) = try_read_as_json_string(struct_ptr) {
            // It's already a valid JSON string, return it directly
            return string_to_c(&json_str);
        }
        // Otherwise, fall through to struct serialization (for actual in-memory arrays)
    }

    // Serialize struct recursively
    let json = serialize_struct_recursive(
        struct_ptr as *const u8,
        return_type,
        &metadata.struct_layouts,
    );

    string_to_c(&json.to_string())
}

/// Try to read a pointer as a JSON string. Returns Some(json) if successful, None otherwise.
/// This is used to detect when db.raw() returns a JSON string that should be passed through.
fn try_read_as_json_string(ptr: *const c_void) -> Option<String> {
    if ptr.is_null() {
        return None;
    }

    unsafe {
        // First, check if this looks like a valid C string pointer
        // A valid JSON array from db.raw() starts with '['
        let byte_ptr = ptr as *const u8;

        // Safety check: try to read the first byte
        // If the pointer is to a struct with header, the first bytes would be
        // length/capacity (integers), not a printable character like '['
        let first_byte = *byte_ptr;

        // JSON arrays start with '[', JSON objects start with '{'
        // These are the only valid starts for db.raw() results
        if first_byte == b'[' || first_byte == b'{' {
            // This looks like it could be a JSON string, try to read it
            let c_str = std::ffi::CStr::from_ptr(ptr as *const c_char);
            if let Ok(s) = c_str.to_str() {
                // Validate that it's actually valid JSON
                if serde_json::from_str::<serde_json::Value>(s).is_ok() {
                    return Some(s.to_string());
                }
            }
        }
    }

    None
}

/// Recursively serialize a struct to JSON value.
fn serialize_struct_recursive(
    struct_ptr: *const u8,
    struct_name: &str,
    struct_layouts: &HashMap<String, serde_json::Value>,
) -> serde_json::Value {
    if struct_ptr.is_null() {
        return serde_json::Value::Null;
    }

    let layout = match struct_layouts.get(struct_name) {
        Some(l) => l,
        None => return serde_json::Value::Null,
    };

    let fields = match layout.get("fields").and_then(|f| f.as_array()) {
        Some(f) => f,
        None => return serde_json::Value::Null,
    };

    let mut json_obj = serde_json::Map::new();

    for field in fields {
        let field_obj = match field.as_object() {
            Some(obj) => obj,
            None => continue,
        };

        let field_name = match field_obj.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => continue,
        };

        let field_type = match field_obj.get("type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => continue,
        };

        // Use pre-computed offset from metadata (critical for correct struct layout)
        let offset = match field_obj.get("offset").and_then(|v| v.as_u64()) {
            Some(o) => o as usize,
            None => continue, // Skip fields without offset
        };

        unsafe {
            let field_ptr = struct_ptr.add(offset);
            let field_value = match field_type {
                "Str" => {
                    let str_ptr = *(field_ptr as *const *const c_char);
                    let s = if str_ptr.is_null() {
                        String::new()
                    } else {
                        c_to_string(str_ptr)
                    };
                    serde_json::Value::String(s)
                }
                "Int" => {
                    let i = *(field_ptr as *const i64);
                    serde_json::json!(i)
                }
                "Float" => {
                    let f = *(field_ptr as *const f64);
                    serde_json::json!(f)
                }
                "Bool" => {
                    let b = *(field_ptr as *const i8) != 0;
                    serde_json::json!(b)
                }
                t if t.starts_with("[") && t.ends_with("]") => {
                    // Array - struct stores pointer to data section
                    let arr_data = *(field_ptr as *const *const u8);
                    if arr_data.is_null() {
                        serde_json::Value::Array(vec![])
                    } else {
                        let elem_type = &t[1..t.len() - 1];
                        serialize_array(arr_data, elem_type, struct_layouts)
                    }
                }
                _ if struct_layouts.contains_key(field_type) => {
                    // Nested struct - pointer to struct
                    let nested_ptr = *(field_ptr as *const *const u8);
                    serialize_struct_recursive(nested_ptr, field_type, struct_layouts)
                }
                _ => serde_json::Value::Null,
            };
            json_obj.insert(field_name.to_string(), field_value);
        }
    }

    serde_json::Value::Object(json_obj)
}

/// Align a value up to the given alignment
#[allow(dead_code)]
fn align_up(offset: usize, align: usize) -> usize {
    if align == 0 {
        return offset;
    }
    (offset + align - 1) & !(align - 1)
}

/// Get the size and alignment for a type
#[allow(dead_code)]
fn get_type_size_align(
    type_name: &str,
    struct_layouts: &HashMap<String, serde_json::Value>,
) -> (usize, usize) {
    match type_name {
        "Str" => (8, 8),                                       // pointer
        "Int" => (8, 8),                                       // i64
        "Float" => (8, 8),                                     // double
        "Bool" => (1, 1),                                      // i1/i8
        t if t.starts_with("[") && t.ends_with("]") => (8, 8), // pointer to array data
        _ if struct_layouts.contains_key(type_name) => (8, 8), // pointer to struct
        _ => (8, 8),                                           // default to pointer size
    }
}

/// Serialize array to JSON.
fn serialize_array(
    arr_data_ptr: *const u8,
    elem_type: &str,
    struct_layouts: &HashMap<String, serde_json::Value>,
) -> serde_json::Value {
    if arr_data_ptr.is_null() {
        return serde_json::Value::Array(vec![]);
    }

    // The arr_data_ptr points to the data section
    // The header (len, cap) is 16 bytes before the data
    unsafe {
        let header_ptr = arr_data_ptr.offset(-16);
        let len = *(header_ptr as *const i64) as usize;

        if len == 0 {
            return serde_json::Value::Array(vec![]);
        }

        let mut arr = Vec::with_capacity(len);

        // Get element size for iteration
        let elem_size = match elem_type {
            "Str" => 8,                                       // pointer
            "Int" => 8,                                       // i64
            "Float" => 8,                                     // double
            "Bool" => 1,                                      // i1/i8
            _ if struct_layouts.contains_key(elem_type) => 8, // pointer
            _ => 8,
        };

        for i in 0..len {
            let elem_offset = i * elem_size;
            let elem_ptr = arr_data_ptr.add(elem_offset);

            let elem = match elem_type {
                "Str" => {
                    let str_ptr = *(elem_ptr as *const *const c_char);
                    let s = if str_ptr.is_null() {
                        String::new()
                    } else {
                        c_to_string(str_ptr)
                    };
                    serde_json::Value::String(s)
                }
                "Int" => {
                    let val = *(elem_ptr as *const i64);
                    serde_json::json!(val)
                }
                "Float" => {
                    let val = *(elem_ptr as *const f64);
                    serde_json::json!(val)
                }
                "Bool" => {
                    let val = *(elem_ptr as *const i8) != 0;
                    serde_json::json!(val)
                }
                _ if struct_layouts.contains_key(elem_type) => {
                    // Array of structs (each element is a pointer)
                    let nested_ptr = *(elem_ptr as *const *const u8);
                    serialize_struct_recursive(nested_ptr, elem_type, struct_layouts)
                }
                _ => serde_json::Value::Null,
            };
            arr.push(elem);
        }

        serde_json::Value::Array(arr)
    }
}

// ============================================================================
// UTILITY FUNCTIONS
// ============================================================================

fn make_ok_void() -> *mut DooResult {
    unsafe {
        let ptr = libc::malloc(std::mem::size_of::<DooResult>()) as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        (*ptr).tag = 0;
        (*ptr).value = std::ptr::null_mut();
        (*ptr).owner = owner::FFI;
        ptr
    }
}

fn make_ok_json(json: &str) -> *mut DooResult {
    ffi_debug!("FFI", "make_ok_json called with len={}", json.len());
    unsafe {
        let ptr = libc::malloc(std::mem::size_of::<DooResult>()) as *mut DooResult;
        if ptr.is_null() {
            ffi_debug!("FFI", "make_ok_json: malloc failed!");
            return std::ptr::null_mut();
        }
        (*ptr).tag = 0;
        let value_ptr = string_to_c(json) as *mut c_void;
        ffi_debug!("FFI", "make_ok_json: value_ptr={:?}", value_ptr);
        (*ptr).value = value_ptr;
        (*ptr).owner = owner::FFI;
        ffi_debug!("FFI", "make_ok_json: returning DooResult at {:?}", ptr);
        ptr
    }
}

/// Create an error result using centralized error response builder
/// Error response struct layout: { i32 status, ptr body, ptr content_type }
fn make_err_http(status: i32, message: &str) -> *mut DooResult {
    // Ensure set_last_error always stores proper RFC 7807 JSON
    let json_body = if message.starts_with('{') || message.starts_with('[') {
        message.to_string()
    } else {
        let path = get_current_request_path();
        Rfc7807Error::new(status as u16, message)
            .with_instance(&path)
            .to_json()
    };
    set_last_error(status, json_body);
    unsafe {
        // Use centralized helper to build error response struct
        let error_response = alloc_error_response(status, message);
        if error_response.is_null() {
            return std::ptr::null_mut();
        }

        // Now allocate and fill DooResult
        let ptr = libc::malloc(std::mem::size_of::<DooResult>()) as *mut DooResult;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        (*ptr).tag = 1; // Error
        (*ptr).value = error_response;
        (*ptr).owner = owner::FFI;
        ptr
    }
}

// ============================================================================
// MIDDLEWARE NEXT CALL
// ============================================================================

/// Call the next middleware/handler in the chain
/// The `next` parameter is actually a function pointer (DooNextFn) passed from the wrapper
/// We need to get the current request from thread-local and call the next function
#[no_mangle]
pub extern "C" fn doo_http_next_call(next: *const std::ffi::c_void) -> *mut std::ffi::c_void {
    use crate::server::get_current_request;

    if next.is_null() {
        // No next function - return null result
        return std::ptr::null_mut();
    }

    // The `next` is actually a DooNextFn: fn(*const DooRequest) -> *mut DooResult
    // Get the current request from thread-local storage
    let request_ptr = get_current_request();
    if request_ptr.is_null() {
        return std::ptr::null_mut();
    }

    // Cast next to the function type and call it
    let next_fn: crate::types::DooNextFn = unsafe { std::mem::transmute(next) };
    let result = next_fn(request_ptr);

    // The result is a DooResult containing a Response
    // We need to extract the Response pointer and return it
    if result.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let doo_result = &*result;

        // Build Response struct from DooResult
        // Response layout: { i64 Status, ptr Body, ptr ContentType }
        let response_size = std::mem::size_of::<i64>() + 2 * std::mem::size_of::<*const i8>();
        let response_ptr = libc::malloc(response_size) as *mut u8;
        if response_ptr.is_null() {
            return std::ptr::null_mut();
        }

        if doo_result.tag == 0 {
            // Ok result - build Response with status 200 and the JSON body
            *(response_ptr as *mut i64) = 200;
            *((response_ptr as *mut u8).add(8) as *mut *const i8) = doo_result.value as *const i8;
            *((response_ptr as *mut u8).add(16) as *mut *const i8) =
                string_to_c("application/json");
        } else {
            // Error result - value is error response struct { i32 status, ptr body, ptr content_type }
            // Extract fields from error struct and build Response
            let error_struct = doo_result.value as *const u8;
            let status = *(error_struct as *const i32) as i64;
            let body_ptr = *((error_struct as *const u8).add(8) as *const *const i8);
            let ct_ptr = *((error_struct as *const u8).add(16) as *const *const i8);

            *(response_ptr as *mut i64) = status;
            *((response_ptr as *mut u8).add(8) as *mut *const i8) = body_ptr;
            *((response_ptr as *mut u8).add(16) as *mut *const i8) = ct_ptr;
        }

        response_ptr as *mut std::ffi::c_void
    }
}

// ============================================================================
// WEBSOCKET FFI ENTRY POINTS — All WS FFI functions centralized here
// ============================================================================
// These delegate to the ws:: submodule. Keeping FFI surface in lib.rs
// follows the same pattern as HTTP routes above.

/// Register a WebSocket route on the HTTP server.
/// Doo syntax: `app.ws("/chat", (conn) => { ... })`
#[no_mangle]
pub extern "C" fn doo_ws_route(
    _server: *const c_void,
    path: *const c_char,
    handler: ws::WsConnectionHandler,
) -> *mut DooResult {
    let path_str = c_to_string(path);
    ffi_debug!("WS", "Registering WebSocket route: {}", path_str);
    ws::get_ws_registry().register_route(&path_str, handler);
    make_ok_void()
}

/// Initialize WebSocket subsystem (called automatically).
#[no_mangle]
pub extern "C" fn doo_ws_init() {
    ffi_debug!("WS", "WebSocket subsystem initialized");
}

/// Get the connection ID.
/// Doo syntax: `conn.id`
#[no_mangle]
pub extern "C" fn doo_ws_conn_id(conn: *const ws::WsConnection) -> *const c_char {
    if conn.is_null() {
        return string_to_c("");
    }
    unsafe { string_to_c(&(*conn).id) }
}

/// Emit a JSON event to a specific connection.
/// Doo syntax: `conn.emit("event", data)?`
#[no_mangle]
pub extern "C" fn doo_ws_conn_emit(
    conn: *const ws::WsConnection,
    event: *const c_char,
    payload: *const c_char,
) -> *mut DooResult {
    if conn.is_null() {
        return make_err_http(400, "Null connection");
    }
    let event_str = c_to_string(event);
    let payload_str = c_to_string(payload);
    let conn_id = unsafe { &(*conn).id };

    let frame = ws::build_ws_frame(&event_str, &payload_str);

    ffi_debug!("WS", "conn.emit({}) to {}", event_str, conn_id);

    match ws::get_conn_registry().send_text(conn_id, &frame) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err_http(500, &format!("emit failed: {}", e)),
    }
}

/// Emit binary data to a specific connection.
/// Doo syntax: `conn.emitBinary(bytes)?`
#[no_mangle]
pub extern "C" fn doo_ws_conn_emit_binary(
    conn: *const ws::WsConnection,
    data: *const u8,
    len: i64,
) -> *mut DooResult {
    if conn.is_null() || data.is_null() || len <= 0 {
        return make_err_http(400, "Invalid binary emit parameters");
    }
    let conn_id = unsafe { &(*conn).id };
    let bytes = unsafe { std::slice::from_raw_parts(data, len as usize) };

    match ws::get_conn_registry().send_binary(conn_id, bytes) {
        Ok(_) => make_ok_void(),
        Err(e) => make_err_http(500, &format!("emitBinary failed: {}", e)),
    }
}

/// Join a room.
/// Doo syntax: `conn.join("room1")?`
#[no_mangle]
pub extern "C" fn doo_ws_conn_join(
    conn: *const ws::WsConnection,
    room: *const c_char,
) -> *mut DooResult {
    if conn.is_null() {
        return make_err_http(400, "Null connection");
    }
    let conn_id = unsafe { &(*conn).id };
    let room_str = c_to_string(room);
    ffi_debug!("WS", "conn.join({}) for {}", room_str, conn_id);
    ws::get_room_registry().join(&room_str, conn_id);
    make_ok_void()
}

/// Leave a room.
/// Doo syntax: `conn.leave("room1")?`
#[no_mangle]
pub extern "C" fn doo_ws_conn_leave(
    conn: *const ws::WsConnection,
    room: *const c_char,
) -> *mut DooResult {
    if conn.is_null() {
        return make_err_http(400, "Null connection");
    }
    let conn_id = unsafe { &(*conn).id };
    let room_str = c_to_string(room);
    ffi_debug!("WS", "conn.leave({}) for {}", room_str, conn_id);
    ws::get_room_registry().leave(&room_str, conn_id);
    make_ok_void()
}

/// Close a connection.
/// Doo syntax: `conn.close()?`
#[no_mangle]
pub extern "C" fn doo_ws_conn_close(conn: *const ws::WsConnection) -> *mut DooResult {
    if conn.is_null() {
        return make_err_http(400, "Null connection");
    }
    let conn_id = unsafe { &(*conn).id };
    ffi_debug!("WS", "conn.close() for {}", conn_id);
    ws::get_conn_registry().close(conn_id);
    make_ok_void()
}

/// Check if a connection is closed.
/// Doo syntax: `conn.isClosed()`
#[no_mangle]
pub extern "C" fn doo_ws_conn_is_closed(conn: *const ws::WsConnection) -> i64 {
    if conn.is_null() {
        return 1;
    }
    let conn_id = unsafe { &(*conn).id };
    if ws::get_conn_registry().is_closed(conn_id) {
        1
    } else {
        0
    }
}

/// Register an event handler on the connection.
/// Doo syntax: `conn.on("message", (msg) => { ... })`
#[no_mangle]
pub extern "C" fn doo_ws_conn_on(
    conn: *const ws::WsConnection,
    event: *const c_char,
    handler: ws::WsEventHandler,
) -> *mut DooResult {
    if conn.is_null() {
        return make_err_http(400, "Null connection");
    }
    let conn_id = unsafe { &(*conn).id };
    let event_str = c_to_string(event);
    ffi_debug!("WS", "conn.on({}) for {}", event_str, conn_id);
    ws::get_conn_registry().register_event_handler(conn_id, &event_str, handler);
    make_ok_void()
}

/// Register onConnect handler.
/// Doo syntax: `conn.onConnect(() => { ... })`
#[no_mangle]
pub extern "C" fn doo_ws_conn_on_connect(
    conn: *const ws::WsConnection,
    handler: ws::WsLifecycleHandler,
) -> *mut DooResult {
    if conn.is_null() {
        return make_err_http(400, "Null connection");
    }
    let conn_id = unsafe { &(*conn).id };
    ffi_debug!("WS", "conn.onConnect for {}", conn_id);
    ws::get_conn_registry().set_on_connect(conn_id, handler);
    make_ok_void()
}

/// Register onDisconnect handler.
/// Doo syntax: `conn.onDisconnect(() => { ... })`
#[no_mangle]
pub extern "C" fn doo_ws_conn_on_disconnect(
    conn: *const ws::WsConnection,
    handler: ws::WsLifecycleHandler,
) -> *mut DooResult {
    if conn.is_null() {
        return make_err_http(400, "Null connection");
    }
    let conn_id = unsafe { &(*conn).id };
    ffi_debug!("WS", "conn.onDisconnect for {}", conn_id);
    ws::get_conn_registry().set_on_disconnect(conn_id, handler);
    make_ok_void()
}

/// Register onError handler.
/// Doo syntax: `conn.onError((err) => { ... })`
#[no_mangle]
pub extern "C" fn doo_ws_conn_on_error(
    conn: *const ws::WsConnection,
    handler: ws::WsErrorHandler,
) -> *mut DooResult {
    if conn.is_null() {
        return make_err_http(400, "Null connection");
    }
    let conn_id = unsafe { &(*conn).id };
    ffi_debug!("WS", "conn.onError for {}", conn_id);
    ws::get_conn_registry().set_on_error(conn_id, handler);
    make_ok_void()
}

/// Broadcast an event to ALL connected clients.
/// Doo syntax: `ws.broadcast("event", data)?`
#[no_mangle]
pub extern "C" fn doo_ws_broadcast(
    _server: *const c_void,
    event: *const c_char,
    payload: *const c_char,
) -> *mut DooResult {
    let event_str = c_to_string(event);
    let payload_str = c_to_string(payload);

    let frame = ws::build_ws_frame(&event_str, &payload_str);

    ffi_debug!("WS", "broadcast({})", event_str);
    let failures = ws::get_conn_registry().broadcast_text(&frame);
    if failures > 0 {
        ffi_debug!("WS", "broadcast had {} failed sends", failures);
    }
    make_ok_void()
}

/// Emit an event to all connections in a specific room.
/// Doo syntax: `ws.to("room1").emit("event", data)?`
#[no_mangle]
pub extern "C" fn doo_ws_room_emit(
    _server: *const c_void,
    room: *const c_char,
    event: *const c_char,
    payload: *const c_char,
) -> *mut DooResult {
    let room_str = c_to_string(room);
    let event_str = c_to_string(event);
    let payload_str = c_to_string(payload);

    let frame = ws::build_ws_frame(&event_str, &payload_str);

    ffi_debug!("WS", "room_emit({}, {})", room_str, event_str);
    let conn_ids = ws::get_room_registry().get_members(&room_str);
    let mut failures = 0usize;
    for conn_id in &conn_ids {
        if ws::get_conn_registry().send_text(conn_id, &frame).is_err() {
            failures += 1;
        }
    }
    if failures > 0 {
        ffi_debug!("WS", "room_emit had {} failed sends", failures);
    }
    make_ok_void()
}

/// Set WebSocket configuration.
/// Doo syntax: `ws.config({ "max_message_size": "65536", ... })`
#[no_mangle]
pub extern "C" fn doo_ws_config(
    _server: *const c_void,
    config_json: *const c_char,
) -> *mut DooResult {
    let json_str = c_to_string(config_json);
    ffi_debug!("WS", "Setting WS config: {}", json_str);

    match serde_json::from_str::<serde_json::Value>(&json_str) {
        Ok(val) => {
            let mut cfg = ws::get_ws_config().write().unwrap();
            if let Some(v) = val.get("max_message_size").and_then(|v| v.as_u64()) {
                cfg.max_message_size = v as usize;
            }
            if let Some(v) = val.get("heartbeat_interval").and_then(|v| v.as_u64()) {
                cfg.heartbeat_interval_secs = v;
            }
            if let Some(v) = val.get("heartbeat_timeout").and_then(|v| v.as_u64()) {
                cfg.heartbeat_timeout_secs = v;
            }
            if let Some(v) = val.get("send_queue_size").and_then(|v| v.as_u64()) {
                cfg.send_queue_size = v as usize;
            }
            drop(cfg);
            make_ok_void()
        }
        Err(e) => make_err_http(400, &format!("Invalid config JSON: {}", e)),
    }
}

/// Graceful shutdown — close all WebSocket connections.
#[no_mangle]
pub extern "C" fn doo_ws_shutdown(_server: *const c_void) {
    ffi_debug!("WS", "Shutting down WebSocket subsystem");
    ws::get_conn_registry().shutdown_all();
}

/// Get count of active WebSocket connections.
#[no_mangle]
pub extern "C" fn doo_ws_active_connections(_server: *const c_void) -> i64 {
    ws::get_conn_registry().count() as i64
}

/// Check if a path is a registered WebSocket route (used by server.rs).
#[no_mangle]
pub extern "C" fn doo_ws_is_ws_route(_server: *const c_void, path: *const c_char) -> i64 {
    let path_str = c_to_string(path);
    if ws::is_ws_route(&path_str) {
        1
    } else {
        0
    }
}
