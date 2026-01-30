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
//! - **Import Resolution**: Build import graph, detect circular imports
//! - **Cross-Module Resolution**: Resolve symbols across module boundaries
//! - **Exhaustiveness**: Check match expressions for missing patterns
//! - **Error Flow**: Track Result flows, ensure errors are handled
//! - **Method Resolution**: Resolve methods by receiver type (TASK-017)

pub mod decorators;
pub mod error_flow;
pub mod exhaustiveness;
pub mod resolve;
pub mod scope;
pub mod type_check;
pub mod visibility;

pub use decorators::{DecoratorError, DecoratorKind, DecoratorValidator};
pub use error_flow::{ErrorFlowChecker, ErrorFlowError, ErrorFlowErrorKind};
pub use exhaustiveness::{ExhaustivenessChecker, ExhaustivenessError, ExhaustivenessErrorKind};
pub use resolve::{
    CircularImportDetector,
    CircularImportError,
    // Cross-module resolution
    CrossModuleResolver,
    ImportEdge,
    ImportGraph,
    ImportItemKind,
    ImportKind,
    ImportStack,
    ImportedModule,
    // Method resolution (TASK-017)
    MethodResolver,
    MethodSignature,
    MethodTable,
    NameResolver,
    ResolveError,
    ResolvedMethod,
    ResolvedSymbol,
    SymbolDef,
    SymbolKindDef,
    SymbolTable,
};
pub use scope::{Scope, ScopeManager, Symbol, SymbolKind};
pub use type_check::{TypeChecker, TypeError, TypeErrorKind};
pub use visibility::{
    check_field_visibility, visibility_from_flag, FieldVisibilityChecker, FieldVisibilityError,
    Visibility, VisibilityChecker, VisibilityError,
};
