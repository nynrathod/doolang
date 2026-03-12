//! FFI Function Names & Built-in Module Names - Single Source of Truth
//!
//! ALL FFI function name strings and built-in module names are centralized here.
//! No hardcoded strings in codegen, type checker, or anywhere else.
//!
//! ## Usage
//! ```rust,ignore
//! use doo_core::constants::ffi_names;
//! let malloc_fn = module.get_function(ffi_names::MALLOC)?;
//!
//! // Check if a name is a built-in module
//! if ffi_names::is_core_module(name) { ... }
//! ```

// ============================================================================
// Built-in Module Names
// ============================================================================
// NOTE: Package modules (Http, Auth, Database, WebSocket, Process, Git) are
// listed here temporarily for module recognition by type_check, MIR, and
// capture analysis. In Phase 2 (Solution 2), these will be discovered
// dynamically via the import resolution system from packages/ directory.
// Core modules (JSON, Math, File, Random, Array, Console, Config) will
// remain as true built-ins.

/// JSON module for parsing and stringifying JSON data
pub const MODULE_JSON: &str = "JSON";
/// Math module for mathematical operations
pub const MODULE_MATH: &str = "Math";
/// File module for file system operations
pub const MODULE_FILE: &str = "File";
/// Random module for random number generation
pub const MODULE_RANDOM: &str = "Random";
/// Array module for array utilities
pub const MODULE_ARRAY: &str = "Array";
/// Console module for console I/O
pub const MODULE_CONSOLE: &str = "Console";
/// Config module for environment variable access
pub const MODULE_CONFIG: &str = "Config";

/// Core language modules — fallback list of true built-ins.
/// The canonical list is discovered at runtime from the `std/` directory
/// via `discovered_core_modules()`. This constant is only used as a fallback
/// when the std directory cannot be found.
pub const CORE_MODULES: &[&str] = &[
    MODULE_JSON,
    MODULE_MATH,
    MODULE_FILE,
    MODULE_RANDOM,
    MODULE_ARRAY,
    MODULE_CONSOLE,
    MODULE_CONFIG,
];

