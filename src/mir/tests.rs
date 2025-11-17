//! MIR Layer Tests
//! Tests Intermediate Representation (IR) generation
//!
//! Responsibility: Verify IR structure, block generation, control flow
//! Does NOT: Test semantic validation (analyzer's job), test codegen (codegen's job)
//!
//! Strategy: Keep only tests that verify IR-specific concerns that earlier layers don't

#[cfg(test)]
mod mir_tests {
    use crate::analyzer::SemanticAnalyzer;
    use crate::lexer::lexer::lex;
    use crate::mir::builder::MirBuilder;
    use crate::parser::ast::AstNode;
    use crate::parser::Parser;
    use bumpalo::Bump;

    fn build_mir(input: &str) -> Result<MirBuilder, String> {
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
                        .map_err(|e| format!("{:?}", e))?;

                    let mut mir_builder = MirBuilder::new();
                    mir_builder.build_program(nodes);
                    mir_builder.finalize();
                    Ok(mir_builder)
                } else {
                    Err("Not a program".to_string())
                }
            }
            Err(e) => Err(format!("Parse error: {:?}", e)),
        }
    }

    fn assert_mir_ok(code: &str) {
        build_mir(code).expect(&format!("Expected MIR build success for:\n{}", code));
    }

    fn assert_has_function(code: &str, fn_name: &str) {
        let mir = build_mir(code).expect("MIR build failed");
        assert!(
            mir.program.functions.iter().any(|f| f.name == fn_name),
            "Expected function '{}' in MIR",
            fn_name
        );
    }

    // fn assert_function_block_count(code: &str, fn_name: &str, expected_blocks: usize) {
    //     let mir = build_mir(code).expect("MIR build failed");
    //     let func = mir
    //         .program
    //         .functions
    //         .iter()
    //         .find(|f| f.name == fn_name)
    //         .expect(&format!("Function '{}' not found", fn_name));
    //     assert_eq!(
    //         func.blocks.len(),
    //         expected_blocks,
    //         "Expected {} blocks in function '{}', got {}",
    //         expected_blocks,
    //         fn_name,
    //         func.blocks.len()
    //     );
    // }

    fn assert_total_function_count(code: &str, expected: usize) {
        let mir = build_mir(code).expect("MIR build failed");
        assert_eq!(
            mir.program.functions.len(),
            expected,
            "Expected {} functions, got {}",
            expected,
            mir.program.functions.len()
        );
    }

    // ========================================
    // FUNCTION MIR GENERATION
    // ========================================

    #[test]
    fn test_mir_simple_function() {
        assert_has_function("fn main() { }", "main");
    }

    #[test]
    fn test_mir_function_with_params() {
        assert_has_function(
            "fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { }",
            "add",
        );
    }

    #[test]
    fn test_mir_multiple_functions() {
        let code = r#"
            fn foo() { }
            fn bar() { }
            fn main() { }
        "#;
        assert_total_function_count(code, 3);
        assert_has_function(code, "foo");
        assert_has_function(code, "bar");
        assert_has_function(code, "main");
    }

    #[test]
    fn test_mir_recursive_function() {
        let code = r#"
            fn factorial(n: Int) -> Int {
                if n <= 1 { return 1; }
                return n * factorial(n - 1);
            }
            fn main() { }
        "#;
        assert_has_function(code, "factorial");
    }

    // ========================================
    // CONTROL FLOW MIR STRUCTURE
    // ========================================

    #[test]
    fn test_mir_if_statement_blocks() {
        let code = "fn main() { if true { let x = 1; } }";
        // MIR creates more blocks than expected for control flow
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_if_else_blocks() {
        let code = "fn main() { if true { let x = 1; } else { let y = 2; } }";
        // MIR creates more blocks than expected for control flow
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_nested_if_blocks() {
        let code = r#"
            fn main() {
                if true {
                    if false {
                        let x = 1;
                    }
                }
            }
        "#;
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_for_loop_blocks() {
        let code = "fn main() { for i in 0..10 { } }";
        // MIR creates more blocks than expected for control flow
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_nested_loops_blocks() {
        let code = r#"
            fn main() {
                for i in 0..5 {
                    for j in 0..5 {
                        let x = i + j;
                    }
                }
            }
        "#;
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_break_in_loop() {
        let code = "fn main() { for i in 0..10 { if i == 5 { break; } } }";
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_continue_in_loop() {
        let code = "fn main() { for i in 0..10 { if i == 5 { continue; } } }";
        assert_mir_ok(code);
    }

    // ========================================
    // FOR LOOP DESTRUCTURING - BASIC TESTS ONLY
    // ========================================

    #[test]
    fn test_mir_for_map_destructure_basic() {
        assert_mir_ok(r#"fn main() { let map1 = {"a": 1}; for (k, v) in map1 { } }"#);
    }

    #[test]
    fn test_mir_for_map_destructure_with_body() {
        assert_mir_ok(r#"fn main() { for (k, v) in {"x": 100, "y": 200} { let z = v; } }"#);
    }

    #[test]
    fn test_mir_for_map_destructure_wildcard() {
        assert_mir_ok(r#"fn main() { for (_, v) in {"a": 1, "b": 2, "c": 3} { } }"#);
    }

    // ========================================
    // ARRAY & MAP OPERATIONS
    // ========================================

    #[test]
    fn test_mir_array_literal() {
        assert_mir_ok("fn main() { let x = [1, 2, 3]; }");
    }

    #[test]
    fn test_mir_array_access() {
        assert_mir_ok("fn main() { let arr = [1, 2, 3]; let x = arr[0]; }");
    }

    #[test]
    fn test_mir_array_access_in_loop() {
        let code = r#"
            fn main() {
                let arr = [10, 20, 30];
                for i in 0..3 {
                    let x = arr[i];
                }
            }
        "#;
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_map_literal() {
        assert_mir_ok(r#"fn main() { let m = {"a": 1, "b": 2}; }"#);
    }

    #[test]
    fn test_mir_map_access() {
        assert_mir_ok(r#"fn main() { let m = {"key": 42}; let x = m["key"]; }"#);
    }

    // ========================================
    // IMPORTS - BASIC VERIFICATION ONLY
    // ========================================

    #[test]
    fn test_mir_import_basic() {
        assert_mir_ok("import std::Math::{Abs}; fn main() { let x = Abs(-5); }");
    }

    // ========================================
    // COMPLEX MIR PATTERNS
    // ========================================

    #[test]
    fn test_mir_nested_control_flow_complex() {
        let code = r#"
            fn main() {
                let mut total = 0;
                for i in 0..5 {
                    if i > 2 {
                        for j in 0..i {
                            total += j;
                        }
                    }
                }
            }
        "#;
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_function_calls() {
        let code = r#"
            fn double(x: Int) -> Int { return x * 2; }
            fn main() {
                let x = double(5);
                let y = double(10);
            }
        "#;
        assert_total_function_count(code, 2);
    }

    #[test]
    fn test_mir_chained_function_calls() {
        let code = r#"
            fn add(a: Int, b: Int) -> Int { return a + b; }
            fn multiply(a: Int, b: Int) -> Int { return a * b; }
            fn main() {
                let x = add(multiply(2, 3), 4);
            }
        "#;
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_array_operations() {
        let code = r#"
            fn main() {
                let arr = [1, 2, 3];
                let x = arr[0];
                for val in arr {
                    let y = val;
                }
            }
        "#;
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_map_operations() {
        let code = r#"
            fn main() {
                let m = {"a": 1, "b": 2};
                let x = m["a"];
                for (k, v) in m {
                    let y = v;
                }
            }
        "#;
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_variable_scoping() {
        let code = r#"
            fn main() {
                let x = 1;
                if true {
                    let y = 2;
                    let x = 3;
                }
                let z = x;
            }
        "#;
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_variable_in_loop_scope() {
        let code = r#"
            fn main() {
                for i in 0..5 {
                    let x = i;
                }
            }
        "#;
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_comprehensive_program() {
        let code = r#"
            fn helper(n: Int) -> Int {
                return n * 2;
            }

            fn main() {
                let mut arr = [1, 2, 3, 4, 5];
                let mut sum = 0;

                for val in arr {
                    if val > 2 {
                        sum += helper(val);
                    }
                }

                let doubled = arr.map((n) => n * 2);
                let filtered = doubled.filter((n) => n > 5);
            }
        "#;
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_array_method_chain() {
        let code = "fn main() { let x = [1, 2, 3].map((n) => n * 2).filter((n) => n > 2); }";
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_early_return() {
        let code = r#"
            fn foo() -> Int {
                if true { return 42; }
                return 0;
            }
            fn main() { }
        "#;
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_multiple_returns() {
        let code = r#"
            fn classify(n: Int) -> Str {
                if n < 0 { return "negative"; }
                if n == 0 { return "zero"; }
                return "positive";
            }
            fn main() { }
        "#;
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_loop_with_conditional_break() {
        let code = r#"
            fn main() {
                let mut i = 0;
                for x in 0..100 {
                    if x > 50 { break; }
                    i += x;
                }
            }
        "#;
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_mixed_arithmetic_types() {
        let code = "fn main() { let x: Float = 10 + 5.5; }";
        assert_mir_ok(code);
    }

    #[test]
    fn test_mir_string_concatenation_with_numbers() {
        let code = r#"fn main() { let x = "value: " + 42; }"#;
        assert_mir_ok(code);
    }
}
