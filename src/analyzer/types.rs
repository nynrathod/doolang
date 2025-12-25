#![allow(dead_code)]

use crate::parser::ast::{AstNode, Pattern, TypeNode};
use std::fmt;

#[derive(Debug)]
pub struct TypeMismatch {
    pub expected: TypeNode,
    pub found: TypeNode,
    pub value: Option<Box<AstNode>>,
    pub line: Option<usize>,
    pub col: Option<usize>,
}

#[derive(Debug)]
pub struct NamedError {
    pub name: String,
}

#[derive(Debug)]
pub enum SemanticError {
    // Variable Declaration/Assignment Errors
    VariableRedeclaration(NamedError),
    UndeclaredVariable(NamedError),
    VarTypeMismatch(TypeMismatch),
    TupleAssignmentMismatch {
        expected: usize,
        found: usize,
    },
    InvalidAssignmentTarget {
        target: String,
    },
    OutOfScopeVariable(NamedError),
    InvalidMapKeyType {
        found: TypeNode,
        expected: TypeNode,
    },

    // Function Declaration/Call Errors
    FunctionRedeclaration(NamedError),
    FunctionParamRedeclaration(NamedError),
    MissingParamType(NamedError),
    UndeclaredFunction(NamedError),
    InvalidFunctionCall {
        func: String,
    },
    MethodNotFoundOnType {
        object_type: String,
        method_name: String,
        correct_type: Option<String>,
    },
    InvalidMethodCall {
        method: String,
        type_name: String,
        message: String,
    },
    FunctionArgumentMismatch {
        name: String,
        expected: usize,
        found: usize,
    },
    FunctionArgumentTypeMismatch {
        name: String,
        expected: TypeNode,
        found: TypeNode,
    },
    MissingFunctionReturn {
        function: String,
    },
    InvalidReturnInVoidFunction {
        function: String,
    },
    ReturnTypeMismatch {
        function: String,
        mismatch: TypeMismatch,
    },
    InvalidPublicName(NamedError),

    // Type/Operator Errors
    OperatorTypeMismatch(TypeMismatch),
    EmptyCollectionTypeInferenceError(TypeMismatch),
    ImmutableEmptyCollection {
        found: TypeNode,
    },
    InvalidConditionType(TypeMismatch),

    // Print
    InvalidPrintType {
        found: TypeNode,
    },
    UnexpectedNode {
        expected: String,
    },

    // For
    InvalidForIterableType {
        found: TypeNode,
    },
    ArrayIterationWithTuple {
        tuple_len: usize,
    },
    MapIterationRequiresTuple,
    NonIterableType {
        found: TypeNode,
    },
    InfiniteLoopWithPattern {
        pattern: Pattern,
    },
    RangeIterationTypeMismatch {
        expected: TypeNode,
        found: TypeNode,
    },

    // Struct
    StructRedeclaration(NamedError),
    DuplicateField {
        struct_name: String,
        field: String,
    },

    // Enum
    EnumRedeclaration(NamedError),

    DuplicateEnumVariant {
        enum_name: String,
        variant: String,
    },

    // --- Module Import Errors ---
    ModuleNotFound(String),
    /// Error when trying to import a private function (camelCase name)
    PrivateFunctionImport {
        name: String,
        module: String,
    },
    /// Error when trying to import a private struct (camelCase name)
    PrivateStructImport {
        name: String,
        module: String,
    },
    /// Error when trying to import a private enum (camelCase name)
    PrivateEnumImport {
        name: String,
        module: String,
    },
    /// Error when trying to access a private field (camelCase name) from outside the module
    PrivateFieldAccess {
        struct_name: String,
        field_name: String,
    },
    /// Dedicated error for circular imports, includes the cycle of modules
    CircularImport {
        cycle: Vec<String>,
    },
    ParseError,
    ParseErrorMsg(String),

    ParseErrorInModule {
        file: String,
        error: String,
    },

    // Error handling
    UnhandledResult {
        ok_type: TypeNode,
        error_type: TypeNode,
    },
    MissingOkInFunctionWithReturnType {
        function: String,
    },
    MissingErrInFunctionWithErrorType {
        function: String,
    },
    UnexpectedReturnWithReturnType {
        function: String,
    },

