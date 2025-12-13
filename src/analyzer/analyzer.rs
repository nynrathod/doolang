use crate::analyzer::types::{NamedError, SemanticError};
use crate::limits::{
    ANALYZER_MAX_FUNCTION_DEPTH, ANALYZER_MAX_LOOP_DEPTH, ANALYZER_MAX_SCOPE_DEPTH,
};
use crate::parser::ast::{AstNode, Pattern, TypeNode};
use crate::path_resolver::PathResolver;
use bumpalo::Bump;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct SymbolInfo {
    pub ty: TypeNode,         // The type of the variable
    pub mutable: bool,        // Is the variable mutable?
    pub is_ref_counted: bool, // Should reference counting be used?
    pub is_parameter: bool,   // Is this variable a function parameter?
}

/// The main semantic analyzer for the language.
/// Responsible for type checking, symbol resolution, and semantic validation.
pub struct SemanticAnalyzer {
    pub(crate) symbol_table: HashMap<String, SymbolInfo>, // Current scope variables
    pub(crate) function_table: HashMap<String, (Vec<TypeNode>, TypeNode, Option<TypeNode>)>, // Function signatures (params, return_type, error_type)
    pub(crate) struct_table: HashMap<String, HashMap<String, TypeNode>>, // Struct definitions: name -> field map
    pub(crate) enum_table: HashMap<String, HashMap<String, Option<TypeNode>>>, // Enum definitions: name -> variant map
    pub enum_variant_order: HashMap<String, Vec<(String, Option<TypeNode>)>>, // Ordered enum variants: enum_name -> [(variant_name, payload_type)]
    pub(crate) method_table:
        HashMap<String, HashMap<String, (Vec<TypeNode>, TypeNode, Option<TypeNode>)>>, // Methods per type: TypeName -> MethodName -> (params, return_type, error_type)
    pub(crate) struct_field_visibility: HashMap<String, HashMap<String, bool>>, // Track field visibility for imported structs: struct_name -> (field_name -> is_public)
    pub(crate) imported_struct_names: std::collections::HashSet<String>, // Track which structs are imported (for visibility checking)
    pub struct_field_decorators: HashMap<String, HashMap<String, Vec<(String, Vec<String>)>>>, // Struct field decorators: struct_name -> field_name -> [(decorator_name, [args])]
    pub ffi_metadata: HashMap<String, (Option<String>, Option<String>)>, // Function FFI metadata: func_name -> (ffi_lib, ffi_symbol)

    pub(crate) outer_symbol_table: Option<HashMap<String, SymbolInfo>>, // For nested scopes
    pub(crate) project_root: PathBuf, // Root directory for module resolution
    pub(crate) imported_modules: HashMap<String, bool>, // Track imported modules to prevent circular imports
    pub imported_functions: Vec<AstNode>, // Store imported function AST nodes for MIR generation
    pub imported_structs: Vec<AstNode>,   // Store imported struct AST nodes for MIR generation
    pub function_aliases: HashMap<String, String>, // Maps alias names to original function names
    pub loop_depth: usize,                // Track loop nesting for break/continue error handling
    pub scope_stack: Vec<HashMap<String, SymbolInfo>>, // Scope stack for block scoping
    pub function_depth: usize,            // Track function nesting for return statement validation
    pub scope_sizes_stack: Vec<usize>,    // Track symbol table size at each scope level
    pub collected_errors: Vec<SemanticError>, // Collect all errors for reporting
    pub is_main_module: bool,             // Track if analyzing main program or imported module
    pub type_inference_depth: RefCell<usize>, // Track type inference recursion depth using interior mutability
    pub(crate) current_function_error_type: Option<TypeNode>, // Track current function's error type for ? operator validation
}

impl SemanticAnalyzer {
    /// Lookup a variable by name, searching current scope and then walking up the scope stack.
    pub fn lookup_variable(&self, name: &str) -> Option<&SymbolInfo> {
        if let Some(info) = self.symbol_table.get(name) {
            return Some(info);
        }
        for scope in self.scope_stack.iter().rev() {
            if let Some(info) = scope.get(name) {
                return Some(info);
            }
        }
        None
    }

    /// Create a new semantic analyzer with empty symbol/function tables.
    pub fn new(project_root: Option<PathBuf>) -> Self {
        let project_root = project_root
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let mut function_table = HashMap::new();

        function_table.insert("print".to_string(), (vec![], TypeNode::Void, None));
        function_table.insert("println".to_string(), (vec![], TypeNode::Void, None));
        function_table.insert(
            "panic".to_string(),
            (vec![TypeNode::String], TypeNode::Void, None),
        );
        function_table.insert("typeOf".to_string(), (vec![], TypeNode::String, None));

        Self {
            symbol_table: HashMap::new(),
            function_table,
            struct_table: HashMap::new(),
            enum_table: HashMap::new(),
            enum_variant_order: HashMap::new(),
            method_table: HashMap::new(),
            struct_field_visibility: HashMap::new(),
            imported_struct_names: std::collections::HashSet::new(),
            struct_field_decorators: HashMap::new(),
            ffi_metadata: HashMap::new(),
            outer_symbol_table: None,
            project_root,
            imported_modules: HashMap::new(),
            imported_functions: Vec::new(),
            imported_structs: Vec::new(),
            function_aliases: HashMap::new(),
            loop_depth: 0,
            scope_stack: Vec::new(),
            function_depth: 0,
            scope_sizes_stack: Vec::new(),
            collected_errors: Vec::new(),
            is_main_module: true,
            type_inference_depth: RefCell::new(0),
            current_function_error_type: None,
        }
    }

    /// Check if function nesting is valid
    pub(crate) fn check_function_depth(&self) -> Result<(), SemanticError> {
        if self.function_depth > ANALYZER_MAX_FUNCTION_DEPTH {
            return Err(SemanticError::UnexpectedNode {
                expected: "Function nesting too deep".to_string(),
            });
        }
        Ok(())
    }

    /// Check if loop nesting is valid
    pub(crate) fn check_loop_depth(&self) -> Result<(), SemanticError> {
        if self.loop_depth > ANALYZER_MAX_LOOP_DEPTH {
            return Err(SemanticError::UnexpectedNode {
                expected: "Loop nesting too deep".to_string(),
            });
        }
        Ok(())
    }

    /// Check if scope nesting is valid
    pub(crate) fn check_scope_depth(&self) -> Result<(), SemanticError> {
        if self.scope_stack.len() > ANALYZER_MAX_SCOPE_DEPTH {
            return Err(SemanticError::UnexpectedNode {
                expected: "Scope nesting too deep".to_string(),
            });
        }
        Ok(())
    }

    /// Analyze a list of AST nodes (entire program or a block).
    /// Returns Ok if all nodes are semantically valid, or an error otherwise.
    /// Uses a two-pass approach:
    /// 1. First pass: Process imports and register all function signatures (for forward references)
    /// 2. Second pass: Analyze function bodies and other statements
    pub fn analyze_program(&mut self, nodes: &mut Vec<AstNode>) -> Result<(), SemanticError> {
        let mut import_stack = Vec::new();
        self.analyze_program_with_stack(nodes, &mut import_stack)
    }

