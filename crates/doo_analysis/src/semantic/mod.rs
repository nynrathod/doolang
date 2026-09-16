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

pub mod error_flow;
pub mod exhaustiveness;
pub mod resolve;
pub mod scope;
pub mod type_check;
pub mod visibility;

pub use error_flow::{ErrorFlowChecker, ErrorFlowError, ErrorFlowErrorKind};
pub use exhaustiveness::{ExhaustivenessChecker, ExhaustivenessError, ExhaustivenessErrorKind};
pub use resolve::{
    CircularImportDetector, CircularImportError, CrossModuleResolver, ImportEdge, ImportGraph,
    ImportItemKind, ImportKind, ImportStack, ImportedModule, MethodResolver, MethodSignature,
    MethodTable, NameResolver, ResolveError, ResolvedMethod, ResolvedSymbol, SymbolDef,
    SymbolKindDef, SymbolTable,
};
// Phase 23: Module-level scope resolution
pub use scope::{
    ModuleImport, ModuleScope, Scope, ScopeError, ScopeItem, ScopeManager, ScopeResolver,
    ScopeResolverError, Symbol, SymbolKind, Visibility,
};
pub use type_check::{TypeChecker, TypeError, TypeErrorKind};
pub use visibility::{
    check_field_visibility, is_public, visibility_from_flag, visibility_from_name,
    FieldVisibilityChecker, FieldVisibilityError, VisibilityChecker, VisibilityError,
};
