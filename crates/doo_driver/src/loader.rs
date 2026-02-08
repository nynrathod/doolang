//! Module Loader
//!
//! Handles loading and parsing of imported modules.
//! This is the single source of truth for module file I/O in the new compiler.
//!
//! ## Architecture
//!
//! ```text
//! doo_driver/loader.rs          doo_analysis/resolve.rs
//! ─────────────────────         ──────────────────────────
//! ModuleLoader                   SymbolTable
//!   - find files                   - track symbols
//!   - read source                  - visibility rules
//!   - parse AST                  CrossModuleResolver
//!   - cache modules                - resolve lookups
//!                                ImportGraph
//!                                  - detect cycles
//! ```
//!
//! ## Responsibilities
//! - **File I/O**: Finding module files (stdlib, project-relative)
//! - **Parsing**: Reading and parsing module source code  
//! - **Caching**: Caching parsed modules to avoid re-parsing
//! - **Extraction**: Extracting requested symbols based on import declarations
//!
//! ## Design Principles
//! - File I/O is isolated here (not in analysis crate)
//! - Symbol resolution types from `doo_analysis::resolve` are reused
//! - Returns AST items ready for merging into the main program

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use doo_core::doo_debug;
use doo_core::errors::codes::{CompilerError, ErrorCode};
use doo_core::Span;
use doo_frontend::ast::{ImportDecl, ImportItem, Item, Program};
use doo_frontend::Parser;

// Re-export analysis types for consistency (single source of truth for symbol resolution)
pub use doo_analysis::{
    CrossModuleResolver, ImportItemKind, ImportKind, ImportedModule, ResolvedSymbol, SymbolTable,
};
// SymbolKindDef is in semantic submodule
pub use doo_analysis::semantic::SymbolKindDef;

/// Module loader for the Doo compiler.
///
/// Handles discovery, loading, and parsing of imported modules.
/// Caches parsed modules to avoid redundant work.
#[derive(Debug, Default)]
pub struct ModuleLoader {
    /// Cached parsed modules: module_key (e.g., "std::Math") -> Program
    cache: HashMap<String, Program>,
    /// Standard library path
    stdlib_path: Option<PathBuf>,
    /// Debug mode
    debug: bool,
}

