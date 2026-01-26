//! Type Analysis
//!
//! Type inference and compatibility checking.
//!
//! This module works with doo_core types for centralized type definitions.

pub mod compat;
pub mod infer;

pub use compat::TypeCompat;
pub use infer::{ClosureContext, InferenceError, TypeInference};
