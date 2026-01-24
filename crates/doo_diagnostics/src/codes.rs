//! Error Codes
//!
//! Centralized error codes for all compiler phases.
//! Single source of truth for error identification.

use serde::{Deserialize, Serialize};

/// Error category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCategory {
    /// Syntax errors (E0001-E0099)
    Syntax,
    /// Type errors (E0100-E0199)
    Type,
    /// Ownership errors (E0200-E0299)
    Ownership,
    /// Name resolution errors (E0300-E0399)
    Name,
    /// Borrow errors (E0400-E0499)
    Borrow,
    /// FFI errors (E0500-E0599)
    Ffi,
    /// Runtime errors (E0600-E0699)
    Runtime,
    /// Internal compiler errors (E9000-E9999)
    Internal,
}

/// Error code enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    // === Syntax E0001-E0099 ===
    /// Unexpected token
    E0001,
    /// Unclosed delimiter
    E0002,
    /// Invalid literal
    E0003,
    /// Missing semicolon
    E0004,
    /// Invalid escape sequence
    E0005,
    /// Unterminated string
    E0006,
    /// Invalid number
    E0007,
    /// Reserved keyword
    E0008,

    // === Type E0100-E0199 ===
    /// Type mismatch
    E0100,
    /// Unknown type
    E0101,
    /// Cannot infer type
    E0102,
    /// Invalid cast
    E0103,
    /// Missing return type
    E0104,
    /// Invalid array element type
    E0105,
    /// Invalid map key type
    E0106,
    /// Generic type error
    E0107,

    // === Ownership E0200-E0299 ===
    /// Use after move
    E0200,
    /// Cannot move out of borrowed
    E0201,
    /// Cannot return local reference
    E0202,
    /// Double move
    E0203,

    // === Name E0300-E0399 ===
    /// Undefined variable
    E0300,
    /// Undefined function
    E0301,
    /// Undefined type
    E0302,
    /// Undefined field
    E0303,
    /// Undefined method
    E0304,
    /// Duplicate definition
    E0305,
    /// Private access
    E0306,

    // === Borrow E0400-E0499 ===
    /// Concurrent mutable borrow (THE main user-facing error)
    E0400,
    /// Borrow of moved value
    E0401,
    /// Cannot borrow as mutable
    E0402,
    /// Borrow conflict
    E0403,

    // === FFI E0500-E0599 ===
    /// FFI function not found
    E0500,
    /// FFI type mismatch
    E0501,
    /// FFI call failed
    E0502,

    // === Runtime E0600-E0699 ===
    /// Division by zero
    E0600,
    /// Index out of bounds
    E0601,
    /// Null pointer
    E0602,
    /// Stack overflow
    E0603,

    // === Internal E9000-E9999 ===
    /// Internal compiler error
    E9000,
    /// Unimplemented feature
    E9001,
}

impl ErrorCode {
    /// Get the numeric code.
    pub fn code(&self) -> u16 {
        match self {
            Self::E0001 => 1,
            Self::E0002 => 2,
            Self::E0003 => 3,
            Self::E0004 => 4,
            Self::E0005 => 5,
            Self::E0006 => 6,
            Self::E0007 => 7,
            Self::E0008 => 8,
            Self::E0100 => 100,
            Self::E0101 => 101,
            Self::E0102 => 102,
            Self::E0103 => 103,
            Self::E0104 => 104,
            Self::E0105 => 105,
            Self::E0106 => 106,
            Self::E0107 => 107,
            Self::E0200 => 200,
            Self::E0201 => 201,
            Self::E0202 => 202,
            Self::E0203 => 203,
            Self::E0300 => 300,
            Self::E0301 => 301,
            Self::E0302 => 302,
            Self::E0303 => 303,
            Self::E0304 => 304,
            Self::E0305 => 305,
            Self::E0306 => 306,
            Self::E0400 => 400,
            Self::E0401 => 401,
            Self::E0402 => 402,
            Self::E0403 => 403,
            Self::E0500 => 500,
            Self::E0501 => 501,
            Self::E0502 => 502,
            Self::E0600 => 600,
            Self::E0601 => 601,
            Self::E0602 => 602,
            Self::E0603 => 603,
            Self::E9000 => 9000,
            Self::E9001 => 9001,
        }
    }

