//! Module Linker and Cross-Module Resolver
//!
//! Handles multi-file compilation by linking multiple LLVM modules together
//! and resolving cross-module function references.

use inkwell::context::Context;
use inkwell::module::Module;
use rustc_hash::FxHashMap;

// ============================================================================
// Link Error
// ============================================================================

/// Error type for module linking operations.
#[derive(Debug, Clone)]
pub enum LinkError {
    /// LLVM module linking failed.
    LinkFailed(String),
    /// Symbol not found in any module.
    SymbolNotFound(String),
    /// Duplicate symbol definition.
    DuplicateSymbol(String),
    /// Module not found.
    ModuleNotFound(String),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::LinkFailed(msg) => write!(f, "Link failed: {}", msg),
            LinkError::SymbolNotFound(sym) => write!(f, "Symbol not found: {}", sym),
            LinkError::DuplicateSymbol(sym) => write!(f, "Duplicate symbol: {}", sym),
            LinkError::ModuleNotFound(name) => write!(f, "Module not found: {}", name),
        }
    }
}

impl std::error::Error for LinkError {}

impl From<LinkError> for doo_core::errors::codes::CompilerError {
    fn from(e: LinkError) -> Self {
        use doo_core::errors::codes::ErrorCode;
        let (code, msg) = match &e {
            LinkError::LinkFailed(m) => (ErrorCode::LlvmError, format!("link failed: {}", m)),
            LinkError::SymbolNotFound(s) => {
                (ErrorCode::CodegenFailed, format!("symbol not found: {}", s))
            }
            LinkError::DuplicateSymbol(s) => (
                ErrorCode::NameAlreadyDefined,
                format!("duplicate symbol: {}", s),
            ),
            LinkError::ModuleNotFound(n) => (
                ErrorCode::ModuleNotFound,
                format!("module not found: {}", n),
            ),
        };
        doo_core::errors::codes::CompilerError::new(code, msg, doo_core::Span::new(0, 0, 0))
    }
}

// ============================================================================
// Module Linker
// ============================================================================

/// Module linker for multi-file compilation.
///
/// Links multiple LLVM modules together, resolving cross-module references.
///
/// ## Design
///
/// The Doo compiler uses a "merge all" approach:
/// 1. Each source file is compiled to a separate LLVM module
/// 2. Imported functions are declared as external in the importing module
/// 3. All modules are linked together into a single module for final codegen
///
/// This matches the legacy compiler behavior where all imported functions
/// are merged into the AST before MIR generation.
pub struct ModuleLinker<'ctx> {
    /// LLVM context (shared across all modules).
    context: &'ctx Context,
    /// Modules to be linked.
    modules: Vec<Module<'ctx>>,
    /// Symbol table: function_name -> defining_module_index.
    symbol_table: FxHashMap<String, usize>,
    /// Function aliases across all modules.
    function_aliases: FxHashMap<String, String>,
}

impl<'ctx> ModuleLinker<'ctx> {
    /// Create a new module linker.
    pub fn new(context: &'ctx Context) -> Self {
        Self {
            context,
            modules: Vec::new(),
            symbol_table: FxHashMap::default(),
            function_aliases: FxHashMap::default(),
        }
    }

    /// Add a module to be linked.
    ///
    /// Builds the symbol table from the module's function definitions.
    pub fn add_module(&mut self, module: Module<'ctx>) -> Result<(), LinkError> {
        let module_index = self.modules.len();

        // Build symbol table from function definitions
        let mut func_iter = module.get_first_function();
        while let Some(func) = func_iter {
            let name = func.get_name().to_str().unwrap_or("").to_string();

            // Only add functions with bodies (definitions, not declarations)
            if func.count_basic_blocks() > 0 {
                if self.symbol_table.contains_key(&name) {
                    // Allow redefinitions if the existing one is just a declaration
                    let existing_idx = self.symbol_table[&name];
                    if let Some(existing_module) = self.modules.get(existing_idx) {
                        if let Some(existing_func) = existing_module.get_function(&name) {
                            if existing_func.count_basic_blocks() > 0 {
                                return Err(LinkError::DuplicateSymbol(name));
                            }
                        }
                    }
                }
                self.symbol_table.insert(name, module_index);
            }

            func_iter = func.get_next_function();
        }

        self.modules.push(module);
        Ok(())
    }

    /// Add function aliases from a module context.
    pub fn add_aliases(&mut self, aliases: &FxHashMap<String, String>) {
        self.function_aliases.extend(aliases.clone());
    }

    /// Link all modules into a single output module.
    ///
    /// Creates a new module containing all functions from all input modules.
    pub fn link(&mut self, output_name: &str) -> Result<Module<'ctx>, LinkError> {
        if self.modules.is_empty() {
            let output = self.context.create_module(output_name);
            return Ok(output);
        }

        // Take the first module as the base
        let output = self.modules.remove(0);
        let module = output;
        module.set_name(output_name);

        // Link remaining modules into the base
        for additional in self.modules.drain(..) {
            module
                .link_in_module(additional)
                .map_err(|e| LinkError::LinkFailed(e.to_string()))?;
        }

        Ok(module)
    }

    /// Check if a symbol is defined in any module.
    pub fn has_symbol(&self, name: &str) -> bool {
        self.symbol_table.contains_key(name)
    }

    /// Get the module index where a symbol is defined.
    pub fn get_symbol_module(&self, name: &str) -> Option<usize> {
        self.symbol_table.get(name).copied()
    }

    /// Resolve a function name through aliases.
    pub fn resolve_alias(&self, name: &str) -> String {
        self.function_aliases
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    /// Get all defined symbols.
    pub fn symbols(&self) -> impl Iterator<Item = &String> {
        self.symbol_table.keys()
    }

    /// Get the number of modules.
    pub fn module_count(&self) -> usize {
        self.modules.len()
    }
}

// ============================================================================
// Cross-Module Reference Resolver
// ============================================================================

/// Resolves cross-module function references.
///
/// When a function is called from another module, we need to:
/// 1. Declare the function as external in the calling module
/// 2. Track the dependency for linking
pub struct CrossModuleResolver {
    /// External function declarations needed: (calling_module, function_name).
    pending_declarations: Vec<(String, String)>,
    /// Resolved references: function_name -> source_module.
    resolved: FxHashMap<String, String>,
}

impl CrossModuleResolver {
    /// Create a new cross-module resolver.
    pub fn new() -> Self {
        Self {
            pending_declarations: Vec::new(),
            resolved: FxHashMap::default(),
        }
    }

    /// Record a cross-module function call.
    ///
    /// Called when the current module calls a function from another module.
    pub fn record_call(&mut self, current_module: &str, function_name: &str, source_module: &str) {
        if !self.resolved.contains_key(function_name) {
            self.pending_declarations
                .push((current_module.to_string(), function_name.to_string()));
            self.resolved
                .insert(function_name.to_string(), source_module.to_string());
        }
    }

    /// Get pending external declarations for a module.
    pub fn get_pending(&self, module: &str) -> Vec<String> {
        self.pending_declarations
            .iter()
            .filter(|(m, _)| m == module)
            .map(|(_, f)| f.clone())
            .collect()
    }

    /// Get the source module for a function.
    pub fn get_source(&self, function_name: &str) -> Option<&String> {
        self.resolved.get(function_name)
    }

    /// Check if a function has been resolved.
    pub fn is_resolved(&self, function_name: &str) -> bool {
        self.resolved.contains_key(function_name)
    }
}

impl Default for CrossModuleResolver {
    fn default() -> Self {
        Self::new()
    }
}
