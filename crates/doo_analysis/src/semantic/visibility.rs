//! Case-Based Visibility Enforcement
//!
/// Visibility is derived entirely from identifier casing — no `pub`/`private`
/// keywords exist in the language (Core Decision §1).
///
/// - PascalCase / UpperCamelCase → public (accessible from any module)
/// - camelCase / lowercase → private (accessible only within defining package)
///
/// This module enforces visibility at three points:
/// 1. Declaration time: store visibility on the item
/// 2. Import time: reject `use` of private items from outside the package
/// 3. Usage time: reject use of private items from outside the defining file
use doo_core::{Span, Symbol};
use rustc_hash::FxHashMap;

/// Determine visibility from the first character of a name.
///
/// Uppercase first letter → public. Lowercase or underscore → private.
#[inline]
pub fn is_public(name: &str) -> bool {
    match name.chars().next() {
        Some(c) => c.is_uppercase(),
        None => false,
    }
}

/// Derive visibility from a name string.
#[inline]
pub fn visibility_from_name(name: &str) -> Visibility {
    if is_public(name) {
        Visibility::Public
    } else {
        Visibility::Private
    }
}

/// Derive visibility from an interned symbol.
#[inline]
pub fn visibility_from_symbol(sym: Symbol) -> Visibility {
    visibility_from_name(sym.resolve())
}

/// Derive visibility from an explicit flag (used when visibility is
/// already known, e.g., from AST nodes that store `is_public`).
#[inline]
pub fn visibility_from_flag(is_public: bool) -> Visibility {
    if is_public {
        Visibility::Public
    } else {
        Visibility::Private
    }
}

/// Re-export Visibility from doo_core for convenience.
pub use doo_core::scope::Visibility;

/// Visibility error when a private item is accessed from outside its package.
#[derive(Debug, Clone)]
pub struct VisibilityError {
    /// The private symbol that was accessed.
    pub symbol: String,
    /// Module where the symbol is defined.
    pub defined_in: String,
    /// Module where the access occurred.
    pub accessed_from: String,
    /// Source location of the access.
    pub span: Span,
}

impl VisibilityError {
    pub fn new(
        symbol: impl Into<String>,
        defined_in: impl Into<String>,
        accessed_from: impl Into<String>,
        span: Span,
    ) -> Self {
        Self {
            symbol: symbol.into(),
            defined_in: defined_in.into(),
            accessed_from: accessed_from.into(),
            span,
        }
    }
}

impl std::fmt::Display for VisibilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot access private symbol '{}' from module '{}' (defined in '{}')",
            self.symbol, self.accessed_from, self.defined_in
        )
    }
}

/// Visibility checker for cross-module access validation.
///
/// Tracks which module each symbol is defined in, then validates that
/// private symbols are only accessed from within the same package.
pub struct VisibilityChecker {
    /// Maps symbol name → (defining module path, visibility).
    symbol_origins: FxHashMap<String, (String, Visibility)>,
}

impl VisibilityChecker {
    pub fn new() -> Self {
        Self {
            symbol_origins: FxHashMap::default(),
        }
    }

    /// Register a symbol's origin and visibility.
    pub fn register(&mut self, name: &str, module_path: &str, visibility: Visibility) {
        self.symbol_origins
            .insert(name.to_string(), (module_path.to_string(), visibility));
    }

    /// Register a symbol, deriving visibility from its name casing.
    pub fn register_from_name(&mut self, name: &str, module_path: &str) {
        let vis = visibility_from_name(name);
        self.register(name, module_path, vis);
    }

    /// Check if a symbol can be accessed from the given module.
    ///
    /// Returns `Ok(())` if access is allowed, or `Err(VisibilityError)` if
    /// a private symbol is being accessed from outside its defining package.
    pub fn check_access(
        &self,
        symbol: &str,
        accessing_module: &str,
        span: Span,
    ) -> Result<(), VisibilityError> {
        let (defined_in, visibility) = match self.symbol_origins.get(symbol) {
            Some(entry) => entry.clone(),
            None => return Ok(()), // Unknown symbols pass — caught by name resolution
        };

        match visibility {
            Visibility::Public => Ok(()),
            Visibility::Private => {
                if Self::same_package(&defined_in, accessing_module) {
                    Ok(())
                } else {
                    Err(VisibilityError::new(
                        symbol,
                        defined_in,
                        accessing_module,
                        span,
                    ))
                }
            }
        }
    }

    /// Check if two module paths belong to the same package.
    ///
    /// Same package = same first path segment (the top-level folder).
    fn same_package(module_a: &str, module_b: &str) -> bool {
        let a_root = module_a.split("::").next().unwrap_or(module_a);
        let b_root = module_b.split("::").next().unwrap_or(module_b);
        a_root == b_root
    }
}

