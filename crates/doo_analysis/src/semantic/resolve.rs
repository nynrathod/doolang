//! Name Resolver and Import Analysis
//!
//! Resolves identifiers to their definitions and detects circular imports.
//!
//! ## Circular Import Detection
//!
//! Uses DFS to build an import dependency graph and detect cycles.
//! When a cycle is detected, returns a deterministic error with the cycle path.
//!
//! ## Cross-Module Symbol Resolution
//!
//! Supports importing symbols across module boundaries with:
//! - Single symbol imports: `import std::Math::Abs;`
//! - Multiple symbols: `import std::Math::{Min, Max};`
//! - Aliased symbols: `import std::Math::{Sqrt as Sq};`
//! - Wildcard imports: `import std::Array::*;`
//! - Namespace imports: `import std::File;` (use as `File::Read`)
//! - Namespace aliases: `import std::Array as A;` (use as `A::Sum`)
//!
//! ## Example
//!
//! ```text
//! A imports B
//! B imports C
//! C imports A  <-- circular!
//! ```
//!
//! Error: circular import detected: A -> B -> C -> A

use doo_core::Span;
use std::collections::{HashMap, HashSet};

/// Resolution error.
#[derive(Debug, Clone)]
pub struct ResolveError {
    pub message: String,
    pub span: Span,
}

/// Import edge in the dependency graph.
#[derive(Debug, Clone)]
pub struct ImportEdge {
    /// The module being imported.
    pub target: String,
    /// Location of the import statement.
    pub span: Span,
}

/// Import dependency graph for cycle detection.
#[derive(Debug, Default)]
pub struct ImportGraph {
    /// Adjacency list: module -> list of imported modules.
    edges: HashMap<String, Vec<ImportEdge>>,
    /// All known modules.
    modules: HashSet<String>,
}

impl ImportGraph {
    /// Create a new empty import graph.
    pub fn new() -> Self {
        Self {
            edges: HashMap::new(),
            modules: HashSet::new(),
        }
    }

    /// Add a module to the graph.
    pub fn add_module(&mut self, module: &str) {
        self.modules.insert(module.to_string());
        self.edges.entry(module.to_string()).or_default();
    }

    /// Add an import edge from `from` module to `to` module.
    pub fn add_import(&mut self, from: &str, to: &str, span: Span) {
        self.modules.insert(from.to_string());
        self.modules.insert(to.to_string());
        self.edges
            .entry(from.to_string())
            .or_default()
            .push(ImportEdge {
                target: to.to_string(),
                span,
            });
    }

    /// Get all modules that a given module imports.
    pub fn get_imports(&self, module: &str) -> &[ImportEdge] {
        self.edges.get(module).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Check if a module exists in the graph.
    pub fn has_module(&self, module: &str) -> bool {
        self.modules.contains(module)
    }

    /// Get all modules in the graph.
    pub fn modules(&self) -> impl Iterator<Item = &String> {
        self.modules.iter()
    }
}

/// Circular import error with the cycle path.
#[derive(Debug, Clone)]
pub struct CircularImportError {
    /// The cycle as a list of module names (first == last).
    pub cycle: Vec<String>,
    /// Span of the import that closes the cycle.
    pub span: Span,
}

impl CircularImportError {
    /// Format the cycle as a readable string.
    pub fn format_cycle(&self) -> String {
        self.cycle.join(" -> ")
    }
}

/// Detects circular imports in an import dependency graph using DFS.
pub struct CircularImportDetector {
    /// The import graph to analyze.
    graph: ImportGraph,
}

impl CircularImportDetector {
    /// Create a new detector with the given import graph.
    pub fn new(graph: ImportGraph) -> Self {
        Self { graph }
    }

    /// Detect cycles in the import graph.
    ///
    /// Returns `Ok(())` if no cycles are found, or `Err(CircularImportError)` with
    /// the first cycle found (deterministic order based on module names).
    pub fn detect_cycles(&self) -> Result<(), CircularImportError> {
        // Track visited nodes globally (gray = in current path, black = fully processed)
        let mut visited: HashSet<&str> = HashSet::new();
        let mut in_stack: HashSet<&str> = HashSet::new();

        // Process modules in sorted order for deterministic results
        let mut modules: Vec<&String> = self.graph.modules().collect();
        modules.sort();

        for module in modules {
            if !visited.contains(module.as_str()) {
                let mut path = Vec::new();
                if let Some(error) = self.dfs_detect(module, &mut visited, &mut in_stack, &mut path)
                {
                    return Err(error);
                }
            }
        }

        Ok(())
    }

    /// Detect cycles starting from a specific module.
    ///
    /// This is useful when you want to check for cycles involving a specific module
    /// (e.g., when processing an import statement).
    pub fn detect_cycle_from(&self, start_module: &str) -> Result<(), CircularImportError> {
        let mut visited: HashSet<&str> = HashSet::new();
        let mut in_stack: HashSet<&str> = HashSet::new();
        let mut path = Vec::new();

        if let Some(error) = self.dfs_detect(start_module, &mut visited, &mut in_stack, &mut path) {
            return Err(error);
        }

        Ok(())
    }

    /// DFS helper for cycle detection.
    ///
    /// Returns `Some(CircularImportError)` if a cycle is found, `None` otherwise.
    fn dfs_detect<'a>(
        &'a self,
        module: &'a str,
        visited: &mut HashSet<&'a str>,
        in_stack: &mut HashSet<&'a str>,
        path: &mut Vec<&'a str>,
    ) -> Option<CircularImportError> {
        // Mark as visited and add to current path
        visited.insert(module);
        in_stack.insert(module);
        path.push(module);

        // Get imports, sorted for deterministic ordering
        let imports = self.graph.get_imports(module);
        let mut sorted_imports: Vec<&ImportEdge> = imports.iter().collect();
        sorted_imports.sort_by(|a, b| a.target.cmp(&b.target));

        for edge in sorted_imports {
            let target = edge.target.as_str();

            if !visited.contains(target) {
                // Not yet visited - recurse
                if let Some(error) = self.dfs_detect(target, visited, in_stack, path) {
                    return Some(error);
                }
            } else if in_stack.contains(target) {
                // Found a cycle! Build the cycle path.
                let cycle_start_idx = path.iter().position(|&m| m == target).unwrap();
                let mut cycle: Vec<String> = path[cycle_start_idx..]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                cycle.push(target.to_string()); // Close the cycle

                return Some(CircularImportError {
                    cycle,
                    span: edge.span,
                });
            }
            // If visited but not in stack, it's already fully processed - no cycle through this path
        }

        // Done processing this node - remove from current path
        in_stack.remove(module);
        path.pop();

        None
    }
}

/// Import stack for tracking the current import chain during analysis.
///
/// This is used during incremental analysis to detect cycles as imports are processed.
#[derive(Debug, Default)]
pub struct ImportStack {
    /// Stack of module names currently being imported.
    stack: Vec<String>,
    /// Set for O(1) lookup.
    set: HashSet<String>,
}

