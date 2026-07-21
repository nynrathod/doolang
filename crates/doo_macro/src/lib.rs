//! # Doo Macro — Compile-Time Macro Expansion Engine
//!
//! ## Architecture
//!
//! The macro engine is the single extension point in the Doo compiler.
//! It runs **once**, between Parse and HIR lowering:
//!
//! ```text
//! Source → Lex → Parse → MACRO EXPANSION → HIR → Analysis → MIR → Codegen
//! ```
//!
//! ## Design Constraints (immutable)
//!
//! 1. **TokenStream → TokenStream only.** Macros receive a token stream and
//!    return a token stream. They have no access to AST, HIR, MIR, types,
//!    ownership decisions, or LLVM IR.
//!
//! 2. **No compiler hooks.** There is no `after_hir`, `before_mir`, or
//!    `codegen_call` hook. One hook only: post-parse, pre-typecheck.
//!
//! 3. **Re-parsed as ordinary Doolang.** Macro output is re-parsed and goes
//!    through the exact same Type Check → Ownership → Borrow → Move → Drop
//!    pipeline as hand-written code.
//!
//! 4. **Deterministic.** Fixed expansion order (declaration order).
//!    Inputs/outputs are plain, versioned tokens.
//!
//! ## Extension Model
//!
//! Macros are the **Level 3** extension mechanism (see Memory Model Master,
//! Part VIII). Only features that genuinely need new compile-time syntax or
//! `@attr(...)` meaning should use macros. Everything else is a Level 1
//! (normal package via traits/generics) or Level 2 (FFI-backed package).
//!
//! ## Crate Status
//!
//! This crate is a **Phase 0 skeleton**. The macro expansion engine will be
//! implemented in Phase 6 after the compiler core is purified (Phases 1–5).
//! For now, it provides:
//!
//! - The `MacroExpander` trait (stable interface for macro-provider crates)
//! - A no-op expansion pass placeholder
//! - Module structure for the future expansion engine

// ============================================================================
// Placeholder Types (will be replaced with doo_frontend types in Phase 6)
// ============================================================================

/// Placeholder for the compiler's token stream type.
///
/// In Phase 6, this will be replaced with the canonical token stream type
/// from `doo_frontend`. For Phase 0, this is an opaque placeholder that
/// allows the crate to compile without depending on `doo_frontend`.
#[derive(Debug, Clone, Default)]
pub struct TokenStream {
    _private: (),
}

impl TokenStream {
    /// Create an empty token stream.
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Check if the token stream is empty.
    pub fn is_empty(&self) -> bool {
        true
    }
}

// ============================================================================
// Macro Expansion Interface
// ============================================================================

/// The single stable interface for macro-provider crates.
///
/// A macro crate implements this trait and is registered via its
/// `[lib] kind = "macro"` manifest flag. The compiler loads the crate,
/// calls `expand()` with the token stream, and re-parses the result.
///
/// ## Contract
///
/// - **Input**: A `TokenStream` representing the item being decorated (e.g.,
///   a struct with `@table`, a field with `@email`).
/// - **Output**: A `TokenStream` that replaces the original tokens. This
///   output must be valid Doolang syntax — it will be re-parsed by the
///   normal parser and go through the full pipeline.
/// - **Purity**: The macro MUST NOT read files, access the network, or
///   depend on non-deterministic state. It is a pure function from tokens
///   to tokens.
pub trait MacroExpander {
    /// Expand a token stream.
    ///
    /// Called once per decorated item during the macro expansion pass.
    /// The returned token stream replaces the original item in the AST.
    fn expand(&self, tokens: &TokenStream) -> TokenStream;

    /// Return the macro crate name (for diagnostics).
    fn name(&self) -> &'static str;
}

// ============================================================================
// Expansion Pass
// ============================================================================

/// Run macro expansion on a parsed program.
///
/// This is the single hook between Parse and HIR lowering. It walks the
/// token stream, identifies items with decorators, dispatches to the
/// appropriate macro-provider crate, and replaces the decorated tokens
/// with the expanded output.
///
/// ## Current Status (Phase 0)
///
/// This is a **no-op pass**. It returns the token stream unchanged.
/// The expansion engine will be implemented in Phase 6.
pub fn expand_macros(tokens: TokenStream) -> TokenStream {
    // Phase 0: No-op pass. Macro expansion engine will be implemented
    // in Phase 6 after the compiler core is purified.
    tokens
}

// ============================================================================
// Module Declarations (future)
// ============================================================================

// The following modules will be added in Phase 6:
//
// - `registry.rs` — Macro crate discovery and loading
// - `dispatcher.rs` — Decorator name → macro crate routing
// - `sandbox.rs` — Secure execution for untrusted community macros
// - `cache.rs` — Incremental caching of macro expansions

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the no-op pass returns tokens unchanged.
    #[test]
    fn test_noop_pass_identity() {
        // In Phase 0, expand_macros is the identity function.
        // This test validates the skeleton compiles and links correctly.
        let tokens = TokenStream::new();
        let result = expand_macros(tokens);
        // TokenStream from doo_frontend: verify we can construct and pass through
        assert!(result.is_empty());
    }
}
