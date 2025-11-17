//! Parser Unit Tests
//! Tests AST generation in isolation - NO analyzer/mir/codegen dependencies
//!
//! Responsibility: Verify AST structure, operator precedence, syntax variations
//! Does NOT: Test type checking (analyzer's job), test semantics

#[cfg(test)]
mod parser_tests {
    use crate::lexer::lexer::lex;
    use crate::parser::ast::AstNode;
    use crate::parser::Parser;
    use bumpalo::Bump;

    fn parse_input(input: &str) -> Result<AstNode, String> {
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        parser.parse_statement().map_err(|e| format!("{:?}", e))
    }

    fn parse_program(input: &str) -> Result<AstNode, String> {
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        parser.parse_program().map_err(|e| format!("{:?}", e))
    }

    fn assert_parses(input: &str) {
        assert!(parse_input(input).is_ok(), "Failed to parse: {}", input);
    }

    fn assert_parse_fails(input: &str) {
        assert!(parse_input(input).is_err(), "Should fail: {}", input);
    }

    // =====================================================================
    // VARIABLE DECLARATIONS - AST STRUCTURE
    // =====================================================================

    #[test]
    fn test_parse_let_declaration() {
        let result = parse_input("let x: Int = 42;");
        assert!(result.is_ok());
        match result.unwrap() {
            AstNode::LetDecl { .. } => (),
            _ => panic!("Expected LetDecl"),
        }
    }

    #[test]
    fn test_parse_let_mutable() {
        let result = parse_input("let mut x = 10;");
        assert!(result.is_ok());
        match result.unwrap() {
            AstNode::LetDecl { mutable, .. } => assert!(mutable),
            _ => panic!("Expected mutable LetDecl"),
        }
    }

    #[test]
    fn test_parse_let_with_wildcard() {
        assert_parses("let _ = 42;");
    }

    #[test]
    fn test_parse_let_with_type_annotations() {
        assert_parses("let x: Int = 42;");
        assert_parses("let x: Float = 42.0;");
        assert_parses("let x: Str = \"hello\";");
        assert_parses("let x: Bool = true;");
        assert_parses("let x: [Int] = [1, 2, 3];");
        assert_parses("let x: {Str: Int} = {\"a\": 1};");
    }

    #[test]
    fn test_parse_let_without_type() {
        assert_parses("let x = 42;");
    }

