//! Compilation Pipeline
//!
//! Orchestrates the full compilation process:
//! Source → Tokens → AST → HIR → MIR → LLVM IR → Executable
//!
//! This module is the **single source of truth** for the Doo compilation pipeline.
//! All compilation commands (build, run, check) flow through this module.

use std::collections::{HashSet, VecDeque};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use doo_codegen::{optimize_module, CodegenBuilder, OptLevel};
use doo_core::doo_debug;
use doo_core::errors::codes::CompilerError;
use doo_core::types::TypeRegistry;
use doo_diagnostics::{DiagnosticEmitter, SourceMap};
use doo_frontend::{Lexer, Parser};
use doo_hir::Lower;
use doo_mir::builder::MirBuilder;
use doo_mir::sym::resolve;

// Module loader - single source of truth for import resolution
use crate::loader::{merge_imports, resolve_imports, ModuleLoader};

// Analysis imports - wire in the semantic analysis phase
use doo_analysis::{
    // Field visibility checking
    check_field_visibility,
    // Error conversions (analysis errors → CompilerError)
    conversions::{
        borrow_errors_to_compiler, error_flow_errors_to_compiler,
        exhaustiveness_errors_to_compiler, ownership_errors_to_compiler, scope_errors_to_compiler,
        type_errors_to_compiler,
    },
    // AST transformations
    transform::{transform_inline_closures, transform_route_groups},
    // Borrow checking
    BorrowChecker,
    // Decorator validation
    DecoratorValidator,
    DropInserter,
    // Error flow analysis
    ErrorFlowChecker,
    // Exhaustiveness checking
    ExhaustivenessChecker,
    // Ownership analysis
    OwnershipAnalyzer,
    // Type checking
    TypeChecker,
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
    /// Print HIR after lowering
    pub print_hir: bool,
    /// Print MIR before codegen
    pub print_mir: bool,
    /// Keep generated LLVM IR (.ll) file
    pub keep_ll: bool,
    /// Keep object file (.o) after linking
    pub keep_obj: bool,
    /// Only check for errors, don't generate code
    pub check_only: bool,
    /// Show warnings (suppressed by default)
    pub show_warnings: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            input_path: PathBuf::from("."),
            output_name: "output".to_string(),
            dev_mode: cfg!(debug_assertions),
            print_ast: false,
            print_hir: false,
            print_mir: false,
            keep_ll: false,
            keep_obj: false,
            check_only: false,
            show_warnings: false,
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
    doo_debug!("DEBUG", "Source length: {} chars", source.len());
    if doo_core::debug::is_enabled() {
        doo_debug!(
            "DEBUG",
            "First 100 chars: {:?}",
            &source[..source.len().min(100)]
        );

        // Debug lexer output
        let mut debug_lexer = Lexer::new(&source, 0);
        let debug_tokens = debug_lexer.tokenize();
        doo_debug!("DEBUG", "Lexer produced {} tokens", debug_tokens.len());
        for (i, tok) in debug_tokens.iter().take(10).enumerate() {
            doo_debug!("DEBUG", "  Token {}: {:?} {:?}", i, tok.kind, tok.text);
        }
    }

    let mut parser = Parser::new(&source, 0);
    let program = match parser.parse_program() {
        Ok(p) => {
            // Collect any non-fatal parser errors (recovered during parsing)
            let parser_errors = parser.errors();
            if !parser_errors.is_empty() {
                let mut source_map = SourceMap::new();
                let main_filename = input_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("main.doo");
                source_map.add_file(main_filename, &source);

                let mut emitter = DiagnosticEmitter::new(true);
                let _ = emitter.emit_all(parser_errors, &source_map);

                // Non-fatal parser errors are warnings — continue compilation
                doo_debug!(
                    "DEBUG",
                    "Parser recovered from {} error(s)",
                    parser_errors.len()
                );
            }
            p
        }
        Err(e) => {
            // Fatal parse error — render with DiagnosticEmitter
            let mut source_map = SourceMap::new();
            let main_filename = input_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("main.doo");
            source_map.add_file(main_filename, &source);

            let mut emitter = DiagnosticEmitter::new(true);
            let _ = emitter.emit(&e, &source_map);

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

    // Collect import errors (previously silently dropped)
    let import_errors = import_resolution.errors.clone();

    // Debug: show what was resolved
    if doo_core::debug::is_enabled() {
        doo_debug!(
            "DEBUG",
            "Import resolution items: {}",
            import_resolution.items.len()
        );
        for item in &import_resolution.items {
            match item {
                doo_frontend::ast::Item::Struct(s) => {
                    doo_debug!("DEBUG", "  Imported Struct: {}", s.name)
                }
                doo_frontend::ast::Item::Function(f) => {
                    doo_debug!("DEBUG", "  Imported Function: {}", f.name)
                }
                doo_frontend::ast::Item::Enum(e) => {
                    doo_debug!("DEBUG", "  Imported Enum: {}", e.name)
                }
                _ => doo_debug!("DEBUG", "  Imported other item"),
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

    // Print HIR if requested
    if opts.print_hir {
        eprintln!("=== HIR ===");
        eprintln!("{:#?}", hir);
    }

    // Debug: Show what functions were found in AST and HIR
    if doo_core::debug::is_enabled() {
        doo_debug!("DEBUG", "AST items: {}", program.items.len());
        for item in &program.items {
            match item {
                doo_frontend::ast::Item::Function(f) => {
                    doo_debug!("DEBUG", "  Function: {}", f.name)
                }
                doo_frontend::ast::Item::Struct(s) => doo_debug!("DEBUG", "  Struct: {}", s.name),
                doo_frontend::ast::Item::Enum(e) => doo_debug!("DEBUG", "  Enum: {}", e.name),
                doo_frontend::ast::Item::Import(i) => doo_debug!("DEBUG", "  Import: {:?}", i.path),
                doo_frontend::ast::Item::Statement(_) => doo_debug!("DEBUG", "  Statement"),
            }
        }
        doo_debug!("DEBUG", "HIR items: {}", hir.items.len());
        for item in &hir.items {
            match item {
                doo_hir::HirItem::Function(f) => doo_debug!("DEBUG", "  HIR Function: {}", f.name),
                doo_hir::HirItem::Struct(s) => doo_debug!("DEBUG", "  HIR Struct: {}", s.name),
                doo_hir::HirItem::Enum(e) => doo_debug!("DEBUG", "  HIR Enum: {}", e.name),
                doo_hir::HirItem::Import(_) => doo_debug!("DEBUG", "  HIR Import"),
            }
        }
    }

    // Phase 5: Semantic Analysis (type checking, name resolution, etc.)

    // ========================================================================
    // Phase 5: Semantic Analysis
    // ========================================================================
    // Run all analysis passes in sequence. The compiler handles ownership,
    // borrowing, types, error flow, and exhaustiveness automatically.
    // Users don't write `&` or `*` - the compiler does it all.

    // Build SourceMap for diagnostic rendering
    let mut source_map = SourceMap::new();
    let main_filename = input_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("main.doo");
    source_map.add_file(main_filename, &source);

    let mut hir = hir; // Make HIR mutable for drop insertion
    let mut analysis_errors: Vec<CompilerError> = Vec::new();

    // 5.0: Import errors (module not found, I/O errors)
    if !import_errors.is_empty() {
        analysis_errors.extend(import_errors);
    }

    // 5.1: Type Checking
    // Validates type compatibility across the program
    let mut type_checker = TypeChecker::new(type_registry.clone());
    if let Err(errors) = type_checker.check(&hir) {
        analysis_errors.extend(type_errors_to_compiler(errors));
    }
    // Collect scope errors (redeclarations, duplicate symbols)
    let scope_errs = type_checker.take_scope_errors();
    if !scope_errs.is_empty() {
        analysis_errors.extend(scope_errors_to_compiler(scope_errs));
    }
    // Collect direct compiler errors (MissingReturn, UnreachableCode, etc.)
    let direct_errs = type_checker.take_direct_errors();
    if !direct_errs.is_empty() {
        analysis_errors.extend(direct_errs);
    }

    // 5.2: Ownership Analysis
    // Tracks variable ownership and decides Move/Copy/Clone automatically
    let mut ownership_analyzer = OwnershipAnalyzer::new();
    let ownership_results = match ownership_analyzer.analyze(&hir) {
        Ok(results) => Some(results),
        Err(errors) => {
            analysis_errors.extend(ownership_errors_to_compiler(errors));
            None
        }
    };

    // 5.3: Borrow Checking
    // Ensures safe memory access - the ONLY error users can see is concurrent mutable borrow
    let mut borrow_checker = BorrowChecker::new();
    if let Err(errors) = borrow_checker.check(&hir) {
        analysis_errors.extend(borrow_errors_to_compiler(errors));
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
        analysis_errors.extend(error_flow_errors_to_compiler(errors));
    }

    // 5.6: Exhaustiveness Checking
    // Ensures all match expressions cover all possible patterns
    let mut exhaustiveness_checker = ExhaustivenessChecker::new(&type_registry);
    let exhaustiveness_errors = exhaustiveness_checker.check_program(&hir);
    analysis_errors.extend(exhaustiveness_errors_to_compiler(exhaustiveness_errors));

    // 5.7: Field Visibility Checking
    // Ensures private fields (camelCase) are not accessed from outside their module
    doo_debug!(
        "DEBUG",
        "Imported struct names: {:?}",
        imported_struct_names
    );
    let visibility_errors = check_field_visibility(&hir, &type_registry, &imported_struct_names);
    for err in visibility_errors {
        let capitalized = {
            let mut c = err.field_name.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().to_string() + c.as_str(),
            }
        };
        analysis_errors.push(
            CompilerError::new(
                doo_core::errors::codes::ErrorCode::PrivateItemAccess,
                format!("'{}' is private", err.field_name),
                err.span,
            )
            .with_suggestion(format!("rename to '{}'", capitalized)),
        );
    }

    // 5.8: Decorator Validation
    // All logic lives in DecoratorValidator::validate_program() — single source of truth.
    // Validates: type compatibility, arg counts, combination conflicts, and
    // compile-time constant values in struct literals (email format, min/max).
    {
        let decorator_validator = DecoratorValidator::new(&type_registry);
        analysis_errors.extend(decorator_validator.validate_program(&hir));
    }

    // Report any analysis errors via the diagnostic emitter
    // Filter warnings unless --warn flag is passed
    let show_warnings = opts.show_warnings;
    let errors_only: Vec<CompilerError> = analysis_errors
        .iter()
        .filter(|e| {
            e.severity == doo_core::errors::codes::ErrorSeverity::Error
                || e.severity == doo_core::errors::codes::ErrorSeverity::Ice
                || show_warnings
        })
        .cloned()
        .collect();
    let has_real_errors = errors_only.iter().any(|e| {
        e.severity == doo_core::errors::codes::ErrorSeverity::Error
            || e.severity == doo_core::errors::codes::ErrorSeverity::Ice
    });

    if !errors_only.is_empty() {
        let mut emitter = DiagnosticEmitter::new(true);
        let _ = emitter.emit_all(&errors_only, &source_map);
        if has_real_errors {
            return Ok(CompileResult {
                success: false,
                error_count: errors_only
                    .iter()
                    .filter(|e| e.severity == doo_core::errors::codes::ErrorSeverity::Error)
                    .count(),
                exe_path: None,
            });
        }
    }

    doo_debug!("DEBUG", "Semantic analysis passed");

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
    if doo_core::debug::is_enabled() {
        doo_debug!("DEBUG", "MIR functions: {}", mir_program.functions.len());
        for f in &mir_program.functions {
            doo_debug!("DEBUG", "  MIR Function: {}", resolve(f.name));
        }
    }

    // Validate MIR
    doo_debug!("DEBUG", "Validating MIR...");
    if let Err(e) = mir_program.validate() {
        return Err(format!("MIR validation failed: {}", e));
    }
    doo_debug!("DEBUG", "MIR validation passed");

    // Check for main function
    let has_main = mir_program
        .functions
        .iter()
        .any(|f| resolve(f.name) == "main");
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
    doo_debug!("DEBUG", "Starting LLVM codegen...");
    let context = Context::create();
    let codegen = CodegenBuilder::new(&context);
    let module = codegen.build(&mir_program, "main_module", type_registry.clone());
    doo_debug!("DEBUG", "LLVM codegen complete");

    // Phase 8: Verify module
    doo_debug!("DEBUG", "Verifying LLVM module...");
    if let Err(e) = module.verify() {
        // Dump IR on verification failure for debugging
        if opts.keep_ll {
            let ll_file = format!("{}.ll", opts.output_name);
            let ir_string = module.print_to_string();
            let _ = fs::write(&ll_file, ir_string.to_string());
        }
        return Err(format!("LLVM module verification failed: {}", e));
    }
    doo_debug!("DEBUG", "LLVM module verified");

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

/// Normalize FFI library name from @extern decorator to actual crate name.
///
/// Generic rule: `doo_X` → `doo_ffi_X`.
/// Only special-cases are aliases where the short name differs from the crate suffix.
fn normalize_ffi_lib_name(name: &str) -> String {
    // Already normalized
    if name.starts_with("doo_ffi_") {
        return name.to_string();
    }

    // Generic rule: doo_X -> doo_ffi_X
    if let Some(suffix) = name.strip_prefix("doo_") {
        match suffix {
            // Aliases where short name doesn't match crate suffix
            "database" => "doo_ffi_db".to_string(),
            "ws" | "websocket" => "doo_ffi_http".to_string(), // WS lives in HTTP crate
            "config" => "doo_ffi_core".to_string(),           // Config lives in core crate
            _ => format!("doo_ffi_{}", suffix),
        }
    } else {
        // Not a doo library — pass through as-is
        name.to_string()
    }
}

/// Link object file into executable.
fn link_object_file(
    obj_file: &Path,
    opts: &CompileOptions,
    mir_program: &doo_mir::MirProgram,
) -> Result<PathBuf, String> {
    // Collect FFI libraries from @extern declarations in MIR.
    // This is entirely discovery-based — only libraries that the program
    // actually imports via @extern will be linked.
    let mut ffi_libs: HashSet<String> = mir_program
        .functions
        .iter()
        .filter_map(|f| {
            f.ffi
                .as_ref()
                .map(|l| normalize_ffi_lib_name(&resolve(l.library)))
        })
        .collect();

    // Always include core runtime and JSON (language fundamentals)
    ffi_libs.insert("doo_ffi_core".to_string());
    ffi_libs.insert("doo_ffi_json".to_string());

    // Transitive dependency: async runtime is needed by HTTP server, WebSocket,
    // and Process — link it when any of those are present or async features used.
    if mir_program.has_async_features()
        || ffi_libs.contains("doo_ffi_http")
        || ffi_libs.contains("doo_ffi_process")
    {
        ffi_libs.insert("doo_ffi_runtime".to_string());
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

            // Search packages/ subdirectories next to executable
            let packages_dir = dir.join("packages");
            if packages_dir.exists() {
                if let Ok(entries) = fs::read_dir(&packages_dir) {
                    for entry in entries.flatten() {
                        if entry.path().is_dir() {
                            paths.push(entry.path());
                        }
                    }
                }
            }
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

        // Search packages/ subdirectories in project root
        let packages_dir = cwd.join("packages");
        if packages_dir.exists() {
            if let Ok(entries) = fs::read_dir(&packages_dir) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        paths.push(entry.path());
                    }
                }
            }
        }
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

    // When linking multiple Rust static libraries, each contains its own copy of
    // the Rust runtime symbols (__rust_alloc, compiler-builtins, etc.).
    // Allow duplicates so the linker uses the first definition and ignores the rest.
    if ffi_libs.len() > 1 {
        cmd.arg("/FORCE:MULTIPLE");
    }

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
                .arg("advapi32.lib")
                .arg("ntdll.lib");
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

    // Enable per-function/data section GC — critical for static linking performance.
    // Combined with --gc-sections in the linker, this eliminates unused code from
    // the Rust runtime copies embedded in each static library.
    cmd.arg("-ffunction-sections").arg("-fdata-sections");

    // Use lld if available — much faster than GNU ld for large Rust static libraries.
    // lld can be 10-50x faster for linking multiple Rust static archives.
    #[cfg(target_os = "linux")]
    let _has_lld = {
        let lld_available = Command::new("ld.lld").arg("--version").output().is_ok();
        if lld_available {
            cmd.arg("-fuse-ld=lld");
        } else {
            eprintln!("hint: install lld for 10-50x faster linking: sudo apt install lld");
        }
        lld_available
    };

    // Platform-specific system libraries
    #[cfg(target_os = "linux")]
    {
        cmd.arg("-lpthread").arg("-ldl");
        // GC unused sections — critical for static linking performance.
        // Each Rust static library embeds the full runtime; --gc-sections
        // strips the 8 duplicate copies, drastically reducing link time.
        cmd.arg("-Wl,--gc-sections");
        // Export symbols to dynamic table only when auth/OAuth is linked —
        // OAuth uses dlsym for cross-library symbol resolution at runtime.
        // Programs without auth skip this, avoiding the overhead of
        // processing all symbols for the dynamic symbol table.
        if ffi_libs.contains("doo_ffi_auth") {
            cmd.arg("-rdynamic");
        }
        // When linking multiple Rust static libraries, each contains its own copy of
        // the Rust runtime symbols (__rust_alloc, compiler-builtins, etc.).
        // Allow duplicates so the linker uses the first definition and ignores the rest.
        if ffi_libs.len() > 1 {
            cmd.arg("-Wl,--allow-multiple-definition");
        }
    }

    #[cfg(target_os = "macos")]
    {
        cmd.arg("-lpthread");
        cmd.arg("-framework").arg("Security");
        cmd.arg("-framework").arg("CoreFoundation");
        // When linking multiple Rust static libraries, each contains its own copy of
        // the Rust runtime symbols (__rust_alloc, compiler-builtins, etc.).
        // Allow duplicates so the linker uses the first definition and ignores the rest.
        if ffi_libs.len() > 1 {
            cmd.arg("-Wl,-multiply_defined,suppress");
        }
    }

    // Link FFI libraries in dependency order
    // Names must match the normalized names in ffi_libs (doo_ffi_* format)
    let lib_order = [
        "doo_ffi_http",
        "doo_ffi_auth",
        "doo_ffi_db",
        "doo_ffi_file",
        "doo_ffi_process",
        "doo_ffi_runtime",
        "doo_ffi_core",
        "doo_ffi_json",
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
                // Static archive (.a) linking.
                // Only use --whole-archive for the HTTP server library — it has
                // runtime-registered route handlers and init code that the linker
                // can't see direct references to. Auth and DB symbols are called
                // explicitly from compiler-generated code and don't need it.
                // Minimizing --whole-archive usage is critical for link speed:
                // each Rust static lib embeds ~10MB of runtime, and --whole-archive
                // forces the linker to process ALL of it.
                #[cfg(target_os = "linux")]
                {
                    let needs_whole_archive = lib.contains("http");
                    if needs_whole_archive {
                        cmd.arg("-Wl,--whole-archive");
                        cmd.arg(lib_file.to_str().unwrap());
                        cmd.arg("-Wl,--no-whole-archive");
                    } else {
                        cmd.arg(lib_file.to_str().unwrap());
                    }
                }
                #[cfg(not(target_os = "linux"))]
                {
                    cmd.arg(lib_file.to_str().unwrap());
                }
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
    // Error Conversion Tests (analysis errors → CompilerError)
    // ========================================================================

    #[test]
    fn test_type_error_to_compiler_error() {
        use doo_analysis::conversions::type_errors_to_compiler;
        use doo_analysis::{TypeError, TypeErrorKind};
        use doo_core::errors::codes::ErrorCode;
        use doo_core::types::builtin;

        let errors = vec![
            TypeError {
                kind: TypeErrorKind::Mismatch {
                    expected: builtin::INT,
                    found: builtin::STR,
                },
                span: test_span(),
            },
            TypeError {
                kind: TypeErrorKind::Undefined("my_var".to_string()),
                span: test_span(),
            },
            TypeError {
                kind: TypeErrorKind::ArgMismatch {
                    expected: 2,
                    found: 3,
                },
                span: test_span(),
            },
        ];
        let compiler_errors = type_errors_to_compiler(errors);
        assert_eq!(compiler_errors.len(), 3);
        assert_eq!(compiler_errors[0].code, ErrorCode::TypeMismatch);
        assert_eq!(compiler_errors[1].code, ErrorCode::UndefinedVariable);
        assert_eq!(compiler_errors[2].code, ErrorCode::ArgCountMismatch);
    }

    #[test]
    fn test_borrow_error_to_compiler_error() {
        use doo_analysis::conversions::borrow_errors_to_compiler;
        use doo_analysis::BorrowError;
        use doo_core::errors::codes::ErrorCode;

        let errors = vec![
            BorrowError::concurrent_mut("x".into(), Span::new(0, 0, 5), Span::new(0, 10, 15)),
            BorrowError::borrow_while_mut("y".into(), Span::new(0, 0, 5), Span::new(0, 10, 15)),
        ];
        let compiler_errors = borrow_errors_to_compiler(errors);
        assert_eq!(compiler_errors.len(), 2);
        assert_eq!(compiler_errors[0].code, ErrorCode::ConcurrentMutableBorrow);
        assert_eq!(compiler_errors[1].code, ErrorCode::ConcurrentMutableBorrow);
        // Should have secondary label pointing to original borrow
        assert_eq!(compiler_errors[0].labels.len(), 1);
    }

    #[test]
    fn test_error_flow_to_compiler_error() {
        use doo_analysis::conversions::error_flow_errors_to_compiler;
        use doo_analysis::{ErrorFlowError, ErrorFlowErrorKind};
        use doo_core::errors::codes::ErrorCode;

        let errors = vec![
            ErrorFlowError::new(ErrorFlowErrorKind::PanicWithoutMessage, test_span()),
            ErrorFlowError::new(
                ErrorFlowErrorKind::TryInNonResultFunction {
                    func_name: "my_func".into(),
                },
                test_span(),
            ),
        ];
        let compiler_errors = error_flow_errors_to_compiler(errors);
        assert_eq!(compiler_errors.len(), 2);
        assert_eq!(compiler_errors[0].code, ErrorCode::PanicWithoutMessage);
        assert_eq!(compiler_errors[1].code, ErrorCode::TryInNonResultFunction);
    }

    #[test]
    fn test_exhaustiveness_to_compiler_error() {
        use doo_analysis::conversions::exhaustiveness_errors_to_compiler;
        use doo_analysis::{ExhaustivenessError, ExhaustivenessErrorKind};
        use doo_core::errors::codes::ErrorCode;

        let errors = vec![
            ExhaustivenessError {
                kind: ExhaustivenessErrorKind::NonExhaustive {
                    missing: vec!["true".into(), "false".into()],
                },
                span: test_span(),
            },
            ExhaustivenessError {
                kind: ExhaustivenessErrorKind::UnreachablePattern,
                span: test_span(),
            },
        ];
        let compiler_errors = exhaustiveness_errors_to_compiler(errors);
        assert_eq!(compiler_errors.len(), 2);
        assert_eq!(compiler_errors[0].code, ErrorCode::NonExhaustiveMatch);
        assert_eq!(compiler_errors[1].code, ErrorCode::UnreachablePattern);
        assert!(compiler_errors[0].message.contains("true"));
    }

    #[test]
    fn test_diagnostic_emitter_integration() {
        use doo_core::errors::codes::{CompilerError, ErrorCode};
        use doo_diagnostics::{DiagnosticEmitter, SourceMap};

        let mut sm = SourceMap::new();
        let fid = sm.add_file("test.doo", "let age: Int = \"twenty\"");

        let err = CompilerError::new(
            ErrorCode::TypeMismatch,
            "Str, expected Int",
            Span::new(fid, 15, 23),
        )
        .with_suggestion("use: 20");

        let mut emitter = DiagnosticEmitter::new(false);
        emitter.emit(&err, &sm).unwrap();
        // Should not panic — output goes to stderr
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
            print_hir: false,
            print_mir: false,
            keep_ll: true,
            keep_obj: false,
            check_only: false,
            show_warnings: false,
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
                is_async: false,
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
                is_async: false,
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
