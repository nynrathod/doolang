//! Compilation Pipeline
//!
//! Orchestrates the full compilation process:
//! Source → Tokens → AST → HIR → MIR → LLVM IR → Executable
//!
//! This module is the **single source of truth** for the Doo compilation pipeline.
//! All compilation commands (build, run, check) flow through this module.

use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use doo_codegen::{optimize_module, CodegenBuilder, OptLevel};
use doo_core::types::TypeRegistry;
use doo_frontend::{Lexer, Parser};
use doo_hir::Lower;
use doo_mir::builder::MirBuilder;

// Module loader - single source of truth for import resolution
use crate::loader::{merge_imports, resolve_imports, ModuleLoader};

// Analysis imports - wire in the semantic analysis phase
use doo_analysis::{
    // Field visibility checking
    check_field_visibility,
    // AST transformations
    transform::{transform_inline_closures, transform_route_groups},
    // Borrow checking
    BorrowChecker,
    BorrowError,
    DropInserter,
    // Error flow analysis
    ErrorFlowChecker,
    ErrorFlowError,
    ErrorFlowErrorKind,
    // Exhaustiveness checking
    ExhaustivenessChecker,
    ExhaustivenessError,
    ExhaustivenessErrorKind,
    // Ownership analysis
    OwnershipAnalyzer,
    OwnershipError,
    // Type checking
    TypeChecker,
    TypeError,
    TypeErrorKind,
};

use inkwell::context::Context;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::OptimizationLevel;

// ============================================================================
// Compilation Options
// ============================================================================

/// Options controlling compilation behavior.
#[derive(Clone, Debug)]
pub struct CompileOptions {
    /// Path to input file or directory containing main.doo
    pub input_path: PathBuf,
    /// Base name for output (without extension)
    pub output_name: String,
    /// Enable development mode (extra debug info)
    pub dev_mode: bool,
    /// Print AST after parsing
    pub print_ast: bool,
    /// Print MIR before codegen
    pub print_mir: bool,
    /// Keep generated LLVM IR (.ll) file
    pub keep_ll: bool,
    /// Keep object file (.o) after linking
    pub keep_obj: bool,
    /// Only check for errors, don't generate code
    pub check_only: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            input_path: PathBuf::from("."),
            output_name: "output".to_string(),
            dev_mode: cfg!(debug_assertions),
            print_ast: false,
            print_mir: false,
            keep_ll: false,
            keep_obj: false,
            check_only: false,
        }
    }
}

// ============================================================================
// Compilation Result
// ============================================================================

/// Result of a compilation.
pub struct CompileResult {
    /// Whether compilation succeeded
    pub success: bool,
    /// Number of errors encountered
    pub error_count: usize,
    /// Path to generated executable (if successful)
    pub exe_path: Option<PathBuf>,
}

// ============================================================================
// Main Entry Point
// ============================================================================

