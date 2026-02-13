use super::analyzer::{types_compatible, SemanticAnalyzer, SymbolInfo};
use super::types::{NamedError, SemanticError, TypeMismatch};
use crate::lexer::token::TokenType;
use crate::limits::ANALYZER_MAX_DEPTH;
use crate::parser::ast::{AstNode, Pattern, TypeNode};
use std::cell::RefCell;

/// Helper to extract line/col from an AstNode
/// For now, returns None since parser hasn't been updated yet
fn get_node_location(_node: &AstNode) -> (Option<usize>, Option<usize>) {
    // 🟡 TODO: Once parser is updated to include line/col in AST nodes,
    // implement proper extraction here
    (None, None)
}

impl SemanticAnalyzer {
    /// Infers the type of an AST node with an expected type context.
    /// If the inferred type is `Any` (e.g., from JSON.parse), returns the expected type instead.
    /// This allows proper type checking when passing dynamic values to typed parameters.
    /// Also handles empty arrays and maps by using the expected type.
    pub fn infer_type_with_expected(
        &self,
        node: &AstNode,
        expected: &TypeNode,
    ) -> Result<TypeNode, SemanticError> {
        // Special case: empty array literal should use expected array type
        if let AstNode::ArrayLiteral(elements) = node {
            if elements.is_empty() {
                if let TypeNode::Array(_) = expected {
                    return Ok(expected.clone());
                }
            }
        }

        // Special case: empty map literal should use expected map type
        if let AstNode::MapLiteral(pairs) = node {
            if pairs.is_empty() {
                if let TypeNode::Map(_, _) = expected {
                    return Ok(expected.clone());
                }
            }
        }

        let inferred = self.infer_type(node)?;
        // If inferred type is Any, use the expected type instead
        if matches!(inferred, TypeNode::Any) {
            Ok(expected.clone())
        } else {
            Ok(inferred)
        }
    }

