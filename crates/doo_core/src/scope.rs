//! Scope Management
//!
//! Hierarchical symbol tables for tracking declarations.
//! Module-level scope resolution across files with visibility enforcement.

use crate::symbol::Symbol as CoreSymbol;
use crate::{types::TypeId, Span};
use rustc_hash::FxHashMap;

// ============================================================================
// Module-Level Scope Resolution
// ============================================================================

/// Visibility derived from identifier casing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// PascalCase — accessible from any module.
    Public,
    /// camelCase / lowercase — accessible only within defining package.
    Private,
}

impl Visibility {
    /// Derive visibility from the first character of a name.
    pub fn from_name(name: &str) -> Self {
        match name.chars().next() {
            Some(c) if c.is_uppercase() => Visibility::Public,
            _ => Visibility::Private,
        }
    }

    pub fn is_public(self) -> bool {
        matches!(self, Visibility::Public)
    }
}

/// A resolved scope item at the module level.
#[derive(Debug, Clone)]
pub enum ScopeItem {
    Function(CoreSymbol, Visibility),
    Struct(CoreSymbol, Visibility),
    Enum(CoreSymbol, Visibility),
    Const(CoreSymbol, Visibility),
    Static(CoreSymbol, Visibility),
}

impl ScopeItem {
    pub fn visibility(&self) -> Visibility {
        match self {
            ScopeItem::Function(_, v)
            | ScopeItem::Struct(_, v)
            | ScopeItem::Enum(_, v)
            | ScopeItem::Const(_, v)
            | ScopeItem::Static(_, v) => *v,
        }
    }

    pub fn name(&self) -> CoreSymbol {
        match self {
            ScopeItem::Function(s, _)
            | ScopeItem::Struct(s, _)
            | ScopeItem::Enum(s, _)
            | ScopeItem::Const(s, _)
            | ScopeItem::Static(s, _) => *s,
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            ScopeItem::Function(_, _) => "function",
            ScopeItem::Struct(_, _) => "struct",
            ScopeItem::Enum(_, _) => "enum",
            ScopeItem::Const(_, _) => "const",
            ScopeItem::Static(_, _) => "static",
        }
    }
}

/// An import declaration resolved at module scope.
#[derive(Debug, Clone)]
pub struct ModuleImport {
    /// Absolute path from project root.
    pub path: Vec<CoreSymbol>,
    /// Imported symbol name, or None for namespace/wildcard.
    pub item: Option<CoreSymbol>,
    /// Local alias for the imported symbol.
    pub alias: Option<CoreSymbol>,
    /// Whether this is a wildcard import.
    pub wildcard: bool,
    /// Span of the import statement.
    pub span: Span,
}

/// Symbol table for a single module (file).
#[derive(Debug, Clone, Default)]
pub struct ModuleScope {
    /// Items declared in this module, keyed by name.
    pub items: FxHashMap<CoreSymbol, ScopeItem>,
    /// Import declarations in this module.
    pub imports: Vec<ModuleImport>,
    /// Module path from project root.
    pub path: Vec<CoreSymbol>,
}

impl ModuleScope {
    pub fn new(path: Vec<CoreSymbol>) -> Self {
        Self {
            items: FxHashMap::default(),
            imports: Vec::new(),
            path,
        }
    }

    /// Define an item in this module.
    pub fn define(&mut self, name: CoreSymbol, item: ScopeItem) {
        self.items.insert(name, item);
    }

    /// Look up an item by name in this module.
    pub fn lookup(&self, name: CoreSymbol) -> Option<&ScopeItem> {
        self.items.get(&name)
    }

    /// Get all public items (for cross-module export).
    pub fn public_items(&self) -> impl Iterator<Item = (&CoreSymbol, &ScopeItem)> {
        self.items
            .iter()
            .filter(|(_, item)| item.visibility().is_public())
    }
}

/// Errors from scope resolution.
#[derive(Debug, Clone)]
pub enum ScopeResolverError {
    /// Unresolved import: symbol not found in target module.
    UnresolvedImport {
        path: Vec<String>,
        symbol: String,
        span: Span,
    },
    /// Private item accessed from outside its package.
    PrivateItemAccess {
        symbol: String,
        defined_in: Vec<String>,
        accessed_from: Vec<String>,
        span: Span,
    },
    /// Circular import detected.
    CircularImport { cycle: Vec<String>, span: Span },
    /// Symbol not found in any visible scope.
    UnknownSymbol {
        name: String,
        span: Span,
        suggestion: Option<String>,
    },
    /// Duplicate definition in the same module.
    DuplicateDefinition { name: String, span: Span },
}

