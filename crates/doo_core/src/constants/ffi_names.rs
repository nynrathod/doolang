//! FFI Function Names - Single Source of Truth
//!
//! ALL FFI function name strings are centralized here. No hardcoded strings in codegen.
//!
//! ## Usage
//! ```
//! use doo_core::constants::ffi_names;
//! let malloc_fn = module.get_function(ffi_names::MALLOC)?;
//! ```

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

// Primitives
pub const DOO_JSON_WRITE_INT: &str = "doo_json_write_int";
pub const DOO_JSON_WRITE_FLOAT: &str = "doo_json_write_float";
pub const DOO_JSON_WRITE_BOOL: &str = "doo_json_write_bool";
pub const DOO_JSON_WRITE_STRING: &str = "doo_json_write_string";
pub const DOO_JSON_WRITE_NULL: &str = "doo_json_write_null";

// Reader API
pub const DOO_JSON_PARSE: &str = "doo_json_parse";

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