    /// Infers the type of an AST node (expression).
    /// This is the core type inference function for all expressions in the language.
    /// - Returns the type of literals directly.
    /// - Looks up identifiers in the symbol table.
    /// - Checks types for binary/unary expressions, function calls, arrays, maps, etc.
    /// - Returns errors for undeclared variables, type mismatches, or invalid operations.
    pub fn infer_type(&self, node: &AstNode) -> Result<TypeNode, SemanticError> {
        // Check and increment recursion depth for type inference
        let mut depth = self.type_inference_depth.borrow_mut();
        *depth += 1;
        if *depth > ANALYZER_MAX_DEPTH {
            *depth -= 1;
            return Err(SemanticError::UnexpectedNode {
                expected: "Type inference recursion too deep (limit exceeded)".to_string(),
            });
        }
        drop(depth); // Release the borrow

        let result = match node {
            AstNode::NumberLiteral(_) => Ok(TypeNode::Int),
            // Float literal: always Float type
            AstNode::FloatLiteral(_) => Ok(TypeNode::Float),
            // String literal: always String type
            AstNode::StringLiteral(_s) => {
                // String literals support interpolation via ${...}
                // The parser will expand these into string concatenation
                Ok(TypeNode::String)
            }
            // Boolean literal: always Bool type
            AstNode::BoolLiteral(_name) => Ok(TypeNode::Bool),
            // Nil literal: polymorphic null value - compatible with any pointer/optional type
            AstNode::NilLiteral => Ok(TypeNode::Nil),

            // Range literal: start..end or start..=end
            AstNode::Range {
                start,
                end,
                inclusive,
            } => {
                let start_type = self.infer_type(start)?;
                let end_type = self.infer_type(end)?;

                // Both start and end must be Int
                if start_type != TypeNode::Int {
                    let (line, col) = get_node_location(start);
                    return Err(SemanticError::OperatorTypeMismatch(TypeMismatch {
                        expected: TypeNode::Int,
                        found: start_type,
                        value: None,
                        line,
                        col,
                    }));
                }
                if end_type != TypeNode::Int {
                    let (line, col) = get_node_location(end);
                    return Err(SemanticError::OperatorTypeMismatch(TypeMismatch {
                        expected: TypeNode::Int,
                        found: end_type,
                        value: None,
                        line,
                        col,
                    }));
                }

                Ok(TypeNode::Range(
                    Box::new(TypeNode::Int),
                    Box::new(TypeNode::Int),
                    *inclusive,
                ))
            }

            // Spread element - should not be analyzed standalone, only within array/map literals
            AstNode::SpreadElement(inner) => {
                let inner_type = self.infer_type(inner)?;
                // Return the inner type as-is for analysis
                Ok(inner_type)
            }
            // Identifier (variable name): look up in symbol table (with shadowing support)
            AstNode::Identifier(name) => {
                // Check for builtin identifiers first
                if name == "JSON" {
                    return Ok(TypeNode::Builtin("JSON".to_string()));
                }

                if let Some(info) = self.lookup_variable(name) {
                    Ok(info.ty.clone())
                } else if let Some(outer) = &self.outer_symbol_table {
                    if let Some(info) = outer.get(name) {
                        // Allow access to imported types (structs, enums, type references) from outer scope
                        // These are registered at module level and should be accessible within function bodies
                        match &info.ty {
                            TypeNode::Struct(_, _) | TypeNode::Enum(_, _) | TypeNode::TypeRef(_) => {
                                return Ok(info.ty.clone());
                            }
                            _ => {
                                // For regular variables from outer scope, return out of scope error
                                return Err(SemanticError::OutOfScopeVariable(NamedError {
                                    name: name.clone(),
                                }));
                            }
                        }
                    }
                    Err(SemanticError::UndeclaredVariable(NamedError {
                        name: name.clone(),
                    }))
                } else {
                    Err(SemanticError::UndeclaredVariable(NamedError {
                        name: name.clone(),
                    }))
                }
            }

            // Binary expressions (e.g., arithmetic, comparison, logical, range)
            // Ex., let is_equal = x == y;
            // TODO: check llvm handled for this or not
            AstNode::BinaryExpr { left, op, right } => {
                // Infer types of both sides
                let left_type = self.infer_type(left)?;
                let right_type = self.infer_type(right)?;

                match op {
                    // "in" operator for checking key existence in maps or element in arrays
                    TokenType::In => {
                        // Left side should be the key/element type, right side should be a map or array
                        match &right_type {
                            TypeNode::Map(key_type, _) => {
                                // Check if left type matches the map's key type
                                if !super::analyzer::types_compatible(
                                    &left_type,
                                    key_type,
                                    &self.struct_table,
                                    &self.enum_table,
                                ) {
                                    let (line, col) = get_node_location(node);
                                    return Err(SemanticError::OperatorTypeMismatch(
                                        TypeMismatch {
                                            expected: (**key_type).clone(),
                                            found: left_type,
                                            value: None,
                                            line,
                                            col,
                                        },
                                    ));
                                }
                                // "in" operator returns Bool
                                Ok(TypeNode::Bool)
                            }
                            TypeNode::Array(elem_type) => {
                                // Check if left type matches the array's element type
                                if !super::analyzer::types_compatible(
                                    &left_type,
                                    elem_type,
                                    &self.struct_table,
                                    &self.enum_table,
                                ) {
                                    let (line, col) = get_node_location(node);
                                    return Err(SemanticError::OperatorTypeMismatch(
                                        TypeMismatch {
                                            expected: (**elem_type).clone(),
                                            found: left_type,
                                            value: None,
                                            line,
                                            col,
                                        },
                                    ));
                                }
                                // "in" operator returns Bool
                                Ok(TypeNode::Bool)
                            }
                            _ => {
                                let (line, col) = get_node_location(node);
                                Err(SemanticError::OperatorTypeMismatch(TypeMismatch {
                                    expected: TypeNode::Map(
                                        Box::new(TypeNode::String),
                                        Box::new(TypeNode::Int),
                                    ),
                                    found: right_type,
                                    value: None,
                                    line,
                                    col,
                                }))
                            }
                        }
                    }

                    // Comparison operators (==, !=, >, <, etc.)
                    TokenType::EqEq
                    | TokenType::NotEq
                    | TokenType::Gt
                    | TokenType::Lt
                    | TokenType::GtEq
                    | TokenType::LtEq => {
                        // Both sides must be the same type, EXCEPT for Nil which is compatible with any type
                        // Nil can be compared with any type for equality/inequality checks
                        let types_compatible = if matches!(op, TokenType::EqEq | TokenType::NotEq) {
                            // For == and !=, Nil is compatible with any type
                            super::analyzer::types_compatible(
                                &left_type,
                                &right_type,
                                &self.struct_table,
                                &self.enum_table,
                            ) || left_type == TypeNode::Nil
                                || right_type == TypeNode::Nil
                        } else {
                            // For <, >, <=, >=, types must match using types_compatible
                            super::analyzer::types_compatible(
                                &left_type,
                                &right_type,
                                &self.struct_table,
                                &self.enum_table,
                            )
                        };

                        if !types_compatible {
                            let (line, col) = get_node_location(node);
                            return Err(SemanticError::OperatorTypeMismatch(TypeMismatch {
                                expected: left_type,
                                found: right_type,
                                value: None,
                                line,
                                col,
                            }));
                        }

                        if matches!(
                            op,
                            TokenType::Gt | TokenType::Lt | TokenType::GtEq | TokenType::LtEq
                        ) {
                            if left_type != TypeNode::Int && left_type != TypeNode::Float {
                                let (line, col) = get_node_location(node);
                                return Err(SemanticError::OperatorTypeMismatch(TypeMismatch {
                                    expected: TypeNode::Int,
                                    found: left_type,
                                    value: None,
                                    line,
                                    col,
                                }));
                            }
                        }

                        // Comparison always returns Bool
                        Ok(TypeNode::Bool)
                    }

                    // Range operators for loops (.. and ..=)
                    // Ex., for i in 0..10 {
                    // TODO: check llvm handled for this or not
                    TokenType::RangeExc | TokenType::RangeInc => {
                        // Both start and end must be Int
                        if left_type != TypeNode::Int || right_type != TypeNode::Int {
                            let (line, col) = get_node_location(node);
                            return Err(SemanticError::OperatorTypeMismatch(TypeMismatch {
                                expected: TypeNode::Int,
                                found: if left_type != TypeNode::Int {
                                    left_type
                                } else {
                                    right_type
                                },
                                value: None,
                                line,
                                col,
                            }));
                        }
                        // Determine if range is inclusive or exclusive
                        let inclusive = matches!(op, TokenType::RangeInc);
                        // Return Range type
                        Ok(TypeNode::Range(
                            Box::new(TypeNode::Int),
                            Box::new(TypeNode::Int),
                            inclusive,
                        ))
                    }

                    // Logical operators (&&, ||)
                    // Ex., let a = true;
                    // let b = a && c;
                    // TODO: check llvm handled for this or not
                    TokenType::AndAnd | TokenType::OrOr => {
                        // Both sides must be Bool
                        if left_type != TypeNode::Bool || right_type != TypeNode::Bool {
                            let (line, col) = get_node_location(node);
                            return Err(SemanticError::OperatorTypeMismatch(TypeMismatch {
                                expected: TypeNode::Bool,
                                found: if left_type != TypeNode::Bool {
                                    left_type
                                } else {
                                    right_type
                                },
                                value: None,
                                line,
                                col,
                            }));
                        }
                        Ok(TypeNode::Bool)
                    }

                    // Arithmetic operators (+, -, *, /, %)
                    // Ex., let a = "hello" + "world";
                    // Ex., let b = 1 + 2;
                    TokenType::Plus
                    | TokenType::Minus
                    | TokenType::Star
                    | TokenType::Slash
                    | TokenType::Percent => {
                        // Plus allows string concatenation with mixed types
                        if op == &TokenType::Plus {
                            match (left_type.clone(), right_type.clone()) {
                                // Int + Int -> Int
                                (TypeNode::Int, TypeNode::Int) => Ok(TypeNode::Int),
                                // Float + Float -> Float
                                (TypeNode::Float, TypeNode::Float) => Ok(TypeNode::Float),
                                // Int + Float -> Float (arithmetic coercion)
                                (TypeNode::Int, TypeNode::Float) => Ok(TypeNode::Float),
                                // Float + Int -> Float (arithmetic coercion)
                                (TypeNode::Float, TypeNode::Int) => Ok(TypeNode::Float),
                                // String + String -> String (concatenation)
                                (TypeNode::String, TypeNode::String) => Ok(TypeNode::String),
                                // String + Int -> String (concatenation)
                                (TypeNode::String, TypeNode::Int) => Ok(TypeNode::String),
                                // Int + String -> String (concatenation)
                                (TypeNode::Int, TypeNode::String) => Ok(TypeNode::String),
                                // String + Float -> String (concatenation)
                                (TypeNode::String, TypeNode::Float) => Ok(TypeNode::String),
                                // Float + String -> String (concatenation)
                                (TypeNode::Float, TypeNode::String) => Ok(TypeNode::String),
                                // String + Bool -> String (for interpolation)
                                (TypeNode::String, TypeNode::Bool) => Ok(TypeNode::String),
                                // Bool + String -> String (for interpolation)
                                (TypeNode::Bool, TypeNode::String) => Ok(TypeNode::String),
                                // String + Array -> String (for interpolation)
                                (TypeNode::String, TypeNode::Array(_)) => Ok(TypeNode::String),
                                // Array + String -> String (for interpolation)
                                (TypeNode::Array(_), TypeNode::String) => Ok(TypeNode::String),
                                // String + Map -> String (for interpolation)
                                (TypeNode::String, TypeNode::Map(_, _)) => Ok(TypeNode::String),
                                // Map + String -> String (for interpolation)
                                (TypeNode::Map(_, _), TypeNode::String) => Ok(TypeNode::String),
                                // String + Struct -> String (for interpolation)
                                (TypeNode::String, TypeNode::Struct(_, _)) => Ok(TypeNode::String),
                                // Struct + String -> String (for interpolation)
                                (TypeNode::Struct(_, _), TypeNode::String) => Ok(TypeNode::String),
                                // String + TypeRef -> String (for interpolation, struct references)
                                (TypeNode::String, TypeNode::TypeRef(_)) => Ok(TypeNode::String),
                                // TypeRef + String -> String (for interpolation)
                                (TypeNode::TypeRef(_), TypeNode::String) => Ok(TypeNode::String),
                                // String + Enum -> String (for interpolation)
                                (TypeNode::String, TypeNode::Enum(_, _)) => Ok(TypeNode::String),
                                // Enum + String -> String (for interpolation)
                                (TypeNode::Enum(_, _), TypeNode::String) => Ok(TypeNode::String),
                                // String + Tuple -> String (for interpolation)
                                (TypeNode::String, TypeNode::Tuple(_)) => Ok(TypeNode::String),
                                // Tuple + String -> String (for interpolation)
                                (TypeNode::Tuple(_), TypeNode::String) => Ok(TypeNode::String),
                                // String + Result -> String (for interpolation)
                                (TypeNode::String, TypeNode::Result(_, _)) => Ok(TypeNode::String),
                                // Result + String -> String (for interpolation)
                                (TypeNode::Result(_, _), TypeNode::String) => Ok(TypeNode::String),
                                // String + Any -> String (for dynamic types like JSON)
                                (TypeNode::String, TypeNode::Any) => Ok(TypeNode::String),
                                // Any + String -> String (for dynamic types)
                                (TypeNode::Any, TypeNode::String) => Ok(TypeNode::String),
                                // Any other type combination is invalid
                                _ => {
                                    let (line, col) = get_node_location(node);
                                    Err(SemanticError::OperatorTypeMismatch(TypeMismatch {
                                        expected: left_type,
                                        found: right_type,
                                        value: None,
                                        line,
                                        col,
                                    }))
                                }
                            }
                        } else if op == &TokenType::Percent {
                            // Modulo (%) is only supported for integers
                            if left_type == TypeNode::Int && right_type == TypeNode::Int {
                                Ok(TypeNode::Int)
                            } else {
                                let (line, col) = get_node_location(node);
                                return Err(SemanticError::OperatorTypeMismatch(TypeMismatch {
                                    expected: TypeNode::Int,
                                    found: if left_type != TypeNode::Int {
                                        left_type
                                    } else {
                                        right_type
                                    },
                                    value: None,
                                    line,
                                    col,
                                }));
                            }
                        } else {
                            // For other arithmetic operators (-, *, /), no concatenation allowed
                            match (left_type.clone(), right_type.clone()) {
                                // Int with Int
                                (TypeNode::Int, TypeNode::Int) => Ok(TypeNode::Int),
                                // Float with Float
                                (TypeNode::Float, TypeNode::Float) => Ok(TypeNode::Float),
                                // Int with Float or Float with Int -> Float
                                (TypeNode::Int, TypeNode::Float) => Ok(TypeNode::Float),
                                (TypeNode::Float, TypeNode::Int) => Ok(TypeNode::Float),
                                // Any other type combination is invalid
                                _ => {
                                    let (line, col) = get_node_location(node);
                                    Err(SemanticError::OperatorTypeMismatch(TypeMismatch {
                                        expected: left_type,
                                        found: right_type,
                                        value: None,
                                        line,
                                        col,
                                    }))
                                }
                            }
                        }
                    }

                    // Any other operator is not implemented
                    _ => unimplemented!("Operator {:?} not handled", op),
                }
            }

            // Unary expressions (e.g., -x, !x): infer type of the inner expression
            // Ex., let neg = -x;
            // Ex., let not = !flag;
            // TODO: check llvm handled for this or not
            AstNode::UnaryExpr { op, expr } => {
                let expr_type = self.infer_type(expr)?;
                match op {
                    TokenType::Minus => match expr_type {
                        TypeNode::Int | TypeNode::Float => Ok(expr_type),
                        _ => {
                            let (line, col) = get_node_location(expr);
                            Err(SemanticError::OperatorTypeMismatch(TypeMismatch {
                                expected: TypeNode::Int,
                                found: expr_type,
                                value: None,
                                line,
                                col,
                            }))
                        }
                    },
                    TokenType::Bang => {
                        if expr_type == TypeNode::Bool {
                            Ok(TypeNode::Bool)
                        } else {
                            let (line, col) = get_node_location(expr);
                            Err(SemanticError::OperatorTypeMismatch(TypeMismatch {
                                expected: TypeNode::Bool,
                                found: expr_type,
                                value: None,
                                line,
                                col,
                            }))
                        }
                    }
                    _ => Err(SemanticError::UnexpectedNode {
                        expected: "Minus or Bang operator".to_string(),
                    }),
                }
            }

            // Function call: infer return type from function signature
            // Ex., let result = myFunction(1, "abc");
            AstNode::FunctionCall { func, args } => {
                // Function must be an identifier
                // - Allowed: `myFunction(1, 2)`
                // - Not allowed: `(some_expr)(1, 2)` or `foo.bar(1, 2)`
                let name = if let AstNode::Identifier(n) = &**func {
                    n
                } else {
                    return Err(SemanticError::InvalidFunctionCall {
                        func: format!("{:?}", func),
                    });
                };

                // Check if this is actually an enum variant with data (Status::Pending(25))
                // Parser can't distinguish between enum variant and namespace function call
                if name.contains("::") {
                    let parts: Vec<&str> = name.split("::").collect();
                    if parts.len() == 2 {
                        let enum_name = parts[0];
                        let variant_name = parts[1];

                        // Check if the first part is an enum type
                        if let Some(enum_variants) = self.enum_table.get(enum_name) {
                            // Check if the variant exists
                            if let Some(variant_type) = enum_variants.get(variant_name) {
                                // This is an enum variant, not a function call
                                // Verify payload: should have exactly 1 argument
                                if args.len() == 1 {
                                    if let Some(_expected_type) = variant_type {
                                        // TODO: Type check the argument
                                        let _actual_type = self.infer_type(&args[0])?;
                                        return Ok(TypeNode::Enum(
                                            enum_name.to_string(),
                                            enum_variants.clone(),
                                        ));
                                    } else {
                                        return Err(SemanticError::UndeclaredVariable(
                                            NamedError {
                                                name: format!(
                                                    "Variant '{}::{}' does not take a payload",
                                                    enum_name, variant_name
                                                ),
                                            },
                                        ));
                                    }
                                } else {
                                    return Err(SemanticError::UndeclaredVariable(NamedError {
                                        name: format!("Variant '{}::{}' requires exactly one argument, got {}", enum_name, variant_name, args.len()),
                                    }));
                                }
                            }
                        }
                    }
                }

                // Look up function in function table
                if let Some((_param_types, ret_ty, err_ty)) = self.function_table.get(name) {
                    // If the function has an error type, wrap the return type in Result
                    // This prevents automatic tuple unpacking for functions with error handling
                    if let Some(error_type) = err_ty {
                        Ok(TypeNode::Result(
                            Box::new(ret_ty.clone()),
                            Box::new(error_type.clone()),
                        ))
                    } else {
                        Ok(ret_ty.clone())
                    }
                } else {
                    // Function not found
                    Err(SemanticError::UndeclaredFunction(NamedError {
                        name: name.clone(),
                    }))
                }
            }

            // Method call: infer return type based on object type and method name
            AstNode::MethodCall {
                object,
                method,
                args,
            } => {
                let object_type = self.infer_type(object)?;

                // Check mutability for methods that modify the array
                self.check_method_mutability(object, method)?;

                self.infer_method_return_type(&object_type, method, args)
            }

            // Array literal: infer type of elements
            AstNode::ArrayLiteral(elements) => {
                // Error if array is empty: cannot infer type
                // let empty = [];
                if elements.is_empty() {
                    // Allow empty array: infer type from annotation if present, otherwise default to Array<Int>
                    // Note: Type annotation should be passed from analyze_let_decl when available
                    return Ok(TypeNode::Array(Box::new(TypeNode::Int)));
                }

                // Find first non-spread element to determine array type
                let mut first_type: Option<TypeNode> = None;
                for el in elements.iter() {
                    match el {
                        AstNode::SpreadElement(inner) => {
                            // Spread element: ensure it's an array type
                            let spread_type = self.infer_type(inner)?;
                            match &spread_type {
                                TypeNode::Array(elem_type) => {
                                    if first_type.is_none() {
                                        first_type = Some((**elem_type).clone());
                                    } else if let Some(ref ft) = first_type {
                                        if **elem_type != *ft {
                                            let (line, col) = get_node_location(el);
                                            return Err(SemanticError::VarTypeMismatch(
                                                TypeMismatch {
                                                    expected: TypeNode::Array(Box::new(ft.clone())),
                                                    found: spread_type.clone(),
                                                    value: None,
                                                    line,
                                                    col,
                                                },
                                            ));
                                        }
                                    }
                                }
                                _ => {
                                    let (line, col) = get_node_location(el);
                                    return Err(SemanticError::VarTypeMismatch(TypeMismatch {
                                        expected: TypeNode::Array(Box::new(TypeNode::Int)),
                                        found: spread_type.clone(),
                                        value: None,
                                        line,
                                        col,
                                    }));
                                }
                            }
                        }
                        _ => {
                            let t = self.infer_type(el)?;
                            if first_type.is_none() {
                                first_type = Some(t.clone());
                            } else if let Some(ref ft) = first_type {
                                if t != *ft {
                                    let (line, col) = get_node_location(el);
                                    return Err(SemanticError::VarTypeMismatch(TypeMismatch {
                                        expected: ft.clone(),
                                        found: t,
                                        value: None,
                                        line,
                                        col,
                                    }));
                                }
                            }
                        }
                    }
                }

                // All elements are the same type: return Array of that type
                Ok(TypeNode::Array(Box::new(
                    first_type.unwrap_or(TypeNode::Int),
                )))
            }

            // Map literal: infer type of keys and values
            AstNode::MapLiteral(pairs) => {
                // Allow empty map: infer type from annotation if present, otherwise default to Map<String, Int>
                if pairs.is_empty() {
                    // If you want to support type annotation, you can pass it in or check node context.
                    // For now, default to Map<String, Int>
                    return Ok(TypeNode::Map(
                        Box::new(TypeNode::String),
                        Box::new(TypeNode::Int),
                    ));
                }

                // Find first non-spread pair to determine map types
                let mut key_type: Option<TypeNode> = None;
                let mut value_type: Option<TypeNode> = None;

                for (k, v) in pairs.iter() {
                    match k {
                        AstNode::SpreadElement(inner) => {
                            // Spread element: ensure it's a map type
                            let spread_type = self.infer_type(inner)?;
                            match &spread_type {
                                TypeNode::Map(kt, vt) => {
                                    if key_type.is_none() {
                                        key_type = Some((**kt).clone());
                                        value_type = Some((**vt).clone());
                                    } else {
                                        // Verify types match
                                        if let (Some(ref expected_kt), Some(ref expected_vt)) =
                                            (&key_type, &value_type)
                                        {
                                            if **kt != *expected_kt || **vt != *expected_vt {
                                                let (line, col) = get_node_location(k);
                                                return Err(SemanticError::VarTypeMismatch(
                                                    TypeMismatch {
                                                        expected: TypeNode::Map(
                                                            Box::new(expected_kt.clone()),
                                                            Box::new(expected_vt.clone()),
                                                        ),
                                                        found: spread_type.clone(),
                                                        value: None,
                                                        line,
                                                        col,
                                                    },
                                                ));
                                            }
                                        }
                                    }
                                }
                                _ => {
                                    let (line, col) = get_node_location(k);
                                    return Err(SemanticError::VarTypeMismatch(TypeMismatch {
                                        expected: TypeNode::Map(
                                            Box::new(TypeNode::String),
                                            Box::new(TypeNode::Int),
                                        ),
                                        found: spread_type.clone(),
                                        value: None,
                                        line,
                                        col,
                                    }));
                                }
                            }
                        }
                        _ => {
                            let kt = self.infer_type(k)?;
                            let vt = self.infer_type(v)?;

                            if key_type.is_none() {
                                key_type = Some(kt.clone());
                                value_type = Some(vt.clone());
                            } else {
                                // Verify types match
                                if let (Some(ref expected_kt), Some(ref expected_vt)) =
                                    (&key_type, &value_type)
                                {
                                    if kt != *expected_kt {
                                        let (line, col) = get_node_location(k);
                                        return Err(SemanticError::VarTypeMismatch(TypeMismatch {
                                            expected: expected_kt.clone(),
                                            found: kt,
                                            value: None,
                                            line,
                                            col,
                                        }));
                                    }
                                    if vt != *expected_vt {
                                        let (line, col) = get_node_location(v);
                                        return Err(SemanticError::VarTypeMismatch(TypeMismatch {
                                            expected: expected_vt.clone(),
                                            found: vt,
                                            value: None,
                                            line,
                                            col,
                                        }));
                                    }
                                }
                            }
                        }
                    }
                }

                let final_key_type = key_type.unwrap_or(TypeNode::String);
                let final_value_type = value_type.unwrap_or(TypeNode::Int);

                // Only allow Int, String, Float, or Bool as map keys
                // TODO: check codegen if implemented or not
                match final_key_type {
                    TypeNode::Int | TypeNode::String | TypeNode::Float | TypeNode::Bool => {}
                    _ => {
                        return Err(SemanticError::InvalidMapKeyType {
                            found: final_key_type.clone(),
                            expected: TypeNode::Map(
                                Box::new(TypeNode::Int),
                                Box::new(TypeNode::String),
                            ),
                        });
                    }
                }

                // Check all pairs for type consistency
                // All key-value pairs have consistent types
                Ok(TypeNode::Map(
                    Box::new(final_key_type),
                    Box::new(final_value_type),
                ))
            }

            // Object literal: heterogeneous object used for inline options/config
            // Treat as Any so it can be passed into builtins that interpret it structurally.
            AstNode::ObjectLiteral(_entries) => Ok(TypeNode::Any),

            // Element access: arr[index] or map[key] or arr[start..end]
            // Infer type of the array/map and the index/key
            AstNode::ElementAccess { array, index } => {
                let array_type = self.infer_type(array)?;
                let index_type = self.infer_type(index)?;

                // Check if this is a range/slice operation
                match &index_type {
                    TypeNode::Range(_, _, _) => {
                        // Slicing returns the same array/string type
                        match &array_type {
                            TypeNode::Array(_) => return Ok(array_type),
                            TypeNode::String => return Ok(TypeNode::String),
                            _ => {
                                let (line, col) = get_node_location(array);
                                return Err(SemanticError::VarTypeMismatch(TypeMismatch {
                                    expected: TypeNode::Array(Box::new(TypeNode::Int)),
                                    found: array_type,
                                    value: None,
                                    line,
                                    col,
                                }));
                            }
                        }
                    }
                    _ => {}
                }

                // Reject negative indices for arrays
                if let AstNode::UnaryExpr {
                    op: TokenType::Minus,
                    expr: _,
                } = &**index
                {
                    return Err(SemanticError::InvalidAssignmentTarget {
                        target: "Array indices cannot be negative".to_string(),
                    });
                }

                match array_type {
                    // Array element access: arr[Int] -> T
                    TypeNode::Array(element_type) => {
                        // Index must be an Int
                        if index_type != TypeNode::Int {
                            let (line, col) = get_node_location(index);
                            return Err(SemanticError::OperatorTypeMismatch(TypeMismatch {
                                expected: TypeNode::Int,
                                found: index_type,
                                value: None,
                                line,
                                col,
                            }));
                        }
                        // Return the element type
                        Ok(*element_type)
                    }
                    // Map element access: map[Key] -> Value
                    TypeNode::Map(key_type, value_type) => {
                        // Index must match the key type
                        if index_type != *key_type {
                            let (line, col) = get_node_location(index);
                            return Err(SemanticError::OperatorTypeMismatch(TypeMismatch {
                                expected: *key_type,
                                found: index_type,
                                value: None,
                                line,
                                col,
                            }));
                        }
                        // Return the value type
                        Ok(*value_type)
                    }
                    // Element access on non-indexable type
                    _ => {
                        let (line, col) = get_node_location(array);
                        Err(SemanticError::OperatorTypeMismatch(TypeMismatch {
                            expected: TypeNode::Array(Box::new(TypeNode::Int)),
                            found: array_type,
                            value: None,
                            line,
                            col,
                        }))
                    }
                }
            }

            // Type casting: expr as TargetType
            AstNode::Cast { expr, target_type } => {
                let source_type = self.infer_type(expr)?;

                match (&source_type, target_type) {
                    // Int casts
                    (TypeNode::Int, TypeNode::Int) => Ok(TypeNode::Int),
                    (TypeNode::Int, TypeNode::Float) => Ok(TypeNode::Float),
                    (TypeNode::Int, TypeNode::String) => Ok(TypeNode::String),
                    (TypeNode::Int, TypeNode::Bool) => Err(SemanticError::UnexpectedNode {
                        expected: "Int to Bool is not allowed".to_string(),
                    }),

                    // Float casts
                    (TypeNode::Float, TypeNode::Int) => Ok(TypeNode::Int),
                    (TypeNode::Float, TypeNode::Float) => Ok(TypeNode::Float),
                    (TypeNode::Float, TypeNode::String) => Ok(TypeNode::String),
                    (TypeNode::Float, TypeNode::Bool) => Err(SemanticError::UnexpectedNode {
                        expected: "Float to Bool is not allowed".to_string(),
                    }),

                    // Bool casts
                    (TypeNode::Bool, TypeNode::Int) => Ok(TypeNode::Int),
                    (TypeNode::Bool, TypeNode::String) => Ok(TypeNode::String),
                    (TypeNode::Bool, TypeNode::Float) => Err(SemanticError::UnexpectedNode {
                        expected: "Bool to Float is not allowed".to_string(),
                    }),
                    (TypeNode::Bool, TypeNode::Bool) => Ok(TypeNode::Bool),

                    // String casts
                    (TypeNode::String, TypeNode::Int) => Ok(TypeNode::Int),
                    (TypeNode::String, TypeNode::Float) => Ok(TypeNode::Float),
                    (TypeNode::String, TypeNode::String) => Ok(TypeNode::String),
                    (TypeNode::String, TypeNode::Bool) => Err(SemanticError::UnexpectedNode {
                        expected: "String to Bool is not allowed".to_string(),
                    }),

                    // Identity cast
                    (src, tgt) if *src == *tgt => Ok(tgt.clone()),
                    // ... other arms, update src to &src as needed ...
                    (_, tgt) => Err(SemanticError::UnexpectedNode {
                        expected: format!("Cast to {} is not allowed from {:?}", tgt, source_type),
                    }),
                }
            }

            // Ok expression: infer from the values
            AstNode::OkExpr { values } => {
                if values.is_empty() {
                    Ok(TypeNode::Void)
                } else if values.len() == 1 {
                    self.infer_type(&values[0])
                } else {
                    // Multiple values: infer as tuple
                    let types: Result<Vec<TypeNode>, SemanticError> =
                        values.iter().map(|v| self.infer_type(v)).collect();
                    Ok(TypeNode::Tuple(types?))
                }
            }

            // Err expression: infer from the error value
            AstNode::ErrExpr { value } => self.infer_type(value),

            // Try propagate: infer from the expression being propagated
            // The ? operator unwraps a Result<T, E> to just T
            AstNode::TryPropagate { expr } => {
                // Validate that ? is used inside a function with error type
                if self.current_function_error_type.is_none() {
                    return Err(SemanticError::UnexpectedNode {
                        expected: "? operator can only be used in functions with error return type (e.g., -> T ! E or ! E)".to_string(),
                    });
                }
                
                let expr_type = self.infer_type(expr)?;
                match expr_type {
                    TypeNode::Result(ok_type, _err_type) => Ok(*ok_type),
                    // If not a Result type, just return the type as-is
                    other => Ok(other),
                }
            }

            // UnwrapOrPanic: ?? panic() operator
            // Unwraps a Result<T, E> to T, or panics if there's an error
            AstNode::UnwrapOrPanic { expr, panic_msg } => {
                // Validate panic message expression
                self.infer_type(panic_msg)?;

                let expr_type = self.infer_type(expr)?;
                match expr_type {
                    TypeNode::Result(ok_type, _err_type) => Ok(*ok_type),
                    // If not a Result type, just return the type as-is
                    other => Ok(other),
                }
            }

            // Block: infer type from the last statement/expression
            AstNode::Block(statements) => {
                if statements.is_empty() {
                    return Ok(TypeNode::Void);
                }

                // For blocks, infer the type from the last statement
                // This is especially important for lambda bodies
                let last_stmt = &statements[statements.len() - 1];
                match last_stmt {
                    AstNode::Return { values } => {
                        if values.is_empty() {
                            Ok(TypeNode::Void)
                        } else {
                            self.infer_type(&values[0])
                        }
                    }
                    _ => {
                        // For other statements, just infer their type
                        self.infer_type(last_stmt)
                    }
                }
            }

            // Struct literal: Point { x: 10, y: 20 }
            AstNode::StructLiteral { name, fields } => {
                // Check if struct type exists
                if let Some(struct_fields) = self.struct_table.get(name) {
                    // IMPORTANT: Check if struct is accessible
                    // For imported structs, they must be in symbol_table or outer scopes to be instantiated
                    // This enforces proper import semantics (namespace imports don't expose structs directly)
                    // Local structs (declared in current module) are always accessible via struct_table
                    let is_imported = self.imported_struct_names.contains(name);

                    if is_imported {
                        // Check if struct is accessible in current scope or any outer scope
                        let mut is_accessible = self.symbol_table.contains_key(name);

                        // Check outer_symbol_table (for nested function scopes)
                        if !is_accessible {
                            if let Some(outer) = &self.outer_symbol_table {
                                is_accessible = outer.contains_key(name);
                            }
                        }

                        // Check scope_stack (for nested block scopes)
                        if !is_accessible {
                            for scope in self.scope_stack.iter().rev() {
                                if scope.contains_key(name) {
                                    is_accessible = true;
                                    break;
                                }
                            }
                        }

                        if !is_accessible {
                            return Err(SemanticError::UndeclaredVariable(NamedError {
                                name: format!(
                                    "Struct '{}' is not accessible. Did you import it? (Use 'import module::{}' to import directly)",
                                    name, name
                                ),
                            }));
                        }
                    }

                    // Check field visibility for imported structs

                    // Verify all required fields are provided
                    for (field_name, _field_type) in struct_fields {
                        // For imported structs, only require public fields
                        if is_imported {
                            if let Some(field_visibility) = self.struct_field_visibility.get(name) {
                                if let Some(is_public) = field_visibility.get(field_name) {
                                    if !*is_public {
                                        // Private field - skip requirement check (it should have a default or be handled internally)
                                        continue;
                                    }
                                }
                            }
                        }

                        let field_provided = fields.iter().any(|(f, _)| f == field_name);
                        if !field_provided {
                            // Check if field has default value or is optional
                            // For now, require all fields
                            return Err(SemanticError::UndeclaredVariable(NamedError {
                                name: format!(
                                    "Missing field '{}' in struct '{}'",
                                    field_name, name
                                ),
                            }));
                        }
                    }

                    // Verify field types match and check visibility for provided fields
                    for (field_name, field_value) in fields {
                        // Check field visibility for imported structs
                        if is_imported {
                            if let Some(field_visibility) = self.struct_field_visibility.get(name) {
                                if let Some(is_public) = field_visibility.get(field_name) {
                                    if !*is_public {
                                        return Err(SemanticError::PrivateFieldAccess {
                                            struct_name: name.clone(),
                                            field_name: field_name.clone(),
                                        });
                                    }
                                }
                            }
                        }

                        if let Some(expected_type) = struct_fields.get(field_name) {
                            // Use infer_type_with_expected to handle empty arrays/maps properly
                            let actual_type =
                                self.infer_type_with_expected(field_value, expected_type)?;
                            // Check type compatibility
                            if !types_compatible(
                                &actual_type,
                                expected_type,
                                &self.struct_table,
                                &self.enum_table,
                            ) {
                                return Err(SemanticError::VarTypeMismatch(TypeMismatch {
                                    expected: expected_type.clone(),
                                    found: actual_type,
                                    value: None,
                                    line: None,
                                    col: None,
                                }));
                            }
                        } else {
                            return Err(SemanticError::UndeclaredVariable(NamedError {
                                name: format!(
                                    "Unknown field '{}' in struct '{}'",
                                    field_name, name
                                ),
                            }));
                        }
                    }

                    Ok(TypeNode::Struct(name.clone(), struct_fields.clone()))
                } else {
                    Err(SemanticError::UndeclaredVariable(NamedError {
                        name: format!("Undefined struct type '{}'", name),
                    }))
                }
            }

            // Field access: obj.field
            AstNode::FieldAccess { object, field } => {
                let object_type = self.infer_type(object)?;

                // Duration sugar: 1.hour / 5.minutes / 30.seconds
                // This is a compile-time numeric convenience. It is only valid on Int.
                if object_type == TypeNode::Int {
                    if matches!(
                        field.as_str(),
                        "second" | "seconds" | "minute" | "minutes" | "hour" | "hours"
                    ) {
                        return Ok(TypeNode::Int);
                    }
                }

                // Resolve TypeRef to actual struct type
                let resolved_type = match &object_type {
                    TypeNode::TypeRef(name) => {
                        if let Some(fields) = self.struct_table.get(name) {
                            TypeNode::Struct(name.clone(), fields.clone())
                        } else {
                            object_type.clone()
                        }
                    }
                    _ => object_type.clone(),
                };

                match resolved_type {
                    TypeNode::Struct(struct_name, fields) => {
                        if let Some(field_type) = fields.get(field) {
                            // Check field visibility for imported structs
                            // If the struct is imported from another module, check if the field is public
                            if self.imported_struct_names.contains(&struct_name) {
                                if let Some(field_visibility) =
                                    self.struct_field_visibility.get(&struct_name)
                                {
                                    if let Some(is_public) = field_visibility.get(field) {
                                        if !*is_public {
                                            return Err(SemanticError::PrivateFieldAccess {
                                                struct_name: struct_name.clone(),
                                                field_name: field.clone(),
                                            });
                                        }
                                    }
                                }
                            }
                            Ok(field_type.clone())
                        } else {
                            Err(SemanticError::UndeclaredVariable(NamedError {
                                name: format!("Struct '{}' has no field '{}'", struct_name, field),
                            }))
                        }
                    }
                    _ => Err(SemanticError::UndeclaredVariable(NamedError {
                        name: format!(
                            "Cannot access field on non-struct type: {:?}",
                            resolved_type
                        ),
                    })),
                }
            }

            // Match expression
            AstNode::MatchExpr { values, arms } => {
                // Type check the match values if present
                for v in values {
                    self.infer_type(v)?;
                }

                // Infer the type from the first arm's body
                // All arms should return the same type
                if let Some(first_arm) = arms.first() {
                    let first_type = self.infer_type(&first_arm.body)?;

                    // Verify all other arms return the same type
                    for arm in arms.iter().skip(1) {
                        let arm_type = self.infer_type(&arm.body)?;
                        if !super::analyzer::types_compatible(
                            &arm_type,
                            &first_type,
                            &self.struct_table,
                            &self.enum_table,
                        ) {
                            return Err(SemanticError::VarTypeMismatch(TypeMismatch {
                                expected: first_type.clone(),
                                found: arm_type,
                                value: None,
                                line: None,
                                col: None,
                            }));
                        }
                    }

                    Ok(first_type)
                } else {
                    // Empty match should not happen, but return Void as fallback
                    Ok(TypeNode::Void)
                }
            }

            // Enum variant: Direction::North or Status::Active(value)
            // OR namespaced function call: File::Write(...)
            AstNode::EnumVariant {
                enum_name,
                variant,
                payload,
            } => {
                // First, check if enum type exists
                if let Some(enum_variants) = self.enum_table.get(enum_name) {
                    // Check if variant exists
                    if let Some(variant_type) = enum_variants.get(variant) {
                        // Verify payload matches
                        match (payload.is_empty(), variant_type) {
                            (false, Some(expected_type)) => {
                                // Enum variant with payload
                                // Check if it's a tuple type (multiple arguments expected)
                                if let TypeNode::Tuple(tuple_types) = expected_type {
                                    // Tuple payload - expect multiple arguments
                                    if payload.len() == tuple_types.len() {
                                        for (arg, expected_elem_type) in
                                            payload.iter().zip(tuple_types.iter())
                                        {
                                            let actual_type = self.infer_type(arg)?;
                                            // Type compatibility check for each element
                                            if !types_compatible(
                                                &actual_type,
                                                expected_elem_type,
                                                &self.struct_table,
                                                &self.enum_table,
                                            ) {
                                                return Err(SemanticError::VarTypeMismatch(
                                                    TypeMismatch {
                                                        expected: expected_elem_type.clone(),
                                                        found: actual_type,
                                                        value: None,
                                                        line: None,
                                                        col: None,
                                                    },
                                                ));
                                            }
                                        }
                                        Ok(TypeNode::Enum(enum_name.clone(), enum_variants.clone()))
                                    } else {
                                        Err(SemanticError::UndeclaredVariable(NamedError {
                                            name: format!(
                                                "Variant '{}::{}' expects {} arguments, got {}",
                                                enum_name,
                                                variant,
                                                tuple_types.len(),
                                                payload.len()
                                            ),
                                        }))
                                    }
                                } else if payload.len() == 1 {
                                    // Single payload
                                    let actual_type = self.infer_type(&payload[0])?;
                                    // Type compatibility check
                                    if !types_compatible(
                                        &actual_type,
                                        expected_type,
                                        &self.struct_table,
                                        &self.enum_table,
                                    ) {
                                        return Err(SemanticError::VarTypeMismatch(TypeMismatch {
                                            expected: expected_type.clone(),
                                            found: actual_type,
                                            value: None,
                                            line: None,
                                            col: None,
                                        }));
                                    }
                                    Ok(TypeNode::Enum(enum_name.clone(), enum_variants.clone()))
                                } else {
                                    Err(SemanticError::UndeclaredVariable(NamedError {
                                        name: format!(
                                            "Variant '{}::{}' expects 1 argument, got {}",
                                            enum_name,
                                            variant,
                                            payload.len()
                                        ),
                                    }))
                                }
                            }
                            (true, None) => {
                                // Unit variant with no payload
                                Ok(TypeNode::Enum(enum_name.clone(), enum_variants.clone()))
                            }
                            (false, None) => Err(SemanticError::UndeclaredVariable(NamedError {
                                name: format!(
                                    "Variant '{}::{}' does not take a payload",
                                    enum_name, variant
                                ),
                            })),
                            (true, Some(_)) => Err(SemanticError::UndeclaredVariable(NamedError {
                                name: format!(
                                    "Variant '{}::{}' requires a payload",
                                    enum_name, variant
                                ),
                            })),
                        }
                    } else {
                        Err(SemanticError::UndeclaredVariable(NamedError {
                            name: format!("Enum '{}' has no variant '{}'", enum_name, variant),
                        }))
                    }
                } else {
                    // Enum not found - check if it's a namespaced function call (e.g., File::Write)
                    let qualified_name = format!("{}::{}", enum_name, variant);

                    if let Some((_param_types, ret_ty, err_ty)) =
                        self.function_table.get(&qualified_name)
                    {
                        // It's a function call - type check all arguments
                        for arg in payload {
                            self.infer_type(arg)?;
                        }

                        // If the function has an error type, wrap the return type in Result
                        if let Some(error_type) = err_ty {
                            Ok(TypeNode::Result(
                                Box::new(ret_ty.clone()),
                                Box::new(error_type.clone()),
                            ))
                        } else {
                            Ok(ret_ty.clone())
                        }
                    } else {
                        // Check if it's a static method call
                        // Methods are stored as Type::method in function_table
                        // But also check Type.method for compatibility
                        let method_name_colon = format!("{}::{}", enum_name, variant);
                        let method_name_dot = format!("{}.{}", enum_name, variant);

                        if let Some((_param_types, ret_ty, err_ty)) = self
                            .function_table
                            .get(&method_name_colon)
                            .or_else(|| self.function_table.get(&method_name_dot))
                        {
                            // It's a static method - type check all arguments
                            for arg in payload {
                                self.infer_type(arg)?;
                            }

                            // If the function has an error type, wrap the return type in Result
                            if let Some(error_type) = err_ty {
                                Ok(TypeNode::Result(
                                    Box::new(ret_ty.clone()),
                                    Box::new(error_type.clone()),
                                ))
                            } else {
                                Ok(ret_ty.clone())
                            }
                        } else {
                            // Neither enum, function, nor static method found
                            // Check if the type exists (to give better error message)
                            let error_msg = if self.struct_table.contains_key(enum_name) {
                                format!(
                                    "Undefined method '{}' for type '{}'. Did you forget to import it explicitly?",
                                    variant, enum_name
                                )
                            } else if self.enum_table.contains_key(enum_name) {
                                format!(
                                    "Undefined enum variant '{}::{}'. Check available variants for enum '{}'",
                                    enum_name, variant, enum_name
                                )
                            } else {
                                format!(
                                    "Undefined function or type '{}'. Did you forget to import '{}' explicitly?",
                                    qualified_name, enum_name
                                )
                            };

                            Err(SemanticError::UndeclaredVariable(NamedError {
                                name: error_msg,
                            }))
                        }
                    }
                }
            }

            // Conditional expressions (inline if-else and ternary)
            AstNode::ConditionalExpr {
                condition,
                then_expr,
                else_expr,
            } => {
                // Type check condition (must be Bool)
                let cond_type = self.infer_type(condition)?;
                if !matches!(cond_type, TypeNode::Bool) {
                    return Err(SemanticError::VarTypeMismatch(TypeMismatch {
                        expected: TypeNode::Bool,
                        found: cond_type,
                        value: None,
                        line: None,
                        col: None,
                    }));
                }

                // Type check both branches and ensure they have compatible types
                let then_type = self.infer_type(then_expr)?;
                let else_type = self.infer_type(else_expr)?;

                // Both branches must have the same type
                if then_type != else_type {
                    return Err(SemanticError::VarTypeMismatch(TypeMismatch {
                        expected: then_type,
                        found: else_type,
                        value: None,
                        line: None,
                        col: None,
                    }));
                }

                Ok(then_type)
            }

            AstNode::TernaryExpr {
                condition,
                true_expr,
                false_expr,
            } => {
                // Type check condition (must be Bool)
                let cond_type = self.infer_type(condition)?;
                if !matches!(cond_type, TypeNode::Bool) {
                    return Err(SemanticError::VarTypeMismatch(TypeMismatch {
                        expected: TypeNode::Bool,
                        found: cond_type,
                        value: None,
                        line: None,
                        col: None,
                    }));
                }

                // Type check both branches and ensure they have compatible types
                let true_type = self.infer_type(true_expr)?;
                let false_type = self.infer_type(false_expr)?;

                // Both branches must have the same type
                if true_type != false_type {
                    return Err(SemanticError::VarTypeMismatch(TypeMismatch {
                        expected: true_type,
                        found: false_type,
                        value: None,
                        line: None,
                        col: None,
                    }));
                }

                Ok(true_type)
            }

            // Block expression: { statements; result_expr }
            // Use a temporary analyzer with cloned symbol table for proper scoping
            AstNode::BlockExpr { statements, result } => {
                // Create a temporary analyzer with cloned state
                let mut temp_analyzer = SemanticAnalyzer {
                    symbol_table: self.symbol_table.clone(),
                    function_table: self.function_table.clone(),
                    struct_table: self.struct_table.clone(),
                    enum_table: self.enum_table.clone(),
                    enum_variant_order: self.enum_variant_order.clone(),
                    method_table: self.method_table.clone(),
                    struct_field_visibility: self.struct_field_visibility.clone(),
                    imported_struct_names: self.imported_struct_names.clone(),
                    struct_field_decorators: self.struct_field_decorators.clone(),
                    ffi_metadata: self.ffi_metadata.clone(),
                    function_aliases: self.function_aliases.clone(),
                    loop_depth: self.loop_depth,
                    scope_stack: self.scope_stack.clone(),
                    function_depth: self.function_depth,
                    scope_sizes_stack: self.scope_sizes_stack.clone(),
                    outer_symbol_table: self.outer_symbol_table.clone(),
                    project_root: self.project_root.clone(),
                    imported_modules: self.imported_modules.clone(),
                    imported_functions: self.imported_functions.clone(),
                    imported_structs: self.imported_structs.clone(),
                    collected_errors: Vec::new(),
                    is_main_module: self.is_main_module,
                    type_inference_depth: RefCell::new(0),
                    current_function_error_type: self.current_function_error_type.clone(),
                };

                // Process statements in the temporary analyzer
                for stmt in statements {
                    match stmt {
                        AstNode::LetDecl {
                            mutable,
                            type_annotation: _,
                            pattern,
                            value,
                            is_ref_counted: _,
                        } => {
                            // Infer type and add to temp symbol table
                            if let Ok(ty) = temp_analyzer.infer_type(value) {
                                if let Pattern::Identifier(name) = pattern {
                                    temp_analyzer.symbol_table.insert(
                                        name.clone(),
                                        SymbolInfo {
                                            ty,
                                            mutable: *mutable,
                                            is_ref_counted: false,
                                            is_parameter: false,
                                        },
                                    );
                                }
                            }
                        }
                        _ => {
                            // Other statements, just infer type
                            let _ = temp_analyzer.infer_type(stmt);
                        }
                    }
                }

                // Infer the type of the result expression with the updated symbol table
                temp_analyzer.infer_type(result)
            }

            // Any other AST node (usually statements): return Void type.
            // Actual semantic checking for statements happens elsewhere.
            _ => Ok(TypeNode::Void),
        };

        // Decrement recursion depth when returning
        {
            let mut depth = self.type_inference_depth.borrow_mut();
            if *depth > 0 {
                *depth -= 1;
            }
        }

        result
    }

