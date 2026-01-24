//! Type Analysis
//!
//! Type inference and compatibility checking.
//!
//! This module works with doo_core types for centralized type definitions.

pub mod infer;
pub mod compat;

pub use infer::TypeInference;
pub use compat::TypeCompat;
