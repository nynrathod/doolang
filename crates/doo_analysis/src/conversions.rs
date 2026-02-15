//! Error Conversions
//!
//! Converts analysis-specific error types into `CompilerError` (doo_core).
//! This bridges the gap between granular analysis errors and the unified
//! diagnostic system — single source of truth.

use doo_core::errors::codes::{CompilerError, ErrorCode, ErrorSeverity};

use crate::borrow::{BorrowError, BorrowErrorKind};
use crate::ownership::OwnershipError;
use crate::semantic::decorators::DecoratorError;
use crate::semantic::error_flow::{ErrorFlowError, ErrorFlowErrorKind};
use crate::semantic::exhaustiveness::{ExhaustivenessError, ExhaustivenessErrorKind};
use crate::semantic::resolve::{CircularImportError, ResolveError};
use crate::semantic::scope::ScopeError;
use crate::semantic::type_check::{TypeError, TypeErrorKind};
use crate::semantic::visibility::VisibilityError;

// ============================================================================
// TypeError → CompilerError
// ============================================================================

impl From<TypeError> for CompilerError {
    fn from(e: TypeError) -> Self {
        match e.kind {
            TypeErrorKind::Mismatch { expected, found } => CompilerError::new(
                ErrorCode::TypeMismatch,
                format!("expected {}, found {}", expected, found),
                e.span,
            ),
            TypeErrorKind::Undefined(ref name) => CompilerError::new(
                ErrorCode::UndefinedVariable,
                format!("'{}' is not defined", name),
                e.span,
            )
            .with_suggestion(format!("check spelling of '{}'", name)),
            TypeErrorKind::UndefinedFunction(ref name) => CompilerError::new(
                ErrorCode::UndefinedFunction,
                format!("function '{}' is not defined", name),
                e.span,
            )
            .with_suggestion(format!("did you mean to import '{}'?", name)),
            TypeErrorKind::UndefinedType(ref name) => CompilerError::new(
                ErrorCode::UndefinedType,
                format!("type '{}' is not defined", name),
                e.span,
            )
            .with_suggestion(format!("check if '{}' is imported", name)),
            TypeErrorKind::UndefinedField {
                ref type_name,
                ref field,
            } => CompilerError::new(
                ErrorCode::UndefinedField,
                format!("no field '{}' on type '{}'", field, type_name),
                e.span,
            ),
            TypeErrorKind::UndefinedMethod {
                ref type_name,
                ref method,
            } => CompilerError::new(
                ErrorCode::UndefinedMethod,
                format!("no method '{}' on type '{}'", method, type_name),
                e.span,
            ),
            TypeErrorKind::UndefinedVariant {
                ref enum_name,
                ref variant,
            } => CompilerError::new(
                ErrorCode::UndefinedVariant,
                format!("no variant '{}' in enum '{}'", variant, enum_name),
                e.span,
            ),
            TypeErrorKind::InvalidOp(ref msg) => {
                CompilerError::new(ErrorCode::InvalidTypeForOperation, msg.clone(), e.span)
            }
            TypeErrorKind::ArgMismatch { expected, found } => CompilerError::new(
                ErrorCode::ArgCountMismatch,
                format!("expected {} argument(s), found {}", expected, found),
                e.span,
            ),
            TypeErrorKind::InvalidCondition { found } => CompilerError::new(
                ErrorCode::InvalidConditionType,
                format!("condition must be Bool, found {}", found),
                e.span,
            ),
            TypeErrorKind::InvalidCast { from, to } => CompilerError::new(
                ErrorCode::InvalidCast,
                format!("cannot cast {} to {}", from, to),
                e.span,
            ),
            TypeErrorKind::ReturnTypeMismatch {
                ref function,
                expected,
                found,
            } => CompilerError::new(
                ErrorCode::ReturnTypeMismatch,
                format!("expected {}, found {} in '{}'", expected, found, function),
                e.span,
            ),
            TypeErrorKind::UnknownType(ref name) => CompilerError::new(
                ErrorCode::UnknownType,
                format!("unknown type '{}'", name),
                e.span,
            ),
            TypeErrorKind::CannotInfer(ref ctx) => CompilerError::new(
                ErrorCode::CannotInferType,
                format!(
                    "cannot infer type{}",
                    if ctx.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", ctx)
                    }
                ),
                e.span,
            )
            .with_suggestion("add a type annotation"),
            TypeErrorKind::Incompatible {
                left,
                right,
                ref operation,
            } => CompilerError::new(
                ErrorCode::IncompatibleTypes,
                format!(
                    "incompatible types {} and {} for '{}'",
                    left, right, operation
                ),
                e.span,
            ),
            TypeErrorKind::CannotConvert { from, to } => CompilerError::new(
                ErrorCode::CannotConvert,
                format!("cannot convert {} to {}", from, to),
                e.span,
            ),
            TypeErrorKind::TupleLengthMismatch { expected, found } => CompilerError::new(
                ErrorCode::TupleLengthMismatch,
                format!("tuple expects {} element(s), found {}", expected, found),
                e.span,
            ),
            TypeErrorKind::TypeParamCount { expected, found } => CompilerError::new(
                ErrorCode::TypeParameterCount,
                format!("expected {} type parameter(s), found {}", expected, found),
                e.span,
            ),
            TypeErrorKind::InvalidArrayElement {
                expected,
                found,
                index,
            } => CompilerError::new(
                ErrorCode::InvalidArrayElementType,
                format!("expected {}, found {} at index {}", expected, found, index),
                e.span,
            ),
            TypeErrorKind::InvalidMapKey { found } => CompilerError::new(
                ErrorCode::InvalidMapKeyType,
                format!("map key must be hashable (Str, Int, Bool), found {}", found),
                e.span,
            ),
            TypeErrorKind::IfElseMismatch {
                then_type,
                else_type,
            } => CompilerError::new(
                ErrorCode::IfElseBranchMismatch,
                format!("expected {}, found {} in else branch", then_type, else_type),
                e.span,
            )
            .with_suggestion("ensure both branches return the same type"),
            TypeErrorKind::NilNonOptional { expected } => CompilerError::new(
                ErrorCode::NilWithNonOptional,
                format!("expected {}, found nil", expected),
                e.span,
            )
            .with_suggestion("make the type optional with '?'"),
            TypeErrorKind::MissingStructField {
                ref struct_name,
                ref field,
            } => CompilerError::new(
                ErrorCode::MissingStructField,
                format!("missing field '{}' in struct '{}'", field, struct_name),
                e.span,
            ),
            TypeErrorKind::UnknownStructField {
                ref struct_name,
                ref field,
            } => CompilerError::new(
                ErrorCode::UnknownStructField,
                format!("unknown field '{}' in struct '{}'", field, struct_name),
                e.span,
            ),
            TypeErrorKind::InvalidSignature(ref msg) => {
                CompilerError::new(ErrorCode::InvalidSignature, msg.clone(), e.span)
            }
        }
    }
}

