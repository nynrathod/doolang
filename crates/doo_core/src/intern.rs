//! String Interning — efficient string storage and deduplication.
//!
//! Interning stores each unique string only once, allowing cheap comparison
//! via integer IDs ([`Symbol`]) instead of string comparison. This is critical
//! for compiler performance — identifiers are compared thousands of times
//! during compilation.

use rustc_hash::FxHashMap;
use std::sync::{OnceLock, RwLock};

use crate::symbol::Symbol;

// ============================================================================
// Interner
// ============================================================================

/// A thread-safe string interner.
///
/// Stores each unique string once and returns a [`Symbol`] handle for future
/// reference. Symbols are 4-byte integers that can be compared in O(1).
pub struct Interner {
    /// String → index lookup map.
    map: RwLock<FxHashMap<String, u32>>,
    /// Index → string storage (the canonical copy of each interned string).
    strings: RwLock<Vec<String>>,
}

impl Default for Interner {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Interner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.strings.read().map(|s| s.len()).unwrap_or(0);
        f.debug_struct("Interner")
            .field("interned_count", &len)
            .finish()
    }
}

impl Interner {
    /// Create a new empty interner.
    pub fn new() -> Self {
        Self {
            map: RwLock::new(FxHashMap::default()),
            strings: RwLock::new(Vec::new()),
        }
    }

    /// Create a new interner with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            map: RwLock::new(FxHashMap::with_capacity_and_hasher(
                capacity,
                Default::default(),
            )),
            strings: RwLock::new(Vec::with_capacity(capacity)),
        }
    }

    /// Intern a string, returning its [`Symbol`].
    #[inline]
    pub fn intern(&self, s: &str) -> Symbol {
        // Fast path: read lock — check if already interned
        {
            let map = self.map.read().expect("interner map lock poisoned");
            if let Some(&idx) = map.get(s) {
                return Symbol(idx);
            }
        }

        // Slow path: write lock — insert new string
        let mut map = self.map.write().expect("interner map lock poisoned");
        // Double-check after acquiring write lock
        if let Some(&idx) = map.get(s) {
            return Symbol(idx);
        }

        let mut strings = self
            .strings
            .write()
            .expect("interner strings lock poisoned");
        let idx = strings.len() as u32;
        strings.push(s.to_string());
        map.insert(s.to_string(), idx);
        Symbol(idx)
    }

    /// Intern a `&'static str` (e.g., a keyword), returning its [`Symbol`].
    #[inline]
    pub fn intern_static(&self, s: &'static str) -> Symbol {
        // Fast path: read lock
        {
            let map = self.map.read().expect("interner map lock poisoned");
            if let Some(&idx) = map.get(s) {
                return Symbol(idx);
            }
        }

        // Slow path: write lock
        let mut map = self.map.write().expect("interner map lock poisoned");
        if let Some(&idx) = map.get(s) {
            return Symbol(idx);
        }

        let mut strings = self
            .strings
            .write()
            .expect("interner strings lock poisoned");
        let idx = strings.len() as u32;
        strings.push(s.to_string());
        map.insert(s.to_string(), idx);
        Symbol(idx)
    }

    /// Resolve a [`Symbol`] back to its string.
    pub fn resolve(&self, sym: Symbol) -> &str {
        let strings = self.strings.read().expect("interner strings lock poisoned");
        let s: &str = strings
            .get(sym.0 as usize)
            .map(|s| s.as_str())
            .unwrap_or("<invalid symbol>");
        // SAFETY: The str data is heap-allocated by String. When the Vec<String>
        // reallocates, String objects are moved but their heap data does NOT move.
        // The returned &str points to the String's heap data, which remains valid
        // for the lifetime of the Interner. For the global interner (in OnceLock),
        // this means 'static.
        unsafe { &*(s as *const str) }
    }

    /// Get the symbol for a string if it has already been interned.
    #[inline]
    pub fn get(&self, s: &str) -> Option<Symbol> {
        let map = self.map.read().expect("interner map lock poisoned");
        map.get(s).copied().map(Symbol)
    }

    /// Number of unique interned strings.
    pub fn len(&self) -> usize {
        self.strings
            .read()
            .expect("interner strings lock poisoned")
            .len()
    }

    /// Check if the interner is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Check if a symbol is valid (within bounds).
    pub fn contains(&self, sym: Symbol) -> bool {
        let strings = self.strings.read().expect("interner strings lock poisoned");
        (sym.0 as usize) < strings.len()
    }
}

// ============================================================================
// Global Interner
// ============================================================================

/// Global string interner — lives for the entire program lifetime.
static GLOBAL_INTERNER: OnceLock<Interner> = OnceLock::new();

/// Get a reference to the global interner.
fn global() -> &'static Interner {
    GLOBAL_INTERNER.get_or_init(|| {
        let interner = Interner::new();
        // Pre-intern all keywords
        crate::symbol::keywords::pre_intern(&interner);
        interner
    })
}

/// Intern a string into the global interner.
#[inline]
pub fn intern(s: &str) -> Symbol {
    global().intern(s)
}

/// Intern a `&'static str` into the global interner (for keywords).
#[inline]
pub fn intern_static(s: &'static str) -> Symbol {
    global().intern_static(s)
}

/// Resolve a [`Symbol`] to its string via the global interner.
#[inline]
pub fn resolve(sym: Symbol) -> &'static str {
    // SAFETY: The global interner lives in OnceLock and is never dropped.
    unsafe { &*(global().resolve(sym) as *const str) }
}

/// Get the symbol for a string if already interned in the global interner.
#[inline]
pub fn get(s: &str) -> Option<Symbol> {
    global().get(s)
}

/// Get a keyword symbol from the global interner.
#[inline]
pub fn kw(s: &'static str) -> Symbol {
    global().intern_static(s)
}
