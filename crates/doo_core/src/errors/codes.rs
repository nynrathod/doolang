//! Error codes and compiler error types.
use crate::span::{FileId, FileSpan, Span};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorCode {
    UnexpectedToken,
    UnclosedDelimiter,
    InvalidNumberLiteral,
    InvalidStringLiteral,
    InvalidEscapeSequence,
    UnterminatedString,
    InvalidCharacter,
    MissingSemicolon,
    InvalidExpression,
    ExpectedIdentifier,
    InvalidPattern,
    InvalidTypeExpr,
    MissingFunctionBody,
    InvalidOperator,
    UnexpectedEof,
    InvalidAssignTarget,
    MissingClosingParen,
    MissingClosingBrace,
    MissingClosingBracket,
    ExpectedTypeAnnotation,
    InvalidDecoratorSyntax,
    ExpectedBlock,
    ExpectedExprAfterOp,
    InvalidMatchSyntax,
    InvalidForSyntax,
    InvalidConstExpr,
    TypeMismatch,
    UnknownType,
    CannotInferType,
    TypeAnnotationRequired,
    IncompatibleTypes,
    CannotConvert,
    InvalidReturnType,
    TupleLengthMismatch,
    TypeParameterCount,
    InvalidTypeForOperation,
    InvalidConditionType,
    InvalidCast,
    ReturnTypeMismatch,
    ArgCountMismatch,
    InvalidArrayElementType,
    InvalidMapKeyType,
    IfElseBranchMismatch,
    NilWithNonOptional,
    ConcurrentMutableBorrow,
    AssignToImmutable,
    UndefinedVariable,
    UndefinedFunction,
    UndefinedType,
    UndefinedField,
    UndefinedMethod,
    UndefinedVariant,
    NameAlreadyDefined,
    PrivateItemAccess,
    InvalidPath,
    DuplicateParameter,
    DuplicateField,
    DuplicateVariant,
    InvalidSignature,
    MissingReturn,
    UnreachableCode,
    InvalidDecorator,
    ConflictingDecorators,
    InvalidDefaultValue,
    MissingOkReturn,
    ErrWithoutErrorType,
    TryInNonResultFunction,
    UnhandledResult,
    PanicWithoutMessage,
    MissingStructField,
    UnknownStructField,
    NonExhaustiveMatch,
    UnreachablePattern,
    BreakOutsideLoop,
    ContinueOutsideLoop,
    ReturnOutsideFunction,
    DuplicateMethod,
    DuplicateConst,
    ModuleNotFound,
    ImportNotFound,
    CircularImport,
    InvalidImportPath,
    PrivateImport,
    InternalError,
    CodegenFailed,
    LlvmError,
    IoError,
}

