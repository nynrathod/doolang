//! Analyzer Unit Tests
//! Tests semantic analysis in isolation
//!
//! Responsibility: Verify SEMANTIC ERROR detection (type checking, scope, etc.)
//! Does NOT: Duplicate "happy path" tests that codegen already covers
//!
//! Rationale: Codegen runs the full pipeline (lexer→parser→analyzer→MIR→codegen),
//! so if codegen passes, analyzer already passed. Analyzer tests should focus on
//! catching ERRORS that only the analyzer can detect.

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

    // ========================================
    // VARIABLE SCOPING ERRORS
    // These errors are ONLY caught by analyzer
    // ========================================

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
                let y = x;
            }
        "#,
        );
    }

    // ========================================
    // MUTABILITY ERRORS
    // These errors are ONLY caught by analyzer
    // ========================================

    #[test]
    fn test_analyze_immutable_assignment_error() {
        assert_err("fn main() { let x = 1; x = 2; }");
    }

    #[test]
    fn test_analyze_compound_assignment_undeclared() {
        assert_err("fn main() { x += 5; }");
    }

    #[test]
    fn test_analyze_compound_assignment_immutable() {
        assert_err("fn main() { let x = 10; x += 5; }");
    }

    // ========================================
    // TYPE MISMATCH ERRORS
    // These errors are ONLY caught by analyzer
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

    // ========================================
    // TYPE CAST ERRORS
    // These errors are ONLY caught by analyzer
    // ========================================

    #[test]
    fn test_analyze_invalid_type_casts() {
        // Int to Bool - not allowed
        assert_err("let x = 0 as Bool;");
        assert_err("let y = 1 as Bool;");
        assert_err("let z = 42 as Bool;");

        // Float to Bool - not allowed
        assert_err("let a = 0.0 as Bool;");
        assert_err("let b = 1.0 as Bool;");
        assert_err("let c = 3.14 as Bool;");

        // Str to Bool - not allowed
        assert_err("let d = \"true\" as Bool;");
        assert_err("let e = \"false\" as Bool;");
        assert_err("let f = \"1\" as Bool;");

        // Bool to Float - not allowed
        assert_err("let g = true as Float;");
        assert_err("let h = false as Float;");
    }

    // ========================================
    // ARITHMETIC TYPE ERRORS
    // These errors are ONLY caught by analyzer
    // ========================================

    #[test]
    fn test_analyze_arithmetic_coercion_wrong_result_type() {
        assert_err("fn main() { let x: Int = 1 + 2.0; }");
    }

    #[test]
    fn test_analyze_arithmetic_unsupported_operation() {
        assert_err(r#"fn main() { let x = "hello" - 5; }"#);
        assert_err(r#"fn main() { let x = "hello" * 2; }"#);
    }

    // ========================================
    // COMPARISON TYPE ERRORS
    // ========================================

    #[test]
    fn test_analyze_comparison_type_mismatch() {
        assert_err("fn main() { let x = 1 < \"hello\"; }");
        assert_err("fn main() { let x = \"a\" > 1; }");
    }

    // ========================================
    // BOOLEAN OPERATION ERRORS
    // ========================================

    #[test]
    fn test_analyze_logical_op_non_bool_error() {
        assert_err("fn main() { let x = 1 && 2; }");
        assert_err("fn main() { let x = !42; }");
        assert_err("fn main() { let x = \"a\" || \"b\"; }");
    }

    #[test]
    fn test_analyze_boolean_conditional() {
        assert_err("fn main() { if true > false { print(\"true\"); } }");
    }

    // ========================================
    // FUNCTION ERRORS
    // ========================================

    #[test]
    fn test_analyze_duplicate_function_error() {
        assert_err("fn foo() { } fn foo() { } fn main() { }");
    }

    #[test]
    fn test_analyze_duplicate_parameter_error() {
        assert_err("fn foo(x: Int, x: Int) { } fn main() { }");
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
    fn test_analyze_function_return_type_mismatch() {
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

    // ========================================
    // CONTROL FLOW ERRORS
    // ========================================

    #[test]
    fn test_analyze_if_condition_not_bool() {
        assert_err("fn main() { if 42 { } }");
        assert_err("fn main() { if \"true\" { } }");
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
    fn test_analyze_break_outside_loop() {
        assert_err("fn main() { break; }");
    }

    #[test]
    fn test_analyze_continue_outside_loop() {
        assert_err("fn main() { continue; }");
    }

    // ========================================
    // ARRAY/MAP OPERATION ERRORS
    // ========================================

    #[test]
    fn test_analyze_array_access_wrong_index_type() {
        assert_err("fn main() { let x = [1, 2, 3]; let y = x[\"hello\"]; }");
        assert_err("fn main() { let x = [1, 2, 3]; let y = x[1.5]; }");
    }

    #[test]
    fn test_analyze_method_on_wrong_type() {
        // push is an array method, not valid on Int
        assert_err("fn main() { let x = 42; x.push(1); }");
        // Note: x.len() on strings IS valid - strings have a len() method in Doo
    }

    #[test]
    fn test_analyze_chained_method_type_error() {
        assert_err("fn main() { let x = [1, 2]; let y = x.map((n) => n).push(3); }");
    }

    // ========================================
    // LAMBDA ERRORS
    // ========================================

    #[test]
    fn test_analyze_array_filter_wrong_return_type() {
        assert_err("fn main() { let x = [1, 2, 3]; let y = x.filter((n) => n * 2); }");
    }

    #[test]
    fn test_analyze_reduce_wrong_params() {
        assert_err("fn main() { let x = [1, 2]; let y = x.reduce((n) => n); }");
    }

    // ========================================
    // EMPTY COLLECTION ERRORS
    // (must be mutable or typed)
    // ========================================

    #[test]
    fn test_analyze_empty_array_needs_mut_or_type() {
        assert_err("fn main() { let x: [Int] = []; }"); // typed but immutable - can't use
        assert_err("fn main() { let x = []; }"); // neither typed nor mutable
    }

    #[test]
    fn test_analyze_empty_map_needs_mut_or_type() {
        assert_err("fn main() { let x: {Str: Int} = {}; }");
        assert_err("fn main() { let x = {}; }");
    }

    // ========================================
    // STRUCT ERRORS
    // ========================================

    #[test]
    fn test_analyze_struct_field_type_mismatch() {
        assert_err(
            r#"struct User { name: Str, age: Int } fn main() { let u = User { name: 123, age: 25 }; }"#,
        );
    }

    #[test]
    fn test_analyze_struct_missing_field() {
        assert_err(
            r#"struct User { name: Str, age: Int } fn main() { let u = User { name: "Alice" }; }"#,
        );
    }

    #[test]
    fn test_analyze_struct_undefined_field() {
        assert_err(
            r#"struct User { name: Str } fn main() { let u = User { name: "Alice", unknown: 1 }; }"#,
        );
    }

    // ========================================
    // ENUM ERRORS
    // ========================================

    #[test]
    fn test_analyze_enum_payload_type_mismatch() {
        assert_err(
            r#"enum Result { Success(Int) } fn main() { let r = Result::Success("wrong"); }"#,
        );
    }

    #[test]
    fn test_analyze_enum_undefined_variant() {
        assert_err("enum Status { Active } fn main() { let s = Status::Unknown; }");
    }

    #[test]
    fn test_analyze_enum_missing_payload() {
        assert_err("enum Result { Success(Int) } fn main() { let r = Result::Success; }");
    }

    // ========================================
    // IN OPERATOR ERRORS
    // ========================================

    #[test]
    fn test_analyze_in_operator_type_mismatch() {
        assert_err(r#"fn main() { let arr = [1, 2, 3]; if "hello" in arr { } }"#);
    }

    // ========================================
    // INLINE IF TYPE ERRORS
    // ========================================

    #[test]
    fn test_analyze_inline_if_type_mismatch() {
        assert_err(r#"fn main() { let x: Int = if true { 1 } else { "hello" }; }"#);
    }

    // ========================================
    // IMPORT ERRORS
    // ========================================

    #[test]
    fn test_analyze_program_missing_main() {
        assert_err("import std::Math::{Abs};");
    }

    // ========================================
    // HAPPY PATH SANITY CHECKS (MINIMAL)
    // Keep very few to ensure analyzer doesn't reject valid code
    // These are minimal checks, not comprehensive coverage
    // ========================================

    #[test]
    fn test_analyze_basic_program_ok() {
        assert_ok("fn main() { }");
    }

    #[test]
    fn test_analyze_variable_shadowing_in_nested_scope_ok() {
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
    fn test_analyze_mutable_assignment_ok() {
        assert_ok("fn main() { let mut x = 1; x = 2; }");
    }

    #[test]
    fn test_analyze_empty_array_mutable_ok() {
        assert_ok("fn main() { let mut x = []; }");
        assert_ok("fn main() { let mut x: [Int] = []; }");
    }

    #[test]
    fn test_analyze_empty_map_mutable_ok() {
        assert_ok("fn main() { let mut x: {Str: Int} = {}; }");
        assert_ok("fn main() { let mut x = {}; }");
    }

    #[test]
    fn test_analyze_function_with_return_ok() {
        assert_ok("fn foo() -> Int { return 42; } fn main() { }");
    }

    #[test]
    fn test_analyze_recursive_function_ok() {
        assert_ok(
            r#"
            fn factorial(n: Int) -> Int {
                if n <= 1 { return 1; }
                return n * factorial(n - 1);
            }
            fn main() { }
        "#,
        );
    }

    #[test]
    fn test_analyze_import_ok() {
        assert_ok("import std::Math::{Abs}; fn main() { let x = Abs(-5); }");
    }
}