    // Decorator validation errors
    InvalidDecoratorType {
        decorator: String,
        field: String,
        struct_name: String,
        expected_type: String,
        found_type: String,
    },
    InvalidDecoratorArgs {
        decorator: String,
        field: String,
        message: String,
    },
    UnknownDecorator {
        decorator: String,
        field: String,
        struct_name: String,
    },
}

impl fmt::Display for TypeNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeNode::Float => write!(f, "Float"),
            TypeNode::Int => write!(f, "Int"),
            TypeNode::String => write!(f, "Str"),
            TypeNode::Bool => write!(f, "Bool"),
            TypeNode::Nil => write!(f, "Nil"),
            TypeNode::Array(t) => write!(f, "Array<{}>", t),
            TypeNode::Map(k, v) => write!(f, "Map<{}, {}>", k, v),
            TypeNode::Tuple(ts) => {
                let parts: Vec<String> = ts.iter().map(|t| t.to_string()).collect();
                write!(f, "({})", parts.join(", "))
            }
            TypeNode::Void => write!(f, "Void"),
            TypeNode::Optional(t) => write!(f, "{}?", t),
            TypeNode::Struct(name, _) => write!(f, "Struct {}", name),
            TypeNode::Enum(name, _) => write!(f, "Enum {}", name),
            TypeNode::Range(a, b, inclusive) => write!(
                f,
                "Range<{}, {}{}>",
                a,
                b,
                if *inclusive { ", inclusive" } else { "" }
            ),
            TypeNode::TypeRef(s) => write!(f, "{}", s),
            TypeNode::Function(params, ret) => {
                let param_strs: Vec<String> = params.iter().map(|t| t.to_string()).collect();
                write!(f, "Fn({}) -> {}", param_strs.join(", "), ret)
            }
            TypeNode::Result(ok_type, err_type) => {
                write!(f, "Result<{}, {}>", ok_type, err_type)
            }
            TypeNode::Builtin(name) => write!(f, "Builtin({})", name),
            TypeNode::Any => write!(f, "Any"),
            TypeNode::Error => write!(f, "Error"),
        }
    }
}

impl fmt::Display for TypeMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let expected = self.expected.to_string().to_lowercase();
        let found = self.found.to_string().to_lowercase();
        write!(f, "expected {}, found {}", expected, found)
    }
}

