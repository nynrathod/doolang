//! String Case Conversion — Single Source of Truth
//!
//! Centralized case conversion utilities used across FFI crates.
//! Extracted from `doo_core` per Compiler↔Framework Separation Audit §3.1.
//!
//! FFI crates must NOT depend on `doo_core`. These utilities live here
//! (`doo_ffi_core`) because both `doo_ffi_http` and `doo_ffi_db` already
//! depend on this crate as a Tier A runtime dependency.

/// Convert PascalCase or camelCase to snake_case.
///
/// Examples:
/// - "AuthorId" → "author_id"
/// - "firstName" → "first_name"
/// - "createdAt" → "created_at"
/// - "ID" → "i_d" (each uppercase char gets an underscore prefix)
pub fn to_snake_case(name: &str) -> String {
    let mut result = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(ch.to_lowercase().next().unwrap_or(ch));
        } else {
            result.push(ch);
        }
    }
    result
}

/// Convert snake_case to PascalCase.
///
/// Examples:
/// - "author_id" → "AuthorId"
/// - "first_name" → "FirstName"
/// - "id" → "id" (special case: stays lowercase per Doo convention)
pub fn to_pascal_case(s: &str) -> String {
    // Special case: "id" stays lowercase (Doo convention)
    if s == "id" {
        return "id".to_string();
    }

    let mut result = String::with_capacity(s.len());
    let mut capitalize = true;
    for ch in s.chars() {
        if ch == '_' {
            capitalize = true;
        } else if capitalize {
            result.push(ch.to_uppercase().next().unwrap_or(ch));
            capitalize = false;
        } else {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("AuthorId"), "author_id");
        assert_eq!(to_snake_case("firstName"), "first_name");
        assert_eq!(to_snake_case("createdAt"), "created_at");
        assert_eq!(to_snake_case("id"), "id");
        assert_eq!(to_snake_case("User"), "user");
        assert_eq!(to_snake_case("HTTPResponse"), "h_t_t_p_response");
        assert_eq!(to_snake_case("ABC"), "a_b_c");
    }

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("author_id"), "AuthorId");
        assert_eq!(to_pascal_case("first_name"), "FirstName");
        assert_eq!(to_pascal_case("id"), "id"); // special case
        assert_eq!(to_pascal_case("user"), "User");
        assert_eq!(to_pascal_case("created_at"), "CreatedAt");
    }
}