    /// Check if a method that requires mutability is being called on a mutable object
    fn check_method_mutability(&self, object: &AstNode, method: &str) -> Result<(), SemanticError> {
        // Methods that require the array to be mutable
        let mutating_methods = ["push", "pop", "set", "clear", "sort"];

        if !mutating_methods.contains(&method) {
            return Ok(());
        }

        // Check if the object is a mutable variable
        match object {
            AstNode::Identifier(name) => {
                if let Some(info) = self.lookup_variable(name) {
                    if !info.mutable {
                        return Err(SemanticError::InvalidAssignmentTarget {
                            target: format!("Cannot call mutating method '{}' on immutable array", method),
                        });
                    }
                }
                Ok(())
            }
            // For method chains like x.map().push(), reject because map returns immutable
            AstNode::MethodCall { .. } => {
                Err(SemanticError::InvalidAssignmentTarget {
                    target: format!("Cannot call mutating method '{}' on method result (returned arrays are immutable)", method),
                })
            }
            // For array literals and other expressions, they're immutable
            AstNode::ArrayLiteral(_) => {
                Err(SemanticError::InvalidAssignmentTarget {
                    target: format!("Cannot call mutating method '{}' on array literal (immutable)", method),
                })
            }
            _ => Ok(()),
        }
    }