impl ImportStack {
    /// Create a new empty import stack.
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            set: HashSet::new(),
        }
    }

    /// Push a module onto the import stack.
    ///
    /// Returns `Err(CircularImportError)` if the module is already in the stack (cycle detected).
    pub fn push(&mut self, module: &str, span: Span) -> Result<(), CircularImportError> {
        if self.set.contains(module) {
            // Cycle detected - build the cycle path
            let cycle_start_idx = self.stack.iter().position(|m| m == module).unwrap();
            let mut cycle: Vec<String> = self.stack[cycle_start_idx..].to_vec();
            cycle.push(module.to_string());

            return Err(CircularImportError { cycle, span });
        }

        self.stack.push(module.to_string());
        self.set.insert(module.to_string());
        Ok(())
    }

    /// Pop a module from the import stack.
    pub fn pop(&mut self) {
        if let Some(module) = self.stack.pop() {
            self.set.remove(&module);
        }
    }

    /// Check if a module is currently in the import stack.
    pub fn contains(&self, module: &str) -> bool {
        self.set.contains(module)
    }

    /// Get the current import path as a slice.
    pub fn path(&self) -> &[String] {
        &self.stack
    }

    /// Check if the stack is empty.
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Get the current depth of the import stack.
    pub fn depth(&self) -> usize {
        self.stack.len()
    }
}

/// Name resolver pass.
pub struct NameResolver {
    /// Import graph for cycle detection.
    import_graph: ImportGraph,
    /// Current import stack for incremental cycle detection.
    import_stack: ImportStack,
    /// Current module being analyzed.
    current_module: String,
}

impl NameResolver {
    /// Create a new name resolver.
    pub fn new() -> Self {
        Self {
            import_graph: ImportGraph::new(),
            import_stack: ImportStack::new(),
            current_module: String::new(),
        }
    }

    /// Set the current module being analyzed.
    pub fn set_current_module(&mut self, module: &str) {
        self.current_module = module.to_string();
        self.import_graph.add_module(module);
    }

    /// Get the current module.
    pub fn current_module(&self) -> &str {
        &self.current_module
    }

    /// Record an import from the current module to another module.
    ///
    /// Returns `Err(CircularImportError)` if this import creates a cycle.
    pub fn add_import(
        &mut self,
        target_module: &str,
        span: Span,
    ) -> Result<(), CircularImportError> {
        // Add to the import graph
        self.import_graph
            .add_import(&self.current_module, target_module, span);

        // Check for cycle using import stack
        self.import_stack.push(target_module, span)?;

        Ok(())
    }

    /// Signal that we're done processing an imported module.
    pub fn finish_import(&mut self) {
        self.import_stack.pop();
    }

    /// Begin processing a module (push onto import stack).
    pub fn begin_module(&mut self, module: &str, span: Span) -> Result<(), CircularImportError> {
        self.set_current_module(module);
        self.import_stack.push(module, span)
    }

    /// End processing a module (pop from import stack).
    pub fn end_module(&mut self) {
        self.import_stack.pop();
    }

    /// Run full cycle detection on the import graph.
    ///
    /// This is useful after all imports have been recorded to find any cycles.
    pub fn detect_all_cycles(&self) -> Result<(), CircularImportError> {
        let detector = CircularImportDetector::new(self.import_graph.clone());
        detector.detect_cycles()
    }

    /// Get access to the import graph for inspection.
    pub fn import_graph(&self) -> &ImportGraph {
        &self.import_graph
    }

    /// Get access to the import stack for inspection.
    pub fn import_stack(&self) -> &ImportStack {
        &self.import_stack
    }
}

impl Default for NameResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ImportGraph {
    fn clone(&self) -> Self {
        Self {
            edges: self.edges.clone(),
            modules: self.modules.clone(),
        }
    }
}

// ============================================================================
// Cross-Module Symbol Resolution
// ============================================================================

/// Types of imports supported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportKind {
    /// Single symbol: `import std::Math::Abs;`
    Symbol(String),
    /// Aliased symbol: `import std::Math::Sqrt as Sq;`
    SymbolAlias { name: String, alias: String },
    /// Wildcard: `import std::Array::*;`
    Wildcard,
    /// Namespace import: `import std::File;` (use as `File::Read`)
    Namespace,
    /// Namespace alias: `import std::Array as A;`
    NamespaceAlias { alias: String },
}

/// A resolved symbol with its origin.
#[derive(Debug, Clone)]
pub struct ResolvedSymbol {
    /// The name used to access this symbol in the current module.
    pub local_name: String,
    /// The original name in the source module.
    pub original_name: String,
    /// The module path where the symbol is defined.
    pub module_path: String,
    /// Whether this symbol is public.
    pub is_public: bool,
    /// The span of the import statement.
    pub span: Span,
}

/// An imported module with its symbols and import configuration.
#[derive(Debug, Clone)]
pub struct ImportedModule {
    /// Full module path (e.g., "std::Math").
    pub path: Vec<String>,
    /// Import kind.
    pub kind: ImportKind,
    /// Resolved symbols from this import.
    pub symbols: Vec<ResolvedSymbol>,
    /// Span of the import statement.
    pub span: Span,
}

/// Symbol table for cross-module resolution.
#[derive(Debug, Default)]
pub struct SymbolTable {
    /// Symbols defined in the current module.
    local_symbols: HashMap<String, SymbolDef>,
    /// Imported symbols by their local name.
    imported_symbols: HashMap<String, ResolvedSymbol>,
    /// Namespace imports: namespace -> module path.
    namespaces: HashMap<String, String>,
    /// Wildcard imports: module path -> span.
    wildcards: Vec<(String, Span)>,
    /// All imported modules.
    modules: Vec<ImportedModule>,
}

/// A symbol definition.
#[derive(Debug, Clone)]
pub struct SymbolDef {
    /// Symbol name.
    pub name: String,
    /// Whether this symbol is public.
    pub is_public: bool,
    /// Symbol kind.
    pub kind: SymbolKindDef,
    /// Span where defined.
    pub span: Span,
}

/// Kinds of symbols that can be defined/imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKindDef {
    Function,
    Struct,
    Enum,
    Const,
    Static,
    TypeAlias,
}

impl SymbolTable {
    /// Create a new empty symbol table.
    pub fn new() -> Self {
        Self {
            local_symbols: HashMap::new(),
            imported_symbols: HashMap::new(),
            namespaces: HashMap::new(),
            wildcards: Vec::new(),
            modules: Vec::new(),
        }
    }

    /// Define a local symbol.
    pub fn define(&mut self, name: &str, is_public: bool, kind: SymbolKindDef, span: Span) {
        self.local_symbols.insert(
            name.to_string(),
            SymbolDef {
                name: name.to_string(),
                is_public,
                kind,
                span,
            },
        );
    }