    /// Get the category.
    pub fn category(&self) -> ErrorCategory {
        let code = self.code();
        match code {
            1..=99 => ErrorCategory::Syntax,
            100..=199 => ErrorCategory::Type,
            200..=299 => ErrorCategory::Ownership,
            300..=399 => ErrorCategory::Name,
            400..=499 => ErrorCategory::Borrow,
            500..=599 => ErrorCategory::Ffi,
            600..=699 => ErrorCategory::Runtime,
            _ => ErrorCategory::Internal,
        }
    }

    /// Get the short message.
    pub fn message(&self) -> &'static str {
        match self {
            Self::E0001 => "UNEXPECTED TOKEN",
            Self::E0002 => "UNCLOSED DELIMITER",
            Self::E0003 => "INVALID LITERAL",
            Self::E0004 => "MISSING SEMICOLON",
            Self::E0005 => "INVALID ESCAPE",
            Self::E0006 => "UNTERMINATED STRING",
            Self::E0007 => "INVALID NUMBER",
            Self::E0008 => "RESERVED KEYWORD",
            Self::E0100 => "TYPE MISMATCH",
            Self::E0101 => "UNKNOWN TYPE",
            Self::E0102 => "CANNOT INFER TYPE",
            Self::E0103 => "INVALID CAST",
            Self::E0104 => "MISSING RETURN TYPE",
            Self::E0105 => "INVALID ARRAY ELEMENT",
            Self::E0106 => "INVALID MAP KEY",
            Self::E0107 => "TYPE ERROR",
            Self::E0200 => "USE AFTER MOVE",
            Self::E0201 => "CANNOT MOVE FROM BORROWED",
            Self::E0202 => "CANNOT RETURN LOCAL REF",
            Self::E0203 => "DOUBLE MOVE",
            Self::E0300 => "UNDEFINED VARIABLE",
            Self::E0301 => "UNDEFINED FUNCTION",
            Self::E0302 => "UNDEFINED TYPE",
            Self::E0303 => "UNDEFINED FIELD",
            Self::E0304 => "UNDEFINED METHOD",
            Self::E0305 => "DUPLICATE DEFINITION",
            Self::E0306 => "PRIVATE ACCESS",
            Self::E0400 => "CONCURRENT MUTABLE BORROW",
            Self::E0401 => "BORROW OF MOVED VALUE",
            Self::E0402 => "CANNOT BORROW AS MUTABLE",
            Self::E0403 => "BORROW CONFLICT",
            Self::E0500 => "FFI FUNCTION NOT FOUND",
            Self::E0501 => "FFI TYPE MISMATCH",
            Self::E0502 => "FFI CALL FAILED",
            Self::E0600 => "DIVISION BY ZERO",
            Self::E0601 => "INDEX OUT OF BOUNDS",
            Self::E0602 => "NULL POINTER",
            Self::E0603 => "STACK OVERFLOW",
            Self::E9000 => "INTERNAL COMPILER ERROR",
            Self::E9001 => "UNIMPLEMENTED FEATURE",
        }
    }

    /// Get the detailed explanation (for --explain).
    pub fn explanation(&self) -> &'static str {
        match self {
            Self::E0400 => {
                "This error occurs when you try to borrow a value mutably while \
                 it is already borrowed. In Doo, you can have either:\n\
                 - Multiple immutable borrows, OR\n\
                 - One mutable borrow\n\n\
                 This is the only ownership-related error users see in Doo. \
                 The compiler handles all other memory management automatically."
            }
            Self::E0300 => {
                "This error occurs when you use a variable that hasn't been \
                 declared. Check for typos in the variable name."
            }
            _ => "No detailed explanation available for this error.",
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "E{:04}", self.code())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_display() {
        assert_eq!(format!("{}", ErrorCode::E0001), "E0001");
        assert_eq!(format!("{}", ErrorCode::E0400), "E0400");
    }

    #[test]
    fn test_error_category() {
        assert_eq!(ErrorCode::E0001.category(), ErrorCategory::Syntax);
        assert_eq!(ErrorCode::E0100.category(), ErrorCategory::Type);
        assert_eq!(ErrorCode::E0400.category(), ErrorCategory::Borrow);
    }
}
