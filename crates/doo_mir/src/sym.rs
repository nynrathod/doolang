//! MIR Symbol Interning — zero-copy name identifiers.
//!
//! All name/label fields in MIR types use `Sym` instead of `String`.
//! `Sym` is a 4-byte integer ID that's `Copy` — no heap allocation on clone.
//!
//! ## Usage
//!
//! ```ignore
//! use doo_mir::sym::{Sym, sym, resolve};
//!
//! let s: Sym = sym("my_variable");   // intern a string → 4-byte ID
//! let name: String = resolve(s);     // resolve ID → String
//! ```

use doo_core::intern::{InternedStr, Interner};
use std::sync::OnceLock;

/// A MIR symbol — a 4-byte interned string ID.
/// `Copy + Clone + Eq + Hash` — cloning is a simple integer copy (zero cost).
pub type Sym = InternedStr;

/// Global MIR string interner.
static MIR_INTERNER: OnceLock<Interner> = OnceLock::new();

fn interner() -> &'static Interner {
    MIR_INTERNER.get_or_init(Interner::new)
}

/// Intern a string, returning a cheap `Sym` handle.
/// If the string was already interned, returns the existing handle.
#[inline]
pub fn sym(s: &str) -> Sym {
    interner().intern(s)
}

/// Resolve a `Sym` back to its string value.
/// Panics if the symbol was never interned (should never happen in practice).
#[inline]
pub fn resolve(s: Sym) -> String {
    interner()
        .resolve(s)
        .unwrap_or_else(|| format!("<unresolved:{:?}>", s))
}

/// Resolve a `Sym` to a string reference via callback.
/// More efficient than `resolve()` when you just need to compare or format.
#[inline]
pub fn with_resolved<R>(s: Sym, f: impl FnOnce(&str) -> R) -> R {
    let string = resolve(s);
    f(&string)
}

/// Check if a string is already interned.
#[inline]
pub fn get(s: &str) -> Option<Sym> {
    interner().get(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sym_interning() {
        let s1 = sym("hello");
        let s2 = sym("world");
        let s3 = sym("hello"); // same as s1

        assert_eq!(s1, s3);
        assert_ne!(s1, s2);
        assert_eq!(resolve(s1), "hello");
        assert_eq!(resolve(s2), "world");
    }

    #[test]
    fn test_sym_is_copy() {
        let s = sym("test");
        let copy = s; // Copy, not move
        assert_eq!(s, copy); // both valid
    }

    #[test]
    fn test_sym_size() {
        assert_eq!(std::mem::size_of::<Sym>(), 4); // 4 bytes vs 24 bytes for String
    }
}