impl ModuleLoader {
    /// Create a new module loader.
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            stdlib_path: None,
            debug: env::var("DOO_DEBUG").is_ok(),
        }
    }

    /// Create a module loader with debug enabled.
    pub fn with_debug(mut self, debug: bool) -> Self {
        self.debug = debug;
        self
    }

    /// Resolve the stdlib path.
    ///
    /// Search order:
    /// 1. DOO_STDLIB_PATH environment variable
    /// 2. Next to the executable (production deployment)
    /// 3. Search up directory tree from cwd (development)
    /// 4. Relative ./std (CI/testing)
    pub fn resolve_stdlib_path(&mut self) -> Result<&Path, String> {
        if self.stdlib_path.is_some() {
            return Ok(self.stdlib_path.as_ref().unwrap());
        }

        // 1. Explicit env var
        if let Ok(stdlib_env) = env::var("DOO_STDLIB_PATH") {
            let path = PathBuf::from(&stdlib_env);
            if path.exists() {
                self.stdlib_path = Some(path);
                return Ok(self.stdlib_path.as_ref().unwrap());
            }
        }

        // 2. Next to executable
        if let Ok(exe_path) = env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let stdlib_dir = exe_dir.join("std");
                if stdlib_dir.exists() {
                    self.stdlib_path = Some(stdlib_dir);
                    return Ok(self.stdlib_path.as_ref().unwrap());
                }
            }
        }

        // 3. Search up from cwd
        if let Ok(current_dir) = env::current_dir() {
            let mut current = current_dir;
            for _ in 0..20 {
                let stdlib_dir = current.join("std");
                if stdlib_dir.exists() {
                    self.stdlib_path = Some(stdlib_dir);
                    return Ok(self.stdlib_path.as_ref().unwrap());
                }
                if !current.pop() {
                    break;
                }
            }
        }

        // 4. Relative fallback
        let dev_stdlib = PathBuf::from("./std");
        if dev_stdlib.exists() {
            self.stdlib_path = Some(dev_stdlib);
            return Ok(self.stdlib_path.as_ref().unwrap());
        }

        Err(
            "Could not find stdlib directory. Set DOO_STDLIB_PATH or run from project root."
                .to_string(),
        )
    }

    /// Load and parse a module file.
    ///
    /// Returns the cached version if already loaded.
    pub fn load_module(&mut self, module_key: &str) -> Result<&Program, String> {
        // Return cached if available
        if self.cache.contains_key(module_key) {
            return Ok(self.cache.get(module_key).unwrap());
        }

        // Parse module key: "std::Math" -> ("std", "Math")
        let parts: Vec<&str> = module_key.split("::").collect();
        if parts.len() < 2 {
            return Err(format!("Invalid module key: {}", module_key));
        }

        let namespace = parts[0];
        let module_name = parts[1];

        // Resolve file path
        let module_file = match namespace {
            "std" => {
                let stdlib = self.resolve_stdlib_path()?;
                stdlib.join(format!("{}.doo", module_name))
            }
            _ => {
                // TODO: Support project-relative imports
                return Err(format!("Unsupported namespace: {}", namespace));
            }
        };

        if !module_file.exists() {
            return Err(format!("Module file not found: {}", module_file.display()));
        }

        if self.debug {
            doo_debug!(
                "LOADER",
                "Loading module: {} from {}",
                module_key,
                module_file.display()
            );
        }

        // Read and parse
        let source = fs::read_to_string(&module_file)
            .map_err(|e| format!("Failed to read {}: {}", module_file.display(), e))?;

        let mut parser = Parser::new(&source, 0);
        let program = parser
            .parse_program()
            .map_err(|e| format!("Failed to parse {}: {}", module_key, e))?;

        // Cache and return
        self.cache.insert(module_key.to_string(), program);
        Ok(self.cache.get(module_key).unwrap())
    }

    /// Get a cached module (doesn't load if not cached).
    pub fn get_cached(&self, module_key: &str) -> Option<&Program> {
        self.cache.get(module_key)
    }

    /// Check if a module is cached.
    pub fn is_cached(&self, module_key: &str) -> bool {
        self.cache.contains_key(module_key)
    }

    /// Clear the module cache.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

/// Import resolution result.
///
/// Contains the items to be merged into the main program.
#[derive(Debug, Default)]
pub struct ImportResolution {
    /// Items to prepend to the program (imported functions, structs, enums).
    pub items: Vec<Item>,
    /// Errors encountered during resolution.
    pub errors: Vec<CompilerError>,
}

