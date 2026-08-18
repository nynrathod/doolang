//! CompileSession — the shared context passed to every compiler pass.
//!
//! Bundles references to the global, immutable compilation state:
//! arena, interner, type registry, source map, query cache.
//! Every compiler pass (Parser, HIR, THIR, MIR, Codegen) receives
//! `&CompileSession` to access shared resources.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use doo_core::arena::CompilerArena;
use doo_core::constants::env_vars;
use doo_core::errors::codes::CompilerError;
use doo_core::intern::Interner;
use doo_core::query::QueryCache;
use doo_core::span::FileId;
use doo_core::types::TypeRegistry;
use doo_diagnostics::{DiagnosticEmitter, SourceMap};

// ============================================================================
// Compile Options
// ============================================================================

/// Optimization level for code generation.
///
/// Maps to LLVM optimization pipelines:
/// - None: fastest compilation, no optimization
/// - Aggressive: maximum performance (default)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OptLevel {
    None,
    Less,
    #[default]
    Default,
    Aggressive,
    Size,
    MinSize,
}

impl OptLevel {
    /// Parse from CLI string ("0", "1", "2", "3", "s", "z").
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "0" | "O0" | "o0" => Some(Self::None),
            "1" | "O1" | "o1" => Some(Self::Less),
            "2" | "O2" | "o2" => Some(Self::Default),
            "3" | "O3" | "o3" => Some(Self::Aggressive),
            "s" | "Os" | "os" => Some(Self::Size),
            "z" | "Oz" | "oz" => Some(Self::MinSize),
            _ => None,
        }
    }
}

/// Options controlling compilation behavior.
#[derive(Debug, Clone)]
pub struct CompileOptions {
    /// Optimization level.
    pub opt_level: OptLevel,
    /// Generate DWARF debug information.
    pub debug: bool,
    /// Print verbose progress information.
    pub verbose: bool,
    /// Unstable feature flags from doo.toml.
    pub unstable_features: Vec<String>,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            opt_level: OptLevel::default(),
            debug: false,
            verbose: false,
            unstable_features: Vec::new(),
        }
    }
}

// ============================================================================
// Project Paths
// ============================================================================

/// Filesystem paths for a compilation unit.
#[derive(Debug, Clone)]
pub struct ProjectPaths {
    /// Project root directory.
    pub root: PathBuf,
    /// Source directory (usually root/src).
    pub src: PathBuf,
    /// Output directory for compiled binaries.
    pub out: PathBuf,
}

// ============================================================================
// Target Triple
// ============================================================================

/// Target platform triple (arch-vendor-os-env).
///
/// Defaults to the host platform. Used for cross-compilation target selection.
#[derive(Debug, Clone)]
pub struct TargetTriple {
    pub arch: String,
    pub vendor: String,
    pub os: String,
    pub env: String,
}

impl TargetTriple {
    /// Detect the host platform.
    pub fn host() -> Self {
        let arch = std::env::consts::ARCH.to_string();
        let os = std::env::consts::OS.to_string();

        let (vendor, env) = match os.as_str() {
            "linux" => ("unknown".to_string(), "gnu".to_string()),
            "macos" => ("apple".to_string(), String::new()),
            "windows" => ("pc".to_string(), "msvc".to_string()),
            _ => ("unknown".to_string(), String::new()),
        };

        Self {
            arch,
            vendor,
            os,
            env,
        }
    }

    /// Format as a standard target triple string.
    pub fn to_triple_string(&self) -> String {
        if self.env.is_empty() {
            format!("{}-{}-{}", self.arch, self.vendor, self.os)
        } else {
            format!("{}-{}-{}-{}", self.arch, self.vendor, self.os, self.env)
        }
    }
}

impl Default for TargetTriple {
    fn default() -> Self {
        Self::host()
    }
}

// ============================================================================
// Package Graph
// ============================================================================

/// Package dependency graph resolved from doo.toml.
///
/// Populated in Phase 52 when doo.toml parsing is implemented.
#[derive(Debug, Clone, Default)]
pub struct PackageGraph {
    /// Names of all resolved packages.
    pub packages: Vec<String>,
}

// ============================================================================
// CompileSession
// ============================================================================

