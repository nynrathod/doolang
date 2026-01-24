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

pub mod result;
pub mod string;
pub mod memory;
pub mod rfc7807;
pub mod errors;
pub mod validation;

pub use result::{DooResult, ResultTag};
pub use string::DooString;
pub use memory::{doo_alloc, doo_free};
pub use rfc7807::{Rfc7807Error, FieldError};
pub use errors::{DbErrorCode, AuthErrorCode};
pub use validation::{FieldDecorator, ValidationError, validate_field};
