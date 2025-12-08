use super::analyzer::{types_compatible, SemanticAnalyzer};
use std::collections::HashMap;

use super::types::{NamedError, SemanticError, TypeMismatch};
use crate::analyzer::analyzer::SymbolInfo;
use crate::parser::ast::{AstNode, TypeNode};

impl SemanticAnalyzer {
    /// Analyze a variable declaration (`let` statement).
    ///
    /// This function performs semantic analysis for variable declarations. It:
    /// - Checks if a type annotation is present and ensures the assigned value matches it.
    /// - If no annotation, infers the type from the assigned value.
    /// - Updates the AST node with the inferred type and reference counting info.
    /// - Validates the assignment pattern (identifiers, wildcards, tuples).
    /// - Ensures the number of patterns matches the number of values (for tuples).
    /// - Adds variables to the symbol table, marking mutability and reference counting.
    /// - Returns semantic errors for type mismatches, redeclarations, or invalid patterns.
    pub fn analyze_let_decl(&mut self, node: &mut AstNode) -> Result<(), SemanticError> {
        match node {
            AstNode::LetDecl {
                mutable,
                type_annotation,
                pattern,
                value,
                is_ref_counted,
            } => {
                // Check if immutable variable is initialized with empty collection
                // Only error if there's no type annotation (type annotation makes it valid)
                if !*mutable {
                    match &**value {
                        AstNode::ArrayLiteral(elements) if elements.is_empty() => {
                            return Err(SemanticError::ImmutableEmptyCollection {
                                found: TypeNode::Array(Box::new(TypeNode::Int)),
                            });
                        }
                        AstNode::MapLiteral(pairs) if pairs.is_empty() => {
                            return Err(SemanticError::ImmutableEmptyCollection {
                                found: TypeNode::Map(
                                    Box::new(TypeNode::String),
                                    Box::new(TypeNode::Int),
                                ),
                            });
                        }
                        _ => {}
                    }
                }

                // First, collect patterns to know how many values we expect
                let patterns = self.collect_and_validate_targets(pattern)?;
                let expected_count = patterns.len();

                // For empty maps and arrays, use the type annotation if available
                let rhs_type = if let Some(annotated_type) = type_annotation.as_ref() {
                    // If we have a type annotation and value is empty map/array, use annotation directly
                    match (&**value, annotated_type) {
                        (AstNode::MapLiteral(pairs), _) if pairs.is_empty() => {
                            annotated_type.clone()
                        }
                        (AstNode::ArrayLiteral(elements), _) if elements.is_empty() => {
                            annotated_type.clone()
                        }
                        _ => {
                            // Otherwise, infer normally - request correct number of types
                            let rhs_types_vec = self.infer_rhs_types(value, expected_count)?;

                            // If we got multiple types back, wrap in Tuple
                            let inferred = if rhs_types_vec.len() > 1 {
                                TypeNode::Tuple(rhs_types_vec.clone())
                            } else {
                                rhs_types_vec.get(0).cloned().ok_or_else(|| {
                                    SemanticError::VarTypeMismatch(TypeMismatch {
                                        expected: annotated_type.clone(),
                                        found: TypeNode::Void,
                                        value: Some(value.clone()),
                                        line: None,
                                        col: None,
                                    })
                                })?
                            };

                            // If inferred type is Any (e.g., from JSON.parse), use the type annotation
                            // This allows explicit type annotations to override dynamic types
                            if matches!(inferred, TypeNode::Any) {
                                annotated_type.clone()
                            } else {
                                // Verify inferred type matches annotation
                                if !types_compatible(
                                    &inferred,
                                    annotated_type,
                                    &self.struct_table,
                                    &self.enum_table,
                                ) {
                                    return Err(SemanticError::VarTypeMismatch(TypeMismatch {
                                        expected: annotated_type.clone(),
                                        found: inferred,
                                        value: Some(value.clone()),
                                        line: None,
                                        col: None,
                                    }));
                                }
                                inferred
                            }
                        }
                    }
                } else {
                    // Use infer_rhs_types to ensure function call argument checks are performed
                    // Request the correct number of types based on pattern count
                    let rhs_types_vec = self.infer_rhs_types(value, expected_count)?;

                    // If we got multiple types back, wrap in Tuple
                    if rhs_types_vec.len() > 1 {
                        TypeNode::Tuple(rhs_types_vec)
                    } else {
                        rhs_types_vec.get(0).cloned().ok_or_else(|| {
                            SemanticError::VarTypeMismatch(TypeMismatch {
                                expected: TypeNode::Int,
                                found: TypeNode::Void,
                                value: Some(value.clone()),
                                line: None,
                                col: None,
                            })
                        })?
                    }
                };

                // Update the type annotation to reflect the inferred type if it was missing.
                *type_annotation = Some(rhs_type.clone());

                // println!("Before: {:?}", is_ref_counted);

                // Update AST with reference counting info based on the type.
                *is_ref_counted = Some(Self::should_be_rc(&rhs_type));
                // println!("After: {:?}", is_ref_counted);

                // Validate and collect assignment targets from the pattern.
                // Note: patterns was already collected above
                let targets = patterns;

                // If RHS is a tuple (but not a Result), each element must match a pattern.
                // Result types CAN be unpacked if the inner type is a tuple and user destructures
                // Otherwise, treat RHS as a single-element list.
                let rhs_types = match &rhs_type {
                    TypeNode::Tuple(types) => types.clone(),
                    TypeNode::Result(ok_type, error_type) => {
                        // Check if this is manual error extraction with comma syntax
                        // Pattern: let result, err = Func() where Func returns Result(ok_type, error_type)
                        // In this case, targets.len() should be ok_count + 1 (for the error variable)

                        let ok_count = match &**ok_type {
                            TypeNode::Tuple(inner_types) => inner_types.len(),
                            TypeNode::Void => 0, // Void has no ok values
                            _ => 1,              // Single ok value
                        };

                        // If targets.len() == ok_count + 1, this is manual error extraction
                        if targets.len() == ok_count + 1 {
                            // Manual error extraction: last target is error variable
                            // Return ok types + error type
                            let mut types = match &**ok_type {
                                TypeNode::Tuple(inner_types) => inner_types.clone(),
                                TypeNode::Void => vec![],
                                _ => vec![(**ok_type).clone()],
                            };
                            types.push((**error_type).clone());
                            types
                        } else {
                            // Not manual error extraction - this is an error!
                            // Result types MUST be handled with either ? operator or manual extraction
                            return Err(SemanticError::UnhandledResult {
                                ok_type: (**ok_type).clone(),
                                error_type: (**error_type).clone(),
                            });
                        }
                    }
                    t => vec![t.clone()],
                };
                // Check that the number of LHS patterns matches the number of RHS types.
                if rhs_types.len() != targets.len() {
                    return Err(SemanticError::TupleAssignmentMismatch {
                        expected: rhs_types.len(),
                        found: targets.len(),
                    });
                }

                // Bind each pattern to its type in the symbol table.
                for (target, ty) in targets.iter().zip(rhs_types.iter()) {
                    match target {
                        // Identifier: add to symbol table, mark mutability.
                        crate::parser::ast::Pattern::Identifier(name) => {
                            // Disallow variable names starting with underscore
                            if name.starts_with('_') {
                                return Err(SemanticError::InvalidAssignmentTarget {
                                    target: format!("Variable names starting with underscore are not allowed: '{}'", name),
                                    // No line/col available here
                                });
                            }
                            // Skip wildcards (do not store them).
                            if name != "_" {
                                // Check for redeclaration
                                // If not in a nested scope, don't allow redeclaration
                                // If in a nested scope, allow shadowing but not redeclaration in same scope
                                if self.scope_stack.is_empty() {
                                    // Top-level scope - no redeclaration allowed
                                    // Exception: allow shadowing of parameters
                                    if let Some(existing) = self.symbol_table.get(name) {
                                        if !existing.is_parameter {
                                            return Err(SemanticError::VariableRedeclaration(
                                                NamedError { name: name.clone() },
                                            ));
                                        }
                                    }
                                }
                                // If in nested scope, allow shadowing - don't check at all for now
                                // Just add the variable

                                // Add to symbol_table
                                self.symbol_table.insert(
                                    name.clone(),
                                    SymbolInfo {
                                        ty: ty.clone(),
                                        mutable: *mutable,
                                        is_ref_counted: Self::should_be_rc(&ty),
                                        is_parameter: false,
                                    },
                                );
                            }
                        }
                        // Wildcard: allowed but not stored.
                        crate::parser::ast::Pattern::Wildcard => {}
                        // Anything else: invalid pattern.
                        _ => {
                            return Err(SemanticError::InvalidAssignmentTarget {
                                target: format!("{:?}", target),
                            });
                        }
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Analyze a function declaration.
    ///
    /// This function performs semantic analysis for function declarations. It:
    /// - Checks if the function is already defined (prevents redeclaration).
    /// - Validates parameter types and ensures no duplicate parameter names.
    /// - Handles public/private visibility rules (public functions must start with uppercase).
    /// - Adds the function signature to the function table.
    /// - Creates a local scope for parameters and analyzes the function body in isolation.
    /// - If no return type is specified, marks as `Void` and ensures no return values are present.
    /// - Appends an implicit empty return if needed.
    /// - Checks for required return statements and verifies their types.
    /// - Restores the outer symbol table after analysis.
    /// - Returns semantic errors for any violations.
    pub fn analyze_functional_decl(
        &mut self,
        name: &str,
        visibility: &str,
        params: &mut Vec<(String, Option<TypeNode>)>,
        return_type: &mut Option<TypeNode>,
        error_type: &mut Option<TypeNode>,
        body: &mut Vec<AstNode>,
        decorators: &Vec<crate::parser::ast::Decorator>,
        receiver_type: &Option<String>,
        is_expression: &bool,
    ) -> Result<(), SemanticError> {
        // Function signature is already registered in analyze_program's first pass
        // No need to check for redeclaration or add to function_table here

        // Check if this is an FFI function (has @ffi decorator)
        let is_ffi = decorators.iter().any(|d| d.name == "ffi");

        // FFI functions should have empty bodies
        if is_ffi && !body.is_empty() {
            return Err(SemanticError::ParseErrorMsg(format!(
                "FFI function '{}' should have an empty body",
                name
            )));
        }

        // Is public or private function
        // Enforce public function naming convention.
        if visibility == "Public" {
            if let Some(first_char) = name.chars().next() {
                if !first_char.is_uppercase() {
                    return Err(SemanticError::InvalidPublicName(NamedError {
                        name: name.to_string(),
                    }));
                }
            }
        }

        // Create a local scope for function parameters.
        let mut local_scope: HashMap<String, SymbolInfo> = HashMap::new();

        // If this is a method declaration, add the receiver parameter (first param)
        if let Some(receiver_type_name) = receiver_type {
            // Get the first parameter name (the receiver)
            if let Some((receiver_param_name, _)) = params.first() {
                // Convert receiver type name to TypeNode
                let receiver_type_node = match receiver_type_name.as_str() {
                    "Int" => TypeNode::Int,
                    "Float" => TypeNode::Float,
                    "Str" => TypeNode::String,
                    "Bool" => TypeNode::Bool,
                    other => TypeNode::TypeRef(other.to_string()),
                };

                // Add receiver parameter to local scope
                local_scope.insert(
                    receiver_param_name.clone(),
                    SymbolInfo {
                        ty: receiver_type_node,
                        mutable: true,
                        is_ref_counted: false,
                        is_parameter: true,
                    },
                );
            }
        }

        // Process remaining parameters (skip first one if it's a method)
        let params_to_process = if receiver_type.is_some() {
            params.iter().skip(1)
        } else {
            params.iter().skip(0)
        };

        for (param_name, param_type) in params_to_process {
            // Type is mandatory for parameters. Check type exists.
            let param_type = param_type.as_ref().ok_or_else(|| {
                SemanticError::MissingParamType(NamedError {
                    name: param_name.clone(),
                })
            })?;

            // Check for duplicate parameter names.
            if local_scope.contains_key(param_name) {
                return Err(SemanticError::FunctionParamRedeclaration(NamedError {
                    name: param_name.clone(),
                }));
            }

            // Insert parameter into local scope (parameters are always immutable).
            local_scope.insert(
                param_name.clone(),
                SymbolInfo {
                    ty: param_type.clone(),
                    mutable: true,
                    is_ref_counted: Self::should_be_rc(&param_type),
                    is_parameter: true,
                },
            );
        }

        // If no return type, mark as Void and ensure no return values are present.
        if return_type.is_none() {
            *return_type = Some(TypeNode::Void);

            // Skip body validation for FFI functions
            if !is_ffi {
                // For functions with no return type and no error type: no Ok, Err, or return with values
                if error_type.is_none() {
                    for node in body.iter() {
                        match node {
                            AstNode::Return { values } => {
                                if !values.is_empty() {
                                    return Err(SemanticError::InvalidReturnInVoidFunction {
                                        function: name.to_string(),
                                    });
                                }
                            }
                            AstNode::OkExpr { .. } => {
                                return Err(SemanticError::UnexpectedNode {
                                    expected: format!(
                                        "Ok cannot be used in function '{}' without a return type or error type (! ErrorType)",
                                        name
                                    ),
                                });
                            }
                            AstNode::ErrExpr { .. } => {
                                return Err(SemanticError::UnexpectedNode {
                                    expected: format!(
                                        "Err cannot be used in function '{}' without error type (! ErrorType)",
                                        name
                                    ),
                                });
                            }
                            _ => {}
                        }
                    }
                } else {
                    // Function has error type but no success return type (error-only function)
                    // Don't allow Ok with values, but allow empty Ok for error-only functions
                    for node in body.iter() {
                        match node {
                            AstNode::Return { values } => {
                                if !values.is_empty() {
                                    return Err(SemanticError::InvalidReturnInVoidFunction {
                                        function: name.to_string(),
                                    });
                                }
                            }
                            AstNode::OkExpr { values } => {
                                if !values.is_empty() {
                                    return Err(SemanticError::UnexpectedNode {
                                        expected: format!(
                                            "Ok with values cannot be used in error-only function '{}' (no return type specified). Use empty Ok or Err.",
                                            name
                                        ),
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // Append implicit empty return if last statement is not Return/Ok/Err.
                if let Some(last) = body.last() {
                    if !matches!(
                        last,
                        AstNode::Return { .. } | AstNode::OkExpr { .. } | AstNode::ErrExpr { .. }
                    ) {
                        // If function has error type, use OkExpr instead of Return
                        // so that it returns a Result struct even with void Ok value
                        if error_type.is_some() {
                            body.push(AstNode::OkExpr { values: vec![] });
                        } else {
                            body.push(AstNode::Return { values: vec![] });
                        }
                    }
                }
            }
        }

        // FFI functions don't need body analysis - they're implemented externally
        if is_ffi {
            return Ok(());
        }

        // Save outer symbol table and switch to local scope for function analysis.
        let outer_symbol_table = Some(self.symbol_table.clone());
        self.outer_symbol_table = outer_symbol_table;
        self.symbol_table = local_scope; // only params visible

        // Check for required return statements (but don't verify types yet - need body analyzed first).
        if let Some(ret_type) = return_type.as_ref() {
            if *ret_type != TypeNode::Void {
                self.ensure_has_return(body, name)?;
            }
        }

        // ENFORCEMENT: If function has return type, it MUST use Ok (not bare Return)
        // BUT: Skip this check for expression functions (which are just Return statements)
        if !*is_expression && return_type.is_some() && return_type.as_ref() != Some(&TypeNode::Void)
        {
            self.ensure_uses_ok_not_return(body, name, error_type.is_some())?;
        }

        // ENFORCEMENT: If function has error type, it MUST have at least one Err path
        // BUT: Skip this check for expression functions (they can't have Err in => syntax)
        if !*is_expression && error_type.is_some() {
            self.ensure_has_error_path(body, name)?;
        }

        self.function_depth += 1;

        // Set current function's error type for ? operator validation
        // Special case: main() can use ? without declaring error type (like Rust)
        let prev_error_type = self.current_function_error_type.clone();
        if name == "main" && error_type.is_none() {
            // Allow main to use error handling by setting a default error type
            self.current_function_error_type = Some(TypeNode::String);
        } else {
            self.current_function_error_type = error_type.clone();
        }

        // Analyze function body with isolated scope.
        self.analyze_program(body)?;

        // Restore previous error type
        self.current_function_error_type = prev_error_type;

        // Now verify return types after body has been analyzed and local variables are in scope.
        if let Some(ret_type) = return_type.as_ref() {
            if *ret_type != TypeNode::Void {
                self.verify_return_types(body, ret_type, name)?;
            }
        }

        // Restore outer scope after function analysis.
        if let Some(outer) = self.outer_symbol_table.take() {
            self.function_depth -= 1;
            self.symbol_table = outer;
        }

        Ok(())
    }

    /// Ensure function has at least one return statement
    /// Ensures that a function body contains at least one return statement.
    ///
    /// Used for functions that declare a non-void return type.
    /// Returns an error if no return statement is found.
    /// Treats Ok/Err expressions as implicit returns.
    fn ensure_has_return(&self, body: &Vec<AstNode>, fn_name: &str) -> Result<(), SemanticError> {
        if !self.has_return_statement(body) {
            // Check if last statement is a match expression (implicit return)
            if let Some(last) = body.last() {
                if matches!(last, AstNode::MatchExpr { .. }) {
                    return Ok(());
                }
            }
            return Err(SemanticError::MissingFunctionReturn {
                function: fn_name.to_string(),
            });
        }
        Ok(())
    }

    /// Checks if any node in a list contains a return statement.
    /// Used to recursively scan function bodies, blocks, and conditional branches
    /// to ensure that a return statement exists where required.
    fn has_return_statement(&self, nodes: &Vec<AstNode>) -> bool {
        for node in nodes {
            match node {
                AstNode::Return { .. } | AstNode::OkExpr { .. } | AstNode::ErrExpr { .. } => {
                    return true
                }
                AstNode::ConditionalStmt {
                    then_block,
                    else_branch,
                    ..
                } => {
                    // Both branches must have a return for the function to be considered as returning.
                    let then_has = self.has_return_statement(then_block);
                    let else_has = else_branch
                        .as_ref()
                        .map(|b| self.has_return_statement(&vec![*b.clone()]))
                        .unwrap_or(false);
                    if then_has && else_has {
                        return true;
                    }
                }
                AstNode::Block(inner_nodes) => {
                    if self.has_return_statement(inner_nodes) {
                        return true;
                    }
                }
                AstNode::MatchExpr { arms, .. } => {
                    // Match is a return if all arms have returns
                    let all_arms_return = arms.iter().all(|arm| match arm.body.as_ref() {
                        AstNode::Block(stmts) => self.has_return_statement(stmts),
                        AstNode::Return { .. }
                        | AstNode::OkExpr { .. }
                        | AstNode::ErrExpr { .. } => true,
                        other => {
                            let v = vec![other.clone()];
                            self.has_return_statement(&v)
                        }
                    });
                    if all_arms_return && !arms.is_empty() {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// Verifies that each return statement in a function matches the expected return type.
    /// Recursively checks all return statements in the function body, including those in
    /// conditional branches and blocks. Returns an error if any return statement has a type mismatch.
    fn verify_return_types(
        &self,
        nodes: &Vec<AstNode>,
        expected: &TypeNode,
        fn_name: &str,
    ) -> Result<(), SemanticError> {
        for node in nodes {
            match node {
                AstNode::Return { values } => {
                    self.verify_single_return(values, expected, fn_name)?;
                }
                AstNode::OkExpr { values } => {
                    self.verify_single_return(values, expected, fn_name)?;
                }
                AstNode::ErrExpr { value } => {
                    // For Err expressions, verify the error type matches (skip for now)
                    // Just ensure it's treated as a valid return
                }
                AstNode::ConditionalStmt {
                    then_block,
                    else_branch,
                    ..
                } => {
                    self.verify_return_types(then_block, expected, fn_name)?;
                    if let Some(else_node) = else_branch {
                        match &**else_node {
                            AstNode::Block(nodes) => {
                                self.verify_return_types(nodes, expected, fn_name)?
                            }
                            _ => self.verify_return_types(
                                &vec![*else_node.clone()],
                                expected,
                                fn_name,
                            )?,
                        }
                    }
                }
                AstNode::Block(inner_nodes) => {
                    self.verify_return_types(inner_nodes, expected, fn_name)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Verifies a single return statement matches the expected type.
    /// Handles both tuple and single-value returns. Returns an error if the number of returned
    /// values or their types do not match the function's declared return type.
    fn verify_single_return(
        &self,
        values: &Vec<AstNode>,
        expected: &TypeNode,
        fn_name: &str,
    ) -> Result<(), SemanticError> {
        // If the return contains OkExpr or ErrExpr, extract the inner values
        let actual_values = if values.len() == 1 {
            match &values[0] {
                AstNode::OkExpr {
                    values: inner_values,
                } => inner_values.clone(),
                AstNode::ErrExpr { .. } => {
                    // ErrExpr is valid, skip type checking for now
                    return Ok(());
                }
                _ => values.clone(),
            }
        } else {
            values.clone()
        };

        match expected {
            TypeNode::Tuple(expected_vec) => {
                // For tuple returns, check length and types of each element.
                if actual_values.len() != expected_vec.len() {
                    return Err(SemanticError::ReturnTypeMismatch {
                        function: fn_name.to_string(),
                        mismatch: TypeMismatch {
                            expected: expected.clone(),
                            found: TypeNode::Tuple(
                                actual_values
                                    .iter()
                                    .map(|v| self.infer_type(v))
                                    .collect::<Result<Vec<_>, _>>()?,
                            ),
                            value: None,
                            line: None,
                            col: None,
                        },
                    });
                }
                for (value, expected_type) in actual_values.iter().zip(expected_vec.iter()) {
                    let value_type = self.infer_type(value)?;
                    if !super::analyzer::types_compatible(
                        &value_type,
                        expected_type,
                        &self.struct_table,
                        &self.enum_table,
                    ) {
                        return Err(SemanticError::ReturnTypeMismatch {
                            function: fn_name.to_string(),
                            mismatch: TypeMismatch {
                                expected: expected_type.clone(),
                                found: value_type,
                                value: None,
                                line: None,
                                col: None,
                            },
                        });
                    }
                }
            }
            _ => {
                // single return
                // For single-value returns, check there is exactly one value and its type matches.
                if actual_values.len() != 1 {
                    return Err(SemanticError::ReturnTypeMismatch {
                        function: fn_name.to_string(),
                        mismatch: TypeMismatch {
                            expected: expected.clone(),
                            found: TypeNode::Tuple(
                                actual_values
                                    .iter()
                                    .map(|v| self.infer_type(v))
                                    .collect::<Result<Vec<_>, _>>()?,
                            ),
                            value: None,
                            line: None,
                            col: None,
                        },
                    });
                }
                let value_type = self.infer_type(&actual_values[0])?;
                if !super::analyzer::types_compatible(
                    &value_type,
                    expected,
                    &self.struct_table,
                    &self.enum_table,
                ) {
                    return Err(SemanticError::ReturnTypeMismatch {
                        function: fn_name.to_string(),
                        mismatch: TypeMismatch {
                            expected: expected.clone(),
                            found: value_type,
                            value: None,
                            line: None,
                            col: None,
                        },
                    });
                }
            }
        }
        Ok(())
    }

    /// This function checks for redeclaration of struct names, validates field names and types,
    /// ensures no duplicate fields, and adds the struct type to the symbol table.
    /// Returns semantic errors for any violations.
    pub fn analyze_struct(&mut self, node: &AstNode) -> Result<(), SemanticError> {
        if let AstNode::StructDecl {
            name,
            fields,
            is_public,
        } = node
        {
            // Prevent redeclaration of struct names.
            if self.symbol_table.contains_key(name) {
                return Err(SemanticError::StructRedeclaration(NamedError {
                    name: name.clone(),
                }));
            }

            let mut field_map = HashMap::new();
            for field in fields {
                let field_name = &field.name;
                let field_type = &field.field_type;
                // Ensure no duplicate field names.
                if field_map.contains_key(field_name.as_str()) {
                    return Err(SemanticError::DuplicateField {
                        struct_name: name.clone(),
                        field: field_name.clone(),
                    });
                }
                
                // Validate decorators on this field
                super::decorators::validate_field_decorators(
                    &field.decorators,
                    field_type,
                    field_name,
                    name,
                )?;
                
                field_map.insert(field_name.clone(), field_type.clone());
            }

            // Insert struct type into the struct registry
            self.struct_table.insert(name.clone(), field_map.clone());

            // Insert struct type into the symbol table for type checking
            self.symbol_table.insert(
                name.clone(),
                SymbolInfo {
                    ty: TypeNode::Struct(name.clone(), field_map),
                    mutable: false,
                    is_ref_counted: true,
                    is_parameter: false,
                },
            );
        }
        Ok(())
    }

    /// This function checks for redeclaration of enum names, validates variant names and types,
    /// ensures no duplicate variants, and adds the enum type to the symbol table.
    /// Returns semantic errors for any violations.
    pub fn analyze_enum(&mut self, node: &AstNode) -> Result<(), SemanticError> {
        if let AstNode::EnumDecl {
            name,
            variants,
            is_public,
        } = node
        {
            // Prevent redeclaration of enum names.
            if self.symbol_table.contains_key(name) {
                return Err(SemanticError::EnumRedeclaration(NamedError {
                    name: name.clone(),
                }));
            }

            let mut variant_map = HashMap::new();
            for variant in variants {
                let variant_name = &variant.name;
                let variant_type = &variant.payload;
                // Ensure no duplicate variant names.
                if variant_map.contains_key(variant_name.as_str()) {
                    return Err(SemanticError::DuplicateEnumVariant {
                        enum_name: name.clone(),
                        variant: variant_name.clone(),
                    });
                }
                variant_map.insert(variant_name.clone(), variant_type.clone());
            }

            // Insert enum type into the enum registry
            self.enum_table.insert(name.clone(), variant_map.clone());

            // Insert enum type into the symbol table for type checking
            self.symbol_table.insert(
                name.clone(),
                SymbolInfo {
                    ty: TypeNode::Enum(name.clone(), variant_map),
                    mutable: false,
                    is_ref_counted: true,
                    is_parameter: false,
                },
            );
        }
        Ok(())
    }

    /// Enforces that functions with return types use Ok expressions, not bare Return statements
    /// If function has error type, Return is never allowed. If no error type, bare Return is allowed for void functions.
    fn ensure_uses_ok_not_return(
        &self,
        body: &[AstNode],
        function_name: &str,
        has_error_type: bool,
    ) -> Result<(), SemanticError> {
        self.check_return_usage(body, function_name, has_error_type)
    }

    /// Recursively checks for Return statements that should be Ok or Err
    fn check_return_usage(
        &self,
        nodes: &[AstNode],
        function_name: &str,
        has_error_type: bool,
    ) -> Result<(), SemanticError> {
        for node in nodes {
            match node {
                AstNode::Return { values } => {
                    // Rule 16: Allow `return Ok ...` and `return Err ...` syntax
                    // Check if the return value is an Ok or Err expression
                    let is_ok_or_err = values.len() == 1
                        && matches!(&values[0], AstNode::OkExpr { .. } | AstNode::ErrExpr { .. });

                    if is_ok_or_err {
                        // `return Ok ...` or `return Err ...` is allowed
                        continue;
                    }

                    if has_error_type {
                        // If function has error type, Return is not allowed at all (must use Ok/Err)
                        return Err(SemanticError::UnexpectedReturnWithReturnType {
                            function: function_name.to_string(),
                        });
                    }
                    // If function has NO error type, bare return is allowed (no check needed)
                }
                AstNode::ConditionalStmt {
                    then_block,
                    else_branch,
                    ..
                } => {
                    self.check_return_usage(then_block, function_name, has_error_type)?;
                    if let Some(else_block) = else_branch {
                        self.check_return_usage(
                            &vec![*else_block.clone()],
                            function_name,
                            has_error_type,
                        )?;
                    }
                }
                AstNode::Block(inner_nodes) => {
                    self.check_return_usage(inner_nodes, function_name, has_error_type)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Enforces that functions declaring error types must have at least one Err expression
    fn ensure_has_error_path(
        &self,
        body: &[AstNode],
        function_name: &str,
    ) -> Result<(), SemanticError> {
        if !self.has_error_statement(body) {
            return Err(SemanticError::MissingErrInFunctionWithErrorType {
                function: function_name.to_string(),
            });
        }
        Ok(())
    }

    /// Recursively checks if a function body contains at least one Err expression or TryPropagate (?)
    /// TryPropagate counts because it can propagate errors from called functions
    fn has_error_statement(&self, nodes: &[AstNode]) -> bool {
        for node in nodes {
            if self.node_has_error_path(node) {
                return true;
            }
        }
        false
    }

    /// Check if a single node contains an error path (Err or TryPropagate)
    fn node_has_error_path(&self, node: &AstNode) -> bool {
        match node {
            AstNode::ErrExpr { .. } => true,
            // TryPropagate (?) is a valid error path - it propagates errors from child calls
            AstNode::TryPropagate { .. } => true,
            // Check inside LetDecl value expression
            AstNode::LetDecl { value, .. } => self.node_has_error_path(value),
            // Check inside Return values
            AstNode::Return { values } => values.iter().any(|v| self.node_has_error_path(v)),
            // Check inside OkExpr values
            AstNode::OkExpr { values } => values.iter().any(|v| self.node_has_error_path(v)),
            // Check conditionals
            AstNode::ConditionalStmt {
                then_block,
                else_branch,
                ..
            } => {
                if self.has_error_statement(then_block) {
                    return true;
                }
                if let Some(else_block) = else_branch {
                    if self.node_has_error_path(else_block) {
                        return true;
                    }
                }
                false
            }
            AstNode::Block(inner_nodes) => self.has_error_statement(inner_nodes),
            // Check inside FunctionCall arguments (shouldn't contain errors but be thorough)
            AstNode::FunctionCall { args, .. } => args.iter().any(|a| self.node_has_error_path(a)),
            // Check inside ManualErrorExtract
            AstNode::ManualErrorExtract { .. } => true, // Manual extract is an error path
            _ => false,
        }
    }
}
