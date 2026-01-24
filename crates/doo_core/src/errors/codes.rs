//! Error codes and compiler error types.
//!
//! Error codes are organized by category:
//! - E0001-E0099: Syntax errors
//! - E0100-E0199: Type errors
//! - E0200-E0299: Ownership/borrow errors
//! - E0300-E0399: Name resolution errors
//! - E0400-E0499: Declaration errors
//! - E0500-E0599: Import errors
//! - E0600-E0699: HTTP/FFI errors
//! - E0700-E0799: Database errors
//! - E0800-E0899: Validation errors

use crate::span::Span;
use serde::{Deserialize, Serialize};

/// Error code enumeration.
///
/// Each variant corresponds to a specific error with a unique code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorCode {
    // === Syntax Errors (E0001-E0099) ===
    /// E0001: Unexpected token encountered.
    UnexpectedToken,
    /// E0002: Unclosed delimiter (paren, brace, bracket).
    UnclosedDelimiter,
    /// E0003: Invalid number literal.
    InvalidNumberLiteral,
    /// E0004: Invalid string literal.
    InvalidStringLiteral,
    /// E0005: Invalid escape sequence.
    InvalidEscapeSequence,
    /// E0006: Unterminated string.
    UnterminatedString,
    /// E0007: Invalid character in source.
    InvalidCharacter,
    /// E0008: Missing semicolon.
    MissingSemicolon,
    /// E0009: Invalid expression.
    InvalidExpression,
    /// E0010: Expected identifier.
    ExpectedIdentifier,

    // === Type Errors (E0100-E0199) ===
    /// E0100: Type mismatch.
    TypeMismatch,
    /// E0101: Unknown type name.
    UnknownType,
    /// E0102: Cannot infer type.
    CannotInferType,
    /// E0103: Type annotation required.
    TypeAnnotationRequired,
    /// E0104: Incompatible types in binary operation.
    IncompatibleTypes,
    /// E0105: Cannot convert between types.
    CannotConvert,
    /// E0106: Invalid return type.
    InvalidReturnType,
    /// E0107: Mismatched tuple length.
    TupleLengthMismatch,
    /// E0108: Wrong number of type parameters.
    TypeParameterCount,
    /// E0109: Invalid type for operation.
    InvalidTypeForOperation,

    // === Ownership/Borrow Errors (E0200-E0299) ===
    /// E0200: Use of moved value.
    UseAfterMove,
    /// E0201: Cannot borrow mutably while already borrowed.
    ConcurrentMutableBorrow,
    /// E0202: Cannot borrow immutably while mutably borrowed.
    BorrowWhileMutablyBorrowed,
    /// E0203: Attempt to use dropped value.
    UseAfterDrop,
    /// E0204: Cannot move out of borrowed context.
    CannotMoveFromBorrowed,
    /// E0205: Assignment to immutable variable.
    AssignToImmutable,

    // === Name Resolution Errors (E0300-E0399) ===
    /// E0300: Undefined variable.
    UndefinedVariable,
    /// E0301: Undefined function.
    UndefinedFunction,
    /// E0302: Undefined type.
    UndefinedType,
    /// E0303: Undefined field.
    UndefinedField,
    /// E0304: Undefined method.
    UndefinedMethod,
    /// E0305: Undefined enum variant.
    UndefinedVariant,
    /// E0306: Name already defined.
    NameAlreadyDefined,
    /// E0307: Private item accessed from outside module.
    PrivateItemAccess,
    /// E0308: Invalid path.
    InvalidPath,

    // === Declaration Errors (E0400-E0499) ===
    /// E0400: Duplicate function parameter.
    DuplicateParameter,
    /// E0401: Duplicate struct field.
    DuplicateField,
    /// E0402: Duplicate enum variant.
    DuplicateVariant,
    /// E0403: Invalid function signature.
    InvalidSignature,
    /// E0404: Missing return statement.
    MissingReturn,
    /// E0405: Unreachable code after return.
    UnreachableCode,
    /// E0406: Invalid decorator usage.
    InvalidDecorator,
    /// E0407: Conflicting decorators.
    ConflictingDecorators,
    /// E0408: Invalid default value.
    InvalidDefaultValue,

    // === Import Errors (E0500-E0599) ===
    /// E0500: Module not found.
    ModuleNotFound,
    /// E0501: Import not found in module.
    ImportNotFound,
    /// E0502: Circular import detected.
    CircularImport,
    /// E0503: Invalid import path.
    InvalidImportPath,
    /// E0504: Private import (trying to import private item).
    PrivateImport,

    // === HTTP/FFI Errors (E0600-E0699) ===
    /// E0600: Invalid route handler signature.
    InvalidRouteHandler,
    /// E0601: Route already defined.
    DuplicateRoute,
    /// E0602: Invalid middleware.
    InvalidMiddleware,
    /// E0603: Invalid CORS config.
    InvalidCorsConfig,
    /// E0604: Invalid rate limit config.
    InvalidRateLimit,
    /// E0605: FFI function not found.
    FfiFunctionNotFound,
    /// E0606: FFI type mismatch.
    FfiTypeMismatch,

    // === Database Errors (E0700-E0799) ===
    /// E0700: Database not connected.
    DatabaseNotConnected,
    /// E0701: Invalid SQL query.
    InvalidSqlQuery,
    /// E0702: Model not found.
    ModelNotFound,
    /// E0703: Migration failed.
    MigrationFailed,
    /// E0704: Invalid database URL.
    InvalidDatabaseUrl,

    // === Validation Errors (E0800-E0899) ===
    /// E0800: Invalid email format.
    InvalidEmail,
    /// E0801: Invalid URL format.
    InvalidUrl,
    /// E0802: Value below minimum.
    BelowMinimum,
    /// E0803: Value above maximum.
    AboveMaximum,
    /// E0804: Pattern not matched.
    PatternNotMatched,
    /// E0805: Required field missing.
    RequiredFieldMissing,

    // === Internal Errors (E0900-E0999) ===
    /// E0900: Internal compiler error.
    InternalError,
    /// E0901: Code generation failed.
    CodegenFailed,
    /// E0902: LLVM error.
    LlvmError,
    /// E0903: IO error.
    IoError,
}