    /// Check if a symbol is defined locally.
    pub fn is_defined(&self, name: &str) -> bool {
        self.local_symbols.contains_key(name)
    }

    /// Get a local symbol definition.
    pub fn get_local(&self, name: &str) -> Option<&SymbolDef> {
        self.local_symbols.get(name)
    }

    /// Get all public symbols (for exporting).
    pub fn public_symbols(&self) -> impl Iterator<Item = &SymbolDef> {
        self.local_symbols.values().filter(|s| s.is_public)
    }

    /// Register an import.
    pub fn register_import(&mut self, module: ImportedModule) {
        // Process symbols based on import kind
        match &module.kind {
            ImportKind::Symbol(name) => {
                // Find the symbol in the module's exported symbols
                if let Some(sym) = module.symbols.iter().find(|s| s.original_name == *name) {
                    self.imported_symbols
                        .insert(sym.local_name.clone(), sym.clone());
                }
            }
            ImportKind::SymbolAlias { name, alias } => {
                if let Some(sym) = module.symbols.iter().find(|s| s.original_name == *name) {
                    let mut aliased = sym.clone();
                    aliased.local_name = alias.clone();
                    self.imported_symbols.insert(alias.clone(), aliased);
                }
            }
            ImportKind::Wildcard => {
                // Add all public symbols from the module
                for sym in &module.symbols {
                    if sym.is_public {
                        self.imported_symbols
                            .insert(sym.local_name.clone(), sym.clone());
                    }
                }
                self.wildcards.push((module.path.join("::"), module.span));
            }
            ImportKind::Namespace => {
                // Register namespace for qualified access
                if let Some(ns) = module.path.last() {
                    self.namespaces.insert(ns.clone(), module.path.join("::"));
                }
                // Also register all symbols with namespace prefix
                for sym in &module.symbols {
                    if sym.is_public {
                        let qualified_name = format!(
                            "{}::{}",
                            module.path.last().unwrap_or(&String::new()),
                            sym.original_name
                        );
                        let mut qualified_sym = sym.clone();
                        qualified_sym.local_name = qualified_name.clone();
                        self.imported_symbols.insert(qualified_name, qualified_sym);
                    }
                }
            }
            ImportKind::NamespaceAlias { alias } => {
                // Register alias for qualified access
                self.namespaces
                    .insert(alias.clone(), module.path.join("::"));
                // Register all symbols with alias prefix
                for sym in &module.symbols {
                    if sym.is_public {
                        let qualified_name = format!("{}::{}", alias, sym.original_name);
                        let mut qualified_sym = sym.clone();
                        qualified_sym.local_name = qualified_name.clone();
                        self.imported_symbols.insert(qualified_name, qualified_sym);
                    }
                }
            }
        }

        self.modules.push(module);
    }

    /// Resolve a symbol by name.
    ///
    /// Returns the resolved symbol if found, or None if not found.
    pub fn resolve(&self, name: &str) -> Option<&ResolvedSymbol> {
        // First check local definitions (they shadow imports)
        // Local symbols don't have ResolvedSymbol representation
        // so we only check imports here

        // Check direct imports
        self.imported_symbols.get(name)
    }

    /// Resolve a qualified name (e.g., "File::Read").
    pub fn resolve_qualified(&self, namespace: &str, name: &str) -> Option<&ResolvedSymbol> {
        let qualified = format!("{}::{}", namespace, name);
        self.imported_symbols.get(&qualified)
    }

    /// Check if a namespace exists.
    pub fn has_namespace(&self, namespace: &str) -> bool {
        self.namespaces.contains_key(namespace)
    }

    /// Get the module path for a namespace.
    pub fn get_namespace_path(&self, namespace: &str) -> Option<&String> {
        self.namespaces.get(namespace)
    }

    /// Get all imported symbols.
    pub fn imported_symbols(&self) -> impl Iterator<Item = &ResolvedSymbol> {
        self.imported_symbols.values()
    }

    /// Get all wildcard imports.
    pub fn wildcards(&self) -> &[(String, Span)] {
        &self.wildcards
    }

    /// Get all imported modules.
    pub fn modules(&self) -> &[ImportedModule] {
        &self.modules
    }
}

/// Cross-module symbol resolver.
///
/// Handles resolution of symbols across module boundaries, supporting:
/// - Direct imports
/// - Aliased imports
/// - Wildcard imports
/// - Namespace imports
/// - Namespace aliases
#[derive(Debug, Default)]
pub struct CrossModuleResolver {
    /// Symbol tables for each module.
    module_tables: HashMap<String, SymbolTable>,
    /// Current module being processed.
    current_module: String,
}

impl CrossModuleResolver {
    /// Create a new cross-module resolver.
    pub fn new() -> Self {
        Self {
            module_tables: HashMap::new(),
            current_module: String::new(),
        }
    }

    /// Set the current module being processed.
    pub fn set_current_module(&mut self, module: &str) {
        self.current_module = module.to_string();
        self.module_tables
            .entry(module.to_string())
            .or_insert_with(SymbolTable::new);
    }

    /// Get the current module's symbol table.
    pub fn current_table(&self) -> Option<&SymbolTable> {
        self.module_tables.get(&self.current_module)
    }

    /// Get the current module's symbol table mutably.
    pub fn current_table_mut(&mut self) -> &mut SymbolTable {
        self.module_tables
            .entry(self.current_module.clone())
            .or_insert_with(SymbolTable::new)
    }

    /// Get a module's symbol table.
    pub fn get_table(&self, module: &str) -> Option<&SymbolTable> {
        self.module_tables.get(module)
    }

    /// Define a symbol in the current module.
    pub fn define(&mut self, name: &str, is_public: bool, kind: SymbolKindDef, span: Span) {
        self.current_table_mut().define(name, is_public, kind, span);
    }

