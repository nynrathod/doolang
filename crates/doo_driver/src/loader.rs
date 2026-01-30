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
            eprintln!(
                "[LOADER] Loading module: {} from {}",
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
    pub errors: Vec<String>,
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
        eprintln!("[LOADER] Resolving {} imports", imports.len());
    }

    // Build import requests: module_key -> set of (symbol_name, optional_alias)
    let mut std_import_requests: HashMap<String, HashMap<String, Option<String>>> = HashMap::new();
    let mut local_import_requests: Vec<(&ImportDecl, PathBuf)> = Vec::new();

    for import in &imports {
        if import.path.is_empty() || import.path.len() < 2 {
            continue;
        }

        if import.path[0] == "std" {
            // Standard library import
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
                    symbols.insert("*".to_string(), None);
                } else {
                    // import std::Math::Abs or import std::Math::Abs as A
                    symbols.insert(sym, import.alias.clone());
                }
            } else if import.alias.is_some() {
                // import std::Array as A - namespace alias, import all
                symbols.insert("*".to_string(), import.alias.clone());
            } else if import.wildcard {
                // import std::Array::*
                symbols.insert("*".to_string(), None);
            } else if import.items.is_empty() {
                // import std::File - namespace import, import all
                symbols.insert("*".to_string(), None);
            } else {
                // import std::Math::{Min, Max, Sqrt as Sq}
                for item in &import.items {
                    match item {
                        ImportItem::Symbol(name) => {
                            symbols.insert(name.clone(), None);
                        }
                        ImportItem::Alias { name, alias } => {
                            symbols.insert(name.clone(), Some(alias.clone()));
                        }
                        ImportItem::Wildcard => {
                            symbols.insert("*".to_string(), None);
                        }
                    }
                }
            }
        } else {
            // Local module import (e.g., import defs::types::{PublicUser})
            // Build path: defs/types.doo relative to project_root
            let mut module_path = project_root.to_path_buf();

            // path[0] = "defs", path[1] = "types", items = [PublicUser, CreateUser]
            // Build path up to but not including the last element (which is the file)
            for i in 0..import.path.len() - 1 {
                module_path.push(&import.path[i]);
            }

            // Last path element is the file name
            module_path.push(&import.path[import.path.len() - 1]);
            module_path.set_extension("doo");

            if module_path.exists() {
                if debug {
                    eprintln!("[LOADER] Found local module: {}", module_path.display());
                }
                local_import_requests.push((import, module_path));
            } else {
                if debug {
                    eprintln!("[LOADER] Local module not found: {}", module_path.display());
                }
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
                result.errors.push(e);
                continue;
            }
        };

        let import_all = requested.contains_key("*");

        // Extract requested items
        for item in &module_program.items {
            match item {
                Item::Function(f) => {
                    // Public = starts with uppercase
                    let is_public = f
                        .name
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false);
                    let is_wanted = import_all || requested.contains_key(&f.name);

                    if is_public && is_wanted && !imported_names.contains(&f.name) {
                        if debug {
                            eprintln!("[LOADER]   Importing function: {}", f.name);
                        }
                        imported_names.insert(f.name.clone());
                        result.items.push(item.clone());
                    }
                }
                Item::Struct(s) => {
                    let is_wanted = import_all || requested.contains_key(&s.name);
                    if is_wanted && !imported_names.contains(&s.name) {
                        if debug {
                            eprintln!("[LOADER]   Importing struct: {}", s.name);
                        }
                        imported_names.insert(s.name.clone());
                        result.items.push(item.clone());
                    }
                }
                Item::Enum(e) => {
                    let is_wanted = import_all || requested.contains_key(&e.name);
                    if is_wanted && !imported_names.contains(&e.name) {
                        if debug {
                            eprintln!("[LOADER]   Importing enum: {}", e.name);
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
    for (import, module_path) in &local_import_requests {
        // Read and parse the local module
        let source = match fs::read_to_string(module_path) {
            Ok(s) => s,
            Err(e) => {
                result.errors.push(format!(
                    "Failed to read module {}: {}",
                    module_path.display(),
                    e
                ));
                continue;
            }
        };

        let mut parser = Parser::new(&source, 0);
        let module_program = match parser.parse_program() {
            Ok(p) => p,
            Err(e) => {
                result.errors.push(format!(
                    "Failed to parse module {}: {}",
                    module_path.display(),
                    e
                ));
                continue;
            }
        };

        // Determine what symbols to import
        let import_all = import.wildcard;
        let requested_symbols: HashSet<String> = import
            .items
            .iter()
            .filter_map(|item| match item {
                ImportItem::Symbol(name) => Some(name.clone()),
                ImportItem::Alias { name, .. } => Some(name.clone()),
                ImportItem::Wildcard => None,
            })
            .collect();

        // Extract requested items
        for item in &module_program.items {
            match item {
                Item::Function(f) => {
                    // Check visibility: PascalCase = public
                    let is_public = f
                        .name
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false);
                    let is_wanted = import_all || requested_symbols.contains(&f.name);

                    if is_public && is_wanted && !imported_names.contains(&f.name) {
                        if debug {
                            eprintln!("[LOADER]   Importing local function: {}", f.name);
                        }
                        imported_names.insert(f.name.clone());
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
                    let is_wanted = import_all || requested_symbols.contains(&s.name);

                    if is_public && is_wanted && !imported_names.contains(&s.name) {
                        if debug {
                            eprintln!("[LOADER]   Importing local struct: {}", s.name);
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
                    let is_wanted = import_all || requested_symbols.contains(&e.name);

                    if is_public && is_wanted && !imported_names.contains(&e.name) {
                        if debug {
                            eprintln!("[LOADER]   Importing local enum: {}", e.name);
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
        eprintln!("[LOADER] Total imported items: {}", result.items.len());
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
