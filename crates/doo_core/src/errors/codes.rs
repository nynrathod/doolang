//! Error codes and compiler error types.
//!
//! Single source of truth for all compiler error codes.
//! Error codes are organized by category:
//! - E0001-E0099: Syntax errors (lexer + parser)
//! - E0100-E0199: Type errors (analyzer)
//! - E0200-E0299: Ownership errors (only ConcurrentMutableBorrow + AssignToImmutable visible to users)
//! - E0300-E0399: Name resolution errors (analyzer)
//! - E0400-E0499: Declaration / error-flow errors (analyzer)
//! - E0500-E0599: Import errors
//! - E0600-E0699: HTTP/FFI errors
//! - E0700-E0799: Database errors
//! - E0800-E0899: Validation errors
//! - E0900-E0999: Internal errors

use crate::span::Span;
use serde::{Deserialize, Serialize};

/// Error code enumeration — every compiler error has exactly one of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorCode {
    // === Syntax Errors (E0001-E0099) ===
    UnexpectedToken,        // E0001
    UnclosedDelimiter,      // E0002
    InvalidNumberLiteral,   // E0003
    InvalidStringLiteral,   // E0004
    InvalidEscapeSequence,  // E0005
    UnterminatedString,     // E0006
    InvalidCharacter,       // E0007
    MissingSemicolon,       // E0008
    InvalidExpression,      // E0009
    ExpectedIdentifier,     // E0010
    InvalidPattern,         // E0011
    InvalidTypeExpr,        // E0012
    MissingFunctionBody,    // E0013
    InvalidOperator,        // E0014
    UnexpectedEof,          // E0015
    InvalidAssignTarget,    // E0016
    MissingClosingParen,    // E0017
    MissingClosingBrace,    // E0018
    MissingClosingBracket,  // E0019
    ExpectedTypeAnnotation, // E0020
    InvalidDecoratorSyntax, // E0021
    ExpectedBlock,          // E0022
    ExpectedExprAfterOp,    // E0023
    InvalidMatchSyntax,     // E0024
    InvalidForSyntax,       // E0025
    InvalidConstExpr,       // E0026 — const must be a compile-time literal expression

    // === Type Errors (E0100-E0199) ===
    TypeMismatch,            // E0100
    UnknownType,             // E0101
    CannotInferType,         // E0102
    TypeAnnotationRequired,  // E0103
    IncompatibleTypes,       // E0104
    CannotConvert,           // E0105
    InvalidReturnType,       // E0106
    TupleLengthMismatch,     // E0107
    TypeParameterCount,      // E0108
    InvalidTypeForOperation, // E0109
    InvalidConditionType,    // E0110
    InvalidCast,             // E0111
    ReturnTypeMismatch,      // E0112
    ArgCountMismatch,        // E0113
    InvalidArrayElementType, // E0114
    InvalidMapKeyType,       // E0115
    IfElseBranchMismatch,    // E0116
    NilWithNonOptional,      // E0117

    // === Ownership Errors (E0200-E0299) ===
    // Doo auto-handles move/copy/clone/borrow — users only see these two:
    ConcurrentMutableBorrow, // E0201 — THE main user-facing ownership error
    AssignToImmutable,       // E0205 — `x = 5` on a non-mut variable

    // === Name Resolution Errors (E0300-E0399) ===
    UndefinedVariable,  // E0300
    UndefinedFunction,  // E0301
    UndefinedType,      // E0302
    UndefinedField,     // E0303
    UndefinedMethod,    // E0304
    UndefinedVariant,   // E0305
    NameAlreadyDefined, // E0306
    PrivateItemAccess,  // E0307
    InvalidPath,        // E0308

    // === Declaration / Error-Flow Errors (E0400-E0499) ===
    DuplicateParameter,     // E0400
    DuplicateField,         // E0401
    DuplicateVariant,       // E0402
    InvalidSignature,       // E0403
    MissingReturn,          // E0404
    UnreachableCode,        // E0405
    InvalidDecorator,       // E0406
    ConflictingDecorators,  // E0407
    InvalidDefaultValue,    // E0408
    MissingOkReturn,        // E0409  — Result fn missing Ok on some paths
    ErrWithoutErrorType,    // E0410  — Err used in fn without ! E
    TryInNonResultFunction, // E0411  — ? used in fn without ! E (except main)
    UnhandledResult,        // E0412  — Result value ignored
    PanicWithoutMessage,    // E0413  — ?? without message
    MissingStructField,     // E0414  — struct construction missing required field
    UnknownStructField,     // E0415  — struct construction has unknown field
    NonExhaustiveMatch,     // E0416  — match missing patterns
    UnreachablePattern,     // E0417  — match arm never reached
    BreakOutsideLoop,       // E0418
    ContinueOutsideLoop,    // E0419
    ReturnOutsideFunction,  // E0420
    DuplicateMethod,        // E0421  — duplicate method in interface
    DuplicateConst,         // E0422  — duplicate const declaration

    // === Import Errors (E0500-E0599) ===
    ModuleNotFound,    // E0500
    ImportNotFound,    // E0501
    CircularImport,    // E0502
    InvalidImportPath, // E0503
    PrivateImport,     // E0504

    // === HTTP/FFI Errors (E0600-E0699) ===
    InvalidRouteHandler, // E0600
    DuplicateRoute,      // E0601
    InvalidMiddleware,   // E0602
    InvalidCorsConfig,   // E0603
    InvalidRateLimit,    // E0604
    FfiFunctionNotFound, // E0605
    FfiTypeMismatch,     // E0606

    // === Database Errors (E0700-E0799) ===
    DatabaseNotConnected,     // E0700
    InvalidSqlQuery,          // E0701
    ModelNotFound,            // E0702
    MigrationFailed,          // E0703
    InvalidDatabaseUrl,       // E0704
    QueryBuilderUnknownModel, // E0705
    QueryBuilderUnknownField, // E0706
    QueryBuilderMissingWhere, // E0707
    QueryBuilderInvalidChain, // E0708

    // === Validation Errors (E0800-E0899) ===
    InvalidEmail,         // E0800
    InvalidUrl,           // E0801
    BelowMinimum,         // E0802
    AboveMaximum,         // E0803
    PatternNotMatched,    // E0804
    RequiredFieldMissing, // E0805

    // === Internal Errors (E0900-E0999) ===
    InternalError, // E0900
    CodegenFailed, // E0901
    LlvmError,     // E0902
    IoError,       // E0903
}