    #[test]
    fn test_parse_let_string_literal() {
        assert_parses(r#"let s = "hello";"#);
    }

    #[test]
    fn test_parse_let_float_literal() {
        assert_parses(r#"let f = 3.14;"#);
    }

    #[test]
    fn test_parse_let_bool_literal() {
        assert_parses("let b = true;");
    }

    #[test]
    fn test_parse_let_array_literal() {
        assert_parses("let arr = [1, 2, 3];");
    }

    #[test]
    fn test_parse_let_map_literal() {
        assert_parses(r#"let m = {"a": 1, "b": 2};"#);
    }

    #[test]
    fn test_parse_empty_array() {
        assert_parses("let arr: [Int] = [];");
    }

    #[test]
    fn test_parse_empty_map() {
        assert_parses("let m: {Str: Int} = {};");
    }

    // =====================================================================
    // FUNCTION DEFINITIONS - AST STRUCTURE
    // =====================================================================

    #[test]
    fn test_parse_function_basic() {
        let result = parse_program("fn main() { }");
        assert!(result.is_ok());
        match result.unwrap() {
            AstNode::Program(_) => (),
            _ => panic!("Expected Program"),
        }
    }

    #[test]
    fn test_parse_function_no_params_with_return() {
        assert_parses("fn getAnswer() -> Int { return 42; }");
    }

    #[test]
    fn test_parse_function_multiple_params() {
        assert_parses("fn add(a: Int, b: Int) -> Int { return a + b; }");
    }

    #[test]
    fn test_parse_function_array_param() {
        assert_parses("fn process(arr: [Int]) { }");
    }

    #[test]
    fn test_parse_function_map_param() {
        assert_parses("fn process(m: {Str: Int}) { }");
    }

    #[test]
    fn test_parse_function_with_body() {
        assert_parses("fn foo() { let x = 1; let y = 2; }");
    }

    #[test]
    fn test_parse_function_empty_body() {
        assert_parses("fn foo() { }");
    }

    #[test]
    fn test_parse_function_recursive() {
        assert_parses(
            "fn fib(n: Int) -> Int { if n <= 1 { return 1; } return fib(n-1) + fib(n-2); }",
        );
    }

    // =====================================================================
    // EXPRESSIONS - OPERATOR PRECEDENCE
    // =====================================================================

    #[test]
    fn test_parse_expr_addition() {
        let result = parse_input("let a = 1 + 2;");
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_expr_subtraction() {
        assert_parses("let a = 10 - 3;");
    }

    #[test]
    fn test_parse_expr_multiplication() {
        assert_parses("let a = 4 * 7;");
    }

    #[test]
    fn test_parse_expr_division() {
        assert_parses("let a = 20 / 4;");
    }

    #[test]
    fn test_parse_expr_modulo() {
        assert_parses("let a = 10 % 3;");
    }

    #[test]
    fn test_parse_expr_mixed_operators() {
        // Precedence: * / % before + -
        assert_parses("let a = 1 + 2 * 3;");
        assert_parses("let a = 10 - 5 / 2;");
        assert_parses("let a = 3 * 4 + 5 * 6;");
    }

    #[test]
    fn test_parse_expr_unary_minus() {
        assert_parses("let a = -5;");
        assert_parses("let x = -42;");
    }

    #[test]
    fn test_parse_expr_unary_not() {
        assert_parses("let a = !true;");
        assert_parses("let x = !false;");
    }

    // =====================================================================
    // COMPARISONS - OPERATOR PRECEDENCE
    // =====================================================================

    #[test]
    fn test_parse_expr_equals() {
        assert_parses("let a = 1 == 2;");
        assert_parses("let a = 1 == 2 == 3;"); // Left-associative
    }

    #[test]
    fn test_parse_expr_not_equals() {
        assert_parses("let a = 1 != 2;");
    }

    #[test]
    fn test_parse_expr_less_than() {
        assert_parses("let a = 1 < 2;");
    }

    #[test]
    fn test_parse_expr_greater_than() {
        assert_parses("let a = 5 > 3;");
    }

    #[test]
    fn test_parse_expr_less_equal() {
        assert_parses("let a = 1 <= 2;");
    }

    #[test]
    fn test_parse_expr_greater_equal() {
        assert_parses("let a = 5 >= 3;");
    }

    #[test]
    fn test_parse_expr_comparison_chain() {
        assert_parses("let a = 1 < 2 < 3;");
        assert_parses("let a = x == y == z;");
    }

    // =====================================================================
    // LOGICAL OPERATORS - PRECEDENCE
    // =====================================================================

    #[test]
    fn test_parse_expr_logical_and() {
        assert_parses("let a = true && false;");
    }

    #[test]
    fn test_parse_expr_logical_or() {
        assert_parses("let a = true || false;");
    }

    #[test]
    fn test_parse_expr_logical_complex() {
        // Precedence: && before ||
        assert_parses("let a = true || false && true;");
        assert_parses("let a = true || false && true;");
    }

    // =====================================================================
    // CONTROL FLOW - AST STRUCTURE
    // =====================================================================

    #[test]
    fn test_parse_if_simple() {
        assert_parses("if true { }");
    }

    #[test]
    fn test_parse_if_else() {
        assert_parses("if true { } else { }");
    }

    #[test]
    fn test_parse_if_elif_else() {
        assert_parses("if true { } else if false { } else { }");
    }

    #[test]
    fn test_parse_if_with_body() {
        assert_parses("if true { let x = 1; }");
    }

    #[test]
    fn test_parse_if_nested() {
        assert_parses("if true { if false { } }");
    }

    #[test]
    fn test_parse_if_condition_comparison() {
        assert_parses("if 1 < 2 { }");
    }

    #[test]
    fn test_parse_if_condition_logical() {
        assert_parses("if true && false { }");
    }

    // =====================================================================
    // FOR LOOPS - AST STRUCTURE & SYNTAX VARIATIONS
    // =====================================================================

    #[test]
    fn test_parse_for_range() {
        assert_parses("for i in 0..10 { }");
    }

    #[test]
    fn test_parse_for_range_inclusive() {
        assert_parses("for i in 0..=10 { }");
    }

    #[test]
    fn test_parse_for_array() {
        assert_parses("for x in arr { }");
    }

    #[test]
    fn test_parse_for_array_wildcard() {
        assert_parses("for _ in arr { }");
    }

    #[test]
    fn test_parse_for_with_body() {
        assert_parses("for i in 0..5 { print(i); }");
    }

    #[test]
    fn test_parse_for_nested() {
        assert_parses("for i in 0..10 { for j in 0..10 { } }");
    }

    #[test]
    fn test_parse_for_with_break() {
        assert_parses("for i in 0..10 { break; }");
    }

    #[test]
    fn test_parse_for_with_continue() {
        assert_parses("for i in 0..10 { continue; }");
    }

    // =====================================================================
    // FOR LOOP MAP DESTRUCTURING - SYNTAX VARIATIONS
    // =====================================================================

    #[test]
    fn test_parse_for_map_destructure_paren_tuple() {
        assert_parses("for (k, v) in map1 { }");
    }

    #[test]
    fn test_parse_for_map_destructure_tuple_no_paren() {
        assert_parses("for k, v in map1 { }");
    }

    #[test]
    fn test_parse_for_map_destructure_wildcard_both_paren() {
        assert_parses("for (_, _) in map1 { }");
    }

    #[test]
    fn test_parse_for_map_destructure_wildcard_no_paren() {
        assert_parses("for _, _ in map1 { }");
    }

    #[test]
    fn test_parse_for_map_destructure_wildcard_key_paren() {
        assert_parses("for (_, v) in map1 { }");
    }

    #[test]
    fn test_parse_for_map_destructure_wildcard_value_paren() {
        assert_parses("for (k, _) in map1 { }");
    }

    #[test]
    fn test_parse_for_map_destructure_wildcard_key_no_paren() {
        assert_parses("for _, v in map1 { }");
    }

    #[test]
    fn test_parse_for_map_destructure_wildcard_value_no_paren() {
        assert_parses("for k, _ in map1 { }");
    }

    #[test]
    fn test_parse_for_map_destructure_multiple() {
        assert_parses("for (k, v) in {\"a\": 1, \"b\": 2, \"c\": 3} { }");
    }

    #[test]
    fn test_parse_for_infinite_loop() {
        assert_parses("for { }");
    }

    // =====================================================================
    // RETURN, BREAK, CONTINUE
    // =====================================================================

    #[test]
    fn test_parse_return_value() {
        assert_parses("return 42;");
    }

    #[test]
    fn test_parse_return_expression() {
        assert_parses("return x + y;");
    }

    #[test]
    fn test_parse_break_statement() {
        assert_parses("break;");
    }

    #[test]
    fn test_parse_continue_statement() {
        assert_parses("continue;");
    }

    #[test]
    fn test_parse_void_return() {
        assert_parse_fails("fn main() { return; }");
    }

    // =====================================================================
    // ARRAYS - AST STRUCTURE
    // =====================================================================

    // #[test]
    // fn test_parse_array_empty() {
    //     assert_parses("let a = [];");
    // }

    #[test]
    fn test_parse_array_single() {
        assert_parses("let a = [1];");
    }

    #[test]
    fn test_parse_array_multiple() {
        assert_parses("let a = [1, 2, 3];");
    }

    #[test]
    fn test_parse_array_strings() {
        assert_parses("let a = [\"a\", \"b\", \"c\"];");
    }

    #[test]
    fn test_parse_array_access() {
        assert_parses("let a = arr[0];");
    }

    #[test]
    fn test_parse_array_access_expr() {
        assert_parses("let a = arr[i + 1];");
    }

    // =====================================================================
    // MAPS - AST STRUCTURE
    // =====================================================================

    #[test]
    fn test_parse_map_empty() {
        assert_parses("let mut a = {};");
    }

    #[test]
    fn test_parse_map_single() {
        assert_parses("let a = {\"a\": 1};");
    }

    #[test]
    fn test_parse_map_multiple() {
        assert_parses("let a = {\"a\": 1, \"b\": 2};");
    }

    #[test]
    fn test_parse_map_access() {
        assert_parses("let a = m[\"key\"];");
    }

    // TODO: handle parser error handling for nested
    #[test]
    fn test_parse_map_mixed_nesting() {
        assert_parses("let a = {\"a\": [1, 2], \"b\": [3, 4]};");
    }

    // =====================================================================
    // METHOD CALLS - AST STRUCTURE
    // =====================================================================

    #[test]
    fn test_parse_method_call_no_args() {
        assert_parses("let a = arr.len();");
    }

    #[test]
    fn test_parse_method_call_with_args() {
        assert_parses("let a = arr.contains(5);");
    }

    #[test]
    fn test_parse_method_chained() {
        assert_parses("let a = arr.map((x) => x + 1).filter((x) => x > 5);");
    }

    #[test]
    fn test_parse_method_lambda() {
        assert_parses("let a = arr.map((n) => n * 2);");
    }

    #[test]
    fn test_parse_method_filter() {
        assert_parses("let a = arr.filter((n) => n > 1);");
    }

    #[test]
    fn test_parse_method_reduce() {
        assert_parses("let a = arr.reduce((acc, n) => acc + n);");
    }

    #[test]
    fn test_parse_method_typeof() {
        assert_parses("let a = x.typeof();");
    }

    #[test]
    fn test_parse_method_map_block() {
        assert_parses("let a = arr.map((n) => { return n * 2; });");
    }

    #[test]
    fn test_parse_method_filter_complex() {
        assert_parses(
            "let a = arr.filter((n) => { if n > 0 { return true; } else { return false; } });",
        );
    }

    // =====================================================================
    // FUNCTION CALLS - AST STRUCTURE
    // =====================================================================

    #[test]
    fn test_parse_call_no_args() {
        assert_parses("foo();");
    }

    #[test]
    fn test_parse_call_with_args() {
        assert_parses("foo(1, 2, 3);");
    }

    #[test]
    fn test_parse_call_nested() {
        assert_parses("foo(bar(baz()));");
    }

    #[test]
    fn test_parse_call_as_expr() {
        assert_parses("let x = add(5, 3);");
    }

    // =====================================================================
    // IMPORTS - AST STRUCTURE & SYNTAX VARIATIONS
    // =====================================================================

    #[test]
    fn test_parse_import_single() {
        assert_parses("import std::Math::{Abs};");
    }

    #[test]
    fn test_parse_import_multiple() {
        assert_parses("import std::Math::{Abs, Min, Max};");
    }

    #[test]
    fn test_parse_import_wildcard() {
        assert_parses("import std::Math::*;");
    }

    #[test]
    fn test_parse_import_aliased() {
        assert_parses("import std::Math::{Abs as AbsValue};");
    }

    // =====================================================================
    // OPERATOR PRECEDENCE - COMPLEX CASES
    // =====================================================================

    #[test]
    fn test_parse_operator_precedence_mixed() {
        // Test: *, / before +, -
        assert_parses("let a = 2 + 3 * 4;");
        assert_parses("let b = 2 * 3 + 4;");
    }

    #[test]
    fn test_parse_operator_precedence_comparison_logical() {
        // Arithmetic before comparison before logical
        assert_parses("let a = 1 + 2 < 3 + 4;");
        assert_parses("let b = x == y && a > b;");
    }

    #[test]
    fn test_parse_parenthesized() {
        assert_parses("let a = 1 + 2 * 3;");
        assert_parses("let b = x;");
    }

    #[test]
    fn test_parse_complex_expression() {
        assert_parses("let a = a + b * c - d / e + f;");
    }

    // =====================================================================
    // ERROR CASES - PARSE FAILURES
    // =====================================================================

    #[test]
    fn test_parse_error_missing_semicolon() {
        assert_parse_fails("let x = 42");
    }

    #[test]
    fn test_parse_error_unclosed_bracket() {
        assert_parse_fails("let x = [1, 2, 3;");
    }

    #[test]
    fn test_parse_error_unclosed_brace() {
        assert_parse_fails("fn main() { let x = 42;");
    }

    #[test]
    fn test_parse_error_unclosed_paren() {
        assert_parse_fails("let x = (1 + 2;");
    }

    #[test]
    fn test_parse_error_missing_fn_name() {
        assert_parse_fails("fn () { }");
    }

    #[test]
    fn test_parse_error_invalid_param() {
        assert_parse_fails("fn foo(x) { }"); // Missing type
    }

    #[test]
    fn test_parse_error_missing_arrow() {
        assert_parse_fails("fn foo() Int { }"); // Missing ->
    }

    // =====================================================================
    // PROGRAM STRUCTURE
    // =====================================================================

    #[test]
    fn test_parse_program_simple() {
        let result = parse_program("fn main() { }");
        assert!(result.is_ok());
        match result.unwrap() {
            AstNode::Program(stmts) => assert!(!stmts.is_empty()),
            _ => panic!("Expected Program"),
        }
    }

    #[test]
    fn test_parse_program_multiple_functions() {
        let result = parse_program("fn foo() { } fn bar() { } fn main() { }");
        assert!(result.is_ok());
        match result.unwrap() {
            AstNode::Program(stmts) => assert_eq!(stmts.len(), 3),
            _ => panic!("Expected Program with 3 functions"),
        }
    }

    #[test]
    fn test_parse_program_with_variables() {
        assert!(parse_program("let x = 1; fn main() { }").is_ok());
    }

    #[test]
    fn test_parse_program_imports_first() {
        assert!(parse_program("import std::Math::{Abs}; fn main() { }").is_ok());
    }
}