/// Resolves names across modules, enforcing visibility rules.
///
/// Builds a module graph from loaded files, populates per-module symbol
/// tables, and resolves `use` imports eagerly. Every cross-file reference
/// requires an explicit `use` — there is no implicit scope.
pub struct ScopeResolver {
    /// Module path → module scope.
    modules: FxHashMap<Vec<CoreSymbol>, ModuleScope>,
    /// Current module path being analyzed.
    current_path: Vec<CoreSymbol>,
    /// Collected errors.
    errors: Vec<ScopeResolverError>,
}

impl ScopeResolver {
    pub fn new() -> Self {
        Self {
            modules: FxHashMap::default(),
            current_path: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Register a module with its scope.
    pub fn register_module(&mut self, path: Vec<CoreSymbol>, scope: ModuleScope) {
        self.modules.insert(path, scope);
    }

    /// Set the current module being analyzed.
    pub fn set_current_module(&mut self, path: Vec<CoreSymbol>) {
        self.current_path = path;
    }

    /// Get the current module's scope.
    pub fn current_scope(&self) -> Option<&ModuleScope> {
        self.modules.get(&self.current_path)
    }

    /// Get the current module's scope mutably.
    pub fn current_scope_mut(&mut self) -> Option<&mut ModuleScope> {
        self.modules.get_mut(&self.current_path)
    }

    /// Define an item in the current module.
    pub fn define_item(&mut self, name: CoreSymbol, item: ScopeItem) {
        if let Some(scope) = self.current_scope_mut() {
            if scope.items.contains_key(&name) {
                self.errors.push(ScopeResolverError::DuplicateDefinition {
                    name: name.resolve().to_string(),
                    span: Span::dummy(),
                });
                return;
            }
            scope.define(name, item);
        }
    }

    /// Add an import to the current module.
    pub fn add_import(&mut self, import: ModuleImport) {
        if let Some(scope) = self.current_scope_mut() {
            scope.imports.push(import);
        }
    }

    /// Resolve a symbol name in the current module.
    ///
    /// Validates that the resolved symbol is accessible: public OR in the
    /// same package.
    pub fn resolve(&self, name: CoreSymbol) -> Result<&ScopeItem, ScopeResolverError> {
        if let Some(scope) = self.current_scope() {
            // Check local definitions first
            if let Some(item) = scope.lookup(name) {
                return Ok(item);
            }

            // Check imports
            for import in &scope.imports {
                if import.wildcard {
                    if let Some(target_scope) = self.modules.get(&import.path) {
                        if let Some(item) = target_scope.lookup(name) {
                            if item.visibility().is_public() {
                                return Ok(item);
                            }
                        }
                    }
                } else if let Some(item_name) = import.item {
                    let local_name = import.alias.unwrap_or(item_name);
                    if local_name == name {
                        if let Some(target_scope) = self.modules.get(&import.path) {
                            if let Some(item) = target_scope.lookup(item_name) {
                                if item.visibility().is_public() || self.same_package(&import.path)
                                {
                                    return Ok(item);
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(err) = self.errors.first() {
            Err(err.clone())
        } else {
            Err(ScopeResolverError::UnknownSymbol {
                name: name.resolve().to_string(),
                span: Span::dummy(),
                suggestion: None,
            })
        }
    }

    /// Check if a module path is in the same package as the current module.
    /// Same package = same first path segment.
    fn same_package(&self, other: &[CoreSymbol]) -> bool {
        match (self.current_path.first(), other.first()) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }

    /// Validate all imports in the current module.
    ///
    /// Rejects unresolved imports and private item access from outside the
    /// defining package.
    pub fn validate_imports(&mut self) {
        let imports: Vec<ModuleImport> = match self.current_scope() {
            Some(scope) => scope.imports.clone(),
            None => return,
        };

        for import in &imports {
            let target_scope = match self.modules.get(&import.path) {
                Some(s) => s,
                None => {
                    self.errors.push(ScopeResolverError::UnresolvedImport {
                        path: import
                            .path
                            .iter()
                            .map(|s| s.resolve().to_string())
                            .collect(),
                        symbol: import
                            .item
                            .map(|s| s.resolve().to_string())
                            .unwrap_or_default(),
                        span: import.span,
                    });
                    continue;
                }
            };

            if import.wildcard {
                continue;
            }

            if let Some(item_name) = import.item {
                match target_scope.lookup(item_name) {
                    Some(item) => {
                        if !item.visibility().is_public() && !self.same_package(&import.path) {
                            self.errors.push(ScopeResolverError::PrivateItemAccess {
                                symbol: item_name.resolve().to_string(),
                                defined_in: import
                                    .path
                                    .iter()
                                    .map(|s| s.resolve().to_string())
                                    .collect(),
                                accessed_from: self
                                    .current_path
                                    .iter()
                                    .map(|s| s.resolve().to_string())
                                    .collect(),
                                span: import.span,
                            });
                        }
                    }
                    None => {
                        self.errors.push(ScopeResolverError::UnresolvedImport {
                            path: import
                                .path
                                .iter()
                                .map(|s| s.resolve().to_string())
                                .collect(),
                            symbol: item_name.resolve().to_string(),
                            span: import.span,
                        });
                    }
                }
            }
        }
    }

    /// Take collected errors.
    pub fn take_errors(&mut self) -> Vec<ScopeResolverError> {
        std::mem::take(&mut self.errors)
    }

    /// Check if any errors were collected.
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Get all registered module paths.
    pub fn module_paths(&self) -> impl Iterator<Item = &Vec<CoreSymbol>> {
        self.modules.keys()
    }

    /// Get a module scope by path.
    pub fn get_module(&self, path: &[CoreSymbol]) -> Option<&ModuleScope> {
        self.modules.get(path)
    }
}

impl Default for ScopeResolver {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Lexical Scope Management (within functions)
// ============================================================================

/// Symbol in a scope.
#[derive(Debug, Clone)]
pub struct Symbol {
    /// Name of the symbol.
    pub name: String,
    /// Kind of symbol.
    pub kind: SymbolKind,
    /// Type of the symbol (may be unresolved initially).
    pub type_id: Option<TypeId>,
    /// Is this symbol mutable?
    pub mutable: bool,
    /// Where the symbol was declared.
    pub span: Span,
    /// Is this symbol used?
    pub used: bool,
}

/// Symbol kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    /// Local variable.
    Variable,
    /// Function parameter.
    Parameter,
    /// Function definition.
    Function,
    /// Struct type.
    Struct,
    /// Enum type.
    Enum,
    /// Interface / generic type name.
    Type,
    /// Import.
    Import,
    /// Loop variable (for-in).
    LoopVar,
    /// Compile-time constant.
    Const,
    /// Runtime global variable (OnceLock semantics).
    Static,
}

/// A single scope with its symbols.
#[derive(Debug, Clone)]
pub struct Scope {
    /// Parent scope index (None for global).
    pub parent: Option<usize>,
    /// Symbols in this scope.
    pub symbols: FxHashMap<String, Symbol>,
    /// Kind of scope.
    pub kind: ScopeKind,
}

/// Kinds of scopes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeKind {
    /// Global/module scope.
    Global,
    /// Function body.
    Function,
    /// Block (if, loop body).
    Block,
    /// Loop (for/while).
    Loop,
}

impl Scope {
    pub fn new(kind: ScopeKind, parent: Option<usize>) -> Self {
        Self {
            parent,
            symbols: FxHashMap::default(),
            kind,
        }
    }
}

/// Manages a hierarchy of scopes.
pub struct ScopeManager {
    /// All scopes (index 0 is global).
    scopes: Vec<Scope>,
    /// Current scope index.
    current: usize,
}

impl ScopeManager {
    /// Create a new scope manager with global scope.
    pub fn new() -> Self {
        let global = Scope::new(ScopeKind::Global, None);
        Self {
            scopes: vec![global],
            current: 0,
        }
    }

    /// Enter a new scope.
    pub fn enter_scope(&mut self, kind: ScopeKind) {
        let new_scope = Scope::new(kind, Some(self.current));
        self.scopes.push(new_scope);
        self.current = self.scopes.len() - 1;
    }

    /// Exit current scope, return to parent.
    pub fn exit_scope(&mut self) {
        if let Some(parent) = self.scopes[self.current].parent {
            self.current = parent;
        }
    }

    /// Keep exiting scopes until we reach one of the given kind.
    /// Safety net for unbalanced scope entries from buggy check_stmt handlers.
    pub fn exit_to_kind(&mut self, target: ScopeKind) {
        while self.scopes[self.current].kind != target && self.scopes[self.current].parent.is_some()
        {
            self.exit_scope();
        }
    }

    /// Get the current scope index (for debugging).
    pub fn current_idx(&self) -> usize {
        self.current
    }

    /// Define a symbol in the current scope.
    pub fn define(&mut self, symbol: Symbol) -> Result<(), ScopeError> {
        let scope = &mut self.scopes[self.current];
        if scope.symbols.contains_key(&symbol.name) {
            return Err(ScopeError::Redeclaration {
                name: symbol.name.clone(),
                original: scope.symbols[&symbol.name].span,
                redeclared: symbol.span,
            });
        }
        scope.symbols.insert(symbol.name.clone(), symbol);
        Ok(())
    }

    /// Look up a symbol, searching parent scopes.
    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        let mut scope_idx = Some(self.current);
        while let Some(idx) = scope_idx {
            if let Some(sym) = self.scopes[idx].symbols.get(name) {
                return Some(sym);
            }
            scope_idx = self.scopes[idx].parent;
        }
        None
    }

    /// Look up a symbol mutably.
    pub fn lookup_mut(&mut self, name: &str) -> Option<&mut Symbol> {
        let mut scope_idx = Some(self.current);
        while let Some(idx) = scope_idx {
            if self.scopes[idx].symbols.contains_key(name) {
                return self.scopes[idx].symbols.get_mut(name);
            }
            scope_idx = self.scopes[idx].parent;
        }
        None
    }

    /// Mark a symbol as used.
    pub fn mark_used(&mut self, name: &str) {
        if let Some(sym) = self.lookup_mut(name) {
            sym.used = true;
        }
    }

    /// Check if currently in a loop scope.
    pub fn in_loop(&self) -> bool {
        let mut scope_idx = Some(self.current);
        while let Some(idx) = scope_idx {
            if self.scopes[idx].kind == ScopeKind::Loop {
                return true;
            }
            scope_idx = self.scopes[idx].parent;
        }
        false
    }

    /// Check if currently in a function scope.
    pub fn in_function(&self) -> bool {
        let mut scope_idx = Some(self.current);
        while let Some(idx) = scope_idx {
            if self.scopes[idx].kind == ScopeKind::Function {
                return true;
            }
            scope_idx = self.scopes[idx].parent;
        }
        false
    }

    /// Get all unused variables (for warnings).
    pub fn unused_symbols(&self) -> Vec<&Symbol> {
        self.scopes
            .iter()
            .flat_map(|s| s.symbols.values())
            .filter(|s| !s.used && matches!(s.kind, SymbolKind::Variable | SymbolKind::Parameter))
            .collect()
    }

    /// Collect all visible symbol names from the current scope chain.
    pub fn collect_visible_names(&self) -> Vec<&str> {
        let mut names = Vec::new();
        let mut scope_idx = Some(self.current);
        while let Some(idx) = scope_idx {
            for key in self.scopes[idx].symbols.keys() {
                // Skip internal symbols (e.g. __struct_Foo)
                if !key.starts_with("__") {
                    names.push(key.as_str());
                }
            }
            scope_idx = self.scopes[idx].parent;
        }
        names
    }

    /// Find the closest matching name for a given name using Levenshtein distance.
    /// Returns `Some(name)` if a match is found within `max_distance` (default 3).
    pub fn find_suggestion(&self, name: &str) -> Option<String> {
        const MAX_DISTANCE: usize = 3;
        let visible = self.collect_visible_names();
        let mut best: Option<(&str, usize)> = None;
        for candidate in &visible {
            if *candidate == name {
                continue;
            }
            let dist = levenshtein(name, candidate);
            if dist <= MAX_DISTANCE {
                if best.is_none() || dist < best.unwrap().1 {
                    best = Some((candidate, dist));
                }
            }
        }
        best.map(|(s, _)| s.to_string())
    }
}

/// Compute Levenshtein edit distance between two strings.
fn levenshtein(a: &str, b: &str) -> usize {
    let a_len = a.len();
    let b_len = b.len();
    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut prev: Vec<usize> = (0..=b_len).collect();
    let mut curr = vec![0usize; b_len + 1];

    for (i, ac) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, bc) in b.chars().enumerate() {
            let cost = if ac == bc { 0 } else { 1 };
            curr[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(curr[j] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_len]
}

impl Default for ScopeManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Scope-related errors.
#[derive(Debug, Clone)]
pub enum ScopeError {
    /// Variable redeclared in same scope.
    Redeclaration {
        name: String,
        original: Span,
        redeclared: Span,
    },
    /// Variable not found.
    Undeclared {
        name: String,
        span: Span,
        /// Closest matching name from visible scope (for "did you mean?" suggestions).
        suggestion: Option<String>,
    },
}

impl ScopeError {
    pub fn message(&self) -> String {
        match self {
            Self::Redeclaration { name, .. } => {
                format!("Variable '{}' already declared in this scope", name)
            }
            Self::Undeclared { name, .. } => {
                format!("Undefined variable '{}'", name)
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_creation() {
        let mgr = ScopeManager::new();
        assert_eq!(mgr.current, 0);
    }

    #[test]
    fn test_enter_exit_scope() {
        let mut mgr = ScopeManager::new();
        mgr.enter_scope(ScopeKind::Function);
        assert_eq!(mgr.current, 1);
        mgr.exit_scope();
        assert_eq!(mgr.current, 0);
    }

    #[test]
    fn test_define_and_lookup() {
        let mut mgr = ScopeManager::new();
        let sym = Symbol {
            name: "x".to_string(),
            kind: SymbolKind::Variable,
            type_id: None,
            mutable: false,
            span: Span::dummy(),
            used: false,
        };
        mgr.define(sym).unwrap();
        assert!(mgr.lookup("x").is_some());
        assert!(mgr.lookup("y").is_none());
    }

    #[test]
    fn test_redeclaration_error() {
        let mut mgr = ScopeManager::new();
        let sym1 = Symbol {
            name: "x".to_string(),
            kind: SymbolKind::Variable,
            type_id: None,
            mutable: false,
            span: Span::dummy(),
            used: false,
        };
        let sym2 = sym1.clone();
        mgr.define(sym1).unwrap();
        assert!(mgr.define(sym2).is_err());
    }

    #[test]
    fn test_nested_scope_lookup() {
        let mut mgr = ScopeManager::new();
        let sym = Symbol {
            name: "x".to_string(),
            kind: SymbolKind::Variable,
            type_id: None,
            mutable: false,
            span: Span::dummy(),
            used: false,
        };
        mgr.define(sym).unwrap();
        mgr.enter_scope(ScopeKind::Block);
        // Should still find x from parent scope
        assert!(mgr.lookup("x").is_some());
    }

    #[test]
    fn test_visibility_from_name() {
        assert_eq!(Visibility::from_name("User"), Visibility::Public);
        assert_eq!(Visibility::from_name("MyFunc"), Visibility::Public);
        assert_eq!(Visibility::from_name("save"), Visibility::Private);
        assert_eq!(Visibility::from_name("userService"), Visibility::Private);
    }

    #[test]
    fn test_module_scope_define_and_lookup() {
        let mut scope = ModuleScope::new(vec![]);
        let name = CoreSymbol::intern("User");
        scope.define(name, ScopeItem::Struct(name, Visibility::Public));
        assert!(scope.lookup(name).is_some());
    }

    #[test]
    fn test_scope_resolver_register_and_lookup() {
        let mut resolver = ScopeResolver::new();
        let path = vec![CoreSymbol::intern("models")];
        let mut scope = ModuleScope::new(path.clone());
        let name = CoreSymbol::intern("User");
        scope.define(name, ScopeItem::Struct(name, Visibility::Public));
        resolver.register_module(path.clone(), scope);
        resolver.set_current_module(path);
        assert!(resolver.current_scope().is_some());
    }

    #[test]
    fn test_same_package_check() {
        let mut resolver = ScopeResolver::new();
        resolver.set_current_module(vec![
            CoreSymbol::intern("services"),
            CoreSymbol::intern("auth"),
        ]);
        assert!(
            resolver.same_package(&[CoreSymbol::intern("services"), CoreSymbol::intern("user")])
        );
        assert!(!resolver.same_package(&[CoreSymbol::intern("models")]));
    }
}
