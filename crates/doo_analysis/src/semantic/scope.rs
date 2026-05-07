//! Scope Management
//!
//! Hierarchical symbol tables for tracking declarations.

use doo_core::{types::TypeId, Span};
use rustc_hash::FxHashMap;

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
}
