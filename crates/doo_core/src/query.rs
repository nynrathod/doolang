//! Type Context (TyCtxt) — The central compilation context.
//!
//! Instead of a complex query database, Doo uses a `TyCtxt` struct that holds
//! references to the global, immutable compilation state (Arena, Interner,
//! TypeRegistry). This is passed by reference to every compiler pass.
//!
//! ## Design (Architecture Part VI)
//!
//! - **Zero-Cost**: Passing `&TyCtxt` is a single pointer copy.
//! - **Immutable**: The context is read-only, preventing passes from mutating global state.
//! - **Future-Proof**: When incremental compilation (Phase 45) is added, this struct
//!   can be wrapped in a query engine (like Salsa) without rewriting the pass logic.

use crate::arena::CompilerArena;
use crate::intern::Interner;
use crate::types::registry::TypeRegistry;

/// The central compilation context.
///
/// Contains references to all global, immutable state required during compilation.
/// Every compiler pass (Parser, HIR, THIR, MIR) receives a `&TyCtxt` to access
/// shared resources.
pub struct TyCtxt<'tcx> {
    /// The arena allocator for AST/HIR/MIR nodes.
    pub arena: &'tcx CompilerArena,

    /// The global string interner.
    pub interner: &'tcx Interner,

    /// The central type registry (Single Source of Truth for types).
    pub type_registry: &'tcx TypeRegistry,
}

impl<'tcx> TyCtxt<'tcx> {
    /// Create a new type context.
    #[inline]
    pub fn new(
        arena: &'tcx CompilerArena,
        interner: &'tcx Interner,
        type_registry: &'tcx TypeRegistry,
    ) -> Self {
        Self {
            arena,
            interner,
            type_registry,
        }
    }

    /// Intern a string into the global interner.
    #[inline]
    pub fn intern(&self, s: &str) -> crate::symbol::Symbol {
        self.interner.intern(s)
    }

    /// Resolve a symbol back to its string.
    #[inline]
    pub fn resolve(&self, sym: crate::symbol::Symbol) -> &'static str {
        crate::intern::resolve(sym)
    }
}