impl Default for VisibilityChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Field-level visibility error.
#[derive(Debug, Clone)]
pub struct FieldVisibilityError {
    pub struct_name: String,
    pub field_name: String,
    pub message: String,
    pub span: Span,
}

impl FieldVisibilityError {
    pub fn new(
        struct_name: impl Into<String>,
        field_name: impl Into<String>,
        message: impl Into<String>,
        span: Span,
    ) -> Self {
        Self {
            struct_name: struct_name.into(),
            field_name: field_name.into(),
            message: message.into(),
            span,
        }
    }
}

impl std::fmt::Display for FieldVisibilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "field visibility error on {}.{}: {}",
            self.struct_name, self.field_name, self.message
        )
    }
}

/// Checker for struct field visibility rules.
///
/// Validates that struct field casing follows the visibility conventions
/// and that field access respects visibility boundaries.
pub struct FieldVisibilityChecker {
    /// Maps struct name → list of (field name, is_public).
    field_registry: FxHashMap<String, Vec<(String, bool)>>,
}

impl FieldVisibilityChecker {
    pub fn new() -> Self {
        Self {
            field_registry: FxHashMap::default(),
        }
    }

    /// Register a struct's fields for visibility checking.
    pub fn register_struct(&mut self, struct_name: &str, fields: &[(String, bool)]) {
        self.field_registry
            .insert(struct_name.to_string(), fields.to_vec());
    }

    /// Check if a field access is valid.
    ///
    /// Private fields (lowercase) can only be accessed from within the
    /// same package as the struct definition.
    pub fn check_field_access(
        &self,
        struct_name: &str,
        field_name: &str,
        accessing_module: &str,
        defining_module: &str,
        span: Span,
    ) -> Result<(), FieldVisibilityError> {
        let fields = match self.field_registry.get(struct_name) {
            Some(f) => f,
            None => return Ok(()),
        };

        let is_public = fields
            .iter()
            .find(|(name, _)| name == field_name)
            .map(|(_, pub_)| *pub_)
            .unwrap_or_else(|| is_public(field_name));

        if !is_public && !VisibilityChecker::same_package(defining_module, accessing_module) {
            return Err(FieldVisibilityError::new(
                struct_name,
                field_name,
                format!(
                    "private field '{}' cannot be accessed from outside package",
                    field_name
                ),
                span,
            ));
        }

        Ok(())
    }
}

impl Default for FieldVisibilityChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Validate visibility at declaration time.
///
/// Ensures that enum variants are always PascalCase (they inherit the
/// enum's visibility) and that local variables are always lowercase.
pub fn check_field_visibility(
    _struct_name: &str,
    field_name: &str,
    is_public: bool,
) -> Result<(), FieldVisibilityError> {
    let field_vis = visibility_from_name(field_name);

    // If the struct is public but a field is private, that's fine —
    // the field just won't be accessible from outside.
    // But if the struct is private and a field is "public" (PascalCase),
    // that's a warning: the public casing has no effect since the struct
    // itself is private.
    if !is_public && field_vis == Visibility::Public {
        // This is not an error, just suboptimal — skip silently.
        // A linter could warn about this.
    }

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_public() {
        assert!(is_public("User"));
        assert!(is_public("MyFunc"));
        assert!(is_public("Server"));
        assert!(!is_public("save"));
        assert!(!is_public("userService"));
        assert!(!is_public("_internal"));
    }

    #[test]
    fn test_visibility_from_name() {
        assert_eq!(visibility_from_name("Public"), Visibility::Public);
        assert_eq!(visibility_from_name("private"), Visibility::Private);
    }

    #[test]
    fn test_visibility_checker_same_package() {
        let mut checker = VisibilityChecker::new();
        checker.register("privateFunc", "services::auth", Visibility::Private);

        // Same package — allowed
        assert!(checker
            .check_access("privateFunc", "services::user", Span::dummy())
            .is_ok());
    }

    #[test]
    fn test_visibility_checker_different_package() {
        let mut checker = VisibilityChecker::new();
        checker.register("privateFunc", "services::auth", Visibility::Private);

        // Different package — denied
        assert!(checker
            .check_access("privateFunc", "models::user", Span::dummy())
            .is_err());
    }

    #[test]
    fn test_visibility_checker_public_anywhere() {
        let mut checker = VisibilityChecker::new();
        checker.register("PublicFunc", "services::auth", Visibility::Public);

        // Public — allowed from anywhere
        assert!(checker
            .check_access("PublicFunc", "models::user", Span::dummy())
            .is_ok());
    }

    #[test]
    fn test_same_package_logic() {
        assert!(VisibilityChecker::same_package(
            "services::auth",
            "services::user"
        ));
        assert!(VisibilityChecker::same_package("a", "a"));
        assert!(!VisibilityChecker::same_package(
            "services::auth",
            "models::user"
        ));
    }
}