impl ErrorCode {
    /// Get the numeric code as a string (e.g., "E0001").
    pub fn code(&self) -> &'static str {
        match self {
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
            Self::InvalidPattern => "E0011",
            Self::InvalidTypeExpr => "E0012",
            Self::MissingFunctionBody => "E0013",
            Self::InvalidOperator => "E0014",
            Self::UnexpectedEof => "E0015",
            Self::InvalidAssignTarget => "E0016",
            Self::MissingClosingParen => "E0017",
            Self::MissingClosingBrace => "E0018",
            Self::MissingClosingBracket => "E0019",
            Self::ExpectedTypeAnnotation => "E0020",
            Self::InvalidDecoratorSyntax => "E0021",
            Self::ExpectedBlock => "E0022",
            Self::ExpectedExprAfterOp => "E0023",
            Self::InvalidMatchSyntax => "E0024",
            Self::InvalidForSyntax => "E0025",
            Self::InvalidConstExpr => "E0026",

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
            Self::InvalidConditionType => "E0110",
            Self::InvalidCast => "E0111",
            Self::ReturnTypeMismatch => "E0112",
            Self::ArgCountMismatch => "E0113",
            Self::InvalidArrayElementType => "E0114",
            Self::InvalidMapKeyType => "E0115",
            Self::IfElseBranchMismatch => "E0116",
            Self::NilWithNonOptional => "E0117",

            Self::ConcurrentMutableBorrow => "E0201",
            Self::AssignToImmutable => "E0205",

            Self::UndefinedVariable => "E0300",
            Self::UndefinedFunction => "E0301",
            Self::UndefinedType => "E0302",
            Self::UndefinedField => "E0303",
            Self::UndefinedMethod => "E0304",
            Self::UndefinedVariant => "E0305",
            Self::NameAlreadyDefined => "E0306",
            Self::PrivateItemAccess => "E0307",
            Self::InvalidPath => "E0308",

            Self::DuplicateParameter => "E0400",
            Self::DuplicateField => "E0401",
            Self::DuplicateVariant => "E0402",
            Self::InvalidSignature => "E0403",
            Self::MissingReturn => "E0404",
            Self::UnreachableCode => "E0405",
            Self::InvalidDecorator => "E0406",
            Self::ConflictingDecorators => "E0407",
            Self::InvalidDefaultValue => "E0408",
            Self::MissingOkReturn => "E0409",
            Self::ErrWithoutErrorType => "E0410",
            Self::TryInNonResultFunction => "E0411",
            Self::UnhandledResult => "E0412",
            Self::PanicWithoutMessage => "E0413",
            Self::MissingStructField => "E0414",
            Self::UnknownStructField => "E0415",
            Self::NonExhaustiveMatch => "E0416",
            Self::UnreachablePattern => "E0417",
            Self::BreakOutsideLoop => "E0418",
            Self::ContinueOutsideLoop => "E0419",
            Self::ReturnOutsideFunction => "E0420",
            Self::DuplicateMethod => "E0421",
            Self::DuplicateConst => "E0422",

            Self::ModuleNotFound => "E0500",
            Self::ImportNotFound => "E0501",
            Self::CircularImport => "E0502",
            Self::InvalidImportPath => "E0503",
            Self::PrivateImport => "E0504",

            Self::InvalidRouteHandler => "E0600",
            Self::DuplicateRoute => "E0601",
            Self::InvalidMiddleware => "E0602",
            Self::InvalidCorsConfig => "E0603",
            Self::InvalidRateLimit => "E0604",
            Self::FfiFunctionNotFound => "E0605",
            Self::FfiTypeMismatch => "E0606",

            Self::DatabaseNotConnected => "E0700",
            Self::InvalidSqlQuery => "E0701",
            Self::ModelNotFound => "E0702",
            Self::MigrationFailed => "E0703",
            Self::InvalidDatabaseUrl => "E0704",
            Self::QueryBuilderUnknownModel => "E0705",
            Self::QueryBuilderUnknownField => "E0706",
            Self::QueryBuilderMissingWhere => "E0707",
            Self::QueryBuilderInvalidChain => "E0708",

            Self::InvalidEmail => "E0800",
            Self::InvalidUrl => "E0801",
            Self::BelowMinimum => "E0802",
            Self::AboveMaximum => "E0803",
            Self::PatternNotMatched => "E0804",
            Self::RequiredFieldMissing => "E0805",

            Self::InternalError => "E0900",
            Self::CodegenFailed => "E0901",
            Self::LlvmError => "E0902",
            Self::IoError => "E0903",
        }
    }

    /// Parse an error code string (e.g., "E0100") back to an ErrorCode variant.
    pub fn from_code(code: &str) -> Option<ErrorCode> {
        match code {
            "E0001" => Some(Self::UnexpectedToken),
            "E0002" => Some(Self::UnclosedDelimiter),
            "E0003" => Some(Self::InvalidNumberLiteral),
            "E0004" => Some(Self::InvalidStringLiteral),
            "E0005" => Some(Self::InvalidEscapeSequence),
            "E0006" => Some(Self::UnterminatedString),
            "E0007" => Some(Self::InvalidCharacter),
            "E0008" => Some(Self::MissingSemicolon),
            "E0009" => Some(Self::InvalidExpression),
            "E0010" => Some(Self::ExpectedIdentifier),
            "E0011" => Some(Self::InvalidPattern),
            "E0012" => Some(Self::InvalidTypeExpr),
            "E0013" => Some(Self::MissingFunctionBody),
            "E0014" => Some(Self::InvalidOperator),
            "E0015" => Some(Self::UnexpectedEof),
            "E0016" => Some(Self::InvalidAssignTarget),
            "E0017" => Some(Self::MissingClosingParen),
            "E0018" => Some(Self::MissingClosingBrace),
            "E0019" => Some(Self::MissingClosingBracket),
            "E0020" => Some(Self::ExpectedTypeAnnotation),
            "E0021" => Some(Self::InvalidDecoratorSyntax),
            "E0022" => Some(Self::ExpectedBlock),
            "E0023" => Some(Self::ExpectedExprAfterOp),
            "E0024" => Some(Self::InvalidMatchSyntax),
            "E0025" => Some(Self::InvalidForSyntax),
            "E0026" => Some(Self::InvalidConstExpr),

            "E0100" => Some(Self::TypeMismatch),
            "E0101" => Some(Self::UnknownType),
            "E0102" => Some(Self::CannotInferType),
            "E0103" => Some(Self::TypeAnnotationRequired),
            "E0104" => Some(Self::IncompatibleTypes),
            "E0105" => Some(Self::CannotConvert),
            "E0106" => Some(Self::InvalidReturnType),
            "E0107" => Some(Self::TupleLengthMismatch),
            "E0108" => Some(Self::TypeParameterCount),
            "E0109" => Some(Self::InvalidTypeForOperation),
            "E0110" => Some(Self::InvalidConditionType),
            "E0111" => Some(Self::InvalidCast),
            "E0112" => Some(Self::ReturnTypeMismatch),
            "E0113" => Some(Self::ArgCountMismatch),
            "E0114" => Some(Self::InvalidArrayElementType),
            "E0115" => Some(Self::InvalidMapKeyType),
            "E0116" => Some(Self::IfElseBranchMismatch),
            "E0117" => Some(Self::NilWithNonOptional),

            "E0201" => Some(Self::ConcurrentMutableBorrow),
            "E0205" => Some(Self::AssignToImmutable),

            "E0300" => Some(Self::UndefinedVariable),
            "E0301" => Some(Self::UndefinedFunction),
            "E0302" => Some(Self::UndefinedType),
            "E0303" => Some(Self::UndefinedField),
            "E0304" => Some(Self::UndefinedMethod),
            "E0305" => Some(Self::UndefinedVariant),
            "E0306" => Some(Self::NameAlreadyDefined),
            "E0307" => Some(Self::PrivateItemAccess),
            "E0308" => Some(Self::InvalidPath),

            "E0400" => Some(Self::DuplicateParameter),
            "E0401" => Some(Self::DuplicateField),
            "E0402" => Some(Self::DuplicateVariant),
            "E0403" => Some(Self::InvalidSignature),
            "E0404" => Some(Self::MissingReturn),
            "E0405" => Some(Self::UnreachableCode),
            "E0406" => Some(Self::InvalidDecorator),
            "E0407" => Some(Self::ConflictingDecorators),
            "E0408" => Some(Self::InvalidDefaultValue),
            "E0409" => Some(Self::MissingOkReturn),
            "E0410" => Some(Self::ErrWithoutErrorType),
            "E0411" => Some(Self::TryInNonResultFunction),
            "E0412" => Some(Self::UnhandledResult),
            "E0413" => Some(Self::PanicWithoutMessage),
            "E0414" => Some(Self::MissingStructField),
            "E0415" => Some(Self::UnknownStructField),
            "E0416" => Some(Self::NonExhaustiveMatch),
            "E0417" => Some(Self::UnreachablePattern),
            "E0418" => Some(Self::BreakOutsideLoop),
            "E0419" => Some(Self::ContinueOutsideLoop),
            "E0420" => Some(Self::ReturnOutsideFunction),
            "E0421" => Some(Self::DuplicateMethod),
            "E0422" => Some(Self::DuplicateConst),

            "E0500" => Some(Self::ModuleNotFound),
            "E0501" => Some(Self::ImportNotFound),
            "E0502" => Some(Self::CircularImport),
            "E0503" => Some(Self::InvalidImportPath),
            "E0504" => Some(Self::PrivateImport),

            "E0600" => Some(Self::InvalidRouteHandler),
            "E0601" => Some(Self::DuplicateRoute),
            "E0602" => Some(Self::InvalidMiddleware),
            "E0603" => Some(Self::InvalidCorsConfig),
            "E0604" => Some(Self::InvalidRateLimit),
            "E0605" => Some(Self::FfiFunctionNotFound),
            "E0606" => Some(Self::FfiTypeMismatch),

            "E0700" => Some(Self::DatabaseNotConnected),
            "E0701" => Some(Self::InvalidSqlQuery),
            "E0702" => Some(Self::ModelNotFound),
            "E0703" => Some(Self::MigrationFailed),
            "E0704" => Some(Self::InvalidDatabaseUrl),
            "E0705" => Some(Self::QueryBuilderUnknownModel),
            "E0706" => Some(Self::QueryBuilderUnknownField),
            "E0707" => Some(Self::QueryBuilderMissingWhere),
            "E0708" => Some(Self::QueryBuilderInvalidChain),

            "E0800" => Some(Self::InvalidEmail),
            "E0801" => Some(Self::InvalidUrl),
            "E0802" => Some(Self::BelowMinimum),
            "E0803" => Some(Self::AboveMaximum),
            "E0804" => Some(Self::PatternNotMatched),
            "E0805" => Some(Self::RequiredFieldMissing),

            "E0900" => Some(Self::InternalError),
            "E0901" => Some(Self::CodegenFailed),
            "E0902" => Some(Self::LlvmError),
            "E0903" => Some(Self::IoError),

            _ => None,
        }
    }

    /// Get the short title for this error (SCREAMING_CASE for compact output).
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
            Self::InvalidPattern => "INVALID PATTERN",
            Self::InvalidTypeExpr => "INVALID TYPE",
            Self::MissingFunctionBody => "MISSING BODY",
            Self::InvalidOperator => "INVALID OPERATOR",
            Self::UnexpectedEof => "UNEXPECTED EOF",
            Self::InvalidAssignTarget => "INVALID ASSIGN",
            Self::MissingClosingParen => "MISSING `)`",
            Self::MissingClosingBrace => "MISSING `}`",
            Self::MissingClosingBracket => "MISSING `]`",
            Self::ExpectedTypeAnnotation => "EXPECTED TYPE",
            Self::InvalidDecoratorSyntax => "INVALID DECORATOR",
            Self::ExpectedBlock => "EXPECTED BLOCK",
            Self::ExpectedExprAfterOp => "EXPECTED EXPRESSION",
            Self::InvalidMatchSyntax => "INVALID MATCH",
            Self::InvalidForSyntax => "INVALID FOR",
            Self::InvalidConstExpr => "INVALID CONST EXPR",

            Self::TypeMismatch => "TYPE MISMATCH",
            Self::UnknownType => "UNKNOWN TYPE",
            Self::CannotInferType => "CANNOT INFER TYPE",
            Self::TypeAnnotationRequired => "TYPE REQUIRED",
            Self::IncompatibleTypes => "INCOMPATIBLE TYPES",
            Self::CannotConvert => "CANNOT CONVERT",
            Self::InvalidReturnType => "INVALID RETURN TYPE",
            Self::TupleLengthMismatch => "TUPLE MISMATCH",
            Self::TypeParameterCount => "TYPE PARAM COUNT",
            Self::InvalidTypeForOperation => "INVALID TYPE FOR OP",
            Self::InvalidConditionType => "INVALID CONDITION",
            Self::InvalidCast => "INVALID CAST",
            Self::ReturnTypeMismatch => "RETURN MISMATCH",
            Self::ArgCountMismatch => "WRONG ARGS",
            Self::InvalidArrayElementType => "INVALID ARRAY ELEM",
            Self::InvalidMapKeyType => "INVALID MAP KEY",
            Self::IfElseBranchMismatch => "BRANCH MISMATCH",
            Self::NilWithNonOptional => "NIL NON-OPTIONAL",

            Self::ConcurrentMutableBorrow => "CONCURRENT MUTABLE BORROW",
            Self::AssignToImmutable => "IMMUTABLE ASSIGN",

            Self::UndefinedVariable => "UNDEFINED VARIABLE",
            Self::UndefinedFunction => "UNDEFINED FUNCTION",
            Self::UndefinedType => "UNDEFINED TYPE",
            Self::UndefinedField => "UNDEFINED FIELD",
            Self::UndefinedMethod => "UNDEFINED METHOD",
            Self::UndefinedVariant => "UNDEFINED VARIANT",
            Self::NameAlreadyDefined => "ALREADY DEFINED",
            Self::PrivateItemAccess => "PRIVATE ACCESS",
            Self::InvalidPath => "INVALID PATH",

            Self::DuplicateParameter => "DUPLICATE PARAM",
            Self::DuplicateField => "DUPLICATE FIELD",
            Self::DuplicateVariant => "DUPLICATE VARIANT",
            Self::DuplicateMethod => "DUPLICATE METHOD",
            Self::DuplicateConst => "DUPLICATE CONST",
            Self::InvalidSignature => "INVALID SIGNATURE",
            Self::MissingReturn => "MISSING RETURN",
            Self::UnreachableCode => "UNREACHABLE CODE",
            Self::InvalidDecorator => "INVALID DECORATOR",
            Self::ConflictingDecorators => "CONFLICTING DECORATORS",
            Self::InvalidDefaultValue => "INVALID DEFAULT",
            Self::MissingOkReturn => "MISSING OK",
            Self::ErrWithoutErrorType => "ERR NO ERROR TYPE",
            Self::TryInNonResultFunction => "TRY NO RESULT",
            Self::UnhandledResult => "UNHANDLED RESULT",
            Self::PanicWithoutMessage => "PANIC NO MSG",
            Self::MissingStructField => "MISSING FIELD",
            Self::UnknownStructField => "UNKNOWN FIELD",
            Self::NonExhaustiveMatch => "NON-EXHAUSTIVE",
            Self::UnreachablePattern => "UNREACHABLE PATTERN",
            Self::BreakOutsideLoop => "BREAK OUTSIDE LOOP",
            Self::ContinueOutsideLoop => "CONTINUE OUTSIDE LOOP",
            Self::ReturnOutsideFunction => "RETURN OUTSIDE FN",

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
            Self::QueryBuilderUnknownModel => "QB UNKNOWN MODEL",
            Self::QueryBuilderUnknownField => "QB UNKNOWN FIELD",
            Self::QueryBuilderMissingWhere => "QB MISSING WHERE",
            Self::QueryBuilderInvalidChain => "QB INVALID CHAIN",

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
            Self::UnreachableCode | Self::UnreachablePattern => ErrorSeverity::Warning,
            Self::InternalError | Self::CodegenFailed | Self::LlvmError => ErrorSeverity::Ice,
            _ => ErrorSeverity::Error,
        }
    }

    /// Get the error category.
    pub fn category(&self) -> ErrorCategory {
        let code = self.code();
        let num: u16 = code[1..].parse().unwrap_or(9999);
        match num {
            1..=99 => ErrorCategory::Syntax,
            100..=199 => ErrorCategory::Type,
            200..=299 => ErrorCategory::Ownership,
            300..=399 => ErrorCategory::Name,
            400..=499 => ErrorCategory::Declaration,
            500..=599 => ErrorCategory::Import,
            600..=699 => ErrorCategory::Http,
            700..=799 => ErrorCategory::Database,
            800..=899 => ErrorCategory::Validation,
            _ => ErrorCategory::Internal,
        }
    }

    /// Get detailed explanation (for `doo --explain E0XXX`).
    pub fn explanation(&self) -> &'static str {
        match self {
            Self::UnexpectedToken => {
                "\
Unexpected token encountered during parsing.\n\
Check your syntax — you may have a missing comma, colon, or operator.\n\n\
  fn add(a: Int b: Int) -> Int   // ❌ missing comma\n\
  fn add(a: Int, b: Int) -> Int  // ✅"
            }
            Self::UnclosedDelimiter => {
                "\
A delimiter was opened but never closed.\n\
Every `(` needs `)`, `{` needs `}`, `[` needs `]`.\n\n\
  let arr = [1, 2, 3    // ❌ missing ]\n\
  let arr = [1, 2, 3]   // ✅"
            }
            Self::UnterminatedString => {
                "\
A string literal was started with `\"` but never closed.\n\n\
  let s = \"hello     // ❌ missing closing quote\n\
  let s = \"hello\"   // ✅"
            }
            Self::ExpectedIdentifier => {
                "\
An identifier (name) was expected but something else was found.\n\n\
  let 123 = 5     // ❌ not an identifier\n\
  let count = 5   // ✅"
            }
            Self::TypeMismatch => {
                "\
The type of a value doesn't match what was expected.\n\n\
  let age: Int = \"twenty\"   // ❌ Str given, Int expected\n\
  let age: Int = 20          // ✅"
            }
            Self::InvalidCast => {
                "\
Cannot cast between these types.\n\
Allowed: Int↔Float↔Str, Bool→Int, Bool→Str.\n\
Not allowed: Bool↔Float, Str→Bool.\n\n\
  let x = true as Float   // ❌\n\
  let x = true as Int     // ✅"
            }
            Self::InvalidConditionType => {
                "\
Conditions in `if`/`for`/match guards must be Bool.\n\n\
  if \"hello\" { }     // ❌ Str is not Bool\n\
  if x > 0 { }        // ✅"
            }
            Self::ArgCountMismatch => {
                "\
Wrong number of arguments passed to a function.\n\n\
  fn add(a: Int, b: Int) -> Int { ... }\n\
  add(1)        // ❌ got 1, need 2\n\
  add(1, 2)     // ✅"
            }
            Self::ReturnTypeMismatch => {
                "\
Return value type doesn't match function signature.\n\n\
  fn getAge() -> Int { Ok \"twenty\" }  // ❌\n\
  fn getAge() -> Int { Ok 20 }         // ✅"
            }
            Self::ConcurrentMutableBorrow => {
                "\
This is the ONLY ownership error visible in Doo.\n\
You can have multiple reads OR one write, but not both.\n\
The compiler auto-handles all other memory management.\n\n\
  let mut arr = [1, 2, 3]\n\
  let ref1 = arr     // read\n\
  arr.push(4)        // ❌ write while ref1 exists\n\
  print(ref1)"
            }
            Self::AssignToImmutable => {
                "\
Cannot assign to a variable declared without `mut`.\n\n\
  let x = 5; x = 10         // ❌\n\
  let mut x = 5; x = 10     // ✅"
            }
            Self::UndefinedVariable => {
                "\
Variable not declared. Check for typos.\n\n\
  print(userName)              // ❌ undefined\n\
  let userName = \"Alice\"\n\
  print(userName)              // ✅"
            }
            Self::UndefinedFunction => {
                "\
Function not defined or imported. Check name and imports.\n\n\
  calculate(5)   // ❌ not defined\n\
  fn calculate(x: Int) -> Int { Ok x * 2 }\n\
  calculate(5)   // ✅"
            }
            Self::UndefinedField => {
                "\
Struct doesn't have this field.\n\n\
  struct User { name: Str, age: Int }\n\
  u.email   // ❌ no 'email' field\n\
  u.name    // ✅"
            }
            Self::NameAlreadyDefined => {
                "\
A symbol with this name already exists in this scope.\n\n\
  let x = 5\n\
  let x = 10   // ❌ already defined"
            }
            Self::MissingReturn => {
                "\
Not all code paths return a value.\n\n\
  fn get(flag: Bool) -> Int {\n\
      if flag { Ok 42 }                  // ❌ no else\n\
  }\n\
  fn get(flag: Bool) -> Int {\n\
      if flag { Ok 42 } else { Ok 0 }   // ✅\n\
  }"
            }
            Self::MissingOkReturn => {
                "\
Function returns Result but missing Ok on some paths.\n\n\
  fn div(a: Int, b: Int) -> Int ! Str {\n\
      if b == 0 { Err \"zero\" }\n\
      a / b       // ❌ missing Ok\n\
  }\n\
  // ✅ Ok a / b"
            }
            Self::ErrWithoutErrorType => {
                "\
`Err` used in function without error type (! E).\n\n\
  fn test() -> Int { Err \"fail\" }        // ❌\n\
  fn test() -> Int ! Str { Err \"fail\" }  // ✅"
            }
            Self::TryInNonResultFunction => {
                "\
`?` propagates errors, so function must declare error type.\n\
Exception: main() can use `?` (errors exit the program).\n\n\
  fn process() -> Int { divide(10, 0)? }       // ❌\n\
  fn process() -> Int ! Str { divide(10, 0)? } // ✅"
            }
            Self::UnhandledResult => {
                "\
Result value not handled. Use `?` or `let val ?? err = ...`.\n\n\
  divide(10, 0)                     // ❌ ignored\n\
  let result = divide(10, 0)?      // ✅ propagate\n\
  let val ?? err = divide(10, 0)   // ✅ manual"
            }
            Self::NonExhaustiveMatch => {
                "\
Match doesn't cover all cases. Add missing patterns or `_`.\n\n\
  match color {\n\
      Color::Red => \"red\"   // ❌ missing others\n\
  }\n\
  match color {\n\
      Color::Red => \"red\"\n\
      _ => \"other\"          // ✅ catch-all\n\
  }"
            }
            Self::BreakOutsideLoop => {
                "\
`break` can only be used inside a `for` loop."
            }
            Self::ContinueOutsideLoop => {
                "\
`continue` can only be used inside a `for` loop."
            }
            Self::ModuleNotFound => {
                "\
Module not found. Check import path and file location.\n\n\
  import utils::helpers   // Needs utils/helpers.doo"
            }
            Self::CircularImport => {
                "\
Files import each other, creating a cycle.\n\
Refactor shared code into a separate module."
            }
            _ => "Run `doo --explain EXXXX` with a specific error code for details.",
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.code(), self.title())
    }
}

