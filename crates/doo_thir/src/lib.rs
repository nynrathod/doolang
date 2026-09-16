//! # Doo THIR (Typed High-level Intermediate Representation)
//!
//! THIR is the IR where every expression carries its fully resolved `TypeId`
//! and every method call contains its explicit resolved trait/impl choice.
//!
//! ## Purpose
//!
//! - Closes the gap between "resolved once" and "re-derived every pass"
//! - Ownership/Borrow/Move/Drop analysis reads THIR only — never AST or raw HIR
//! - Prevents repeated trait resolution overhead during deep analysis

pub mod expr;
pub mod item;
pub mod lower;
pub mod pattern;
pub mod solve;
pub mod stmt;
pub mod types;

pub use expr::*;
pub use item::*;
pub use lower::ThirLoweringContext;
pub use pattern::*;
pub use solve::{TraitImpl, TraitSolver};
pub use stmt::*;
pub use types::*;
