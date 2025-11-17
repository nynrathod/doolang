///! Common test utilities for all test suites
///! Provides shared compilation helpers and assertion functions
use bumpalo::Bump;
use doo::analyzer::SemanticAnalyzer;
use doo::codegen::core::CodeGen;
use doo::lexer::lexer::lex;
use doo::mir::builder::MirBuilder;
use doo::parser::ast::AstNode;
use doo::parser::Parser;
use inkwell::context::Context;

/// Compile code through full pipeline: lex → parse → analyze → mir → codegen
pub fn compile_snippet(input: &str) -> Result<String, String> {
    let arena = Bump::new();
    let tokens = lex(input, &arena);
    let mut parser = Parser::new(&tokens);
    let result = parser.parse_program();

    match result {
        Ok(mut ast) => {
            let mut analyzer = SemanticAnalyzer::new(None);
            if let AstNode::Program(ref mut nodes) = ast {
                analyzer
                    .analyze_program(nodes)
                    .map_err(|e| format!("{}", e))?;

                let mut mir_builder = MirBuilder::new();

                // Build imported functions first (from analyzer)
                for imported_fn in &analyzer.imported_functions {
                    mir_builder.build_program(&[imported_fn.clone()]);
                }

                // Then build the main program
                mir_builder.build_program(nodes);
                mir_builder.finalize();

                let context = Context::create();
                let mut codegen = CodeGen::new("test", &context);
                codegen.generate_program(&mir_builder.program);

                Ok(codegen.module.print_to_string().to_string())
            } else {
                Err("Not a program".to_string())
            }
        }
        Err(e) => Err(format!("Parse error: {}", e)),
    }
}

/// Lex only - returns token count (can't return tokens due to lifetime issues)
pub fn lex_snippet_count(input: &str) -> usize {
    let arena = Bump::new();
    let tokens = lex(input, &arena);
    tokens.len()
}

/// Parse only - returns AST
pub fn parse_snippet(input: &str) -> Result<AstNode, String> {
    let arena = Bump::new();
    let tokens = lex(input, &arena);
    let mut parser = Parser::new(&tokens);
    parser.parse_program().map_err(|e| format!("{:?}", e))
}

/// Analyze only - returns Ok(()) or error
pub fn analyze_snippet(input: &str) -> Result<(), String> {
    let arena = Bump::new();
    let tokens = lex(input, &arena);
    let mut parser = Parser::new(&tokens);
    let result = parser.parse_program();

    match result {
        Ok(mut ast) => {
            let mut analyzer = SemanticAnalyzer::new(None);
            if let AstNode::Program(ref mut nodes) = ast {
                analyzer
                    .analyze_program(nodes)
                    .map_err(|e| format!("{:?}", e))
            } else {
                Err("Not a program".to_string())
            }
        }
        Err(e) => Err(format!("Parse error: {:?}", e)),
    }
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
        Ok(ir) => panic!(
            "Expected IR to contain '{}' but got:\n{}\nCode: {}",
            ir_substr, ir, code
        ),
        Err(e) => panic!("Expected to compile but failed: {}\nCode: {}", e, code),
    }
}

/// Assert that an expression compiles and has the expected inferred type.
/// Uses the existing pipeline (parse_snippet + analyze) instead of re-running lex/parse/analyze manually.
///
/// NOTE:
/// The test helper previously wrapped the expression inside `fn main() { let tmpVar = ... }`,
/// which placed `tmpVar` into a function-local scope. After analysis the analyzer restores
/// its symbol table and block-local variables are removed, so looking up `tmpVar` failed.
///
/// To avoid that, we place `tmpVar` as a top-level `let` (module-level) so it remains in the
/// analyzer's `symbol_table` after analysis. The analyzer normally requires a `main()` function
/// to exist for the "main module", so we disable that requirement for this helper by marking
/// the analyzer as not the main module (`is_main_module = false`) before running analysis.
pub fn assert_expr_type(expr: &str, expected: &str) {
    use bumpalo::Bump;
    use doo::analyzer::SemanticAnalyzer;
    use doo::lexer::lexer::lex;
    use doo::parser::{ast::AstNode, Parser};

    // Place tmpVar at top-level so it isn't removed during function/block scope cleanup.
    let code = format!("let tmpVar = {};", expr);

    // ─────────────────────────────────────
    // 1. Parse using existing helper
    // ─────────────────────────────────────
    let arena = Bump::new();
    let tokens = lex(&code, &arena);
    let mut parser = Parser::new(&tokens);

    let mut ast = parser
        .parse_program()
        .unwrap_or_else(|e| panic!("[COMPILATION FAILED] Parse error: {:?}\nCode: {}", e, code));

    // ─────────────────────────────────────
    // 2. Analyze using existing analyzer logic
    // ─────────────────────────────────────
    let mut analyzer = SemanticAnalyzer::new(None);

    // Disable main() requirement for this helper so the analyzer won't error when main() is absent.
    analyzer.is_main_module = false;

    let nodes = match &mut ast {
        AstNode::Program(n) => n,
        _ => panic!("Not a program"),
    };

    if let Err(e) = analyzer.analyze_program(nodes) {
        panic!(
            "[COMPILATION FAILED] Semantic error: {:?}\nCode: {}",
            e, code
        );
    }

    // ─────────────────────────────────────
    // 3. Extract tmpVar type (now present in top-level symbol table)
    // ─────────────────────────────────────
    let sym = analyzer
        .lookup_variable("tmpVar")
        .unwrap_or_else(|| panic!("[TYPE CHECK FAILED] tmpVar not found"));

    let actual_type = sym.ty.to_string();

    if actual_type != expected {
        panic!(
            "[TYPE CHECK FAILED]\nExpr: {}\nExpected: {}\nFound: {}\n",
            expr, expected, actual_type
        );
    }
}

pub fn assert_typeof(expr: &str, expected: &str) {
    // Verify typeOf(expr) RETURNS Str
    assert_expr_type(&format!("typeOf({})", expr), "Str");
}