// ============================================================================
// BorrowError → CompilerError
// ============================================================================

impl From<BorrowError> for CompilerError {
    fn from(e: BorrowError) -> Self {
        match &e.kind {
            BorrowErrorKind::ConcurrentMutableBorrow {
                variable,
                existing_borrow_span,
            } => CompilerError::new(
                ErrorCode::ConcurrentMutableBorrow,
                format!("cannot mutably borrow '{}' — already borrowed", variable),
                e.span,
            )
            .with_label(*existing_borrow_span, "first borrow here".to_string())
            .with_note("you can have multiple reads OR one write, but not both at the same time"),
            BorrowErrorKind::BorrowWhileMutablyBorrowed {
                variable,
                mutable_borrow_span,
            } => {
                // Maps to ConcurrentMutableBorrow since Doo auto-handles most borrow issues
                CompilerError::new(
                    ErrorCode::ConcurrentMutableBorrow,
                    format!("cannot borrow '{}' — already mutably borrowed", variable),
                    e.span,
                )
                .with_label(*mutable_borrow_span, "mutable borrow here".to_string())
            }
            BorrowErrorKind::ModifyWhileBorrowed {
                variable,
                borrow_span,
            } => CompilerError::new(
                ErrorCode::ConcurrentMutableBorrow,
                format!("cannot modify '{}' — currently borrowed", variable),
                e.span,
            )
            .with_label(*borrow_span, "borrow here".to_string()),
        }
    }
}

// ============================================================================
// OwnershipError → CompilerError
// ============================================================================

impl From<OwnershipError> for CompilerError {
    fn from(e: OwnershipError) -> Self {
        // OwnershipError has a generic message; determine the code from message content
        let code = if e.message.contains("immutable") || e.message.contains("not mutable") {
            ErrorCode::AssignToImmutable
        } else {
            // Fallback to generic — ownership errors are mostly auto-handled by the compiler
            ErrorCode::ConcurrentMutableBorrow
        };
        CompilerError::new(code, e.message, e.span)
    }
}

// ============================================================================
// ErrorFlowError → CompilerError
// ============================================================================

