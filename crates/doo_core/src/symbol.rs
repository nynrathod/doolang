//! # Symbol Table
//!
//! Manages symbols (variables, functions, types) in scopes.
//!
//! ## Design
//!
//! - Scope stack for proper variable shadowing
//! - Fast lookup with HashMap
//! - Tracks mutability and ownership state

use crate::types::TypeId;
use crate::span::Span;
use std::collections::HashMap;

// ============================================================================
// Symbol Info
// ============================================================================

/// Information about a symbol (variable, function, type).
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    /// Symbol name
    pub name: String,
    /// Symbol kind
    pub kind: SymbolKind,
    /// Type of the symbol
    pub type_id: TypeId,
    /// Whether the symbol is mutable
    pub mutable: bool,
    /// Where the symbol was defined
    pub span: Span,
}

/// What kind of symbol this is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    /// Local variable
    Variable,
    /// Function parameter
    Parameter,
    /// Function definition
    Function,
    /// Struct type
    Struct,
    /// Enum type
    Enum,
    /// Constant
    Constant,
}

// ============================================================================
// Scope
// ============================================================================

/// A single scope level.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    /// Symbols in this scope
    symbols: HashMap<String, SymbolInfo>,
}

impl Scope {
    pub fn new() -> Self {
        Self {
            symbols: HashMap::new(),
        }
    }
    
    /// Define a symbol in this scope
    pub fn define(&mut self, symbol: SymbolInfo) {
        self.symbols.insert(symbol.name.clone(), symbol);
    }
    
    /// Lookup a symbol in this scope only
    pub fn lookup(&self, name: &str) -> Option<&SymbolInfo> {
        self.symbols.get(name)
    }
}

// ============================================================================
// Symbol Table
// ============================================================================

/// The symbol table with scope stack.
#[derive(Debug)]
pub struct SymbolTable {
    /// Stack of scopes (innermost last)
    scopes: Vec<Scope>,
}

impl SymbolTable {
    /// Create new symbol table with global scope
    pub fn new() -> Self {
        Self {
            scopes: vec![Scope::new()],
        }
    }
    
    /// Push a new scope
    pub fn push_scope(&mut self) {
        self.scopes.push(Scope::new());
    }
    
    /// Pop the current scope
    pub fn pop_scope(&mut self) -> Option<Scope> {
        if self.scopes.len() > 1 {
            self.scopes.pop()
        } else {
            None // Never pop global scope
        }
    }
    
    /// Current scope depth
    pub fn depth(&self) -> usize {
        self.scopes.len()
    }
    
    /// Define a symbol in the current scope
    pub fn define(&mut self, symbol: SymbolInfo) -> Result<(), SymbolError> {
        let current = self.scopes.last_mut().unwrap();
        
        // Check for redefinition in same scope
        if current.lookup(&symbol.name).is_some() {
            return Err(SymbolError::Redefinition {
                name: symbol.name.clone(),
                span: symbol.span,
            });
        }
        
        current.define(symbol);
        Ok(())
    }
    
    /// Lookup a symbol, searching from innermost to outermost scope
    pub fn lookup(&self, name: &str) -> Option<&SymbolInfo> {
        for scope in self.scopes.iter().rev() {
            if let Some(sym) = scope.lookup(name) {
                return Some(sym);
            }
        }
        None
    }
    
    /// Check if a symbol exists
    pub fn contains(&self, name: &str) -> bool {
        self.lookup(name).is_some()
    }
    
    /// Lookup only in current scope
    pub fn lookup_current(&self, name: &str) -> Option<&SymbolInfo> {
        self.scopes.last().and_then(|s| s.lookup(name))
    }
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Errors
// ============================================================================

/// Symbol table errors
#[derive(Debug, Clone)]
pub enum SymbolError {
    /// Symbol already defined in this scope
    Redefinition { name: String, span: Span },
    /// Symbol not found
    NotFound { name: String, span: Span },
}

impl std::fmt::Display for SymbolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Redefinition { name, .. } => {
                write!(f, "symbol '{}' already defined in this scope", name)
            }
            Self::NotFound { name, .. } => {
                write!(f, "symbol '{}' not found", name)
            }
        }
    }
}

impl std::error::Error for SymbolError {}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::builtin;

    #[test]
    fn test_scope_stack() {
        let mut table = SymbolTable::new();
        assert_eq!(table.depth(), 1);
        
        table.push_scope();
        assert_eq!(table.depth(), 2);
        
        table.pop_scope();
        assert_eq!(table.depth(), 1);
    }

    #[test]
    fn test_symbol_lookup() {
        let mut table = SymbolTable::new();
        
        let sym = SymbolInfo {
            name: "x".to_string(),
            kind: SymbolKind::Variable,
            type_id: builtin::INT,
            mutable: false,
            span: Span::empty(),
        };
        
        table.define(sym).unwrap();
        
        assert!(table.lookup("x").is_some());
        assert!(table.lookup("y").is_none());
    }

    #[test]
    fn test_shadowing() {
        let mut table = SymbolTable::new();
        
        // Define x in global scope
        table.define(SymbolInfo {
            name: "x".to_string(),
            kind: SymbolKind::Variable,
            type_id: builtin::INT,
            mutable: false,
            span: Span::empty(),
        }).unwrap();
        
        // Push scope and shadow x
        table.push_scope();
        table.define(SymbolInfo {
            name: "x".to_string(),
            kind: SymbolKind::Variable,
            type_id: builtin::STR, // Different type
            mutable: true,
            span: Span::empty(),
        }).unwrap();
        
        // Lookup finds inner x
        let inner = table.lookup("x").unwrap();
        assert_eq!(inner.type_id, builtin::STR);
        assert!(inner.mutable);
        
        // Pop scope
        table.pop_scope();
        
        // Lookup finds outer x
        let outer = table.lookup("x").unwrap();
        assert_eq!(outer.type_id, builtin::INT);
        assert!(!outer.mutable);
    }
}
