//! Macro Registry — loads and manages macro-provider crates.

use crate::{Macro, MacroContext, MacroError, TokenStream};
use doo_core::Symbol;
use rustc_hash::FxHashMap;
use std::path::PathBuf;

// ============================================================================
// MacroCrate
// ============================================================================

/// A macro-provider package declared in doo.toml with `[lib] kind = "macro"`.
#[derive(Debug, Clone)]
pub struct MacroCrate {
    /// Package name.
    pub name: Symbol,
    /// Path to the crate (for `path` dependencies).
    pub path: PathBuf,
    /// Execution model for the macro crate.
    pub kind: MacroCrateKind,
}

/// How a macro crate is executed during expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroCrateKind {
    /// Trusted official macro — runs in-process as a dylib.
    InProcess,
    /// Untrusted community macro — runs sandboxed (WASM or subprocess).
    Sandboxed,
}

// ============================================================================
// MacroRegistry
// ============================================================================

/// Registry of available macros, indexed by name.
///
/// Macros are registered by their decorator name (e.g., "table", "email").
/// When the expansion hook encounters `@decorator(...)`, it looks up
/// the decorator name here.
pub struct MacroRegistry {
    /// Map of macro name → implementation.
    macros: FxHashMap<Symbol, Box<dyn Macro>>,
    /// Macro crates that were loaded.
    macro_crates: Vec<MacroCrate>,
}

impl MacroRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            macros: FxHashMap::default(),
            macro_crates: Vec::new(),
        }
    }

    /// Discover macro-provider crates from the package graph.
    ///
    /// Scans all dependencies for `[lib] kind = "macro"`. For each macro
    /// crate found, loads it (InProcess as dylib, Sandboxed as WASM) and
    /// registers exported macros by name.
    ///
    /// Currently returns an empty registry — dynamic crate loading
    /// requires a build system integration that is not yet implemented.
    /// Macros are registered programmatically via `register()`.
    pub fn discover(package_names: &[String]) -> Self {
        let mut registry = Self::new();

        for _name in package_names {
            // Check if this package is a macro crate
            // In the future, this would load the crate's doo.toml
            // and check for [lib] kind = "macro"
            // No dynamic loading yet — macros registered via register()
        }

        registry
    }

    /// Register a macro by name.
    ///
    /// Called by the compiler to register built-in macros, or by
    /// the macro loader after loading a dylib macro crate.
    pub fn register(&mut self, name: &str, mac: Box<dyn Macro>) {
        let sym = Symbol::intern(name);
        self.macros.insert(sym, mac);
    }

    /// Register a macro crate.
    pub fn register_crate(&mut self, mac_crate: MacroCrate) {
        self.macro_crates.push(mac_crate);
    }

    /// Look up a macro by name.
    pub fn get(&self, name: Symbol) -> Option<&dyn Macro> {
        self.macros.get(&name).map(|m| m.as_ref())
    }

    /// Check if a macro is registered.
    pub fn has(&self, name: Symbol) -> bool {
        self.macros.contains_key(&name)
    }

    /// Get all registered macro names.
    pub fn names(&self) -> impl Iterator<Item = Symbol> + '_ {
        self.macros.keys().copied()
    }

    /// Number of registered macros.
    pub fn len(&self) -> usize {
        self.macros.len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.macros.is_empty()
    }

    /// Get all loaded macro crates.
    pub fn crates(&self) -> &[MacroCrate] {
        &self.macro_crates
    }
}

impl Default for MacroRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for MacroRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacroRegistry")
            .field("macro_count", &self.macros.len())
            .field("crate_count", &self.macro_crates.len())
            .finish()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MacroContext, MacroError, TokenStream};
    use doo_core::span::FileId;
    use doo_core::{Span, Symbol};

    /// A test macro that returns its input unchanged (identity macro).
    struct IdentityMacro;

    impl Macro for IdentityMacro {
        fn expand(
            &self,
            input: TokenStream,
            _ctx: &MacroContext,
        ) -> Result<TokenStream, MacroError> {
            Ok(input)
        }
    }

    /// A test macro that returns an empty stream.
    struct EmptyMacro;

    impl Macro for EmptyMacro {
        fn expand(
            &self,
            _input: TokenStream,
            _ctx: &MacroContext,
        ) -> Result<TokenStream, MacroError> {
            Ok(TokenStream::new())
        }
    }

    #[test]
    fn test_empty_registry() {
        let registry = MacroRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_register_and_lookup() {
        let mut registry = MacroRegistry::new();
        registry.register("table", Box::new(IdentityMacro));

        let name = Symbol::intern("table");
        assert!(registry.has(name));
        assert!(registry.get(name).is_some());
    }

    #[test]
    fn test_lookup_missing() {
        let registry = MacroRegistry::new();
        let name = Symbol::intern("nonexistent");
        assert!(!registry.has(name));
        assert!(registry.get(name).is_none());
    }

    #[test]
    fn test_register_multiple() {
        let mut registry = MacroRegistry::new();
        registry.register("table", Box::new(IdentityMacro));
        registry.register("email", Box::new(EmptyMacro));
        registry.register("primary", Box::new(IdentityMacro));

        assert_eq!(registry.len(), 3);
        assert!(registry.has(Symbol::intern("table")));
        assert!(registry.has(Symbol::intern("email")));
        assert!(registry.has(Symbol::intern("primary")));
    }

    #[test]
    fn test_discover_empty() {
        let packages: Vec<String> = vec![];
        let registry = MacroRegistry::discover(&packages);
        assert!(registry.is_empty());
    }

    #[test]
    fn test_register_crate() {
        let mut registry = MacroRegistry::new();
        let mac_crate = MacroCrate {
            name: Symbol::intern("doo_derive_json"),
            path: PathBuf::from("../doo-derive-json"),
            kind: MacroCrateKind::InProcess,
        };
        registry.register_crate(mac_crate);

        assert_eq!(registry.crates().len(), 1);
    }

    #[test]
    fn test_identity_macro() {
        let macro_impl = IdentityMacro;
        let input = TokenStream::from_str("struct User { Name: Str }");
        let ctx = MacroContext {
            crate_name: Symbol::intern("test"),
            macro_name: Symbol::intern("table"),
            source_file: FileId::new(0),
            invocation_span: Span::new(0, 10),
        };
        let output = macro_impl.expand(input, &ctx).unwrap();
        assert!(!output.is_empty());
    }

    #[test]
    fn test_empty_macro() {
        let macro_impl = EmptyMacro;
        let input = TokenStream::from_str("struct User");
        let ctx = MacroContext {
            crate_name: Symbol::intern("test"),
            macro_name: Symbol::intern("email"),
            source_file: FileId::new(0),
            invocation_span: Span::new(0, 5),
        };
        let output = macro_impl.expand(input, &ctx).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn test_names_iterator() {
        let mut registry = MacroRegistry::new();
        registry.register("table", Box::new(IdentityMacro));
        registry.register("email", Box::new(EmptyMacro));

        let names: Vec<_> = registry.names().collect();
        assert_eq!(names.len(), 2);
    }
}
