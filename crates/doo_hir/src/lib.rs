//! # Doo HIR (High-Level Intermediate Representation)
//!
//! The HIR is a desugared, typed representation between the AST and analysis passes.
//!
//! ## Purpose
//!
//! - Simplifies AST constructs through desugaring
//! - Provides explicit type annotations on all nodes via `TypeId`
//! - Prepares for ownership analysis with ownership placeholders
//! - Enables simpler semantic analysis passes
//!
//! ## Desugaring Examples
//!
//! - `x += 1` → `x = x + 1`
//! - `x++` → `x = x + 1`
//! - Range `1..10` → `Range::new(1, 10, false)`

pub mod lowering;
pub mod monomorphize;
pub mod types;
pub mod visitor;

pub use lowering::Lower;
pub use monomorphize::Monomorphizer;
pub use types::*;
pub use visitor::{walk_expr, walk_pattern, walk_stmt, HirVisitor, HirVisitorMut, WalkHir};
