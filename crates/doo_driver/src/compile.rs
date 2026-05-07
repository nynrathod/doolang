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
            let _ = emitter.emit(&e, &source_map);

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

    merge_imports(&mut program, import_resolution);
    timings.push(("Import resolution", t.elapsed()));

    // Phase 3.5: AST Transformations (AFTER imports so all files are transformed)
    // Transform route DSL (groups, decorators) into explicit route registrations
    let t = Instant::now();
    transform_route_groups(&mut program);
    transform_inline_closures(&mut program);
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
                doo_frontend::ast::Item::Policy(p) => {
                    doo_debug!("DEBUG", "  Policy for {}", p.for_struct)
                }
                doo_frontend::ast::Item::Interface(i) => {
                    doo_debug!("DEBUG", "  Interface: {}", i.name)
                }
                doo_frontend::ast::Item::Static(s) => {
                    doo_debug!("DEBUG", "  Static: {}", s.name)
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
                doo_hir::HirItem::Policy(p) => {
                    doo_debug!("DEBUG", "  HIR Policy for {}", p.for_struct)
                }
                doo_hir::HirItem::Interface(i) => {
                    doo_debug!("DEBUG", "  HIR Interface: {}", i.name)
                }
                doo_hir::HirItem::Static(s) => {
                    doo_debug!("DEBUG", "  HIR Static: {}", s.name)
                }
            }
        }
    }

    // Phase 5: Semantic Analysis (type checking, name resolution, etc.)

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
    // The loader assigns file_ids starting from 1, and SourceMap.add_file()
    // returns sequential indices. We must add them in file_id order so indices match.
    let mut imported = loader.imported_sources().to_vec();
    imported.sort_by_key(|(fid, _, _)| *fid);
    for (expected_id, name, src) in &imported {
        let actual_id = source_map.add_file(name, src);
        debug_assert_eq!(actual_id, *expected_id, "file_id mismatch for {}", name);
    }

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
    // Filter warnings unless --warn flag is passed, and deduplicate by (code, span)
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
                // Deduplicate by (error_code discriminant, span start, span end, file_id)
                let key = (
                    std::mem::discriminant(&e.code),
                    e.span.start,
                    e.span.end,
                    e.span.file_id,
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
    // Pass ownership analysis results to MIR builder so it can emit
    // Move/Copy/Clone/Borrow instructions based on ownership decisions
    let mut mir_builder = if let Some(results) = ownership_results {
        MirBuilder::with_ownership(&type_registry, results)
    } else {
        MirBuilder::new(&type_registry)
    };
    let mir_program = mir_builder.build(&hir);

    // Surface query builder errors (field validation, missing where, etc.).
    let qb_errors = std::mem::take(&mut mir_builder.query_errors);
    if !qb_errors.is_empty() {
        let has_real = qb_errors.iter().any(|e| {
            e.severity == doo_core::errors::codes::ErrorSeverity::Error
                || e.severity == doo_core::errors::codes::ErrorSeverity::Ice
        });
        let mut emitter = DiagnosticEmitter::new(true);
        let _ = emitter.emit_all(&qb_errors, &source_map);
        if has_real {
            return Ok(CompileResult {
                success: false,
                error_count: qb_errors.len(),
                exe_path: None,
            });
        }
    }

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

    timings.push(("MIR build", t.elapsed()));

    // Phase 7: LLVM Codegen
    let t = Instant::now();
    doo_debug!("DEBUG", "Starting LLVM codegen...");
    let context = Context::create();
    let codegen = CodegenBuilder::new(&context);
    let module = codegen.build(&mir_program, "main_module", type_registry.clone());
    doo_debug!("DEBUG", "LLVM codegen complete");
    timings.push(("LLVM codegen", t.elapsed()));

    // Phase 8: Verify module
    let t = Instant::now();
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
    //
    // Four-pronged fix:
    //   1. Re-apply "disable-tail-calls" to ALL functions (attribute-level guard)
    //   2. Strip `tail` marker from ALL call instructions (instruction-level guard)
    //   3. Add "frame-pointer"="all" to prevent frame pointer elimination,
    //      which stabilises stack frames and prevents backend frame reuse
    //   4. Add stack canary (sspstrong) for buffer overflow detection
    {
        use inkwell::values::InstructionOpcode;

        let no_tail = context.create_string_attribute("disable-tail-calls", "true");
        let frame_ptr = context.create_string_attribute("frame-pointer", "all");
        // Stack canary: sspstrong inserts canaries for functions with arrays or address-taken vars
        let ssp = context.create_string_attribute("sspstrong", "");
        let ssp_buf_size = context.create_string_attribute("ssp-buffer-size", "4");
        let mut func = module.get_first_function();
        while let Some(f) = func {
            // (1) Function-level attribute: tell backend "no tail calls"
            f.add_attribute(inkwell::attributes::AttributeLoc::Function, no_tail);
            // (3) Preserve frame pointer in every function
            f.add_attribute(inkwell::attributes::AttributeLoc::Function, frame_ptr);
            // (4) Stack canary for buffer overflow detection
            f.add_attribute(inkwell::attributes::AttributeLoc::Function, ssp);
            f.add_attribute(inkwell::attributes::AttributeLoc::Function, ssp_buf_size);

            // (2) Walk every instruction and clear the `tail` flag on calls.
            //     O3 adds `tail call` markers to many calls (printf, malloc,
            //     strlen, FFI, etc.). Even with disable-tail-calls, belt-and-
            //     suspenders: remove the IR-level hint so the backend can never
            //     see it.
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
    // Must be after optimization so the stubs aren't removed by dead code elimination.
    // FFI C code (libgit2, libssh2) uses POSIX names (close, read) which on
    // Windows map to _close, _read. These stubs enable lld-link auto-import.
    #[cfg(target_os = "windows")]
    add_posix_compat_stubs(&module);
    timings.push(("Optimize + harden", t.elapsed()));

    // Phase 10: Write LLVM IR if requested
    if opts.keep_ll {
        let ll_file = format!("{}.ll", opts.output_name);
        let ir_string = module.print_to_string();
        fs::write(&ll_file, ir_string.to_string())
            .map_err(|e| format!("Failed to write LLVM IR: {}", e))?;
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

/// Resolve the input path to an actual main.doo file.
/// Print phase-by-phase timing information.
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

/// Add POSIX-to-MSVC forwarding stubs to the LLVM module.
/// On Windows, C code compiled for MSVC uses _close/_read (underscore-prefixed)
/// but some FFI libraries (libgit2, libssh2) reference the unprefixed POSIX names
/// via __declspec(dllimport). The dllimport mechanism looks for __imp_close etc.
/// We define these as global pointers pointing to the MSVC-named functions.
///
/// H04: Uses dynamic discovery instead of a hardcoded table. Scans the module
/// for external function references and generates stubs for any POSIX names
/// that have a corresponding MSVC `_`-prefixed equivalent in the UCRT.
#[cfg(target_os = "windows")]
fn add_posix_compat_stubs(module: &inkwell::module::Module) {
    use inkwell::module::Linkage;
    use inkwell::AddressSpace;

    let ctx = module.get_context();
    let ptr_type = ctx.ptr_type(AddressSpace::default());

    // Known POSIX names that have _-prefixed MSVC equivalents in libucrt.
    // This set is authoritative: only these names get stubs (prevents false matches).
    // New FFI crates just need to use standard POSIX calls — no manual updates needed.
    static POSIX_STUBS: &[&str] = &[
        "close", "read", "open", "write", "lseek", "access", "chmod", "mkdir", "rmdir", "unlink",
        "stat", "fstat", "dup", "dup2", "fileno", "isatty", "getcwd", "chdir", "umask", "mktemp",
        "setmode", "strdup", "stricmp", "strnicmp", "pipe", "fdopen", "popen", "pclose", "putenv",
        "getpid", "swab", "tempnam", "tzset", "wopen", "waccess", "wstat",
    ];

    // Scan module for external function references matching known POSIX names.
    // Generate stubs for ALL known POSIX names — external FFI libraries (.lib files)
    // may reference these symbols (e.g., libgit2 uses close/read/write) but those
    // references aren't visible in the LLVM module. The linker ignores unused stubs.
    let generic_fn_ty = ctx.void_type().fn_type(&[], false);

    for posix_name in POSIX_STUBS {
        let imp_name = format!("__imp_{}", posix_name);

        // Skip if stub already exists
        if module.get_global(&imp_name).is_some() {
            continue;
        }

        // Build the MSVC-prefixed name
        let msvc_name = format!("_{}", posix_name);

        // Declare the MSVC-named function (if not already present)
        let msvc_fn = module.get_function(&msvc_name).unwrap_or_else(|| {
            module.add_function(&msvc_name, generic_fn_ty, Some(Linkage::External))
        });

        // Create __imp_<posix_name> = constant pointer to _<posix_name>
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

    // Prefer MSVC's native link.exe over embedded lld-link.
    //
    // MSVC's linker correctly handles .pdata/.xdata associative COMDAT
    // deduplication when /FORCE:MULTIPLE is used. The embedded lld-link
    // (LLVM 18) has a known bug where it does not properly discard
    // .pdata entries for COMDATs removed during duplicate resolution,
    // leaving stale exception table entries that corrupt SEH unwinding
    // and cause DEP crashes at the module base address.
    //
    // This is especially critical for FFI crates containing C code
    // (libgit2, zlib, etc.) whose functions have complex .pdata entries.
    let linker = sdk_paths
        .as_ref()
        .and_then(|p| p.msvc_lib.as_ref())
        .and_then(|lib| find_msvc_linker(lib))
        .map(Ok)
        .unwrap_or_else(|| extract_embedded_linker())?;

    let mut cmd = Command::new(&linker);

    // Generate MAP file for crash debugging (maps addresses to function names)
    let map_path = exe_path.with_extension("map");
    cmd.arg(format!("/MAP:{}", map_path.display()));

    cmd.arg(format!("/OUT:{}", exe_path.display()))
        .arg(obj_file)
        .arg("/SUBSYSTEM:CONSOLE")
        // Use mainCRTStartup (from libcmt.lib) as the entry point.
        // This initializes the C runtime (TLS, heap, static ctors) before
        // calling main(). Without this, Tokio/async runtimes fail silently
        // because thread-local storage isn't set up.
        .arg("/ENTRY:mainCRTStartup");

    // When linking multiple Rust static libraries, each contains its own copy of
    // the Rust runtime symbols (__rust_alloc, compiler-builtins, etc.).
    // Allow duplicates so the linker uses the first definition and ignores the rest.
    //
    // IMPORTANT: /FORCE:MULTIPLE disables /OPT:REF by default (per MSVC docs).
    // Do NOT re-enable /OPT:REF with /FORCE:MULTIPLE — it can strip sections
    // that surviving duplicate copies still reference, corrupting .pdata/.xdata
    // exception tables and causing DEP crashes at the module base address.
    let use_force_multiple = ffi_libs.len() > 1;
    if use_force_multiple {
        cmd.arg("/FORCE:MULTIPLE");
        // CRITICAL: Explicitly disable ICF and REF to prevent .pdata corruption.
        //
        // /FORCE:MULTIPLE only implies /OPT:NOREF (per MSVC docs), but ICF
        // (Identical COMDAT Folding) REMAINS ENABLED by default for non-debug
        // builds. ICF folds COMDATs with identical machine code but potentially
        // different .pdata/.xdata exception table entries, corrupting SEH
        // unwind data and causing DEP crashes at the module base address.
        //
        // This is the root cause of the DEP crash in FFI C code (libgit2, etc.):
        //  1. Multiple Rust static libraries define identical generic functions
        //  2. ICF folds them into one, discarding some .pdata entries
        //  3. An exception in the surviving function finds a stale .pdata entry
        //  4. The SEH unwinder jumps to a corrupted address (module base) → DEP
        //
        // /OPT:NOICF prevents content-based folding, keeping all .pdata entries valid.
        // /OPT:NOREF prevents section garbage collection, keeping all code referenced.
        // The size increase is negligible (~100KB) vs. the risk of silent DEP crashes.
        cmd.arg("/OPT:NOICF,NOREF");
        // Also produce a MAP file for crash diagnostics
        let map_path = exe_path.with_extension("map");
        cmd.arg(format!("/MAP:{}", map_path.display()));
    }

    // Suppress dynamic CRT defaultlib to avoid MSVCRT.lib(utility.obj) conflicts.
    // All CRT symbols come from the static set instead.
    cmd.arg("/NODEFAULTLIB:MSVCRT")
        .arg("/NODEFAULTLIB:MSVCRTD")
        .arg("/NODEFAULTLIB:libcmtd");

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
        // STATIC CRT strategy for single-binary FFI linking:
        //
        //   libcmt.lib        → CRT startup, _tls_used, atexit
        //   libvcruntime.lib  → __vcrt_initialize, __chkstk, __security_cookie
        //   libucrt.lib       → All C runtime functions (getenv, malloc, etc.)
        //
        // /WHOLEARCHIVE:libucrt.lib forces ALL UCRT objects to load,
        // enabling lld-link's __imp_X → X auto-import resolution for
        // FFI C code compiled with /MD (__declspec(dllimport)).
        // This produces harmless LNK4217 warnings.
        cmd.arg("/WHOLEARCHIVE:libucrt.lib")
            .arg("libvcruntime.lib")
            .arg("libcmt.lib")
            .arg("legacy_stdio_definitions.lib");
    }

    // Windows system libraries required by FFI crates (added once, not per-lib)
    cmd.arg("ws2_32.lib")
        .arg("userenv.lib")
        .arg("bcrypt.lib")
        .arg("kernel32.lib")
        .arg("advapi32.lib")
        .arg("ntdll.lib")
        .arg("winhttp.lib")
        .arg("ole32.lib")
        .arg("rpcrt4.lib")
        .arg("secur32.lib")
        .arg("crypt32.lib")
        .arg("user32.lib")
        .arg("shell32.lib");

    // Link FFI libraries in deterministic alphabetical order.
    // With /FORCE:MULTIPLE, the FIRST definition of each duplicate symbol wins.
    // Alphabetical order ensures doo_ffi_core's runtime symbols win deduplication,
    // which is correct since core provides the canonical runtime implementation.
    let mut lib_entries: Vec<(&String, PathBuf, PathBuf)> = Vec::new();
    for lib in ffi_libs.iter() {
        if let Some((lib_dir, lib_file)) = find_ffi_library(lib, search_paths) {
            lib_entries.push((lib, lib_dir, lib_file));
        } else {
            return Err(format!(
                "FFI library '{}.dll.lib' not found.\n\
                Build it first with: cargo build --release\n\
                Searched in: {:?}",
                lib, search_paths
            ));
        }
    }

    // Alphabetical by library name for deterministic, reproducible builds.
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
            // Show linker warnings even on success — critical for diagnosing
            // symbol conflicts with /FORCE:MULTIPLE linking.
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

    // Dynamically scan all Visual Studio installation years and editions.
    // This avoids hardcoding year/edition lists (2022, 2019, etc.) and
    // automatically supports future VS releases (2025, 2027, etc.).
    // Picks the newest year + newest MSVC toolchain version.
    let mut best: Option<(String, String)> = None; // (year_sort_key, full_path)

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

/// Find MSVC's native link.exe from the Visual Studio installation.
///
/// MSVC's link.exe handles /FORCE:MULTIPLE + .pdata/.xdata correctly,
/// unlike lld-link (LLVM 18) which has known bugs with associative
/// COMDAT deduplication that corrupt SEH exception tables and cause
/// DEP crashes at the module base address.
///
/// The path is derived from the MSVC lib directory:
///   .../VC/Tools/MSVC/{version}/lib/x64  →
///   .../VC/Tools/MSVC/{version}/bin/Hostx64/x64/link.exe
#[cfg(target_os = "windows")]
fn find_msvc_linker(msvc_lib: &str) -> Option<PathBuf> {
    let path = Path::new(msvc_lib);
    // Go up from lib/x64 to the MSVC version root
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
        // Export symbols to dynamic table when multiple FFI libs are linked —
        // cross-crate FFI uses dlsym for runtime symbol resolution
        // (e.g., auth → http register, http → db bridge).
        // Programs with a single FFI lib skip this overhead.
        if ffi_libs.len() > 1 {
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
        cmd.arg("-framework").arg("SystemConfiguration");
        // When linking multiple Rust static libraries, each contains its own copy of
        // the Rust runtime symbols (__rust_alloc, compiler-builtins, etc.).
        // Allow duplicates so the linker uses the first definition and ignores the rest.
        if ffi_libs.len() > 1 {
            cmd.arg("-Wl,-multiply_defined,suppress");
            // Export dynamic symbols so dlsym(RTLD_DEFAULT) can find cross-crate
            // bridge symbols at runtime (macOS equivalent of -rdynamic on Linux).
            cmd.arg("-Wl,-export_dynamic");
        }
    }

    // Link FFI libraries in dependency order
    // Names must match the normalized names in ffi_libs (doo_ffi_* format)
    let lib_order = [
        "doo_ffi_http",
        "doo_ffi_auth",
        "doo_ffi_db",
        "doo_ffi_git",
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
                        cmd.arg(lib_file.to_string_lossy().as_ref());
                        cmd.arg("-Wl,--no-whole-archive");
                    } else {
                        cmd.arg(lib_file.to_string_lossy().as_ref());
                    }
                }
                #[cfg(not(target_os = "linux"))]
                {
                    cmd.arg(lib_file.to_string_lossy().as_ref());
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

    // System library dependencies for FFI crates.
    // These must come AFTER the static archives (linker resolves left-to-right).
    // Discovered dynamically from the ffi_libs set — no hardcoded assumptions.
    // When using the bundle, all system deps are needed since it contains all crates.
    if ffi_libs.contains("doo_ffi_git") {
        // libgit2 depends on zlib for compression. OpenSSL is vendored (statically
        // compiled into the git2 static archive via `vendored-openssl` feature).
        cmd.arg("-lz");
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
            timings: false,
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
                type_params: vec![],
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
                type_params: vec![],
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