/// Resolve all imports in a program.
///
/// This function:
/// 1. Collects all import declarations
/// 2. Determines which modules need to be loaded
/// 3. Loads and parses the modules
/// 4. Extracts the requested symbols
/// 5. Returns items to be merged into the program
///
/// ## Import Patterns Supported
/// - `import std::Math::Abs;` - single symbol from path
/// - `import std::Math::{Pow, Sqrt};` - multiple symbols
/// - `import std::Math::Floor as Fl;` - aliased symbol
/// - `import std::Math::{Sqrt as Sq};` - aliased in braces
/// - `import std::Array::*;` - wildcard
/// - `import std::File;` - namespace import
/// - `import std::Array as A;` - namespace alias
/// - `import defs::types::{PublicUser};` - local module import
pub fn resolve_imports(
    program: &Program,
    loader: &mut ModuleLoader,
    project_root: &Path,
) -> Result<ImportResolution, String> {
    let debug = env::var("DOO_DEBUG").is_ok();

    // Collect imports
    let imports: Vec<&ImportDecl> = program
        .items
        .iter()
        .filter_map(|item| {
            if let Item::Import(i) = item {
                Some(i)
            } else {
                None
            }
        })
        .collect();

    if imports.is_empty() {
        return Ok(ImportResolution::default());
    }

    if debug {
        doo_debug!("LOADER", "Resolving {} imports", imports.len());
    }

    // Build import requests: module_key -> set of (symbol_name, (optional_alias, span))
    let mut std_import_requests: HashMap<
        String,
        HashMap<String, (Option<String>, doo_core::Span)>,
    > = HashMap::new();
    // (import_decl, module_path, path_symbols) - path_symbols are symbols extracted
    // from the import path when the last segment is a symbol name, not a file.
    // e.g., `import Models::Task;` → module=Models.doo, path_symbols=["Task"]
    let mut local_import_requests: Vec<(&ImportDecl, PathBuf, Vec<String>)> = Vec::new();

    for import in &imports {
        if import.path.is_empty() {
            continue;
        }

        if import.path[0] == "std" {
            // Standard library import - needs at least std::ModuleName
            if import.path.len() < 2 {
                continue;
            }
            let module_name = &import.path[1];
            let module_key = format!("std::{}", module_name);
            let symbols = std_import_requests.entry(module_key).or_default();

            // Determine what symbols are requested
            let path_symbol = if import.path.len() >= 3 {
                Some(import.path[2].clone())
            } else {
                None
            };

            if let Some(sym) = path_symbol {
                if import.wildcard {
                    // import std::Module::*
                    symbols.insert("*".to_string(), (None, import.span));
                } else {
                    // import std::Math::Abs or import std::Math::Abs as A
                    symbols.insert(sym, (import.alias.clone(), import.span));
                }
            } else if import.alias.is_some() {
                // import std::Array as A - namespace alias, import all
                symbols.insert("*".to_string(), (import.alias.clone(), import.span));
            } else if import.wildcard {
                // import std::Array::*
                symbols.insert("*".to_string(), (None, import.span));
            } else if import.items.is_empty() {
                // import std::File - namespace import, import all
                symbols.insert("*".to_string(), (None, import.span));
            } else {
                // import std::Math::{Min, Max, Sqrt as Sq}
                for item in &import.items {
                    match item {
                        ImportItem::Symbol(name) => {
                            symbols.insert(name.clone(), (None, import.span));
                        }
                        ImportItem::Alias { name, alias } => {
                            symbols.insert(name.clone(), (Some(alias.clone()), import.span));
                        }
                        ImportItem::Wildcard => {
                            symbols.insert("*".to_string(), (None, import.span));
                        }
                    }
                }
            }
        } else {
            // Local module import
            // Supports two patterns:
            //   1. Directory-based: `import defs::types::{PublicUser}` → defs/types.doo
            //   2. Symbol-from-file: `import Models::Task;` → Models.doo, symbol=Task
            //
            // Resolution order:
            //   a) Try full path as file (all segments form the file path)
            //   b) If not found, treat last segment as symbol name, rest as module path
            let mut module_path = project_root.to_path_buf();
            let mut path_symbols: Vec<String> = Vec::new();

            // Build file path from all path segments
            for segment in &import.path {
                module_path.push(segment);
            }
            module_path.set_extension("doo");

            if module_path.exists() {
                // Full path exists as a file (e.g., defs/types.doo)
                if debug {
                    doo_debug!("LOADER", "Found local module: {}", module_path.display());
                }
                local_import_requests.push((import, module_path, path_symbols));
            } else if import.path.len() >= 2 {
                // Full path not found. Try treating last segment(s) as symbol names.
                // e.g., `import Models::Task;` → Models.doo + symbol "Task"
                let mut alt_path = project_root.to_path_buf();
                for i in 0..import.path.len() - 1 {
                    alt_path.push(&import.path[i]);
                }
                alt_path.set_extension("doo");

                if alt_path.exists() {
                    let symbol = import.path.last().unwrap().clone();
                    if debug {
                        doo_debug!(
                            "LOADER",
                            "Found local module: {} (importing symbol: {})",
                            alt_path.display(),
                            symbol
                        );
                    }
                    path_symbols.push(symbol);
                    local_import_requests.push((import, alt_path, path_symbols));
                } else if debug {
                    doo_debug!(
                        "LOADER",
                        "Local module not found: {}",
                        module_path.display()
                    );
                }
            } else if debug {
                doo_debug!(
                    "LOADER",
                    "Local module not found: {}",
                    module_path.display()
                );
            }
        }
    }

    // Load modules and extract symbols
    let mut result = ImportResolution::default();
    let mut imported_names: HashSet<String> = HashSet::new();

    // Process standard library imports
    for (module_key, requested) in &std_import_requests {
        // Load the module
        let module_program = match loader.load_module(module_key) {
            Ok(p) => p,
            Err(e) => {
                let code = if e.contains("not found") {
                    ErrorCode::ModuleNotFound
                } else if e.contains("Invalid module key") {
                    ErrorCode::InvalidImportPath
                } else {
                    ErrorCode::IoError
                };
                result.errors.push(CompilerError::new(
                    code,
                    format!("failed to load '{}': {}", module_key, e),
                    doo_core::Span::new(0, 0, 0),
                ));
                continue;
            }
        };

        let import_all = requested.contains_key("*");

        // Collect all available symbol names in the module for ImportNotFound check
        let mut available_names: HashSet<String> = HashSet::new();
        for item in &module_program.items {
            match item {
                Item::Function(f) => {
                    available_names.insert(f.name.clone());
                }
                Item::Struct(s) => {
                    available_names.insert(s.name.clone());
                }
                Item::Enum(e) => {
                    available_names.insert(e.name.clone());
                }
                _ => {}
            }
        }

        // Check for ImportNotFound + PrivateImport on explicitly requested symbols
        if !import_all {
            for (sym_name, (_, span)) in requested.iter() {
                if sym_name == "*" {
                    continue;
                }
                if !available_names.contains(sym_name) {
                    // Symbol not found in module
                    result.errors.push(
                        CompilerError::new(
                            ErrorCode::ImportNotFound,
                            format!("'{}' not found in module '{}'", sym_name, module_key),
                            *span,
                        )
                        .with_suggestion(format!("check available exports in {}", module_key)),
                    );
                } else {
                    // Symbol found — check if private (camelCase = private, PascalCase = public)
                    let is_public = sym_name
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false);
                    if !is_public {
                        result.errors.push(
                            CompilerError::new(
                                ErrorCode::PrivateImport,
                                format!("'{}' is private", sym_name),
                                *span,
                            )
                            .with_suggestion(format!("rename to '{}'", capitalize_first(sym_name))),
                        );
                    }
                }
            }
        }

        // First pass: collect struct/enum names that will be imported
        // so we can also import their associated functions
        let mut imported_type_names: HashSet<String> = HashSet::new();
        for item in &module_program.items {
            match item {
                Item::Struct(s) => {
                    let is_wanted = import_all || requested.contains_key(&s.name);
                    if is_wanted {
                        imported_type_names.insert(s.name.clone());
                    }
                }
                Item::Enum(e) => {
                    let is_wanted = import_all || requested.contains_key(&e.name);
                    if is_wanted {
                        imported_type_names.insert(e.name.clone());
                    }
                }
                _ => {}
            }
        }

        // Second pass: extract requested items including associated functions
        for item in &module_program.items {
            match item {
                Item::Function(f) => {
                    // Public = starts with uppercase (PascalCase)
                    let is_public = f
                        .name
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false);

                    // Check if this is an associated function for an imported type
                    let is_associated_with_imported_type = f
                        .associated_type
                        .as_ref()
                        .map(|t| imported_type_names.contains(t))
                        .unwrap_or(false);

                    // Check if this function is explicitly requested by name
                    // This allows importing associated functions like `postgres` from Database
                    // even if the struct itself isn't imported
                    let is_explicitly_requested = requested.contains_key(&f.name);

                    let is_wanted =
                        import_all || is_explicitly_requested || is_associated_with_imported_type;

                    // Create a unique key for the function to avoid duplicates
                    let func_key = if let Some(ref assoc_type) = f.associated_type {
                        format!("{}.{}", assoc_type, f.name)
                    } else {
                        f.name.clone()
                    };

                    // Import if:
                    // 1. Explicitly requested by name (e.g., import std::Database::{postgres})
                    // 2. Public and wanted (import_all or associated with imported type)
                    // 3. Associated with an imported type
                    if (is_explicitly_requested || is_public || is_associated_with_imported_type)
                        && is_wanted
                        && !imported_names.contains(&func_key)
                    {
                        if debug {
                            if is_associated_with_imported_type {
                                doo_debug!(
                                    "LOADER",
                                    "  Importing associated function: {}.{}",
                                    f.associated_type.as_deref().unwrap_or("?"),
                                    f.name
                                );
                            } else {
                                doo_debug!("LOADER", "  Importing function: {}", f.name);
                            }
                        }
                        imported_names.insert(func_key);
                        result.items.push(item.clone());
                    }
                }
                Item::Struct(s) => {
                    let is_wanted = import_all || requested.contains_key(&s.name);
                    if is_wanted && !imported_names.contains(&s.name) {
                        if debug {
                            doo_debug!("LOADER", "  Importing struct: {}", s.name);
                        }
                        imported_names.insert(s.name.clone());
                        result.items.push(item.clone());
                    }
                }
                Item::Enum(e) => {
                    let is_wanted = import_all || requested.contains_key(&e.name);
                    if is_wanted && !imported_names.contains(&e.name) {
                        if debug {
                            doo_debug!("LOADER", "  Importing enum: {}", e.name);
                        }
                        imported_names.insert(e.name.clone());
                        result.items.push(item.clone());
                    }
                }
                Item::Import(_) | Item::Statement(_) => {
                    // Don't re-export
                }
            }
        }
    }

    // Process local module imports
    // Track visited modules to prevent infinite loops
    let mut visited_modules: HashSet<PathBuf> = HashSet::new();
    // Each entry: (module_path, path_symbols, import_chain_for_circular_detection, origin_span)
    let mut pending_modules: Vec<(PathBuf, Vec<String>, Vec<PathBuf>, doo_core::Span)> =
        local_import_requests
            .iter()
            .map(|(import_decl, path, symbols)| {
                (path.clone(), symbols.clone(), vec![], import_decl.span)
            })
            .collect();

    // Check visibility for explicitly requested local import symbols
    for (import_decl, _module_path, path_symbols) in &local_import_requests {
        for sym_name in path_symbols {
            let is_public = sym_name
                .chars()
                .next()
                .map(|c| c.is_uppercase())
                .unwrap_or(false);
            if !is_public {
                // Narrow span to just the symbol name at end of import path
                let sym_span = narrow_span_to_symbol(&import_decl.span, sym_name);
                result.errors.push(
                    CompilerError::new(
                        ErrorCode::PrivateImport,
                        format!("'{}' is private", sym_name),
                        sym_span,
                    )
                    .with_suggestion(format!("rename to '{}'", capitalize_first(sym_name))),
                );
            }
        }
        for item in &import_decl.items {
            let sym_name = match item {
                ImportItem::Symbol(name) => name,
                ImportItem::Alias { name, .. } => name,
                ImportItem::Wildcard => continue,
            };
            let is_public = sym_name
                .chars()
                .next()
                .map(|c| c.is_uppercase())
                .unwrap_or(false);
            if !is_public {
                let sym_span = narrow_span_to_symbol(&import_decl.span, sym_name);
                result.errors.push(
                    CompilerError::new(
                        ErrorCode::PrivateImport,
                        format!("'{}' is private", sym_name),
                        sym_span,
                    )
                    .with_suggestion(format!("rename to '{}'", capitalize_first(sym_name))),
                );
            }
        }
    }

    while let Some((module_path, path_symbols, import_chain, origin_span)) = pending_modules.pop() {
        // Skip if already visited
        let canonical_path = module_path
            .canonicalize()
            .unwrap_or_else(|_| module_path.clone());
        if visited_modules.contains(&canonical_path) {
            continue;
        }
        visited_modules.insert(canonical_path.clone());

        // Read and parse the local module
        let source = match fs::read_to_string(&module_path) {
            Ok(s) => s,
            Err(e) => {
                result.errors.push(CompilerError::new(
                    ErrorCode::ModuleNotFound,
                    format!("failed to read module '{}': {}", module_path.display(), e),
                    doo_core::Span::new(0, 0, 0),
                ));
                continue;
            }
        };

        let mut parser = Parser::new(&source, 0);
        let module_program = match parser.parse_program() {
            Ok(p) => p,
            Err(e) => {
                result.errors.push(CompilerError::new(
                    ErrorCode::IoError,
                    format!("failed to parse module '{}': {}", module_path.display(), e),
                    doo_core::Span::new(0, 0, 0),
                ));
                continue;
            }
        };

        // Check for parser errors even on Ok result
        if !parser.errors().is_empty() {
            doo_debug!("LOADER", "Parser errors in {}:", module_path.display());
            for err in parser.errors() {
                doo_debug!("LOADER", "  {}", err);
            }
        }

        // DEBUG: Show parsed functions and their bodies
        if debug {
            doo_debug!("LOADER", "Parsed module: {}", module_path.display());
            for item in &module_program.items {
                if let Item::Function(f) = item {
                    doo_debug!(
                        "LOADER",
                        "  Function {}, body has {} statements",
                        f.name,
                        f.body.len()
                    );
                    for (i, stmt) in f.body.iter().enumerate() {
                        doo_debug!(
                            "LOADER",
                            "    Stmt {}: {:?}",
                            i,
                            std::mem::discriminant(&stmt.kind)
                        );
                    }
                }
            }
        }

        // CRITICAL: Process nested imports from this module
        // This ensures transitive dependencies (e.g., User from directory.doo importing models::user)
        // are loaded into the merged program.
        // NOTE: Nested imports are resolved relative to project_root, not the current module's parent

        // Build the import chain for circular detection: current chain + this module
        let mut nested_chain = import_chain.clone();
        nested_chain.push(canonical_path.clone());

        for item in &module_program.items {
            if let Item::Import(nested_import) = item {
                if nested_import.path.is_empty() {
                    continue;
                }

                // Skip std library imports (they're handled separately)
                if nested_import.path[0] == "std" {
                    // Queue std import handling - add to std_import_requests would need refactoring
                    // For now, std imports from nested modules inherit the parent's type definitions
                    continue;
                }

                // Build nested module path - ALWAYS from project_root
                let mut nested_module_path = project_root.to_path_buf();
                let mut nested_path_symbols: Vec<String> = Vec::new();

                for segment in &nested_import.path {
                    nested_module_path.push(segment);
                }
                nested_module_path.set_extension("doo");

                let resolved_path = if nested_module_path.exists() {
                    Some(nested_module_path)
                } else if nested_import.path.len() >= 2 {
                    // Try alternate path: treat last segment as symbol
                    let mut alt_path = project_root.to_path_buf();
                    for i in 0..nested_import.path.len() - 1 {
                        alt_path.push(&nested_import.path[i]);
                    }
                    alt_path.set_extension("doo");

                    if alt_path.exists() {
                        let symbol = nested_import.path.last().unwrap().clone();
                        nested_path_symbols.push(symbol);
                        Some(alt_path)
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some(resolved) = resolved_path {
                    let nested_canonical =
                        resolved.canonicalize().unwrap_or_else(|_| resolved.clone());

                    // Check for circular import: if this module is in our import chain
                    if nested_chain.contains(&nested_canonical) {
                        let from_name = module_path
                            .file_stem()
                            .and_then(|f| f.to_str())
                            .unwrap_or("unknown");
                        let to_name = resolved
                            .file_stem()
                            .and_then(|f| f.to_str())
                            .unwrap_or("unknown");
                        result.errors.push(
                            CompilerError::new(
                                ErrorCode::CircularImport,
                                format!("'{}' and '{}' form a cycle", from_name, to_name),
                                origin_span,
                            )
                            .with_suggestion("extract shared types into a third module"),
                        );
                    } else if !visited_modules.contains(&nested_canonical) {
                        if debug {
                            doo_debug!(
                                "LOADER",
                                "Queueing nested import: {} from {}",
                                resolved.display(),
                                module_path.display()
                            );
                        }
                        pending_modules.push((
                            resolved,
                            nested_path_symbols,
                            nested_chain.clone(),
                            origin_span,
                        ));
                    }
                }
            }
        }

        // Determine what symbols to import
        //
        // For local modules, we always import ALL public items regardless of the
        // specific symbols listed in the import declaration. This is because the Doo
        // compiler uses a merge-based import system where imported items are placed
        // into a single flat program. Imported functions often reference sibling types
        // (enums, structs) from their source module, and those types must be present
        // in the merged program for codegen to work correctly.
        //
        // The `{...}` syntax in imports documents usage intent but doesn't restrict
        // what gets loaded — all public items from the local module are included.
        let import_all = true;

        // First pass: collect struct/enum names that will be imported
        // so we can also import their associated functions
        let mut imported_type_names: HashSet<String> = HashSet::new();
        for item in &module_program.items {
            match item {
                Item::Struct(s) => {
                    let is_public = s
                        .name
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false);
                    if is_public {
                        imported_type_names.insert(s.name.clone());
                    }
                }
                Item::Enum(e) => {
                    let is_public = e
                        .name
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false);
                    if is_public {
                        imported_type_names.insert(e.name.clone());
                    }
                }
                _ => {}
            }
        }

        // Second pass: extract requested items including associated functions
        for item in &module_program.items {
            match item {
                Item::Function(f) => {
                    // Check visibility: PascalCase = public, camelCase = private
                    let is_public = f
                        .name
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false);

                    // Check if this is an associated function for an imported type
                    let is_associated_with_imported_type = f
                        .associated_type
                        .as_ref()
                        .map(|t| imported_type_names.contains(t))
                        .unwrap_or(false);

                    let is_wanted = is_public || is_associated_with_imported_type;

                    // Create a unique key for the function to avoid duplicates
                    let func_key = if let Some(ref assoc_type) = f.associated_type {
                        format!("{}.{}", assoc_type, f.name)
                    } else {
                        f.name.clone()
                    };

                    // Import public functions and associated methods
                    if is_wanted && !imported_names.contains(&func_key) {
                        if debug {
                            if is_associated_with_imported_type {
                                doo_debug!(
                                    "LOADER",
                                    "  Importing local associated function: {}.{}",
                                    f.associated_type.as_deref().unwrap_or("?"),
                                    f.name
                                );
                            } else {
                                doo_debug!("LOADER", "  Importing local function: {}", f.name);
                            }
                        }
                        imported_names.insert(func_key);
                        result.items.push(item.clone());
                    }
                }
                Item::Struct(s) => {
                    // Check visibility: PascalCase = public
                    let is_public = s
                        .name
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false);

                    if is_public && !imported_names.contains(&s.name) {
                        if debug {
                            doo_debug!("LOADER", "  Importing local struct: {}", s.name);
                        }
                        imported_names.insert(s.name.clone());
                        result.items.push(item.clone());
                    }
                }
                Item::Enum(e) => {
                    // Check visibility: PascalCase = public
                    let is_public = e
                        .name
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false);

                    if is_public && !imported_names.contains(&e.name) {
                        if debug {
                            doo_debug!("LOADER", "  Importing local enum: {}", e.name);
                        }
                        imported_names.insert(e.name.clone());
                        result.items.push(item.clone());
                    }
                }
                Item::Import(_) | Item::Statement(_) => {
                    // Don't re-export
                }
            }
        }
    }

    if debug {
        doo_debug!("LOADER", "Total imported items: {}", result.items.len());
    }

    Ok(result)
}

