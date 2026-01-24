//! Type Compatibility
//!
//! Checks if types are compatible for operations.

use doo_core::types::{TypeId, builtin};

/// Type compatibility checker.
pub struct TypeCompat;

impl TypeCompat {
    /// Check if source can be assigned to target.
    pub fn assignable(source: TypeId, target: TypeId) -> bool {
        if source == target {
            return true;
        }
        // Any is compatible with everything
        if target == builtin::ANY || source == builtin::ANY {
            return true;
        }
        // Numeric coercion: Int -> Float
        if source == builtin::INT && target == builtin::FLOAT {
            return true;
        }
        false
    }

    /// Check if types can be compared with ==, !=.
    pub fn comparable(a: TypeId, b: TypeId) -> bool {
        if a == b {
            return true;
        }
        // Numeric types can be compared
        let numeric = [builtin::INT, builtin::FLOAT];
        if numeric.contains(&a) && numeric.contains(&b) {
            return true;
        }
        false
    }

    /// Check if types can be used in arithmetic (+, -, *, /).
    pub fn arithmetic(a: TypeId, b: TypeId) -> bool {
        let numeric = [builtin::INT, builtin::FLOAT];
        numeric.contains(&a) && numeric.contains(&b)
    }

    /// Check if type supports string concatenation.
    pub fn string_concat(a: TypeId, b: TypeId) -> bool {
        a == builtin::STR && b == builtin::STR
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_same_type_assignable() {
        assert!(TypeCompat::assignable(builtin::INT, builtin::INT));
    }

    #[test]
    fn test_int_to_float() {
        assert!(TypeCompat::assignable(builtin::INT, builtin::FLOAT));
    }

    #[test]
    fn test_any_compatible() {
        assert!(TypeCompat::assignable(builtin::STR, builtin::ANY));
        assert!(TypeCompat::assignable(builtin::ANY, builtin::INT));
    }
}