/// Compile a Doo project.
///
/// This is the main entry point for all compilation. It:
/// 1. Locates the main.doo file
/// 2. Parses all source files
/// 3. Runs semantic analysis
/// 4. Generates MIR
/// 5. Generates LLVM IR
/// 6. Compiles to object code
/// 7. Links into executable
pub fn compile_project(opts: CompileOptions) -> Result<CompileResult, String> {
    // Allow environment overrides
    let output_name = env::var("DOO_OUTPUT_NAME").unwrap_or(opts.output_name.clone());
    let check_only = env::var("DOO_CHECK_ONLY").is_ok() || opts.check_only;

    let opts = CompileOptions {
        output_name,
        check_only,
        ..opts
    };

    // Phase 0: Locate main.doo
    let input_path = resolve_input_path(&opts.input_path)?;

    // Phase 1: Read source
    let source = fs::read_to_string(&input_path)
        .map_err(|e| format!("Failed to read {}: {}", input_path.display(), e))?;

    let project_root = input_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Phase 2: Parse (Parser creates lexer internally)
    // Debug: Show source info
    if env::var("DOO_DEBUG").is_ok() {
        eprintln!("[DEBUG] Source length: {} chars", source.len());
        eprintln!(
            "[DEBUG] First 100 chars: {:?}",
            &source[..source.len().min(100)]
        );

        // Debug lexer output
        let mut debug_lexer = Lexer::new(&source, 0);
        let debug_tokens = debug_lexer.tokenize();
        eprintln!("[DEBUG] Lexer produced {} tokens", debug_tokens.len());
        for (i, tok) in debug_tokens.iter().take(10).enumerate() {
            eprintln!("[DEBUG]   Token {}: {:?} {:?}", i, tok.kind, tok.text);
        }
    }

    let mut parser = Parser::new(&source, 0);
    let program = match parser.parse_program() {
        Ok(p) => {
            // Debug: Check for parser errors even on success
            if env::var("DOO_DEBUG").is_ok() {
                let errors = parser.errors();
                eprintln!("[DEBUG] Parser errors: {}", errors.len());
                for (i, e) in errors.iter().take(5).enumerate() {
                    eprintln!("[DEBUG]   Error {}: {}", i, e);
                }
            }
            p
        }
        Err(e) => {
            eprintln!("Parse error: {}", e);
            return Ok(CompileResult {
                success: false,
                error_count: 1,
                exe_path: None,
            });
        }
    };

    // Phase 3: AST Transformations
    // Transform route DSL (groups, decorators) into explicit route registrations
    let mut program = program;
    transform_route_groups(&mut program);
    transform_inline_closures(&mut program);

    // Phase 3.5: Resolve Imports (using centralized loader module)
    // Load and merge imported functions/structs/enums from std library and other modules
    let mut loader = ModuleLoader::new();
    let import_resolution = resolve_imports(&program, &mut loader, &project_root)?;

    // Debug: show what was resolved
    if env::var("DOO_DEBUG").is_ok() {
        eprintln!(
            "[DEBUG] Import resolution items: {}",
            import_resolution.items.len()
        );
        for item in &import_resolution.items {
            match item {
                doo_frontend::ast::Item::Struct(s) => {
                    eprintln!("[DEBUG]   Imported Struct: {}", s.name)
                }
                doo_frontend::ast::Item::Function(f) => {
                    eprintln!("[DEBUG]   Imported Function: {}", f.name)
                }
                doo_frontend::ast::Item::Enum(e) => {
                    eprintln!("[DEBUG]   Imported Enum: {}", e.name)
                }
                _ => eprintln!("[DEBUG]   Imported other item"),
            }
        }
    }

    // Track which structs were imported (for visibility checking)
    let imported_struct_names: HashSet<String> = import_resolution
        .items
        .iter()
        .filter_map(|item| match item {
            doo_frontend::ast::Item::Struct(s) => Some(s.name.clone()),
            _ => None,
        })
        .collect();

    merge_imports(&mut program, import_resolution);

    if opts.print_ast {
        eprintln!("=== AST ===");
        eprintln!("{:#?}", program);
    }

    // Phase 4: Lower to HIR (with type information)
    let mut type_registry = TypeRegistry::new();
    let mut lowerer = Lower::new();
    let hir = lowerer.lower_program_typed(&program, &mut type_registry);

    // Wrap in Arc for shared access
    let type_registry = Arc::new(type_registry);

    // Debug: Show what functions were found in AST and HIR
    if env::var("DOO_DEBUG").is_ok() {
        eprintln!("[DEBUG] AST items: {}", program.items.len());
        for item in &program.items {
            match item {
                doo_frontend::ast::Item::Function(f) => eprintln!("[DEBUG]   Function: {}", f.name),
                doo_frontend::ast::Item::Struct(s) => eprintln!("[DEBUG]   Struct: {}", s.name),
                doo_frontend::ast::Item::Enum(e) => eprintln!("[DEBUG]   Enum: {}", e.name),
                doo_frontend::ast::Item::Import(i) => eprintln!("[DEBUG]   Import: {:?}", i.path),
                doo_frontend::ast::Item::Statement(_) => eprintln!("[DEBUG]   Statement"),
            }
        }
        eprintln!("[DEBUG] HIR items: {}", hir.items.len());
        for item in &hir.items {
            match item {
                doo_hir::HirItem::Function(f) => eprintln!("[DEBUG]   HIR Function: {}", f.name),
                doo_hir::HirItem::Struct(s) => eprintln!("[DEBUG]   HIR Struct: {}", s.name),
                doo_hir::HirItem::Enum(e) => eprintln!("[DEBUG]   HIR Enum: {}", e.name),
                doo_hir::HirItem::Import(_) => eprintln!("[DEBUG]   HIR Import"),
            }
        }
    }

    // Phase 5: Semantic Analysis (type checking, name resolution, etc.)
    // TODO: Integrate semantic passes when doo_analysis is fully ready
    // For now, we proceed directly to MIR

    // ========================================================================
    // Phase 5: Semantic Analysis
    // ========================================================================
    // Run all analysis passes in sequence. The compiler handles ownership,
    // borrowing, types, error flow, and exhaustiveness automatically.
    // Users don't write `&` or `*` - the compiler does it all.

    let mut hir = hir; // Make HIR mutable for drop insertion
    let mut analysis_errors: Vec<String> = Vec::new();

    // 5.1: Type Checking
    // Validates type compatibility across the program
    let mut type_checker = TypeChecker::new(type_registry.clone());
    if let Err(errors) = type_checker.check(&hir) {
        for err in &errors {
            analysis_errors.push(format_type_error(err));
        }
    }

    // 5.2: Ownership Analysis
    // Tracks variable ownership and decides Move/Copy/Clone automatically
    let mut ownership_analyzer = OwnershipAnalyzer::new();
    let ownership_results = match ownership_analyzer.analyze(&hir) {
        Ok(results) => Some(results),
        Err(errors) => {
            for err in &errors {
                analysis_errors.push(format_ownership_error(err));
            }
            None
        }
    };

    // 5.3: Borrow Checking
    // Ensures safe memory access - the ONLY error users can see is concurrent mutable borrow
    let mut borrow_checker = BorrowChecker::new();
    if let Err(errors) = borrow_checker.check(&hir) {
        for err in &errors {
            analysis_errors.push(format_borrow_error(err));
        }
    }

    // 5.4: Drop Insertion
    // Automatically inserts Drop statements at optimal points (after last use)
    // Uses ownership results to skip dropping moved variables
    let mut drop_inserter = if let Some(ref results) = ownership_results {
        DropInserter::with_ownership_results(results)
    } else {
        DropInserter::new()
    };
    drop_inserter.insert_drops_program(&mut hir);

    // 5.5: Error Flow Checking
    // Ensures all Result types are properly handled
    let mut error_flow_checker = ErrorFlowChecker::new(&type_registry);
    if let Err(errors) = error_flow_checker.check(&hir) {
        for err in &errors {
            analysis_errors.push(format_error_flow_error(err));
        }
    }

    // 5.6: Exhaustiveness Checking
    // Ensures all match expressions cover all possible patterns
    let mut exhaustiveness_checker = ExhaustivenessChecker::new(&type_registry);
    let exhaustiveness_errors = exhaustiveness_checker.check_program(&hir);
    for err in &exhaustiveness_errors {
        analysis_errors.push(format_exhaustiveness_error(err));
    }

    // 5.7: Field Visibility Checking
    // Ensures private fields (camelCase) are not accessed from outside their module
    if env::var("DOO_DEBUG").is_ok() {
        eprintln!("[DEBUG] Imported struct names: {:?}", imported_struct_names);
    }
    let visibility_errors = check_field_visibility(&hir, &type_registry, &imported_struct_names);
    for err in &visibility_errors {
        analysis_errors.push(err.clone());
    }

    // Report any analysis errors
    if !analysis_errors.is_empty() {
        for err in &analysis_errors {
            eprintln!("{}", err);
        }
        return Ok(CompileResult {
            success: false,
            error_count: analysis_errors.len(),
            exe_path: None,
        });
    }

    if env::var("DOO_DEBUG").is_ok() {
        eprintln!("[DEBUG] Semantic analysis passed");
    }

    // Phase 6: Build MIR
    // Pass ownership analysis results to MIR builder so it can emit
    // Move/Copy/Clone/Borrow instructions based on ownership decisions
    let mut mir_builder = if let Some(results) = ownership_results {
        MirBuilder::with_ownership(&type_registry, results)
    } else {
        MirBuilder::new(&type_registry)
    };
    let mir_program = mir_builder.build(&hir);

    if opts.print_mir {
        eprintln!("=== MIR ===");
        eprintln!("{:#?}", mir_program);
    }

    // Debug: Show MIR functions
    if env::var("DOO_DEBUG").is_ok() {
        eprintln!("[DEBUG] MIR functions: {}", mir_program.functions.len());
        for f in &mir_program.functions {
            eprintln!("[DEBUG]   MIR Function: {}", f.name);
        }
    }

    // Validate MIR
    if env::var("DOO_DEBUG").is_ok() {
        eprintln!("[DEBUG] Validating MIR...");
    }
    if let Err(e) = mir_program.validate() {
        return Err(format!("MIR validation failed: {}", e));
    }
    if env::var("DOO_DEBUG").is_ok() {
        eprintln!("[DEBUG] MIR validation passed");
    }

    // Check for main function
    let has_main = mir_program.functions.iter().any(|f| f.name == "main");
    if !has_main {
        return Err(
            "Error: main() function not found. Every program must have a main() function."
                .to_string(),
        );
    }

    // If check-only, we're done
    if opts.check_only {
        return Ok(CompileResult {
            success: true,
            error_count: 0,
            exe_path: None,
        });
    }

    // Phase 7: LLVM Codegen
    if env::var("DOO_DEBUG").is_ok() {
        eprintln!("[DEBUG] Starting LLVM codegen...");
    }
    let context = Context::create();
    let codegen = CodegenBuilder::new(&context);
    let module = codegen.build(&mir_program, "main_module", type_registry.clone());
    if env::var("DOO_DEBUG").is_ok() {
        eprintln!("[DEBUG] LLVM codegen complete");
    }

    // Phase 8: Verify module
    if env::var("DOO_DEBUG").is_ok() {
        eprintln!("[DEBUG] Verifying LLVM module...");
    }
    if let Err(e) = module.verify() {
        return Err(format!("LLVM module verification failed: {}", e));
    }
    if env::var("DOO_DEBUG").is_ok() {
        eprintln!("[DEBUG] LLVM module verified");
    }

    // Phase 9: Optimize
    optimize_module(&module, OptLevel::O2);

    // Phase 10: Write LLVM IR if requested
    if opts.keep_ll {
        let ll_file = format!("{}.ll", opts.output_name);
        let ir_string = module.print_to_string();
        fs::write(&ll_file, ir_string.to_string())
            .map_err(|e| format!("Failed to write LLVM IR: {}", e))?;
    }

    // Phase 12: Compile to object file
    let obj_file = compile_to_object(&module, &opts)?;

    // Phase 13: Link
    let exe_path = link_object_file(&obj_file, &opts, &mir_program)?;

    // Cleanup object file unless requested to keep
    if !opts.keep_obj {
        let _ = fs::remove_file(&obj_file);
    }

    Ok(CompileResult {
        success: true,
        error_count: 0,
        exe_path: Some(exe_path),
    })
}