/// Merge imported items into a program.
///
/// Prepends imported items before the original items so that
/// imported functions are declared before they're called.
pub fn merge_imports(program: &mut Program, resolution: ImportResolution) {
    if resolution.items.is_empty() {
        return;
    }

    let original_items = std::mem::take(&mut program.items);
    program.items = resolution.items;
    program.items.extend(original_items);
}

/// Capitalize the first character of a string (for suggestions)
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Narrow a full-import span to just the symbol name at the end.
/// e.g. `import defs::types::internalState;` -> caret on `internalState` only.
fn narrow_span_to_symbol(full_span: &Span, sym_name: &str) -> Span {
    let sym_len = sym_name.len() as u32;
    // The symbol is near the end of the span (before the `;`)
    // Approximate: end of span - 1 (semicolon) - sym_len
    let span_len = full_span.end.saturating_sub(full_span.start);
    if span_len > sym_len + 1 {
        let sym_start = full_span.end.saturating_sub(sym_len).saturating_sub(1);
        Span::new(
            full_span.file_id,
            sym_start,
            full_span.end.saturating_sub(1),
        )
    } else {
        *full_span
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_loader_creation() {
        let loader = ModuleLoader::new();
        assert!(loader.cache.is_empty());
    }

    #[test]
    fn test_import_resolution_default() {
        let resolution = ImportResolution::default();
        assert!(resolution.items.is_empty());
        assert!(resolution.errors.is_empty());
    }
}