impl fmt::Display for NamedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl SemanticError {
    pub fn code(&self) -> &'static str {
        match self {
            // Variable Declaration/Assignment Errors
            SemanticError::VariableRedeclaration(_) => "E0001",
            SemanticError::UndeclaredVariable(_) => "E0002",
            SemanticError::VarTypeMismatch(_) => "E0003",
            SemanticError::TupleAssignmentMismatch { .. } => "E0004",
            SemanticError::InvalidAssignmentTarget { .. } => "E0005",
            SemanticError::OutOfScopeVariable(_) => "E0006",
            SemanticError::InvalidMapKeyType { .. } => "E0007",

            // Function Declaration/Call Errors
            SemanticError::FunctionRedeclaration(_) => "E0101",
            SemanticError::FunctionParamRedeclaration(_) => "E0102",
            SemanticError::MissingParamType(_) => "E0103",
            SemanticError::UndeclaredFunction(_) => "E0104",
            SemanticError::InvalidFunctionCall { .. } => "E0105",
            SemanticError::FunctionArgumentMismatch { .. } => "E0106",
            SemanticError::MethodNotFoundOnType { .. } => "E0112",
            SemanticError::InvalidMethodCall { .. } => "E0113",
            SemanticError::FunctionArgumentTypeMismatch { .. } => "E0107",
            SemanticError::MissingFunctionReturn { .. } => "E0108",
            SemanticError::InvalidReturnInVoidFunction { .. } => "E0109",
            SemanticError::ReturnTypeMismatch { .. } => "E0110",
            SemanticError::InvalidPublicName(_) => "E0111",

            // Type/Operator Errors
            SemanticError::OperatorTypeMismatch(_) => "E0201",
            SemanticError::EmptyCollectionTypeInferenceError(_) => "E0202",
            SemanticError::ImmutableEmptyCollection { .. } => "E0204",
            SemanticError::InvalidConditionType(_) => "E0203",

            // Print
            SemanticError::InvalidPrintType { .. } => "E0301",
            SemanticError::UnexpectedNode { .. } => "E0302",

            // For
            SemanticError::InvalidForIterableType { .. } => "E0401",
            SemanticError::ArrayIterationWithTuple { .. } => "E0402",
            SemanticError::MapIterationRequiresTuple => "E0403",
            SemanticError::NonIterableType { .. } => "E0404",
            SemanticError::InfiniteLoopWithPattern { .. } => "E0405",
            SemanticError::RangeIterationTypeMismatch { .. } => "E0406",

            // Struct
            SemanticError::StructRedeclaration(_) => "E0501",
            SemanticError::DuplicateField { .. } => "E0502",

            // Enum
            SemanticError::EnumRedeclaration(_) => "E0601",
            SemanticError::DuplicateEnumVariant { .. } => "E0602",

            // Module Import / Parse
            SemanticError::ModuleNotFound(_) => "E0701",
            SemanticError::ParseError => "E0702",
            SemanticError::ParseErrorMsg(_) => "E0703",

            SemanticError::ParseErrorInModule { .. } => "E0704",
            SemanticError::CircularImport { .. } => "E0705",
            SemanticError::PrivateFunctionImport { .. } => "E0706",
            SemanticError::PrivateStructImport { .. } => "E0707",
            SemanticError::PrivateEnumImport { .. } => "E0708",
            SemanticError::PrivateFieldAccess { .. } => "E0709",

            // Error handling
            SemanticError::UnhandledResult { .. } => "E0801",
            SemanticError::MissingOkInFunctionWithReturnType { .. } => "E0802",
            SemanticError::MissingErrInFunctionWithErrorType { .. } => "E0803",
            SemanticError::UnexpectedReturnWithReturnType { .. } => "E0804",

            // Decorator validation
            SemanticError::InvalidDecoratorType { .. } => "E0901",
            SemanticError::InvalidDecoratorArgs { .. } => "E0902",
            SemanticError::UnknownDecorator { .. } => "E0903",
        }
    }
}