// ============================================================================
// Input Resolution
// ============================================================================

/// Resolve the input path to an actual main.doo file.
fn resolve_input_path(input: &Path) -> Result<PathBuf, String> {
    if input.is_file() {
        return Ok(input.to_path_buf());
    }

    // Try main.doo in directory
    let main_file = input.join("main.doo");
    if main_file.exists() {
        return Ok(main_file);
    }

    // Try src/main.doo
    let src_main = input.join("src").join("main.doo");
    if src_main.exists() {
        return Ok(src_main);
    }

    // Search for candidates
    let candidates = discover_main_doo_candidates(input, 4, 25);

    if candidates.len() == 1 {
        return Ok(candidates[0].clone());
    }

    if candidates.is_empty() {
        return Err(format!(
            "Error: main.doo not found in {} or {}/src",
            input.display(),
            input.display()
        ));
    }

    // Check DOO_ENTRY environment variable
    if let Ok(entry) = env::var("DOO_ENTRY") {
        let entry_path = PathBuf::from(&entry);
        let entry_path = if entry_path.is_absolute() {
            entry_path
        } else {
            input.join(&entry_path)
        };

        if entry_path.is_file() {
            return Ok(entry_path);
        }

        if entry_path.is_dir() {
            let entry_main = entry_path.join("main.doo");
            if entry_main.exists() {
                return Ok(entry_main);
            }
        }
    }

    // Multiple candidates found
    let mut msg = format!(
        "Error: main.doo not found in {} or {}/src\n\nFound multiple candidates:\n",
        input.display(),
        input.display()
    );
    for c in &candidates {
        msg.push_str(&format!("  - {}\n", c.display()));
    }
    msg.push_str("\nRun with an explicit path, e.g. `doo run <project_dir_or_main.doo>`\n");
    msg.push_str("Or set DOO_ENTRY to a project dir or main.doo path to disambiguate.\n");
    Err(msg)
}

/// Discover main.doo candidates in a directory tree.
pub fn discover_main_doo_candidates(
    start: &Path,
    max_depth: usize,
    max_results: usize,
) -> Vec<PathBuf> {
    let mut results = Vec::new();
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    queue.push_back((start.to_path_buf(), 0));

    while let Some((dir, depth)) = queue.pop_front() {
        if depth > max_depth || results.len() >= max_results {
            continue;
        }

        // Check main.doo
        let main_file = dir.join("main.doo");
        if main_file.exists() {
            results.push(main_file);
            if results.len() >= max_results {
                break;
            }
        }

        // Check src/main.doo
        let src_main = dir.join("src").join("main.doo");
        if src_main.exists() {
            results.push(src_main);
            if results.len() >= max_results {
                break;
            }
        }

        // Explore subdirectories
        if depth < max_depth {
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }

                    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                    // Skip hidden, target, and node_modules
                    if name.starts_with('.')
                        || name == "target"
                        || name == "target-windows"
                        || name == "target-linux"
                        || name == "node_modules"
                    {
                        continue;
                    }

                    queue.push_back((path, depth + 1));
                }
            }
        }
    }

    results.sort();
    results.dedup();
    results
}

// ============================================================================
// Analysis Error Formatting
// ============================================================================

