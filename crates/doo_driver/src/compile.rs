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
use std::time::Instant;

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
use crate::loader::{resolve_imports, ModuleLoader};

// Analysis imports - wire in the semantic analysis phase
use doo_analysis::{
    // Error conversions (analysis errors → CompilerError)
    conversions::{
        borrow_errors_to_compiler, error_flow_errors_to_compiler,
        exhaustiveness_errors_to_compiler, ownership_errors_to_compiler, scope_errors_to_compiler,
        type_errors_to_compiler,
    },
    // Borrow checking
    BorrowChecker,
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
    /// Print phase-by-phase timing information
    pub timings: bool,
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
            timings: false,
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
    let show_timings = opts.timings || env::var("DOO_TIMINGS").is_ok();

    let opts = CompileOptions {
        output_name,
        check_only,
        ..opts
    };

    // Timing accumulator: (phase_name, duration)
    let mut timings: Vec<(&str, std::time::Duration)> = Vec::new();
    let total_start = Instant::now();

    // Phase 0: Locate main.doo
    let t = Instant::now();
    let input_path = resolve_input_path(&opts.input_path)?;
    timings.push(("Resolve input", t.elapsed()));

    // Phase 1: Read source
    let t = Instant::now();
    let source = fs::read_to_string(&input_path)
        .map_err(|e| format!("Failed to read {}: {}", input_path.display(), e))?;
    timings.push(("Read source", t.elapsed()));

    let project_root = input_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Phase 2: Parse (Parser creates lexer internally)
    // Debug: Show source info
    doo_debug!("DEBUG", "Source length: {} chars", source.len());
    let t = Instant::now();
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
            let _ = emitter.emit_all(&e, &source_map);

            return Ok(CompileResult {
                success: false,
                error_count: 1,
                exe_path: None,
            });
        }
    };
    timings.push(("Parse", t.elapsed()));

    // Phase 3: Resolve Imports FIRST (so transforms see the complete program)
    // Load and merge imported functions/structs/enums from std library and other modules
    let t = Instant::now();
    let mut program = program;
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

    program.items.extend(import_resolution.items);
    timings.push(("Import resolution", t.elapsed()));

    // Phase 3.5: AST Transformations — removed (framework route DSL).
    // Route group transforms and inline closure transforms are framework
    // concerns that belong in a macro crate, not in the pure compiler.
    // The doo_analysis::transform module has been deleted.
    let t = Instant::now();
    // transform_route_groups(&mut program);       // REMOVED: framework DSL
    // transform_inline_closures(&mut program);   // REMOVED: framework DSL
    timings.push(("AST transforms", t.elapsed()));

    if opts.print_ast {
        eprintln!("=== AST ===");
        eprintln!("{:#?}", program);
    }

    // Phase 4: Lower to HIR (with type information)
    let t = Instant::now();
    let mut type_registry = TypeRegistry::new();
    let mut lowerer = Lower::new();
    let mut hir = lowerer.lower_program_typed(&program, &mut type_registry);

    // Phase 4.5: Monomorphization
    // Transforms generic function/struct templates into concrete specializations.
    // Must run BEFORE analysis (which doesn't understand TypeParam) and MIR building.
    {
        let mut mono = doo_hir::Monomorphizer::new(&mut type_registry);
        mono.monomorphize(&mut hir);
    }

    // Phase 4.6: Closure Type Inference
    // Infer closure parameter/return types from function call context.
    // e.g., fn applyToAll(items: [Int], action: fn(Int) -> Int) → closure (x) => x*2
    //       infers x: Int from the expected fn(Int) -> Int type.
    lowerer.infer_closure_types_in_program(&mut hir, &mut type_registry);

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
                doo_frontend::ast::Item::Const(c) => doo_debug!("DEBUG", "  Const: {}", c.name),
                doo_frontend::ast::Item::Interface(i) => {
                    doo_debug!("DEBUG", "  Interface: {}", i.name)
                }
                doo_frontend::ast::Item::Static(s) => {
                    doo_debug!("DEBUG", "  Static: {}", s.name)
                }
                doo_frontend::ast::Item::Impl(i) => {
                    doo_debug!("DEBUG", "  Impl for {}", i.struct_name)
                }
            }
        }
        doo_debug!("DEBUG", "HIR items: {}", hir.items.len());
        for item in &hir.items {
            match item {
                doo_hir::HirItem::Const(c) => doo_debug!("DEBUG", "  HIR Const: {}", c.name),
                doo_hir::HirItem::Function(f) => doo_debug!("DEBUG", "  HIR Function: {}", f.name),
                doo_hir::HirItem::Struct(s) => doo_debug!("DEBUG", "  HIR Struct: {}", s.name),
                doo_hir::HirItem::Enum(e) => doo_debug!("DEBUG", "  HIR Enum: {}", e.name),
                doo_hir::HirItem::Import(_) => doo_debug!("DEBUG", "  HIR Import"),
                doo_hir::HirItem::Interface(i) => {
                    doo_debug!("DEBUG", "  HIR Interface: {}", i.name)
                }
                doo_hir::HirItem::Static(s) => {
                    doo_debug!("DEBUG", "  HIR Static: {}", s.name)
                }
            }
        }
    }

    // ========================================================================
    // Phase 5: Semantic Analysis
    // ========================================================================
    timings.push(("HIR lowering", t.elapsed()));
    let t = Instant::now();
    // Run all analysis passes in sequence. The compiler handles ownership,
    // borrowing, types, error flow, and exhaustiveness automatically.
    // Users don't write `&` or `*` - the compiler does it all.

    // Build SourceMap for diagnostic rendering
    let mut source_map = SourceMap::new();
    let main_filename = input_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("main.doo");
    source_map.add_file(main_filename, &source); // file_id = 0

    // Register all imported module sources in the SourceMap.
    let mut imported = loader.imported_sources().to_vec();
    imported.sort_by_key(|(fid, _, _)| *fid);
    for (expected_id, name, src) in &imported {
        let actual_id = source_map.add_file(name, src);
        debug_assert_eq!(
            actual_id,
            doo_core::FileId(*expected_id),
            "file_id mismatch for {}",
            name
        );
    }

    let mut hir = hir; // Make HIR mutable for drop insertion
    let mut analysis_errors: Vec<CompilerError> = Vec::new();

    // 5.0: Import errors (module not found, I/O errors)
    if !import_errors.is_empty() {
        analysis_errors.extend(import_errors);
    }

    // 5.1: Type Checking
    let mut type_checker = TypeChecker::new(type_registry.as_ref());
    let mut thir_lowerer = doo_thir::ThirLoweringContext::new(type_registry.as_ref());
    let thir = thir_lowerer.lower_program(&hir);

    if let Err(errors) = type_checker.check_program(&thir) {
        analysis_errors.extend(type_errors_to_compiler(errors));
    }

    // 5.2: Ownership Analysis
    let mut ownership_analyzer = OwnershipAnalyzer::new();
    let ownership_results = match ownership_analyzer.analyze(&hir) {
        Ok(results) => Some(results),
        Err(errors) => {
            analysis_errors.extend(ownership_errors_to_compiler(errors));
            None
        }
    };

    // 5.3: Borrow Checking
    let mut borrow_checker = BorrowChecker::new();
    if let Err(errors) = borrow_checker.check(&hir) {
        analysis_errors.extend(borrow_errors_to_compiler(errors));
    }

    // 5.4: Drop Insertion
    let mut drop_inserter = if let Some(ref results) = ownership_results {
        DropInserter::with_ownership_results(results)
    } else {
        DropInserter::new()
    };
    drop_inserter.insert_drops_program(&mut hir);

    // 5.5: Error Flow Checking
    let mut error_flow_checker = ErrorFlowChecker::new(&type_registry);
    if let Err(errors) = error_flow_checker.check_thir(&thir) {
        analysis_errors.extend(error_flow_errors_to_compiler(errors));
    }

    // 5.6: Exhaustiveness Checking
    let mut exhaustiveness_checker = ExhaustivenessChecker::new(&type_registry);
    if let Err(errors) = exhaustiveness_checker.check_program(&thir) {
        analysis_errors.extend(exhaustiveness_errors_to_compiler(errors));
    }

    // Report any analysis errors via the diagnostic emitter
    let show_warnings = opts.show_warnings;
    let errors_only: Vec<CompilerError> = {
        let mut seen = HashSet::new();
        analysis_errors
            .iter()
            .filter(|e| {
                e.severity == doo_core::errors::codes::ErrorSeverity::Error
                    || e.severity == doo_core::errors::codes::ErrorSeverity::Ice
                    || show_warnings
            })
            .filter(|e| {
                let key = (
                    std::mem::discriminant(&e.code),
                    e.span.start,
                    e.span.end,
                    e.file_id,
                );
                seen.insert(key)
            })
            .cloned()
            .collect()
    };
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
    timings.push(("Semantic analysis", t.elapsed()));

    // If check-only, we're done — skip MIR and codegen
    if opts.check_only {
        if show_timings {
            print_timings(&timings, total_start.elapsed());
        }
        return Ok(CompileResult {
            success: true,
            error_count: 0,
            exe_path: None,
        });
    }

    // Phase 6: Build MIR
    let t = Instant::now();
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

    if doo_core::debug::is_enabled() {
        doo_debug!("DEBUG", "MIR functions: {}", mir_program.functions.len());
        for f in &mir_program.functions {
            doo_debug!("DEBUG", "  MIR Function: {}", resolve(f.name));
        }
    }

    doo_debug!("DEBUG", "Validating MIR...");
    if let Err(e) = mir_program.validate() {
        return Err(format!("MIR validation failed: {}", e));
    }
    doo_debug!("DEBUG", "MIR validation passed");

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

    timings.push(("MIR build", t.elapsed()));

    // Phase 7: LLVM Codegen
    let t = Instant::now();
    doo_debug!("DEBUG", "Starting LLVM codegen...");
    let context = Context::create();
    let codegen = CodegenBuilder::new(&context);
    let module = codegen.build(&mir_program, "main_module", type_registry.clone());
    drop(codegen);
    doo_debug!("DEBUG", "LLVM codegen complete");
    timings.push(("LLVM codegen", t.elapsed()));

    // Phase 8: Verify module (SKIPPED: crashes with LLVM 22 on Windows)
    let t = Instant::now();
    doo_debug!(
        "DEBUG",
        "Verifying LLVM module... (skipped for LLVM 22 compat)"
    );
    timings.push(("LLVM verify", t.elapsed()));

    // Phase 9: Optimize
    let t = Instant::now();
    optimize_module(&module, OptLevel::O3);

    // Phase 9.1: Harden ALL functions against stack corruption post-optimization.
    //
    // LLVM's O3 pass pipeline can strip function attributes and add `tail call`
    // markers during transforms. On Windows x64, functions returning structs
    // >8 bytes use sret (hidden stack pointer). If the backend honours a `tail
    // call` inside such a function, it reuses the caller's frame while sret
    // still points into it → corrupted return address → DEP violation.
    {
        use inkwell::values::InstructionOpcode;

        let no_tail = context.create_string_attribute("disable-tail-calls", "true");
        let frame_ptr = context.create_string_attribute("frame-pointer", "all");
        let ssp = context.create_string_attribute("sspstrong", "");
        let ssp_buf_size = context.create_string_attribute("ssp-buffer-size", "4");
        let mut func = module.get_first_function();
        while let Some(f) = func {
            f.add_attribute(inkwell::attributes::AttributeLoc::Function, no_tail);
            f.add_attribute(inkwell::attributes::AttributeLoc::Function, frame_ptr);
            f.add_attribute(inkwell::attributes::AttributeLoc::Function, ssp);
            f.add_attribute(inkwell::attributes::AttributeLoc::Function, ssp_buf_size);

            let mut bb = f.get_first_basic_block();
            while let Some(block) = bb {
                let mut instr = block.get_first_instruction();
                while let Some(inst) = instr {
                    if inst.get_opcode() == InstructionOpcode::Call {
                        if let Ok(call_site) = inkwell::values::CallSiteValue::try_from(inst) {
                            if call_site.is_tail_call() {
                                call_site.set_tail_call(false);
                            }
                        }
                    }
                    instr = inst.get_next_instruction();
                }
                bb = block.get_next_basic_block();
            }

            func = f.get_next_function();
        }
    }

    // Phase 9.5: Add POSIX compatibility stubs AFTER optimization (Windows only)
    #[cfg(target_os = "windows")]
    add_posix_compat_stubs(&module);
    timings.push(("Optimize + harden", t.elapsed()));

    // Phase 10: Write LLVM IR if requested
    if opts.keep_ll {
        let ll_file = format!("{}.ll", opts.output_name);
        let ir_string = module.print_to_string();
        let ir_rust = ir_string.to_str().unwrap_or("").to_string();
        std::mem::forget(ir_string);
        fs::write(&ll_file, &ir_rust).map_err(|e| format!("Failed to write LLVM IR: {}", e))?;
    }

    // Phase 12: Compile to object file
    let t = Instant::now();
    let obj_file = compile_to_object(&module, &opts)?;
    timings.push(("Compile to object", t.elapsed()));

    // Phase 13: Link
    let t = Instant::now();
    let exe_path = link_object_file(&obj_file, &opts, &mir_program)?;
    timings.push(("Link", t.elapsed()));

    // Cleanup object file unless requested to keep
    if !opts.keep_obj {
        let _ = fs::remove_file(&obj_file);
    }

    // Print timing summary
    if show_timings {
        print_timings(&timings, total_start.elapsed());
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

fn print_timings(timings: &[(&str, std::time::Duration)], total: std::time::Duration) {
    eprintln!();
    eprintln!("=== Compilation Timings ===");
    let max_label = timings.iter().map(|(l, _)| l.len()).max().unwrap_or(0);
    for (label, dur) in timings {
        let ms = dur.as_secs_f64() * 1000.0;
        let pct = if total.as_nanos() > 0 {
            (dur.as_nanos() as f64 / total.as_nanos() as f64) * 100.0
        } else {
            0.0
        };
        eprintln!(
            "  {:<width$}  {:>8.2}ms  ({:>5.1}%)",
            label,
            ms,
            pct,
            width = max_label
        );
    }
    eprintln!(
        "  {:-<width$}  {:-^8}--  -------",
        "",
        "",
        width = max_label
    );
    eprintln!(
        "  {:<width$}  {:>8.2}ms",
        "Total",
        total.as_secs_f64() * 1000.0,
        width = max_label
    );
    eprintln!();
}

fn try_find_main_in(dir: &Path) -> Option<PathBuf> {
    let main_file = dir.join("main.doo");
    if main_file.exists() {
        return Some(main_file);
    }
    let src_main = dir.join("src").join("main.doo");
    if src_main.exists() {
        return Some(src_main);
    }
    None
}

fn find_project_up(start: &Path) -> Option<(PathBuf, PathBuf)> {
    let mut current = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };

    loop {
        if let Some(main_path) = try_find_main_in(&current) {
            return Some((current, main_path));
        }
        current = current.parent()?.to_path_buf();
    }
}

