//! String Interning - Efficient string storage and deduplication.
//!
//! Interning stores each unique string only once, allowing cheap comparison
//! via integer IDs instead of string comparison.

use string_interner::backend::StringBackend;
use string_interner::symbol::SymbolU32;
use string_interner::StringInterner as BaseInterner;
use std::sync::RwLock;

/// Type alias for our specific interner configuration.
type InternalInterner = BaseInterner<StringBackend<SymbolU32>>;

/// Interned string ID.
///
/// This is a cheap, copyable handle to an interned string.
pub type InternedStr = SymbolU32;

/// Global string interner wrapped in RwLock for thread safety.
pub struct Interner {
    inner: RwLock<InternalInterner>,
}

impl Default for Interner {
    fn default() -> Self {
        Self::new()
    }
}

impl Interner {
    /// Create a new interner.
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(InternalInterner::default()),
        }
    }

    /// Intern a string, returning its ID.
    pub fn intern(&self, s: &str) -> InternedStr {
        self.inner.write().unwrap().get_or_intern(s)
    }

    /// Get the string for an interned ID.
    pub fn resolve(&self, sym: InternedStr) -> Option<String> {
        self.inner.read().unwrap().resolve(sym).map(|s| s.to_string())
    }

    /// Check if a string is already interned.
    pub fn get(&self, s: &str) -> Option<InternedStr> {
        self.inner.read().unwrap().get(s)
    }

    /// Get the number of interned strings.
    pub fn len(&self) -> usize {
        self.inner.read().unwrap().len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intern_and_resolve() {
        let interner = Interner::new();
        
        let sym1 = interner.intern("hello");
        let sym2 = interner.intern("world");
        let sym3 = interner.intern("hello"); // Same as sym1
        
        assert_eq!(sym1, sym3);
        assert_ne!(sym1, sym2);
        
        assert_eq!(interner.resolve(sym1), Some("hello".to_string()));
        assert_eq!(interner.resolve(sym2), Some("world".to_string()));
    }

    #[test]
    fn test_get() {
        let interner = Interner::new();
        
        assert!(interner.get("hello").is_none());
        
        let sym = interner.intern("hello");
        
        assert_eq!(interner.get("hello"), Some(sym));
        assert!(interner.get("world").is_none());
    }

    #[test]
    fn test_len() {
        let interner = Interner::new();
        
        assert!(interner.is_empty());
        
        interner.intern("a");
        interner.intern("b");
        interner.intern("a"); // Duplicate
        
        assert_eq!(interner.len(), 2);
    }
}
