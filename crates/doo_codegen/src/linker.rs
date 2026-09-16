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
        doo_core::errors::codes::CompilerError::new(code, msg, doo_core::Span::dummy())
    }
}

// ============================================================================
// Module Linker
// ============================================================================

/// Module linker for multi-file compilation.
///
/// Links multiple LLVM modules together, resolving cross-module references.
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

    /// Add a module to be linked. Builds the symbol table from function definitions.
    pub fn add_module(&mut self, module: Module<'ctx>) -> Result<(), LinkError> {
        let module_index = self.modules.len();

        let mut func_iter = module.get_first_function();
        while let Some(func) = func_iter {
            let name = func.get_name().to_str().unwrap_or("").to_string();

            if func.count_basic_blocks() > 0 {
                if self.symbol_table.contains_key(&name) {
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

/// Resolves cross-module function references by declaring
/// external functions and tracking dependencies.
pub struct CrossModuleResolver {
    /// External function declarations needed: (calling_module, function_name).
    pending_declarations: Vec<(String, String)>,
    /// Resolved references: function_name -> source_module.
    resolved: FxHashMap<String, String>,
}

impl CrossModuleResolver {
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

    pub fn get_pending(&self, module: &str) -> Vec<String> {
        self.pending_declarations
            .iter()
            .filter(|(m, _)| m == module)
            .map(|(_, f)| f.clone())
            .collect()
    }

    pub fn get_source(&self, function_name: &str) -> Option<&String> {
        self.resolved.get(function_name)
    }

    pub fn is_resolved(&self, function_name: &str) -> bool {
        self.resolved.contains_key(function_name)
    }
}

impl Default for CrossModuleResolver {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Binary Linker
// ============================================================================

/// Binary linker — invokes the system linker to produce an executable.
///
/// Links the compiled LLVM module with:
/// - Tier A runtime libraries (libdoo_core, libdoo_runtime, libdoo_json)
/// - Platform libc (automatic via cc/clang)
/// - Platform system libraries (ws2_32 on Windows, pthread/dl on Unix)
/// - Any @extern-specified libraries
///
/// Does NOT automatically link framework libraries (doo_ffi_http, doo_ffi_db).
/// Those are only linked when the package explicitly declares them as dependencies.
pub struct BinaryLinker {
    /// Output executable name.
    output_name: String,
    /// Additional libraries specified via @extern declarations.
    extern_libs: Vec<String>,
    /// Path to the toolchain lib directory containing pre-compiled .a files.
    toolchain_lib: std::path::PathBuf,
}

impl BinaryLinker {
    /// Create a new binary linker.
    ///
    /// - `output_name`: name of the output executable (without extension)
    /// - `extern_libs`: library names from @extern declarations (without "lib" prefix)
    pub fn new(output_name: &str, extern_libs: Vec<String>) -> Self {
        let toolchain_lib = resolve_toolchain_lib_path();
        Self {
            output_name: output_name.to_string(),
            extern_libs,
            toolchain_lib,
        }
    }

    /// Link an LLVM module into an executable binary.
    ///
    /// Steps:
    /// 1. Write the module to a temporary object file
    /// 2. Invoke the system linker (cc/gcc/clang)
    /// 3. Link Tier A runtime + libc + system libraries + @extern libraries
    /// 4. Clean up temporary files
    pub fn link<'ctx>(
        &self,
        module: &Module<'ctx>,
    ) -> Result<std::path::PathBuf, doo_core::errors::codes::CompilerError> {
        use inkwell::targets::{
            CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
        };
        use inkwell::OptimizationLevel;

        Target::initialize_native(&InitializationConfig::default()).map_err(|e| {
            doo_core::errors::codes::CompilerError::new(
                doo_core::errors::codes::ErrorCode::LlvmError,
                format!("failed to initialize native target: {}", e),
                doo_core::Span::dummy(),
            )
        })?;

        let triple = TargetMachine::get_default_triple();
        let target = Target::from_triple(&triple).map_err(|e| {
            doo_core::errors::codes::CompilerError::new(
                doo_core::errors::codes::ErrorCode::LlvmError,
                format!("failed to get target: {}", e),
                doo_core::Span::dummy(),
            )
        })?;

        let cpu = TargetMachine::get_host_cpu_name();
        let features = TargetMachine::get_host_cpu_features();

        let target_machine = target
            .create_target_machine(
                &triple,
                cpu.to_str().unwrap_or("generic"),
                features.to_str().unwrap_or(""),
                OptimizationLevel::Default,
                RelocMode::PIC,
                CodeModel::Default,
            )
            .ok_or_else(|| {
                doo_core::errors::codes::CompilerError::new(
                    doo_core::errors::codes::ErrorCode::LlvmError,
                    "failed to create target machine",
                    doo_core::Span::dummy(),
                )
            })?;

        {
            let td = target_machine.get_target_data();
            let dl = td.get_data_layout();
            module.set_data_layout(&dl);
            std::mem::forget(td);
            std::mem::forget(dl);
        }

        let temp_dir = std::env::temp_dir();
        let object_path = temp_dir.join(format!("doo_{}.o", std::process::id()));

        target_machine
            .write_to_file(module, FileType::Object, &object_path)
            .map_err(|e| {
                doo_core::errors::codes::CompilerError::new(
                    doo_core::errors::codes::ErrorCode::LlvmError,
                    format!("failed to write object file: {}", e),
                    doo_core::Span::dummy(),
                )
            })?;

        // Leak LLVM-allocated strings to avoid disposal crashes on LLVM 22.
        std::mem::forget(triple);
        std::mem::forget(cpu);
        std::mem::forget(features);
        std::mem::forget(target_machine);

        let output_path = self.invoke_system_linker(&object_path)?;

        let _ = std::fs::remove_file(&object_path);

        Ok(output_path)
    }

    /// Invoke the system linker (cc/gcc/clang) to produce the executable.
    ///
    /// Links the object file with Tier A runtime libraries, libc,
    /// platform system libraries, and @extern-specified libraries.
    fn invoke_system_linker(
        &self,
        object_path: &std::path::Path,
    ) -> Result<std::path::PathBuf, doo_core::errors::codes::CompilerError> {
        let linker = find_system_linker();
        let output_path = std::path::PathBuf::from(&self.output_name);

        let mut cmd = std::process::Command::new(&linker);
        cmd.arg("-o").arg(&output_path);
        cmd.arg(object_path);

        if self.toolchain_lib.exists() {
            cmd.arg("-L").arg(&self.toolchain_lib);
            cmd.arg("-ldoo_core");
            cmd.arg("-ldoo_runtime");
            cmd.arg("-ldoo_json");
        }

        cmd.arg("-lm");

        for lib in &self.extern_libs {
            // The C standard library ("c") is always linked by the system toolchain.
            // On Windows it's msvcrt.lib, on Linux it's libc. We don't need to 
            // explicitly pass -lc, and doing so can cause issues.
            if lib == "c" {
                continue;
            }
            cmd.arg(format!("-l{}", lib));
        }

        // Windows system libraries
        if cfg!(target_os = "windows") {
            cmd.arg("ws2_32.lib");
            cmd.arg("userenv.lib");
            cmd.arg("bcrypt.lib");
            cmd.arg("ntdll.lib");
            cmd.arg("advapi32.lib");
            cmd.arg("kernel32.lib");
            cmd.arg("secur32.lib");
            cmd.arg("crypt32.lib");
            cmd.arg("ole32.lib");
            cmd.arg("oleaut32.lib");
            cmd.arg("rpcrt4.lib");
            cmd.arg("gdi32.lib");
            cmd.arg("msvcrt.lib");
        } else {
            cmd.arg("-lpthread");
            cmd.arg("-ldl");
        }

        // DEBUG: Print the full linker command
        eprintln!("[LINKER] binary: {}", linker);
        eprintln!(
            "[LINKER] toolchain_lib exists: {}",
            self.toolchain_lib.exists()
        );
        eprintln!(
            "[LINKER] toolchain_lib path: {}",
            self.toolchain_lib.display()
        );
        eprintln!("[LINKER] cfg!(windows): {}", cfg!(target_os = "windows"));
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_str().unwrap_or("<non-utf8>").to_string())
            .collect();
        eprintln!("[LINKER] full args: {}", args.join(" "));

        let output = cmd.output().map_err(|e| {
            doo_core::errors::codes::CompilerError::new(
                doo_core::errors::codes::ErrorCode::LlvmError,
                format!("failed to invoke linker '{}': {}", linker, e),
                doo_core::Span::dummy(),
            )
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(doo_core::errors::codes::CompilerError::new(
                doo_core::errors::codes::ErrorCode::LlvmError,
                format!("linker failed: {}", stderr),
                doo_core::Span::dummy(),
            ));
        }

        Ok(output_path)
    }
}

/// Find a usable system linker (cc, gcc, or clang).
fn find_system_linker() -> String {
    if cfg!(target_os = "windows") {
        for candidate in &["clang-cl", "cl", "clang", "gcc", "cc"] {
            if std::process::Command::new(candidate)
                .arg("--version")
                .output()
                .is_ok()
            {
                return candidate.to_string();
            }
        }
        "link".to_string()
    } else {
        for candidate in &["cc", "gcc", "clang"] {
            if std::process::Command::new(candidate)
                .arg("--version")
                .output()
                .is_ok()
            {
                return candidate.to_string();
            }
        }
        "cc".to_string()
    }
}

/// Resolve the toolchain library path from the DOO_STDLIB_PATH env var
/// or fall back to a relative path from the executable.
fn resolve_toolchain_lib_path() -> std::path::PathBuf {
    use doo_core::constants::env_vars;

    // 1. Explicit env var — highest priority
    if let Ok(path) = std::env::var(env_vars::DOO_STDLIB_PATH) {
        return std::path::PathBuf::from(path).join("lib");
    }

    // 2. Next to the compiler executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let candidate = parent.join("lib");
            if candidate.exists() {
                return candidate;
            }
            let candidate2 = parent.join("..").join("lib");
            if candidate2.exists() {
                return candidate2;
            }
        }
    }

    // 3. User home directory
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let candidate = std::path::PathBuf::from(home)
            .join(".doolang")
            .join("toolchains")
            .join("stable")
            .join("lib");
        if candidate.exists() {
            return candidate;
        }
    }

    // 4. System install paths (platform-specific last resort)
    if cfg!(target_os = "windows") {
        std::path::PathBuf::from("C:\\Program Files\\doolang\\lib")
    } else if cfg!(target_os = "macos") {
        std::path::PathBuf::from("/usr/local/lib")
    } else {
        std::path::PathBuf::from("/usr/lib/doolang")
    }
}
