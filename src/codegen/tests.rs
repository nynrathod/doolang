#[cfg(test)]
mod codegen_tests {
    use bumpalo::Bump;
    use crate::analyzer::SemanticAnalyzer;
    use crate::codegen::core::CodeGen;
    use crate::lexar::lexer::lex;
    use crate::mir::builder::MirBuilder;
    use crate::parser::Parser;
    use inkwell::context::Context;

    fn compile_code(input: &str) -> Result<String, String> {
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_program();

        match result {
            Ok(mut ast) => {
                let mut analyzer = SemanticAnalyzer::new(None);
                if let crate::parser::ast::AstNode::Program(ref mut nodes) = ast {
                    match analyzer.analyze_program(nodes) {
                        Ok(_) => {}
                        Err(e) => {
                            eprintln!("Analyzer errors: {:?}", analyzer.collected_errors);
                            return Err(format!("Analyzer error: {:?}", e));
                        }
                    }

                    let mut mir_builder = MirBuilder::new();
                    mir_builder.build_program(nodes);
                    mir_builder.finalize();

                    let context = Context::create();
                    let mut codegen = CodeGen::new("test_module", &context);
                    codegen.generate_program(&mir_builder.program);

                    Ok(codegen.module.print_to_string().to_string())
                } else {
                    Err("Not a program".to_string())
                }
            }
            Err(e) => Err(format!("Parse error: {:?}", e)),
        }
    }

    // =====================
    // Declarations & Functions
    // =====================

    #[test]
    fn test_simple_function_codegen() {
        let input = r#"fn main() { let x = 42; }"#;
        let result = compile_code(input);
        assert!(result.is_ok());
        let ir = result.unwrap();
        assert!(ir.contains("define"));
        assert!(ir.contains("main"));
    }

    #[test]
    fn test_function_with_return() {
        let input = r#"fn getValue() -> Int { return 42; } fn main() { }"#;
        let result = compile_code(input);
        assert!(result.is_ok());
        let ir = result.unwrap();
        assert!(ir.contains("getValue"));
    }

    #[test]
    fn test_arithmetic_operations() {
        let input = r#"fn main() { let x = 5 + 3; let y = x * 2; }"#;
        let result = compile_code(input);
        assert!(result.is_ok());
    }

    // =====================
    // Invalid Declarations & Functions
    // =====================

    #[test]
    fn test_invalid_syntax_codegen() {
        // Missing variable name
        let input = "fn main() { let = 42; }";
        let result = compile_code(input);
        assert!(result.is_err(), "Should fail on invalid syntax");
    }

    #[test]
    fn test_type_error_codegen() {
        // Assign string to Int variable
        let input = "fn main() { let x: Int = \"hello\"; }";
        let result = compile_code(input);
        assert!(result.is_err(), "Should fail on type error");
    }

    #[test]
    fn test_undefined_variable_codegen() {
        // Use of undeclared variable
        let input = "fn main() { let x = y; }";
        let result = compile_code(input);
        assert!(result.is_err(), "Should fail on undefined variable");
    }

    #[test]
    fn test_wrong_function_arg_count_codegen() {
        let input = r#"
            fn add(x: Int, y: Int) -> Int { return x + y; }
            fn main() { let x = add(5); }
        "#;
        let result = compile_code(input);
        assert!(result.is_err(), "Should fail on wrong argument count");
    }

    #[test]
    fn test_wrong_function_arg_type_codegen() {
        let input = r#"
            fn add(x: Int, y: Int) -> Int { return x + y; }
            fn main() { let x = add(5, "hello"); }
        "#;
        let result = compile_code(input);
        assert!(result.is_err(), "Should fail on wrong argument type");
    }

    #[test]
    fn test_return_type_mismatch_codegen() {
        let input = r#"
            fn getValue() -> Int { return "hello"; }
            fn main() { }
        "#;
        let result = compile_code(input);
        assert!(result.is_err(), "Should fail on return type mismatch");
    }

    #[test]
    fn test_immutable_assignment_codegen() {
        let input = "fn main() { let x = 5; x = 10; }";
        let result = compile_code(input);
        assert!(
            result.is_err(),
            "Should fail on assignment to immutable variable"
        );
    }

    // =====================
    // Arrays
    // =====================

    #[test]
    fn test_codegen_array_access_basic() {
        let input = r#"
            fn main() {
                let arr = [10, 20, 30];
                let x = arr[0];
                print(x);
            }
        "#;
        let result = compile_code(input);
        assert!(
            result.is_ok(),
            "Codegen should succeed for basic array access"
        );
        let ir = result.unwrap();
        assert!(ir.contains("main"));
        assert!(ir.contains("arr"));
    }

    #[test]
    fn test_codegen_array_access_in_loop() {
        let input = r#"
            fn main() {
                let arr = [5, 10, 15, 20];
                for i in 0..4 {
                    let x = arr[i];
                    print(x);
                }
            }
        "#;
        let result = compile_code(input);
        assert!(
            result.is_ok(),
            "Codegen should succeed for array access in loop"
        );
        let ir = result.unwrap();
        assert!(ir.contains("main"));
        assert!(ir.contains("arr"));
    }

    // =====================
    // Invalid Arrays
    // =====================

    #[test]
    fn test_codegen_array_access_invalid_empty_index() {
        let input = r#"
            fn main() {
                let arr = [1,2,3];
                let x = arr[];
            }
        "#;
        let result = compile_code(input);
        assert!(
            result.is_err(),
            "Codegen should fail for arr[] (empty index)"
        );
    }