/// Format a type checking error for display.
fn format_type_error(err: &TypeError) -> String {
    let msg = match &err.kind {
        TypeErrorKind::Mismatch { expected, found } => {
            format!("type mismatch: expected {:?}, found {:?}", expected, found)
        }
        TypeErrorKind::Undefined(name) => {
            format!("undefined variable: '{}'", name)
        }
        TypeErrorKind::InvalidOp(op) => {
            format!("invalid operation: {}", op)
        }
        TypeErrorKind::ArgMismatch { expected, found } => {
            format!(
                "argument count mismatch: expected {}, found {}",
                expected, found
            )
        }
        TypeErrorKind::InvalidCondition { found } => {
            format!("invalid condition type: expected Bool, found {:?}", found)
        }
        TypeErrorKind::InvalidCast { from, to } => {
            format!("invalid cast: cannot cast {:?} to {:?}", from, to)
        }
        TypeErrorKind::ReturnTypeMismatch {
            function,
            expected,
            found,
        } => {
            format!(
                "return type mismatch in '{}': expected {:?}, found {:?}",
                function, expected, found
            )
        }
    };
    format!("❌ Type Error at {:?}: {}", err.span, msg)
}

/// Format an ownership error for display.
fn format_ownership_error(err: &OwnershipError) -> String {
    format!("❌ Ownership Error at {:?}: {}", err.span, err.message)
}

/// Format a borrow checking error for display.
fn format_borrow_error(err: &BorrowError) -> String {
    format!("❌ Borrow Error at {:?}: {}", err.span, err.message())
}

/// Format an error flow error for display.
fn format_error_flow_error(err: &ErrorFlowError) -> String {
    let msg = match &err.kind {
        ErrorFlowErrorKind::UnhandledResult { ok_type, err_type } => {
            format!(
                "unhandled Result: {:?} ! {:?} - use `?` or `let ok, err = ...`",
                ok_type, err_type
            )
        }
        ErrorFlowErrorKind::TryInNonResultFunction { func_name } => {
            format!(
                "`?` operator used in function '{}' which doesn't return a Result",
                func_name
            )
        }
        ErrorFlowErrorKind::ErrInNonResultFunction { func_name } => {
            format!(
                "`Err` used in function '{}' which doesn't have an error type",
                func_name
            )
        }
        ErrorFlowErrorKind::MissingOkPath { func_name } => {
            format!(
                "function '{}' returns Result but not all paths return Ok",
                func_name
            )
        }
        ErrorFlowErrorKind::PanicWithoutMessage => {
            "panic (`??`) used without a message".to_string()
        }
    };
    format!("❌ Error Flow Error at {:?}: {}", err.span, msg)
}

/// Format an exhaustiveness error for display.
fn format_exhaustiveness_error(err: &ExhaustivenessError) -> String {
    let msg = match &err.kind {
        ExhaustivenessErrorKind::NonExhaustive { missing } => {
            format!(
                "non-exhaustive match: missing patterns {}",
                missing.join(", ")
            )
        }
        ExhaustivenessErrorKind::UnreachablePattern => {
            "unreachable pattern: this pattern will never be matched".to_string()
        }
    };
    format!("❌ Match Error at {:?}: {}", err.span, msg)
}

// ============================================================================
// Object File Generation
// ============================================================================

/// Compile LLVM module to object file.
fn compile_to_object(
    module: &inkwell::module::Module,
    opts: &CompileOptions,
) -> Result<PathBuf, String> {
    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("Failed to initialize target: {}", e))?;

    let triple = TargetMachine::get_default_triple();
    let cpu = TargetMachine::get_host_cpu_name().to_string();
    let features = TargetMachine::get_host_cpu_features().to_string();

    let target =
        Target::from_triple(&triple).map_err(|e| format!("Failed to create target: {}", e))?;

    let target_machine = target
        .create_target_machine(
            &triple,
            &cpu,
            &features,
            OptimizationLevel::Aggressive,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or("Failed to create target machine")?;

    let obj_path = PathBuf::from(format!("{}.o", opts.output_name));

    target_machine
        .write_to_file(module, FileType::Object, &obj_path)
        .map_err(|e| format!("Failed to write object file: {}", e))?;

    Ok(obj_path)
}

// ============================================================================
// Linking
// ============================================================================

/// Link object file into executable.
fn link_object_file(
    obj_file: &Path,
    opts: &CompileOptions,
    mir_program: &doo_mir::MirProgram,
) -> Result<PathBuf, String> {
    // Collect FFI libraries from MIR (ffi field contains FfiLinkage with library name)
    let mut ffi_libs: HashSet<String> = mir_program
        .functions
        .iter()
        .filter_map(|f| f.ffi.as_ref().map(|l| l.library.clone()))
        .collect();

    // Always include core runtime library (new naming: doo_ffi_core)
    ffi_libs.insert("doo_ffi_core".to_string());

    // Detect HTTP usage
    let has_http = mir_program.functions.iter().any(|f| {
        f.ffi
            .as_ref()
            .map(|l| l.library == "doo_ffi_http" || l.library == "doo_http")
            .unwrap_or(false)
            || f.name.starts_with("Server::")
            || f.name.contains("::get")
            || f.name.contains("::post")
            || f.name.contains("::put")
            || f.name.contains("::delete")
    });

    if has_http {
        ffi_libs.insert("doo_ffi_http".to_string());
        ffi_libs.insert("doo_ffi_db".to_string());
        ffi_libs.insert("doo_ffi_auth".to_string());
    }

    // Detect database usage
    let has_db = mir_program.functions.iter().any(|f| {
        f.ffi
            .as_ref()
            .map(|l| l.library == "doo_ffi_db" || l.library == "doo_db")
            .unwrap_or(false)
            || f.name.starts_with("Database::")
    });
    if has_db {
        ffi_libs.insert("doo_ffi_db".to_string());
    }

    // Build search paths
    let search_paths = build_library_search_paths();

    // Platform-specific linking
    #[cfg(target_os = "windows")]
    {
        link_windows(obj_file, opts, &ffi_libs, &search_paths)
    }

    #[cfg(not(target_os = "windows"))]
    {
        link_unix(obj_file, opts, &ffi_libs, &search_paths)
    }
}

