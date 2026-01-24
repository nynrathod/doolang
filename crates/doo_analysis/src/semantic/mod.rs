//! Semantic Analysis
//!
//! Type checking, scope management, and semantic validation.
//!
//! ## Responsibilities
//!
//! - **Type Checking**: Verify type compatibility, infer types
//! - **Scope Management**: Track symbols per scope, handle imports
//! - **Validation**: Check declarations, mutability, decorators
//! - **Visibility**: Check pub access across modules
//! - **Decorators**: Validate decorator usage on fields

pub mod scope;
pub mod type_check;
pub mod resolve;
pub mod visibility;
pub mod decorators;

pub use scope::{ScopeManager, Scope, Symbol, SymbolKind};
pub use type_check::{TypeChecker, TypeError, TypeErrorKind};
pub use resolve::{NameResolver, ResolveError};
pub use visibility::{VisibilityChecker, Visibility, VisibilityError, visibility_from_flag};
pub use decorators::{DecoratorValidator, DecoratorError, DecoratorKind};