/// Discover core modules by scanning the `std/` directory for `.doo` files.
/// Falls back to `CORE_MODULES` constant if the directory cannot be found.
/// Result is cached via OnceLock — directory is only scanned once per process.
pub fn discovered_core_modules() -> &'static [String] {
    use std::sync::OnceLock;
    static DISCOVERED: OnceLock<Vec<String>> = OnceLock::new();

    DISCOVERED.get_or_init(|| {
        // Try to find the std/ directory relative to the executable
        let mut modules = Vec::new();

        // Look for std/ in several locations (executable dir, cwd, parent dirs)
        let search_paths: Vec<std::path::PathBuf> = {
            let mut paths = Vec::new();
            if let Ok(exe_path) = std::env::current_exe() {
                if let Some(exe_dir) = exe_path.parent() {
                    paths.push(exe_dir.join("std"));
                    // Also check parent (for cargo target/release/ layout)
                    if let Some(parent) = exe_dir.parent() {
                        paths.push(parent.join("std"));
                        if let Some(grandparent) = parent.parent() {
                            paths.push(grandparent.join("std"));
                            if let Some(great) = grandparent.parent() {
                                paths.push(great.join("std"));
                            }
                        }
                    }
                }
            }
            if let Ok(cwd) = std::env::current_dir() {
                paths.push(cwd.join("std"));
            }
            paths
        };

        for std_dir in &search_paths {
            if std_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(std_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().map_or(false, |e| e == "doo") {
                            if let Some(stem) = path.file_stem() {
                                if let Some(name) = stem.to_str() {
                                    if !modules.contains(&name.to_string()) {
                                        modules.push(name.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
                if !modules.is_empty() {
                    // Also add the always-available built-ins that may not be in std/ as files
                    for &builtin in &[MODULE_JSON, MODULE_CONSOLE] {
                        if !modules.iter().any(|m| m == builtin) {
                            modules.push(builtin.to_string());
                        }
                    }
                    return modules;
                }
            }
        }

        // Fallback to the hardcoded list
        CORE_MODULES.iter().map(|s| s.to_string()).collect()
    })
}

/// Check if a name is a core built-in module (language-level, not packages).
/// Uses runtime discovery from std/ directory, with fallback to CORE_MODULES constant.
/// Package modules (Http, Auth, Database, etc.) are discovered from program
/// imports — they are NOT hardcoded here. See TypeChecker::is_known_module()
/// and MirBuilder::is_module_name() for the discovery-based approach.
#[inline]
pub fn is_core_module(name: &str) -> bool {
    discovered_core_modules().iter().any(|m| m == name)
}

// ============================================================================
// Compiler Internal Names
// ============================================================================

/// Name used for anonymous object literals `{ key: value }` in HIR.
/// ObjectLit is lowered to `HirExprKind::Struct { name: OBJECT_LIT_NAME, .. }`
/// and compiled to a `HashMap<String, String>` at codegen time.
pub const OBJECT_LIT_NAME: &str = "__anon";

/// Check if a struct name is an anonymous object literal
#[inline]
pub fn is_object_lit(name: &str) -> bool {
    name == OBJECT_LIT_NAME
}

// ============================================================================
// Standard C Library Functions
// ============================================================================

pub const MALLOC: &str = "malloc";
pub const FREE: &str = "free";
pub const REALLOC: &str = "realloc";
pub const MEMCPY: &str = "memcpy";
pub const MEMSET: &str = "memset";
pub const MEMMOVE: &str = "memmove";
pub const STRLEN: &str = "strlen";
pub const STRCMP: &str = "strcmp";
pub const STRSTR: &str = "strstr";
pub const STRNCMP: &str = "strncmp";
pub const STRCPY: &str = "strcpy";
pub const STRCAT: &str = "strcat";
pub const PRINTF: &str = "printf";
pub const SNPRINTF: &str = "snprintf";
pub const PUTCHAR: &str = "putchar";
pub const PUTS: &str = "puts";
pub const SPRINTF: &str = "sprintf";
pub const EXIT: &str = "exit";
pub const FFLUSH: &str = "fflush";

// ============================================================================
// Doo Core Runtime Functions
// ============================================================================

pub const DOO_ALLOC: &str = "doo_alloc";
pub const DOO_FREE: &str = "doo_free";
pub const DOO_REALLOC: &str = "doo_realloc";
pub const DOO_CLONE: &str = "doo_clone";
pub const DOO_MEMCPY: &str = "doo_memcpy";
pub const DOO_ZERO: &str = "doo_zero";

// ============================================================================
// Doo JSON FFI (doo_ffi_json)
// ============================================================================

// Writer API
pub const DOO_JSON_WRITER_NEW: &str = "doo_json_writer_new";
pub const DOO_JSON_WRITER_NEW_WITH_CAP: &str = "doo_json_writer_new_with_cap";
pub const DOO_JSON_WRITER_FREE: &str = "doo_json_writer_free";
pub const DOO_JSON_WRITER_FINISH: &str = "doo_json_writer_finish";

// Structural
pub const DOO_JSON_WRITE_START_OBJECT: &str = "doo_json_write_start_object";
pub const DOO_JSON_WRITE_END_OBJECT: &str = "doo_json_write_end_object";
pub const DOO_JSON_WRITE_START_ARRAY: &str = "doo_json_write_start_array";
pub const DOO_JSON_WRITE_END_ARRAY: &str = "doo_json_write_end_array";
pub const DOO_JSON_WRITE_COMMA: &str = "doo_json_write_comma";
pub const DOO_JSON_WRITE_COLON: &str = "doo_json_write_colon";
pub const DOO_JSON_WRITE_KEY: &str = "doo_json_write_key";
pub const DOO_JSON_WRITE_KEY_INT: &str = "doo_json_write_key_int";
pub const DOO_JSON_WRITE_KEY_FLOAT: &str = "doo_json_write_key_float";
pub const DOO_JSON_WRITE_KEY_BOOL: &str = "doo_json_write_key_bool";

// Primitives
pub const DOO_JSON_WRITE_INT: &str = "doo_json_write_int";
pub const DOO_JSON_WRITE_FLOAT: &str = "doo_json_write_float";
pub const DOO_JSON_WRITE_BOOL: &str = "doo_json_write_bool";
pub const DOO_JSON_WRITE_STRING: &str = "doo_json_write_string";
pub const DOO_JSON_WRITE_NULL: &str = "doo_json_write_null";

// Reader/Parse API (type-specific)
pub const DOO_JSON_PARSE: &str = "doo_json_parse";
pub const DOO_JSON_PARSE_INT: &str = "doo_json_parse_int";
pub const DOO_JSON_PARSE_FLOAT: &str = "doo_json_parse_float";
pub const DOO_JSON_PARSE_BOOL: &str = "doo_json_parse_bool";
pub const DOO_JSON_PARSE_STR: &str = "doo_json_parse_str";
pub const DOO_JSON_PARSE_ARRAY_INT: &str = "doo_json_parse_array_int";
pub const DOO_JSON_PARSE_ARRAY_FLOAT: &str = "doo_json_parse_array_float";
pub const DOO_JSON_PARSE_ARRAY_BOOL: &str = "doo_json_parse_array_bool";
pub const DOO_JSON_PARSE_ARRAY_STR: &str = "doo_json_parse_array_str";

// Array helper functions (for codegen-driven struct/enum array parsing)
pub const DOO_JSON_ARRAY_COUNT: &str = "doo_json_array_count";
pub const DOO_JSON_ARRAY_GET_ELEMENT: &str = "doo_json_array_get_element";
pub const DOO_JSON_PARSE_MAP_STR_INT: &str = "doo_json_parse_map_str_int";
pub const DOO_JSON_PARSE_MAP_STR_FLOAT: &str = "doo_json_parse_map_str_float";
pub const DOO_JSON_PARSE_MAP_STR_BOOL: &str = "doo_json_parse_map_str_bool";
pub const DOO_JSON_PARSE_MAP_STR_STR: &str = "doo_json_parse_map_str_str";
pub const DOO_JSON_PARSE_MAP_INT_INT: &str = "doo_json_parse_map_int_int";
pub const DOO_JSON_PARSE_MAP_INT_FLOAT: &str = "doo_json_parse_map_int_float";
pub const DOO_JSON_PARSE_MAP_INT_BOOL: &str = "doo_json_parse_map_int_bool";
pub const DOO_JSON_PARSE_MAP_INT_STR: &str = "doo_json_parse_map_int_str";
pub const DOO_JSON_PARSE_MAP_FLOAT_INT: &str = "doo_json_parse_map_float_int";
pub const DOO_JSON_PARSE_MAP_FLOAT_FLOAT: &str = "doo_json_parse_map_float_float";
pub const DOO_JSON_PARSE_MAP_FLOAT_BOOL: &str = "doo_json_parse_map_float_bool";
pub const DOO_JSON_PARSE_MAP_FLOAT_STR: &str = "doo_json_parse_map_float_str";
pub const DOO_JSON_PARSE_MAP_BOOL_INT: &str = "doo_json_parse_map_bool_int";
pub const DOO_JSON_PARSE_MAP_BOOL_FLOAT: &str = "doo_json_parse_map_bool_float";
pub const DOO_JSON_PARSE_MAP_BOOL_BOOL: &str = "doo_json_parse_map_bool_bool";
pub const DOO_JSON_PARSE_MAP_BOOL_STR: &str = "doo_json_parse_map_bool_str";

// Struct/Enum parse helpers
pub const DOO_JSON_GET_FIELD: &str = "doo_json_get_field";
pub const DOO_JSON_GET_VARIANT_NAME: &str = "doo_json_get_variant_name";
pub const DOO_JSON_GET_VARIANT_PAYLOAD: &str = "doo_json_get_variant_payload";
pub const DOO_JSON_IS_UNIT_VARIANT: &str = "doo_json_is_unit_variant";

// Parse-once object API (zero re-serialization)
pub const DOO_JSON_PARSE_OBJECT: &str = "doo_json_parse_object";
pub const DOO_JSON_OBJECT_GET_INT: &str = "doo_json_object_get_int";
pub const DOO_JSON_OBJECT_GET_FLOAT: &str = "doo_json_object_get_float";
pub const DOO_JSON_OBJECT_GET_BOOL: &str = "doo_json_object_get_bool";
pub const DOO_JSON_OBJECT_GET_STR: &str = "doo_json_object_get_str";
pub const DOO_JSON_OBJECT_GET_JSON: &str = "doo_json_object_get_json";
pub const DOO_JSON_OBJECT_FREE: &str = "doo_json_object_free";

// ============================================================================
// Doo String FFI (UTF-8 Safe Operations)
// ============================================================================

pub const DOO_STRING_LEN_UTF8: &str = "doo_string_len_utf8";
pub const DOO_STRING_CHAR_AT_UTF8: &str = "doo_string_char_at_utf8";
pub const DOO_STRING_REVERSE_UTF8: &str = "doo_string_reverse_utf8";
pub const DOO_STRING_SUBSTRING_UTF8: &str = "doo_string_substring_utf8";
pub const DOO_STRING_REPLACE: &str = "doo_string_replace";
pub const DOO_STRING_TRIM: &str = "doo_string_trim";
pub const DOO_STRING_TRIM_START: &str = "doo_string_trim_start";
pub const DOO_STRING_TRIM_END: &str = "doo_string_trim_end";
pub const DOO_STRING_SPLIT: &str = "doo_string_split";

// ============================================================================
// Math Functions (if needed beyond LLVM intrinsics)
// ============================================================================

pub const FABS: &str = "fabs";
pub const FLOOR: &str = "floor";
pub const CEIL: &str = "ceil";
pub const ROUND: &str = "round";
pub const SQRT: &str = "sqrt";
pub const POW: &str = "pow";

// ============================================================================
// Type Conversion / Casting
// ============================================================================

pub const DOO_CAST_STR_TO_INT: &str = "doo_cast_str_to_int";
pub const DOO_CAST_STR_TO_FLOAT: &str = "doo_cast_str_to_float";
pub const DOO_CAST_INT_TO_STR: &str = "doo_cast_int_to_str";
pub const DOO_CAST_FLOAT_TO_STR: &str = "doo_cast_float_to_str";
pub const DOO_FORMAT_FLOAT: &str = "doo_format_float";

// ============================================================================
// Runtime Type Information (for Any/JSON support)
// ============================================================================

pub const DOO_BOX_INT: &str = "doo_box_int";
pub const DOO_BOX_FLOAT: &str = "doo_box_float";
pub const DOO_BOX_BOOL: &str = "doo_box_bool";
pub const DOO_BOX_NULL: &str = "doo_box_null";
pub const DOO_UNBOX_INT: &str = "doo_unbox_int";
pub const DOO_UNBOX_FLOAT: &str = "doo_unbox_float";
pub const DOO_UNBOX_BOOL: &str = "doo_unbox_bool";
pub const DOO_TYPEOF: &str = "doo_typeof";

// ============================================================================
// Collection Helpers (FFI-based)
// ============================================================================

pub const DOO_ARRAY_CREATE: &str = "doo_array_create";
pub const DOO_ARRAY_CREATE_WITH_CAP: &str = "doo_array_create_with_cap";
pub const DOO_ARRAY_PUSH: &str = "doo_array_push";
pub const DOO_ARRAY_FREE: &str = "doo_array_free";

pub const DOO_MAP_CREATE: &str = "doo_map_create";
pub const DOO_MAP_NEW: &str = "doo_map_new";
pub const DOO_MAP_SET: &str = "doo_map_set";
pub const DOO_MAP_GET: &str = "doo_map_get";
pub const DOO_MAP_FREE: &str = "doo_map_free";
pub const DOO_MAP_SET_STR_ARRAY: &str = "doo_map_set_str_array";

pub const DOO_STRING_CREATE: &str = "doo_string_create";
pub const DOO_STRING_FREE: &str = "doo_string_free";

// ============================================================================
// Doo Async/Runtime FFI (doo_ffi_runtime)
// ============================================================================

pub const DOO_RUNTIME_INIT: &str = "doo_runtime_init";
pub const DOO_RUNTIME_BLOCK_ON: &str = "doo_runtime_block_on";
pub const DOO_SPAWN: &str = "doo_spawn";
pub const DOO_SPAWN_DETACH: &str = "doo_spawn_detach";
pub const DOO_SPAWN_BLOCKING: &str = "doo_spawn_blocking";
pub const DOO_TASK_AWAIT: &str = "doo_task_await";
pub const DOO_TASK_CANCEL: &str = "doo_task_cancel";
pub const DOO_TASK_FREE: &str = "doo_task_free";
pub const DOO_SCOPE_CREATE: &str = "doo_scope_create";
pub const DOO_SCOPE_SPAWN: &str = "doo_scope_spawn";
pub const DOO_SCOPE_WAIT: &str = "doo_scope_wait";
pub const DOO_SCOPE_FREE: &str = "doo_scope_free";
pub const DOO_SLEEP: &str = "doo_sleep";
pub const DOO_SLEEP_ASYNC: &str = "doo_sleep_async";
pub const DOO_TIMEOUT: &str = "doo_timeout";

// ============================================================================
// Self-Returning Method Names - Single Source of Truth
// ============================================================================
// These static methods on struct types return the struct type itself.
// Used by visibility checker to infer types for expressions like:
//   let db = Database::get()?;    // Database.get() -> Database
//   let app = Server::new(":3000"); // Server.new() -> Server
//
// Patterns:
// - Constructor methods: create new instances of the type
// - Accessor methods: get existing instances (singletons, globals, pools)
// - Connection methods: establish connections that return typed handles

/// Constructor method names that create new instances of the receiver type
pub const SELF_RETURNING_CONSTRUCTORS: &[&str] = &[
    "new",     // Type.new() -> Type (most common)
    "create",  // Type.create() -> Type
    "build",   // Type.build() -> Type (builder pattern)
    "default", // Type.default() -> Type (default instance)
    "init",    // Type.init() -> Type
];

/// Accessor method names that return existing instances of the receiver type
pub const SELF_RETURNING_ACCESSORS: &[&str] = &[
    "get",       // Type.get() -> Type (global/singleton access)
    "instance",  // Type.instance() -> Type (singleton)
    "global",    // Type.global() -> Type
    "singleton", // Type.singleton() -> Type
    "shared",    // Type.shared() -> Type (shared instance)
];

/// Connection method names that establish connections returning typed handles.
/// Only generic patterns — specific driver names (Postgres, mysql, sqlite)
/// are not hardcoded; return types come from @extern declarations.
pub const SELF_RETURNING_CONNECTORS: &[&str] = &[
    "connect", // Generic connection pattern
    "open",    // Generic open pattern
];

/// Check if a method name is a self-returning pattern
/// Returns true if the method likely returns an instance of its receiver type
#[inline]
pub fn is_self_returning_method(method: &str) -> bool {
    SELF_RETURNING_CONSTRUCTORS.contains(&method)
        || SELF_RETURNING_ACCESSORS.contains(&method)
        || SELF_RETURNING_CONNECTORS.contains(&method)
}

// ============================================================================
// Doo Config FFI (doo_ffi_core) - Single Source of Truth
// ============================================================================

/// Get environment variable by key (panics if missing)
pub const DOO_CONFIG_GET: &str = "doo_config_get";
/// Get environment variable by key with default fallback
pub const DOO_CONFIG_GET_OR: &str = "doo_config_get_or";
/// Check if environment variable exists
pub const DOO_CONFIG_HAS: &str = "doo_config_has";
/// Get environment variable as integer with default
pub const DOO_CONFIG_GET_INT: &str = "doo_config_get_int";
/// Get environment variable as boolean with default
pub const DOO_CONFIG_GET_BOOL: &str = "doo_config_get_bool";
/// Set environment variable at runtime
pub const DOO_CONFIG_SET: &str = "doo_config_set";

// ============================================================================
// FFI Symbol Derivation — Single Source of Truth
// ============================================================================

/// Derive an FFI symbol name from a library name and function name.
///
/// This is the **single source of truth** for FFI symbol naming.
/// Used by both MIR builder and codegen to generate consistent symbol names.
///
/// # Examples
///
/// ```
/// use doo_core::constants::ffi_names::derive_ffi_symbol;
/// assert_eq!(derive_ffi_symbol("doo_http", "Server.new"), "doo_http_server_new");
/// assert_eq!(derive_ffi_symbol("doo_http", "Server.get"), "doo_http_server_get");
/// assert_eq!(derive_ffi_symbol("doo_db", "Query.exec"), "doo_db_query_exec");
/// assert_eq!(derive_ffi_symbol("doo_http", "myFunc"), "doo_http_myfunc");
/// ```
pub fn derive_ffi_symbol(library: &str, func_name: &str) -> String {
    // Split function name by '.' for methods
    let parts: Vec<&str> = func_name.split('.').collect();

    if parts.len() == 2 {
        // Method: Server.get -> {library}_server_get
        let type_name = parts[0].to_lowercase();
        let method_name = parts[1].to_lowercase();
        format!("{}_{}_{}", library, type_name, method_name)
    } else {
        // Plain function: myFunc -> {library}_myfunc
        format!("{}_{}", library, func_name.to_lowercase())
    }
}
