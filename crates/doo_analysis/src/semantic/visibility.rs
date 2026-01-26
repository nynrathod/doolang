//! Visibility Checker
//!
//! Validates that access to non-public items respects visibility rules.
//!
//! ## Rules:
//!
//! - `pub` items are accessible from any module
//! - Non-`pub` items are only accessible within the same module
//! - Struct fields follow the struct's visibility by default
//! - Enum variants are always accessible if the enum is accessible

use doo_core::Span;
use std::collections::HashMap;

/// Visibility of an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// Public - accessible from any module.
    Public,
    /// Private - only accessible within the same module.
    Private,
}

impl Default for Visibility {
    fn default() -> Self {
        Self::Private
    }
}

/// A symbol with its visibility and location.
#[derive(Debug, Clone)]
pub struct VisibleSymbol {
    /// Name of the symbol.
    pub name: String,
    /// Visibility level.
    pub visibility: Visibility,
    /// Module where the symbol is defined.
    pub module_path: String,
    /// Span of the definition.
    pub span: Span,
}

/// Visibility error.
#[derive(Debug, Clone)]
pub struct VisibilityError {
    /// The symbol being accessed.
    pub symbol: String,
    /// Module where the symbol is defined.
    pub defined_in: String,
    /// Module where the access occurs.
    pub accessed_from: String,
    /// Span of the access.
    pub span: Span,
}

impl std::fmt::Display for VisibilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot access private symbol `{}` from module `{}` (defined in `{}`)",
            self.symbol, self.accessed_from, self.defined_in
        )
    }
}

impl std::error::Error for VisibilityError {}

/// Visibility checker.
pub struct VisibilityChecker {
    /// Map of symbol name to visibility info.
    symbols: HashMap<String, VisibleSymbol>,
    /// Current module path being analyzed.
    current_module: String,
}

impl VisibilityChecker {
    /// Create a new visibility checker.
    pub fn new() -> Self {
        Self {
            symbols: HashMap::new(),
            current_module: String::new(),
        }
    }

    /// Set the current module being analyzed.
    pub fn set_current_module(&mut self, module_path: &str) {
        self.current_module = module_path.to_string();
    }

    /// Register a symbol with its visibility.
    pub fn register_symbol(
        &mut self,
        name: &str,
        visibility: Visibility,
        module_path: &str,
        span: Span,
    ) {
        self.symbols.insert(
            name.to_string(),
            VisibleSymbol {
                name: name.to_string(),
                visibility,
                module_path: module_path.to_string(),
                span,
            },
        );
    }

    /// Register a public symbol.
    pub fn register_public(&mut self, name: &str, module_path: &str, span: Span) {
        self.register_symbol(name, Visibility::Public, module_path, span);
    }

    /// Register a private symbol.
    pub fn register_private(&mut self, name: &str, module_path: &str, span: Span) {
        self.register_symbol(name, Visibility::Private, module_path, span);
    }

    /// Check if an access is allowed.
    pub fn check_access(
        &self,
        symbol_name: &str,
        access_span: Span,
    ) -> Result<(), VisibilityError> {
        if let Some(symbol) = self.symbols.get(symbol_name) {
            // Public symbols are always accessible
            if symbol.visibility == Visibility::Public {
                return Ok(());
            }

            // Private symbols are only accessible in the same module
            if symbol.module_path == self.current_module {
                return Ok(());
            }

            // Cross-module access to private symbol is an error
            return Err(VisibilityError {
                symbol: symbol_name.to_string(),
                defined_in: symbol.module_path.clone(),
                accessed_from: self.current_module.clone(),
                span: access_span,
            });
        }

        // Symbol not found - let name resolution handle this error
        Ok(())
    }

    /// Check if a symbol is accessible from a given module.
    pub fn is_accessible(&self, symbol_name: &str, from_module: &str) -> bool {
        if let Some(symbol) = self.symbols.get(symbol_name) {
            symbol.visibility == Visibility::Public || symbol.module_path == from_module
        } else {
            // Symbol not registered - assume accessible (let resolution handle it)
            true
        }
    }

    /// Get all public symbols.
    pub fn public_symbols(&self) -> impl Iterator<Item = &VisibleSymbol> {
        self.symbols.values().filter(|s| s.visibility == Visibility::Public)
    }

    /// Get all symbols in a module.
    pub fn module_symbols<'a>(&'a self, module_path: &'a str) -> impl Iterator<Item = &'a VisibleSymbol> + 'a {
        self.symbols.values().filter(move |s| s.module_path == module_path)
    }

    /// Get exported symbols (public symbols from a module).
    pub fn exports(&self, module_path: &str) -> Vec<String> {
        self.symbols
            .values()
            .filter(|s| s.module_path == module_path && s.visibility == Visibility::Public)
            .map(|s| s.name.clone())
            .collect()
    }

    /// Clear all symbols (for fresh analysis).
    pub fn clear(&mut self) {
        self.symbols.clear();
    }
}

impl Default for VisibilityChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to determine visibility from `is_public` flag.
pub fn visibility_from_flag(is_public: bool) -> Visibility {
    if is_public {
        Visibility::Public
    } else {
        Visibility::Private
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_public_access() {
        let mut checker = VisibilityChecker::new();
        let span = Span::new(0, 0, 0);
        
        checker.register_public("foo", "module_a", span);
        checker.set_current_module("module_b");
        
        assert!(checker.check_access("foo", span).is_ok());
    }

    #[test]
    fn test_private_same_module() {
        let mut checker = VisibilityChecker::new();
        let span = Span::new(0, 0, 0);
        
        checker.register_private("foo", "module_a", span);
        checker.set_current_module("module_a");
        
        assert!(checker.check_access("foo", span).is_ok());
    }

    #[test]
    fn test_private_cross_module() {
        let mut checker = VisibilityChecker::new();
        let span = Span::new(0, 0, 0);
        
        checker.register_private("foo", "module_a", span);
        checker.set_current_module("module_b");
        
        assert!(checker.check_access("foo", span).is_err());
    }

    #[test]
    fn test_is_accessible() {
        let mut checker = VisibilityChecker::new();
        let span = Span::new(0, 0, 0);
        
        checker.register_public("pub_fn", "module_a", span);
        checker.register_private("priv_fn", "module_a", span);
        
        assert!(checker.is_accessible("pub_fn", "module_b"));
        assert!(!checker.is_accessible("priv_fn", "module_b"));
        assert!(checker.is_accessible("priv_fn", "module_a"));
    }
}