fn resolve_input_path(input: &Path) -> Result<PathBuf, String> {
    if input.is_file() {
        return Ok(input.to_path_buf());
    }

    if input.is_dir() {
        if let Some(main_path) = try_find_main_in(input) {
            return Ok(main_path);
        }
    }

    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if let Some((_root, main_path)) = find_project_up(&cwd) {
        return Ok(main_path);
    }

    let search_root = if input.is_dir() {
        input.to_path_buf()
    } else {
        cwd
    };
    let candidates = discover_main_doo_candidates(&search_root, 4, 25);

    if candidates.len() == 1 {
        return Ok(candidates[0].clone());
    }

    if candidates.is_empty() {
        return Err(format!(
            "Error: main.doo not found in {} or {}/src",
            search_root.display(),
            search_root.display()
        ));
    }

    if let Ok(entry) = env::var("DOO_ENTRY") {
        let entry_path = PathBuf::from(&entry);
        let entry_path = if entry_path.is_absolute() {
            entry_path
        } else {
            search_root.join(&entry_path)
        };

        if entry_path.is_file() {
            return Ok(entry_path);
        }
        if let Some(main_path) = try_find_main_in(&entry_path) {
            return Ok(main_path);
        }
    }

    let display_path = if input.is_dir() || input.is_file() {
        input
    } else {
        &search_root
    };
    let mut msg = format!(
        "Error: main.doo not found in {} or {}/src\n\nFound multiple candidates:\n",
        display_path.display(),
        display_path.display()
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

        let main_file = dir.join("main.doo");
        if main_file.exists() {
            results.push(main_file);
            if results.len() >= max_results {
                break;
            }
        }

        let src_main = dir.join("src").join("main.doo");
        if src_main.exists() {
            results.push(src_main);
            if results.len() >= max_results {
                break;
            }
        }

        if depth < max_depth {
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }

                    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
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

fn compile_to_object(
    module: &inkwell::module::Module,
    opts: &CompileOptions,
) -> Result<PathBuf, String> {
    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("Failed to initialize target: {}", e))?;

    let triple = TargetMachine::get_default_triple();
    let cpu_llvm = TargetMachine::get_host_cpu_name();
    let features_llvm = TargetMachine::get_host_cpu_features();
    let cpu = cpu_llvm.to_str().unwrap_or("generic").to_string();
    let features = features_llvm.to_str().unwrap_or("").to_string();

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

    std::mem::forget(target_machine);
    std::mem::forget(triple);
    std::mem::forget(cpu_llvm);
    std::mem::forget(features_llvm);

    Ok(obj_path)
}

// ============================================================================
// Linking — Pure compiler: links only Tier A runtime + @extern-declared libs
// ============================================================================

/// Normalize FFI library name: `doo_X` → `doo_ffi_X`.
/// No special cases — the compiler does not know which libraries exist.
fn normalize_ffi_lib_name(name: &str) -> String {
    if name.starts_with("doo_ffi_") {
        return name.to_string();
    }
    if let Some(suffix) = name.strip_prefix("doo_") {
        format!("doo_ffi_{}", suffix)
    } else {
        name.to_string()
    }
}

fn link_object_file(
    obj_file: &Path,
    opts: &CompileOptions,
    mir_program: &doo_mir::MirProgram,
) -> Result<PathBuf, String> {
    // Collect FFI libraries from @extern declarations in MIR.
    // Only libraries the program actually imports will be linked.
    let mut ffi_libs: HashSet<String> = mir_program
        .functions
        .iter()
        .filter_map(|f| {
            f.ffi
                .as_ref()
                .map(|l| normalize_ffi_lib_name(&resolve(l.library)))
        })
        .collect();

    // Always include Tier A runtime (language fundamentals)
    ffi_libs.insert("doo_ffi_core".to_string());
    ffi_libs.insert("doo_ffi_json".to_string());

    // Link async runtime when the program uses async features
    if mir_program.has_async_features() {
        ffi_libs.insert("doo_ffi_runtime".to_string());
    }

    let search_paths = build_library_search_paths();

    #[cfg(target_os = "windows")]
    {
        link_windows(obj_file, opts, &ffi_libs, &search_paths)
    }

    #[cfg(not(target_os = "windows"))]
    {
        link_unix(obj_file, opts, &ffi_libs, &search_paths)
    }
}

fn build_library_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(exe_path) = env::current_exe() {
        if let Some(dir) = exe_path.parent() {
            paths.push(dir.to_path_buf());

            let lib_dir = dir.join("lib");
            if lib_dir.exists() {
                paths.push(lib_dir);
            }

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

    if let Ok(cwd) = env::current_dir() {
        paths.push(cwd.join("target").join("release"));
        paths.push(cwd.join("target").join("debug"));

        #[cfg(target_os = "windows")]
        paths.push(cwd.join("target-windows").join("release"));

        #[cfg(target_os = "linux")]
        paths.push(cwd.join("target-linux").join("release"));

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

    if let Ok(home) = env::var("HOME").or_else(|_| env::var("USERPROFILE")) {
        let home_path = PathBuf::from(&home);
        paths.push(home_path.join(".local").join("bin").join("doo"));
        paths.push(home_path.join(".local").join("lib"));
        paths.push(home_path.join(".doo").join("lib"));
    }

    #[cfg(not(target_os = "windows"))]
    {
        paths.push(PathBuf::from("/usr/local/lib"));
        paths.push(PathBuf::from("/usr/lib"));
    }

    paths
}

fn find_ffi_library(lib_name: &str, paths: &[PathBuf]) -> Option<(PathBuf, PathBuf)> {
    for search_path in paths {
        #[cfg(target_os = "windows")]
        {
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
fn add_posix_compat_stubs(module: &inkwell::module::Module) {
    use inkwell::module::Linkage;
    use inkwell::AddressSpace;

    let ctx = module.get_context();
    let ptr_type = ctx.ptr_type(AddressSpace::default());

    static POSIX_STUBS: &[&str] = &[
        "close", "read", "open", "write", "lseek", "access", "chmod", "mkdir", "rmdir", "unlink",
        "stat", "fstat", "dup", "dup2", "fileno", "isatty", "getcwd", "chdir", "umask", "mktemp",
        "setmode", "strdup", "stricmp", "strnicmp", "pipe", "fdopen", "popen", "pclose", "putenv",
        "getpid", "swab", "tempnam", "tzset", "wopen", "waccess", "wstat",
    ];

    let generic_fn_ty = ctx.void_type().fn_type(&[], false);

    for posix_name in POSIX_STUBS {
        let imp_name = format!("__imp_{}", posix_name);

        if module.get_global(&imp_name).is_some() {
            continue;
        }

        let msvc_name = format!("_{}", posix_name);

        let msvc_fn = module.get_function(&msvc_name).unwrap_or_else(|| {
            module.add_function(&msvc_name, generic_fn_ty, Some(Linkage::External))
        });

        let imp_global = module.add_global(ptr_type, Some(AddressSpace::default()), &imp_name);
        imp_global.set_initializer(&msvc_fn.as_global_value().as_pointer_value());
        imp_global.set_linkage(Linkage::External);
        imp_global.set_constant(true);
    }
}

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
    let exe_path = PathBuf::from(format!("{}.exe", opts.output_name));

    let sdk_paths = find_windows_sdk_paths();

    let linker = sdk_paths
        .as_ref()
        .and_then(|p| p.msvc_lib.as_ref())
        .and_then(|lib| find_msvc_linker(lib))
        .map(Ok)
        .unwrap_or_else(|| extract_embedded_linker())?;

    let mut cmd = Command::new(&linker);

    let map_path = exe_path.with_extension("map");
    cmd.arg(format!("/MAP:{}", map_path.display()));

    cmd.arg(format!("/OUT:{}", exe_path.display()))
        .arg(obj_file)
        .arg("/SUBSYSTEM:CONSOLE")
        .arg("/ENTRY:mainCRTStartup");

    let use_force_multiple = ffi_libs.len() > 1;
    if use_force_multiple {
        cmd.arg("/FORCE:MULTIPLE");
        cmd.arg("/OPT:NOICF,NOREF");
        let map_path = exe_path.with_extension("map");
        cmd.arg(format!("/MAP:{}", map_path.display()));
    }

    cmd.arg("/NODEFAULTLIB:MSVCRT")
        .arg("/NODEFAULTLIB:MSVCRTD")
        .arg("/NODEFAULTLIB:libcmtd");

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
        cmd.arg("/WHOLEARCHIVE:libucrt.lib")
            .arg("libvcruntime.lib")
            .arg("libcmt.lib")
            .arg("legacy_stdio_definitions.lib");
    }

    // Core Windows runtime libraries only — no framework system libraries.
    // Framework packages (HTTP, DB, Auth) declare their own system dependencies.
    cmd.arg("kernel32.lib")
        .arg("advapi32.lib")
        .arg("ntdll.lib")
        .arg("userenv.lib");

    // Link FFI libraries in alphabetical order for deterministic builds.
    let mut lib_entries: Vec<(&String, PathBuf, PathBuf)> = Vec::new();
    for lib in ffi_libs.iter() {
        if let Some((lib_dir, lib_file)) = find_ffi_library(lib, search_paths) {
            lib_entries.push((lib, lib_dir, lib_file));
        } else {
            return Err(format!(
                "FFI library '{}' (.lib/.dll.lib) not found.\n\
                Build it first with: cargo build --release\n\
                Searched in: {:?}",
                lib, search_paths
            ));
        }
    }

    lib_entries.sort_by(|a, b| a.0.cmp(b.0));

    let mut added_paths = HashSet::new();
    for (_, lib_dir, lib_file) in &lib_entries {
        if added_paths.insert(lib_dir.clone()) {
            cmd.arg(format!("/LIBPATH:{}", lib_dir.display()));
        }
        cmd.arg(lib_file.to_string_lossy().as_ref());
    }

    let result = cmd.output();
    match result {
        Ok(r) if r.status.success() => {
            let stderr = String::from_utf8_lossy(&r.stderr);
            if !stderr.is_empty() {
                let debug = std::env::var("DOO_DEBUG").is_ok();
                if debug {
                    eprintln!("[LINKER] Warnings:\n{}", stderr);
                }
            }
            Ok(exe_path)
        }
        Ok(r) => {
            let stderr = String::from_utf8_lossy(&r.stderr);
            let stdout = String::from_utf8_lossy(&r.stdout);
            let mut msg = String::from("Linking failed:");
            if !stderr.is_empty() {
                msg.push('\n');
                msg.push_str(&stderr);
            }
            if !stdout.is_empty() {
                msg.push('\n');
                msg.push_str(&stdout);
            }
            Err(msg)
        }
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
    if !base_path.exists() {
        return None;
    }

    let mut best: Option<(String, String)> = None;

    if let Ok(years) = fs::read_dir(base_path) {
        for year_entry in years.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()) {
            let year_name = year_entry.file_name().to_string_lossy().to_string();
            if let Ok(editions) = fs::read_dir(year_entry.path()) {
                for edition_entry in editions
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                {
                    let vc_path = edition_entry.path().join("VC").join("Tools").join("MSVC");
                    if let Ok(versions) = fs::read_dir(&vc_path) {
                        if let Some(version) = versions
                            .filter_map(|e| e.ok())
                            .filter(|e| e.path().is_dir())
                            .map(|e| e.file_name().to_string_lossy().to_string())
                            .max()
                        {
                            let path = format!("{}\\{}\\lib\\x64", vc_path.display(), version);
                            let sort_key = format!("{}_{}", year_name, version);
                            if best.as_ref().map(|(k, _)| sort_key > *k).unwrap_or(true) {
                                best = Some((sort_key, path));
                            }
                        }
                    }
                }
            }
        }
    }

    best.map(|(_, path)| path)
}

#[cfg(target_os = "windows")]
fn find_msvc_linker(msvc_lib: &str) -> Option<PathBuf> {
    let path = Path::new(msvc_lib);
    let msvc_version_root = path.parent()?.parent()?;
    let linker = msvc_version_root
        .join("bin")
        .join("Hostx64")
        .join("x64")
        .join("link.exe");
    if linker.exists() {
        Some(linker)
    } else {
        None
    }
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

    cmd.arg("-ffunction-sections").arg("-fdata-sections");

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

    // Core system libraries only — no framework dependencies.
    // Framework packages declare their own system library dependencies.
    #[cfg(target_os = "linux")]
    {
        cmd.arg("-lpthread").arg("-ldl");
        cmd.arg("-Wl,--gc-sections");
        if ffi_libs.len() > 1 {
            cmd.arg("-rdynamic");
        }
        if ffi_libs.len() > 1 {
            cmd.arg("-Wl,--allow-multiple-definition");
        }
    }

    #[cfg(target_os = "macos")]
    {
        cmd.arg("-lpthread");
        if ffi_libs.len() > 1 {
            cmd.arg("-Wl,-multiply_defined,suppress");
            cmd.arg("-Wl,-export_dynamic");
        }
    }

    // Link all FFI libraries in alphabetical order — no hardcoded dependency list.
    let mut sorted_libs: Vec<&String> = ffi_libs.iter().collect();
    sorted_libs.sort();

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
                // Link all static archives the same way — no special whole-archive treatment.
                cmd.arg(lib_file.to_string_lossy().as_ref());
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

    fn test_span() -> Span {
        Span::new(0, 10)
    }

    fn empty_program() -> doo_hir::HirProgram {
        doo_hir::HirProgram {
            items: vec![],
            span: test_span(),
        }
    }

    #[test]
    fn test_type_error_to_compiler_error() {
        use doo_analysis::conversions::type_errors_to_compiler;
        use doo_analysis::{TypeError, TypeErrorKind};
        use doo_core::errors::codes::ErrorCode;
        use doo_core::types::builtin;

        let errors = vec![
            TypeError {
                kind: TypeErrorKind::Mismatch {
                    expected: builtin::INT.to_string(),
                    found: builtin::STR.to_string(),
                },
                span: test_span(),
            },
            TypeError {
                kind: TypeErrorKind::Undefined("my_var".to_string(), None),
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
            BorrowError::concurrent_mut("x".into(), Span::new(0, 5), Span::new(10, 15)),
            BorrowError::borrow_while_mut("y".into(), Span::new(0, 5), Span::new(10, 15)),
        ];
        let compiler_errors = borrow_errors_to_compiler(errors);
        assert_eq!(compiler_errors.len(), 2);
        assert_eq!(compiler_errors[0].code, ErrorCode::ConcurrentMutableBorrow);
        assert_eq!(compiler_errors[1].code, ErrorCode::ConcurrentMutableBorrow);
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
        let _fid = sm.add_file("test.doo", "let age: Int = \"twenty\"");

        let err = CompilerError::new(
            ErrorCode::TypeMismatch,
            "Str, expected Int",
            Span::new(15, 23),
        )
        .with_suggestion("use: 20");

        let mut emitter = DiagnosticEmitter::new(false);
        emitter.emit(&err, &sm).unwrap();
    }

    #[test]
    fn test_type_checker_new() {
        let registry = Arc::new(TypeRegistry::new());
        let _checker = TypeChecker::new(&registry);
    }

    #[test]
    fn test_ownership_analyzer_new() {
        let _analyzer = OwnershipAnalyzer::new();
    }

    #[test]
    fn test_borrow_checker_new() {
        let _checker = BorrowChecker::new();
    }

    #[test]
    fn test_drop_inserter_new() {
        let _inserter = DropInserter::new();
    }

    #[test]
    fn test_error_flow_checker_new() {
        let registry = TypeRegistry::new();
        let _checker = ErrorFlowChecker::new(&registry);
    }

    #[test]
    fn test_exhaustiveness_checker_new() {
        let registry = TypeRegistry::new();
        let _checker = ExhaustivenessChecker::new(&registry);
    }

    #[test]
    fn test_type_checker_empty_program() {
        let hir = empty_program();
        let registry = Arc::new(TypeRegistry::new());
        let mut lowerer = doo_thir::ThirLoweringContext::new(registry.as_ref());
        let thir = lowerer.lower_program(&hir);
        let mut checker = TypeChecker::new(registry.as_ref());
        let result = checker.check_program(&thir);
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
    }

    #[test]
    fn test_error_flow_checker_empty_program() {
        let hir = empty_program();
        let registry = TypeRegistry::new();
        let mut checker = ErrorFlowChecker::new(&registry);
        let result = checker.check_hir(&hir);
        assert!(
            result.is_ok(),
            "Error flow checker should pass on empty program"
        );
    }

    #[test]
    fn test_exhaustiveness_checker_empty_program() {
        let hir = empty_program();
        let registry = TypeRegistry::new();
        let mut lowerer = doo_thir::ThirLoweringContext::new(&registry);
        let thir = lowerer.lower_program(&hir);
        let mut checker = ExhaustivenessChecker::new(&registry);
        let result = checker.check_program(&thir);
        assert!(
            result.is_ok(),
            "Exhaustiveness checker should pass on empty program"
        );
    }

    #[test]
    fn test_discover_main_doo_candidates_nonexistent_dir() {
        let candidates =
            discover_main_doo_candidates(Path::new("/this/path/does/not/exist"), 4, 25);
        assert!(candidates.is_empty());
    }

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
            timings: false,
        };
        let opts2 = opts1.clone();
        assert_eq!(opts1.output_name, opts2.output_name);
        assert_eq!(opts1.print_ast, opts2.print_ast);
        assert_eq!(opts1.keep_ll, opts2.keep_ll);
    }

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

    #[test]
    fn test_full_analysis_pipeline_simple_function() {
        use doo_hir::{HirFunction, HirItem, HirProgram, HirStmt, HirStmtKind};

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
                type_params: vec![],
            })],
            span: test_span(),
        };

        let registry = Arc::new(TypeRegistry::new());
        let mut lowerer = doo_thir::ThirLoweringContext::new(registry.as_ref());
        let thir = lowerer.lower_program(&hir);

        let mut type_checker = TypeChecker::new(registry.as_ref());
        assert!(
            type_checker.check_program(&thir).is_ok(),
            "Type checker should pass"
        );

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

        let mut error_flow_checker = ErrorFlowChecker::new(&registry);
        assert!(
            error_flow_checker.check_thir(&thir).is_ok(),
            "Error flow checker should pass"
        );

        let mut exhaustiveness_checker = ExhaustivenessChecker::new(&registry);
        assert!(
            exhaustiveness_checker.check_program(&thir).is_ok(),
            "Exhaustiveness checker should pass"
        );
    }

    #[test]
    fn test_analysis_pipeline_with_variable() {
        use doo_hir::{
            ConstValue, HirExpr, HirExprKind, HirFunction, HirItem, HirProgram, HirStmt,
            HirStmtKind, Ownership,
        };

        let hir = HirProgram {
            items: vec![HirItem::Function(HirFunction {
                name: "main".to_string(),
                params: vec![],
                return_type: None,
                error_type: None,
                body: vec![HirStmt {
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
                }],
                span: test_span(),
                decorators: vec![],
                is_async: false,
                type_params: vec![],
            })],
            span: test_span(),
        };

        let registry = Arc::new(TypeRegistry::new());

        let mut lowerer = doo_thir::ThirLoweringContext::new(registry.as_ref());
        let thir = lowerer.lower_program(&hir);

        let mut type_checker = TypeChecker::new(registry.as_ref());
        assert!(type_checker.check_program(&thir).is_ok());

        let mut ownership_analyzer = OwnershipAnalyzer::new();
        assert!(ownership_analyzer.analyze(&hir).is_ok());

        let mut borrow_checker = BorrowChecker::new();
        assert!(borrow_checker.check(&hir).is_ok());

        let mut hir_mut = hir.clone();
        let mut drop_inserter = DropInserter::new();
        drop_inserter.insert_drops_program(&mut hir_mut);
    }
}