impl From<ErrorFlowError> for CompilerError {
    fn from(e: ErrorFlowError) -> Self {
        match &e.kind {
            ErrorFlowErrorKind::UnhandledResult { .. } => CompilerError::new(
                ErrorCode::UnhandledResult,
                "Result value not handled",
                e.span,
            )
            .with_suggestion("use `?` to propagate or `let val ?? err = ...` to handle"),
            ErrorFlowErrorKind::TryInNonResultFunction { func_name } => CompilerError::new(
                ErrorCode::TryInNonResultFunction,
                format!("`?` used in '{}' which doesn't return Result", func_name),
                e.span,
            )
            .with_suggestion(format!("add error type: fn {}(...) -> T ! E", func_name)),
            ErrorFlowErrorKind::ErrInNonResultFunction { func_name } => CompilerError::new(
                ErrorCode::ErrWithoutErrorType,
                format!("`Err` used in '{}' without error type", func_name),
                e.span,
            )
            .with_suggestion(format!("declare: fn {}(...) -> T ! ErrType", func_name)),
            ErrorFlowErrorKind::OkInNonResultFunction { func_name } => CompilerError::new(
                ErrorCode::ErrWithoutErrorType,
                format!("`Ok` used in '{}' without error type", func_name),
                e.span,
            )
            .with_suggestion(format!("declare: fn {}(...) -> T ! ErrType", func_name)),
            ErrorFlowErrorKind::MissingOkPath { func_name } => CompilerError::new(
                ErrorCode::MissingOkReturn,
                format!("'{}' missing Ok return on some paths", func_name),
                e.span,
            )
            .with_suggestion("ensure all code paths return Ok value"),
            ErrorFlowErrorKind::PanicWithoutMessage => CompilerError::new(
                ErrorCode::PanicWithoutMessage,
                "`??` (panic) used without message",
                e.span,
            )
            .with_suggestion("add message: result ?? \"operation failed\""),
        }
    }
}

// ============================================================================
// ExhaustivenessError → CompilerError
// ============================================================================

impl From<ExhaustivenessError> for CompilerError {
    fn from(e: ExhaustivenessError) -> Self {
        match &e.kind {
            ExhaustivenessErrorKind::NonExhaustive { missing } => {
                let missing_str = missing.join(", ");
                CompilerError::new(
                    ErrorCode::NonExhaustiveMatch,
                    format!("non-exhaustive match: missing {}", missing_str),
                    e.span,
                )
                .with_suggestion("add missing patterns or use `_` as catch-all")
            }
            ExhaustivenessErrorKind::UnreachablePattern => CompilerError::new(
                ErrorCode::UnreachablePattern,
                "this pattern is unreachable",
                e.span,
            )
            .with_note("a previous pattern already covers this case"),
        }
    }
}

// ============================================================================
// ScopeError → CompilerError
// ============================================================================

impl From<ScopeError> for CompilerError {
    fn from(e: ScopeError) -> Self {
        match &e {
            ScopeError::Redeclaration {
                name,
                original,
                redeclared,
            } => CompilerError::new(
                ErrorCode::NameAlreadyDefined,
                format!("'{}' is already defined in this scope", name),
                *redeclared,
            )
            .with_label(*original, "first defined here".to_string())
            .with_suggestion(format!("rename one of the '{}' declarations", name)),
            ScopeError::Undeclared { name, span } => CompilerError::new(
                ErrorCode::UndefinedVariable,
                format!("'{}' is not defined", name),
                *span,
            )
            .with_suggestion(format!("check spelling of '{}'", name)),
        }
    }
}

// ============================================================================
// VisibilityError → CompilerError
// ============================================================================

impl From<VisibilityError> for CompilerError {
    fn from(e: VisibilityError) -> Self {
        CompilerError::new(
            ErrorCode::PrivateItemAccess,
            format!(
                "cannot access private symbol '{}' from module '{}'",
                e.symbol, e.accessed_from
            ),
            e.span,
        )
        .with_note(format!(
            "'{}' is defined in module '{}'",
            e.symbol, e.defined_in
        ))
        .with_suggestion(format!(
            "make '{}' public or access it from '{}'",
            e.symbol, e.defined_in
        ))
    }
}

// ============================================================================
// DecoratorError → CompilerError
// Delegates to decorators::to_compiler_error (single source of truth).
// ============================================================================

impl From<DecoratorError> for CompilerError {
    fn from(e: DecoratorError) -> Self {
        crate::semantic::decorators::to_compiler_error(&e, doo_core::Span::dummy())
    }
}

// ============================================================================
// CircularImportError → CompilerError
// ============================================================================

impl From<CircularImportError> for CompilerError {
    fn from(e: CircularImportError) -> Self {
        CompilerError::new(
            ErrorCode::CircularImport,
            format!("circular import detected: {}", e.format_cycle()),
            e.span,
        )
        .with_suggestion("refactor shared code into a separate module to break the cycle")
    }
}

// ============================================================================
// ResolveError → CompilerError
// ============================================================================

