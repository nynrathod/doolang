//! # Doo Analysis
//!
//! Semantic analysis passes for the Doo compiler.
//!
//! ## Modules
//!
//! - `semantic` - Type checking, scope management, name resolution, import cycle detection
//! - `ownership` - Ownership tracking and auto-clone insertion
//! - `borrow` - Borrow checking for concurrent mutable access
//! - `types` - Type inference and compatibility
//! - `transform` - AST transformations (route groups, inline closures)

pub mod borrow;
pub mod ownership;
pub mod semantic;
pub mod transform;
pub mod types;

pub use borrow::{BorrowChecker, BorrowError, BorrowErrorKind};
pub use ownership::{
    Decision, DropInserter, OwnershipAnalyzer, OwnershipError, OwnershipResults, UseLocation,
};
pub use semantic::{
    CircularImportDetector,
    CircularImportError,
    // Cross-module resolution
    CrossModuleResolver,
    // Error flow analysis
    ErrorFlowChecker,
    ErrorFlowError,
    ErrorFlowErrorKind,
    // Exhaustiveness checking
    ExhaustivenessChecker,
    ExhaustivenessError,
    ExhaustivenessErrorKind,
    ImportGraph,
    ImportItemKind,
    ImportKind,
    ImportStack,
    ImportedModule,
    NameResolver,
    ResolvedSymbol,
    ScopeManager,
    SymbolTable,
    // Type checking
    TypeChecker,
    TypeError,
    TypeErrorKind,
};
pub use types::{ClosureContext, InferenceError, TypeCompat, TypeInference};