impl ErrorCode {
    /// Get the numeric code as a string (e.g., "E0001").
    pub fn code(&self) -> &'static str {
        match self {
            // Syntax
            Self::UnexpectedToken => "E0001",
            Self::UnclosedDelimiter => "E0002",
            Self::InvalidNumberLiteral => "E0003",
            Self::InvalidStringLiteral => "E0004",
            Self::InvalidEscapeSequence => "E0005",
            Self::UnterminatedString => "E0006",
            Self::InvalidCharacter => "E0007",
            Self::MissingSemicolon => "E0008",
            Self::InvalidExpression => "E0009",
            Self::ExpectedIdentifier => "E0010",

            // Type
            Self::TypeMismatch => "E0100",
            Self::UnknownType => "E0101",
            Self::CannotInferType => "E0102",
            Self::TypeAnnotationRequired => "E0103",
            Self::IncompatibleTypes => "E0104",
            Self::CannotConvert => "E0105",
            Self::InvalidReturnType => "E0106",
            Self::TupleLengthMismatch => "E0107",
            Self::TypeParameterCount => "E0108",
            Self::InvalidTypeForOperation => "E0109",

            // Ownership
            Self::UseAfterMove => "E0200",
            Self::ConcurrentMutableBorrow => "E0201",
            Self::BorrowWhileMutablyBorrowed => "E0202",
            Self::UseAfterDrop => "E0203",
            Self::CannotMoveFromBorrowed => "E0204",
            Self::AssignToImmutable => "E0205",

            // Names
            Self::UndefinedVariable => "E0300",
            Self::UndefinedFunction => "E0301",
            Self::UndefinedType => "E0302",
            Self::UndefinedField => "E0303",
            Self::UndefinedMethod => "E0304",
            Self::UndefinedVariant => "E0305",
            Self::NameAlreadyDefined => "E0306",
            Self::PrivateItemAccess => "E0307",
            Self::InvalidPath => "E0308",

            // Declarations
            Self::DuplicateParameter => "E0400",
            Self::DuplicateField => "E0401",
            Self::DuplicateVariant => "E0402",
            Self::InvalidSignature => "E0403",
            Self::MissingReturn => "E0404",
            Self::UnreachableCode => "E0405",
            Self::InvalidDecorator => "E0406",
            Self::ConflictingDecorators => "E0407",
            Self::InvalidDefaultValue => "E0408",

            // Imports
            Self::ModuleNotFound => "E0500",
            Self::ImportNotFound => "E0501",
            Self::CircularImport => "E0502",
            Self::InvalidImportPath => "E0503",
            Self::PrivateImport => "E0504",

            // HTTP/FFI
            Self::InvalidRouteHandler => "E0600",
            Self::DuplicateRoute => "E0601",
            Self::InvalidMiddleware => "E0602",
            Self::InvalidCorsConfig => "E0603",
            Self::InvalidRateLimit => "E0604",
            Self::FfiFunctionNotFound => "E0605",
            Self::FfiTypeMismatch => "E0606",

            // Database
            Self::DatabaseNotConnected => "E0700",
            Self::InvalidSqlQuery => "E0701",
            Self::ModelNotFound => "E0702",
            Self::MigrationFailed => "E0703",
            Self::InvalidDatabaseUrl => "E0704",

            // Validation
            Self::InvalidEmail => "E0800",
            Self::InvalidUrl => "E0801",
            Self::BelowMinimum => "E0802",
            Self::AboveMaximum => "E0803",
            Self::PatternNotMatched => "E0804",
            Self::RequiredFieldMissing => "E0805",

            // Internal
            Self::InternalError => "E0900",
            Self::CodegenFailed => "E0901",
            Self::LlvmError => "E0902",
            Self::IoError => "E0903",
        }
    }

    /// Get the short title for this error.
    pub fn title(&self) -> &'static str {
        match self {
            Self::UnexpectedToken => "UNEXPECTED TOKEN",
            Self::UnclosedDelimiter => "UNCLOSED DELIMITER",
            Self::InvalidNumberLiteral => "INVALID NUMBER",
            Self::InvalidStringLiteral => "INVALID STRING",
            Self::InvalidEscapeSequence => "INVALID ESCAPE",
            Self::UnterminatedString => "UNTERMINATED STRING",
            Self::InvalidCharacter => "INVALID CHARACTER",
            Self::MissingSemicolon => "MISSING SEMICOLON",
            Self::InvalidExpression => "INVALID EXPRESSION",
            Self::ExpectedIdentifier => "EXPECTED IDENTIFIER",

            Self::TypeMismatch => "TYPE MISMATCH",
            Self::UnknownType => "UNKNOWN TYPE",
            Self::CannotInferType => "CANNOT INFER TYPE",
            Self::TypeAnnotationRequired => "TYPE ANNOTATION REQUIRED",
            Self::IncompatibleTypes => "INCOMPATIBLE TYPES",
            Self::CannotConvert => "CANNOT CONVERT",
            Self::InvalidReturnType => "INVALID RETURN TYPE",
            Self::TupleLengthMismatch => "TUPLE LENGTH MISMATCH",
            Self::TypeParameterCount => "TYPE PARAMETER COUNT",
            Self::InvalidTypeForOperation => "INVALID TYPE FOR OPERATION",

            Self::UseAfterMove => "USE AFTER MOVE",
            Self::ConcurrentMutableBorrow => "CONCURRENT MUTABLE BORROW",
            Self::BorrowWhileMutablyBorrowed => "BORROW CONFLICT",
            Self::UseAfterDrop => "USE AFTER DROP",
            Self::CannotMoveFromBorrowed => "CANNOT MOVE",
            Self::AssignToImmutable => "IMMUTABLE ASSIGNMENT",

            Self::UndefinedVariable => "UNDEFINED VARIABLE",
            Self::UndefinedFunction => "UNDEFINED FUNCTION",
            Self::UndefinedType => "UNDEFINED TYPE",
            Self::UndefinedField => "UNDEFINED FIELD",
            Self::UndefinedMethod => "UNDEFINED METHOD",
            Self::UndefinedVariant => "UNDEFINED VARIANT",
            Self::NameAlreadyDefined => "ALREADY DEFINED",
            Self::PrivateItemAccess => "PRIVATE ACCESS",
            Self::InvalidPath => "INVALID PATH",

            Self::DuplicateParameter => "DUPLICATE PARAMETER",
            Self::DuplicateField => "DUPLICATE FIELD",
            Self::DuplicateVariant => "DUPLICATE VARIANT",
            Self::InvalidSignature => "INVALID SIGNATURE",
            Self::MissingReturn => "MISSING RETURN",
            Self::UnreachableCode => "UNREACHABLE CODE",
            Self::InvalidDecorator => "INVALID DECORATOR",
            Self::ConflictingDecorators => "CONFLICTING DECORATORS",
            Self::InvalidDefaultValue => "INVALID DEFAULT",

            Self::ModuleNotFound => "MODULE NOT FOUND",
            Self::ImportNotFound => "IMPORT NOT FOUND",
            Self::CircularImport => "CIRCULAR IMPORT",
            Self::InvalidImportPath => "INVALID IMPORT",
            Self::PrivateImport => "PRIVATE IMPORT",

            Self::InvalidRouteHandler => "INVALID HANDLER",
            Self::DuplicateRoute => "DUPLICATE ROUTE",
            Self::InvalidMiddleware => "INVALID MIDDLEWARE",
            Self::InvalidCorsConfig => "INVALID CORS",
            Self::InvalidRateLimit => "INVALID RATE LIMIT",
            Self::FfiFunctionNotFound => "FFI NOT FOUND",
            Self::FfiTypeMismatch => "FFI TYPE MISMATCH",

            Self::DatabaseNotConnected => "DB NOT CONNECTED",
            Self::InvalidSqlQuery => "INVALID SQL",
            Self::ModelNotFound => "MODEL NOT FOUND",
            Self::MigrationFailed => "MIGRATION FAILED",
            Self::InvalidDatabaseUrl => "INVALID DB URL",

            Self::InvalidEmail => "INVALID EMAIL",
            Self::InvalidUrl => "INVALID URL",
            Self::BelowMinimum => "BELOW MINIMUM",
            Self::AboveMaximum => "ABOVE MAXIMUM",
            Self::PatternNotMatched => "PATTERN NOT MATCHED",
            Self::RequiredFieldMissing => "REQUIRED FIELD",

            Self::InternalError => "INTERNAL ERROR",
            Self::CodegenFailed => "CODEGEN FAILED",
            Self::LlvmError => "LLVM ERROR",
            Self::IoError => "IO ERROR",
        }
    }

    /// Get the default severity for this error.
    pub fn severity(&self) -> ErrorSeverity {
        match self {
            Self::UnreachableCode => ErrorSeverity::Warning,
            Self::InternalError | Self::CodegenFailed | Self::LlvmError => ErrorSeverity::Ice,
            _ => ErrorSeverity::Error,
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.code(), self.title())
    }
}