    /// Helper function to infer the return type of a lambda/closure
    /// elem_type is the element type from the array (for type inference of lambda parameters)
    fn infer_lambda_return_type(
        &self,
        closure: &AstNode,
        elem_type: &TypeNode,
    ) -> Result<TypeNode, SemanticError> {
        match closure {
            AstNode::Closure {
                params,
                body,
                return_type,
                error_type: _, // Lambda error type not checked here
            } => {
                // If explicit return type is provided, use it
                if let Some(ret_type) = return_type {
                    return Ok(ret_type.clone());
                }

                // Create a temporary analyzer with the lambda's scope
                let mut temp_analyzer = self.clone_for_lambda_analysis();

                // Add parameters to the temporary analyzer's scope
                for (param_name, param_type) in params {
                    let inferred_type = if let Some(ty) = param_type {
                        ty.clone()
                    } else {
                        // Infer from array element type
                        elem_type.clone()
                    };
                    temp_analyzer.symbol_table.insert(
                        param_name.clone(),
                        SymbolInfo {
                            ty: inferred_type,
                            mutable: false,
                            is_ref_counted: false,
                            is_parameter: true,
                        },
                    );
                }

                // If body is a block, analyze statements sequentially to build symbol table
                if let AstNode::Block(statements) = body.as_ref() {
                    for stmt in statements {
                        match stmt {
                            AstNode::Return { values } => {
                                if !values.is_empty() {
                                    return temp_analyzer.infer_type(&values[0]);
                                } else {
                                    return Ok(TypeNode::Void);
                                }
                            }
                            AstNode::LetDecl {
                                mutable,
                                type_annotation: _,
                                pattern,
                                value,
                                is_ref_counted: _,
                            } => {
                                // Infer type and add to symbol table
                                if let Ok(ty) = temp_analyzer.infer_type(value) {
                                    if let Pattern::Identifier(name) = pattern {
                                        temp_analyzer.symbol_table.insert(
                                            name.clone(),
                                            SymbolInfo {
                                                ty,
                                                mutable: *mutable,
                                                is_ref_counted: false,
                                                is_parameter: false,
                                            },
                                        );
                                    }
                                }
                            }
                            _ => {
                                // Other statements, just infer type
                                let _ = temp_analyzer.infer_type(stmt);
                            }
                        }
                    }
                    Ok(TypeNode::Void)
                } else {
                    // Not a block, infer type directly
                    temp_analyzer.infer_type(body)
                }
            }
            _ => Err(SemanticError::UnexpectedNode {
                expected: "Expected a closure/lambda function".to_string(),
            }),
        }
    }

