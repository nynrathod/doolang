//! Symbol — an interned string identifier.
//!
//! A [`Symbol`] is a compact (4-byte) handle to an interned string. Symbols
//! are used throughout the compiler for identifiers, keywords, and other
//! string data that needs fast comparison and low memory overhead.

use std::fmt;

use crate::intern;

// ============================================================================
// Symbol
// ============================================================================

/// An interned string symbol — a 4-byte handle to a unique string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Symbol(pub u32);

impl Symbol {
    /// Create a symbol from a raw index.
    pub const fn new(idx: u32) -> Self {
        Self(idx)
    }

    /// Get the raw `u32` index.
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Intern a string into the global interner and return its symbol.
    #[inline]
    pub fn intern(s: &str) -> Self {
        intern::intern(s)
    }

    /// Resolve this symbol to its string via the global interner.
    #[inline]
    pub fn resolve(self) -> &'static str {
        intern::resolve(self)
    }

    /// A dummy symbol (index 0).
    pub const DUMMY: Self = Self(0);
}

impl Default for Symbol {
    fn default() -> Self {
        Self::DUMMY
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.resolve())
    }
}

impl AsRef<str> for Symbol {
    fn as_ref(&self) -> &str {
        self.resolve()
    }
}

impl From<&str> for Symbol {
    #[inline]
    fn from(s: &str) -> Self {
        Self::intern(s)
    }
}

impl From<&String> for Symbol {
    #[inline]
    fn from(s: &String) -> Self {
        Self::intern(s)
    }
}

// ============================================================================
// Pre-interned Keywords
// ============================================================================

/// Pre-interned keyword symbols for the Doo language.
pub mod keywords {
    use super::Symbol;
    use crate::intern::Interner;

    /// All keyword strings in the Doo language.
    pub const KEYWORD_STRS: &[&str] = &[
        "fn", "let", "const", "static", "mut", "if", "else", "return", "for", "while", "in",
        "match", "break", "continue", "struct", "enum", "use", "import", "as", "true", "false",
        "null", "async", "await", "go", "try", "catch", "throw", "scope", "route", "ws",
    ];

    /// Pre-intern all keywords into the given interner.
    pub(crate) fn pre_intern(interner: &Interner) {
        for &kw in KEYWORD_STRS {
            interner.intern_static(kw);
        }
    }

    /// Check if a symbol corresponds to a Doo keyword.
    pub fn is_keyword(sym: Symbol) -> bool {
        KEYWORD_STRS
            .iter()
            .any(|&kw| crate::intern::intern_static(kw) == sym)
    }

    /// Check if a string is a Doo keyword.
    pub fn is_keyword_str(s: &str) -> bool {
        KEYWORD_STRS.contains(&s)
    }

    /// Get the symbol for a keyword.
    #[inline]
    pub fn get(s: &'static str) -> Symbol {
        crate::intern::kw(s)
    }
}
