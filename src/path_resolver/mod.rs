use std::env;
use std::path::{Path, PathBuf};

/// Path resolver for the Doo import system
/// Handles resolving module paths from both dev and production environments
pub struct PathResolver {
    stdlib_path: PathBuf,
    project_root: PathBuf,
}

impl PathResolver {
    /// Create a new PathResolver
    ///
    /// Environment variables (checked in order):
    /// - DOO_STDLIB_PATH: Custom std location
    /// - DOO_PROJECT_ROOT: Custom project root
    ///
    /// Defaults:
    /// - std: ./std (dev) or relative to executable (production)
    /// - project_root: current working directory
    pub fn new() -> Result<Self, String> {
        let stdlib_path = Self::resolve_stdlib_path()?;
        let project_root = Self::resolve_project_root()?;

        Ok(Self {
            stdlib_path,
            project_root,
        })
    }

    /// Resolve the standard library path
    fn resolve_stdlib_path() -> Result<PathBuf, String> {
        // 1. Check explicit env var first
        if let Ok(stdlib_env) = env::var("DOO_STDLIB_PATH") {
            let path = PathBuf::from(&stdlib_env);
            if path.exists() {
                return Ok(path);
            }
        }

        // 2. FIRST: Look next to executable (production - THIS SHOULD BE FIRST)
        if let Ok(exe_path) = env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let stdlib_dir = exe_dir.join("std");
                if stdlib_dir.exists() {
                    return Ok(stdlib_dir);
                }
            }
        }

        // 3. Fallback to relative paths (dev)
        let dev_stdlib = PathBuf::from("./std");
        if dev_stdlib.exists() {
            return Ok(dev_stdlib);
        }

        let parent_stdlib = PathBuf::from("../std");
        if parent_stdlib.exists() {
            return Ok(parent_stdlib);
        }

        Err("Could not find stdlib directory".to_string())
    }

    /// Resolve the project root path
    fn resolve_project_root() -> Result<PathBuf, String> {
        if let Ok(custom_root) = env::var("DOO_PROJECT_ROOT") {
            let path = PathBuf::from(&custom_root);
            if path.exists() {
                return Ok(path);
            }
            return Err(format!(
                "DOO_PROJECT_ROOT points to non-existent directory: {}",
                custom_root
            ));
        }

        env::current_dir().map_err(|e| format!("Could not determine project root: {}", e))
    }

    /// Resolve a module path for import
    ///
    /// Examples:
    /// - "std:Array" -> resolves to std/Array.doo
    /// - "mymodule" -> resolves to ./mymodule.doo or ./mymodule/main.doo
    pub fn resolve_module(&self, module_path: &str) -> Result<PathBuf, String> {
        // Handle std imports: "std:ModuleName"
        if module_path.starts_with("std:") {
            let module_name = &module_path[4..];
            let stdlib_file = self.stdlib_path.join(format!("{}.doo", module_name));
            if stdlib_file.exists() {
                return Ok(stdlib_file);
            }
            return Err(format!(
                "Standard library module not found: {}",
                module_name
            ));
        }

        // Handle project-relative imports
        // First try as file: ./mymodule.doo
        let file_path = self.project_root.join(format!("{}.doo", module_path));
        if file_path.exists() {
            return Ok(file_path);
        }

        // Try as directory with main.doo: ./mymodule/main.doo
        let dir_path = self.project_root.join(module_path).join("main.doo");
        if dir_path.exists() {
            return Ok(dir_path);
        }

        Err(format!("Module not found: {}", module_path))
    }

    /// Get the std path
    pub fn stdlib_path(&self) -> &Path {
        &self.stdlib_path
    }

    /// Get the project root path
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// List all available std modules
    pub fn list_stdlib_modules(&self) -> Result<Vec<String>, String> {
        let entries = std::fs::read_dir(&self.stdlib_path)
            .map_err(|e| format!("Could not read std directory: {}", e))?;

        let modules = entries
            .filter_map(|entry| {
                entry.ok().and_then(|e| {
                    let path = e.path();
                    if path.extension().map(|ext| ext == "doo").unwrap_or(false) {
                        path.file_stem()
                            .and_then(|stem| stem.to_str().map(|s| s.to_string()))
                    } else {
                        None
                    }
                })
            })
            .collect();

        Ok(modules)
    }
}

impl Default for PathResolver {
    fn default() -> Self {
        Self::new().expect("Failed to initialize PathResolver with default settings")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stdlib_module_resolution() {
        if let Ok(resolver) = PathResolver::new() {
            // This will work if std exists
            let result = resolver.resolve_module("std:Array");
            println!("Stdlib resolution: {:?}", result);
        }
    }
}
