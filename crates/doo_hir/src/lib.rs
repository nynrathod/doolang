//! # Doo HIR (High-Level Intermediate Representation)
//!
//! The HIR is a desugared, typed representation between the AST and analysis passes.
//!
//! ## Purpose
//!
//! - Simplifies AST constructs through desugaring
//! - Provides explicit type annotations on all nodes
//! - Prepares for ownership analysis with ownership placeholders
//! - Enables simpler semantic analysis passes
//!
//! ## Desugaring Examples
//!
//! - `x += 1` → `x = x + 1`
//! - `x++` → `x = x + 1`
//! - Range `1..10` → `Range::new(1, 10, false)`

pub mod types;
pub mod lowering;
pub mod visitor;

pub use types::*;
pub use lowering::Lower;
pub use visitor::{HirVisitor, HirVisitorMut};
