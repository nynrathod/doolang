//! Shared Module Loader — Single Source of Truth
//!
//! Common types and functions for resolving imports across modules.
//! Used by both `doo_driver` (full compiler) and `doo_migrate` (migration engine).
//!
//! ## Design
//!
//! - **ImportResolution**: Result of resolving all imports in a program
//! - **merge_imports**: Merge resolved items into the main program
//! - **resolve_module_path**: Resolve an import path to a file on disk
//!
//! The `ModuleLoader` trait and `resolve_imports` implementation are crate-specific:
//! - `doo_driver::loader` has full caching, stdlib discovery, package resolution
//! - `doo_migrate::extract` has simplified resolution (migration-only use case)
//!
//! Both use these shared building blocks.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use doo_core::errors::codes::CompilerError;
use doo_frontend::ast::{Item, Program};

/// Result of resolving all imports in a program.
#[derive(Debug, Default)]
pub struct ImportResolution {
    /// Items to prepend to the program (imported functions, structs, enums).
    pub items: Vec<Item>,
    /// Errors encountered during resolution.
    pub errors: Vec<CompilerError>,
}

/// Merge imported items into a program.
///
/// Prepends imported items before the original items so that
/// imported functions are declared before they're called.
/// Deduplicates by name to avoid conflicts.
pub fn merge_imports(program: &mut Program, resolution: ImportResolution) {
    if resolution.items.is_empty() {
        return;
    }

    // Collect existing names to avoid duplicates
    let existing_names: HashSet<String> = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(f) => Some(f.name.clone()),
            Item::Struct(s) => Some(s.name.clone()),
            Item::Enum(e) => Some(e.name.clone()),
            _ => None,
        })
        .collect();

    let mut new_items: Vec<Item> = Vec::with_capacity(resolution.items.len());
    for item in resolution.items {
        let name = match &item {
            Item::Function(f) => Some(f.name.clone()),
            Item::Struct(s) => Some(s.name.clone()),
            Item::Enum(e) => Some(e.name.clone()),
            _ => None,
        };

        if let Some(n) = name {
            if existing_names.contains(&n) {
                continue;
            }
        }

        new_items.push(item);
    }

    let original_items = std::mem::take(&mut program.items);
    program.items = new_items;
    program.items.extend(original_items);
}

/// Resolve an import path (e.g., `["std", "Math"]`) to a file path on disk.
///
/// Search order:
/// 1. `stdlib_path/{module}.doo` or `stdlib_path/{module}/mod.doo` (for `std::*`)
/// 2. `project_root/{segments...}/{module}.doo` (for project-relative imports)
///
/// Returns `None` if the module file cannot be found.
pub fn resolve_module_path(
    path: &[String],
    project_root: &Path,
    stdlib_path: Option<&Path>,
) -> Option<PathBuf> {
    if path.is_empty() {
        return None;
    }

    // std::Module → stdlib/module.doo or stdlib/module/mod.doo
    if path[0] == "std" {
        if path.len() < 2 {
            return None; // "std" alone is invalid
        }
        if let Some(stdlib) = stdlib_path {
            let module_name = path[1].to_lowercase();
            let file = stdlib.join(format!("{}.doo", module_name));
            if file.exists() {
                return Some(file);
            }
            let mod_file = stdlib.join(&module_name).join("mod.doo");
            if mod_file.exists() {
                return Some(mod_file);
            }
        }
        return None;
    }

    // Relative import: look in project directory.
    // Try original case first, then lowercase (imports are case-insensitive).
    let mut file_path = project_root.to_path_buf();
    for segment in &path[..path.len().saturating_sub(1)] {
        file_path = file_path.join(segment.to_lowercase());
    }
    let last = path.last().unwrap();
    let last_lower = last.to_lowercase();

    // 1) Try original case
    let file_original = file_path.join(format!("{}.doo", last));
    if file_original.exists() {
        return Some(file_original);
    }
    // 2) Try lowercase (case-insensitive fallback)
    if last != &last_lower {
        let file_lower = file_path.join(format!("{}.doo", last_lower));
        if file_lower.exists() {
            return Some(file_lower);
        }
    }

    // mod.doo variants
    let mod_original = file_path.join(last).join("mod.doo");
    if mod_original.exists() {
        return Some(mod_original);
    }
    if last != &last_lower {
        let mod_lower = file_path.join(&last_lower).join("mod.doo");
        if mod_lower.exists() {
            return Some(mod_lower);
        }
    }

    None
}