impl fmt::Display for SemanticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use SemanticError as E;
        match self {
            E::CircularImport { cycle } => {
                write!(
                    f,
                    "error[{}]: circular import detected: {}",
                    self.code(),
                    cycle.join(" -> ")
                )
            }
            // Variable Declaration/Assignment Errors
            E::VariableRedeclaration(n) => write!(
                f,
                "error[{}]: duplicate variable '{}'",
                self.code(),
                n
            ),
            E::UndeclaredVariable(n) => write!(
                f,
                "error[{}]: use of undefined variable '{}'",
                self.code(),
                n
            ),
            E::VarTypeMismatch(m) => write!(f, "error[{}]: type mismatch: {}", self.code(), m),
            E::TupleAssignmentMismatch { expected, found } => write!(
                f,
                "error[{}]: tuple assignment mismatch: expected {} elements, found {}",
                self.code(),
                expected,
                found
            ),
            E::InvalidAssignmentTarget { target } => write!(
                f,
                "error[{}]: invalid assignment target: {}",
                self.code(),
                target
            ),
            E::OutOfScopeVariable(n) => write!(
                f,
                "error[{}]: variable '{}' is out of scope here",
                self.code(),
                n
            ),
            E::InvalidMapKeyType { found, expected } => write!(
                f,
                "error[{}]: invalid map key type: expected {}, found {}",
                self.code(),
                expected,
                found
            ),
            E::ImmutableEmptyCollection { found } => write!(
                f,
                "error[{}]: immutable variables cannot be initialized with empty collections; only mutable variables (with 'mut' keyword) are allowed to be empty. Found: {}",
                self.code(),
                found
            ),

            // Function Declaration/Call Errors
            E::FunctionRedeclaration(n) => {
                write!(f, "error[{}]: duplicate function '{}'", self.code(), n)
            }
            E::FunctionParamRedeclaration(n) => write!(
                f,
                "error[{}]: duplicate parameter '{}'",
                self.code(),
                n
            ),
            E::MissingParamType(n) => write!(
                f,
                "error[{}]: missing type annotation for parameter '{}'",
                self.code(),
                n
            ),
            E::UndeclaredFunction(n) => write!(
                f,
                "error[{}]: undefined function '{}'",
                self.code(),
                n
            ),
            E::InvalidFunctionCall { func } => write!(
                f,
                "error[{}]: invalid function call target: {}",
                self.code(),
                func
            ),
            E::MethodNotFoundOnType { object_type, method_name, correct_type } => {
                if let Some(ct) = correct_type {
                    write!(
                        f,
                        "error[{}]: {} type does not have method '{}' (this is a {} method)",
                        self.code(),
                        object_type,
                        method_name,
                        ct
                    )
                } else {
                    write!(
                        f,
                        "error[{}]: {} type does not have method '{}'",
                        self.code(),
                        object_type,
                        method_name
                    )
                }
            }
            E::InvalidMethodCall { method, type_name, message } => {
                write!(
                    f,
                    "error[{}]: {}.{}() is not allowed: {}",
                    self.code(),
                    type_name,
                    method,
                    message
                )
            }
            E::FunctionArgumentMismatch {
                name,
                expected,
                found,
            } => write!(
                f,
                "error[{}]: function '{}' expects {} parameters as arguments, found {}",
                self.code(),
                name,
                expected,
                found
            ),
            E::FunctionArgumentTypeMismatch {
                name,
                expected,
                found,
            } => write!(
                f,
                "error[{}]: function '{}' argument type mismatch: expected {}, found {}",
                self.code(),
                name,
                expected,
                found
            ),
            E::MissingFunctionReturn { function } => write!(
                f,
                "error[{}]: function '{}' must return a value",
                self.code(),
                function
            ),
            E::InvalidReturnInVoidFunction { function } => write!(
                f,
                "error[{}]: function '{}' cannot return a value (declared Void)",
                self.code(),
                function
            ),
            E::ReturnTypeMismatch { function, mismatch } => write!(
                f,
                "error[{}]: return type mismatch in '{}': {}",
                self.code(),
                function,
                mismatch
            ),
            E::InvalidPublicName(n) => write!(
                f,
                "error[{}]: public function '{}' must start with an uppercase letter",
                self.code(),
                n
            ),

            // Type/Operator Errors
            E::OperatorTypeMismatch(m) => {
                write!(f, "error[{}]: type mismatch: {}", self.code(), m)
            }
            E::EmptyCollectionTypeInferenceError(m) => write!(
                f,
                "error[{}]: cannot infer type of empty collection: {}",
                self.code(),
                m
            ),
            E::InvalidConditionType(m) => {
                write!(f, "error[{}]: condition must be bool, {}", self.code(), m)
            }

            // Print
            E::InvalidPrintType { found } => write!(
                f,
                "error[{}]: cannot print value of type {}",
                self.code(),
                found
            ),
            E::UnexpectedNode { expected } => write!(
                f,
                "error[{}]: unexpected construct: expected {}",
                self.code(),
                expected
            ),

            // For
            E::InvalidForIterableType { found } => write!(
                f,
                "error[{}]: invalid iterable type in for-loop: {}",
                self.code(),
                found
            ),
            E::ArrayIterationWithTuple { tuple_len } => write!(
                f,
                "error[{}]: array iteration does not support tuple pattern of length {}",
                self.code(),
                tuple_len
            ),
            E::MapIterationRequiresTuple => write!(
                f,
                "error[{}]: map iteration requires a tuple pattern (key, value)",
                self.code()
            ),
            E::NonIterableType { found } => write!(
                f,
                "error[{}]: non-iterable type in for-loop: {}",
                self.code(),
                found
            ),
            E::InfiniteLoopWithPattern { pattern } => write!(
                f,
                "error[{}]: infinite loop with pattern is not allowed: {:?}",
                self.code(),
                pattern
            ),
            E::RangeIterationTypeMismatch { expected, found } => write!(
                f,
                "error[{}]: range iteration type mismatch: expected {}, found {}",
                self.code(),
                expected,
                found
            ),

            // Struct
            E::StructRedeclaration(n) => {
                write!(f, "error[{}]: struct '{}' redeclared", self.code(), n)
            }
            E::DuplicateField { struct_name, field } => write!(
                f,
                "error[{}]: struct '{}' has duplicate field '{}'",
                self.code(),
                struct_name,
                field
            ),

            // Enum
            E::EnumRedeclaration(n) => write!(f, "error[{}]: enum '{}' redeclared", self.code(), n),
            E::DuplicateEnumVariant { enum_name, variant } => write!(
                f,
                "error[{}]: enum '{}' has duplicate variant '{}'",
                self.code(),
                enum_name,
                variant
            ),

            // Module Import / Parse
            E::ModuleNotFound(p) => write!(f, "error[{}]: module not found: {}", self.code(), p),
            E::ParseError => write!(f, "error[{}]: parse error in imported module", self.code()),
            E::ParseErrorMsg(msg) => write!(f, "error[{}]: {}", self.code(), msg),
            E::ParseErrorInModule { file, error } => {
                write!(f, "error[{}] in {}: {}", self.code(), file, error)
            }
            E::PrivateFunctionImport { name, module } => write!(
                f,
                "error[{}]: cannot import private function '{}' from module '{}'. Private functions (camelCase) are not accessible outside their module. Use PascalCase for public functions.",
                self.code(),
                name,
                module
            ),
            E::PrivateStructImport { name, module } => write!(
                f,
                "error[{}]: cannot import private struct '{}' from module '{}'. Private structs (camelCase) are not accessible outside their module. Use PascalCase for public structs.",
                self.code(),
                name,
                module
            ),
            E::PrivateEnumImport { name, module } => write!(
                f,
                "error[{}]: cannot import private enum '{}' from module '{}'. Private enums (camelCase) are not accessible outside their module. Use PascalCase for public enums.",
                self.code(),
                name,
                module
            ),
            E::PrivateFieldAccess { struct_name, field_name } => write!(
                f,
                "error[{}]: cannot access private field '{}' on struct '{}'. Private fields (camelCase) are not accessible outside their module. Use PascalCase for public fields.",
                self.code(),
                field_name,
                struct_name
            ),

            // Error handling
            E::UnhandledResult { ok_type, error_type } => write!(
                f,
                "error[{}]: unhandled Result type: function returns Result({}, {}). Error handling is mandatory - use '?' operator to propagate, or manually extract with 'let ok, err = ...'",
                self.code(),
                ok_type,
                error_type
            ),
            E::MissingOkInFunctionWithReturnType { function } => write!(
                f,
                "error[{}]: function '{}' declares a return type but uses bare 'Return' statements. Functions with return types MUST use 'Ok' expression. Use 'Ok value;' instead of 'return value;'",
                self.code(),
                function
            ),
            E::MissingErrInFunctionWithErrorType { function } => write!(
                f,
                "error[{}]: function '{}' declares an error type (! ErrorType) but has no error handling path (no Err expression). Either add an 'Err value;' branch or remove the error type declaration",
                self.code(),
                function
            ),
            E::UnexpectedReturnWithReturnType { function } => write!(
                f,
                "error[{}]: function '{}' declares a return type but uses bare 'Return' statement. Functions with return types MUST use 'Ok' expression instead. Use 'Ok value;' instead of 'return value;'",
                self.code(),
                function
            ),

            // Decorator validation errors
            E::InvalidDecoratorType { decorator, field, struct_name, expected_type, found_type } => write!(
                f,
                "error[{}]: @{} decorator on field '{}' in struct '{}' requires type {}, found {}",
                self.code(),
                decorator,
                field,
                struct_name,
                expected_type,
                found_type
            ),
            E::InvalidDecoratorArgs { decorator, field, message } => write!(
                f,
                "error[{}]: @{} decorator on field '{}': {}",
                self.code(),
                decorator,
                field,
                message
            ),
            E::UnknownDecorator { decorator, field, struct_name } => write!(
                f,
                "error[{}]: unknown decorator @{} on field '{}' in struct '{}'. Valid decorators: @email, @required, @min, @max, @foreign, @unique, @primary, @autoIncrement, @auto, @hash, @default",
                self.code(),
                decorator,
                field,
                struct_name
            ),
        }
    }
}
