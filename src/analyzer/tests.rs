//! Analyzer Unit Tests
//! Tests semantic analysis in isolation - NO mir/codegen dependencies
//!
//! Responsibility: Type checking, scope resolution, error detection
//! Does NOT: Test parsing (parser's job), test MIR generation (MIR's job)

#[cfg(test)]
mod analyzer_tests {
    use crate::analyzer::SemanticAnalyzer;
    use crate::lexer::lexer::lex;
    use crate::parser::ast::AstNode;
    use crate::parser::Parser;
    use bumpalo::Bump;

    fn analyze_code(input: &str) -> Result<(), String> {
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

    fn assert_ok(code: &str) {
        analyze_code(code).expect(&format!("Expected success but got error for:\n{}", code));
    }

    fn assert_err(code: &str) {
        if analyze_code(code).is_ok() {
            panic!("Expected error but got success for:\n{}", code);
        }
    }

    // fn assert_err_contains(code: &str, msg: &str) {
    //     match analyze_code(code) {
    //         Ok(_) => panic!("Expected error containing '{}' but got success", msg),
    //         Err(e) if !e.contains(msg) => panic!("Expected '{}' in error, got:\n{}", msg, e),
    //         _ => {}
    //     }
    // }

    // ========================================
    // VARIABLE DECLARATIONS & SCOPING
    // ========================================

    #[test]
    fn test_analyze_variable_declaration() {
        assert_ok("fn main() { let x = 42; }");
        assert_ok("fn main() { let mut x = 42; }");
        assert_ok("fn main() { let x = \"hello\"; }");
        assert_ok("fn main() { let x = true; }");
    }

    #[test]
    fn test_analyze_let_with_wildcard() {
        assert_ok("fn main() { let _ = 42; }");
    }

    #[test]
    fn test_analyze_variable_shadowing() {
        assert_ok(
            r#"
            fn main() {
                let x = 42;
                if true {
                    let x = "shadowed";
                }
            }
        "#,
        );
    }

    #[test]
    fn test_analyze_variable_shadowing_nested() {
        assert_ok(
            r#"
            fn main() {
                let x = 1;
                for i in 0..5 {
                    let x = i;  // shadow in loop
                }
                let y = x;  // x should still be 1
            }
        "#,
        );
    }

    #[test]
    fn test_analyze_duplicate_variable_error() {
        assert_err("fn main() { let x = 1; let x = 2; }");
    }

    #[test]
    fn test_analyze_undeclared_variable_error() {
        assert_err("fn main() { let x = y; }");
    }

    #[test]
    fn test_analyze_variable_out_of_scope() {
        assert_err(
            r#"
            fn main() {
                if true {
                    let x = 42;
                }
                let y = x;
            }
        "#,
        );
    }

    #[test]
    fn test_analyze_variable_out_of_scope_loop() {
        assert_err(
            r#"
            fn main() {
                for i in 0..5 {
                    let x = i;
                }
                let y = x;  // x is out of scope
            }
        "#,
        );
    }

    #[test]
    fn test_analyze_complex_scoping() {
        assert_ok(
            r#"
            fn main() {
                let x = 1;
                if true {
                    let y = 2;
                    for i in 0..5 {
                        let z = x + y + i;
                    }
                }
            }
        "#,
        );
    }

    // ========================================
    // MUTABLE VARIABLES
    // ========================================

    #[test]
    fn test_analyze_mutable_assignment() {
        assert_ok("fn main() { let mut x = 1; x = 2; }");
    }

    #[test]
    fn test_analyze_immutable_assignment_error() {
        assert_err("fn main() { let x = 1; x = 2; }");
    }

    #[test]
    fn test_analyze_compound_assignment() {
        assert_ok(
            "fn main() { let mut x = 10; x += 5; x -= 2; x *= 3; x %= 4; x /= 4; x++; x--; }",
        );
    }

    #[test]
    fn test_analyze_compound_assignment_undeclared() {
        assert_err("fn main() { x += 5; }");
    }

    #[test]
    fn test_analyze_compound_assignment_immutable() {
        assert_err("fn main() { let x = 10; x += 5; }");
    }

    #[test]
    fn test_analyze_mutable_in_loop() {
        assert_ok(
            r#"
            fn main() {
                let mut sum = 0;
                for i in 0..10 {
                    sum += i;
                }
            }
        "#,
        );
    }

    // ========================================
    // TYPE CHECKING - BASIC TYPES
    // ========================================

    #[test]
    fn test_analyze_type_mismatch_assignment() {
        assert_err("fn main() { let x: Int = \"hello\"; }");
        assert_err("fn main() { let x: Str = 42; }");
        assert_err("fn main() { let x: Bool = 123; }");
        assert_err("fn main() { let x: Bool = \"true\"; }");
    }

    #[test]
    fn test_analyze_type_mismatch_array() {
        assert_err("fn main() { let x: [Int] = 42; }");
        assert_err("fn main() { let x: [Int] = \"not an array\"; }");
    }

    #[test]
    fn test_analyze_type_mismatch_map() {
        assert_err("fn main() { let x: {Str: Int} = [1,2,3]; }");
    }

    // =====================================================================
    // Type Cast
    // =====================================================================

    #[test]
    fn test_analyze_invalid_type_casts() {
        // Int to Bool
        assert_err("let x = 0 as Bool;");
        assert_err("let y = 1 as Bool;");
        assert_err("let z = 42 as Bool;");

        // Float to Bool
        assert_err("let a = 0.0 as Bool;");
        assert_err("let b = 1.0 as Bool;");
        assert_err("let c = 3.14 as Bool;");

        // Str to Bool
        assert_err("let d = \"true\" as Bool;");
        assert_err("let e = \"false\" as Bool;");
        assert_err("let f = \"1\" as Bool;");

        // Bool to Float
        assert_err("let g = true as Float;");
        assert_err("let h = false as Float;");
    }

    // ========================================
    // ARITHMETIC TYPE CHECKING
    // ========================================

    #[test]
    fn test_analyze_arithmetic_int_int() {
        assert_ok("fn main() { let x = 1 + 2; }");
        assert_ok("fn main() { let x = 10 - 5; }");
        assert_ok("fn main() { let x = 3 * 4; }");
        assert_ok("fn main() { let x = 20 / 4; }");
        assert_ok("fn main() { let x = 10 % 3; }");
    }

    #[test]
    fn test_analyze_arithmetic_float_float() {
        assert_ok("fn main() { let x = 1.0 + 2.0; }");
        assert_ok("fn main() { let x = 3.5 - 1.2; }");
    }

    #[test]
    fn test_analyze_arithmetic_int_float_coercion() {
        assert_ok("fn main() { let x: Float = 1 + 2.0; }");
        assert_ok("fn main() { let x: Float = 1.0 + 2; }");
        assert_ok("fn main() { let x: Float = 5 * 2.5; }");
        assert_ok("fn main() { let x: Float = 10.0 / 2; }");
    }

    #[test]
    fn test_analyze_arithmetic_coercion_wrong_result_type() {
        assert_err("fn main() { let x: Int = 1 + 2.0; }");
    }

    #[test]
    fn test_analyze_arithmetic_string_plus_int() {
        assert_ok(r#"fn main() { let x = "value: " + 42; }"#);
    }

    #[test]
    fn test_analyze_arithmetic_string_plus_float() {
        assert_ok(r#"fn main() { let x = "pi: " + 3.14; }"#);
    }

    #[test]
    fn test_analyze_arithmetic_string_plus_string() {
        assert_ok(r#"fn main() { let x = "hello" + " world"; }"#);
    }

    #[test]
    fn test_analyze_arithmetic_unsupported_operation() {
        assert_err(r#"fn main() { let x = "hello" - 5; }"#);
        assert_err(r#"fn main() { let x = "hello" * 2; }"#);
    }

    // ========================================
    // COMPARISON TYPE CHECKING
    // ========================================

    #[test]
    fn test_analyze_comparison_int_int() {
        assert_ok("fn main() { let x = 1 < 2; }");
        assert_ok("fn main() { let x = 1 <= 2; }");
        assert_ok("fn main() { let x = 5 > 3; }");
        assert_ok("fn main() { let x = 5 >= 3; }");
        assert_ok("fn main() { let x = 1 == 2; }");
        assert_ok("fn main() { let x = 1 != 2; }");
    }

    #[test]
    fn test_analyze_comparison_float_float() {
        assert_ok("fn main() { let x = 1.0 >= 2.0; }");
    }

    #[test]
    fn test_analyze_comparison_string_string() {
        assert_ok("fn main() { let x = \"a\" == \"b\"; }");
    }

    #[test]
    fn test_analyze_comparison_type_mismatch() {
        assert_err("fn main() { let x = 1 < \"hello\"; }");
        assert_err("fn main() { let x = \"a\" > 1; }");
    }

    // ========================================
    // BOOLEAN OPERATIONS
    // ========================================

    #[test]
    fn test_analyze_boolean_operations() {
        assert_ok("fn main() { let x = true && false; }");
        assert_ok("fn main() { let x = true || false; }");
        assert_ok("fn main() { let x = !true; }");
    }

    #[test]
    fn test_analyze_logical_op_non_bool_error() {
        assert_err("fn main() { let x = 1 && 2; }");
        assert_err("fn main() { let x = !42; }");
        assert_err("fn main() { let x = \"a\" || \"b\"; }");
    }

    #[test]
    fn test_analyze_complex_boolean_expr() {
        assert_ok("fn main() { let x = true && false || true; }");
    }

    #[test]
    fn test_analyze_boolean_conditional() {
        assert_err("fn main() {  if true > false { print(\"true\"); } }");
    }

    // ========================================
    // FUNCTIONS
    // ========================================

    #[test]
    fn test_analyze_function_declaration() {
        assert_ok("fn foo() { } fn main() { }");
        assert_ok("fn foo(x: Int) { } fn main() { }");
        assert_ok("fn foo(x: Int, y: Int) -> Int { return x + y; } fn main() { }");
    }

    #[test]
    fn test_analyze_duplicate_function_error() {
        assert_err("fn foo() { } fn foo() { } fn main() { }");
    }

    #[test]
    fn test_analyze_duplicate_parameter_error() {
        assert_err("fn foo(x: Int, x: Int) { } fn main() { }");
    }

    #[test]
    fn test_analyze_function_call() {
        assert_ok("fn foo() { } fn main() { foo(); }");
        assert_ok("fn foo(x: Int) { } fn main() { foo(42); }");
    }

    #[test]
    fn test_analyze_function_call_wrong_arg_count() {
        assert_err("fn foo(x: Int) { } fn main() { foo(); }");
        assert_err("fn foo(x: Int) { } fn main() { foo(1, 2); }");
    }

    #[test]
    fn test_analyze_function_call_wrong_arg_type() {
        assert_err(r#"fn foo(x: Int) { } fn main() { foo("hello"); }"#);
    }

    #[test]
    fn test_analyze_function_return_type() {
        assert_ok("fn foo() -> Int { return 42; } fn main() { }");
        assert_err("fn foo() -> Int { return \"hello\"; } fn main() { }");
    }

    #[test]
    fn test_analyze_function_missing_return() {
        assert_err("fn foo() -> Int { let x = 42; } fn main() { }");
    }

    #[test]
    fn test_analyze_function_return_mismatch_type() {
        assert_err("fn foo() -> Bool { return 42; } fn main() { }");
    }

    #[test]
    fn test_analyze_recursive_function() {
        assert_ok(
            r#"
            fn factorial(n: Int) -> Int {
                if n <= 1 {
                    return 1;
                }
                return n * factorial(n - 1);
            }
            fn main() { }
        "#,
        );
    }

    #[test]
    fn test_analyze_mutual_recursion() {
        assert_ok(
            r#"
            fn isEven(n: Int) -> Bool { if n == 0 { return true; } return isOdd(n - 1); }
            fn isOdd(n: Int) -> Bool { if n == 0 { return false; } return isEven(n - 1); }
            fn main() { }
        "#,
        );
    }

    // ========================================
    // ARRAYS
    // ========================================

    #[test]
    fn test_analyze_empty_array() {
        assert_ok("fn main() { let mut x = []; }");
        assert_ok("fn main() { let mut x: [Int] = []; }");
        assert_err("fn main() { let x: [Int] = []; }");
        assert_err("fn main() { let x = []; }");
    }

    #[test]
    fn test_analyze_array_literals() {
        assert_ok("fn main() { let x = [1, 2, 3]; }");
        assert_ok("fn main() { let x = [\"a\", \"b\", \"c\"]; }");
        assert_err("fn main() { let x = [1, \"hello\"]; }");
    }

    #[test]
    fn test_analyze_array_access() {
        assert_ok("fn main() { let x = [1, 2, 3]; let y = x[0]; }");
        assert_err("fn main() { let x = [1, 2, 3]; let y = x[\"hello\"]; }");
        assert_err("fn main() { let x = [1, 2, 3]; let y = x[1.5]; }");
    }

    #[test]
    fn test_analyze_array_access_expr() {
        assert_ok("fn main() { let x = [1, 2, 3]; let i = 1; let y = x[i]; }");
    }

    #[test]
    fn test_analyze_array_methods() {
        assert_ok("fn main() { let x = [1, 2, 3]; let y = x.len(); }");
        assert_ok("fn main() { let mut x = [1, 2]; x.push(3); }");
    }

    #[test]
    fn test_analyze_array_method_wrong_type() {
        assert_err("fn main() { let x = 42; x.push(1); }");
    }

    // ========================================
    // MAPS
    // ========================================

    #[test]
    fn test_analyze_empty_map() {
        assert_ok("fn main() { let mut x: {Str: Int} = {}; }");
        assert_ok("fn main() { let mut x = {}; }");
        assert_err("fn main() { let x: {Str: Int} = {}; }");
        assert_err("fn main() { let x = {}; }");
    }

    #[test]
    fn test_analyze_map_literals() {
        assert_ok(r#"fn main() { let x = {"a": 1, "b": 2}; }"#);
        assert_err(r#"fn main() { let x = {"a": 1, "b": "hello"}; }"#);
        assert_err(r#"fn main() { let x = {"a": 1, 2: 3}; }"#);
    }

    #[test]
    fn test_analyze_map_access() {
        assert_ok(r#"fn main() { let x = {"a": 1}; let y = x["a"]; }"#);
    }

    // ========================================
    // CONTROL FLOW
    // ========================================

    #[test]
    fn test_analyze_if_statement() {
        assert_ok("fn main() { if true { } }");
        assert_ok("fn main() { if true { } else { } }");
        assert_ok("fn main() { if 1 < 2 { } }");
    }

    #[test]
    fn test_analyze_if_condition_not_bool() {
        assert_err("fn main() { if 42 { } }");
        assert_err("fn main() { if \"true\" { } }");
    }

    #[test]
    fn test_analyze_nested_if() {
        assert_ok(
            r#"
            fn main() {
                if true {
                    if false {
                        let x = 1;
                    }
                }
            }
        "#,
        );
    }

    #[test]
    fn test_analyze_for_loop() {
        assert_ok("fn main() { for i in 0..10 { } }");
        assert_ok("fn main() { for i in 0..=10 { } }");
        assert_ok("fn main() { let arr = [1, 2, 3]; for x in arr { } }");
    }

    #[test]
    fn test_analyze_for_loop_invalid_range() {
        assert_err("fn main() { for i in 42 { } }");
        assert_err("fn main() { for i in \"string\" { } }");
    }

    #[test]
    fn test_analyze_for_loop_range_types() {
        assert_err("fn main() { for i in 0.5..10 { } }");
        assert_err("fn main() { for i in 0..10.5 { } }");
    }

    #[test]
    fn test_analyze_for_map_destructure_paren_tuple() {
        assert_ok(r#"fn main() { let map1 = {"a": 1}; for (k, v) in map1 { } }"#);
    }

    #[test]
    fn test_analyze_for_map_destructure_tuple_no_paren() {
        assert_ok(r#"fn main() { let map1 = {"a": 1}; for k, v in map1 { } }"#);
    }

    #[test]
    fn test_analyze_for_map_destructure_wildcard_both_paren() {
        assert_ok(r#"fn main() { let map1 = {"a": 1}; for (_, _) in map1 { } }"#);
    }

    #[test]
    fn test_analyze_for_map_destructure_wildcard_no_paren() {
        assert_ok(r#"fn main() { let map1 = {"a": 1}; for _, _ in map1 { } }"#);
    }

    #[test]
    fn test_analyze_for_map_destructure_wildcard_key_paren() {
        assert_ok(r#"fn main() { let map1 = {"a": 1}; for (_, v) in map1 { } }"#);
    }

    #[test]
    fn test_analyze_for_map_destructure_wildcard_value_paren() {
        assert_ok(r#"fn main() { let map1 = {"a": 1}; for (k, _) in map1 { } }"#);
    }

    #[test]
    fn test_analyze_for_map_destructure_wildcard_key_no_paren() {
        assert_ok(r#"fn main() { let map1 = {"a": 1}; for _, v in map1 { } }"#);
    }

    #[test]
    fn test_analyze_for_map_destructure_wildcard_value_no_paren() {
        assert_ok(r#"fn main() { let map1 = {"a": 1}; for k, _ in map1 { } }"#);
    }

    #[test]
    fn test_analyze_for_map_destructure_multiple() {
        assert_ok(r#"fn main() { for (k, v) in {"a": 1, "b": 2, "c": 3} { print("iter"); } }"#);
    }

    #[test]
    fn test_analyze_for_map_destructure_body_access() {
        assert_ok(r#"fn main() { for (_, v) in {"x": 100} { let y = v; } }"#);
    }

    #[test]
    fn test_analyze_break_continue() {
        assert_ok("fn main() { for i in 0..10 { break; } }");
        assert_ok("fn main() { for i in 0..10 { continue; } }");
    }

    #[test]
    fn test_analyze_break_outside_loop() {
        assert_err("fn main() { break; }");
    }

    #[test]
    fn test_analyze_continue_outside_loop() {
        assert_err("fn main() { continue; }");
    }

    #[test]
    fn test_analyze_for_loop_array_wildcard() {
        assert_ok(r#"fn main() { let arr = [1, 2, 3]; for _ in arr { } }"#);
    }

    #[test]
    fn test_analyze_for_loop_range_wildcard() {
        assert_ok("fn main() { for _ in 0..10 { } }");
        assert_ok("fn main() { for _ in 0..=10 { } }");
    }

    #[test]
    fn test_analyze_for_loop_infinite() {
        assert_ok("fn main() { for { } }");
    }

    #[test]
    fn test_analyze_nested_loops() {
        assert_ok(
            r#"
            fn main() {
                for i in 0..10 {
                    for j in 0..10 {
                        if i == j { break; }
                    }
                }
            }
        "#,
        );
    }

    // ========================================
    // LAMBDA FUNCTIONS
    // ========================================

    #[test]
    fn test_analyze_array_map_method() {
        assert_ok("fn main() { let x = [1, 2, 3]; let y = x.map((n) => n * 2); }");
    }

    #[test]
    fn test_analyze_array_filter_method() {
        assert_ok("fn main() { let x = [1, 2, 3]; let y = x.filter((n) => n > 1); }");
    }

    #[test]
    fn test_analyze_array_filter_wrong_return_type() {
        assert_err("fn main() { let x = [1, 2, 3]; let y = x.filter((n) => n * 2); }");
    }

    #[test]
    fn test_analyze_lambda_type_inference() {
        assert_ok("fn main() { let x = [1, 2]; let y = x.map((n) => n + 1); }");
    }

    // ========================================
    // ERROR CASES - METHOD CALLS
    // ========================================

    #[test]
    fn test_analyze_method_on_wrong_type() {
        assert_err("fn main() { let x = 42; x.push(1); }");
        assert_err(r#"fn main() { let x = "hello"; x.length(); }"#);
    }

    #[test]
    fn test_analyze_chained_method_type_error() {
        assert_err("fn main() { let x = [1, 2]; let y = x.map((n) => n).push(3); }");
    }

    #[test]
    fn test_analyze_reduce_wrong_params() {
        assert_err("fn main() { let x = [1, 2]; let y = x.reduce((n) => n); }");
    }

    // ========================================
    // COMPLEX PROGRAMS
    // ========================================

    #[test]
    fn test_analyze_complex_nested_scopes() {
        assert_ok(
            r#"
            fn main() {
                let x = 1;
                if true {
                    let y = 2;
                    for i in 0..5 {
                        let z = x + y + i;
                    }
                }
            }
        "#,
        );
    }

    #[test]
    fn test_analyze_comprehensive_program() {
        assert_ok(
            r#"
            fn helper(n: Int) -> Int {
                return n * 2;
            }

            fn main() {
                let x = 42;
                let mut arr = [1, 2, 3];
                let doubled = arr.map((n) => helper(n));

                for val in doubled {
                    if val > 5 {
                        print(val);
                    }
                }
            }
        "#,
        );
    }

    #[test]
    fn test_analyze_multiple_functions_interaction() {
        assert_ok(
            r#"
            fn add(a: Int, b: Int) -> Int { return a + b; }
            fn multiply(a: Int, b: Int) -> Int { return a * b; }
            fn compute(x: Int, y: Int) -> Int {
                let sum = add(x, y);
                return multiply(sum, 2);
            }
            fn main() { let result = compute(3, 4); }
        "#,
        );
    }

    // ========================================
    // IMPORTS
    // ========================================

    #[test]
    fn test_analyze_import_statement_braced() {
        assert_ok("import std::Math::{Abs}; fn main() { let x = Abs(-5); }");
    }

    #[test]
    fn test_analyze_import_multiple_braced() {
        assert_ok("import std::Math::{Abs, Min, Max}; fn main() { let a = Abs(-3); let b = Min(1, 2); let c = Max(4, 5); }");
    }

    #[test]
    fn test_analyze_import_wildcard_braced() {
        assert_ok("import std::Math::*; fn main() { let x = Abs(-7); let y = Min(2, 3); let z = Max(8, 9); }");
    }

    #[test]
    fn test_analyze_import_aliased_braced() {
        assert_ok("import std::Math::{Abs as AbsValue}; fn main() { let x = AbsValue(-10); }");
    }

    #[test]
    fn test_analyze_import_multiple_braced_multiline() {
        assert_ok(
            r#"
            import std::Math::{Abs, Min};
            fn main() { let x = Abs(-5); let y = Min(1, 2); }
            "#,
        );
    }

    #[test]
    fn test_parse_program_imports_main_missing() {
        assert_err("import std::Math::{Abs};");
    }

    // ========================================
    // ADVANCED TYPE SCENARIOS
    // ========================================

    #[test]
    fn test_analyze_array_of_different_types_error() {
        assert_err("fn main() { let x = [1, \"hello\", true]; }");
    }

    #[test]
    fn test_analyze_map_inconsistent_values() {
        assert_err(r#"fn main() { let m = {"a": 1, "b": "hello"}; }"#);
    }

    #[test]
    fn test_analyze_map_inconsistent_keys() {
        assert_err(r#"fn main() { let m = {"a": 1, 2: 3}; }"#);
    }

    #[test]
    fn test_analyze_function_parameter_type_checking() {
        assert_ok("fn foo(x: Int, y: Str, z: Bool) { } fn main() { foo(42, \"hello\", true); }");
        assert_err("fn foo(x: Int) { } fn main() { foo(\"not int\"); }");
    }

    // ========================================
    // PRINT FUNCTION
    // ========================================

    #[test]
    fn test_analyze_print_int() {
        assert_ok("fn main() { print(42); }");
    }

    #[test]
    fn test_analyze_print_string() {
        assert_ok(r#"fn main() { print("hello"); }"#);
    }

    #[test]
    fn test_analyze_print_multiple_args() {
        assert_ok(r#"fn main() { print("x", 5, "y", 10); }"#);
    }
}