    /// Helper function to infer the return type of a reduce lambda
    /// For reduce, the lambda takes (accumulator, element) and returns a value
    /// We only validate that it's a closure and return its inferred type
    fn infer_lambda_return_type_for_reduce(
        &self,
        closure: &AstNode,
        elem_type: &TypeNode,
    ) -> Result<TypeNode, SemanticError> {
        match closure {
            AstNode::Closure {
                params,
                body,
                return_type,
                error_type: _, // Lambda error type not checked here
            } => {
                // If explicit return type is provided, use it
                if let Some(ret_type) = return_type {
                    return Ok(ret_type.clone());
                }

                // Create a temporary analyzer with the lambda's scope
                let mut temp_analyzer = self.clone_for_lambda_analysis();

                // Add parameters to the temporary analyzer's scope
                // For reduce, first param is accumulator, second is element
                for (i, (param_name, param_type)) in params.iter().enumerate() {
                    let inferred_type = if let Some(ty) = param_type {
                        ty.clone()
                    } else if i == 0 {
                        // First parameter (accumulator) - we'll use Int as default
                        TypeNode::Int
                    } else {
                        // Second parameter (element) - infer from array element type
                        elem_type.clone()
                    };
                    temp_analyzer.symbol_table.insert(
                        param_name.clone(),
                        SymbolInfo {
                            ty: inferred_type,
                            mutable: false,
                            is_ref_counted: false,
                            is_parameter: true,
                        },
                    );
                }

                // If body is a block, analyze statements sequentially to build symbol table
                if let AstNode::Block(statements) = body.as_ref() {
                    for stmt in statements {
                        match stmt {
                            AstNode::Return { values } => {
                                if !values.is_empty() {
                                    return temp_analyzer.infer_type(&values[0]);
                                } else {
                                    return Ok(TypeNode::Void);
                                }
                            }
                            AstNode::LetDecl {
                                mutable,
                                type_annotation: _,
                                pattern,
                                value,
                                is_ref_counted: _,
                            } => {
                                // Infer type and add to symbol table
                                if let Ok(ty) = temp_analyzer.infer_type(value) {
                                    if let Pattern::Identifier(name) = pattern {
                                        temp_analyzer.symbol_table.insert(
                                            name.clone(),
                                            SymbolInfo {
                                                ty,
                                                mutable: *mutable,
                                                is_ref_counted: false,
                                                is_parameter: false,
                                            },
                                        );
                                    }
                                }
                            }
                            _ => {
                                // Other statements, just infer type
                                let _ = temp_analyzer.infer_type(stmt);
                            }
                        }
                    }
                    Ok(TypeNode::Void)
                } else {
                    // Not a block, infer type directly
                    temp_analyzer.infer_type(body)
                }
            }
            _ => Err(SemanticError::UnexpectedNode {
                expected: "Expected a closure/lambda function".to_string(),
            }),
        }
    }

