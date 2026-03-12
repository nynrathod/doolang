//! AST Transformation Pass
//!
//! Transforms high-level DSL constructs into simpler forms before lowering to HIR.
//!
//! ## Transformations
//!
//! - `route_transform` - Flattens `app.group()` into individual routes
//! - Extracts inline closures into named functions

pub mod route_transform;

pub use route_transform::{transform_route_groups, transform_inline_closures};