    /// Internal method that performs semantic analysis with an import stack for circular import detection.
    /// This method is used recursively to ensure the import stack is maintained across all import chains.
    fn analyze_program_with_stack(
        &mut self,
        nodes: &mut Vec<AstNode>,
        import_stack: &mut Vec<String>,
    ) -> Result<(), SemanticError> {
        // PREPROCESSING 1: Transform inline closures in route handlers into named functions
        // This converts app.post("/path", (req) -> Res { ... }) into a named function
        crate::analyzer::route_transform::transform_inline_closures(nodes);

        // PREPROCESSING 2: Transform route group DSL syntax before analysis
        // This expands app.group("/api", { get(...), post(...) }) into individual route calls
        crate::analyzer::route_transform::transform_route_groups(nodes);

        // FIRST PASS: Process imports, register all function signatures, structs, and enums
        // Collect errors but don't stop at first module error

        for node in nodes.iter_mut() {
            match node {
                // Process imports first to load external functions
                AstNode::Import { path, items } => {
                    if let Err(e) = self.import_module(path, items, import_stack) {
                        // Collect error but don't return immediately - continue processing
                        self.collected_errors.push(e);
                    }
                }
                // Register struct declarations early so they can be used in function signatures
                AstNode::StructDecl {
                    name,
                    fields,
                    is_public,
                } => {
                    if self.struct_table.contains_key(name) {
                        self.collected_errors
                            .push(SemanticError::StructRedeclaration(NamedError {
                                name: name.clone(),
                            }));
                        continue;
                    }
                    // Build field map and validate decorators
                    let mut field_map = HashMap::new();
                    let mut field_decorators_map = HashMap::new();

                    for field in fields {
                        // Validate decorators on this field
                        if let Err(e) = super::decorators::validate_field_decorators(
                            &field.decorators,
                            &field.field_type,
                            &field.name,
                            name,
                        ) {
                            self.collected_errors.push(e);
                        }

                        // Store decorators for codegen
                        if !field.decorators.is_empty() {
                            let decorator_info: Vec<(String, Vec<String>)> = field
                                .decorators
                                .iter()
                                .map(|d| {
                                    let args: Vec<String> = d
                                        .args
                                        .iter()
                                        .map(|arg| match arg {
                                            AstNode::StringLiteral(s) => s.clone(),
                                            AstNode::NumberLiteral(n) => n.to_string(),
                                            AstNode::FloatLiteral(f) => f.to_string(),
                                            _ => String::new(),
                                        })
                                        .collect();
                                    (d.name.clone(), args)
                                })
                                .collect();
                            field_decorators_map.insert(field.name.clone(), decorator_info);
                        }

                        field_map.insert(field.name.clone(), field.field_type.clone());
                    }

                    // Store decorators in struct_field_decorators for codegen
                    if !field_decorators_map.is_empty() {
                        self.struct_field_decorators
                            .insert(name.clone(), field_decorators_map);
                    }
                    self.struct_table.insert(name.clone(), field_map.clone());
                    // Also add to symbol table
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
                // Register enum declarations early so they can be used in function signatures
                AstNode::EnumDecl {
                    name,
                    variants,
                    is_public,
                } => {
                    if self.enum_table.contains_key(name) {
                        self.collected_errors
                            .push(SemanticError::EnumRedeclaration(NamedError {
                                name: name.clone(),
                            }));
                        continue;
                    }
                    // Build variant map
                    let mut variant_map = HashMap::new();
                    let mut variant_order = Vec::new();
                    for variant in variants {
                        variant_map.insert(variant.name.clone(), variant.payload.clone());
                        variant_order.push((variant.name.clone(), variant.payload.clone()));
                    }
                    self.enum_table.insert(name.clone(), variant_map.clone());
                    self.enum_variant_order.insert(name.clone(), variant_order);
                    // Also add to symbol table
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
                // Register local function signatures
                AstNode::FunctionDecl {
                    name,
                    params,
                    return_type,
                    error_type,
                    receiver_type,
                    associated_type,
                    decorators,
                    ..
                } => {
                    // Extract FFI metadata from decorators
                    let mut ffi_lib: Option<String> = None;
                    let mut ffi_symbol: Option<String> = None;

                    for decorator in decorators {
                        match decorator.name.as_str() {
                            "ffi" => {
                                // @ffi("library_name")
                                if let Some(arg) = decorator.args.first() {
                                    if let AstNode::StringLiteral(lib_name) = arg {
                                        ffi_lib = Some(lib_name.clone());
                                    }
                                }
                            }
                            "extern" => {
                                // @extern("symbol_name")
                                if let Some(arg) = decorator.args.first() {
                                    if let AstNode::StringLiteral(sym_name) = arg {
                                        ffi_symbol = Some(sym_name.clone());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }

                    // Collect parameter types
                    // For instance methods (first param is 'self' with no type), skip first parameter
                    // For static methods (first param has type), include all parameters
                    let param_types: Vec<TypeNode> = if receiver_type.is_some() {
                        // Check if this is a static method (first param has type annotation)
                        let is_static_method =
                            params.first().map(|(_, t)| t.is_some()).unwrap_or(false);

                        if is_static_method {
                            // Static method: include all parameters
                            params
                                .iter()
                                .map(|(_, t)| t.clone().unwrap_or(TypeNode::Int))
                                .collect()
                        } else {
                            // Instance method: skip first parameter (receiver 'self')
                            params
                                .iter()
                                .skip(1)
                                .map(|(_, t)| t.clone().unwrap_or(TypeNode::Int))
                                .collect()
                        }
                    } else {
                        // Regular function: include all parameters
                        params
                            .iter()
                            .map(|(_, t)| t.clone().unwrap_or(TypeNode::Int))
                            .collect()
                    };

                    // Use associated_type for both static and instance methods
                    if let Some(type_name) = associated_type {
                        // This is a method declaration (fn Type.method)
                        // Register in method_table
                        let methods = self
                            .method_table
                            .entry(type_name.clone())
                            .or_insert_with(HashMap::new);

                        if methods.contains_key(name) {
                            self.collected_errors
                                .push(SemanticError::FunctionRedeclaration(NamedError {
                                    name: format!("{}.{}", type_name, name),
                                }));
                            continue;
                        }

                        methods.insert(
                            name.to_string(),
                            (
                                param_types.clone(),
                                return_type.clone().unwrap_or(TypeNode::Void),
                                error_type.clone(),
                            ),
                        );

                        // Also register in function_table with mangled name for codegen
                        let mangled_name = format!("{}::{}", type_name, name);
                        self.function_table.insert(
                            mangled_name,
                            (
                                param_types,
                                return_type.clone().unwrap_or(TypeNode::Void),
                                error_type.clone(),
                            ),
                        );
                    } else {
                        // Regular function declaration
                        // Check if function already defined
                        if self.function_table.contains_key(name) {
                            self.collected_errors
                                .push(SemanticError::FunctionRedeclaration(NamedError {
                                    name: name.to_string(),
                                }));
                            continue;
                        }

                        // Register function signature (all functions, not just public ones)
                        self.function_table.insert(
                            name.to_string(),
                            (
                                param_types,
                                return_type.clone().unwrap_or(TypeNode::Void),
                                error_type.clone(),
                            ),
                        );

                        // Store FFI metadata if present
                        if ffi_lib.is_some() || ffi_symbol.is_some() {
                            self.ffi_metadata
                                .insert(name.to_string(), (ffi_lib.clone(), ffi_symbol.clone()));
                        }
                    }

                    // Also store FFI metadata for methods (with mangled name)
                    if let Some(type_name) = associated_type {
                        let mangled_name = format!("{}::{}", type_name, name);
                        if ffi_lib.is_some() || ffi_symbol.is_some() {
                            self.ffi_metadata
                                .insert(mangled_name, (ffi_lib, ffi_symbol));
                        }
                    }
                }
                _ => {} // Skip other nodes in first pass
            }
        }

        // SECOND PASS: Analyze all nodes (including function bodies)
        // Skip imports, enums, and structs as they're already processed in first pass
        for node in nodes {
            if !matches!(
                node,
                AstNode::Import { .. } | AstNode::EnumDecl { .. } | AstNode::StructDecl { .. }
            ) {
                if let Err(e) = self.analyze_node(node) {
                    self.collected_errors.push(e);
                }
            }
        }

        // Check that main() function exists only for the main module
        if self.is_main_module && !self.function_table.contains_key("main") {
            self.collected_errors.push(SemanticError::ParseErrorMsg(
                "main() function is missing".to_string(),
            ));
        }

        // If any errors were collected, prioritize reporting a circular import error
        if !self.collected_errors.is_empty() {
            // Prefer to report a circular import error if present
            if let Some(circular) = self
                .collected_errors
                .iter()
                .find(|e| matches!(e, SemanticError::CircularImport { .. }))
            {
                return Err(SemanticError::CircularImport {
                    cycle: if let SemanticError::CircularImport { cycle } = circular {
                        cycle.clone()
                    } else {
                        vec![]
                    },
                });
            }
            // Otherwise, report the first error as before
            return Err(self.collected_errors.remove(0));
        } else {
            Ok(())
        }
    }

    /// Determines if a type should use reference counting.
    /// Used for arrays, maps, and strings.
    pub fn should_be_rc(ty: &TypeNode) -> bool {
        matches!(
            ty,
            TypeNode::Array(_) | TypeNode::Map(_, _) | TypeNode::String
        )
    }

    /// Helper to analyze a BlockExpr or regular expression
    /// For BlockExpr, creates a scope and analyzes statements before the result
    /// For other expressions, just infers the type
    fn analyze_block_or_expr(&mut self, expr: &mut AstNode) -> Result<(), SemanticError> {
        match expr {
            AstNode::BlockExpr { statements, result } => {
                // Save the current symbol table to restore after block
                let parent_scope = self.symbol_table.clone();
                let scope_size = self.symbol_table.len();
                self.scope_stack.push(HashMap::new());
                self.scope_sizes_stack.push(scope_size);

                // Analyze all statements
                for stmt in statements {
                    self.analyze_node(stmt)?;
                }

                // Analyze the result expression
                let _ = self.infer_type(result)?;

                // Restore symbol table
                self.scope_stack.pop();
                self.scope_sizes_stack.pop();
                self.symbol_table = parent_scope;

                Ok(())
            }
            AstNode::ConditionalExpr {
                condition,
                then_expr,
                else_expr,
            } => {
                // Recursively handle nested conditional expressions
                let _ = self.infer_type(condition)?;
                self.analyze_block_or_expr(then_expr)?;
                self.analyze_block_or_expr(else_expr)?;
                Ok(())
            }
            _ => {
                // For other expressions, just infer the type
                let _ = self.infer_type(expr)?;
                Ok(())
            }
        }
    }

    /// Dispatch analysis based on AST node type.
    /// Calls the appropriate analysis function for each AST node variant.
    /// Ensures semantic correctness for declarations, assignments, control flow, etc.
    pub fn analyze_node(&mut self, node: &mut AstNode) -> Result<(), SemanticError> {
        // Check other depth limits
        self.check_function_depth()?;
        self.check_loop_depth()?;
        self.check_scope_depth()?;

        let result = self.analyze_node_inner(node);
        result
    }

    fn analyze_node_inner(&mut self, node: &mut AstNode) -> Result<(), SemanticError> {
        match node {
            // Declarations
            AstNode::LetDecl { .. } => self.analyze_let_decl(node),
            AstNode::FunctionDecl {
                name,
                visibility,
                params,
                return_type,
                error_type,
                body,
                decorators,
                receiver_type,
                associated_type,
                is_expression,
            } => self.analyze_functional_decl(
                name,
                visibility,
                params,
                return_type,
                error_type,
                body,
                decorators,
                receiver_type,
                is_expression,
            ),
            AstNode::StructDecl { .. } => self.analyze_struct(node),
            AstNode::EnumDecl { .. } => self.analyze_enum(node),

            // Import statement - already processed in first pass of analyze_program
            AstNode::Import { .. } => Ok(()),

            // Statements
            AstNode::Assignment { pattern, value } => self.analyze_assignment(pattern, value),
            AstNode::CompoundAssignment { pattern, op, value } => {
                self.analyze_compound_assignment(pattern, *op, value)
            }
            AstNode::IncrementDecrement { variable, op: _ } => {
                self.analyze_increment_decrement(variable)
            }
            AstNode::ElementAssignment {
                array,
                index,
                value,
            } => self.analyze_element_assignment(array, index, value),
            AstNode::FieldAssignment {
                object,
                field,
                value,
            } => self.analyze_field_assignment(object, field, value),
            AstNode::Return { values } => {
                // Check that return is inside a function
                if self.function_depth == 0 {
                    return Err(SemanticError::UndeclaredFunction(NamedError {
                        name: "return statement outside of function".to_string(),
                    }));
                }
                // Check return value types
                for v in values {
                    self.infer_type(v)?;
                }
                Ok(())
            }
            AstNode::Print { .. } => self.analyze_print(node),
            AstNode::Break => {
                // Error if not inside a loop
                if self.loop_depth == 0 {
                    return Err(SemanticError::UnexpectedNode {
                        expected: "break inside loop".to_string(),
                    });
                }
                Ok(())
            }
            AstNode::Continue => {
                // Error if not inside a loop
                if self.loop_depth == 0 {
                    return Err(SemanticError::UnexpectedNode {
                        expected: "continue inside loop".to_string(),
                    });
                }
                Ok(())
            }
            AstNode::OkExpr { values } => {
                // Check that Ok is inside a function
                if self.function_depth == 0 {
                    return Err(SemanticError::UnexpectedNode {
                        expected: "Ok expression inside function with error type".to_string(),
                    });
                }
                // Ok can be used as a return statement even without error types
                // This allows Ok to act like return in non-error-returning functions
                // Type check values
                for v in values {
                    self.infer_type(v)?;
                }
                Ok(())
            }
            AstNode::ErrExpr { value } => {
                // Check that Err is inside a function
                if self.function_depth == 0 {
                    return Err(SemanticError::UnexpectedNode {
                        expected: "Err expression inside function with error type".to_string(),
                    });
                }
                // Check that current function has an error return type
                if self.current_function_error_type.is_none() {
                    return Err(SemanticError::UnexpectedNode {
                        expected: "Err can only be used in functions with error return type (e.g., -> T ! E or ! E)".to_string(),
                    });
                }
                // Type check error value
                self.infer_type(value)?;
                Ok(())
            }
            AstNode::TryPropagate { expr } => {
                // Check that ? is inside a function with error type
                if self.function_depth == 0 {
                    return Err(SemanticError::UnexpectedNode {
                        expected: "? operator inside function with error type".to_string(),
                    });
                }
                // Check that current function has an error return type
                if self.current_function_error_type.is_none() {
                    return Err(SemanticError::UnexpectedNode {
                        expected: "? operator can only be used in functions with error return type (e.g., -> T ! E or ! E)".to_string(),
                    });
                }
                // Type check the expression
                self.infer_type(expr)?;
                Ok(())
            }
            AstNode::UnwrapOrPanic { expr, panic_msg } => {
                // Rule 13: ?? panic() operator unwraps Result or panics
                // This can be used anywhere, doesn't require error type in signature
                // Type check the expression (should return a Result type)
                self.infer_type(expr)?;
                // Type check the panic message (should be a string or function call)
                self.infer_type(panic_msg)?;
                Ok(())
            }
            AstNode::ManualErrorExtract {
                expr,
                ok_pattern,
                error_var,
            } => {
                // Rule 12: Cannot ignore both ok value(s) and error
                // let _, _ = divide(10, 2); is not allowed
                let ok_ignored = matches!(ok_pattern, Pattern::Wildcard);
                let err_ignored = error_var == "_";

                if ok_ignored && err_ignored {
                    return Err(SemanticError::UnexpectedNode {
                        expected: "Cannot ignore both success value(s) and error in manual error extraction. Must capture at least one.".to_string(),
                    });
                }

                // Type check the expression that returns Result
                let expr_type = self.infer_type(expr)?;

                // Extract Ok and Err types from Result
                let (ok_type, err_type) = if let TypeNode::Result(ok_type, err_type) = &expr_type {
                    (ok_type.as_ref().clone(), err_type.as_ref().clone())
                } else {
                    // Fallback if not a Result type
                    (TypeNode::Int, TypeNode::String)
                };

                // Declare the error variable in symbol table with the actual error type
                if error_var != "_" {
                    self.symbol_table.insert(
                        error_var.clone(),
                        SymbolInfo {
                            ty: err_type.clone(),
                            mutable: false,
                            is_ref_counted: true,
                            is_parameter: false,
                        },
                    );
                }

                // Declare ok pattern variables in symbol table
                match ok_pattern {
                    Pattern::Identifier(name) => {
                        // For single value, use the extracted ok_type
                        self.symbol_table.insert(
                            name.clone(),
                            SymbolInfo {
                                ty: ok_type.clone(),
                                mutable: false,
                                is_ref_counted: true,
                                is_parameter: false,
                            },
                        );
                    }
                    Pattern::Tuple(patterns) => {
                        // For tuple, extract element types from tuple type
                        let element_types = if let TypeNode::Tuple(types) = &ok_type {
                            types.clone()
                        } else {
                            // Fallback: assign Int to all patterns
                            vec![TypeNode::Int; patterns.len()]
                        };

                        for (i, pattern) in patterns.iter().enumerate() {
                            if let Pattern::Identifier(name) = pattern {
                                let elem_type =
                                    element_types.get(i).cloned().unwrap_or(TypeNode::Int);
                                self.symbol_table.insert(
                                    name.clone(),
                                    SymbolInfo {
                                        ty: elem_type,
                                        mutable: false,
                                        is_ref_counted: true,
                                        is_parameter: false,
                                    },
                                );
                            }
                        }
                    }
                    Pattern::Wildcard => {}
                }
                Ok(())
            }
            AstNode::ConditionalStmt {
                condition,
                then_block,
                else_branch,
            } => self.analyze_conditional_stmt(condition, then_block, else_branch),
            AstNode::ForLoopStmt {
                pattern,
                iterable,
                body,
            } => self.analyze_for_stmt(pattern, iterable.as_deref_mut(), body),
            AstNode::MatchExpr { values, arms } => {
                // Type check the match values if present
                for v in values {
                    self.infer_type(v)?;
                }

                // Type check all match arms
                // We need to handle each arm carefully to support nested matches with proper scope
                for arm in arms.iter_mut() {
                    // First, extract pattern info we need (cloning to avoid borrow conflicts)
                    let pattern_info = match &arm.pattern {
                        crate::parser::ast::MatchPattern::Literal(expr) => {
                            self.infer_type(expr)?;
                            None // No bindings
                        }
                        crate::parser::ast::MatchPattern::Condition(expr) => {
                            self.infer_type(expr)?;
                            None // No bindings
                        }
                        crate::parser::ast::MatchPattern::Wildcard => {
                            None // No bindings
                        }
                        crate::parser::ast::MatchPattern::EnumVariant { enum_name, variant } => {
                            // Type check enum variant exists
                            let _ = (enum_name, variant);
                            None // No bindings
                        }
                        crate::parser::ast::MatchPattern::EnumVariantWithPayload {
                            enum_name,
                            variant,
                            bindings,
                        } => {
                            // Look up the actual payload type from the enum definition
                            let payload_type = self
                                .enum_table
                                .get(enum_name)
                                .and_then(|variants| variants.get(variant))
                                .and_then(|opt_type| opt_type.clone());

                            // Collect bindings with their types
                            let mut binding_types: Vec<(String, TypeNode)> = Vec::new();
                            if let Some(ref ptype) = payload_type {
                                if let TypeNode::Tuple(types) = ptype {
                                    // Tuple payload - multiple bindings
                                    for (i, binding) in bindings.iter().enumerate() {
                                        let elem_type =
                                            types.get(i).cloned().unwrap_or(TypeNode::Int);
                                        binding_types.push((binding.clone(), elem_type));
                                    }
                                } else {
                                    // Single payload - one binding
                                    if let Some(binding) = bindings.first() {
                                        binding_types.push((binding.clone(), ptype.clone()));
                                    }
                                }
                            } else {
                                // Fallback to Int if type lookup fails
                                for binding in bindings {
                                    binding_types.push((binding.clone(), TypeNode::Int));
                                }
                            }
                            Some(binding_types)
                        }
                        crate::parser::ast::MatchPattern::Tuple(patterns) => {
                            // Type check each pattern in the tuple
                            for pattern in patterns {
                                match pattern {
                                    crate::parser::ast::MatchPattern::Literal(expr) => {
                                        self.infer_type(expr)?;
                                    }
                                    crate::parser::ast::MatchPattern::Wildcard => {
                                        // Wildcard matches anything
                                    }
                                    _ => {
                                        // Other nested patterns - just continue
                                    }
                                }
                            }
                            None // No bindings (tuple patterns don't create bindings)
                        }
                    };

                    // Now analyze the arm body with proper scope
                    if let Some(binding_types) = pattern_info {
                        // Create a new scope for this arm with the binding variables
                        let parent_scope = self.symbol_table.clone();

                        // Add the binding variables to the scope with proper types
                        for (binding, ty) in binding_types {
                            self.symbol_table.insert(
                                binding,
                                SymbolInfo {
                                    ty,
                                    mutable: false,
                                    is_parameter: false,
                                    is_ref_counted: false,
                                },
                            );
                        }

                        // Analyze the arm body - use analyze_node_inner to properly handle nested matches
                        let result = self.analyze_node_inner(&mut *arm.body);

                        // Restore the parent scope
                        self.symbol_table = parent_scope;

                        result?;
                    } else {
                        // No bindings - still use analyze_node_inner for nested match support
                        self.analyze_node_inner(&mut *arm.body)?;
                    }
                }
                Ok(())
            }
            AstNode::Block(nodes) => {
                // Save the current symbol table to restore after block
                let parent_scope = self.symbol_table.clone();
                let scope_size = self.symbol_table.len();
                self.scope_stack.push(HashMap::new()); // Marker for block scope
                self.scope_sizes_stack.push(scope_size);

                // Analyze block - variables declared here go into symbol_table
                let result = self.analyze_program(nodes);

                // Restore symbol table to parent scope (removes block variables)
                self.scope_stack.pop();
                self.scope_sizes_stack.pop();
                self.symbol_table = parent_scope;

                result
            }

            AstNode::ConditionalExpr {
                condition,
                then_expr,
                else_expr,
            } => {
                // Analyze condition
                let _ = self.infer_type(condition)?;

                // Analyze then branch (may be BlockExpr)
                self.analyze_block_or_expr(then_expr)?;

                // Analyze else branch (may be BlockExpr or another ConditionalExpr)
                self.analyze_block_or_expr(else_expr)?;

                Ok(())
            }

            AstNode::BlockExpr { statements, result } => {
                // Save the current symbol table to restore after block
                let parent_scope = self.symbol_table.clone();
                let scope_size = self.symbol_table.len();
                self.scope_stack.push(HashMap::new()); // Marker for block scope
                self.scope_sizes_stack.push(scope_size);

                // Analyze all statements - variables declared here go into symbol_table
                for stmt in statements {
                    self.analyze_node(stmt)?;
                }

                // Analyze the result expression
                let _ = self.infer_type(result)?;

                // Restore symbol table to parent scope (removes block variables)
                self.scope_stack.pop();
                self.scope_sizes_stack.pop();
                self.symbol_table = parent_scope;

                Ok(())
            }

            // Catch-all for any AST nodes not explicitly handled above.
            // We call `infer_type` to:
            // Validate that all identifiers exist in scope.
            // Ensure expressions (literals, binary/unary ops, function calls) are type-correct.
            // Future-proof: new AST node types will still be semantically validated.
            _ => {
                // Add function call argument count/type checking
                if let AstNode::FunctionCall { func, args } = node {
                    // Try to extract function name from Identifier node
                    let func_name = if let AstNode::Identifier(name) = &**func {
                        name
                    } else {
                        return Err(SemanticError::InvalidFunctionCall {
                            func: format!("{:?}", func),
                        });
                    };

                    // Check if this is actually an enum variant with data (Status::Pending(25))
                    // Parser can't distinguish between enum variant and namespace function call
                    // Must check this BEFORE looking up in function_table
                    if func_name.contains("::") {
                        let parts: Vec<&str> = func_name.split("::").collect();
                        if parts.len() == 2 {
                            let enum_name = parts[0];
                            let variant_name = parts[1];

                            // Check if the first part is an enum type
                            if let Some(enum_variants) = self.enum_table.get(enum_name) {
                                // Check if the variant exists
                                if enum_variants.contains_key(variant_name) {
                                    // This is an enum variant, not a function call
                                    // Type check it through infer_type and return
                                    self.infer_type(node)?;
                                    return Ok(());
                                }
                            }
                        }
                    }

                    // Not an enum variant, so look up as function call
                    let lookup_result = self.function_table.get(func_name);
                    if lookup_result.is_none() {
                        return Err(SemanticError::UndeclaredFunction(NamedError {
                            name: func_name.clone(),
                        }));
                    }
                    let (param_types, _return_type, _error_type) = lookup_result.unwrap();

                    // Skip argument checking for variadic built-in functions
                    if func_name != "print"
                        && func_name != "println"
                        && func_name != "panic"
                        && func_name != "typeOf"
                    {
                        // Check argument count
                        if args.len() != param_types.len() {
                            return Err(SemanticError::FunctionArgumentMismatch {
                                name: func_name.clone(),
                                expected: param_types.len(),
                                found: args.len(),
                            });
                        }

                        // Check argument types
                        for (arg, expected_type) in args.iter().zip(param_types.iter()) {
                            // Use infer_type_with_expected to handle Any types (from JSON.parse)
                            let arg_type = self.infer_type_with_expected(arg, expected_type)?;
                            if !types_compatible(
                                &arg_type,
                                expected_type,
                                &self.struct_table,
                                &self.enum_table,
                            ) {
                                return Err(SemanticError::FunctionArgumentTypeMismatch {
                                    name: func_name.clone(),
                                    expected: expected_type.clone(),
                                    found: arg_type,
                                });
                            }
                        }
                    }

                    // Return type is not used here, but could be returned if needed
                    Ok(())
                } else {
                    self.infer_type(node)?;
                    Ok(())
                }
            }
        }
    }

    // Helper to check if currently inside a loop (for break/continue validation)

    /// Resolve a module path (e.g., ["http", "Client"]) to a file path
    /// For import http::Client::Fetchuser, we want http/Client.doo
    /// The last element before the symbol is the file name
    fn resolve_module_path(&self, path: &[String], _: &Option<String>) -> Option<PathBuf> {
        // Check if this is a std import (starts with "std")
        if path.first().map(|s| s.as_str()) == Some("std") {
            // Use PathResolver to find std
            if let Ok(resolver) = PathResolver::new() {
                // Build the module path from stdlib root
                // For "std::Math::Abs" or "std::http::Server", we want std/Math.doo or std/Http.doo
                if path.len() >= 2 {
                    let mut stdlib_file = resolver.stdlib_path().to_path_buf();
                    // Use the second element as the file name (Math, String, Array, etc.)
                    // Try the exact case first
                    let module_name = &path[1];
                    stdlib_file.push(module_name);
                    stdlib_file.set_extension("doo");

                    if stdlib_file.exists() {
                        return Some(stdlib_file);
                    }

                    // If not found, try PascalCase version (capitalize first letter)
                    // This handles std::http::Server -> std/Http.doo
                    stdlib_file = resolver.stdlib_path().to_path_buf();
                    let mut pascal_case = module_name.clone();
                    if let Some(first_char) = pascal_case.chars().next() {
                        pascal_case =
                            first_char.to_uppercase().collect::<String>() + &pascal_case[1..];
                    }
                    stdlib_file.push(&pascal_case);
                    stdlib_file.set_extension("doo");

                    if stdlib_file.exists() {
                        return Some(stdlib_file);
                    }
                }
            }
        }

        // Otherwise, try project-relative path
        let mut buf = self.project_root.clone();

        // For imports, we need to determine if the path includes a symbol or not:
        // 1. Wildcard import: core::evaluator::* -> path=["core", "evaluator"], items=[Wildcard]
        //    Should resolve to core/evaluator.doo (use all parts)
        // 2. Specific symbol import: CircularA::FunctionA -> path=["CircularA", "FunctionA"], items=[]
        //    Should resolve to CircularA.doo (use all but last part)
        // 3. Namespace import: core::evaluator -> path=["core", "evaluator"], items=[]
        //    Should resolve to core/evaluator.doo (use all parts)

        // Helper function to build path with case-insensitive directory matching
        let build_path_case_insensitive =
            |base: &std::path::PathBuf, parts: &[String]| -> Option<std::path::PathBuf> {
                let mut current = base.clone();

                // For each path component, try to find it case-insensitively in the current directory
                for (idx, part) in parts.iter().enumerate() {
                    let is_last = idx == parts.len() - 1;

                    if is_last {
                        // Last component - look for .doo file
                        let mut target = current.clone();
                        target.push(part);
                        target.set_extension("doo");

                        if target.exists() {
                            return Some(target);
                        }

                        // Try case-insensitive match
                        if let Ok(entries) = std::fs::read_dir(&current) {
                            let target_lower = part.to_lowercase();
                            for entry in entries.flatten() {
                                let path = entry.path();
                                if let Some(name) = path.file_stem() {
                                    if name.to_string_lossy().to_lowercase() == target_lower {
                                        if path.extension().map(|e| e == "doo").unwrap_or(false) {
                                            return Some(path);
                                        }
                                    }
                                }
                            }
                        }
                        return None;
                    } else {
                        // Directory component
                        let mut next = current.clone();
                        next.push(part);

                        if next.exists() && next.is_dir() {
                            current = next;
                            continue;
                        }

                        // Try case-insensitive directory match
                        if let Ok(entries) = std::fs::read_dir(&current) {
                            let target_lower = part.to_lowercase();
                            let mut found = false;
                            for entry in entries.flatten() {
                                let path = entry.path();
                                if path.is_dir() {
                                    if let Some(name) = path.file_name() {
                                        if name.to_string_lossy().to_lowercase() == target_lower {
                                            current = path;
                                            found = true;
                                            break;
                                        }
                                    }
                                }
                            }
                            if !found {
                                return None;
                            }
                        } else {
                            return None;
                        }
                    }
                }

                None
            };

        // First, try using all path parts (for wildcard and namespace imports)
        if let Some(found) = build_path_case_insensitive(&self.project_root, path) {
            return Some(found);
        }

        // If that didn't work, try excluding the last part (for specific symbol imports)
        // This handles cases like std::http::Server where we want std/Http.doo
        if path.len() > 1 {
            if let Some(found) =
                build_path_case_insensitive(&self.project_root, &path[..path.len() - 1])
            {
                return Some(found);
            }
        }

        None
    }

    fn import_module(
        &mut self,
        path: &[String],
        items: &[crate::parser::ast::ImportItem],
        import_stack: &mut Vec<String>,
    ) -> Result<(), SemanticError> {
        // Create module key for circular import detection
        let module_key = path.join("::");

        // CIRCULAR DEPENDENCY DETECTION
        if import_stack.contains(&module_key) {
            let mut cycle = import_stack.clone();
            cycle.push(module_key.clone());
            return Err(SemanticError::CircularImport { cycle });
        }
        import_stack.push(module_key.clone());

        // Determine import mode:
        // 1. Wildcard import: import std::File::*; (items contains Wildcard)
        // 2. Wildcard with alias: import std::File::* as F; (items contains WildcardWithAlias)
        // 3. Specific symbols: import std::File::{Write, Read}; (items contains symbols)
        // 4. Namespace import: import std::File; (items is empty - import the module itself)
        let has_wildcard = items
            .iter()
            .any(|item| matches!(item, crate::parser::ast::ImportItem::Wildcard));
        let is_namespace_import = items.is_empty();

        // For non-wildcard imports, check if symbols are already imported
        if !has_wildcard && !items.is_empty() {
            // Check if all specific symbols are already imported AND accessible
            // For structs/enums, they must be in symbol_table to be accessible (not just struct_table)
            let all_imported = items.iter().all(|item| match item {
                crate::parser::ast::ImportItem::Symbol(name) => {
                    self.function_table.contains_key(name) || self.symbol_table.contains_key(name)
                }
                crate::parser::ast::ImportItem::SymbolWithAlias(name, _) => {
                    self.function_table.contains_key(name) || self.symbol_table.contains_key(name)
                }
                crate::parser::ast::ImportItem::Wildcard => false,
            });
            if all_imported {
                import_stack.pop();
                return Ok(());
            }
        }

        // For namespace imports, check if already imported
        if is_namespace_import && !path.is_empty() {
            let namespace = path.last().unwrap();
            // Check if any function with this namespace prefix exists
            let namespace_already_imported = self
                .function_table
                .keys()
                .any(|k| k.starts_with(&format!("{}::", namespace)));
            if namespace_already_imported {
                import_stack.pop();
                return Ok(());
            }
        }

        // Check if we've already fully analyzed this module (for wildcard imports)
        let already_analyzed = self.imported_modules.contains_key(&module_key);

        let file_path = self.resolve_module_path(path, &None).ok_or_else(|| {
            let full_path = path.join("::");
            import_stack.pop();
            SemanticError::ModuleNotFound(full_path)
        })?;

        // If this module was already analyzed, we can reuse the cached analysis
        // We only need to parse and analyze once per module file
        let (nodes, imported_analyzer) = if already_analyzed {
            let code = fs::read_to_string(&file_path)
                .map_err(|_| SemanticError::ModuleNotFound(file_path.display().to_string()))?;
            let arena = Bump::new();
            let tokens = crate::lexer::lexer::lex(&code, &arena);

            let mut parser = crate::parser::Parser::new(&tokens);

            let ast = parser
                .parse_program()
                .map_err(|e| SemanticError::ParseErrorInModule {
                    file: file_path.display().to_string(),
                    error: e.to_string(),
                })?;

            if let crate::parser::ast::AstNode::Program(nodes) = ast {
                // Create a temporary analyzer and analyze (will be fast since already done)
                let mut imported_analyzer = SemanticAnalyzer::new(Some(self.project_root.clone()));
                let mut nodes_mut = nodes.clone();
                imported_analyzer.is_main_module = false;
                imported_analyzer.analyze_program_with_stack(&mut nodes_mut, import_stack)?;
                import_stack.pop();
                (nodes, imported_analyzer)
            } else {
                import_stack.pop();
                return Ok(());
            }
        } else {
            // First time analyzing this module
            let code = fs::read_to_string(&file_path).map_err(|_| {
                import_stack.pop();
                SemanticError::ModuleNotFound(file_path.display().to_string())
            })?;

            // Mark this module as being imported
            self.imported_modules.insert(module_key, true);

            let arena = Bump::new();
            let tokens = crate::lexer::lexer::lex(&code, &arena);
            let mut parser = crate::parser::Parser::new(&tokens);
            let ast = parser.parse_program().map_err(|e| {
                import_stack.pop();
                SemanticError::ParseErrorInModule {
                    file: file_path.display().to_string(),
                    error: e.to_string(),
                }
            })?;

            // Recursively analyze the imported AST
            if let crate::parser::ast::AstNode::Program(mut nodes) = ast {
                // Create a temporary analyzer to collect public functions from the imported module
                let mut imported_analyzer = SemanticAnalyzer::new(Some(self.project_root.clone()));

                // Use analyze_program_with_stack for proper two-pass analysis with circular import detection
                // Pass the current import_stack so recursive imports are detected correctly
                imported_analyzer.is_main_module = false;

                if let Err(e) =
                    imported_analyzer.analyze_program_with_stack(&mut nodes, import_stack)
                {
                    return Err(e);
                }

                import_stack.pop();
                (nodes, imported_analyzer)
            } else {
                import_stack.pop();
                return Ok(());
            }
        };

        // Merge public functions from imported module into current function table
        // AND store the function AST nodes for MIR generation
        // Determine which symbols to import
        let should_import_wildcard = items
            .iter()
            .any(|item| matches!(item, crate::parser::ast::ImportItem::Wildcard));
        let specific_imports: Vec<&crate::parser::ast::ImportItem> = items
            .iter()
            .filter(|item| !matches!(item, crate::parser::ast::ImportItem::Wildcard))
            .collect();

        // Get namespace for namespace-qualified imports (import std::File;)
        // Or get alias for namespace alias imports (import std::File as F;)
        let namespace_prefix = if is_namespace_import && !path.is_empty() {
            Some(path.last().unwrap().clone())
        } else {
            None
        };

        // Check if this is a namespace alias import (import module as Alias;)
        let namespace_alias = if !is_namespace_import && specific_imports.len() == 1 {
            if let Some(crate::parser::ast::ImportItem::SymbolWithAlias(module_name, alias)) =
                specific_imports.first()
            {
                // Check if the module name matches the last path component
                if path.last().map(|p| p == module_name).unwrap_or(false) {
                    Some(alias.clone())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Treat namespace alias like namespace import (import all functions)
        let is_namespace_import_or_alias = is_namespace_import || namespace_alias.is_some();

        // Get module name for error messages
        let module_name = path.join("::");

        // First pass: collect all declared types/functions in the module (both public and private)
        // to check if user is trying to import private symbols
        let mut private_functions: Vec<String> = Vec::new();
        let mut private_structs: Vec<String> = Vec::new();
        let mut private_enums: Vec<String> = Vec::new();

        for node in &nodes {
            match node {
                AstNode::FunctionDecl { name, .. } => {
                    // Private functions start with lowercase (camelCase)
                    if !name.chars().next().unwrap_or('A').is_uppercase() {
                        private_functions.push(name.clone());
                    }
                }
                AstNode::StructDecl {
                    name, is_public, ..
                } => {
                    if !*is_public {
                        private_structs.push(name.clone());
                    }
                }
                AstNode::EnumDecl {
                    name, is_public, ..
                } => {
                    if !*is_public {
                        private_enums.push(name.clone());
                    }
                }
                _ => {}
            }
        }

        // NOTE: We allow explicit imports of private (camelCase) functions.
        // If a user explicitly imports a private function by name, they're making
        // a conscious decision to access internal functionality.
        // Only structs and enums maintain strict visibility rules.
        for item in &specific_imports {
            match item {
                crate::parser::ast::ImportItem::Symbol(sym)
                | crate::parser::ast::ImportItem::SymbolWithAlias(sym, _) => {
                    // Allow explicit import of private functions - user knows what they're doing
                    // if private_functions.contains(sym) { ... } - intentionally removed
                    if private_structs.contains(sym) {
                        return Err(SemanticError::PrivateStructImport {
                            name: sym.clone(),
                            module: module_name.clone(),
                        });
                    }
                    if private_enums.contains(sym) {
                        return Err(SemanticError::PrivateEnumImport {
                            name: sym.clone(),
                            module: module_name.clone(),
                        });
                    }
                }
                crate::parser::ast::ImportItem::Wildcard => {}
            }
        }

        for node in nodes {
            match &node {
                // Import functions and methods
                AstNode::FunctionDecl {
                    name,
                    receiver_type,
                    ..
                } => {
                    // For methods, construct the full name (Type::method)
                    let full_name = if let Some(type_name) = receiver_type {
                        format!("{}::{}", type_name, name)
                    } else {
                        name.clone()
                    };

                    // Check if this is a public function (uppercase) OR explicitly imported private function
                    let is_public = name.chars().next().unwrap_or('a').is_uppercase();

                    // For methods, also check if the struct they belong to is being imported
                    let struct_is_imported = if let Some(type_name) = receiver_type {
                        specific_imports.iter().any(|item| match item {
                            crate::parser::ast::ImportItem::Symbol(sym) => sym == type_name,
                            crate::parser::ast::ImportItem::SymbolWithAlias(sym, _) => {
                                sym == type_name
                            }
                            crate::parser::ast::ImportItem::Wildcard => false,
                        })
                    } else {
                        false
                    };

                    let is_explicitly_imported = specific_imports.iter().any(|item| match item {
                        crate::parser::ast::ImportItem::Symbol(sym) => {
                            sym == name || sym == &full_name
                        }
                        crate::parser::ast::ImportItem::SymbolWithAlias(sym, _) => {
                            sym == name || sym == &full_name
                        }
                        crate::parser::ast::ImportItem::Wildcard => false,
                    }) || struct_is_imported;

                    // ALWAYS add ALL functions to imported_functions for MIR generation
                    // This is critical because imported functions may call private helpers from their module
                    // The visibility check only affects what's exposed in function_table (callable by importer)
                    if !self.imported_functions.iter().any(|n| {
                        if let AstNode::FunctionDecl {
                            name: fn_name,
                            receiver_type: rcv,
                            ..
                        } = n
                        {
                            let n_full = if let Some(t) = rcv {
                                format!("{}::{}", t, fn_name)
                            } else {
                                fn_name.clone()
                            };
                            n_full == full_name
                        } else {
                            false
                        }
                    }) {
                        self.imported_functions.push(node.clone());
                    }

                    // Determine what to expose in function_table (only public or explicitly imported)
                    if is_public || is_explicitly_imported {
                        let should_expose = if should_import_wildcard {
                            // Wildcard: expose only public functions (not private ones)
                            is_public
                        } else if is_namespace_import_or_alias {
                            // Namespace import or namespace alias: expose all with namespace prefix
                            true
                        } else if specific_imports.is_empty() {
                            // No specific imports and no wildcard - shouldn't happen but handle it
                            false
                        } else {
                            // Check if this function is in the specific imports list
                            // OR if this is a method of an imported struct
                            struct_is_imported
                                || specific_imports.iter().any(|item| match item {
                                    crate::parser::ast::ImportItem::Symbol(sym) => sym == name,
                                    crate::parser::ast::ImportItem::SymbolWithAlias(sym, _) => {
                                        // Only match if it's not a namespace alias
                                        sym == name && namespace_alias.is_none()
                                    }
                                    crate::parser::ast::ImportItem::Wildcard => false,
                                })
                        };

                        // Only expose to function_table if explicitly requested
                        if should_expose {
                            // Get the alias name for this import if one exists
                            let registered_name =
                                specific_imports.iter().find_map(|item| match item {
                                    crate::parser::ast::ImportItem::SymbolWithAlias(sym, alias)
                                        if sym == name =>
                                    {
                                        Some(alias.clone())
                                    }
                                    _ => None,
                                });

                            // Copy function signature to current function table
                            // Use full_name to get method signatures (Type::method)
                            if let Some((params, ret, err)) =
                                imported_analyzer.function_table.get(&full_name)
                            {
                                // Determine the key to register in function table
                                let fn_table_key = if let Some(alias) = &namespace_alias {
                                    // Namespace alias: register as Alias::FunctionName
                                    format!("{}::{}", alias, name)
                                } else if let Some(ns) = &namespace_prefix {
                                    // Namespace import: register as Namespace::FunctionName
                                    format!("{}::{}", ns, name)
                                } else {
                                    // Regular import: use alias or original name
                                    // For methods, preserve the Type::method format
                                    registered_name.clone().unwrap_or_else(|| full_name.clone())
                                };

                                self.function_table.insert(
                                    fn_table_key.clone(),
                                    (params.clone(), ret.clone(), err.clone()),
                                );

                                // If we have an alias, store the mapping
                                if let Some(alias) = registered_name {
                                    self.function_aliases.insert(alias, full_name.clone());
                                }
                                // For namespace alias, store the mapping from qualified name to original
                                else if namespace_alias.is_some() {
                                    self.function_aliases
                                        .insert(fn_table_key, full_name.clone());
                                }
                                // For namespace imports, store the mapping from qualified name to original
                                else if namespace_prefix.is_some() {
                                    self.function_aliases
                                        .insert(fn_table_key, full_name.clone());
                                }
                            }
                        }
                    }
                }
                // Import structs
                AstNode::StructDecl {
                    name,
                    is_public,
                    fields,
                } => {
                    // Only import public structs (PascalCase - starts with uppercase)
                    if *is_public {
                        let should_import = if should_import_wildcard {
                            // Wildcard: import all public structs
                            true
                        } else if is_namespace_import_or_alias {
                            // Namespace import or namespace alias: import all public structs
                            true
                        } else if specific_imports.is_empty() {
                            false
                        } else {
                            // Check if this struct is in the specific imports list
                            specific_imports.iter().any(|item| match item {
                                crate::parser::ast::ImportItem::Symbol(sym) => sym == name,
                                crate::parser::ast::ImportItem::SymbolWithAlias(sym, _) => {
                                    sym == name
                                }
                                crate::parser::ast::ImportItem::Wildcard => false,
                            })
                        };

                        if should_import {
                            // Copy struct definition to current struct table
                            if let Some(field_types) = imported_analyzer.struct_table.get(name) {
                                self.struct_table.insert(name.clone(), field_types.clone());
                            } else {
                                // Struct not found in imported_analyzer.struct_table - this is a bug
                                // The struct declaration exists but wasn't registered during analysis
                                // This can happen if the imported module had analysis errors
                                // Fall back to building field_types from the AST node directly
                                let mut field_types_fallback: HashMap<String, TypeNode> =
                                    HashMap::new();
                                for field in fields {
                                    field_types_fallback
                                        .insert(field.name.clone(), field.field_type.clone());
                                }
                                self.struct_table
                                    .insert(name.clone(), field_types_fallback.clone());
                            }

                            // The rest of the import logic uses field_types from struct_table
                            let field_types = self.struct_table.get(name).unwrap().clone();

                            // Store field visibility information for imported struct
                            let mut field_visibility: HashMap<String, bool> = HashMap::new();
                            for field in fields {
                                field_visibility.insert(field.name.clone(), field.is_public);
                            }
                            self.struct_field_visibility
                                .insert(name.clone(), field_visibility);

                            // Track this struct as imported (for visibility checking)
                            self.imported_struct_names.insert(name.clone());

                            // CRITICAL: Import methods for this struct from the imported module's method_table
                            if let Some(methods) = imported_analyzer.method_table.get(name) {
                                self.method_table.insert(name.clone(), methods.clone());

                                // Also register methods in function_table with mangled names (Type::method)
                                // This allows static method calls like Server::new() to resolve
                                for (method_name, (params, ret_ty, err_ty)) in methods {
                                    let mangled_name = format!("{}::{}", name, method_name);
                                    if !self.function_table.contains_key(&mangled_name) {
                                        self.function_table.insert(
                                            mangled_name,
                                            (params.clone(), ret_ty.clone(), err_ty.clone()),
                                        );
                                    }
                                }
                            }

                            // CRITICAL: Add to imported_structs for MIR generation
                            // This ensures struct metadata is available in codegen
                            if !self.imported_structs.iter().any(|n| {
                                if let AstNode::StructDecl { name: s_name, .. } = n {
                                    s_name == name
                                } else {
                                    false
                                }
                            }) {
                                self.imported_structs.push(node.clone());
                            }

                            // IMPORTANT: Only add to symbol_table for direct imports, NOT namespace imports
                            // Namespace imports (import std::Http;) should NOT make structs directly accessible
                            // Only specific imports (import std::http::Server;) or wildcard imports should
                            let should_add_to_symbol_table = should_import_wildcard
                                || (!is_namespace_import_or_alias && !specific_imports.is_empty());

                            if should_add_to_symbol_table {
                                self.symbol_table.insert(
                                    name.clone(),
                                    SymbolInfo {
                                        ty: TypeNode::Struct(name.clone(), field_types.clone()),
                                        mutable: false,
                                        is_ref_counted: true,
                                        is_parameter: false,
                                    },
                                );
                            }
                        }
                    }
                }
                // Import enums
                AstNode::EnumDecl {
                    name, is_public, ..
                } => {
                    // Only import public enums (PascalCase - starts with uppercase)
                    if *is_public {
                        let should_import = if should_import_wildcard {
                            // Wildcard: import all public enums
                            true
                        } else if is_namespace_import_or_alias {
                            // Namespace import or namespace alias: import all public enums
                            true
                        } else if specific_imports.is_empty() {
                            false
                        } else {
                            // Check if this enum is in the specific imports list
                            specific_imports.iter().any(|item| match item {
                                crate::parser::ast::ImportItem::Symbol(sym) => sym == name,
                                crate::parser::ast::ImportItem::SymbolWithAlias(sym, _) => {
                                    sym == name
                                }
                                crate::parser::ast::ImportItem::Wildcard => false,
                            })
                        };

                        if should_import {
                            // Copy enum definition to current enum table
                            if let Some(variants) = imported_analyzer.enum_table.get(name) {
                                self.enum_table.insert(name.clone(), variants.clone());
                                // Also copy enum_variant_order for proper printing
                                if let Some(variant_order) =
                                    imported_analyzer.enum_variant_order.get(name)
                                {
                                    self.enum_variant_order
                                        .insert(name.clone(), variant_order.clone());
                                }

                                // IMPORTANT: Only add to symbol_table for direct imports, NOT namespace imports
                                // Namespace imports (import std::Http;) should NOT make enums directly accessible
                                // Only specific imports (import std::http::SomeEnum;) or wildcard imports should
                                let should_add_to_symbol_table = should_import_wildcard
                                    || (!is_namespace_import_or_alias
                                        && !specific_imports.is_empty());

                                if should_add_to_symbol_table {
                                    self.symbol_table.insert(
                                        name.clone(),
                                        SymbolInfo {
                                            ty: TypeNode::Enum(name.clone(), variants.clone()),
                                            mutable: false,
                                            is_ref_counted: true,
                                            is_parameter: false,
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // TRANSITIVE IMPORTS: Import all transitive dependencies from the imported module
        // This ensures that if module A imports from module B, and B imports from C,
        // then A gets C's functions too (just like Rust and Go handle transitive imports)
        for transitive_node in &imported_analyzer.imported_functions {
            if let AstNode::FunctionDecl {
                name: trans_name, ..
            } = transitive_node
            {
                // Add to imported_functions if not already present
                if !self.imported_functions.iter().any(|n| {
                    if let AstNode::FunctionDecl { name: fn_name, .. } = n {
                        fn_name == trans_name
                    } else {
                        false
                    }
                }) {
                    self.imported_functions.push(transitive_node.clone());
                }
                // Add to function_table if not already present
                if !self.function_table.contains_key(trans_name) {
                    if let Some((params, ret, err)) =
                        imported_analyzer.function_table.get(trans_name)
                    {
                        self.function_table.insert(
                            trans_name.clone(),
                            (params.clone(), ret.clone(), err.clone()),
                        );
                    }
                }
            }
        }

        // TRANSITIVE STRUCT IMPORTS: Import all structs from the imported module
        // This ensures that struct types used in function signatures are available
        for transitive_struct in &imported_analyzer.imported_structs {
            if let AstNode::StructDecl {
                name: struct_name,
                fields,
                ..
            } = transitive_struct
            {
                // Add to imported_structs if not already present
                if !self.imported_structs.iter().any(|n| {
                    if let AstNode::StructDecl { name: s_name, .. } = n {
                        s_name == struct_name
                    } else {
                        false
                    }
                }) {
                    self.imported_structs.push(transitive_struct.clone());
                }
                // Add to struct_table if not already present
                if !self.struct_table.contains_key(struct_name) {
                    if let Some(field_types) = imported_analyzer.struct_table.get(struct_name) {
                        self.struct_table
                            .insert(struct_name.clone(), field_types.clone());

                        // Also copy field visibility
                        let mut field_visibility: HashMap<String, bool> = HashMap::new();
                        for field in fields {
                            field_visibility.insert(field.name.clone(), field.is_public);
                        }
                        self.struct_field_visibility
                            .insert(struct_name.clone(), field_visibility);

                        // Mark as imported
                        self.imported_struct_names.insert(struct_name.clone());

                        // Copy methods for this struct
                        if let Some(methods) = imported_analyzer.method_table.get(struct_name) {
                            self.method_table
                                .insert(struct_name.clone(), methods.clone());

                            // Also register methods in function_table with mangled names
                            // This allows static method calls like Server::new() to work
                            for (method_name, (params, ret_ty, err_ty)) in methods {
                                let mangled_name = format!("{}::{}", struct_name, method_name);
                                if !self.function_table.contains_key(&mangled_name) {
                                    self.function_table.insert(
                                        mangled_name,
                                        (params.clone(), ret_ty.clone(), err_ty.clone()),
                                    );
                                }
                            }
                        }

                        // Add to symbol_table for type resolution
                        self.symbol_table.insert(
                            struct_name.clone(),
                            SymbolInfo {
                                ty: TypeNode::Struct(struct_name.clone(), field_types.clone()),
                                mutable: false,
                                is_ref_counted: true,
                                is_parameter: false,
                            },
                        );
                    }
                }
            }
        }

        // Also import structs directly from the imported module's struct_table
        // (these might not be in imported_structs if they were defined in that module)
        for (struct_name, field_types) in &imported_analyzer.struct_table {
            if !self.struct_table.contains_key(struct_name) {
                self.struct_table
                    .insert(struct_name.clone(), field_types.clone());

                // Copy methods for this struct
                if let Some(methods) = imported_analyzer.method_table.get(struct_name) {
                    self.method_table
                        .insert(struct_name.clone(), methods.clone());
                }

                // Add to symbol_table for type resolution
                self.symbol_table.insert(
                    struct_name.clone(),
                    SymbolInfo {
                        ty: TypeNode::Struct(struct_name.clone(), field_types.clone()),
                        mutable: false,
                        is_ref_counted: true,
                        is_parameter: false,
                    },
                );
            }
        }

        // TRANSITIVE ENUM IMPORTS: Import all enums from the imported module
        for (enum_name, variants) in &imported_analyzer.enum_table {
            if !self.enum_table.contains_key(enum_name) {
                self.enum_table.insert(enum_name.clone(), variants.clone());

                // Copy enum_variant_order
                if let Some(variant_order) = imported_analyzer.enum_variant_order.get(enum_name) {
                    self.enum_variant_order
                        .insert(enum_name.clone(), variant_order.clone());
                }

                // Add to symbol_table for type resolution
                self.symbol_table.insert(
                    enum_name.clone(),
                    SymbolInfo {
                        ty: TypeNode::Enum(enum_name.clone(), variants.clone()),
                        mutable: false,
                        is_ref_counted: true,
                        is_parameter: false,
                    },
                );
            }
        }

        // Verify that all requested specific symbols exist
        // Skip verification for namespace alias imports
        if !should_import_wildcard && namespace_alias.is_none() && !specific_imports.is_empty() {
            for item in specific_imports {
                match item {
                    crate::parser::ast::ImportItem::Symbol(sym) => {
                        if !self.function_table.contains_key(sym)
                            && !self.symbol_table.contains_key(sym)
                            && !self.struct_table.contains_key(sym)
                            && !self.enum_table.contains_key(sym)
                        {
                            return Err(SemanticError::UndeclaredFunction(NamedError {
                                name: format!("symbol '{}' not found in module", sym),
                            }));
                        }
                    }
                    crate::parser::ast::ImportItem::SymbolWithAlias(sym, alias) => {
                        // Check using the alias name since that's what we registered
                        if !self.function_table.contains_key(alias)
                            && !self.symbol_table.contains_key(alias)
                            && !self.struct_table.contains_key(sym)
                            && !self.enum_table.contains_key(sym)
                        {
                            return Err(SemanticError::UndeclaredFunction(NamedError {
                                name: format!("symbol '{}' not found in module", sym),
                            }));
                        }
                    }
                    crate::parser::ast::ImportItem::Wildcard => {}
                }
            }
        }
        Ok(())
    }
}

/// Helper function to check if two types are compatible
/// This handles cases where TypeRef("User") should match Struct("User", fields)
pub(crate) fn types_compatible(
    actual: &TypeNode,
    expected: &TypeNode,
    struct_table: &HashMap<String, HashMap<String, TypeNode>>,
    enum_table: &HashMap<String, HashMap<String, Option<TypeNode>>>,
) -> bool {
    // Direct equality check first
    if actual == expected {
        return true;
    }

    // Any type is compatible with everything (used for JSON.parse)
    if matches!(actual, TypeNode::Any) || matches!(expected, TypeNode::Any) {
        return true;
    }

    // Error type is compatible with any error type for generic error handling
    // This allows functions with ! Error to accept any specific error type
    if matches!(expected, TypeNode::Error) {
        // Error type accepts any type as error (Str, Struct, Enum, etc.)
        return true;
    }
    if matches!(actual, TypeNode::Error) {
        // If actual is Error, it can be used where any specific error is expected
        return true;
    }

    // Handle TypeRef resolution
    match (actual, expected) {
        // Case 1: actual is Struct, expected is TypeRef
        (TypeNode::Struct(actual_name, _), TypeNode::TypeRef(expected_name)) => {
            actual_name == expected_name
        }
        // Case 2: actual is TypeRef, expected is Struct
        (TypeNode::TypeRef(actual_name), TypeNode::Struct(expected_name, _)) => {
            actual_name == expected_name
        }
        // Case 3: both are TypeRefs with same name
        (TypeNode::TypeRef(actual_name), TypeNode::TypeRef(expected_name)) => {
            actual_name == expected_name
        }
        // Case 4: actual is Enum, expected is TypeRef
        (TypeNode::Enum(actual_name, _), TypeNode::TypeRef(expected_name)) => {
            actual_name == expected_name
        }
        // Case 5: actual is TypeRef, expected is Enum
        (TypeNode::TypeRef(actual_name), TypeNode::Enum(expected_name, _)) => {
            actual_name == expected_name
        }
        // Case 6: both are Structs with same name (even if fields differ - structural equality)
        (TypeNode::Struct(actual_name, _), TypeNode::Struct(expected_name, _)) => {
            actual_name == expected_name
        }
        // Case 7: both are Enums with same name
        (TypeNode::Enum(actual_name, _), TypeNode::Enum(expected_name, _)) => {
            actual_name == expected_name
        }
        // Case 8: Array types
        (TypeNode::Array(actual_elem), TypeNode::Array(expected_elem)) => {
            types_compatible(actual_elem, expected_elem, struct_table, enum_table)
        }
        // Case 9: Map types
        (TypeNode::Map(ak, av), TypeNode::Map(ek, ev)) => {
            types_compatible(ak, ek, struct_table, enum_table)
                && types_compatible(av, ev, struct_table, enum_table)
        }
        // Case 10: Tuple types
        (TypeNode::Tuple(actual_types), TypeNode::Tuple(expected_types)) => {
            if actual_types.len() != expected_types.len() {
                return false;
            }
            actual_types
                .iter()
                .zip(expected_types.iter())
                .all(|(a, e)| types_compatible(a, e, struct_table, enum_table))
        }
        // Case 11: Result types
        (TypeNode::Result(aok, aerr), TypeNode::Result(eok, eerr)) => {
            types_compatible(aok, eok, struct_table, enum_table)
                && types_compatible(aerr, eerr, struct_table, enum_table)
        }
        // Case 12: Optional types
        (TypeNode::Optional(actual_inner), TypeNode::Optional(expected_inner)) => {
            types_compatible(actual_inner, expected_inner, struct_table, enum_table)
        }
        _ => false,
    }
}
