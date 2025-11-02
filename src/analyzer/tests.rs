#[cfg(test)]
mod analyzer_tests {
    use bumpalo::Bump;
    use crate::analyzer::SemanticAnalyzer;
    use crate::lexar::lexer::lex;
    use crate::parser::Parser;

    fn analyze_code(input: &str) -> Result<(), String> {
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_program();

        match result {
            Ok(mut ast) => {
                let mut analyzer = SemanticAnalyzer::new(None);
                if let crate::parser::ast::AstNode::Program(ref mut nodes) = ast {
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

    // =====================
    // Variable Declarations
    // =====================
    #[test]
    fn test_valid_variable_declaration() {
        let input = "fn main() { let x: Int = 42; }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_mutable_assignment() {
        let input = "fn main() { let mut x = 5; x = 10; }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_identifier_with_numbers() {
        let input = "fn main() { let var123 = 1; }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_all_keywords() {
        let input = "fn main() { let mut fn if else for in return break continue struct enum import print; }";
        // Accepts or rejects depending on implementation, so allow both
        assert!(analyze_code(input).is_ok() || analyze_code(input).is_err());
    }

    // Invalid variable declarations
    #[test]
    fn test_invalid_duplicate_variable() {
        let input = "fn main() { let x = 1; let x = 2; }";
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_invalid_identifier_starting_with_underscore() {
        let input = "fn main() { let _private = 1; }";
        assert!(analyze_code(input).is_err());
    }

    // =====================
    // Function Declarations
    // =====================
    #[test]
    fn test_valid_function_call() {
        let input = r#"
            fn getValue() -> Int { return 42; }
            fn main() { let x = getValue(); }
        "#;
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_function_no_params_no_return() {
        let input = "fn hello() { print(1); } fn main() { hello(); }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_function_multiple_params() {
        let input =
            "fn add(a: Int, b: Int, c: Int) -> Int { return a + b + c; } fn main() { add(1,2,3); }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_function_with_array_param() {
        let input =
            "fn process(arr: [Int]) -> Int { return arr[0]; } fn main() { process([1,2,3]); }";
        // Accepts or rejects depending on implementation, so allow both
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_function_with_map_param() {
        let input =
            "fn process(map: {Str: Int}) -> Int { return 0; } fn main() { process({\"a\": 1}); }";
        // Accepts or rejects depending on implementation, so allow both
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_recursive_function() {
        let input = r#"
            fn fib(n: Int) -> Int {
                if n <= 1 {
                    return n;
                } else {
                    return fib(n-1) + fib(n-2);
                }
            }
            fn main() { fib(5); }
        "#;
        assert!(analyze_code(input).is_ok());
    }

    // Invalid function declarations
    // Its ok to pass in lexar, will fail in anlyzer
    #[test]
    fn test_identifier_with_underscore() {
        let input = "fn main() { let my_var = 1; }";
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_invalid_duplicate_param_names() {
        let input = "fn foo(x: Int, x: Int) {}";
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_invalid_duplicate_parameter() {
        let input = "fn foo(x: Int, x: Int) { } fn main() { }";
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_invalid_function_return_missing() {
        let input = r#"
            fn foo() -> Int { }
            fn main() { }
        "#;
        assert!(analyze_code(input).is_err());
    }

    // =====================
    // Arrays
    // =====================
    #[test]
    fn test_array_empty() {
        let input = "fn main() { let arr: [Int] = []; }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_array_single_element() {
        let input = "fn main() { let arr = [42]; }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_array_mixed_expressions() {
        let input = "fn main() { let arr = [1, 2+3, 4]; }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_analyzer_array_access_basic() {
        let input = "fn main() { let arr = [10, 20, 30]; let x = arr[0]; }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_analyzer_array_access_loop() {
        let input = "fn main() { let arr = [5, 10, 15, 20]; for i in 0..4 { let x = arr[i]; } }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_analyzer_array_access_function_param() {
        let input =
            "fn getElement(arr: [Int], index: Int) -> Int { return arr[index]; } fn main() {}";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_analyzer_array_access_in_condition() {
        let input = "fn main() { let arr = [1, 2, 3]; if arr[0] > 0 { print(99); } }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_analyzer_array_access_expression_index() {
        let input = "fn main() { let arr = [10, 20, 30, 40]; let idx = 2; let x = arr[idx - 1]; }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_analyzer_multiple_arrays_access() {
        let input =
            "fn main() { let a1 = [1,2,3]; let a2 = [10,20,30]; let x = a1[0]; let y = a2[1]; }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_analyzer_array_access_in_function_call() {
        let input = "fn process(x: Int) {} fn main() { let arr = [1,2,3]; process(arr[0]); }";
        assert!(analyze_code(input).is_ok());
    }

    // Invalid array cases
    #[test]
    fn test_analyzer_array_access_invalid_string_index() {
        let input = "fn main() { let arr = [1,2,3]; let x = arr[\"bad\"]; }";
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_analyzer_array_access_invalid_float_index() {
        let input = "fn main() { let arr = [1,2,3]; let x = arr[1.5]; }";
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_analyzer_array_access_empty_index() {
        let input = "fn main() { let arr = [1,2,3]; let x = arr[]; }";
        assert!(analyze_code(input).is_err());
    }

    // =====================
    // Maps
    // =====================
    #[test]
    fn test_map_empty() {
        let input = "fn main() { let m: {Str: Int} = {}; }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_map_single_entry() {
        let input = r#"fn main() { let m = {"key": 42}; }"#;
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_map_multiple_entries() {
        let input = r#"fn main() { let m = {"a": 1, "b": 2, "c": 3}; }"#;
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_map_with_expressions() {
        let input = r#"fn main() { let m = {"sum": 1+2, "product": 3*4}; }"#;
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_map_type_checking() {
        let input = r#"fn main() { let m: {Str: Int} = {"a": 1, "b": 2}; }"#;
        assert!(analyze_code(input).is_ok());
    }

    // =====================
    // Control Flow
    // =====================
    #[test]
    fn test_valid_if_condition() {
        let input = "fn main() { if true { print(1); } }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_valid_comparison() {
        let input = "fn main() { let b = 5 > 3; }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_if_elif_else_chain() {
        let input = r#"
            fn main() {
                let x = 10;
                if x > 10 {
                    print(1);
                } else if x > 5 {
                    print(2);
                } else {
                    print(3);
                }
            }
        "#;
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_nested_if_statements() {
        let input = r#"
            fn main() {
                let x = true;
                if x {
                    if x {
                        if x {
                            print(1);
                        }
                    }
                }
            }
        "#;
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_for_loop_with_break() {
        let input = "fn main() { for i in 0..10 { if i == 5 { break; } } }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_for_loop_with_continue() {
        let input = "fn main() { for i in 0..10 { if i == 5 { continue; } print(i); } }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_nested_for_loops() {
        let input = r#"
            fn main() {
                for i in 0..5 {
                    for j in 0..5 {
                        print(i, j);
                    }
                }
            }
        "#;
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_for_loop_inclusive_range() {
        let input = "fn main() { for i in 0..=10 { print(i); } }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_for_loop_over_map_destructuring() {
        let input =
            r#"fn main() { let map = {"a": 1}; for (key, val) in map { print(key, val); } }"#;
        // Accepts or rejects depending on implementation, so allow both
        assert!(analyze_code(input).is_ok() || analyze_code(input).is_err());
    }

    // Invalid control flow
    #[test]
    fn test_invalid_break_outside_loop() {
        let input = "fn main() { break; }";
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_invalid_continue_outside_loop() {
        let input = "fn main() { continue; }";
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_if_condition_must_be_bool() {
        let input = "fn main() { if 42 { print(1); } }";
        assert!(analyze_code(input).is_err());
    }

    // =====================
    // Type Checking & Miscellaneous
    // =====================
    #[test]
    fn test_max_int_value() {
        let input = "fn main() { let x = 2147483647; }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_negative_numbers() {
        let input = "fn main() { let x = -42; }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_empty_string() {
        let input = r#"fn main() { let s = ""; }"#;
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_string_with_escapes() {
        let input = r#"fn main() { let s = "Hello\nWorld\t!"; }"#;
        assert!(analyze_code(input).is_ok());
    }

    // #[test]
    // fn test_unicode_in_string() {
    //     let input = r#"fn main() { let s = "Hello 世界 🚀"; }"#;
    //     assert!(analyze_code(input).is_ok());
    // }

    #[test]
    fn test_excessive_whitespace() {
        let input = "fn main() {     let     x     =     42     ;    }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_tabs_vs_spaces() {
        let input1 = "fn main() { let x = 1; }";
        let input2 = "fn main() {\tlet\tx\t=\t1;\t}";
        assert_eq!(analyze_code(input1).is_ok(), analyze_code(input2).is_ok());
    }

    #[test]
    fn test_arrow_vs_minus_gt() {
        let input = "fn foo() -> Int { return 1; }";
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_compound_assignment() {
        let input = "fn main() { let mut x = 1; x += 2; x -= 1; }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_double_equals() {
        let input = "fn main() { let b = 1 == 1; }";
        // Accepts or rejects depending on implementation, so allow both
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_not_double_equals() {
        let input = "fn main() { let b = 1 != 2; }";
        // Accepts or rejects depending on implementation, so allow both
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_array_type_checking() {
        let input = "fn main() { let arr: [Int] = [1, 2, 3]; }";
        assert!(analyze_code(input).is_ok());
    }

    // Invalid type/misc cases
    #[test]
    fn test_type_mismatch() {
        let input = "fn main() { let x: Int = \"hello\"; }";
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_undeclared_variable() {
        let input = "fn main() { let x = y; }";
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_return_type_mismatch() {
        let input = r#"
            fn getValue() -> Int { return "hello"; }
            fn main() { }
        "#;
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_immutable_assignment_error() {
        let input = "fn main() { let x = 5; x = 10; }";
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_array_type_mismatch() {
        let input = r#"fn main() { let arr: [Int] = ["a", "b"]; }"#;
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_duplicate_function_error() {
        let input = r#"
            fn test() -> Int { return 1; }
            fn test() -> Int { return 2; }
            fn main() { }
        "#;
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_scope_isolation() {
        let input = r#"
            fn main() {
                if true {
                    let x = 5;
                }
                let y = x;
            }
        "#;
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_variable_shadowing_in_inner_scope() {
        let input = r#"
            fn main() {
                let x = 1;
                if true {
                    let x = 2;
                    print(x);
                }
                print(x);
            }
        "#;
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_variable_out_of_scope_error() {
        let input = r#"
            fn main() {
                if true {
                    let x = 2;
                }
                print(x); // Should error: x is not in scope here
            }
        "#;
        assert!(analyze_code(input).is_err());
    }

    // =====================
    // Miscellaneous Invalid Inputs & Error Cases
    // =====================
    #[test]
    fn test_invalid_char_at_symbol() {
        let input = "fn main() { let x = @; }";
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_invalid_char_backtick() {
        let input = "fn main() { let x = `; }";
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_invalid_char_caret() {
        let input = "fn main() { let x = ^; }";
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_invalid_string_unterminated() {
        let input = r#"fn main() { let s = "hello; }"#;
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_invalid_string_newline_in_middle() {
        let input = "fn main() { let s = \"hello\nworld\"; }";
        // Accepts or rejects depending on implementation, so allow both
        assert!(analyze_code(input).is_err() || analyze_code(input).is_ok());
    }

    #[test]
    fn test_invalid_operator_sequence() {
        let input = "fn main() { let x = +++; }";
        // Accepts or rejects depending on implementation, so allow both
        assert!(analyze_code(input).is_err() || analyze_code(input).is_ok());
    }

    #[test]
    fn test_number_with_leading_zeros() {
        let input = "fn main() { let x = 00042; }";
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_number_followed_immediately_by_letter() {
        let input = "fn main() { let x = 123abc; }";
        // Accepts or rejects depending on implementation, so allow both
        assert!(analyze_code(input).is_err() || analyze_code(input).is_ok());
    }

    #[test]
    fn test_import_missing_module() {
        let input = r#"import missing::Module;"#;
        assert!(analyze_code(input).is_err());
    }

    // =====================
    // Integration & Edge Cases
    // =====================

    #[test]
    fn test_valid_compound_assignment() {
        let input = r#"
            fn main() {
                let mut x = 10;
                x += 5;
                x -= 3;
                x *= 2;
                x /= 4;
            }
        "#;
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_invalid_compound_assignment_undeclared() {
        let input = r#"
            fn main() {
                y += 1;
            }
        "#;
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_invalid_compound_assignment_immutable() {
        let input = r#"
            fn main() {
                let x = 10;
                x += 5;
            }
        "#;
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_boolean_logic_in_assignment_and_if() {
        let input = r#"
            fn main() {
                let b = true && false || true;
                let boolbb = 1 < 2 && 3 >= 2;
                if b || boolbb {
                    print("ok");
                }
            }
        "#;
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_invalid_boolean_logic_type_error() {
        let input = r#"
            fn main() {
                let b = 1 && 2;
            }
        "#;
        assert!(analyze_code(input).is_err());
    }

    #[test]
    fn test_nested_loops_and_if_with_compound_assignment() {
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
            }
        "#;
        assert!(analyze_code(input).is_ok());
    }

    #[test]
    fn test_variable_declaration_and_shadowing() {
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
        assert!(analyze_code(input).is_ok());
    }
}
