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
//! if ffi_names::is_builtin_module(name) { ... }
//! ```

// ============================================================================
// Built-in Module Names (Static Modules like JSON, Math, File, etc.)
// ============================================================================

/// JSON module for parsing and stringifying JSON data
pub const MODULE_JSON: &str = "JSON";
/// Math module for mathematical operations
pub const MODULE_MATH: &str = "Math";
/// File module for file system operations
pub const MODULE_FILE: &str = "File";
/// Http module for HTTP server and client operations  
pub const MODULE_HTTP: &str = "Http";
/// Auth module for authentication (hashing, JWT, etc.)
pub const MODULE_AUTH: &str = "Auth";
/// Database module for database operations
pub const MODULE_DATABASE: &str = "Database";
/// Random module for random number generation
pub const MODULE_RANDOM: &str = "Random";
/// Array module for array utilities
pub const MODULE_ARRAY: &str = "Array";
/// Console module for console I/O
pub const MODULE_CONSOLE: &str = "Console";
/// WebSocket module for WebSocket server operations
pub const MODULE_WEBSOCKET: &str = "WebSocket";
/// Process module for command/process execution
pub const MODULE_PROCESS: &str = "Process";
/// Config module for environment variable access
pub const MODULE_CONFIG: &str = "Config";
/// Git module for native git operations (libgit2)
pub const MODULE_GIT: &str = "Git";

/// All built-in module names in a static array for iteration/lookup
pub const BUILTIN_MODULES: &[&str] = &[
    MODULE_JSON,
    MODULE_MATH,
    MODULE_FILE,
    MODULE_HTTP,
    MODULE_AUTH,
    MODULE_DATABASE,
    MODULE_RANDOM,
    MODULE_ARRAY,
    MODULE_CONSOLE,
    MODULE_WEBSOCKET,
    MODULE_PROCESS,
    MODULE_CONFIG,
    MODULE_GIT,
];

