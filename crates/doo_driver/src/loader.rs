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
//! - Shared types (`ImportResolution`, `merge_imports`) from `doo_analysis::loader`

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

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

// Shared loader types — single source of truth
pub use doo_analysis::loader::{merge_imports, resolve_module_path, ImportResolution};

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
    /// Project root hint (directory of the entry point), also searched for std/
    project_root: Option<PathBuf>,
    /// Debug mode
    debug: bool,
    /// Next file_id to assign (main.doo is 0, imports start at 1)
    next_file_id: u32,
    /// Registered source files: (file_id, display_name, source_content)
    /// These must be added to SourceMap after import resolution.
    imported_sources: Vec<(u32, String, String)>,
}

/// Check if a candidate std/ directory is valid (contains .doo source files, not
/// just compiled FFI artifacts like .rs/.dll/.so).
fn is_valid_stdlib(path: &Path) -> bool {
    // Check for at least one known stdlib .doo file
    path.join("Array.doo").exists()
}

impl ModuleLoader {
    /// Create a new module loader.
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            stdlib_path: None,
            project_root: None,
            debug: env::var("DOO_DEBUG").is_ok(),
            next_file_id: 1, // 0 is reserved for main.doo
            imported_sources: Vec::new(),
        }
    }

    /// Set a project root hint for stdlib discovery.
    /// When the entry point is in a subdirectory (e.g. src/main.doo), this
    /// directs the loader to also search upward from that directory.
    pub fn set_project_root(&mut self, root: PathBuf) {
        self.project_root = Some(root);
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
    /// 2. Next to the executable (production: go install layout)
    /// 3. Search up from project_root (entry point parent dir tree)
    /// 4. Search up directory tree from cwd (development)
    /// 5. Relative ./std (CI/testing)
    ///
    /// IMPORTANT: A directory named "std/" is only accepted if it contains
    /// `.doo` source files (e.g., Array.doo). This prevents accidentally
    /// picking up a project-local std/ that has only compiled FFI artifacts.
    pub fn resolve_stdlib_path(&mut self) -> Result<&Path, String> {
        if self.stdlib_path.is_some() {
            return Ok(self.stdlib_path.as_ref().unwrap());
        }

        // 1. Explicit env var
        if let Ok(stdlib_env) = env::var("DOO_STDLIB_PATH") {
            let path = PathBuf::from(&stdlib_env);
            if path.exists() && is_valid_stdlib(&path) {
                self.stdlib_path = Some(path);
                return Ok(self.stdlib_path.as_ref().unwrap());
            }
        }

        // 2. Next to executable (production: go install copies std/ next to doo binary)
        if let Ok(exe_path) = env::current_exe() {
            // 2a. Direct sibling: ~/.doo/bin/doo → ~/.doo/std/
            if let Some(exe_dir) = exe_path.parent() {
                let stdlib_dir = exe_dir.join("std");
                if stdlib_dir.exists() && is_valid_stdlib(&stdlib_dir) {
                    self.stdlib_path = Some(stdlib_dir);
                    return Ok(self.stdlib_path.as_ref().unwrap());
                }
            }
            // 2b. Search up from exe (dev builds: target/release/doo.exe → project root has std/)
            let mut exe_current = exe_path.clone();
            for _ in 0..10 {
                if !exe_current.pop() {
                    break;
                }
                let stdlib_dir = exe_current.join("std");
                if stdlib_dir.exists() && is_valid_stdlib(&stdlib_dir) {
                    self.stdlib_path = Some(stdlib_dir);
                    return Ok(self.stdlib_path.as_ref().unwrap());
                }
            }
        }

        // 3. Search up from project_root (catches subdirectory entry points like src/main.doo)
        if let Some(ref project_root) = self.project_root {
            let mut current = project_root.clone();
            for _ in 0..20 {
                let stdlib_dir = current.join("std");
                if stdlib_dir.exists() && is_valid_stdlib(&stdlib_dir) {
                    self.stdlib_path = Some(stdlib_dir);
                    return Ok(self.stdlib_path.as_ref().unwrap());
                }
                if !current.pop() {
                    break;
                }
            }
        }

        // 4. Search up from cwd
        if let Ok(current_dir) = env::current_dir() {
            let mut current = current_dir;
            for _ in 0..20 {
                let stdlib_dir = current.join("std");
                if stdlib_dir.exists() && is_valid_stdlib(&stdlib_dir) {
                    self.stdlib_path = Some(stdlib_dir);
                    return Ok(self.stdlib_path.as_ref().unwrap());
                }
                if !current.pop() {
                    break;
                }
            }
        }

        // 5. Relative fallback
        let dev_stdlib = PathBuf::from("./std");
        if dev_stdlib.exists() && is_valid_stdlib(&dev_stdlib) {
            self.stdlib_path = Some(dev_stdlib);
            return Ok(self.stdlib_path.as_ref().unwrap());
        }

        Err(
            "Could not find stdlib directory. Set DOO_STDLIB_PATH or run from project root."
                .to_string(),
        )
    }

    /// Resolve a module from the packages/ directory.
    ///
    /// Searches `packages/*/` (sibling of std/) for `{module_name}.doo`.
    /// This is entirely discovery-based — no hardcoded package names.
    fn resolve_package_module(&self, stdlib: &Path, module_name: &str) -> Result<PathBuf, String> {
        if let Some(parent) = stdlib.parent() {
            let packages_dir = parent.join("packages");
            if packages_dir.exists() {
                // Scan all subdirectories for {module_name}.doo
                if let Ok(entries) = fs::read_dir(&packages_dir) {
                    for entry in entries.flatten() {
                        if entry.path().is_dir() {
                            let candidate = entry.path().join(format!("{}.doo", module_name));
                            if candidate.exists() {
                                return Ok(candidate);
                            }
                        }
                    }
                }
            }
        }

        Err(format!(
            "Module '{}' not found in std/ or packages/",
            module_name
        ))
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
                // Clone to release the mutable borrow on self
                let stdlib = self.resolve_stdlib_path()?.to_path_buf();
                let std_file = stdlib.join(format!("{}.doo", module_name));
                if std_file.exists() {
                    std_file
                } else {
                    // Fallback: search packages/ directory (sibling of std/)
                    self.resolve_package_module(&stdlib, module_name)?
                }
            }
            _ => {
                // TODO: Support project-relative imports
                return Err(format!("Unsupported namespace: {}", namespace));
            }
        };

        if self.debug {}

        // Read and parse
        let source = fs::read_to_string(&module_file)
            .map_err(|e| format!("Failed to read {}: {}", module_file.display(), e))?;

        let file_id = self.allocate_file_id(
            module_file
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(module_key),
            &source,
        );
        let mut parser = Parser::new(&source, file_id);
        let program = parser.parse_program().map_err(|e| {
            let msg = e
                .first()
                .map(|err| err.message.clone())
                .unwrap_or_else(|| "unknown parse error".to_string());
            format!("Failed to parse {}: {}", module_key, msg)
        })?;

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

    /// Allocate a unique file_id for an imported module and register its source.
    /// Returns the assigned file_id.
    fn allocate_file_id(&mut self, display_name: &str, source: &str) -> u32 {
        let file_id = self.next_file_id;
        self.next_file_id += 1;
        self.imported_sources
            .push((file_id, display_name.to_string(), source.to_string()));
        file_id
    }

    /// Get all imported source files for registration in the SourceMap.
    /// Returns (file_id, display_name, source) triples sorted by file_id.
    pub fn imported_sources(&self) -> &[(u32, String, String)] {
        &self.imported_sources
    }
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

    // Set project_root on loader so stdlib discovery can search from entry point
    loader.set_project_root(project_root.to_path_buf());

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

    if debug {}

    // Build import requests: module_key -> set of (symbol_name, (optional_alias, span))
    // IMPORTANT: Use a separate Vec to preserve source-order for processing.
    // HashMap iteration order is non-deterministic, which would break cross-module
    // extension methods (e.g., Server.oauth from Auth.doo needs Server from Http.doo
    // to be imported first).
    let mut std_import_requests: HashMap<
        String,
        HashMap<String, (Option<String>, doo_core::Span)>,
    > = HashMap::new();
    let mut std_import_order: Vec<String> = Vec::new();
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
            if !std_import_requests.contains_key(&module_key) {
                std_import_order.push(module_key.clone());
            }
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
                // import ModuleName with no explicit items → import all
                if import.items.is_empty() && !import.wildcard {
                    let mut symbols_with_all: Vec<String> = path_symbols.clone();
                    symbols_with_all.push("*".to_string());
                    local_import_requests.push((import, module_path, symbols_with_all));
                } else {
                    local_import_requests.push((import, module_path, path_symbols));
                }
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
                    if debug {}
                    path_symbols.push(symbol);
                    local_import_requests.push((import, alt_path, path_symbols));
                } else if debug {
                }
            } else if debug {
            }
        }
    }

    // Load modules and extract symbols
    let mut result = ImportResolution::default();
    let mut imported_names: HashSet<String> = HashSet::new();

    // Process standard library imports (in source order — NOT HashMap order!)
    // Source order ensures that cross-module extension methods work correctly:
    // e.g., `import std::Http::{Server}` must be processed before `import std::Auth::{Jwt}`
    // so that Server.oauth from Auth.doo is found via the cross-module type check.
    for module_key in &std_import_order {
        let requested = match std_import_requests.get(module_key) {
            Some(r) => r,
            None => continue,
        };
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
                    doo_core::Span::dummy(),
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
                    // Auto-import the module's primary struct (name matches module key)
                    // This allows `import std::Database::{DatabaseError}` to also make
                    // Database::postgres() and Database::get() available
                    let is_primary_struct = s.name == *module_key;
                    let is_wanted =
                        import_all || requested.contains_key(&s.name) || is_primary_struct;
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

        // Also include types from previously imported modules so that
        // cross-module extension methods get imported automatically.
        // E.g., `Server` from Http.doo should allow `Server.oauth` from Auth.doo
        // to be imported when `import std::Auth::{Jwt}` is used.
        for item in &result.items {
            match item {
                Item::Struct(s) => {
                    imported_type_names.insert(s.name.clone());
                }
                Item::Enum(e) => {
                    imported_type_names.insert(e.name.clone());
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

                    // Create a unique key for the function to avoid duplicates.
                    // Include param count so overloaded methods (same name, different arity)
                    // can coexist — e.g., Server.oauth with 2 params and Server.oauth with 3 params.
                    let func_key = if let Some(ref assoc_type) = f.associated_type {
                        format!("{}.{}:{}", assoc_type, f.name, f.params.len())
                    } else {
                        format!("{}:{}", f.name, f.params.len())
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
                            } else {
                            }
                        }
                        imported_names.insert(func_key);

                        // Check if this function has an alias
                        // e.g., `import std::Math::{Sqrt as Sq}` or `import std::Math::Floor as Fl`
                        // If so, push both the original (for namespace access) and a renamed copy
                        if let Some((Some(alias), _)) = requested.get(&f.name) {
                            if debug {}
                            // Push original for qualified access (e.g., Math::Sqrt)
                            result.items.push(item.clone());
                            // Push aliased copy for direct access (e.g., Sq)
                            let mut aliased_func = f.clone();
                            aliased_func.name = alias.clone();
                            result.items.push(Item::Function(aliased_func));
                        } else {
                            result.items.push(item.clone());
                        }
                    }
                }
                Item::Struct(s) => {
                    let is_primary_struct = s.name == *module_key;
                    let is_wanted =
                        import_all || requested.contains_key(&s.name) || is_primary_struct;
                    if is_wanted && !imported_names.contains(&s.name) {
                        if debug {}
                        imported_names.insert(s.name.clone());
                        result.items.push(item.clone());
                    }
                }
                Item::Enum(e) => {
                    let is_wanted = import_all || requested.contains_key(&e.name);
                    if is_wanted && !imported_names.contains(&e.name) {
                        if debug {}
                        imported_names.insert(e.name.clone());
                        result.items.push(item.clone());
                    }
                }
                Item::Const(c) => {
                    let is_public = c
                        .name
                        .chars()
                        .next()
                        .map(|ch| ch.is_uppercase())
                        .unwrap_or(false);
                    let is_wanted = import_all || requested.contains_key(&c.name);
                    if is_public && is_wanted && !imported_names.contains(&c.name) {
                        if debug {}
                        imported_names.insert(c.name.clone());
                        result.items.push(item.clone());
                    }
                }
                Item::Static(s) => {
                    let is_public = s.is_public;
                    let is_wanted = import_all || requested.contains_key(&s.name);
                    if is_public && is_wanted && !imported_names.contains(&s.name) {
                        if debug {}
                        imported_names.insert(s.name.clone());
                        result.items.push(item.clone());
                    }
                }
                Item::Import(_) | Item::Statement(_) | Item::Impl(_) => {
                    // Don't re-export
                }
                Item::Policy(p) => {
                    if !imported_names.contains(&p.name) {
                        imported_names.insert(p.name.clone());
                        result.items.push(item.clone());
                    }
                }
                Item::Interface(i) => {
                    let is_wanted = import_all || requested.contains_key(&i.name);
                    if is_wanted && !imported_names.contains(&i.name) {
                        imported_names.insert(i.name.clone());
                        result.items.push(item.clone());
                    }
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

    // Track std imports discovered in nested local modules
    // These need to be processed after the local module loop
    let mut nested_std_import_requests: HashMap<
        String,
        HashMap<String, (Option<String>, doo_core::Span)>,
    > = HashMap::new();
    let mut nested_std_import_order: Vec<String> = Vec::new();

    while let Some((module_path, _path_symbols, import_chain, origin_span)) = pending_modules.pop()
    {
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
                    doo_core::Span::dummy(),
                ));
                continue;
            }
        };

        let module_display_name = module_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("module.doo");
        let file_id = loader.allocate_file_id(module_display_name, &source);
        let mut parser = Parser::new(&source, file_id);
        let module_program = match parser.parse_program() {
            Ok(p) => p,
            Err(e) => {
                let msg = e
                    .first()
                    .map(|err| err.message.clone())
                    .unwrap_or_else(|| "unknown parse error".to_string());
                result.errors.push(CompilerError::new(
                    ErrorCode::IoError,
                    format!(
                        "failed to parse module '{}': {}",
                        module_path.display(),
                        msg
                    ),
                    doo_core::Span::dummy(),
                ));
                continue;
            }
        };

        // Check for parser errors even on Ok result
        if !parser.errors().is_empty() {
            for err in parser.errors() {}
        }

        // DEBUG: Show parsed functions and their bodies
        if debug {
            for item in &module_program.items {
                if let Item::Function(f) = item {
                    for (i, stmt) in f.body.iter().enumerate() {}
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

                // Collect std library imports from nested modules for deferred processing
                if nested_import.path[0] == "std" {
                    if nested_import.path.len() >= 2 {
                        let module_name = &nested_import.path[1];
                        let module_key = format!("std::{}", module_name);
                        if !nested_std_import_requests.contains_key(&module_key) {
                            nested_std_import_order.push(module_key.clone());
                        }
                        let symbols = nested_std_import_requests.entry(module_key).or_default();

                        let path_symbol = if nested_import.path.len() >= 3 {
                            Some(nested_import.path[2].clone())
                        } else {
                            None
                        };

                        if let Some(sym) = path_symbol {
                            if nested_import.wildcard {
                                symbols.insert("*".to_string(), (None, nested_import.span));
                            } else {
                                symbols
                                    .insert(sym, (nested_import.alias.clone(), nested_import.span));
                            }
                        } else if nested_import.alias.is_some() {
                            symbols.insert(
                                "*".to_string(),
                                (nested_import.alias.clone(), nested_import.span),
                            );
                        } else if nested_import.wildcard {
                            symbols.insert("*".to_string(), (None, nested_import.span));
                        } else if nested_import.items.is_empty() {
                            symbols.insert("*".to_string(), (None, nested_import.span));
                        } else {
                            for item_sym in &nested_import.items {
                                match item_sym {
                                    ImportItem::Symbol(name) => {
                                        symbols.insert(name.clone(), (None, nested_import.span));
                                    }
                                    ImportItem::Alias { name, alias } => {
                                        symbols.insert(
                                            name.clone(),
                                            (Some(alias.clone()), nested_import.span),
                                        );
                                    }
                                    ImportItem::Wildcard => {
                                        symbols.insert("*".to_string(), (None, nested_import.span));
                                    }
                                }
                            }
                        }
                    }
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
                        if debug {}
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

                    // Create a unique key for the function to avoid duplicates.
                    // Include param count so overloaded methods can coexist.
                    let func_key = if let Some(ref assoc_type) = f.associated_type {
                        format!("{}.{}:{}", assoc_type, f.name, f.params.len())
                    } else {
                        format!("{}:{}", f.name, f.params.len())
                    };

                    // Import public functions and associated methods
                    if is_wanted && !imported_names.contains(&func_key) {
                        if debug {
                            if is_associated_with_imported_type {
                            } else {
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
                        if debug {}
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
                        if debug {}
                        imported_names.insert(e.name.clone());
                        result.items.push(item.clone());
                    }
                }
                Item::Const(c) => {
                    let is_public = c
                        .name
                        .chars()
                        .next()
                        .map(|ch| ch.is_uppercase())
                        .unwrap_or(false);
                    if is_public && !imported_names.contains(&c.name) {
                        if debug {}
                        imported_names.insert(c.name.clone());
                        result.items.push(item.clone());
                    }
                }
                Item::Static(s) => {
                    let is_public = s.is_public;
                    if is_public && !imported_names.contains(&s.name) {
                        if debug {}
                        imported_names.insert(s.name.clone());
                        result.items.push(item.clone());
                    }
                }
                Item::Import(_) | Item::Statement(_) | Item::Impl(_) => {
                    // Don't re-export
                }
                Item::Policy(p) => {
                    if !imported_names.contains(&p.name) {
                        imported_names.insert(p.name.clone());
                        result.items.push(item.clone());
                    }
                }
                Item::Interface(i) => {
                    let is_wanted = import_all;
                    if is_wanted && !imported_names.contains(&i.name) {
                        imported_names.insert(i.name.clone());
                        result.items.push(item.clone());
                    }
                }
            }
        }
    }

    // Process std imports discovered in nested local modules
    if !nested_std_import_order.is_empty() {
        if debug {}
        for module_key in &nested_std_import_order {
            let requested = match nested_std_import_requests.get(module_key) {
                Some(r) => r,
                None => continue,
            };
            // Load the std module
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
                        doo_core::Span::dummy(),
                    ));
                    continue;
                }
            };

            let import_all = requested.contains_key("*");

            // First pass: collect struct/enum type names for associated function resolution
            let mut imported_type_names: HashSet<String> = HashSet::new();
            for item in &module_program.items {
                match item {
                    Item::Struct(s) => {
                        let is_primary_struct = s.name == *module_key;
                        let is_wanted =
                            import_all || requested.contains_key(&s.name) || is_primary_struct;
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

            // Include types from previously imported items for cross-module methods
            for item in &result.items {
                match item {
                    Item::Struct(s) => {
                        imported_type_names.insert(s.name.clone());
                    }
                    Item::Enum(e) => {
                        imported_type_names.insert(e.name.clone());
                    }
                    _ => {}
                }
            }

            // Second pass: extract requested items
            for item in &module_program.items {
                match item {
                    Item::Function(f) => {
                        let is_public = f
                            .name
                            .chars()
                            .next()
                            .map(|c| c.is_uppercase())
                            .unwrap_or(false);

                        let is_associated_with_imported_type = f
                            .associated_type
                            .as_ref()
                            .map(|t| imported_type_names.contains(t))
                            .unwrap_or(false);

                        let is_explicitly_requested = requested.contains_key(&f.name);

                        let is_wanted = import_all
                            || is_explicitly_requested
                            || is_associated_with_imported_type;

                        let func_key = if let Some(ref assoc_type) = f.associated_type {
                            format!("{}.{}:{}", assoc_type, f.name, f.params.len())
                        } else {
                            format!("{}:{}", f.name, f.params.len())
                        };

                        if (is_explicitly_requested
                            || is_public
                            || is_associated_with_imported_type)
                            && is_wanted
                            && !imported_names.contains(&func_key)
                        {
                            if debug {}
                            imported_names.insert(func_key);

                            if let Some((Some(alias), _)) = requested.get(&f.name) {
                                result.items.push(item.clone());
                                let mut aliased_func = f.clone();
                                aliased_func.name = alias.clone();
                                result.items.push(Item::Function(aliased_func));
                            } else {
                                result.items.push(item.clone());
                            }
                        }
                    }
                    Item::Struct(s) => {
                        let is_primary_struct = s.name == *module_key;
                        let is_wanted =
                            import_all || requested.contains_key(&s.name) || is_primary_struct;
                        if is_wanted && !imported_names.contains(&s.name) {
                            if debug {}
                            imported_names.insert(s.name.clone());
                            result.items.push(item.clone());
                        }
                    }
                    Item::Enum(e) => {
                        let is_wanted = import_all || requested.contains_key(&e.name);
                        if is_wanted && !imported_names.contains(&e.name) {
                            if debug {}
                            imported_names.insert(e.name.clone());
                            result.items.push(item.clone());
                        }
                    }
                    Item::Const(c) => {
                        let is_public = c
                            .name
                            .chars()
                            .next()
                            .map(|ch| ch.is_uppercase())
                            .unwrap_or(false);
                        let is_wanted = import_all || requested.contains_key(&c.name);
                        if is_public && is_wanted && !imported_names.contains(&c.name) {
                            if debug {}
                            imported_names.insert(c.name.clone());
                            result.items.push(item.clone());
                        }
                    }
                    Item::Static(s) => {
                        let is_public = s.is_public;
                        let is_wanted = import_all || requested.contains_key(&s.name);
                        if is_public && is_wanted && !imported_names.contains(&s.name) {
                            if debug {}
                            imported_names.insert(s.name.clone());
                            result.items.push(item.clone());
                        }
                    }
                    Item::Import(_) | Item::Statement(_) | Item::Impl(_) => {}
                    Item::Policy(p) => {
                        if !imported_names.contains(&p.name) {
                            imported_names.insert(p.name.clone());
                            result.items.push(item.clone());
                        }
                    }
                    Item::Interface(i) => {
                        let is_wanted = import_all || requested.contains_key(&i.name);
                        if is_wanted && !imported_names.contains(&i.name) {
                            imported_names.insert(i.name.clone());
                            result.items.push(item.clone());
                        }
                    }
                }
            }
        }
    }

    if debug {}

    Ok(result)
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
    let span_len = full_span.end.saturating_sub(full_span.start);
    if span_len > sym_len + 1 {
        let sym_start = full_span.end.saturating_sub(sym_len).saturating_sub(1);
        Span::new(sym_start, full_span.end.saturating_sub(1))
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
