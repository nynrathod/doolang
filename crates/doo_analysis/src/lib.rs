//! # Doo Analysis
//!
//! Semantic analysis passes for the Doo compiler.
//!
//! ## Modules
//!
//! - `semantic` - Type checking, scope management, name resolution
//! - `ownership` - Ownership tracking and auto-clone insertion
//! - `borrow` - Borrow checking for concurrent mutable access
//! - `types` - Type inference and compatibility

pub mod ownership;
pub mod borrow;
pub mod semantic;
pub mod types;

pub use ownership::{OwnershipAnalyzer, OwnershipError, Decision, DropInserter};
pub use borrow::{BorrowChecker, BorrowError, BorrowErrorKind};
pub use semantic::{ScopeManager, TypeChecker, NameResolver};
pub use types::{TypeInference, TypeCompat};