    /// Process an import into the current module.
    ///
    /// Returns a list of errors for any symbols that couldn't be resolved.
    pub fn process_import(
        &mut self,
        path: &[String],
        items: &[ImportItemKind],
        span: Span,
    ) -> Result<(), Vec<ResolveError>> {
        let module_path = path.join("::");
        let mut errors = Vec::new();

        // Determine import kind from items
        let kind = Self::determine_import_kind(items, path);

        // Get the source module's exported symbols
        let source_symbols = self.get_exported_symbols(&module_path);

        // Build resolved symbols based on import kind
        let mut resolved_symbols = Vec::new();

        match &kind {
            ImportKind::Symbol(name) => {
                if let Some(sym) = source_symbols.iter().find(|s| s.name == *name) {
                    resolved_symbols.push(ResolvedSymbol {
                        local_name: name.clone(),
                        original_name: name.clone(),
                        module_path: module_path.clone(),
                        is_public: sym.is_public,
                        span,
                    });
                } else {
                    errors.push(ResolveError {
                        message: format!("symbol '{}' not found in module '{}'", name, module_path),
                        span,
                    });
                }
            }
            ImportKind::SymbolAlias { name, alias } => {
                if let Some(sym) = source_symbols.iter().find(|s| s.name == *name) {
                    resolved_symbols.push(ResolvedSymbol {
                        local_name: alias.clone(),
                        original_name: name.clone(),
                        module_path: module_path.clone(),
                        is_public: sym.is_public,
                        span,
                    });
                } else {
                    errors.push(ResolveError {
                        message: format!("symbol '{}' not found in module '{}'", name, module_path),
                        span,
                    });
                }
            }
            ImportKind::Wildcard => {
                // Import all public symbols
                for sym in &source_symbols {
                    if sym.is_public {
                        resolved_symbols.push(ResolvedSymbol {
                            local_name: sym.name.clone(),
                            original_name: sym.name.clone(),
                            module_path: module_path.clone(),
                            is_public: true,
                            span,
                        });
                    }
                }
            }
            ImportKind::Namespace | ImportKind::NamespaceAlias { .. } => {
                // Import all public symbols (they'll be accessed via namespace)
                for sym in &source_symbols {
                    if sym.is_public {
                        resolved_symbols.push(ResolvedSymbol {
                            local_name: sym.name.clone(),
                            original_name: sym.name.clone(),
                            module_path: module_path.clone(),
                            is_public: true,
                            span,
                        });
                    }
                }
            }
        }

        // Register the import
        let imported_module = ImportedModule {
            path: path.to_vec(),
            kind,
            symbols: resolved_symbols,
            span,
        };

        self.current_table_mut().register_import(imported_module);

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Determine import kind from import items.
    fn determine_import_kind(items: &[ImportItemKind], path: &[String]) -> ImportKind {
        if items.is_empty() {
            // No items - namespace import
            return ImportKind::Namespace;
        }

        if items.len() == 1 {
            match &items[0] {
                ImportItemKind::Symbol(name) => {
                    // Check if this is a module alias (name matches last path component)
                    if path.last().map(|p| p == name).unwrap_or(false) {
                        return ImportKind::Namespace;
                    }
                    return ImportKind::Symbol(name.clone());
                }
                ImportItemKind::Alias { name, alias } => {
                    // Check if this is a namespace alias
                    if path.last().map(|p| p == name).unwrap_or(false) {
                        return ImportKind::NamespaceAlias {
                            alias: alias.clone(),
                        };
                    }
                    return ImportKind::SymbolAlias {
                        name: name.clone(),
                        alias: alias.clone(),
                    };
                }
                ImportItemKind::Wildcard => return ImportKind::Wildcard,
            }
        }

        // Multiple items - treat as multiple symbol imports
        // For now, process first one (full implementation would handle all)
        if let Some(item) = items.first() {
            match item {
                ImportItemKind::Symbol(name) => ImportKind::Symbol(name.clone()),
                ImportItemKind::Alias { name, alias } => ImportKind::SymbolAlias {
                    name: name.clone(),
                    alias: alias.clone(),
                },
                ImportItemKind::Wildcard => ImportKind::Wildcard,
            }
        } else {
            ImportKind::Namespace
        }
    }

    /// Get exported symbols from a module.
    fn get_exported_symbols(&self, module: &str) -> Vec<SymbolDef> {
        if let Some(table) = self.module_tables.get(module) {
            table.public_symbols().cloned().collect()
        } else {
            Vec::new()
        }
    }

    /// Resolve a simple identifier in the current module.
    pub fn resolve_simple(&self, name: &str) -> Option<ResolvedSymbol> {
        let table = self.current_table()?;

        // First check local definitions
        if let Some(def) = table.get_local(name) {
            return Some(ResolvedSymbol {
                local_name: name.to_string(),
                original_name: name.to_string(),
                module_path: self.current_module.clone(),
                is_public: def.is_public,
                span: def.span,
            });
        }

        // Then check imports
        table.resolve(name).cloned()
    }

    /// Resolve a qualified identifier (e.g., "File::Read").
    pub fn resolve_qualified(&self, parts: &[String]) -> Option<ResolvedSymbol> {
        if parts.len() < 2 {
            return None;
        }

        let namespace = &parts[0];
        let name = &parts[1..].join("::");

        let table = self.current_table()?;
        table.resolve_qualified(namespace, name).cloned()
    }
}

/// Import item for cross-module resolution.
#[derive(Debug, Clone)]
pub enum ImportItemKind {
    /// Single symbol: `Foo`
    Symbol(String),
    /// Aliased symbol: `Foo as Bar`
    Alias { name: String, alias: String },
    /// Wildcard: `*`
    Wildcard,
}

// ============================================================================
// Method Resolution (TASK-017)
// ============================================================================

/// Signature of a method.
///
/// Stores the parameter types, return type, and optional error type
/// for a method defined on a struct or other type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodSignature {
    /// The type name this method is defined on (e.g., "User", "Point").
    pub receiver_type: String,
    /// Method name (e.g., "isAdult", "getAge").
    pub method_name: String,
    /// Parameter type names (excluding the implicit `self` receiver).
    pub param_types: Vec<String>,
    /// Return type name.
    pub return_type: String,
    /// Optional error type for methods that can fail.
    pub error_type: Option<String>,
    /// Whether the method mutates the receiver.
    pub mutates: bool,
    /// Span where the method is defined.
    pub span: Span,
}

impl MethodSignature {
    /// Create a new method signature.
    pub fn new(
        receiver_type: String,
        method_name: String,
        param_types: Vec<String>,
        return_type: String,
        span: Span,
    ) -> Self {
        Self {
            receiver_type,
            method_name,
            param_types,
            return_type,
            error_type: None,
            mutates: false,
            span,
        }
    }

    /// Set the error type.
    pub fn with_error_type(mut self, error_type: Option<String>) -> Self {
        self.error_type = error_type;
        self
    }

    /// Set whether the method mutates the receiver.
    pub fn with_mutates(mut self, mutates: bool) -> Self {
        self.mutates = mutates;
        self
    }

    /// Get the mangled name for codegen (e.g., "User::isAdult").
    pub fn mangled_name(&self) -> String {
        format!("{}::{}", self.receiver_type, self.method_name)
    }
}

/// Table of methods for all types.
///
/// Maps type name → method name → method signature.
/// This follows the legacy compiler's method_table pattern:
/// `HashMap<String, HashMap<String, (Vec<TypeNode>, TypeNode, Option<TypeNode>)>>`
#[derive(Debug, Default)]
pub struct MethodTable {
    /// Type name → (method name → signature)
    table: HashMap<String, HashMap<String, MethodSignature>>,
}

impl MethodTable {
    /// Create a new empty method table.
    pub fn new() -> Self {
        Self {
            table: HashMap::new(),
        }
    }

