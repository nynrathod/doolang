use crate::analyzer::types::{NamedError, SemanticError};
use crate::limits::{
    ANALYZER_MAX_FUNCTION_DEPTH, ANALYZER_MAX_LOOP_DEPTH, ANALYZER_MAX_SCOPE_DEPTH,
};
use crate::parser::ast::{AstNode, TypeNode};
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
    pub(crate) function_table: HashMap<String, (Vec<TypeNode>, TypeNode)>, // Function signatures

    pub(crate) outer_symbol_table: Option<HashMap<String, SymbolInfo>>, // For nested scopes
    pub(crate) project_root: PathBuf, // Root directory for module resolution
    pub(crate) imported_modules: HashMap<String, bool>, // Track imported modules to prevent circular imports
    pub imported_functions: Vec<AstNode>, // Store imported function AST nodes for MIR generation
    pub function_aliases: HashMap<String, String>, // Maps alias names to original function names
    pub loop_depth: usize,                // Track loop nesting for break/continue error handling
    pub scope_stack: Vec<HashMap<String, SymbolInfo>>, // Scope stack for block scoping
    pub function_depth: usize,            // Track function nesting for return statement validation
    pub scope_sizes_stack: Vec<usize>,    // Track symbol table size at each scope level
    pub collected_errors: Vec<SemanticError>, // Collect all errors for reporting
    pub is_main_module: bool,             // Track if analyzing main program or imported module
    pub type_inference_depth: RefCell<usize>, // Track type inference recursion depth using interior mutability
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

        function_table.insert("print".to_string(), (vec![], TypeNode::Void));
        function_table.insert("println".to_string(), (vec![], TypeNode::Void));
        function_table.insert(
            "panic".to_string(),
            (vec![TypeNode::String], TypeNode::Void),
        );
        function_table.insert("typeOf".to_string(), (vec![], TypeNode::String));

        Self {
            symbol_table: HashMap::new(),
            function_table,
            outer_symbol_table: None,
            project_root,
            imported_modules: HashMap::new(),
            imported_functions: Vec::new(),
            function_aliases: HashMap::new(),
            loop_depth: 0,
            scope_stack: Vec::new(),
            function_depth: 0,
            scope_sizes_stack: Vec::new(),
            collected_errors: Vec::new(),
            is_main_module: true,
            type_inference_depth: RefCell::new(0),
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
        // FIRST PASS: Process imports and register all function signatures
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
                // Register local function signatures
                AstNode::FunctionDecl {
                    name,
                    params,
                    return_type,
                    ..
                } => {
                    // Check if function already defined
                    if self.function_table.contains_key(name) {
                        self.collected_errors
                            .push(SemanticError::FunctionRedeclaration(NamedError {
                                name: name.to_string(),
                            }));
                        continue;
                    }

                    // Collect parameter types
                    let param_types: Vec<TypeNode> = params
                        .iter()
                        .map(|(_, t)| t.clone().unwrap_or(TypeNode::Int))
                        .collect();

                    // Register function signature (all functions, not just public ones)
                    self.function_table.insert(
                        name.to_string(),
                        (param_types, return_type.clone().unwrap_or(TypeNode::Void)),
                    );
                }
                _ => {} // Skip other nodes in first pass
            }
        }

        // SECOND PASS: Analyze all nodes (including function bodies)
        // Skip imports as they're already processed
        for node in nodes {
            if !matches!(node, AstNode::Import { .. }) {
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
                body,
            } => self.analyze_functional_decl(name, visibility, params, return_type, body),
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

                    let (param_types, _return_type) =
                        self.function_table.get(func_name).ok_or_else(|| {
                            SemanticError::UndeclaredFunction(NamedError {
                                name: func_name.clone(),
                            })
                        })?;

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
                            let arg_type = self.infer_type(arg)?;
                            if arg_type != *expected_type {
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
                // For "std::Math::Abs", we want std/Math.doo
                if path.len() >= 2 {
                    let mut stdlib_file = resolver.stdlib_path().to_path_buf();
                    // Use the second element as the file name (Math, String, Array, etc.)
                    stdlib_file.push(&path[1]);
                    stdlib_file.set_extension("doo");

                    if stdlib_file.exists() {
                        return Some(stdlib_file);
                    }
                }
            }
        }

        // Otherwise, try project-relative path
        let mut buf = self.project_root.clone();

        // For imports like http::Client::Fetchuser, we want http/Client.doo
        // The path will be ["http", "Client"]
        for part in path {
            buf.push(part);
        }

        // Add .doo extension
        buf.set_extension("doo");

        if buf.exists() {
            Some(buf)
        } else {
            None
        }
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
            if cfg!(debug_assertions) {
                println!("[ERROR] Circular import detected: {}", cycle.join(" -> "));
            }
            return Err(SemanticError::CircularImport { cycle });
        }
        import_stack.push(module_key.clone());

        // For non-wildcard imports, check if symbols are already imported
        let has_wildcard = items
            .iter()
            .any(|item| matches!(item, crate::parser::ast::ImportItem::Wildcard));
        if !has_wildcard && !items.is_empty() {
            // Check if all specific symbols are already imported
            let all_imported = items.iter().all(|item| match item {
                crate::parser::ast::ImportItem::Symbol(name) => {
                    self.function_table.contains_key(name)
                }
                crate::parser::ast::ImportItem::SymbolWithAlias(name, _) => {
                    self.function_table.contains_key(name)
                }
                crate::parser::ast::ImportItem::Wildcard => false,
            });
            if all_imported {
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
                imported_analyzer.analyze_program_with_stack(&mut nodes, import_stack)?;

                import_stack.pop();
                (nodes, imported_analyzer)
            } else {
                println!(
                    "[WARNING] [import_module] Parsed AST from {:?} is not a Program variant",
                    file_path
                );
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

        for node in nodes {
            if let AstNode::FunctionDecl { name, .. } = &node {
                // Only import functions that start with uppercase (public convention)
                if name.chars().next().unwrap_or('a').is_uppercase() {
                    let should_import = if should_import_wildcard {
                        // Wildcard: import all public functions
                        true
                    } else if specific_imports.is_empty() {
                        // No specific imports and no wildcard - shouldn't happen but handle it
                        false
                    } else {
                        // Check if this function is in the specific imports list
                        specific_imports.iter().any(|item| match item {
                            crate::parser::ast::ImportItem::Symbol(sym) => sym == name,
                            crate::parser::ast::ImportItem::SymbolWithAlias(sym, _) => sym == name,
                            crate::parser::ast::ImportItem::Wildcard => false,
                        })
                    };

                    if should_import {
                        // Check if already imported
                        if !self.imported_functions.iter().any(|n| {
                            if let AstNode::FunctionDecl { name: fn_name, .. } = n {
                                fn_name == name
                            } else {
                                false
                            }
                        }) {
                            self.imported_functions.push(node.clone());
                        }

                        // Get the alias name for this import if one exists
                        let registered_name = specific_imports.iter().find_map(|item| match item {
                            crate::parser::ast::ImportItem::SymbolWithAlias(sym, alias)
                                if sym == name =>
                            {
                                Some(alias.clone())
                            }
                            _ => None,
                        });

                        // Copy function signature to current function table
                        if let Some((params, ret)) = imported_analyzer.function_table.get(name) {
                            let fn_table_key =
                                registered_name.clone().unwrap_or_else(|| name.clone());
                            self.function_table
                                .insert(fn_table_key.clone(), (params.clone(), ret.clone()));

                            // If we have an alias, store the mapping
                            if let Some(alias) = registered_name {
                                self.function_aliases.insert(alias, name.clone());
                            }
                        }
                    }
                }
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
                    if let Some((params, ret)) = imported_analyzer.function_table.get(trans_name) {
                        self.function_table
                            .insert(trans_name.clone(), (params.clone(), ret.clone()));
                    }
                }
            }
        }

        // Verify that all requested specific symbols exist
        if !should_import_wildcard && !specific_imports.is_empty() {
            for item in specific_imports {
                match item {
                    crate::parser::ast::ImportItem::Symbol(sym) => {
                        if !self.function_table.contains_key(sym)
                            && !self.symbol_table.contains_key(sym)
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
