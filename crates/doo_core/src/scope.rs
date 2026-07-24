//! Symbol Table — scope-based symbol management for name resolution.
//!
//! This module manages symbols (variables, functions, types) in lexical scopes.
//! It is used during HIR lowering and type checking to track what names are
//! in scope and their associated types.

use crate::span::Span;
use crate::types::registry::TypeId;
use std::collections::HashMap;

// ============================================================================
// Symbol Info
// ============================================================================

/// Information about a symbol (variable, function, type).
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    /// Symbol name.
    pub name: String,
    /// Symbol kind (variable, function, struct, etc.).
    pub kind: SymbolKind,
    /// Type of the symbol.
    pub type_id: TypeId,
    /// Whether the symbol is mutable (can be reassigned).
    pub mutable: bool,
    /// Where the symbol was defined.
    pub span: Span,
}

/// What kind of symbol this is.
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
    /// Constant.
    Constant,
}

// ============================================================================
// Scope
// ============================================================================

/// A single lexical scope level.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    /// Symbols defined in this scope.
    symbols: HashMap<String, SymbolInfo>,
}

impl Scope {
    /// Create a new empty scope.
    pub fn new() -> Self {
        Self::default()
    }

    /// Define a symbol in this scope.
    pub fn define(&mut self, symbol: SymbolInfo) {
        self.symbols.insert(symbol.name.clone(), symbol);
    }

    /// Look up a symbol in this scope only.
    pub fn lookup(&self, name: &str) -> Option<&SymbolInfo> {
        self.symbols.get(name)
    }

    /// Look up a symbol mutably in this scope only.
    pub fn lookup_mut(&mut self, name: &str) -> Option<&mut SymbolInfo> {
        self.symbols.get_mut(name)
    }

    /// Number of symbols in this scope.
    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    /// Check if this scope is empty.
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    /// Iterate over all symbols in this scope.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &SymbolInfo)> {
        self.symbols.iter()
    }
}

// ============================================================================
// Symbol Table (Scope Stack)
// ============================================================================

/// The symbol table with a stack of scopes.
#[derive(Debug)]
pub struct SymbolTable {
    /// Stack of scopes (innermost is last).
    scopes: Vec<Scope>,
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolTable {
    /// Create a new symbol table with a global scope.
    pub fn new() -> Self {
        Self {
            scopes: vec![Scope::new()],
        }
    }

    /// Push a new scope onto the stack.
    pub fn push_scope(&mut self) {
        self.scopes.push(Scope::new());
    }

    /// Pop the current scope from the stack.
    pub fn pop_scope(&mut self) -> Option<Scope> {
        if self.scopes.len() > 1 {
            self.scopes.pop()
        } else {
            None
        }
    }

    /// Current scope depth (1 = global, 2 = one nested block, etc.).
    pub fn depth(&self) -> usize {
        self.scopes.len()
    }

    /// Define a symbol in the current (innermost) scope.
    pub fn define(&mut self, symbol: SymbolInfo) -> Result<(), SymbolError> {
        let current = self.scopes.last_mut().unwrap();
        if current.lookup(&symbol.name).is_some() {
            return Err(SymbolError::Redefinition {
                name: symbol.name.clone(),
                span: symbol.span,
            });
        }
        current.define(symbol);
        Ok(())
    }

    /// Look up a symbol, searching from innermost to outermost scope.
    pub fn lookup(&self, name: &str) -> Option<&SymbolInfo> {
        for scope in self.scopes.iter().rev() {
            if let Some(sym) = scope.lookup(name) {
                return Some(sym);
            }
        }
        None
    }

    /// Look up a symbol mutably, searching from innermost to outermost scope.
    pub fn lookup_mut(&mut self, name: &str) -> Option<&mut SymbolInfo> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(sym) = scope.lookup_mut(name) {
                return Some(sym);
            }
        }
        None
    }

    /// Check if a symbol exists in any scope.
    pub fn contains(&self, name: &str) -> bool {
        self.lookup(name).is_some()
    }

    /// Look up a symbol only in the current (innermost) scope.
    pub fn lookup_current(&self, name: &str) -> Option<&SymbolInfo> {
        self.scopes.last().and_then(|s| s.lookup(name))
    }

    /// Check if a symbol is defined in the current scope only.
    pub fn contains_current(&self, name: &str) -> bool {
        self.lookup_current(name).is_some()
    }

    /// Get all symbols in the current scope.
    pub fn current_scope_symbols(&self) -> impl Iterator<Item = (&String, &SymbolInfo)> {
        self.scopes.last().unwrap().iter()
    }

    /// Number of symbols across all scopes.
    pub fn total_symbols(&self) -> usize {
        self.scopes.iter().map(|s| s.len()).sum()
    }
}

// ============================================================================
// Errors
// ============================================================================

/// Symbol table errors.
#[derive(Debug, Clone)]
pub enum SymbolError {
    /// Symbol already defined in the current scope.
    Redefinition { name: String, span: Span },
    /// Symbol not found in any scope.
    NotFound { name: String, span: Span },
}

impl std::fmt::Display for SymbolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SymbolError::Redefinition { name, .. } => {
                write!(f, "symbol '{}' already defined in this scope", name)
            }
            SymbolError::NotFound { name, .. } => {
                write!(f, "symbol '{}' not found", name)
            }
        }
    }
}

impl std::error::Error for SymbolError {}