    /// Register a method for a type.
    ///
    /// # Arguments
    /// - `type_name`: The type this method is defined on (e.g., "User")
    /// - `signature`: The method signature
    pub fn register(&mut self, type_name: &str, signature: MethodSignature) {
        let methods = self.table.entry(type_name.to_string()).or_default();
        methods.insert(signature.method_name.clone(), signature);
    }

    /// Check if a method exists for a type.
    pub fn has_method(&self, type_name: &str, method_name: &str) -> bool {
        self.table
            .get(type_name)
            .map(|methods| methods.contains_key(method_name))
            .unwrap_or(false)
    }

    /// Get a method signature for a type.
    pub fn get(&self, type_name: &str, method_name: &str) -> Option<&MethodSignature> {
        self.table.get(type_name)?.get(method_name)
    }

    /// Get all methods for a type.
    pub fn methods_for_type(&self, type_name: &str) -> impl Iterator<Item = &MethodSignature> {
        self.table
            .get(type_name)
            .map(|m| m.values())
            .into_iter()
            .flatten()
    }

    /// Get all types that have methods registered.
    pub fn types_with_methods(&self) -> impl Iterator<Item = &String> {
        self.table.keys()
    }

    /// Get total number of methods registered.
    pub fn total_methods(&self) -> usize {
        self.table.values().map(|m| m.len()).sum()
    }
}

/// Method resolution result.
#[derive(Debug, Clone)]
pub enum ResolvedMethod {
    /// User-defined method on a struct.
    UserDefined(MethodSignature),
    /// Built-in method on a primitive or collection type.
    Builtin {
        type_name: String,
        method_name: String,
        param_types: Vec<String>,
        return_type: String,
        mutates: bool,
    },
}

impl ResolvedMethod {
    /// Get the mangled name for codegen.
    pub fn mangled_name(&self) -> String {
        match self {
            ResolvedMethod::UserDefined(sig) => sig.mangled_name(),
            ResolvedMethod::Builtin {
                type_name,
                method_name,
                ..
            } => {
                format!("{}::{}", type_name, method_name)
            }
        }
    }

    /// Get the return type.
    pub fn return_type(&self) -> &str {
        match self {
            ResolvedMethod::UserDefined(sig) => &sig.return_type,
            ResolvedMethod::Builtin { return_type, .. } => return_type,
        }
    }

    /// Check if the method mutates the receiver.
    pub fn mutates(&self) -> bool {
        match self {
            ResolvedMethod::UserDefined(sig) => sig.mutates,
            ResolvedMethod::Builtin { mutates, .. } => *mutates,
        }
    }
}

/// Method resolver.
///
/// Resolves method calls to their definitions by receiver type.
/// Follows the legacy compiler's lookup order:
/// 1. User-defined methods on the exact type
/// 2. Built-in methods on primitive/collection types
///
/// For generic types like `Array(Int)`, also checks the generic form `Array`.
#[derive(Debug, Default)]
pub struct MethodResolver {
    /// Table of user-defined methods.
    method_table: MethodTable,
}

impl MethodResolver {
    /// Create a new method resolver.
    pub fn new() -> Self {
        Self {
            method_table: MethodTable::new(),
        }
    }

    /// Create a method resolver with an existing method table.
    pub fn with_table(table: MethodTable) -> Self {
        Self {
            method_table: table,
        }
    }

    /// Get the method table (for inspection/testing).
    pub fn table(&self) -> &MethodTable {
        &self.method_table
    }

    /// Get the method table mutably.
    pub fn table_mut(&mut self) -> &mut MethodTable {
        &mut self.method_table
    }

    /// Register a method for a type.
    pub fn register_method(&mut self, signature: MethodSignature) {
        self.method_table
            .register(&signature.receiver_type.clone(), signature);
    }

    /// Resolve a method call by receiver type and method name.
    ///
    /// # Arguments
    /// - `receiver_type`: The type of the receiver (e.g., "User", "Array(Int)", "[Int]")
    /// - `method_name`: The method being called (e.g., "isAdult", "push")
    ///
    /// # Returns
    /// The resolved method if found, or None.
    pub fn resolve(&self, receiver_type: &str, method_name: &str) -> Option<ResolvedMethod> {
        // 1. Check user-defined methods on the exact type
        if let Some(sig) = self.method_table.get(receiver_type, method_name) {
            return Some(ResolvedMethod::UserDefined(sig.clone()));
        }

        // 2. For generic types like "Array(Int)", try the generic form "Array"
        if let Some(base_type) = Self::extract_base_type(receiver_type) {
            if let Some(sig) = self.method_table.get(&base_type, method_name) {
                return Some(ResolvedMethod::UserDefined(sig.clone()));
            }
        }

        // 3. Check built-in methods
        self.resolve_builtin(receiver_type, method_name)
    }

    /// Check if a method exists for a type.
    pub fn has_method(&self, receiver_type: &str, method_name: &str) -> bool {
        self.resolve(receiver_type, method_name).is_some()
    }

    /// Extract base type from a generic type.
    ///
    /// Examples:
    /// - "Array(Int)" → Some("Array")
    /// - "[Int]" → Some("Array")
    /// - "{Str: Int}" → Some("Map")
    /// - "User" → None
    fn extract_base_type(type_name: &str) -> Option<String> {
        // Array syntax: [T] or Array(T)
        if type_name.starts_with('[') && type_name.ends_with(']') {
            return Some("Array".to_string());
        }
        if type_name.starts_with("Array(") && type_name.ends_with(')') {
            return Some("Array".to_string());
        }

        // Map syntax: {K: V} or Map(K, V)
        if type_name.starts_with('{') && type_name.ends_with('}') {
            return Some("Map".to_string());
        }
        if type_name.starts_with("Map(") && type_name.ends_with(')') {
            return Some("Map".to_string());
        }

        // Optional syntax: T? or Optional(T)
        if type_name.ends_with('?') {
            return Some("Optional".to_string());
        }
        if type_name.starts_with("Optional(") && type_name.ends_with(')') {
            return Some("Optional".to_string());
        }

        // Result syntax: Result(T, E)
        if type_name.starts_with("Result(") && type_name.ends_with(')') {
            return Some("Result".to_string());
        }

        None
    }

    /// Resolve a built-in method.
    ///
    /// Uses doo_core::methods registry for built-in method definitions.
    fn resolve_builtin(&self, receiver_type: &str, method_name: &str) -> Option<ResolvedMethod> {
        // Normalize type name for lookup
        let normalized = Self::normalize_type_for_builtin(receiver_type);

        // Use the methods registry from doo_core
        let method_def = doo_core::methods::get_method(&normalized, method_name)?;

        Some(ResolvedMethod::Builtin {
            type_name: normalized,
            method_name: method_name.to_string(),
            param_types: method_def.params.iter().map(|s| s.to_string()).collect(),
            return_type: method_def.return_type.to_string(),
            mutates: method_def.mutates,
        })
    }