/// Check if a name is a built-in module
#[inline]
pub fn is_builtin_module(name: &str) -> bool {
    BUILTIN_MODULES.contains(&name)
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
// Doo HTTP FFI (doo_ffi_http)
// ============================================================================

pub const DOO_HTTP_GET_SERVER_INSTANCE: &str = "doo_http_get_server_instance";
pub const DOO_HTTP_GROUP: &str = "doo_http_group";

// HTTP Method Routes with function pointer
pub const DOO_HTTP_GET_FN: &str = "doo_http_get_fn";
pub const DOO_HTTP_POST_FN: &str = "doo_http_post_fn";
pub const DOO_HTTP_PUT_FN: &str = "doo_http_put_fn";
pub const DOO_HTTP_DELETE_FN: &str = "doo_http_delete_fn";
pub const DOO_HTTP_PATCH_FN: &str = "doo_http_patch_fn";

// HTTP Method Routes with middleware
pub const DOO_HTTP_GET_WITH_MIDDLEWARE: &str = "doo_http_get_with_middleware";
pub const DOO_HTTP_POST_WITH_MIDDLEWARE: &str = "doo_http_post_with_middleware";
pub const DOO_HTTP_PUT_WITH_MIDDLEWARE: &str = "doo_http_put_with_middleware";
pub const DOO_HTTP_DELETE_WITH_MIDDLEWARE: &str = "doo_http_delete_with_middleware";
pub const DOO_HTTP_PATCH_WITH_MIDDLEWARE: &str = "doo_http_patch_with_middleware";

// HTTP FFI symbol constants — used by codegen for metadata/auth/crud dispatch
pub const DOO_HTTP_AUTH: &str = "doo_http_auth";
pub const DOO_HTTP_CRUD: &str = "doo_http_crud";

// Default auth route paths — used when app.auth() is called with zero arguments.
// Single source of truth: referenced by route_transform for default injection.
pub const DEFAULT_AUTH_SIGNUP_PATH: &str = "/auth/register";
pub const DEFAULT_AUTH_LOGIN_PATH: &str = "/auth/login";

pub const DOO_HTTP_REGISTER_MIDDLEWARE: &str = "doo_http_register_middleware";
pub const DOO_HTTP_REGISTER_HANDLER_WITH_METADATA: &str = "doo_http_register_handler_with_metadata";
pub const DOO_HTTP_REGISTER_STRUCT_METADATA: &str = "doo_http_register_struct_metadata";
pub const DOO_HTTP_REGISTER_ENUM_METADATA: &str = "doo_http_register_enum_metadata";

// HTTP Serialization Helpers (doohttp_ prefix — compiled into codegen wrappers)
pub const DOOHTTP_POPULATE_STRUCT_FROM_REQUEST: &str = "doohttp_populate_struct_from_request";
pub const DOOHTTP_LAST_ERROR_STATUS: &str = "doohttp_last_error_status";
pub const DOOHTTP_LAST_ERROR_JSON: &str = "doohttp_last_error_json";
pub const DOOHTTP_ERROR_VARIANT_TO_STATUS: &str = "doohttp_error_variant_to_status";
pub const DOOHTTP_BUILD_RFC7807_ERROR: &str = "doohttp_build_rfc7807_error";
pub const DOOHTTP_SERIALIZE_STRUCT_TO_JSON: &str = "doohttp_serialize_struct_to_json";
pub const DOOHTTP_FORMAT_ERROR_AS_JSON: &str = "doohttp_format_error_as_json";

// ============================================================================
// Doo Database FFI (doo_ffi_db)
// ============================================================================

pub const DOO_DB_POSTGRES: &str = "doo_db_postgres";
pub const DOO_DB_FIND: &str = "doo_db_find";
pub const DOO_DB_FIND_ALL: &str = "doo_db_find_all";
pub const DOO_DB_INSERT: &str = "doo_db_insert";
pub const DOO_DB_UPDATE: &str = "doo_db_update";
pub const DOO_DB_DELETE: &str = "doo_db_delete";
pub const DOO_DB_RAW: &str = "doo_db_raw";
pub const DOO_DB_RAW_WITH_PARAMS: &str = "doo_db_raw_with_params";
pub const DOO_DB_QUERY: &str = "doo_db_query";
pub const DOO_DB_EXISTS: &str = "doo_db_exists";
pub const DOO_DB_RESULT_FREE: &str = "doo_db_result_free";
pub const DOO_DB_SERIALIZE_ENUM_ARRAY: &str = "doo_db_serialize_enum_array";
pub const DOO_DB_CONNECT_POSTGRES: &str = "doo_db_connect_postgres";
pub const DOO_DB_GET_GLOBAL: &str = "doo_db_get_global";
pub const DOO_DB_RAW_PARAM: &str = "doo_db_raw_param";
pub const DOO_DB_FREE_STRING: &str = "doo_db_free_string";
pub const DOO_DB_BATCH_QUERY: &str = "doo_db_batch_query";
pub const DOO_DB_BATCH_UPDATE: &str = "doo_db_batch_update";

// ============================================================================
// Doo Auth FFI (doo_ffi_auth)
// ============================================================================

pub const DOO_AUTH_HASH_PASSWORD: &str = "doo_auth_hash_password";
pub const DOO_AUTH_VERIFY_PASSWORD: &str = "doo_auth_verify_password";
pub const DOO_AUTH_SIGN_TOKEN: &str = "doo_auth_sign_token";
pub const DOO_AUTH_VERIFY_TOKEN: &str = "doo_auth_verify_token";
pub const DOO_AUTH_FREE_RESULT: &str = "doo_auth_free_result";
pub const DOO_AUTH_SIGN: &str = "doo_auth_sign";
pub const DOO_AUTH_VERIFY: &str = "doo_auth_verify";
pub const DOO_AUTH_FREE_STRING: &str = "doo_auth_free_string";

// ============================================================================
// Doo File FFI (doo_ffi_file)
// ============================================================================

pub const DOO_FILE_INIT: &str = "doo_file_init";
pub const DOO_FILE_READ: &str = "doo_file_read";
pub const DOO_FILE_WRITE: &str = "doo_file_write";
pub const DOO_FILE_APPEND: &str = "doo_file_append";
pub const DOO_FILE_DELETE: &str = "doo_file_delete";
pub const DOO_FILE_EXISTS: &str = "doo_file_exists";
pub const DOO_FILE_METADATA: &str = "doo_file_metadata";
pub const DOO_FILE_COPY: &str = "doo_file_copy";
pub const DOO_FILE_MOVE: &str = "doo_file_move";
pub const DOO_FILE_SIZE: &str = "doo_file_size";
pub const DOO_FILE_READ_LINES: &str = "doo_file_read_lines";
pub const DOO_FILE_MKDIR: &str = "doo_file_mkdir";
pub const DOO_FILE_MKDIR_ALL: &str = "doo_file_mkdir_all";
pub const DOO_FILE_RMDIR: &str = "doo_file_rmdir";
pub const DOO_FILE_RMDIR_ALL: &str = "doo_file_rmdir_all";
pub const DOO_FILE_LIST_DIR: &str = "doo_file_list_dir";
pub const DOO_FILE_IS_FILE: &str = "doo_file_is_file";
pub const DOO_FILE_IS_DIR: &str = "doo_file_is_dir";
pub const DOO_FILE_MODIFIED_TIME: &str = "doo_file_modified_time";
pub const DOO_FILE_FREE_RESULT: &str = "doo_file_free_result";

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

/// Connection method names that establish connections returning typed handles
pub const SELF_RETURNING_CONNECTORS: &[&str] = &[
    "connect",  // Database.connect() -> Database
    "Postgres", // Database.Postgres() -> Database
    "mysql",    // Database.mysql() -> Database
    "sqlite",   // Database.sqlite() -> Database
    "open",     // File.open() -> File
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
// Middleware Names — Used by doo_analysis route transform
// ============================================================================
// Codegen has its own local copy in packages/http.rs for dispatch.
// These are only needed by analysis (route_transform.rs).

/// JWT middleware function name (the Doo `Jwt()` function)
pub const DOO_JWT_FUNC_NAME: &str = "Jwt";
/// JWT middleware identifier — used by analysis for route transform
pub const MIDDLEWARE_JWT: &str = "Jwt";

// ============================================================================
// Doo WebSocket FFI (doo_ffi_websocket) - Single Source of Truth
// ============================================================================

// Route registration
pub const DOO_WS_ROUTE: &str = "doo_ws_route";
pub const DOO_WS_INIT: &str = "doo_ws_init";
pub const DOO_WS_CONFIG: &str = "doo_ws_config";
pub const DOO_WS_SHUTDOWN: &str = "doo_ws_shutdown";
pub const DOO_WS_ACTIVE_CONNECTIONS: &str = "doo_ws_active_connections";

// Connection operations
pub const DOO_WS_CONN_ID: &str = "doo_ws_conn_id";
pub const DOO_WS_CONN_EMIT: &str = "doo_ws_conn_emit";
pub const DOO_WS_CONN_EMIT_BINARY: &str = "doo_ws_conn_emit_binary";
pub const DOO_WS_CONN_JOIN: &str = "doo_ws_conn_join";
pub const DOO_WS_CONN_LEAVE: &str = "doo_ws_conn_leave";
pub const DOO_WS_CONN_CLOSE: &str = "doo_ws_conn_close";
pub const DOO_WS_CONN_IS_CLOSED: &str = "doo_ws_conn_is_closed";
pub const DOO_WS_CONN_ON: &str = "doo_ws_conn_on";
pub const DOO_WS_CONN_ON_CONNECT: &str = "doo_ws_conn_on_connect";
pub const DOO_WS_CONN_ON_DISCONNECT: &str = "doo_ws_conn_on_disconnect";
pub const DOO_WS_CONN_ON_ERROR: &str = "doo_ws_conn_on_error";

// WebSocket route check (used by HTTP server)
pub const DOO_WS_IS_WS_ROUTE: &str = "doo_ws_is_ws_route";

// Broadcast & room operations
pub const DOO_WS_BROADCAST: &str = "doo_ws_broadcast";
pub const DOO_WS_ROOM_EMIT: &str = "doo_ws_room_emit";

// ============================================================================
// Doo Process FFI (doo_ffi_process) - Single Source of Truth
// ============================================================================

// Synchronous command execution
pub const DOO_PROCESS_RUN: &str = "doo_process_run";
pub const DOO_PROCESS_OUTPUT: &str = "doo_process_output";

// Async spawn / handle
pub const DOO_PROCESS_SPAWN: &str = "doo_process_spawn";
pub const DOO_PROCESS_KILL: &str = "doo_process_kill";
pub const DOO_PROCESS_STATUS: &str = "doo_process_status";
pub const DOO_PROCESS_WAIT_OUTPUT: &str = "doo_process_wait_output";
pub const DOO_PROCESS_IS_RUNNING: &str = "doo_process_is_running";
pub const DOO_PROCESS_READ_STDOUT: &str = "doo_process_read_stdout";
pub const DOO_PROCESS_READ_STDERR: &str = "doo_process_read_stderr";

// Lifecycle
pub const DOO_PROCESS_SHUTDOWN: &str = "doo_process_shutdown";
pub const DOO_PROCESS_ACTIVE_COUNT: &str = "doo_process_active_count";

// ============================================================================
// Doo Git FFI (doo_ffi_git) — Native libgit2 operations
// ============================================================================

pub const DOO_GIT_INIT: &str = "doo_git_init";
pub const DOO_GIT_CLONE: &str = "doo_git_clone";
pub const DOO_GIT_COMMIT_ALL: &str = "doo_git_commit_all";
pub const DOO_GIT_PUSH: &str = "doo_git_push";
pub const DOO_GIT_PULL: &str = "doo_git_pull";
pub const DOO_GIT_IS_DIRTY: &str = "doo_git_is_dirty";
pub const DOO_GIT_STASH: &str = "doo_git_stash";
pub const DOO_GIT_STASH_POP: &str = "doo_git_stash_pop";
pub const DOO_GIT_HAS_REMOTE: &str = "doo_git_has_remote";
pub const DOO_GIT_HEAD_SHORT: &str = "doo_git_head_short";
pub const DOO_GIT_COMMIT_ALL_BG: &str = "doo_git_commit_all_bg";

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
// Doo HTTP Metrics FFI (doo_ffi_http) - Single Source of Truth
// ============================================================================

/// Enable Prometheus-compatible metrics endpoint on the server
pub const DOO_HTTP_METRICS: &str = "doo_http_metrics";

// ============================================================================
// Doo HTTP Client / Fetch FFI (doo_ffi_http) - Single Source of Truth
// ============================================================================

/// `Fetch(url, options?)` — Make an outbound HTTP request.
/// options: ObjectLit `{ method: "POST", body: "...", timeout: 30, headers: ["K: V"] }`
/// Returns JSON: `{"status":200,"body":"...","ok":true,"headers":{...}}`
pub const DOO_HTTP_FETCH: &str = "doo_http_fetch";

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