/// Severity level of an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorSeverity {
    /// Informational note.
    Note,
    /// Warning (compilation continues).
    Warning,
    /// Error (compilation fails).
    Error,
    /// Internal compiler error (bug).
    Ice,
}

impl std::fmt::Display for ErrorSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Note => write!(f, "note"),
            Self::Warning => write!(f, "warning"),
            Self::Error => write!(f, "error"),
            Self::Ice => write!(f, "internal compiler error"),
        }
    }
}

/// A compiler error with full context.
#[derive(Clone, Debug)]
pub struct CompilerError {
    /// Error code.
    pub code: ErrorCode,
    /// Error message.
    pub message: String,
    /// Primary span (where the error occurred).
    pub span: Span,
    /// Severity level.
    pub severity: ErrorSeverity,
    /// Secondary spans with labels.
    pub labels: Vec<(Span, String)>,
    /// Suggested fix.
    pub suggestion: Option<String>,
    /// Additional notes.
    pub notes: Vec<String>,
}

impl CompilerError {
    /// Create a new error.
    pub fn new(code: ErrorCode, message: impl Into<String>, span: Span) -> Self {
        Self {
            code,
            message: message.into(),
            span,
            severity: code.severity(),
            labels: Vec::new(),
            suggestion: None,
            notes: Vec::new(),
        }
    }