    /// Create a temporary analyzer for lambda analysis with shared state
    fn clone_for_lambda_analysis(&self) -> SemanticAnalyzer {
        SemanticAnalyzer {
            symbol_table: self.symbol_table.clone(),
            function_table: self.function_table.clone(),
            struct_table: self.struct_table.clone(),
            enum_table: self.enum_table.clone(),
            enum_variant_order: self.enum_variant_order.clone(),
            method_table: self.method_table.clone(),
            struct_field_visibility: self.struct_field_visibility.clone(),
            imported_struct_names: self.imported_struct_names.clone(),
            struct_field_decorators: self.struct_field_decorators.clone(),
            ffi_metadata: self.ffi_metadata.clone(),
            outer_symbol_table: self.outer_symbol_table.clone(),
            project_root: self.project_root.clone(),
            imported_modules: self.imported_modules.clone(),
            imported_functions: self.imported_functions.clone(),
            imported_structs: self.imported_structs.clone(),
            function_aliases: self.function_aliases.clone(),
            loop_depth: self.loop_depth,
            scope_stack: self.scope_stack.clone(),
            function_depth: self.function_depth,
            scope_sizes_stack: self.scope_sizes_stack.clone(),
            collected_errors: Vec::new(),
            is_main_module: self.is_main_module,
            type_inference_depth: RefCell::new(*self.type_inference_depth.borrow()),
            current_function_error_type: self.current_function_error_type.clone(),
        }
    }