    #[test]
    fn test_codegen_array_access_invalid_string_index() {
        let input = r#"
            fn main() {
                let arr = [1,2,3];
                let x = arr["bad"];
            }
        "#;
        let result = compile_code(input);
        assert!(
            result.is_err(),
            "Codegen should fail for arr[\"bad\"] (string index)"
        );
    }

    #[test]
    fn test_codegen_array_access_invalid_float_index() {
        let input = r#"
            fn main() {
                let arr = [1,2,3];
                let x = arr[1.5];
            }
        "#;
        let result = compile_code(input);
        assert!(
            result.is_err(),
            "Codegen should fail for arr[1.5] (float index)"
        );
    }

    // =====================
    // Control Flow (if, for, etc.)
    // =====================

    #[test]
    fn test_if_statement_codegen() {
        let input = r#"fn main() { if true { let x = 1; } }"#;
        let result = compile_code(input);
        assert!(result.is_ok());
        let ir = result.unwrap();
        assert!(ir.contains("br"));
    }

    #[test]
    fn test_for_loop_codegen() {
        let input = r#"fn main() { for i in 0..5 { print(i); } }"#;
        let result = compile_code(input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_function_call_codegen() {
        let input = r#"fn getValue() -> Int { return 42; } fn main() { let x = getValue(); }"#;
        let result = compile_code(input);
        assert!(result.is_ok());
        let ir = result.unwrap();
        assert!(ir.contains("call"));
    }

    // =====================
    // Invalid Control Flow
    // =====================

    #[test]
    fn test_if_condition_not_bool_codegen() {
        let input = "fn main() { if 42 { print(1); } }";
        let result = compile_code(input);
        assert!(result.is_err(), "Should fail if condition is not bool");
    }

    // =====================
    // Compound Assignment & Boolean Logic
    // =====================

    #[test]
    fn test_codegen_compound_assignment_and_boolean_logic_full() {
        let input = r#"
                fn main() {
                    let mut x = 10;
                    x += 5;
                    x -= 3;
                    x *= 2;
                    x /= 4;
                    let b = true && false || true;
                    let boolbb = 1 < 2 && 3 >= 2;
                    let mut counter = 0;
                    for i in 0..10 {
                        let shouldCount = i > 5 && i < 9;
                        if shouldCount {
                            counter += 1;
                        }
                    }
                    let mut sum = 0;
                    for i in 0..5 {
                        let isEven = i % 2 == 0;
                        if isEven {
                            sum += i;
                        }
                    }
                }
            "#;
        let result = compile_code(input);
        if let Err(ref e) = result {
            eprintln!("Compilation failed: {}", e);
        }
        assert!(result.is_ok(), "Compilation failed: {:?}", result);
        let ir = result.unwrap();
        assert!(ir.contains("define"));
        assert!(ir.contains("main"));
    }

    #[test]
    fn test_codegen_nested_loops_and_if_full() {
        let input = r#"
                    fn main() {
                        let mut total = 0;
                        for i in 0..3 {
                            for j in 0..3 {
                                if i > 0 && j > 0 {
                                    total += 1;
                                }
                            }
                        }
                        let mut count = 0;
                        for i in 0..5 {
                            if i > 1 {
                                for j in 0..3 {
                                    if j < 2 {
                                        count += 1;
                                    }
                                }
                            }
                        }
                    }
                "#;
        let result = compile_code(input);
        if let Err(ref e) = result {
            eprintln!("Compilation failed: {}", e);
        }
        assert!(result.is_ok(), "Compilation failed: {:?}", result);
    }

    #[test]
    fn test_codegen_variable_declaration_and_shadowing_full() {
        let input = r#"
                    fn main() {
                        let i = 1;
                        let j = 1;
                        let cond = i > 0 && j > 0;
                        if cond {
                            print("Inside if: condition is true");
                        }
                        if i > 0 && j > 0 {
                            print("Direct if: condition is true");
                        }
                    }
                "#;
        let result = compile_code(input);
        assert!(result.is_ok());
    }

    // =====================
    // Invalid Compound Assignment & Boolean Logic
    // =====================

    #[test]
    fn test_codegen_compound_assignment_undeclared_full() {
        let input = r#"
                    fn main() {
                        y += 1;
                    }
                "#;
        let result = compile_code(input);
        assert!(
            result.is_err(),
            "Should fail on compound assignment to undeclared variable"
        );
    }

    #[test]
    fn test_codegen_compound_assignment_immutable_full() {
        let input = r#"
                    fn main() {
                        let x = 5;
                        x += 1;
                    }
                "#;
        let result = compile_code(input);
        assert!(
            result.is_err(),
            "Should fail on compound assignment to immutable variable"
        );
    }

    #[test]
    fn test_codegen_compound_assignment_type_error_full() {
        let input = r#"
                    fn main() {
                        let mut s = "hello";
                        s += 1;
                    }
                "#;
        let result = compile_code(input);
        assert!(
            result.is_err(),
            "Should fail on compound assignment with type error"
        );
    }

    // =====================
    // Comparison
    // =====================

    #[test]
    fn test_comparison_codegen() {
        let input = r#"fn main() { let b = 5 > 3; }"#;
        let result = compile_code(input);
        assert!(result.is_ok());
        let ir = result.unwrap();
        assert!(ir.contains("icmp"));
    }
}
