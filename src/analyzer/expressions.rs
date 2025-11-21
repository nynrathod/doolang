use super::analyzer::{SemanticAnalyzer, SymbolInfo};
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
            AstNode::StringLiteral(s) => {
                // Reject string interpolation syntax ${...}
                if s.contains("${") {
                    return Err(SemanticError::UndeclaredFunction(NamedError {
                        name: "String interpolation with ${...} is not supported".to_string(),
                    }));
                }
                Ok(TypeNode::String)
            }
            // Boolean literal: always Bool type
            AstNode::BoolLiteral(_name) => Ok(TypeNode::Bool),
            // Nil literal: polymorphic null value - compatible with any pointer/optional type
            AstNode::NilLiteral => Ok(TypeNode::Nil),
            // Identifier (variable name): look up in symbol table (with shadowing support)
            AstNode::Identifier(name) => {
                if let Some(info) = self.lookup_variable(name) {
                    Ok(info.ty.clone())
                } else if let Some(outer) = &self.outer_symbol_table {
                    if outer.contains_key(name) {
                        return Err(SemanticError::OutOfScopeVariable(NamedError {
                            name: name.clone(),
                        }));
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
            AstNode::FunctionCall { func, args: _ } => {
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

                // Infer type from first element
                // This check type of element insides
                let first_type = self.infer_type(&elements[0])?;
                // Check all elements for type consistency
                for el in elements.iter() {
                    let t = self.infer_type(el)?;
                    if t != first_type {
                        let (line, col) = get_node_location(el);
                        return Err(SemanticError::VarTypeMismatch(TypeMismatch {
                            expected: first_type.clone(),
                            found: t,
                            value: None,
                            line,
                            col,
                        }));
                    }
                }
                // All elements are the same type: return Array of that type
                Ok(TypeNode::Array(Box::new(first_type)))
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

                // Infer key and value types from first pair
                let key_type = self.infer_type(&pairs[0].0)?;
                let value_type = self.infer_type(&pairs[0].1)?;

                // Only allow Int, String, Float, or Bool as map keys
                // TODO: check codegen if implemented or not
                match key_type {
                    TypeNode::Int | TypeNode::String | TypeNode::Float | TypeNode::Bool => {}
                    _ => {
                        return Err(SemanticError::InvalidMapKeyType {
                            found: key_type.clone(),
                            expected: TypeNode::Map(
                                Box::new(TypeNode::Int),
                                Box::new(TypeNode::String),
                            ),
                        });
                    }
                }

                // Check all pairs for type consistency
                for (k, v) in pairs.iter() {
                    let kt = self.infer_type(k)?;
                    let vt = self.infer_type(v)?;
                    if kt != key_type {
                        let (line, col) = get_node_location(k);
                        return Err(SemanticError::VarTypeMismatch(TypeMismatch {
                            expected: key_type.clone(),
                            found: kt,
                            value: None,
                            line,
                            col,
                        }));
                    }
                    if vt != value_type {
                        let (line, col) = get_node_location(v);
                        return Err(SemanticError::VarTypeMismatch(TypeMismatch {
                            expected: value_type.clone(),
                            found: vt,
                            value: None,
                            line,
                            col,
                        }));
                    }
                }

                // All keys and values are consistent: return Map type
                Ok(TypeNode::Map(Box::new(key_type), Box::new(value_type)))
            }

            // Element access: arr[index] or map[key]
            // Infer type of the array/map and the index/key
            AstNode::ElementAccess { array, index } => {
                let array_type = self.infer_type(array)?;
                let index_type = self.infer_type(index)?;

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
                    // Verify all required fields are provided
                    for (field_name, field_type) in struct_fields {
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

                    // Verify field types match
                    for (field_name, field_value) in fields {
                        if let Some(expected_type) = struct_fields.get(field_name) {
                            let actual_type = self.infer_type(field_value)?;
                            // For now, basic type checking
                            // TODO: Handle Optional types and type compatibility
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

            // Enum variant: Direction::North or Status::Active(value)
            AstNode::EnumVariant {
                enum_name,
                variant,
                payload,
            } => {
                // Check if enum type exists
                if let Some(enum_variants) = self.enum_table.get(enum_name) {
                    // Check if variant exists
                    if let Some(variant_type) = enum_variants.get(variant) {
                        // Verify payload matches
                        match (payload, variant_type) {
                            (Some(payload_expr), Some(expected_type)) => {
                                let actual_type = self.infer_type(payload_expr)?;
                                // TODO: Type compatibility check
                                Ok(TypeNode::Enum(enum_name.clone(), enum_variants.clone()))
                            }
                            (None, None) => {
                                Ok(TypeNode::Enum(enum_name.clone(), enum_variants.clone()))
                            }
                            (Some(_), None) => Err(SemanticError::UndeclaredVariable(NamedError {
                                name: format!(
                                    "Variant '{}::{}' does not take a payload",
                                    enum_name, variant
                                ),
                            })),
                            (None, Some(_)) => Err(SemanticError::UndeclaredVariable(NamedError {
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
                    Err(SemanticError::UndeclaredVariable(NamedError {
                        name: format!("Undefined enum type '{}'", enum_name),
                    }))
                }
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
            outer_symbol_table: self.outer_symbol_table.clone(),
            project_root: self.project_root.clone(),
            imported_modules: self.imported_modules.clone(),
            imported_functions: self.imported_functions.clone(),
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
                    if args.len() != 1 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Map.{}", method),
                            expected: 1,
                            found: args.len(),
                        });
                    }
                    Ok(*value_type.clone())
                }
                "set" => {
                    if args.len() != 2 {
                        return Err(SemanticError::FunctionArgumentMismatch {
                            name: format!("Map.{}", method),
                            expected: 2,
                            found: args.len(),
                        });
                    }
                    Ok(TypeNode::Void)
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
            _ => Err(SemanticError::MethodNotFoundOnType {
                object_type: format!("{:?}", object_type).to_lowercase(),
                method_name: method.to_string(),
                correct_type: None,
            }),
        }
    }
}
