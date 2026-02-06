//! FFI Function Names & Built-in Module Names - Single Source of Truth
//!
//! ALL FFI function name strings and built-in module names are centralized here.
//! No hardcoded strings in codegen, type checker, or anywhere else.
//!
//! ## Usage
//! ```
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
];

/// Check if a name is a built-in module
#[inline]
pub fn is_builtin_module(name: &str) -> bool {
    BUILTIN_MODULES.contains(&name)
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

// ============================================================================
// Doo Core Runtime Functions
// ============================================================================

pub const DOO_ALLOC: &str = "doo_alloc";
pub const DOO_FREE: &str = "doo_free";
pub const DOO_REALLOC: &str = "doo_realloc";

// ============================================================================
// Doo JSON FFI (doo_ffi_json)
// ============================================================================

// Writer API
pub const DOO_JSON_WRITER_NEW: &str = "doo_json_writer_new";
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

// ============================================================================
// Doo HTTP FFI (doo_ffi_http)
// ============================================================================

pub const DOO_HTTP_SERVER_NEW: &str = "doo_http_server_new";
pub const DOO_HTTP_SERVER_LISTEN: &str = "doo_http_server_listen";
pub const DOO_HTTP_REGISTER_ROUTE: &str = "doo_http_register_route";
pub const DOO_HTTP_REGISTER_WITH_MIDDLEWARE: &str = "doo_http_register_with_middleware";
pub const DOO_HTTP_GROUP: &str = "doo_http_group";
pub const DOO_HTTP_CORS: &str = "doo_http_cors";
pub const DOO_HTTP_RATE_LIMIT: &str = "doo_http_rate_limit";

// Request/Response
pub const DOO_HTTP_REQ_GET_HEADER: &str = "doo_http_req_get_header";
pub const DOO_HTTP_REQ_GET_BODY: &str = "doo_http_req_get_body";
pub const DOO_HTTP_REQ_GET_PARAM: &str = "doo_http_req_get_param";
pub const DOO_HTTP_REQ_GET_QUERY: &str = "doo_http_req_get_query";

pub const DOO_HTTP_RES_SET_STATUS: &str = "doo_http_res_set_status";
pub const DOO_HTTP_RES_SET_HEADER: &str = "doo_http_res_set_header";
pub const DOO_HTTP_RES_SET_BODY: &str = "doo_http_res_set_body";
pub const DOO_HTTP_RES_JSON: &str = "doo_http_res_json";

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

// ============================================================================
// Doo Auth FFI (doo_ffi_auth)
// ============================================================================

pub const DOO_AUTH_HASH_PASSWORD: &str = "doo_auth_hash_password";
pub const DOO_AUTH_VERIFY_PASSWORD: &str = "doo_auth_verify_password";
pub const DOO_AUTH_SIGN_TOKEN: &str = "doo_auth_sign_token";
pub const DOO_AUTH_VERIFY_TOKEN: &str = "doo_auth_verify_token";
pub const DOO_AUTH_FREE_RESULT: &str = "doo_auth_free_result";

// ============================================================================
// Doo File FFI (doo_ffi_file)
// ============================================================================

pub const DOO_FILE_READ: &str = "doo_file_read";
pub const DOO_FILE_WRITE: &str = "doo_file_write";
pub const DOO_FILE_APPEND: &str = "doo_file_append";
pub const DOO_FILE_DELETE: &str = "doo_file_delete";
pub const DOO_FILE_EXISTS: &str = "doo_file_exists";
pub const DOO_FILE_METADATA: &str = "doo_file_metadata";

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
// Collection Helpers (if FFI-based)
// ============================================================================

pub const DOO_ARRAY_CREATE: &str = "doo_array_create";
pub const DOO_ARRAY_CREATE_WITH_CAP: &str = "doo_array_create_with_cap";
pub const DOO_ARRAY_PUSH: &str = "doo_array_push";
pub const DOO_ARRAY_FREE: &str = "doo_array_free";

pub const DOO_MAP_CREATE: &str = "doo_map_create";
pub const DOO_MAP_SET: &str = "doo_map_set";
pub const DOO_MAP_GET: &str = "doo_map_get";
pub const DOO_MAP_FREE: &str = "doo_map_free";

pub const DOO_STRING_CREATE: &str = "doo_string_create";
pub const DOO_STRING_FREE: &str = "doo_string_free";

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
    "postgres", // Database.postgres() -> Database
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
