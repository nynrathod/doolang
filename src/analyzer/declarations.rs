use super::analyzer::SemanticAnalyzer;
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
                            // Otherwise, infer normally
                            let rhs_types_vec = self.infer_rhs_types(value, 1)?;
                            let inferred = rhs_types_vec.get(0).cloned().ok_or_else(|| {
                                SemanticError::VarTypeMismatch(TypeMismatch {
                                    expected: annotated_type.clone(),
                                    found: TypeNode::Void,
                                    value: Some(value.clone()),
                                    line: None,
                                    col: None,
                                })
                            })?;

                            // Verify inferred type matches annotation
                            if inferred != *annotated_type {
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
                } else {
                    // Use infer_rhs_types to ensure function call argument checks are performed
                    let rhs_types_vec = self.infer_rhs_types(value, 1)?;
                    rhs_types_vec.get(0).cloned().ok_or_else(|| {
                        SemanticError::VarTypeMismatch(TypeMismatch {
                            expected: TypeNode::Int,
                            found: TypeNode::Void,
                            value: Some(value.clone()),
                            line: None,
                            col: None,
                        })
                    })?
                };

                // Update the type annotation to reflect the inferred type if it was missing.
                *type_annotation = Some(rhs_type.clone());

                // println!("Before: {:?}", is_ref_counted);

                // Update AST with reference counting info based on the type.
                *is_ref_counted = Some(Self::should_be_rc(&rhs_type));
                // println!("After: {:?}", is_ref_counted);

                // Validate and collect assignment targets from the pattern.
                let targets = self.collect_and_validate_targets(pattern)?;

                // If RHS is a tuple, each element must match a pattern.
                // Otherwise, treat RHS as a single-element list.
                let rhs_types = match &rhs_type {
                    TypeNode::Tuple(types) => types.clone(),
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
        body: &mut Vec<AstNode>,
    ) -> Result<(), SemanticError> {
        // Function signature is already registered in analyze_program's first pass
        // No need to check for redeclaration or add to function_table here

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

        for (param_name, param_type) in params.iter() {
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

            // Ensure no return values are present in Void functions.
            for node in body.iter() {
                if let AstNode::Return { values } = node {
                    if !values.is_empty() {
                        return Err(SemanticError::InvalidReturnInVoidFunction {
                            function: name.to_string(),
                        });
                    }
                }
            }

            // Append implicit empty return if last statement is not Return.
            if let Some(last) = body.last() {
                if !matches!(last, AstNode::Return { .. }) {
                    body.push(AstNode::Return { values: vec![] });
                }
            }
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

        self.function_depth += 1;
        // Analyze function body with isolated scope.
        self.analyze_program(body)?;

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
    fn ensure_has_return(&self, body: &Vec<AstNode>, fn_name: &str) -> Result<(), SemanticError> {
        if !self.has_return_statement(body) {
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
                AstNode::Return { .. } => return true,
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
        match expected {
            TypeNode::Tuple(expected_vec) => {
                // For tuple returns, check length and types of each element.
                if values.len() != expected_vec.len() {
                    return Err(SemanticError::ReturnTypeMismatch {
                        function: fn_name.to_string(),
                        mismatch: TypeMismatch {
                            expected: expected.clone(),
                            found: TypeNode::Tuple(
                                values
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
                for (value, expected_type) in values.iter().zip(expected_vec.iter()) {
                    let value_type = self.infer_type(value)?;
                    if &value_type != expected_type {
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
                if values.len() != 1 {
                    return Err(SemanticError::ReturnTypeMismatch {
                        function: fn_name.to_string(),
                        mismatch: TypeMismatch {
                            expected: expected.clone(),
                            found: TypeNode::Tuple(
                                values
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
                let value_type = self.infer_type(&values[0])?;
                if &value_type != expected {
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
        if let AstNode::StructDecl { name, fields } = node {
            // Prevent redeclaration of struct names.
            if self.symbol_table.contains_key(name) {
                return Err(SemanticError::StructRedeclaration(NamedError {
                    name: name.clone(),
                }));
            }

            let mut field_map = HashMap::new();
            for (field_name, field_type) in fields {
                // Ensure no duplicate field names.
                if field_map.contains_key(field_name) {
                    return Err(SemanticError::DuplicateField {
                        struct_name: name.clone(),
                        field: field_name.clone(),
                    });
                }
                field_map.insert(field_name.clone(), field_type.clone());
            }

            // Insert struct type into the symbol table
            // Insert struct type into the symbol table.
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
        if let AstNode::EnumDecl { name, variants } = node {
            // Prevent redeclaration of enum names.
            if self.symbol_table.contains_key(name) {
                return Err(SemanticError::EnumRedeclaration(NamedError {
                    name: name.clone(),
                }));
            }

            let mut variant_map = HashMap::new();
            for (variant_name, variant_type) in variants {
                // Ensure no duplicate variant names.
                if variant_map.contains_key(variant_name) {
                    return Err(SemanticError::DuplicateEnumVariant {
                        enum_name: name.clone(),
                        variant: variant_name.clone(),
                    });
                }
                variant_map.insert(variant_name.clone(), variant_type.clone());
            }

            // Insert enum type into the symbol table.
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
}