/// Shared compilation context passed to every compiler pass.
///
/// Owns the arena, interner, type registry, source map, and query cache.
/// Created once at the start of compilation and passed by reference
/// to every pipeline stage.
///
/// ## Design
///
/// - **Zero-Cost**: Passing `&CompileSession` is a single pointer copy.
/// - **Immutable**: Core state is read-only during compilation.
/// - **Single Source of Truth**: All shared resources live here.
pub struct CompileSession {
    /// Compilation options (opt level, debug, verbose).
    pub options: CompileOptions,
    /// Project filesystem paths.
    pub paths: ProjectPaths,
    /// Target platform triple.
    pub target: TargetTriple,
    /// Path to the standard library directory.
    pub stdlib_path: PathBuf,
    /// Resolved package dependency graph.
    pub package_graph: PackageGraph,
    /// Source map for diagnostic rendering.
    ///
    /// Uses `Rc<RefCell<SourceMap>>` for shared ownership and interior
    /// mutability — files are added during session initialization and
    /// by the loader during import resolution.
    pub source_map: Rc<RefCell<SourceMap>>,
    /// Global string interner.
    pub interner: Interner,
    /// Arena allocator for AST/HIR/MIR nodes.
    pub arena: CompilerArena,
    /// Central type registry (single source of truth for types).
    pub type_registry: TypeRegistry,
    /// Query cache for incremental compilation.
    pub query_cache: QueryCache,
    /// Diagnostic emitter for rendering errors.
    pub diagnostics: DiagnosticEmitter,
    /// Collected compiler errors.
    pub errors: Vec<CompilerError>,
    /// Maps file paths to FileIds assigned by the SourceMap.
    file_id_map: HashMap<PathBuf, FileId>,
}

impl CompileSession {
    /// Create a new compilation session.
    ///
    /// Initializes all shared resources, resolves the stdlib path,
    /// and loads standard library source files into the source map.
    pub fn new(options: CompileOptions, root: &Path) -> Result<Self, CompilerError> {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let src = root.join("src");
        let out = root.join("target");

        let stdlib_path = resolve_stdlib_path(&root);

        let target = TargetTriple::host();

        let use_color = std::env::var("NO_COLOR").is_err();
        let session = Self {
            options,
            paths: ProjectPaths { root, src, out },
            target,
            stdlib_path: stdlib_path.clone(),
            package_graph: PackageGraph::default(),
            source_map: Rc::new(RefCell::new(SourceMap::new())),
            interner: Interner::new(),
            arena: CompilerArena::new(),
            type_registry: TypeRegistry::new(),
            query_cache: QueryCache::new(),
            diagnostics: DiagnosticEmitter::new(use_color),
            errors: Vec::new(),
            file_id_map: HashMap::new(),
        };

        let mut session = session;
        session.load_stdlib()?;
        Ok(session)
    }

    /// Load standard library source files into the source map.
    ///
    /// Searches for .doo files in the stdlib directory and registers
    /// them with the SourceMap for diagnostic rendering.
    /// Silently succeeds if no stdlib directory is found — single-file
    /// scripts do not require a stdlib.
    pub fn load_stdlib(&mut self) -> Result<(), CompilerError> {
        if self.stdlib_path.as_os_str().is_empty() || !self.stdlib_path.exists() {
            return Ok(());
        }

        let std_files = collect_doo_files(&self.stdlib_path);

        let mut sm = self.source_map.borrow_mut();
        for file_path in std_files {
            let display_name = file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown.doo")
                .to_string();

            if let Ok(source) = std::fs::read_to_string(&file_path) {
                let file_id = sm.add_file(&display_name, &source);
                self.file_id_map.insert(file_path, file_id);
            }
        }

        Ok(())
    }

    /// Load the package dependency graph from doo.toml.
    ///
    /// Implemented in Phase 52 when doo.toml parsing is available.
    pub fn load_package_graph(&mut self) -> Result<(), CompilerError> {
        Ok(())
    }

    /// Look up the FileId for a source file path.
    pub fn file_id_for_path(&self, path: &Path) -> FileId {
        self.file_id_map.get(path).copied().unwrap_or(FileId::DUMMY)
    }

    /// Emit a compiler error: render it to stderr and store it.
    pub fn emit_diagnostic(&mut self, error: CompilerError) {
        let sm = self.source_map.borrow();
        let _ = self.diagnostics.emit(&error, &sm);
        self.errors.push(error);
    }

    /// Check if any fatal errors were collected.
    pub fn has_errors(&self) -> bool {
        self.errors.iter().any(|e| e.is_fatal())
    }

    /// Add a source file to the session's source map.
    ///
    /// Called by the loader when parsing user source files.
    /// Returns the assigned FileId.
    pub fn add_source_file(&mut self, path: &Path, display_name: &str, source: &str) -> FileId {
        let file_id = self.source_map.borrow_mut().add_file(display_name, source);
        self.file_id_map.insert(path.to_path_buf(), file_id);
        file_id
    }

    /// Take all collected errors, leaving the session's error list empty.
    pub fn take_errors(&mut self) -> Vec<CompilerError> {
        std::mem::take(&mut self.errors)
    }

    /// Get a reference to the source map for read-only access.
    ///
    /// Returns a `Ref<SourceMap>` that must be dropped before any
    /// mutable access to the source map.
    pub fn source_map(&self) -> std::cell::Ref<'_, SourceMap> {
        self.source_map.borrow()
    }
}

impl std::fmt::Debug for CompileSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompileSession")
            .field("options", &self.options)
            .field("paths", &self.paths)
            .field("target", &self.target)
            .field("stdlib_path", &self.stdlib_path)
            .field("error_count", &self.errors.len())
            .field("file_count", &self.file_id_map.len())
            .finish()
    }
}