impl From<ResolveError> for CompilerError {
    fn from(e: ResolveError) -> Self {
        // Simple string-based resolution error
        CompilerError::new(ErrorCode::UndefinedVariable, e.message, e.span)
    }
}

// ============================================================================
// Batch conversion helpers
// ============================================================================

/// Convert a Vec of analysis errors into CompilerErrors.
pub fn type_errors_to_compiler(errors: Vec<TypeError>) -> Vec<CompilerError> {
    errors.into_iter().map(CompilerError::from).collect()
}

pub fn borrow_errors_to_compiler(errors: Vec<BorrowError>) -> Vec<CompilerError> {
    errors.into_iter().map(CompilerError::from).collect()
}

pub fn ownership_errors_to_compiler(errors: Vec<OwnershipError>) -> Vec<CompilerError> {
    errors.into_iter().map(CompilerError::from).collect()
}

pub fn error_flow_errors_to_compiler(errors: Vec<ErrorFlowError>) -> Vec<CompilerError> {
    errors.into_iter().map(CompilerError::from).collect()
}

pub fn exhaustiveness_errors_to_compiler(errors: Vec<ExhaustivenessError>) -> Vec<CompilerError> {
    errors.into_iter().map(CompilerError::from).collect()
}

pub fn scope_errors_to_compiler(errors: Vec<ScopeError>) -> Vec<CompilerError> {
    errors.into_iter().map(CompilerError::from).collect()
}

pub fn visibility_errors_to_compiler(errors: Vec<VisibilityError>) -> Vec<CompilerError> {
    errors.into_iter().map(CompilerError::from).collect()
}

pub fn decorator_errors_to_compiler(errors: Vec<DecoratorError>) -> Vec<CompilerError> {
    errors.into_iter().map(CompilerError::from).collect()
}

pub fn circular_import_error_to_compiler(error: CircularImportError) -> CompilerError {
    CompilerError::from(error)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use doo_core::Span;

    #[test]
    fn test_type_error_conversion() {
        let err = TypeError {
            kind: TypeErrorKind::Undefined("foo".into()),
            span: Span::new(0, 10, 13),
        };
        let ce: CompilerError = err.into();
        assert_eq!(ce.code, ErrorCode::UndefinedVariable);
        assert!(ce.suggestion.is_some());
    }

    #[test]
    fn test_borrow_error_conversion() {
        let err = BorrowError::concurrent_mut("x".into(), Span::new(0, 5, 6), Span::new(0, 20, 21));
        let ce: CompilerError = err.into();
        assert_eq!(ce.code, ErrorCode::ConcurrentMutableBorrow);
        assert_eq!(ce.labels.len(), 1);
    }

    #[test]
    fn test_error_flow_conversion() {
        let err = ErrorFlowError::new(ErrorFlowErrorKind::PanicWithoutMessage, Span::new(0, 0, 2));
        let ce: CompilerError = err.into();
        assert_eq!(ce.code, ErrorCode::PanicWithoutMessage);
    }

    #[test]
    fn test_exhaustiveness_conversion() {
        let err = ExhaustivenessError {
            kind: ExhaustivenessErrorKind::NonExhaustive {
                missing: vec!["Color::Blue".into(), "Color::Green".into()],
            },
            span: Span::new(0, 0, 10),
        };
        let ce: CompilerError = err.into();
        assert_eq!(ce.code, ErrorCode::NonExhaustiveMatch);
        assert!(ce.message.contains("Blue"));
    }

    #[test]
    fn test_scope_error_redeclaration() {
        let err = ScopeError::Redeclaration {
            name: "x".into(),
            original: Span::new(0, 0, 1),
            redeclared: Span::new(0, 10, 11),
        };
        let ce: CompilerError = err.into();
        assert_eq!(ce.code, ErrorCode::NameAlreadyDefined);
        assert!(ce.message.contains("already defined"));
        assert_eq!(ce.labels.len(), 1);
    }

    #[test]
    fn test_visibility_error_conversion() {
        let err = VisibilityError {
            symbol: "helper".into(),
            defined_in: "utils".into(),
            accessed_from: "main".into(),
            span: Span::new(0, 5, 11),
        };
        let ce: CompilerError = err.into();
        assert_eq!(ce.code, ErrorCode::PrivateItemAccess);
        assert!(ce.message.contains("private symbol"));
    }

    #[test]
    fn test_circular_import_conversion() {
        let err = CircularImportError {
            cycle: vec!["a".into(), "b".into(), "c".into(), "a".into()],
            span: Span::new(0, 0, 10),
        };
        let ce: CompilerError = err.into();
        assert_eq!(ce.code, ErrorCode::CircularImport);
        assert!(ce.message.contains("a -> b -> c -> a"));
    }
}
