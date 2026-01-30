//! # Doo FFI Core
//!
//! Single source of truth for ALL FFI types.
//!
//! ## Core Types
//!
//! - `DooResult` - The ONE result type for all FFI calls
//! - `DooString` - The ONE string type for FFI
//! - `DooValue` - Generic value wrapper
//! - `Rfc7807Error` - RFC 7807 error format

pub mod casts;
pub mod errors;
pub mod json;
pub mod memory;
pub mod result;
pub mod rfc7807;
pub mod string;
pub mod validation;

pub use casts::{
    doo_cast_bool_to_str, doo_cast_float_to_str, doo_cast_int_to_str, doo_cast_str_to_float,
    doo_cast_str_to_int,
};
pub use errors::{AuthErrorCode, DbErrorCode};
pub use json::*;
pub use memory::{
    doo_alloc, doo_alloc_array, doo_alloc_empty_string, doo_alloc_map, doo_alloc_string,
    doo_clone_string, doo_free, doo_realloc, HEADER_SIZE,
};
pub use result::{DooResult, ResultTag};
pub use rfc7807::{FieldError, Rfc7807Error};
pub use string::DooString;
pub use validation::{validate_field, FieldDecorator, ValidationError};