    /// Create an error with custom severity.
    pub fn with_severity(mut self, severity: ErrorSeverity) -> Self {
        self.severity = severity;
        self
    }

    /// Add a labeled span.
    pub fn with_label(mut self, span: Span, label: impl Into<String>) -> Self {
        self.labels.push((span, label.into()));
        self
    }

    /// Add a suggestion.
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    /// Add a note.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Check if this is a fatal error.
    pub fn is_fatal(&self) -> bool {
        matches!(self.severity, ErrorSeverity::Error | ErrorSeverity::Ice)
    }
}

impl std::fmt::Display for CompilerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CompilerError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_codes() {
        assert_eq!(ErrorCode::UnexpectedToken.code(), "E0001");
        assert_eq!(ErrorCode::TypeMismatch.code(), "E0100");
        assert_eq!(ErrorCode::UseAfterMove.code(), "E0200");
        assert_eq!(ErrorCode::UndefinedVariable.code(), "E0300");
    }

    #[test]
    fn test_error_creation() {
        let err = CompilerError::new(
            ErrorCode::UndefinedVariable,
            "variable 'x' is not defined",
            Span::new(0, 10, 11),
        )
        .with_suggestion("did you mean 'y'?")
        .with_note("variables must be declared before use");

        assert_eq!(err.code, ErrorCode::UndefinedVariable);
        assert!(err.suggestion.is_some());
        assert_eq!(err.notes.len(), 1);
        assert!(err.is_fatal());
    }

    #[test]
    fn test_severity() {
        assert_eq!(
            ErrorCode::UnreachableCode.severity(),
            ErrorSeverity::Warning
        );
        assert_eq!(ErrorCode::InternalError.severity(), ErrorSeverity::Ice);
        assert_eq!(ErrorCode::TypeMismatch.severity(), ErrorSeverity::Error);
    }
}
