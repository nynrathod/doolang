//! Production-grade limits and bounds for the Doo compiler
//!
//! This module defines all resource limits to prevent:
//! - Stack overflow from deep recursion
//! - Out-of-memory from unbounded allocations
//! - Denial-of-service attacks from malicious input
//!
//! These limits are inspired by production compilers:
//! - We use conservative limits for stability

/// Maximum recursion depth for parsing expressions
/// Reduced for AddressSanitizer compatibility (ASan uses more stack)
pub const PARSER_MAX_DEPTH: usize = 64;

/// Maximum recursion depth for semantic analysis
/// Reduced for AddressSanitizer compatibility
pub const ANALYZER_MAX_DEPTH: usize = 64;

/// Maximum recursion depth for MIR building
/// Reduced for AddressSanitizer compatibility
pub const MIR_MAX_DEPTH: usize = 64;

/// Maximum recursion depth for code generation
/// Reduced for AddressSanitizer compatibility
pub const CODEGEN_MAX_DEPTH: usize = 64;

/// Maximum identifier length in bytes
/// Prevents OOM from UTF-8 sequences and huge identifiers
/// Reduced to prevent memory accumulation from Box::leak
pub const LEXER_MAX_IDENTIFIER_LENGTH: usize = 10_000;

/// Maximum string literal length in bytes
/// Prevents OOM from huge string literals
/// Reduced to prevent memory accumulation from Box::leak
pub const LEXER_MAX_STRING_LENGTH: usize = 100_000;

/// Maximum single-line comment length
/// Reduced to prevent memory accumulation
pub const LEXER_MAX_COMMENT_LENGTH: usize = 100_000;

/// Maximum total tokens in a program
/// Reduced to prevent OOM from token explosion during fuzzing
/// Box::leak causes memory accumulation, so keep this very low for fuzzing
/// Reasonable programs should have far fewer tokens
pub const LEXER_MAX_TOKEN_COUNT: usize = 10_000;

/// Maximum array literal size during parsing
/// Prevents allocation of massive arrays
pub const PARSER_MAX_ARRAY_SIZE: usize = 100_000;

/// Maximum map literal size during parsing
pub const PARSER_MAX_MAP_SIZE: usize = 100_000;

/// Maximum characters to process in a single lexer invocation
/// Reduced to prevent fuzzer OOM issues and Box::leak accumulation
pub const LEXER_MAX_INPUT_SIZE: usize = 100_000;

/// Maximum AST node count in a program
/// Helps catch exponentially-growing AST structures
pub const ANALYZER_MAX_AST_NODES: usize = 100_000;

/// Maximum function nesting depth
pub const ANALYZER_MAX_FUNCTION_DEPTH: usize = 64;

/// Maximum scope nesting depth
pub const ANALYZER_MAX_SCOPE_DEPTH: usize = 256;

/// Maximum loop nesting depth
pub const ANALYZER_MAX_LOOP_DEPTH: usize = 64;

/// Maximum symbol table size per scope
pub const ANALYZER_MAX_SCOPE_SYMBOLS: usize = 100_000;

/// Validation helper - should be called at program entry
pub fn validate_limits() {
    // Ensure limits are sensible
    debug_assert!(PARSER_MAX_DEPTH > 0);
    debug_assert!(ANALYZER_MAX_DEPTH > 0);
    debug_assert!(MIR_MAX_DEPTH > 0);
    debug_assert!(CODEGEN_MAX_DEPTH > 0);
    debug_assert!(LEXER_MAX_STRING_LENGTH > 0);
    debug_assert!(LEXER_MAX_COMMENT_LENGTH > 0);
    debug_assert!(LEXER_MAX_TOKEN_COUNT > 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_limits_are_positive() {
        assert!(PARSER_MAX_DEPTH > 0);
        assert!(ANALYZER_MAX_DEPTH > 0);
        assert!(MIR_MAX_DEPTH > 0);
        assert!(CODEGEN_MAX_DEPTH > 0);
        assert!(LEXER_MAX_STRING_LENGTH > 0);
        assert!(LEXER_MAX_COMMENT_LENGTH > 0);
    }

    #[test]
    fn test_limits_are_reasonable() {
        // Recursion limits should be similar
        assert!(PARSER_MAX_DEPTH < 256);
        assert!(ANALYZER_MAX_DEPTH < 256);
        assert!(MIR_MAX_DEPTH < 256);
        assert!(CODEGEN_MAX_DEPTH < 256);
    }
}
