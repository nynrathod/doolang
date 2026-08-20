//! Compiler-internal names — single source of truth for mangling.
//!
//! Method functions lowered from `impl Type { fn method }` are named
//! `_method_{Type}_{method}` everywhere (HIR, MIR, codegen). Rust does
//! the same kind of symbol mangling in one place, not per crate.

/// Prefix for inherent/impl method symbols.
pub const METHOD_NAME_PREFIX: &str = "_method_";

/// Mangle an inherent method to its compiler symbol.
pub fn mangle_method(type_name: &str, method: &str) -> String {
    format!("{}{}_{}", METHOD_NAME_PREFIX, type_name, method)
}

/// Split a mangled method symbol into `(type_name, method)`.
///
/// Type and method names in Doolang do not contain `_` (Core Decision §1),
/// so the last underscore is the type/method boundary.
pub fn demangle_method(mangled: &str) -> Option<(&str, &str)> {
    let rest = mangled.strip_prefix(METHOD_NAME_PREFIX)?;
    let idx = rest.rfind('_')?;
    if idx == 0 || idx + 1 >= rest.len() {
        return None;
    }
    Some((&rest[..idx], &rest[idx + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mangle_roundtrip() {
        let name = mangle_method("Str", "len");
        assert_eq!(name, "_method_Str_len");
        assert_eq!(demangle_method(&name), Some(("Str", "len")));
    }
}
