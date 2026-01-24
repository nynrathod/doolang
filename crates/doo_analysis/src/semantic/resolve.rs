//! Name Resolver
//!
//! Resolves identifiers to their definitions.

use doo_core::Span;

/// Resolution error.
#[derive(Debug, Clone)]
pub struct ResolveError {
    pub message: String,
    pub span: Span,
}

/// Name resolver pass.
pub struct NameResolver {
    // Uses ScopeManager internally, could be merged with TypeChecker
    // or run as a separate pass before type checking.
}

impl NameResolver {
    pub fn new() -> Self {
        Self {}
    }
}