// ============================================================================
// Stdlib Path Resolution
// ============================================================================

/// Resolve the standard library directory path.
///
/// Search order:
/// 1. DOO_STDLIB_PATH environment variable
/// 2. Next to the compiler executable (production install)
/// 3. Walk up from the project root (development)
/// 4. Walk up from the current working directory
/// 5. Relative ./std (CI/testing fallback)
///
/// Returns an empty path if no stdlib is found — single-file
/// scripts do not require a stdlib.
fn resolve_stdlib_path(project_root: &Path) -> PathBuf {
    if let Ok(path) = std::env::var(env_vars::DOO_STDLIB_PATH) {
        let p = PathBuf::from(path);
        if p.exists() && is_valid_stdlib(&p) {
            return p;
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let std_dir = dir.join("std");
            if std_dir.exists() && is_valid_stdlib(&std_dir) {
                return std_dir;
            }

            let mut current = dir.to_path_buf();
            for _ in 0..10 {
                if !current.pop() {
                    break;
                }
                let std_dir = current.join("std");
                if std_dir.exists() && is_valid_stdlib(&std_dir) {
                    return std_dir;
                }
            }
        }
    }

    let mut current = project_root.to_path_buf();
    for _ in 0..20 {
        let std_dir = current.join("std");
        if std_dir.exists() && is_valid_stdlib(&std_dir) {
            return std_dir;
        }
        if !current.pop() {
            break;
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        let mut current = cwd;
        for _ in 0..20 {
            let std_dir = current.join("std");
            if std_dir.exists() && is_valid_stdlib(&std_dir) {
                return std_dir;
            }
            if !current.pop() {
                break;
            }
        }
    }

    let dev_stdlib = PathBuf::from("./std");
    if dev_stdlib.exists() && is_valid_stdlib(&dev_stdlib) {
        return dev_stdlib;
    }

    PathBuf::new()
}

/// Check if a directory contains .doo source files (valid stdlib).
fn is_valid_stdlib(path: &Path) -> bool {
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if entry
                .path()
                .extension()
                .and_then(|e| e.to_str())
                .map_or(false, |s| s == "doo")
            {
                return true;
            }
        }
    }
    false
}

/// Recursively collect all .doo files in a directory.
fn collect_doo_files(dir: &Path) -> Vec<PathBuf> {
    let mut results = Vec::new();
    collect_doo_files_recursive(dir, &mut results);
    results.sort();
    results
}

fn collect_doo_files_recursive(dir: &Path, results: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !name.starts_with('.')
                    && name != "target"
                    && name != "target-windows"
                    && name != "target-linux"
                    && name != "node_modules"
                {
                    collect_doo_files_recursive(&path, results);
                }
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .map_or(false, |s| s == "doo")
            {
                results.push(path);
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opt_level_default() {
        assert_eq!(OptLevel::default(), OptLevel::Default);
    }

    #[test]
    fn test_opt_level_from_str() {
        assert_eq!(OptLevel::from_str("0"), Some(OptLevel::None));
        assert_eq!(OptLevel::from_str("3"), Some(OptLevel::Aggressive));
        assert_eq!(OptLevel::from_str("s"), Some(OptLevel::Size));
        assert_eq!(OptLevel::from_str("invalid"), None);
    }

    #[test]
    fn test_compile_options_default() {
        let opts = CompileOptions::default();
        assert_eq!(opts.opt_level, OptLevel::Default);
        assert!(!opts.debug);
        assert!(!opts.verbose);
    }

    #[test]
    fn test_target_triple_host() {
        let triple = TargetTriple::host();
        assert!(!triple.arch.is_empty());
        assert!(!triple.os.is_empty());
    }

    #[test]
    fn test_target_triple_to_string() {
        let triple = TargetTriple {
            arch: "x86_64".to_string(),
            vendor: "unknown".to_string(),
            os: "linux".to_string(),
            env: "gnu".to_string(),
        };
        assert_eq!(triple.to_triple_string(), "x86_64-unknown-linux-gnu");
    }

    #[test]
    fn test_target_triple_to_string_no_env() {
        let triple = TargetTriple {
            arch: "aarch64".to_string(),
            vendor: "apple".to_string(),
            os: "macos".to_string(),
            env: String::new(),
        };
        assert_eq!(triple.to_triple_string(), "aarch64-apple-macos");
    }

    #[test]
    fn test_package_graph_default() {
        let graph = PackageGraph::default();
        assert!(graph.packages.is_empty());
    }

    #[test]
    fn test_project_paths() {
        let paths = ProjectPaths {
            root: PathBuf::from("/project"),
            src: PathBuf::from("/project/src"),
            out: PathBuf::from("/project/target"),
        };
        assert_eq!(paths.root, PathBuf::from("/project"));
    }
}