/// Error category for grouping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCategory {
    Syntax,
    Type,
    Ownership,
    Name,
    Declaration,
    Import,
    Http,
    Database,
    Validation,
    Internal,
}

/// Severity level of an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorSeverity {
    /// Informational hint.
    Note,
    /// Warning (compilation continues).
    Warning,
    /// Error (compilation fails).
    Error,
    /// Internal compiler error (bug).
    Ice,
}

impl ErrorSeverity {
    /// Get the emoji for compact output.
    pub fn emoji(&self) -> &'static str {
        match self {
            Self::Note => "💡",
            Self::Warning => "⚠️",
            Self::Error => "❌",
            Self::Ice => "💥",
        }
    }
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
            severity: code.severity(),
            code,
            message: message.into(),
            span,
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

    /// Check if this is a warning.
    pub fn is_warning(&self) -> bool {
        self.severity == ErrorSeverity::Warning
    }
}

impl std::fmt::Display for CompilerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CompilerError {}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_codes() {
        assert_eq!(ErrorCode::UnexpectedToken.code(), "E0001");
        assert_eq!(ErrorCode::TypeMismatch.code(), "E0100");
        assert_eq!(ErrorCode::ConcurrentMutableBorrow.code(), "E0201");
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

    #[test]
    fn test_new_error_codes() {
        assert_eq!(ErrorCode::InvalidPattern.code(), "E0011");
        assert_eq!(ErrorCode::MissingOkReturn.code(), "E0409");
        assert_eq!(ErrorCode::TryInNonResultFunction.code(), "E0411");
        assert_eq!(ErrorCode::NonExhaustiveMatch.code(), "E0416");
        assert_eq!(ErrorCode::BreakOutsideLoop.code(), "E0418");
    }

    #[test]
    fn test_error_category() {
        assert_eq!(ErrorCode::UnexpectedToken.category(), ErrorCategory::Syntax);
        assert_eq!(ErrorCode::TypeMismatch.category(), ErrorCategory::Type);
        assert_eq!(
            ErrorCode::ConcurrentMutableBorrow.category(),
            ErrorCategory::Ownership
        );
        assert_eq!(ErrorCode::UndefinedVariable.category(), ErrorCategory::Name);
        assert_eq!(
            ErrorCode::MissingReturn.category(),
            ErrorCategory::Declaration
        );
    }

    #[test]
    fn test_severity_emoji() {
        assert_eq!(ErrorSeverity::Error.emoji(), "❌");
        assert_eq!(ErrorSeverity::Warning.emoji(), "⚠️");
        assert_eq!(ErrorSeverity::Note.emoji(), "💡");
        assert_eq!(ErrorSeverity::Ice.emoji(), "💥");
    }
}