impl ErrorCode {
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
            "E0900" => Some(Self::InternalError),
            "E0901" => Some(Self::CodegenFailed),
            "E0902" => Some(Self::LlvmError),
            "E0903" => Some(Self::IoError),
            _ => None,
        }
    }

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
            Self::InvalidAssignTarget => "INVALID ASSIGN TARGET",
            Self::MissingClosingParen => "MISSING )",
            Self::MissingClosingBrace => "MISSING }",
            Self::MissingClosingBracket => "MISSING ]",
            Self::ExpectedTypeAnnotation => "MISSING TYPE ANNOTATION",
            Self::InvalidDecoratorSyntax => "INVALID DECORATOR",
            Self::ExpectedBlock => "EXPECTED BLOCK",
            Self::ExpectedExprAfterOp => "EXPECTED EXPRESSION",
            Self::InvalidMatchSyntax => "INVALID MATCH",
            Self::InvalidForSyntax => "INVALID FOR",
            Self::InvalidConstExpr => "INVALID CONST EXPR",
            Self::TypeMismatch => "TYPE MISMATCH",
            Self::UnknownType => "UNKNOWN TYPE",
            Self::CannotInferType => "CANNOT INFER",
            Self::TypeAnnotationRequired => "TYPE ANNOTATION REQUIRED",
            Self::IncompatibleTypes => "INCOMPATIBLE TYPES",
            Self::CannotConvert => "CANNOT CONVERT",
            Self::InvalidReturnType => "INVALID RETURN",
            Self::TupleLengthMismatch => "TUPLE LENGTH MISMATCH",
            Self::TypeParameterCount => "TYPE PARAM COUNT",
            Self::InvalidTypeForOperation => "INVALID TYPE FOR OP",
            Self::InvalidConditionType => "INVALID CONDITION",
            Self::InvalidCast => "INVALID CAST",
            Self::ReturnTypeMismatch => "RETURN MISMATCH",
            Self::ArgCountMismatch => "ARG COUNT MISMATCH",
            Self::InvalidArrayElementType => "INVALID ARRAY ELEMENT",
            Self::InvalidMapKeyType => "INVALID MAP KEY",
            Self::IfElseBranchMismatch => "IF/ELSE MISMATCH",
            Self::NilWithNonOptional => "NIL WITH NON-OPTIONAL",
            Self::ConcurrentMutableBorrow => "CONCURRENT MUTABLE BORROW",
            Self::AssignToImmutable => "ASSIGN TO IMMUTABLE",
            Self::UndefinedVariable => "UNKNOWN NAME",
            Self::UndefinedFunction => "UNKNOWN FUNCTION",
            Self::UndefinedType => "UNKNOWN TYPE",
            Self::UndefinedField => "UNKNOWN FIELD",
            Self::UndefinedMethod => "UNKNOWN METHOD",
            Self::UndefinedVariant => "UNKNOWN VARIANT",
            Self::NameAlreadyDefined => "NAME ALREADY DEFINED",
            Self::PrivateItemAccess => "PRIVATE ITEM ACCESS",
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
            Self::MissingOkReturn => "MISSING OK RETURN",
            Self::ErrWithoutErrorType => "ERR WITHOUT ERROR TYPE",
            Self::TryInNonResultFunction => "TRY IN NON-RESULT FN",
            Self::UnhandledResult => "UNHANDLED RESULT",
            Self::PanicWithoutMessage => "PANIC WITHOUT MSG",
            Self::MissingStructField => "MISSING FIELD",
            Self::UnknownStructField => "UNKNOWN FIELD",
            Self::NonExhaustiveMatch => "NON-EXHAUSTIVE MATCH",
            Self::UnreachablePattern => "UNREACHABLE PATTERN",
            Self::BreakOutsideLoop => "BREAK OUTSIDE LOOP",
            Self::ContinueOutsideLoop => "CONTINUE OUTSIDE LOOP",
            Self::ReturnOutsideFunction => "RETURN OUTSIDE FN",
            Self::DuplicateMethod => "DUPLICATE METHOD",
            Self::DuplicateConst => "DUPLICATE CONST",
            Self::ModuleNotFound => "MODULE NOT FOUND",
            Self::ImportNotFound => "IMPORT NOT FOUND",
            Self::CircularImport => "CIRCULAR IMPORT",
            Self::InvalidImportPath => "INVALID IMPORT PATH",
            Self::PrivateImport => "PRIVATE IMPORT",
            Self::InternalError => "INTERNAL ERROR",
            Self::CodegenFailed => "CODEGEN FAILED",
            Self::LlvmError => "LLVM ERROR",
            Self::IoError => "IO ERROR",
        }
    }

    pub fn severity(&self) -> ErrorSeverity {
        match self {
            Self::UnreachableCode | Self::UnhandledResult => ErrorSeverity::Warning,
            Self::InternalError | Self::CodegenFailed | Self::LlvmError | Self::IoError => {
                ErrorSeverity::Ice
            }
            _ => ErrorSeverity::Error,
        }
    }

    pub fn category(&self) -> ErrorCategory {
        let code = self.code();
        let num: u16 = code[1..].parse().unwrap_or(9999);
        match num {
            0..=99 => ErrorCategory::Syntax,
            100..=199 => ErrorCategory::Type,
            200..=299 => ErrorCategory::Ownership,
            300..=399 => ErrorCategory::Name,
            400..=499 => ErrorCategory::Declaration,
            500..=599 => ErrorCategory::Import,
            _ => ErrorCategory::Internal,
        }
    }

    pub fn explanation(&self) -> &'static str {
        "See documentation for more details."
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.code(), self.title())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCategory {
    Syntax,
    Type,
    Ownership,
    Name,
    Declaration,
    Import,
    Internal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorSeverity {
    Note,
    Warning,
    Error,
    Ice,
}

impl ErrorSeverity {
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

#[derive(Clone, Debug)]
pub struct CompilerError {
    pub code: ErrorCode,
    pub message: String,
    pub file_id: FileId,
    pub span: Span,
    pub severity: ErrorSeverity,
    pub labels: Vec<(FileSpan, String)>,
    pub suggestion: Option<String>,
    pub notes: Vec<String>,
}

impl CompilerError {
    pub fn new(code: ErrorCode, message: impl Into<String>, span: Span) -> Self {
        Self {
            code,
            message: message.into(),
            file_id: FileId::DUMMY,
            span,
            severity: code.severity(),
            labels: Vec::new(),
            suggestion: None,
            notes: Vec::new(),
        }
    }

    pub fn with_file_id(mut self, file_id: FileId) -> Self {
        self.file_id = file_id;
        self
    }

    pub fn with_severity(mut self, severity: ErrorSeverity) -> Self {
        self.severity = severity;
        self
    }

    pub fn with_label(mut self, file_span: impl Into<FileSpan>, label: impl Into<String>) -> Self {
        self.labels.push((file_span.into(), label.into()));
        self
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn file_span(&self) -> FileSpan {
        FileSpan::new(self.file_id, self.span)
    }

    pub fn is_fatal(&self) -> bool {
        matches!(self.severity, ErrorSeverity::Error | ErrorSeverity::Ice)
    }

    pub fn is_warning(&self) -> bool {
        matches!(self.severity, ErrorSeverity::Warning)
    }
}

impl std::fmt::Display for CompilerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
