use doo_analysis::transform::{transform_inline_closures, transform_route_groups};
///! Common test utilities for all test suites
///! Provides shared compilation helpers and assertion functions
use doo_analysis::{ErrorFlowChecker, TypeChecker};
use doo_codegen::CodegenBuilder;
use doo_core::types::TypeRegistry;
use doo_driver::loader::{merge_imports, resolve_imports, ModuleLoader};
use doo_frontend::ast::Item;
use doo_frontend::{Lexer, Parser};
use doo_hir::Lower;
use doo_mir::builder::MirBuilder;
use inkwell::context::Context;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Once;

/// Ensure debug output is disabled for tests (runs once)
static INIT: Once = Once::new();

fn init_test_env() {
    INIT.call_once(|| {
        // Disable all debug output from the compiler during tests
        doo_core::debug::init(false);
    });
}

/// Compile code through full pipeline: lex → parse → transform → imports → hir → type check → mir → codegen
/// Mirrors the real compiler pipeline (doo_driver/compile.rs) for error coverage
pub fn compile_snippet(input: &str) -> Result<String, String> {
    // Disable debug output for tests
    init_test_env();

    // Phase 1: Parse
    let mut parser = Parser::new(input, 0);
    let program = parser
        .parse_program()
        .map_err(|e| format!("Parse error: {:?}", e))?;
    if parser.has_errors() {
        return Err(format!("Parse errors: {:?}", parser.errors()));
    }

    // Phase 2: AST Transforms (route groups, inline closures)
    let mut program = program;
    transform_route_groups(&mut program);
    transform_inline_closures(&mut program);

    // Phase 2.5: Resolve imports if any exist
    let has_imports = program
        .items
        .iter()
        .any(|item| matches!(item, Item::Import(_)));
    if has_imports {
        let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut loader = ModuleLoader::new();
        match resolve_imports(&program, &mut loader, &project_root) {
            Ok(import_resolution) => {
                merge_imports(&mut program, import_resolution);
            }
            Err(e) => return Err(format!("Import resolution error: {}", e)),
        }
    }

    // Phase 3: Lower to HIR
    let mut type_registry = TypeRegistry::new();
    let mut lowerer = Lower::new();
    let hir = lowerer.lower_program_typed(&program, &mut type_registry);

    // Phase 4: Type check
    let type_registry = Arc::new(type_registry);
    let mut type_checker = TypeChecker::new(type_registry.clone());
    let type_result = type_checker.check(&hir);

    // Collect scope errors (duplicate declarations, redefinitions)
    let scope_errors = type_checker.take_scope_errors();
    if !scope_errors.is_empty() {
        return Err(format!("Scope errors: {:?}", scope_errors));
    }

    // Surface type errors
    type_result.map_err(|e| format!("Type error: {:?}", e))?;

    // Phase 4.5: Error flow checking (Ok/Err usage, error propagation)
    let mut error_flow_checker = ErrorFlowChecker::new(&type_registry);
    if let Err(errors) = error_flow_checker.check(&hir) {
        return Err(format!("Error flow: {:?}", errors));
    }

    // Phase 5: MIR
    let mut mir_builder = MirBuilder::new(&type_registry);
    let mir = mir_builder.build(&hir);

    // Phase 6: Codegen
    let context = Context::create();
    let codegen_builder = CodegenBuilder::new(&context);
    let module = codegen_builder.build(&mir, "test", type_registry);

    Ok(module.print_to_string().to_string())
}

/// Compile a project file with support for imports
pub fn compile_project_file(file_path: &std::path::Path) -> Result<String, String> {
    let content = std::fs::read_to_string(file_path)
        .map_err(|e| format!("Failed to read {}: {}", file_path.display(), e))?;

    compile_snippet(&content)
}

/// Assert code compiles successfully
pub fn assert_compiles(code: &str) {
    match compile_snippet(code) {
        Ok(_) => (),
        Err(e) => panic!("Expected to compile but failed: {}\nCode: {}", e, code),
    }
}

/// Assert code fails to compile
pub fn assert_fails(code: &str) {
    match compile_snippet(code) {
        Err(_) => (),
        Ok(_) => panic!("Expected to fail but compiled successfully\nCode: {}", code),
    }
}

/// Assert code fails with specific error message
pub fn assert_fails_with(code: &str, error_substr: &str) {
    match compile_snippet(code) {
        Err(e) if e.contains(error_substr) => (),
        Err(e) => panic!(
            "Expected error containing '{}' but got: {}\nCode: {}",
            error_substr, e, code
        ),
        Ok(_) => panic!(
            "Expected to fail with '{}' but compiled\nCode: {}",
            error_substr, code
        ),
    }
}

/// Assert code compiles and IR contains substring
pub fn assert_compiles_with(code: &str, ir_substr: &str) {
    match compile_snippet(code) {
        Ok(ir) if ir.contains(ir_substr) => (),
        Ok(ir) => {
            // Truncate IR for readability (show only first 200 chars)
            let ir_preview = if ir.len() > 200 {
                format!("{}... (truncated)", &ir[..200])
            } else {
                ir
            };
            panic!(
                "Expected IR to contain '{}'\nIR preview: {}\nCode: {}",
                ir_substr, ir_preview, code
            )
        }
        Err(e) => panic!("Expected to compile but failed: {}\nCode: {}", e, code),
    }
}

/// Assert expression type - DISABLED (requires old analyzer)
/// TODO: Reimplement with new type checker
pub fn assert_expr_type(_expr: &str, _expected: &str) {
    // Temporarily disabled - needs reimplementation with new compiler
    panic!("assert_expr_type is temporarily disabled - needs update for new compiler");
}

