//! Codegen Unit Tests

#[cfg(test)]
mod codegen_tests {
    use crate::analyzer::SemanticAnalyzer;
    use crate::codegen::core::CodeGen;
    use crate::lexer::lexer::lex;
    use crate::mir::builder::MirBuilder;
    use crate::parser::ast::AstNode;
    use crate::parser::Parser;
    use bumpalo::Bump;
    use inkwell::context::Context;

    fn compile_code(input: &str) -> Result<String, String> {
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

                    // Merge imported functions collected by the analyzer into the program AST
                    // so MIR and CodeGen see declarations coming from imported modules
                    // (for example, std::Math functions). This mirrors the behavior in the
                    // full compiler which prepends imported functions before building MIR.
                    let mut all_nodes = analyzer.imported_functions.clone();
                    all_nodes.extend(nodes.clone());

                    let mut mir_builder = MirBuilder::new();
                    mir_builder.build_program(&all_nodes);
                    mir_builder.finalize();

                    let context = Context::create();
                    let mut codegen = CodeGen::new("test", &context);
                    // Propagate function alias mappings collected by the analyzer so
                    // CodeGen can resolve aliased imported functions (e.g. `Abs as AbsValue`)
                    codegen.function_aliases = analyzer.function_aliases.clone();
                    codegen.generate_program(&mir_builder.program);

                    Ok(codegen.module.print_to_string().to_string())
                } else {
                    Err("Not a program".to_string())
                }
            }
            Err(e) => Err(format!("Parse error: {:?}", e)),
        }
    }

    fn assert_codegen_ok(code: &str) {
        compile_code(code).expect(&format!("Expected codegen success for:\n{}", code));
    }

    fn assert_ir_contains(code: &str, pattern: &str) {
        let ir = compile_code(code).expect("Codegen failed");
        assert!(
            ir.contains(pattern),
            "Expected IR to contain '{}', but got:\n{}",
            pattern,
            ir
        );
    }

    fn assert_codegen_err(code: &str) {
        let result = compile_code(code);
        assert!(
            result.is_err(),
            "Expected runtime error for code:\n{}",
            code
        );
    }

    // fn assert_ir_not_contains(code: &str, pattern: &str) {
    //     let ir = compile_code(code).expect("Codegen failed");
    //     assert!(
    //         !ir.contains(pattern),
    //         "Expected IR NOT to contain '{}', but it was found in:\n{}",
    //         pattern,
    //         ir
    //     );
    // }

    fn assert_ir_contains_any(code: &str, patterns: &[&str]) {
        let ir = compile_code(code).expect("Codegen failed");
        let found = patterns.iter().any(|p| ir.contains(p));
        assert!(
            found,
            "Expected IR to contain any of {:?}, but got:\n{}",
            patterns, ir
        );
    }

    // ========================================
    // BASIC FUNCTION & CALL IR GENERATION
    // ========================================

    #[test]
    fn test_ir_simple_function() {
        assert_ir_contains("fn main() { }", "@main");
    }

    #[test]
    fn test_ir_function_with_params() {
        let code = "fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { }";
        assert_ir_contains(code, "@add");
    }

    #[test]
    fn test_ir_function_call() {
        let code =
            "fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { let x = add(2, 3); }";
        assert_ir_contains(code, "call");
    }

    // ========================================
    // ARITHMETIC OPERATIONS - IR PATTERNS
    // ========================================

    #[test]
    fn test_ir_int_arithmetic() {
        // Use variables to avoid constant folding optimization
        assert_ir_contains("fn main() { let a = 5; let b = 3; let x = a + b; }", "add");
        assert_ir_contains("fn main() { let a = 5; let b = 3; let x = a - b; }", "sub");
        assert_ir_contains("fn main() { let a = 5; let b = 3; let x = a * b; }", "mul");
        assert_ir_contains("fn main() { let a = 6; let b = 3; let x = a / b; }", "sdiv");
    }

    #[test]
    fn test_ir_float_arithmetic() {
        // Use variables to avoid constant folding optimization
        assert_ir_contains(
            "fn main() { let a = 5.0; let b = 3.0; let x = a + b; }",
            "fadd",
        );
        assert_ir_contains(
            "fn main() { let a = 5.0; let b = 3.0; let x = a - b; }",
            "fsub",
        );
        assert_ir_contains(
            "fn main() { let a = 5.0; let b = 3.0; let x = a * b; }",
            "fmul",
        );
        assert_ir_contains(
            "fn main() { let a = 6.0; let b = 3.0; let x = a / b; }",
            "fdiv",
        );
    }

    #[test]
    fn test_ir_mixed_arithmetic() {
        let code = "fn main() { let x: Float = 10 + 5.5; }";
        assert_codegen_ok(code);
    }

    #[test]
    fn test_ir_comparison_operations() {
        assert_ir_contains_any("fn main() { let x = 5 == 3; }", &["icmp", "eq"]);
        assert_ir_contains_any("fn main() { let x = 5 < 3; }", &["icmp", "slt"]);
    }

    #[test]
    fn test_ir_boolean_operations() {
        assert_ir_contains("fn main() { let x = true && false; }", "and");
        assert_ir_contains("fn main() { let x = true || false; }", "or");
    }

    // ========================================
    // CONTROL FLOW - IR STRUCTURE
    // ========================================

    #[test]
    fn test_ir_if_statement_branches() {
        let code = "fn main() { if true { } }";
        assert_ir_contains(code, "br");
    }

    #[test]
    fn test_ir_if_else_branches() {
        let code = "fn main() { if true { let x = 1; } else { let y = 2; } }";
        assert_ir_contains(code, "br");
    }

    #[test]
    fn test_ir_nested_if() {
        let code = r#"
            fn main() {
                if true {
                    if false {
                        let x = 1;
                    }
                }
            }
        "#;
        assert_codegen_ok(code);
    }

    #[test]
    fn test_ir_for_loop_branches() {
        let code = "fn main() { for i in 0..10 { } }";
        assert_ir_contains(code, "br");
    }

    #[test]
    fn test_ir_loop_with_break() {
        assert_codegen_ok("fn main() { for i in 0..10 { if i == 5 { break; } } }");
    }

    #[test]
    fn test_ir_loop_with_continue() {
        assert_codegen_ok("fn main() { for i in 0..10 { if i == 5 { continue; } } }");
    }

    // ========================================
    // MEMORY OPERATIONS - IR PATTERNS
    // ========================================

    #[test]
    fn test_ir_variable_allocation() {
        assert_ir_contains("fn main() { let x = 42; }", "alloca");
    }

    #[test]
    fn test_ir_variable_store() {
        assert_ir_contains("fn main() { let x = 42; }", "store");
    }

    #[test]
    fn test_ir_variable_load() {
        assert_ir_contains("fn main() { let x = 42; let y = x; }", "load");
    }

    #[test]
    fn test_ir_mutable_assignment() {
        assert_codegen_ok("fn main() { let mut x = 1; x = 2; }");
    }

    #[test]
    fn test_ir_compound_assignment() {
        assert_codegen_ok("fn main() { let mut x = 10; x += 5; x -= 2; }");
    }

    // ========================================
    // ARRAYS - IR GENERATION
    // ========================================

    #[test]
    fn test_ir_array_literal() {
        assert_ir_contains("fn main() { let x = [1, 2, 3]; }", "alloca");
    }

    #[test]
    fn test_ir_empty_array() {
        assert_codegen_ok("fn main() { let mut x: [Int] = []; }");
    }

    #[test]
    fn test_ir_array_access() {
        assert_codegen_ok("fn main() { let arr = [1, 2, 3]; let x = arr[0]; }");
    }

    #[test]
    fn test_ir_array_in_loop() {
        let code = r#"
            fn main() {
                let arr = [10, 20, 30];
                for i in 0..3 {
                    let x = arr[i];
                }
            }
        "#;
        assert_codegen_ok(code);
    }

    #[test]
    fn test_ir_array_with_expressions() {
        assert_ir_contains("fn main() { let x = [1 + 1, 2 * 2]; }", "alloca");
    }

    // ========================================
    // MAPS - IR GENERATION
    // ========================================

    #[test]
    fn test_ir_map_literal() {
        assert_ir_contains(r#"fn main() { let m = {"a": 1, "b": 2}; }"#, "alloca");
    }

    #[test]
    fn test_ir_empty_map() {
        assert_codegen_ok("fn main() { let mut m: {Str: Int} = {}; }");
    }

    #[test]
    fn test_ir_map_access() {
        assert_codegen_ok(r#"fn main() { let m = {"key": 42}; let x = m.get("key"); }"#);
    }

    #[test]
    fn test_ir_map_destructure_for_loop() {
        assert_codegen_ok(r#"fn main() { let m = {"a": 1, "b": 2}; for (k, v) in m { } }"#);
    }

    // ========================================
    // IMPORTS - IR GENERATION (BASIC ONLY)
    // ========================================

    #[test]
    fn test_ir_import_statement() {
        // Just verify it compiles to IR, don't test all variations
        assert_codegen_ok("import std::Math::{Abs}; fn main() { let x = Abs(-5); }");
    }

    // ========================================
    // STRINGS & CONCATENATION
    // ========================================

    #[test]
    fn test_ir_string_literal() {
        assert_codegen_ok(r#"fn main() { let s = "hello"; }"#);
    }

    #[test]
    fn test_ir_string_concatenation() {
        assert_codegen_ok(r#"fn main() { let s = "hello" + "world"; }"#);
    }

    #[test]
    fn test_ir_string_concat_with_number() {
        assert_codegen_ok(r#"fn main() { let s = "value: " + 42; }"#);
    }

    // ========================================
    // COMPLEX PROGRAMS - IR VERIFICATION
    // ========================================

    #[test]
    fn test_ir_recursive_function() {
        let code = r#"
            fn fact(n: Int) -> Int {
                if n <= 1 { return 1; }
                return n * fact(n - 1);
            }
            fn main() { let x = fact(5); }
        "#;
        assert_codegen_ok(code);
        assert_ir_contains(code, "call");
    }

    #[test]
    fn test_ir_multiple_function_calls() {
        let code = r#"
            fn double(x: Int) -> Int { return x * 2; }
            fn add(a: Int, b: Int) -> Int { return a + b; }
            fn main() {
                let x = double(5);
                let y = add(x, 10);
            }
        "#;
        assert_codegen_ok(code);
        assert_ir_contains(code, "call");
    }

    #[test]
    fn test_ir_nested_control_flow() {
        let code = r#"
            fn main() {
                let mut total = 0;
                for i in 0..5 {
                    if i > 2 {
                        total += i;
                    }
                }
            }
        "#;
        assert_codegen_ok(code);
    }

    #[test]
    fn test_ir_function_with_array_param() {
        let code = r#"
            fn sumArray(arr: [Int]) -> Int {
                let mut total = 0;
                for val in arr {
                    total += val;
                }
                return total;
            }
            fn main() {
                let nums = [1, 2, 3, 4, 5];
                let result = sumArray(nums);
            }
        "#;
        assert_codegen_ok(code);
    }

    #[test]
    fn test_ir_comprehensive_program() {
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
        assert_codegen_ok(code);
    }

    // ========================================
    // IR TYPE VERIFICATION
    // ========================================

    #[test]
    fn test_ir_module_structure() {
        let ir = compile_code("fn main() { }").expect("Codegen failed");
        assert!(ir.contains("ModuleID") || ir.contains("source_filename"));
    }

    #[test]
    fn test_ir_main_function_exists() {
        assert_ir_contains("fn main() { }", "@main");
    }

    #[test]
    fn test_ir_int_type_representation() {
        let ir = compile_code("fn main() { let x: Int = 42; }").expect("Codegen failed");
        // Should have integer type in IR (i32, i64, etc.)
        assert!(ir.contains("i32") || ir.contains("i64"));
    }

    #[test]
    fn test_ir_float_type_representation() {
        let ir = compile_code("fn main() { let x: Float = 3.14; }").expect("Codegen failed");
        // Should have float type in IR (float, double)
        assert!(ir.contains("float") || ir.contains("double"));
    }

    #[test]
    fn test_ir_void_return_type() {
        assert_ir_contains("fn main() { }", "ret void");
    }

    #[test]
    fn test_ir_function_signature_generation() {
        let code = "fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { }";
        assert_ir_contains(code, "define");
    }

    #[test]
    fn test_ir_early_return() {
        let code = r#"
            fn foo() -> Int {
                if true { return 42; }
                return 0;
            }
            fn main() { }
        "#;
        assert_codegen_ok(code);
        assert_ir_contains(code, "ret");
    }

    #[test]
    fn test_ir_multiple_returns() {
        let code = r#"
            fn classify(n: Int) -> Str {
                if n < 0 { return "negative"; }
                if n == 0 { return "zero"; }
                return "positive";
            }
            fn main() { }
        "#;
        assert_codegen_ok(code);
    }

    #[test]
    fn test_ir_array_method_chain() {
        assert_codegen_ok(
            "fn main() { let x = [1, 2, 3].map((n) => n * 2).filter((n) => n > 2); }",
        );
    }

    #[test]
    fn test_ir_nested_loops_structure() {
        let code = r#"
            fn main() {
                for i in 0..5 {
                    for j in 0..5 {
                        let x = i + j;
                    }
                }
            }
        "#;
        assert_codegen_ok(code);
    }

    // =====================================================================
    // Type Cast
    // =====================================================================

    #[test]
    #[ignore = "Runtime error - strings fail to parse at runtime, not compile time"]
    fn test_codegen_runtime_string_parse_panics() {
        // Invalid String -> Int
        assert_codegen_err(r#"fn main() { print("hello" as Int, typeOf("hello" as Int)); }"#);
        assert_codegen_err(r#"fn main() { print("abc123" as Int, typeOf("abc123" as Int)); }"#);
        assert_codegen_err(r#"fn main() { print("12.34" as Int, typeOf("12.34" as Int)); }"#);
        assert_codegen_err(r#"fn main() { print("" as Int, typeOf("" as Int)); }"#);
        assert_codegen_err(r#"fn main() { print("  " as Int, typeOf("  " as Int)); }"#);
        assert_codegen_err(r#"fn main() { print("123abc" as Int, typeOf("123abc" as Int)); }"#);

        // Invalid String -> Float
        assert_codegen_err(r#"fn main() { print("hello" as Float, typeOf("hello" as Float)); }"#);
        assert_codegen_err(r#"fn main() { print("abc" as Float, typeOf("abc" as Float)); }"#);
        assert_codegen_err(r#"fn main() { print("" as Float, typeOf("" as Float)); }"#);
        assert_codegen_err(
            r#"fn main() { print("3.14.15" as Float, typeOf("3.14.15" as Float)); }"#,
        );
    }

    #[test]
    #[ignore = "Runtime error - infinity/NaN conversion fails at runtime, not compile time"]
    fn test_codegen_special_float_values_casts() {
        assert_codegen_err(
            r#"fn main() { let inf = 1.0 / 0.0; print(inf as Int, typeOf(inf as Int)); }"#,
        );
        assert_codegen_err(
            r#"fn main() { let negInf = -1.0 / 0.0; print(negInf as Int, typeOf(negInf as Int)); }"#,
        );

        assert_codegen_err(
            r#"fn main() { let nan = 0.0 / 0.0; print(nan as Int, typeOf(nan as Int)); }"#,
        );
        assert_codegen_err(
            r#"fn main() { let inf = 1.0 / 0.0; print(inf as Str, typeOf(inf as Str)); }"#,
        );

        assert_codegen_err(
            r#"fn main() { let nan = 0.0 / 0.0; print(nan as Str, typeOf(nan as Str)); }"#,
        );
        assert_codegen_err(r#"fn main() { let inf = 1.0 / 0.0; print(inf as Int); }"#);
    }

    #[test]
    #[ignore = "Runtime error - float overflow happens at runtime, not compile time"]
    fn test_codegen_float_to_int_overflow() {
        let code =
            r#"fn main() { let huge = 9999999999.0; print("9999999999.0 -> Int:", huge as Int); }"#;
        assert_codegen_err(code);

        // Max 32-bit signed int
        let code_max =
            r#"fn main() { let maxInt = 2147483647; print("MaxInt -> Float:", maxInt as Float); }"#;
        assert_codegen_err(code_max);

        // MaxInt + 1 (overflow for i32)
        let code_overflow = r#"fn main() { let maxInt = 2147483647; print("MaxInt + 1 -> Float:", maxInt + 1 as Float); }"#;
        assert_codegen_err(code_overflow);
    }
}