    /// Normalize type name for built-in method lookup.
    ///
    /// Converts various type syntaxes to the form expected by doo_core::methods.
    fn normalize_type_for_builtin(type_name: &str) -> String {
        // String/Str normalization
        if type_name == "String" {
            return "Str".to_string();
        }

        // Array syntax normalization
        if type_name.starts_with('[') || type_name.starts_with("Array") {
            return type_name.to_string();
        }

        // Map syntax normalization
        if type_name.starts_with('{') || type_name.starts_with("Map") {
            return type_name.to_string();
        }

        type_name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use doo_core::Span;

    fn span() -> Span {
        Span::new(0, 0, 0)
    }

    #[test]
    fn test_no_cycle_linear() {
        // A -> B -> C (no cycle)
        let mut graph = ImportGraph::new();
        graph.add_import("A", "B", span());
        graph.add_import("B", "C", span());

        let detector = CircularImportDetector::new(graph);
        assert!(detector.detect_cycles().is_ok());
    }

    #[test]
    fn test_simple_cycle() {
        // A -> B -> A (cycle)
        let mut graph = ImportGraph::new();
        graph.add_import("A", "B", span());
        graph.add_import("B", "A", span());

        let detector = CircularImportDetector::new(graph);
        let result = detector.detect_cycles();
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert_eq!(error.cycle, vec!["A", "B", "A"]);
    }

    #[test]
    fn test_indirect_cycle() {
        // A -> B -> C -> A (indirect cycle)
        let mut graph = ImportGraph::new();
        graph.add_import("A", "B", span());
        graph.add_import("B", "C", span());
        graph.add_import("C", "A", span());

        let detector = CircularImportDetector::new(graph);
        let result = detector.detect_cycles();
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert_eq!(error.cycle, vec!["A", "B", "C", "A"]);
    }

    #[test]
    fn test_diamond_no_cycle() {
        // A -> B, A -> C, B -> D, C -> D (diamond, no cycle)
        let mut graph = ImportGraph::new();
        graph.add_import("A", "B", span());
        graph.add_import("A", "C", span());
        graph.add_import("B", "D", span());
        graph.add_import("C", "D", span());

        let detector = CircularImportDetector::new(graph);
        assert!(detector.detect_cycles().is_ok());
    }

    #[test]
    fn test_import_stack_cycle_detection() {
        let mut stack = ImportStack::new();

        // Push A, B, C - no cycle
        assert!(stack.push("A", span()).is_ok());
        assert!(stack.push("B", span()).is_ok());
        assert!(stack.push("C", span()).is_ok());

        // Try to push A again - cycle!
        let result = stack.push("A", span());
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert_eq!(error.cycle, vec!["A", "B", "C", "A"]);
    }

    #[test]
    fn test_import_stack_push_pop() {
        let mut stack = ImportStack::new();

        stack.push("A", span()).unwrap();
        stack.push("B", span()).unwrap();
        assert_eq!(stack.depth(), 2);

        stack.pop();
        assert_eq!(stack.depth(), 1);
        assert!(!stack.contains("B"));
        assert!(stack.contains("A"));

        // Now we can push B again since it was popped
        assert!(stack.push("B", span()).is_ok());
    }

    #[test]
    fn test_name_resolver_cycle() {
        let mut resolver = NameResolver::new();

        // Module A imports B
        resolver.begin_module("A", span()).unwrap();
        resolver.add_import("B", span()).unwrap();

        // Module B imports C
        resolver.set_current_module("B");
        resolver.add_import("C", span()).unwrap();

        // Module C imports A - cycle!
        resolver.set_current_module("C");
        let result = resolver.add_import("A", span());
        assert!(result.is_err());
    }

    #[test]
    fn test_deterministic_cycle_order() {
        // Run multiple times to verify determinism
        for _ in 0..10 {
            let mut graph = ImportGraph::new();
            graph.add_import("C", "A", span());
            graph.add_import("A", "B", span());
            graph.add_import("B", "C", span());

            let detector = CircularImportDetector::new(graph);
            let result = detector.detect_cycles();
            assert!(result.is_err());

            let error = result.unwrap_err();
            // Should always start from "A" (alphabetically first) and follow sorted edges
            assert_eq!(error.cycle, vec!["A", "B", "C", "A"]);
        }
    }

    // ========================================================================
    // Cross-Module Symbol Resolution Tests
    // ========================================================================

    #[test]
    fn test_symbol_table_define_and_lookup() {
        let mut table = SymbolTable::new();
        table.define("MyFunc", true, SymbolKindDef::Function, span());
        table.define("privateHelper", false, SymbolKindDef::Function, span());

        assert!(table.is_defined("MyFunc"));
        assert!(table.is_defined("privateHelper"));
        assert!(!table.is_defined("Unknown"));

        let my_func = table.get_local("MyFunc").unwrap();
        assert!(my_func.is_public);

        let helper = table.get_local("privateHelper").unwrap();
        assert!(!helper.is_public);
    }

    #[test]
    fn test_symbol_table_public_symbols() {
        let mut table = SymbolTable::new();
        table.define("PublicOne", true, SymbolKindDef::Function, span());
        table.define("PublicTwo", true, SymbolKindDef::Struct, span());
        table.define("privateOne", false, SymbolKindDef::Function, span());

        let public: Vec<_> = table.public_symbols().collect();
        assert_eq!(public.len(), 2);
    }

    #[test]
    fn test_symbol_table_direct_import() {
        let mut table = SymbolTable::new();

        let module = ImportedModule {
            path: vec!["std".to_string(), "Math".to_string()],
            kind: ImportKind::Symbol("Abs".to_string()),
            symbols: vec![ResolvedSymbol {
                local_name: "Abs".to_string(),
                original_name: "Abs".to_string(),
                module_path: "std::Math".to_string(),
                is_public: true,
                span: span(),
            }],
            span: span(),
        };

        table.register_import(module);

        let resolved = table.resolve("Abs").unwrap();
        assert_eq!(resolved.original_name, "Abs");
        assert_eq!(resolved.module_path, "std::Math");
    }

    #[test]
    fn test_symbol_table_aliased_import() {
        let mut table = SymbolTable::new();

        let module = ImportedModule {
            path: vec!["std".to_string(), "Math".to_string()],
            kind: ImportKind::SymbolAlias {
                name: "Sqrt".to_string(),
                alias: "Sq".to_string(),
            },
            symbols: vec![ResolvedSymbol {
                local_name: "Sqrt".to_string(),
                original_name: "Sqrt".to_string(),
                module_path: "std::Math".to_string(),
                is_public: true,
                span: span(),
            }],
            span: span(),
        };

        table.register_import(module);

        // Should be accessible via alias
        let resolved = table.resolve("Sq").unwrap();
        assert_eq!(resolved.original_name, "Sqrt");
        assert_eq!(resolved.local_name, "Sq");
    }

    #[test]
    fn test_symbol_table_wildcard_import() {
        let mut table = SymbolTable::new();

        let module = ImportedModule {
            path: vec!["std".to_string(), "Array".to_string()],
            kind: ImportKind::Wildcard,
            symbols: vec![
                ResolvedSymbol {
                    local_name: "Sum".to_string(),
                    original_name: "Sum".to_string(),
                    module_path: "std::Array".to_string(),
                    is_public: true,
                    span: span(),
                },
                ResolvedSymbol {
                    local_name: "Unique".to_string(),
                    original_name: "Unique".to_string(),
                    module_path: "std::Array".to_string(),
                    is_public: true,
                    span: span(),
                },
                ResolvedSymbol {
                    local_name: "internalHelper".to_string(),
                    original_name: "internalHelper".to_string(),
                    module_path: "std::Array".to_string(),
                    is_public: false,
                    span: span(),
                },
            ],
            span: span(),
        };

        table.register_import(module);

        // Public symbols should be accessible
        assert!(table.resolve("Sum").is_some());
        assert!(table.resolve("Unique").is_some());

        // Private symbols should not be imported via wildcard
        assert!(table.resolve("internalHelper").is_none());

        // Should track wildcard import
        assert_eq!(table.wildcards().len(), 1);
    }

    #[test]
    fn test_symbol_table_namespace_import() {
        let mut table = SymbolTable::new();

        let module = ImportedModule {
            path: vec!["std".to_string(), "File".to_string()],
            kind: ImportKind::Namespace,
            symbols: vec![
                ResolvedSymbol {
                    local_name: "Read".to_string(),
                    original_name: "Read".to_string(),
                    module_path: "std::File".to_string(),
                    is_public: true,
                    span: span(),
                },
                ResolvedSymbol {
                    local_name: "Write".to_string(),
                    original_name: "Write".to_string(),
                    module_path: "std::File".to_string(),
                    is_public: true,
                    span: span(),
                },
            ],
            span: span(),
        };

        table.register_import(module);

        // Should be accessible via qualified name
        assert!(table.resolve_qualified("File", "Read").is_some());
        assert!(table.resolve_qualified("File", "Write").is_some());

        // Namespace should exist
        assert!(table.has_namespace("File"));
    }

    #[test]
    fn test_symbol_table_namespace_alias() {
        let mut table = SymbolTable::new();

        let module = ImportedModule {
            path: vec!["std".to_string(), "Array".to_string()],
            kind: ImportKind::NamespaceAlias {
                alias: "A".to_string(),
            },
            symbols: vec![ResolvedSymbol {
                local_name: "Sum".to_string(),
                original_name: "Sum".to_string(),
                module_path: "std::Array".to_string(),
                is_public: true,
                span: span(),
            }],
            span: span(),
        };

        table.register_import(module);

        // Should be accessible via alias
        assert!(table.resolve_qualified("A", "Sum").is_some());
        assert!(table.has_namespace("A"));
    }

    #[test]
    fn test_cross_module_resolver_basic() {
        let mut resolver = CrossModuleResolver::new();

        // Define symbols in std::Math
        resolver.set_current_module("std::Math");
        resolver.define("Abs", true, SymbolKindDef::Function, span());
        resolver.define("Sqrt", true, SymbolKindDef::Function, span());
        resolver.define("internalCalc", false, SymbolKindDef::Function, span());

        // Switch to main module
        resolver.set_current_module("main");

        // Resolve from main (should not find symbols from other module without import)
        assert!(resolver.resolve_simple("Abs").is_none());
    }

    #[test]
    fn test_import_kind_determination() {
        // Empty items = namespace import
        let kind = CrossModuleResolver::determine_import_kind(
            &[],
            &["std".to_string(), "File".to_string()],
        );
        assert_eq!(kind, ImportKind::Namespace);

        // Single symbol
        let kind = CrossModuleResolver::determine_import_kind(
            &[ImportItemKind::Symbol("Abs".to_string())],
            &["std".to_string(), "Math".to_string()],
        );
        assert!(matches!(kind, ImportKind::Symbol(s) if s == "Abs"));

        // Wildcard
        let kind = CrossModuleResolver::determine_import_kind(
            &[ImportItemKind::Wildcard],
            &["std".to_string(), "Array".to_string()],
        );
        assert_eq!(kind, ImportKind::Wildcard);

        // Namespace alias (name matches last path component)
        let kind = CrossModuleResolver::determine_import_kind(
            &[ImportItemKind::Alias {
                name: "Array".to_string(),
                alias: "A".to_string(),
            }],
            &["std".to_string(), "Array".to_string()],
        );
        assert!(matches!(kind, ImportKind::NamespaceAlias { alias } if alias == "A"));

        // Symbol alias (name doesn't match last path component)
        let kind = CrossModuleResolver::determine_import_kind(
            &[ImportItemKind::Alias {
                name: "Sqrt".to_string(),
                alias: "Sq".to_string(),
            }],
            &["std".to_string(), "Math".to_string()],
        );
        assert!(
            matches!(kind, ImportKind::SymbolAlias { name, alias } if name == "Sqrt" && alias == "Sq")
        );
    }

    // ========================================================================
    // Method Resolution Tests (TASK-017)
    // ========================================================================

    #[test]
    fn test_method_signature_creation() {
        let sig = MethodSignature::new(
            "User".to_string(),
            "isAdult".to_string(),
            vec![],
            "Bool".to_string(),
            span(),
        );

        assert_eq!(sig.receiver_type, "User");
        assert_eq!(sig.method_name, "isAdult");
        assert_eq!(sig.return_type, "Bool");
        assert!(sig.param_types.is_empty());
        assert!(!sig.mutates);
        assert!(sig.error_type.is_none());
    }

    #[test]
    fn test_method_signature_mangled_name() {
        let sig = MethodSignature::new(
            "Point".to_string(),
            "distance".to_string(),
            vec!["Point".to_string()],
            "Float".to_string(),
            span(),
        );

        assert_eq!(sig.mangled_name(), "Point::distance");
    }

    #[test]
    fn test_method_signature_with_error_type() {
        let sig = MethodSignature::new(
            "Database".to_string(),
            "connect".to_string(),
            vec!["Str".to_string()],
            "Connection".to_string(),
            span(),
        )
        .with_error_type(Some("DbError".to_string()));

        assert_eq!(sig.error_type, Some("DbError".to_string()));
    }

    #[test]
    fn test_method_signature_with_mutates() {
        let sig = MethodSignature::new(
            "Counter".to_string(),
            "increment".to_string(),
            vec![],
            "Void".to_string(),
            span(),
        )
        .with_mutates(true);

        assert!(sig.mutates);
    }

    #[test]
    fn test_method_table_register_and_get() {
        let mut table = MethodTable::new();

        let sig = MethodSignature::new(
            "User".to_string(),
            "getAge".to_string(),
            vec![],
            "Int".to_string(),
            span(),
        );

        table.register("User", sig);

        assert!(table.has_method("User", "getAge"));
        assert!(!table.has_method("User", "getName"));
        assert!(!table.has_method("Point", "getAge"));

        let retrieved = table.get("User", "getAge").unwrap();
        assert_eq!(retrieved.return_type, "Int");
    }

    #[test]
    fn test_method_table_multiple_methods() {
        let mut table = MethodTable::new();

        // Register multiple methods on User
        table.register(
            "User",
            MethodSignature::new(
                "User".to_string(),
                "getAge".to_string(),
                vec![],
                "Int".to_string(),
                span(),
            ),
        );
        table.register(
            "User",
            MethodSignature::new(
                "User".to_string(),
                "getName".to_string(),
                vec![],
                "Str".to_string(),
                span(),
            ),
        );
        table.register(
            "User",
            MethodSignature::new(
                "User".to_string(),
                "isAdult".to_string(),
                vec![],
                "Bool".to_string(),
                span(),
            ),
        );

        // Register methods on Point
        table.register(
            "Point",
            MethodSignature::new(
                "Point".to_string(),
                "distance".to_string(),
                vec!["Point".to_string()],
                "Float".to_string(),
                span(),
            ),
        );

        assert_eq!(table.methods_for_type("User").count(), 3);
        assert_eq!(table.methods_for_type("Point").count(), 1);
        assert_eq!(table.total_methods(), 4);
    }

    #[test]
    fn test_method_resolver_user_defined() {
        let mut resolver = MethodResolver::new();

        // Register User.isAdult()
        resolver.register_method(MethodSignature::new(
            "User".to_string(),
            "isAdult".to_string(),
            vec![],
            "Bool".to_string(),
            span(),
        ));

        // Resolve it
        let result = resolver.resolve("User", "isAdult");
        assert!(result.is_some());

        let resolved = result.unwrap();
        assert!(matches!(resolved, ResolvedMethod::UserDefined(_)));
        assert_eq!(resolved.return_type(), "Bool");
        assert_eq!(resolved.mangled_name(), "User::isAdult");
    }

    #[test]
    fn test_method_resolver_builtin_string() {
        let resolver = MethodResolver::new();

        // Resolve Str.len() - built-in method
        let result = resolver.resolve("Str", "len");
        assert!(result.is_some());

        let resolved = result.unwrap();
        assert!(matches!(resolved, ResolvedMethod::Builtin { .. }));
        assert_eq!(resolved.return_type(), "Int");
        assert!(!resolved.mutates());
    }

    #[test]
    fn test_method_resolver_builtin_array() {
        let resolver = MethodResolver::new();

        // Resolve [Int].push() - built-in method
        let result = resolver.resolve("[Int]", "push");
        assert!(result.is_some());

        let resolved = result.unwrap();
        assert!(matches!(resolved, ResolvedMethod::Builtin { .. }));
        assert!(resolved.mutates());

        // Resolve Array(String).len()
        let result2 = resolver.resolve("Array(String)", "len");
        assert!(result2.is_some());
    }

    #[test]
    fn test_method_resolver_builtin_map() {
        let resolver = MethodResolver::new();

        // Resolve {Str: Int}.has() - built-in method
        let result = resolver.resolve("{Str: Int}", "has");
        assert!(result.is_some());

        let resolved = result.unwrap();
        assert!(matches!(resolved, ResolvedMethod::Builtin { .. }));
        assert_eq!(resolved.return_type(), "Bool");
    }

    #[test]
    fn test_method_resolver_not_found() {
        let resolver = MethodResolver::new();

        // Try to resolve non-existent method
        let result = resolver.resolve("User", "nonExistent");
        assert!(result.is_none());

        // Try to resolve on non-existent type
        let result2 = resolver.resolve("UnknownType", "someMethod");
        assert!(result2.is_none());
    }

    #[test]
    fn test_method_resolver_user_defined_priority() {
        let mut resolver = MethodResolver::new();

        // Register a custom 'len' method on MyArray (should take priority over builtin)
        resolver.register_method(MethodSignature::new(
            "MyArray".to_string(),
            "len".to_string(),
            vec![],
            "Float".to_string(), // Custom return type
            span(),
        ));

        // User-defined should be found for MyArray
        let result = resolver.resolve("MyArray", "len");
        assert!(result.is_some());
        let resolved = result.unwrap();
        assert!(matches!(resolved, ResolvedMethod::UserDefined(_)));
        assert_eq!(resolved.return_type(), "Float");
    }

    #[test]
    fn test_extract_base_type() {
        // Array types
        assert_eq!(
            MethodResolver::extract_base_type("[Int]"),
            Some("Array".to_string())
        );
        assert_eq!(
            MethodResolver::extract_base_type("Array(String)"),
            Some("Array".to_string())
        );

        // Map types
        assert_eq!(
            MethodResolver::extract_base_type("{Str: Int}"),
            Some("Map".to_string())
        );
        assert_eq!(
            MethodResolver::extract_base_type("Map(Str, User)"),
            Some("Map".to_string())
        );

        // Optional types
        assert_eq!(
            MethodResolver::extract_base_type("Int?"),
            Some("Optional".to_string())
        );
        assert_eq!(
            MethodResolver::extract_base_type("Optional(String)"),
            Some("Optional".to_string())
        );

        // Result types
        assert_eq!(
            MethodResolver::extract_base_type("Result(Int, Error)"),
            Some("Result".to_string())
        );

        // Non-generic types
        assert_eq!(MethodResolver::extract_base_type("User"), None);
        assert_eq!(MethodResolver::extract_base_type("Point"), None);
    }

    #[test]
    fn test_method_resolver_has_method() {
        let mut resolver = MethodResolver::new();

        resolver.register_method(MethodSignature::new(
            "User".to_string(),
            "isAdult".to_string(),
            vec![],
            "Bool".to_string(),
            span(),
        ));

        assert!(resolver.has_method("User", "isAdult"));
        assert!(!resolver.has_method("User", "getName"));

        // Built-in methods
        assert!(resolver.has_method("Str", "len"));
        assert!(resolver.has_method("[Int]", "push"));
    }

    #[test]
    fn test_resolved_method_properties() {
        let mut resolver = MethodResolver::new();

        // Register a mutating method
        resolver.register_method(
            MethodSignature::new(
                "Counter".to_string(),
                "increment".to_string(),
                vec![],
                "Void".to_string(),
                span(),
            )
            .with_mutates(true),
        );

        let result = resolver.resolve("Counter", "increment").unwrap();
        assert!(result.mutates());
        assert_eq!(result.return_type(), "Void");
        assert_eq!(result.mangled_name(), "Counter::increment");
    }
}
