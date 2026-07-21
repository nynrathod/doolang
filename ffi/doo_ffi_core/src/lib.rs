//! # Doo FFI Core
//!
//! Shared foundation for all FFI crates. Contains:
//!
//! ## Core Runtime (Language Fundamentals)
//! - `DooResult` / `ResultTag` — The ONE result type for all FFI calls
//! - `DooString` — The ONE string type for FFI
//! - `casts` — Type casting (Str↔Int, Str↔Float)
//! - `memory` — Allocation/free (doo_alloc, doo_free, doo_clone)
//! - `config` — Environment variable access
//! - `debug` — Debug output helpers
//! - `macros` — Shared FFI macros
//!
//! ## Shared Infrastructure (Used by Multiple Packages)
//! These modules are shared across HTTP, Auth, DB, and JSON packages.
//! They live here to avoid circular dependencies between packages.
//! - `rfc7807` — RFC 7807 structured error format (used by HTTP, JSON, DB, Auth)
//! - `cookies` — Cookie management (used by HTTP, Auth)
//! - `validation` — Field validation (used by HTTP request validation)
//! - `errors` — Error codes (Auth, DB)
//! - `helpers` — Common FFI helpers (string conversion, result builders)

#[macro_use]
pub mod macros;
pub mod case;
pub mod casts;
pub mod config;
pub mod constants;
pub mod cookies;
pub mod debug;
pub mod errors;
pub mod ffi_bridge;
pub mod helpers;
pub mod memory;
pub mod result;
pub mod rfc7807;
pub mod string;
pub mod validation;

pub use case::{to_pascal_case, to_snake_case};
pub use casts::{
    doo_cast_bool_to_str, doo_cast_float_to_str, doo_cast_int_to_str, doo_cast_str_to_float,
    doo_cast_str_to_int,
};
pub use errors::{AuthErrorCode, DbErrorCode};
pub use helpers::{
    c_to_string, c_to_string_lossy, make_err, make_ok_bool, make_ok_int, make_ok_string,
    make_ok_void, make_panic_err, safe_ffi, string_to_c,
};
pub use memory::{
    doo_alloc, doo_alloc_array, doo_alloc_empty_string, doo_alloc_map, doo_alloc_string,
    doo_clone_string, doo_free, doo_realloc, HEADER_SIZE,
};
pub use result::{DooResult, ResultTag};
pub use rfc7807::{
    error_type_for_status, title_for_status, FieldError, ParameterError, Rfc7807Error,
};
pub use string::DooString;
pub use validation::{validate_field, FieldDecorator, ValidationError};