    pub fn infer_method_return_type(
        &self,
        object_type: &TypeNode,
        method: &str,
        args: &[AstNode],
    ) -> Result<TypeNode, SemanticError> {
        // Special handling for Database methods - override stdlib return types
        // Check this BEFORE method_table lookup to allow polymorphic JSON deserialization
        match object_type {
            TypeNode::TypeRef(name) | TypeNode::Struct(name, _) if name == "Database" => {
                match method {
                    "raw" => {
                        if args.len() != 1 {
                            return Err(SemanticError::FunctionArgumentMismatch {
                                name: "Database.raw".to_string(),
                                expected: 1,
                                found: args.len(),
                            });
                        }
                        return Ok(TypeNode::Any);
                    }
                    "rawWithParams" => {
                        if args.len() != 2 {
                            return Err(SemanticError::FunctionArgumentMismatch {
                                name: "Database.rawWithParams".to_string(),
                                expected: 2,
                                found: args.len(),
                            });
                        }
                        return Ok(TypeNode::Any);
                    }
                    _ => {
                        // Fall through to normal method lookup for other Database methods
                    }
                }
            }
            _ => {}
        }

        // Now check if this is a custom user-defined method
        let type_name = match object_type {
            TypeNode::Int => "Int",
            TypeNode::Float => "Float",
            TypeNode::String => "Str",
            TypeNode::Bool => "Bool",
            TypeNode::Array(inner) => {
                // For arrays like [Int], try to match exact type or generic Array
                let full_type = format!("Array({})", inner.format_type_string());
                if let Some(methods) = self.method_table.get(&full_type) {
                    if let Some((param_types, return_type, _)) = methods.get(method) {
                        // Check argument count (allow omitting trailing Optional params)
                        if args.len() > param_types.len() {
                            return Err(SemanticError::FunctionArgumentMismatch {
                                name: format!("{}.{}", full_type, method),
                                expected: param_types.len(),
                                found: args.len(),
                            });
                        }
                        if args.len() < param_types.len() {
                            let missing = &param_types[args.len()..];
                            let ok = missing.iter().all(|t| matches!(t, TypeNode::Optional(_)));
                            if !ok {
                                return Err(SemanticError::FunctionArgumentMismatch {
                                    name: format!("{}.{}", full_type, method),
                                    expected: param_types.len(),
                                    found: args.len(),
                                });
                            }
                        }
                        return Ok(return_type.clone());
                    }
                }
                // Fall through to check built-in methods
                "Array"
            }
            TypeNode::Map(key, val) => {
                // For maps like {Str: Int}, try to match exact type or generic Map
                let full_type = format!(
                    "Map({},{})",
                    key.format_type_string(),
                    val.format_type_string()
                );
                if let Some(methods) = self.method_table.get(&full_type) {
                    if let Some((param_types, return_type, _)) = methods.get(method) {
                        // Check argument count (allow omitting trailing Optional params)
                        if args.len() > param_types.len() {
                            return Err(SemanticError::FunctionArgumentMismatch {
                                name: format!("{}.{}", full_type, method),
                                expected: param_types.len(),
                                found: args.len(),
                            });
                        }
                        if args.len() < param_types.len() {
                            let missing = &param_types[args.len()..];
                            let ok = missing.iter().all(|t| matches!(t, TypeNode::Optional(_)));
                            if !ok {
                                return Err(SemanticError::FunctionArgumentMismatch {
                                    name: format!("{}.{}", full_type, method),
                                    expected: param_types.len(),
                                    found: args.len(),
                                });
                            }
                        }
                        return Ok(return_type.clone());
                    }
                }
                // Fall through to check built-in methods
                "Map"
            }
            TypeNode::TypeRef(name) => name.as_str(),
            TypeNode::Struct(name, _) => name.as_str(),
            _ => "",
        };

        // Check method_table for custom methods on this type
        if !type_name.is_empty() {
            if let Some(methods) = self.method_table.get(type_name) {
                if let Some((param_types, return_type, _)) = methods.get(method) {
                    // Check argument count (allow omitting trailing Optional params)
                    if args.len() > param_types.len() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("{}.{}", type_name, method),
                            expected: param_types.len(),
                            found: args.len(),
                        });
                    }
                    if args.len() < param_types.len() {
                        let missing = &param_types[args.len()..];
                        let ok = missing.iter().all(|t| matches!(t, TypeNode::Optional(_)));
                        if !ok {
                            return Err(SemanticError::FunctionArgumentMismatch {
                                name: format!("{}.{}", type_name, method),
                                expected: param_types.len(),
                                found: args.len(),
                            });
                        }
                    }
                    return Ok(return_type.clone());
                }
            }
        }

        // Fall back to built-in methods
        match object_type {
            TypeNode::String => match method {
                "len" => {
                    if !args.is_empty() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("String.{}", method),
                            expected: 0,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Int)
                }
                "charAt" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("String.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::String)
                }
                "substring" => {
                    if args.len() != 2 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("String.{}", method),
                            expected: 2,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::String)
                }
                "concat" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("String.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::String)
                }
                "toUpper" | "toLower" => {
                    if !args.is_empty() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("String.{}", method),
                            expected: 0,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::String)
                }
                "indexOf" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("String.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Int)
                }
                "contains" | "startsWith" | "endsWith" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("String.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Bool)
                }
                "trim" | "reverse" => {
                    if !args.is_empty() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("String.{}", method),
                            expected: 0,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::String)
                }
                "split" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("String.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Array(Box::new(TypeNode::String)))
                }
                "replace" => {
                    if args.len() != 2 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("String.{}", method),
                            expected: 2,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::String)
                }
                "charCode" => {
                    if !args.is_empty() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("String.{}", method),
                            expected: 0,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Int)
                }
                "repeat" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("String.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::String)
                }
                "countSubstr" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("String.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Int)
                }
                _ => Err(SemanticError::MethodNotFoundOnType {
                    object_type: "string".to_string(),
                    method_name: method.to_string(),
                    correct_type: None,
                }),
            },
            TypeNode::Array(elem_type) => match method {
                "len" => {
                    if !args.is_empty() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Array.{}", method),
                            expected: 0,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Int)
                }
                "push" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Array.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Void)
                }
                "pop" => {
                    if !args.is_empty() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Array.{}", method),
                            expected: 0,
                            found: args.len(),
                        });
                    }
                    Ok(*elem_type.clone())
                }
                "contains" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Array.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Bool)
                }
                "reverse" => {
                    if !args.is_empty() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Array.{}", method),
                            expected: 0,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Array(elem_type.clone()))
                }
                "first" | "last" => {
                    if !args.is_empty() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Array.{}", method),
                            expected: 0,
                            found: args.len(),
                        });
                    }
                    Ok(*elem_type.clone())
                }
                "isEmpty" => {
                    if !args.is_empty() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Array.{}", method),
                            expected: 0,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Bool)
                }
                "clear" => {
                    if !args.is_empty() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Array.{}", method),
                            expected: 0,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Void)
                }
                "sort" => {
                    if !args.is_empty() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Array.{}", method),
                            expected: 0,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Void)
                }
                "slice" => {
                    if args.len() != 2 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Array.{}", method),
                            expected: 2,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Array(elem_type.clone()))
                }
                "indexOf" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Array.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Int)
                }
                "filter" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Array.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    // Validate that the lambda returns Bool
                    let lambda_return_type = self.infer_lambda_return_type(&args[0], elem_type)?;
                    if lambda_return_type != TypeNode::Bool {
                        let (line, col) = get_node_location(&args[0]);
                        return Err(SemanticError::OperatorTypeMismatch(TypeMismatch {
                            expected: TypeNode::Bool,
                            found: lambda_return_type,
                            value: None,
                            line,
                            col,
                        }));
                    }
                    Ok(TypeNode::Array(elem_type.clone()))
                }
                "map" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Array.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    // Infer the return type of the lambda and validate it matches the element type
                    let lambda_return_type = self.infer_lambda_return_type(&args[0], elem_type)?;
                    if lambda_return_type != **elem_type {
                        let (line, col) = get_node_location(&args[0]);
                        return Err(SemanticError::OperatorTypeMismatch(TypeMismatch {
                            expected: (**elem_type).clone(),
                            found: lambda_return_type,
                            value: None,
                            line,
                            col,
                        }));
                    }
                    Ok(TypeNode::Array(Box::new(lambda_return_type)))
                }
                "reduce" => {
                    if args.len() != 2 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Array.{}", method),
                            expected: 2,
                            found: args.len(),
                        });
                    }
                    // Infer the return type of the lambda (second argument)
                    // For reduce, the lambda takes (accumulator, element) and returns a value
                    // We don't validate the accumulator type here, just validate the lambda
                    let lambda_return_type =
                        self.infer_lambda_return_type_for_reduce(&args[1], elem_type)?;
                    Ok(lambda_return_type)
                }
                "join" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Array.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::String)
                }
                _ => Err(SemanticError::MethodNotFoundOnType {
                    object_type: "array".to_string(),
                    method_name: method.to_string(),
                    correct_type: if method == "has"
                        || method == "containsKey"
                        || method == "containsValue"
                        || method == "keys"
                        || method == "values"
                        || method == "clear"
                    {
                        Some("map".to_string())
                    } else {
                        None
                    },
                }),
            },
            TypeNode::Map(_key_type, value_type) => match method {
                "get" => {
                    // map.get() is removed - use map[key] syntax instead
                    return Err(SemanticError::InvalidMethodCall {
                        method: "get".to_string(),
                        type_name: "Map".to_string(),
                        message: "map.get() is removed. Use map[key] syntax instead.".to_string(),
                    });
                }
                "set" => {
                    // map.set() is removed - use map[key] = value syntax instead
                    return Err(SemanticError::InvalidMethodCall {
                        method: "set".to_string(),
                        type_name: "Map".to_string(),
                        message: "map.set() is removed. Use map[key] = value syntax instead."
                            .to_string(),
                    });
                }
                "has" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Map.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Bool)
                }
                "remove" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Map.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Bool)
                }
                "isEmpty" => {
                    if !args.is_empty() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Map.{}", method),
                            expected: 0,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Bool)
                }
                "size" | "len" => {
                    if !args.is_empty() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Map.{}", method),
                            expected: 0,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Int)
                }
                "clear" => {
                    if !args.is_empty() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Map.{}", method),
                            expected: 0,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Bool)
                }
                "keys" => {
                    if !args.is_empty() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Map.{}", method),
                            expected: 0,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Array(Box::new(TypeNode::String)))
                }
                "values" => {
                    if !args.is_empty() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Map.{}", method),
                            expected: 0,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Array(value_type.clone()))
                }
                "containsKey" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Map.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Bool)
                }
                "containsValue" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Map.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Bool)
                }
                _ => Err(SemanticError::MethodNotFoundOnType {
                    object_type: "map".to_string(),
                    method_name: method.to_string(),
                    correct_type: if method == "push"
                        || method == "pop"
                        || method == "contains"
                        || method == "reverse"
                        || method == "first"
                        || method == "last"
                        || method == "isEmpty"
                        || method == "clear"
                        || method == "sort"
                    {
                        Some("array".to_string())
                    } else {
                        None
                    },
                }),
            },
            TypeNode::Int => match method {
                "toChar" => {
                    if !args.is_empty() {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Int.{}", method),
                            expected: 0,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::String)
                }
                _ => Err(SemanticError::MethodNotFoundOnType {
                    object_type: "int".to_string(),
                    method_name: method.to_string(),
                    correct_type: if method == "toUpper"
                        || method == "toLower"
                        || method == "charAt"
                        || method == "substring"
                        || method == "contains"
                        || method == "indexOf"
                        || method == "startsWith"
                        || method == "endsWith"
                        || method == "split"
                        || method == "replace"
                        || method == "repeat"
                        || method == "countSubstr"
                    {
                        Some("string".to_string())
                    } else {
                        None
                    },
                }),
            },
            TypeNode::Float => match method {
                _ => Err(SemanticError::MethodNotFoundOnType {
                    object_type: "float".to_string(),
                    method_name: method.to_string(),
                    correct_type: None,
                }),
            },

            TypeNode::Builtin(name) if name == "JSON" => match method {
                "parse" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("JSON.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    // JSON.parse returns Any type - compatible with any expected type
                    // The actual type is determined at runtime based on the JSON content
                    Ok(TypeNode::Any)
                }
                "stringify" => {
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("JSON.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::String)
                }
                _ => Err(SemanticError::MethodNotFoundOnType {
                    object_type: "JSON".to_string(),
                    method_name: method.to_string(),
                    correct_type: None,
                }),
            },
            _ => Err(SemanticError::MethodNotFoundOnType {
                object_type: format!("{:?}", object_type).to_lowercase(),
                method_name: method.to_string(),
                correct_type: None,
            }),
        }
    }
}