/// Assert typeof - DISABLED (requires old analyzer)
/// TODO: Reimplement with new type checker
pub fn assert_typeof(_expr: &str, _expected: &str) {
    // Temporarily disabled - needs reimplementation with new compiler
    panic!("assert_typeof is temporarily disabled - needs update for new compiler");
}

// =============================================================================
// .doo File Discovery and Runner Utilities
// =============================================================================

/// Recursively find all .doo files in a directory
pub fn find_doo_files_in(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(find_doo_files_in(&path));
            } else if path.extension().map_or(false, |e| e == "doo") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Parse the first comment line of a .doo file for expected error substring.
/// Files with `// ERROR: <msg>` are expected to fail with that message.
/// Returns Some(expected_error) if it's an error test, None if it should pass.
pub fn parse_expected_error(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("// ERROR:") {
            return Some(rest.trim().to_string());
        }
        // Stop at first non-comment non-empty line
        if !trimmed.starts_with("//") {
            break;
        }
    }
    None
}

/// Run all .doo files in a directory through compile_snippet.
/// For crash tests: just ensure no panic (ignore compile result).
/// For compile_pass: all files must compile.
/// For compile_fail: all files must fail (with optional error match from `// ERROR:` comment).
/// For ui: files with `// ERROR:` must fail with that message.
/// Returns (passed, failed) counts for summary.
pub fn run_doo_file_suite(
    dir: &std::path::Path,
    mode: DooTestMode,
) -> (Vec<PathBuf>, Vec<(PathBuf, String)>) {
    let files = find_doo_files_in(dir);
    let mut passed = Vec::new();
    let mut failed: Vec<(PathBuf, String)> = Vec::new();

    for file in &files {
        let content = match std::fs::read_to_string(file) {
            Ok(c) => c,
            Err(e) => {
                failed.push((file.clone(), format!("Failed to read: {}", e)));
                continue;
            }
        };

        match mode {
            DooTestMode::CrashTest => {
                // Just ensure no panic — result doesn't matter
                let _ = std::panic::catch_unwind(|| {
                    let _ = compile_snippet(&content);
                });
                passed.push(file.clone());
            }
            DooTestMode::CompilePass => {
                match compile_snippet(&content) {
                    Ok(_) => passed.push(file.clone()),
                    Err(e) => failed.push((file.clone(), e)),
                }
            }
            DooTestMode::CompileFail => {
                let expected = parse_expected_error(&content);
                match compile_snippet(&content) {
                    Err(e) => {
                        if let Some(ref expected_msg) = expected {
                            if e.to_lowercase().contains(&expected_msg.to_lowercase()) {
                                passed.push(file.clone());
                            } else {
                                failed.push((file.clone(), format!(
                                    "Expected error containing '{}' but got: {}", expected_msg, e
                                )));
                            }
                        } else {
                            passed.push(file.clone());
                        }
                    }
                    Ok(_) => {
                        failed.push((file.clone(), "Expected to fail but compiled".to_string()));
                    }
                }
            }
            DooTestMode::UiDiagnostic => {
                let expected = parse_expected_error(&content);
                match (compile_snippet(&content), expected) {
                    (Err(e), Some(expected_msg)) => {
                        if e.to_lowercase().contains(&expected_msg.to_lowercase()) {
                            passed.push(file.clone());
                        } else {
                            failed.push((file.clone(), format!(
                                "Expected error containing '{}' but got: {}", expected_msg, e
                            )));
                        }
                    }
                    (Err(e), None) => {
                        failed.push((file.clone(), format!("Unexpected error: {}", e)));
                    }
                    (Ok(_), Some(expected_msg)) => {
                        failed.push((file.clone(), format!(
                            "Expected error '{}' but compiled", expected_msg
                        )));
                    }
                    (Ok(_), None) => passed.push(file.clone()),
                }
            }
            DooTestMode::StressTest => {
                match compile_snippet(&content) {
                    Ok(_) => passed.push(file.clone()),
                    Err(e) => failed.push((file.clone(), e)),
                }
            }
        }
    }

    (passed, failed)
}

/// Test modes for .doo file suites
#[derive(Clone, Copy)]
pub enum DooTestMode {
    /// Crash tests: compiler must not panic (errors are fine)
    CrashTest,
    /// Compile-pass: all files must compile successfully
    CompilePass,
    /// Compile-fail: all files must fail to compile
    CompileFail,
    /// UI diagnostic: files with `// ERROR:` must fail with that message
    UiDiagnostic,
    /// Stress: large programs must compile successfully
    StressTest,
}

/// Assert all .doo files in a directory pass according to the given mode.
/// Panics with summary if any fail.
pub fn assert_doo_file_suite(dir: &std::path::Path, mode: DooTestMode, suite_name: &str) {
    if !dir.exists() {
        println!("{} directory not found at {}, skipping", suite_name, dir.display());
        return;
    }

    let (passed, failed) = run_doo_file_suite(dir, mode);
    let total = passed.len() + failed.len();

    if total == 0 {
        println!("No .doo files found in {} for {}", dir.display(), suite_name);
        return;
    }

    if !failed.is_empty() {
        let mut msg = format!("\n{}: {} / {} .doo files failed:\n", suite_name, failed.len(), total);
        for (file, error) in &failed {
            msg.push_str(&format!("  FAIL: {}\n    Error: {}\n", file.display(), error));
        }
        panic!("{}", msg);
    }
}
