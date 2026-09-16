//! Macro expansion engine for the Doo compiler.
//!
//! Provides the TokenStream API that macros receive and produce.
//! Macros see tokens only — never AST, HIR, THIR, or MIR.
//!
//! ## Architecture
//!
//! - `token_stream` — TokenStream, TokenTree, Token types + built-in tokenizer
//! - `registry` — MacroRegistry for loading and looking up macro-provider crates
//!
//! ## Macro Trait
//!
//! A macro receives a TokenStream and returns a TokenStream. The output
//! is re-parsed as ordinary Doolang by the expansion hook in `doo_driver`.
//!
//! ## Pipeline Placement
//!
//! Macro expansion runs between Parse (step 2) and Type Check (step 3).
//! The expansion hook lives in `doo_driver` because it needs AST access
//! (forbidden for `doo_macro` crate itself).

pub mod registry;
pub mod token_stream;

pub use registry::{MacroCrate, MacroCrateKind, MacroRegistry};
pub use token_stream::{
    Delimiter, Keyword, Literal, Punct, Spacing, Token, TokenKind, TokenStream, TokenTree,
};

use doo_core::span::FileId;
use doo_core::{Span, Symbol};

// ============================================================================
// Macro Context
// ============================================================================

/// Context passed to a macro during expansion.
///
/// Provides identifying information about the invocation site
/// so macros can produce appropriate diagnostics.
#[derive(Debug, Clone)]
pub struct MacroContext {
    /// Name of the crate containing the macro being expanded.
    pub crate_name: Symbol,
    /// Name of the macro being expanded.
    pub macro_name: Symbol,
    /// Source file where the macro was invoked.
    pub source_file: FileId,
    /// Span of the macro invocation in the source.
    pub invocation_span: Span,
}

// ============================================================================
// Macro Trait
// ============================================================================

/// Trait implemented by all macro providers.
///
/// A macro receives a TokenStream (the annotated item or macro arguments)
/// and returns a TokenStream (the expanded code). The output is re-parsed
/// as ordinary Doolang by the expansion hook.
///
/// Macros have NO access to AST, HIR, THIR, MIR, or LLVM IR.
/// They operate on tokens only — a narrow, stable contract.
pub trait Macro {
    /// Expand the input token stream into an output token stream.
    fn expand(&self, input: TokenStream, ctx: &MacroContext) -> Result<TokenStream, MacroError>;
}

// ============================================================================
// Macro Error
// ============================================================================

/// Error from macro expansion.
#[derive(Debug, Clone)]
pub enum MacroError {
    /// The macro encountered invalid input.
    InvalidInput(String),
    /// The macro failed to produce valid output.
    ExpansionFailed(String),
    /// The macro attempted an unsupported operation.
    Unsupported(String),
}

impl std::fmt::Display for MacroError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(msg) => write!(f, "invalid macro input: {}", msg),
            Self::ExpansionFailed(msg) => write!(f, "macro expansion failed: {}", msg),
            Self::Unsupported(msg) => write!(f, "unsupported macro operation: {}", msg),
        }
    }
}

impl std::error::Error for MacroError {}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_macro_context_creation() {
        let ctx = MacroContext {
            crate_name: Symbol::intern("doo_derive_json"),
            macro_name: Symbol::intern("DeriveJson"),
            source_file: FileId::new(0),
            invocation_span: Span::new(0, 10),
        };
        assert_eq!(ctx.macro_name.resolve(), "DeriveJson");
    }

    #[test]
    fn test_macro_error_display() {
        let err = MacroError::InvalidInput("expected struct".to_string());
        assert!(format!("{}", err).contains("expected struct"));
    }
}
