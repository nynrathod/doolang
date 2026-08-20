//! Symbol — an interned string identifier.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::intern;

/// An interned string symbol — a 4-byte handle to a unique string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Symbol(pub u32);

impl Symbol {
    pub const fn new(idx: u32) -> Self {
        Self(idx)
    }
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }
    #[inline]
    pub fn intern(s: &str) -> Self {
        intern::intern(s)
    }
    #[inline]
    pub fn resolve(self) -> &'static str {
        intern::resolve(self)
    }
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

pub mod keywords {
    use super::Symbol;
    use crate::intern::Interner;

    pub const KEYWORD_STRS: &[&str] = &[
        "fn", "let", "const", "static", "mut", "if", "else", "return", "for", "while", "in",
        "match", "break", "continue", "struct", "enum", "use", "import", "as", "true", "false",
        "null", "async", "await", "go", "try", "catch", "throw", "scope", "route", "ws",
    ];

    pub(crate) fn pre_intern(interner: &Interner) {
        for &kw in KEYWORD_STRS {
            interner.intern_static(kw);
        }
    }

    pub fn is_keyword(sym: Symbol) -> bool {
        KEYWORD_STRS
            .iter()
            .any(|&kw| crate::intern::intern_static(kw) == sym)
    }

    pub fn is_keyword_str(s: &str) -> bool {
        KEYWORD_STRS.contains(&s)
    }

    #[inline]
    pub fn get(s: &'static str) -> Symbol {
        crate::intern::kw(s)
    }
}