/// Build list of paths to search for FFI libraries.
fn build_library_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // Same directory as doo executable
    if let Ok(exe_path) = env::current_exe() {
        if let Some(dir) = exe_path.parent() {
            paths.push(dir.to_path_buf());
        }
    }

    // Current working directory targets
    if let Ok(cwd) = env::current_dir() {
        paths.push(cwd.join("target").join("release"));
        paths.push(cwd.join("target").join("debug"));

        #[cfg(target_os = "windows")]
        paths.push(cwd.join("target-windows").join("release"));

        #[cfg(target_os = "linux")]
        paths.push(cwd.join("target-linux").join("release"));
    }

    // User home directories
    if let Ok(home) = env::var("HOME").or_else(|_| env::var("USERPROFILE")) {
        let home_path = PathBuf::from(&home);
        paths.push(home_path.join(".local").join("bin").join("doo"));
        paths.push(home_path.join(".local").join("lib"));
        paths.push(home_path.join(".doo").join("lib"));
    }

    // System paths (Unix)
    #[cfg(not(target_os = "windows"))]
    {
        paths.push(PathBuf::from("/usr/local/lib"));
        paths.push(PathBuf::from("/usr/lib"));
    }

    paths
}

/// Find an FFI library in search paths.
fn find_ffi_library(lib_name: &str, paths: &[PathBuf]) -> Option<(PathBuf, PathBuf)> {
    for search_path in paths {
        #[cfg(target_os = "windows")]
        {
            // Windows: .dll.lib or .lib
            let dll_lib = search_path.join(format!("{}.dll.lib", lib_name));
            if dll_lib.exists() {
                return Some((search_path.clone(), dll_lib));
            }
            let lib = search_path.join(format!("{}.lib", lib_name));
            if lib.exists() {
                return Some((search_path.clone(), lib));
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            // Unix: lib*.so, lib*.dylib, lib*.a
            let so = search_path.join(format!("lib{}.so", lib_name));
            if so.exists() {
                return Some((search_path.clone(), so));
            }
            let dylib = search_path.join(format!("lib{}.dylib", lib_name));
            if dylib.exists() {
                return Some((search_path.clone(), dylib));
            }
            let a = search_path.join(format!("lib{}.a", lib_name));
            if a.exists() {
                return Some((search_path.clone(), a));
            }
        }
    }
    None
}

// ============================================================================
// Windows Linking
// ============================================================================

#[cfg(target_os = "windows")]
const EMBEDDED_LINKER: &[u8] = include_bytes!("../../../linkers/lld-link.exe");

#[cfg(target_os = "windows")]
fn extract_embedded_linker() -> Result<PathBuf, String> {
    let temp_dir = env::temp_dir();
    let linker_path = temp_dir.join("doo_lld-link.exe");

    let should_write = if linker_path.exists() {
        fs::metadata(&linker_path)
            .map(|m| m.len() != EMBEDDED_LINKER.len() as u64)
            .unwrap_or(true)
    } else {
        true
    };

    if should_write {
        let mut file = fs::File::create(&linker_path)
            .map_err(|e| format!("Failed to create linker file: {}", e))?;
        file.write_all(EMBEDDED_LINKER)
            .map_err(|e| format!("Failed to write linker: {}", e))?;
    }

    Ok(linker_path)
}

#[cfg(target_os = "windows")]
fn link_windows(
    obj_file: &Path,
    opts: &CompileOptions,
    ffi_libs: &HashSet<String>,
    search_paths: &[PathBuf],
) -> Result<PathBuf, String> {
    let linker = extract_embedded_linker()?;
    let exe_path = PathBuf::from(format!("{}.exe", opts.output_name));

    let sdk_paths = find_windows_sdk_paths();

    let mut cmd = Command::new(&linker);
    cmd.arg(format!("/OUT:{}", exe_path.display()))
        .arg(obj_file)
        .arg("/SUBSYSTEM:CONSOLE")
        .arg("/ENTRY:main");

    // Add Windows SDK paths
    if let Some(paths) = sdk_paths {
        if let Some(ucrt) = paths.ucrt_lib {
            cmd.arg(format!("/LIBPATH:{}", ucrt));
        }
        if let Some(um) = paths.um_lib {
            cmd.arg(format!("/LIBPATH:{}", um));
        }
        if let Some(msvc) = paths.msvc_lib {
            cmd.arg(format!("/LIBPATH:{}", msvc));
        }
        cmd.arg("ucrt.lib")
            .arg("vcruntime.lib")
            .arg("legacy_stdio_definitions.lib")
            .arg("libcmt.lib");
    }

    // Link FFI libraries
    let mut added_paths = HashSet::new();
    for lib in ffi_libs {
        if let Some((lib_dir, lib_file)) = find_ffi_library(lib, search_paths) {
            if added_paths.insert(lib_dir.clone()) {
                cmd.arg(format!("/LIBPATH:{}", lib_dir.display()));
            }
            cmd.arg("ws2_32.lib")
                .arg("userenv.lib")
                .arg("bcrypt.lib")
                .arg("kernel32.lib")
                .arg("advapi32.lib");
            cmd.arg(lib_file.to_str().unwrap());
        } else {
            return Err(format!(
                "FFI library '{}.dll.lib' not found.\n\
                Build it first with: cargo build --release\n\
                Searched in: {:?}",
                lib, search_paths
            ));
        }
    }

    let result = cmd.output();
    match result {
        Ok(r) if r.status.success() => Ok(exe_path),
        Ok(r) => Err(format!(
            "Linking failed:\n{}",
            String::from_utf8_lossy(&r.stderr)
        )),
        Err(e) => Err(format!("Linker error: {}", e)),
    }
}

#[cfg(target_os = "windows")]
struct WindowsSdkPaths {
    ucrt_lib: Option<String>,
    um_lib: Option<String>,
    msvc_lib: Option<String>,
}

#[cfg(target_os = "windows")]
fn find_windows_sdk_paths() -> Option<WindowsSdkPaths> {
    let program_files_x86 = env::var("ProgramFiles(x86)").ok()?;
    let kits_base = format!("{}\\Windows Kits\\10\\Lib", program_files_x86);
    let kits_path = Path::new(&kits_base);

    let ucrt_lib = if kits_path.exists() {
        if let Ok(entries) = fs::read_dir(kits_path) {
            let mut versions: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            versions.sort();
            versions
                .last()
                .map(|v| format!("{}\\{}\\ucrt\\x64", kits_base, v))
        } else {
            None
        }
    } else {
        None
    };

    let um_lib = ucrt_lib.as_ref().map(|u| u.replace("ucrt", "um"));

    let msvc_base = format!("{}\\Microsoft Visual Studio", program_files_x86);
    let msvc_lib = find_msvc_lib_path(&msvc_base);

    Some(WindowsSdkPaths {
        ucrt_lib,
        um_lib,
        msvc_lib,
    })
}

#[cfg(target_os = "windows")]
fn find_msvc_lib_path(base: &str) -> Option<String> {
    let base_path = Path::new(base);
    for year in &["2022", "2019", "2017"] {
        for edition in &["BuildTools", "Community", "Professional", "Enterprise"] {
            let vc_path = base_path.join(year).join(edition).join("VC\\Tools\\MSVC");
            if let Ok(entries) = fs::read_dir(&vc_path) {
                if let Some(version) = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .max()
                {
                    return Some(format!("{}\\{}\\lib\\x64", vc_path.display(), version));
                }
            }
        }
    }
    None
}

// ============================================================================
// Unix Linking (Linux & macOS)
// ============================================================================

#[cfg(not(target_os = "windows"))]
fn link_unix(
    obj_file: &Path,
    opts: &CompileOptions,
    ffi_libs: &HashSet<String>,
    search_paths: &[PathBuf],
) -> Result<PathBuf, String> {
    // Check for clang
    if Command::new("clang").arg("--version").output().is_err() {
        return Err("Clang not found. Install with:\n\
            - Ubuntu/Debian: sudo apt install clang\n\
            - Fedora: sudo dnf install clang\n\
            - macOS: xcode-select --install"
            .to_string());
    }

    let exe_path = PathBuf::from(&opts.output_name);

    let mut cmd = Command::new("clang");
    cmd.arg(obj_file).arg("-o").arg(&exe_path).arg("-lm");

    // Platform-specific system libraries
    #[cfg(target_os = "linux")]
    cmd.arg("-lpthread").arg("-ldl");

    #[cfg(target_os = "macos")]
    {
        cmd.arg("-lpthread");
        cmd.arg("-framework").arg("Security");
        cmd.arg("-framework").arg("CoreFoundation");
    }

    // Link FFI libraries in dependency order
    let lib_order = [
        "doo_http",
        "doo_auth",
        "doo_db",
        "doo_file",
        "doo_runtime",
        "doo",
    ];

    let mut sorted_libs: Vec<&String> = lib_order
        .iter()
        .filter_map(|lib| ffi_libs.iter().find(|l| l.as_str() == *lib))
        .collect();
    sorted_libs.extend(ffi_libs.iter().filter(|l| !lib_order.contains(&l.as_str())));

    let mut added_paths = HashSet::new();
    for lib in sorted_libs {
        if let Some((lib_dir, lib_file)) = find_ffi_library(lib, search_paths) {
            let is_shared = lib_file
                .extension()
                .map(|e| e == "so" || e == "dylib")
                .unwrap_or(false);

            if is_shared {
                if added_paths.insert(lib_dir.clone()) {
                    cmd.arg(format!("-L{}", lib_dir.display()));
                    cmd.arg(format!("-Wl,-rpath,{}", lib_dir.display()));
                }
                cmd.arg(format!("-l{}", lib));
            } else {
                cmd.arg(lib_file.to_str().unwrap());
            }
        } else {
            #[cfg(target_os = "macos")]
            let lib_name = format!("lib{}.dylib", lib);
            #[cfg(target_os = "linux")]
            let lib_name = format!("lib{}.so", lib);

            return Err(format!(
                "FFI library '{}' not found.\n\
                Build it first with: cargo build --release\n\
                Searched in: {:?}",
                lib_name, search_paths
            ));
        }
    }

    let result = cmd.output();
    match result {
        Ok(r) if r.status.success() => Ok(exe_path),
        Ok(r) => Err(format!(
            "Linking failed:\n{}",
            String::from_utf8_lossy(&r.stderr)
        )),
        Err(e) => Err(format!("Linker error: {}", e)),
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use doo_core::Span;

    // Helper to create a test span
    fn test_span() -> Span {
        Span::new(0, 0, 10)
    }

    // Helper to create an empty HIR program
    fn empty_program() -> doo_hir::HirProgram {
        doo_hir::HirProgram {
            items: vec![],
            span: test_span(),
        }
    }

    // ========================================================================
    // Error Formatting Tests
    // ========================================================================

    #[test]
    fn test_format_type_error_mismatch() {
        use doo_core::types::builtin;
        let err = TypeError {
            kind: TypeErrorKind::Mismatch {
                expected: builtin::INT,
                found: builtin::STR,
            },
            span: test_span(),
        };
        let formatted = format_type_error(&err);
        assert!(formatted.contains("type mismatch"));
        assert!(formatted.contains("expected"));
        assert!(formatted.contains("found"));
    }

    #[test]
    fn test_format_type_error_undefined() {
        let err = TypeError {
            kind: TypeErrorKind::Undefined("my_var".to_string()),
            span: test_span(),
        };
        let formatted = format_type_error(&err);
        assert!(formatted.contains("undefined variable"));
        assert!(formatted.contains("my_var"));
    }

    #[test]
    fn test_format_type_error_arg_mismatch() {
        let err = TypeError {
            kind: TypeErrorKind::ArgMismatch {
                expected: 2,
                found: 3,
            },
            span: test_span(),
        };
        let formatted = format_type_error(&err);
        assert!(formatted.contains("argument count mismatch"));
        assert!(formatted.contains("expected 2"));
        assert!(formatted.contains("found 3"));
    }

    #[test]
    fn test_format_type_error_invalid_condition() {
        use doo_core::types::builtin;
        let err = TypeError {
            kind: TypeErrorKind::InvalidCondition {
                found: builtin::INT,
            },
            span: test_span(),
        };
        let formatted = format_type_error(&err);
        assert!(formatted.contains("invalid condition type"));
        assert!(formatted.contains("expected Bool"));
    }

    #[test]
    fn test_format_type_error_invalid_cast() {
        use doo_core::types::builtin;
        let err = TypeError {
            kind: TypeErrorKind::InvalidCast {
                from: builtin::STR,
                to: builtin::BOOL,
            },
            span: test_span(),
        };
        let formatted = format_type_error(&err);
        assert!(formatted.contains("invalid cast"));
        assert!(formatted.contains("cannot cast"));
    }

    #[test]
    fn test_format_type_error_return_type_mismatch() {
        use doo_core::types::builtin;
        let err = TypeError {
            kind: TypeErrorKind::ReturnTypeMismatch {
                function: "my_func".to_string(),
                expected: builtin::INT,
                found: builtin::VOID,
            },
            span: test_span(),
        };
        let formatted = format_type_error(&err);
        assert!(formatted.contains("return type mismatch"));
        assert!(formatted.contains("my_func"));
    }

    #[test]
    fn test_format_ownership_error() {
        let err = OwnershipError::new("variable moved", test_span());
        let formatted = format_ownership_error(&err);
        assert!(formatted.contains("Ownership Error"));
        assert!(formatted.contains("variable moved"));
    }

    #[test]
    fn test_format_borrow_error_concurrent_mut() {
        let err =
            BorrowError::concurrent_mut("x".to_string(), Span::new(0, 0, 5), Span::new(0, 10, 15));
        let formatted = format_borrow_error(&err);
        assert!(formatted.contains("Borrow Error"));
        assert!(formatted.contains("Cannot mutably borrow"));
    }

    #[test]
    fn test_format_borrow_error_borrow_while_mut() {
        let err = BorrowError::borrow_while_mut(
            "y".to_string(),
            Span::new(0, 0, 5),
            Span::new(0, 10, 15),
        );
        let formatted = format_borrow_error(&err);
        assert!(formatted.contains("Cannot borrow"));
        assert!(formatted.contains("already mutably borrowed"));
    }

    #[test]
    fn test_format_error_flow_unhandled_result() {
        use doo_core::types::builtin;
        let err = ErrorFlowError::new(
            ErrorFlowErrorKind::UnhandledResult {
                ok_type: builtin::INT,
                err_type: builtin::ERROR,
            },
            test_span(),
        );
        let formatted = format_error_flow_error(&err);
        assert!(formatted.contains("Error Flow Error"));
        assert!(formatted.contains("unhandled Result"));
    }

    #[test]
    fn test_format_error_flow_try_in_non_result() {
        let err = ErrorFlowError::new(
            ErrorFlowErrorKind::TryInNonResultFunction {
                func_name: "my_func".to_string(),
            },
            test_span(),
        );
        let formatted = format_error_flow_error(&err);
        assert!(formatted.contains("`?` operator"));
        assert!(formatted.contains("my_func"));
    }

    #[test]
    fn test_format_error_flow_err_in_non_result() {
        let err = ErrorFlowError::new(
            ErrorFlowErrorKind::ErrInNonResultFunction {
                func_name: "do_something".to_string(),
            },
            test_span(),
        );
        let formatted = format_error_flow_error(&err);
        assert!(formatted.contains("`Err` used in function"));
        assert!(formatted.contains("do_something"));
    }

    #[test]
    fn test_format_error_flow_missing_ok_path() {
        let err = ErrorFlowError::new(
            ErrorFlowErrorKind::MissingOkPath {
                func_name: "parse_data".to_string(),
            },
            test_span(),
        );
        let formatted = format_error_flow_error(&err);
        assert!(formatted.contains("returns Result"));
        assert!(formatted.contains("not all paths return Ok"));
    }

    #[test]
    fn test_format_error_flow_panic_without_message() {
        let err = ErrorFlowError::new(ErrorFlowErrorKind::PanicWithoutMessage, test_span());
        let formatted = format_error_flow_error(&err);
        assert!(formatted.contains("panic"));
        assert!(formatted.contains("without a message"));
    }

    #[test]
    fn test_format_exhaustiveness_error_non_exhaustive() {
        let err = ExhaustivenessError {
            kind: ExhaustivenessErrorKind::NonExhaustive {
                missing: vec!["true".to_string(), "false".to_string()],
            },
            span: test_span(),
        };
        let formatted = format_exhaustiveness_error(&err);
        assert!(formatted.contains("Match Error"));
        assert!(formatted.contains("non-exhaustive"));
        assert!(formatted.contains("true"));
        assert!(formatted.contains("false"));
    }

    #[test]
    fn test_format_exhaustiveness_error_unreachable() {
        let err = ExhaustivenessError {
            kind: ExhaustivenessErrorKind::UnreachablePattern,
            span: test_span(),
        };
        let formatted = format_exhaustiveness_error(&err);
        assert!(formatted.contains("unreachable pattern"));
    }

    // ========================================================================
    // Analysis Integration Tests
    // ========================================================================

    #[test]
    fn test_type_checker_new() {
        let registry = Arc::new(TypeRegistry::new());
        let _checker = TypeChecker::new(registry);
        // Should not panic
    }

    #[test]
    fn test_ownership_analyzer_new() {
        let _analyzer = OwnershipAnalyzer::new();
        // Should not panic
    }

    #[test]
    fn test_borrow_checker_new() {
        let _checker = BorrowChecker::new();
        // Should not panic
    }

    #[test]
    fn test_drop_inserter_new() {
        let _inserter = DropInserter::new();
        // Should not panic
    }

    #[test]
    fn test_error_flow_checker_new() {
        let registry = TypeRegistry::new();
        let _checker = ErrorFlowChecker::new(&registry);
        // Should not panic
    }

    #[test]
    fn test_exhaustiveness_checker_new() {
        let registry = TypeRegistry::new();
        let _checker = ExhaustivenessChecker::new(&registry);
        // Should not panic
    }

    // ========================================================================
    // Analysis Pass Integration Tests (Empty Program)
    // ========================================================================

    #[test]
    fn test_type_checker_empty_program() {
        let hir = empty_program();
        let registry = Arc::new(TypeRegistry::new());
        let mut checker = TypeChecker::new(registry);
        let result = checker.check(&hir);
        assert!(result.is_ok(), "Type checker should pass on empty program");
    }

    #[test]
    fn test_ownership_analyzer_empty_program() {
        let hir = empty_program();
        let mut analyzer = OwnershipAnalyzer::new();
        let result = analyzer.analyze(&hir);
        assert!(
            result.is_ok(),
            "Ownership analyzer should pass on empty program"
        );
    }

    #[test]
    fn test_borrow_checker_empty_program() {
        let hir = empty_program();
        let mut checker = BorrowChecker::new();
        let result = checker.check(&hir);
        assert!(
            result.is_ok(),
            "Borrow checker should pass on empty program"
        );
    }

    #[test]
    fn test_drop_inserter_empty_program() {
        let mut hir = empty_program();
        let mut inserter = DropInserter::new();
        inserter.insert_drops_program(&mut hir);
        // Should not panic
    }

    #[test]
    fn test_error_flow_checker_empty_program() {
        let hir = empty_program();
        let registry = TypeRegistry::new();
        let mut checker = ErrorFlowChecker::new(&registry);
        let result = checker.check(&hir);
        assert!(
            result.is_ok(),
            "Error flow checker should pass on empty program"
        );
    }

    #[test]
    fn test_exhaustiveness_checker_empty_program() {
        let hir = empty_program();
        let registry = TypeRegistry::new();
        let mut checker = ExhaustivenessChecker::new(&registry);
        let errors = checker.check_program(&hir);
        assert!(
            errors.is_empty(),
            "Exhaustiveness checker should pass on empty program"
        );
    }

    // ========================================================================
    // Path Resolution Tests
    // ========================================================================

    #[test]
    fn test_discover_main_doo_candidates_nonexistent_dir() {
        let candidates =
            discover_main_doo_candidates(Path::new("/this/path/does/not/exist"), 4, 25);
        assert!(candidates.is_empty());
    }

    // ========================================================================
    // Compile Options Tests
    // ========================================================================

    #[test]
    fn test_compile_options_default() {
        let opts = CompileOptions::default();
        assert_eq!(opts.output_name, "output");
        assert!(!opts.print_ast);
        assert!(!opts.print_mir);
        assert!(!opts.keep_ll);
        assert!(!opts.keep_obj);
        assert!(!opts.check_only);
    }

    #[test]
    fn test_compile_options_clone() {
        let opts1 = CompileOptions {
            input_path: PathBuf::from("test.doo"),
            output_name: "my_output".to_string(),
            dev_mode: true,
            print_ast: true,
            print_mir: false,
            keep_ll: true,
            keep_obj: false,
            check_only: false,
        };
        let opts2 = opts1.clone();
        assert_eq!(opts1.output_name, opts2.output_name);
        assert_eq!(opts1.print_ast, opts2.print_ast);
        assert_eq!(opts1.keep_ll, opts2.keep_ll);
    }

    // ========================================================================
    // Compile Result Tests
    // ========================================================================

    #[test]
    fn test_compile_result_success() {
        let result = CompileResult {
            success: true,
            error_count: 0,
            exe_path: Some(PathBuf::from("output.exe")),
        };
        assert!(result.success);
        assert_eq!(result.error_count, 0);
        assert!(result.exe_path.is_some());
    }

    #[test]
    fn test_compile_result_failure() {
        let result = CompileResult {
            success: false,
            error_count: 3,
            exe_path: None,
        };
        assert!(!result.success);
        assert_eq!(result.error_count, 3);
        assert!(result.exe_path.is_none());
    }

    // ========================================================================
    // Full Analysis Pipeline Integration Test
    // ========================================================================

    #[test]
    fn test_full_analysis_pipeline_simple_function() {
        use doo_hir::{HirFunction, HirItem, HirProgram, HirStmt, HirStmtKind};

        // Create a simple function with a return statement
        let hir = HirProgram {
            items: vec![HirItem::Function(HirFunction {
                name: "main".to_string(),
                params: vec![],
                return_type: None,
                error_type: None,
                body: vec![HirStmt {
                    kind: HirStmtKind::Return(vec![]),
                    span: test_span(),
                }],
                span: test_span(),
                decorators: vec![],
            })],
            span: test_span(),
        };

        // Run all analysis passes
        let registry = Arc::new(TypeRegistry::new());
        let mut type_checker = TypeChecker::new(registry);
        assert!(type_checker.check(&hir).is_ok(), "Type checker should pass");

        let mut ownership_analyzer = OwnershipAnalyzer::new();
        assert!(
            ownership_analyzer.analyze(&hir).is_ok(),
            "Ownership analyzer should pass"
        );

        let mut borrow_checker = BorrowChecker::new();
        assert!(
            borrow_checker.check(&hir).is_ok(),
            "Borrow checker should pass"
        );

        let registry = TypeRegistry::new();
        let mut error_flow_checker = ErrorFlowChecker::new(&registry);
        assert!(
            error_flow_checker.check(&hir).is_ok(),
            "Error flow checker should pass"
        );

        let mut exhaustiveness_checker = ExhaustivenessChecker::new(&registry);
        let exhaustiveness_errors = exhaustiveness_checker.check_program(&hir);
        assert!(
            exhaustiveness_errors.is_empty(),
            "Exhaustiveness checker should pass"
        );
    }

    #[test]
    fn test_analysis_pipeline_with_variable() {
        use doo_hir::{
            ConstValue, HirExpr, HirExprKind, HirFunction, HirItem, HirProgram, HirStmt,
            HirStmtKind, Ownership,
        };

        // Create HIR with a variable declaration
        let hir = HirProgram {
            items: vec![HirItem::Function(HirFunction {
                name: "main".to_string(),
                params: vec![],
                return_type: None,
                error_type: None,
                body: vec![
                    // let x = 42
                    HirStmt {
                        kind: HirStmtKind::Let {
                            name: "x".to_string(),
                            value: HirExpr {
                                kind: HirExprKind::Const(ConstValue::Int(42)),
                                span: test_span(),
                                type_id: Some(doo_core::types::builtin::INT),
                            },
                            mutable: false,
                            type_id: Some(doo_core::types::builtin::INT),
                            ownership: Ownership::Owned,
                        },
                        span: test_span(),
                    },
                ],
                span: test_span(),
                decorators: vec![],
            })],
            span: test_span(),
        };

        // Run analysis passes
        let registry = Arc::new(TypeRegistry::new());
        let mut type_checker = TypeChecker::new(registry);
        assert!(type_checker.check(&hir).is_ok());

        let mut ownership_analyzer = OwnershipAnalyzer::new();
        assert!(ownership_analyzer.analyze(&hir).is_ok());

        let mut borrow_checker = BorrowChecker::new();
        assert!(borrow_checker.check(&hir).is_ok());

        // Drop insertion should work
        let mut hir_mut = hir.clone();
        let mut drop_inserter = DropInserter::new();
        drop_inserter.insert_drops_program(&mut hir_mut);
    }
}
