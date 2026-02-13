//! Parser Unit Tests
//! Tests AST generation and syntax validation
//!
//! Responsibility: Verify SYNTAX correctness and SYNTAX ERRORS
//!   - Parse errors (missing semicolons, unclosed braces, etc.)
//!   - Syntax variations (different ways to write same construct)
//!   - AST structure verification for complex expressions
//!
//! Does NOT: Duplicate "valid program" tests that codegen already covers
//!
//! Rationale: Codegen runs the full pipeline (lexer→parser→analyzer→MIR→codegen),
//! so if codegen passes, parser already passed. Parser tests should focus on
//! SYNTAX ERRORS and syntax variations that parser uniquely handles.

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

    fn assert_program_parses(input: &str) {
        assert!(
            parse_program(input).is_ok(),
            "Failed to parse program: {}",
            input
        );
    }

    fn assert_program_fails(input: &str) {
        assert!(
            parse_program(input).is_err(),
            "Program should fail: {}",
            input
        );
    }

    // =====================================================================
    // SYNTAX ERRORS - MISSING DELIMITERS
    // These errors are ONLY caught by parser
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
        assert_program_fails("fn main() { let x = 42;");
    }

    #[test]
    fn test_parse_error_unclosed_string() {
        // Lexer handles this but parser should handle incomplete expressions
        assert_parse_fails("let x = \"hello;");
    }

    #[test]
    fn test_parse_error_missing_closing_bracket_in_array() {
        assert_parse_fails("let arr = [1, 2, 3");
    }

    #[test]
    fn test_parse_error_missing_closing_brace_in_map() {
        assert_parse_fails("let m = {\"a\": 1, \"b\": 2");
    }

    // =====================================================================
    // SYNTAX ERRORS - FUNCTION DEFINITIONS
    // =====================================================================

    #[test]
    fn test_parse_error_missing_fn_name() {
        assert_program_fails("fn () { }");
    }

    #[test]
    fn test_parse_error_invalid_param_missing_type() {
        assert_program_fails("fn foo(x) { }");
    }

    #[test]
    fn test_parse_error_missing_arrow_in_return_type() {
        assert_program_fails("fn foo() Int { }");
    }

    #[test]
    fn test_parse_error_missing_function_body() {
        assert_program_fails("fn foo()");
    }

    #[test]
    fn test_parse_error_missing_param_type_after_colon() {
        assert_program_fails("fn foo(x:) { }");
    }

    #[test]
    fn test_parse_error_extra_comma_in_params() {
        assert_program_fails("fn foo(x: Int,) { }");
    }

    // =====================================================================
    // SYNTAX ERRORS - CONTROL FLOW
    // =====================================================================

    #[test]
    fn test_parse_error_if_missing_brace() {
        assert_parse_fails("if true let x = 1;");
    }

    #[test]
    fn test_parse_error_for_missing_in() {
        assert_parse_fails("for i 0..10 { }");
    }

    #[test]
    fn test_parse_error_for_missing_body() {
        assert_parse_fails("for i in 0..10");
    }

    #[test]
    fn test_parse_error_match_missing_arms() {
        assert_parse_fails("match x { }");
    }

    #[test]
    fn test_parse_error_match_missing_fat_arrow() {
        assert_parse_fails("match x { 1 print(1), }");
    }

    // =====================================================================
    // SYNTAX ERRORS - STRUCT/ENUM
    // =====================================================================

    #[test]
    fn test_parse_error_struct_missing_name() {
        assert_program_fails("struct { name: Str }");
    }

    #[test]
    fn test_parse_error_struct_missing_brace() {
        assert_program_fails("struct User name: Str }");
    }

    #[test]
    fn test_parse_error_struct_field_missing_type() {
        assert_program_fails("struct User { name }");
    }

    #[test]
    fn test_parse_error_enum_missing_name() {
        assert_program_fails("enum { Active, Inactive }");
    }

    #[test]
    fn test_parse_error_enum_missing_brace() {
        assert_program_fails("enum Status Active, Inactive }");
    }

    // =====================================================================
    // SYNTAX ERRORS - EXPRESSIONS
    // =====================================================================

    #[test]
    fn test_parse_error_binary_op_missing_operand() {
        assert_parse_fails("let x = 1 +;");
    }

    #[test]
    fn test_parse_error_unary_op_missing_operand() {
        assert_parse_fails("let x = !;");
    }

    #[test]
    fn test_parse_error_method_call_missing_paren() {
        // arr.len is valid field access syntax, not a parse error
        // If len is a method, semantic analysis should catch the error
        assert_parses("let x = arr.len;");
    }

    #[test]
    fn test_parse_error_array_access_missing_bracket() {
        assert_parse_fails("let x = arr[0;");
    }

    // =====================================================================
    // SYNTAX ERRORS - IMPORTS
    // =====================================================================

    #[test]
    fn test_parse_error_import_missing_path() {
        assert_parse_fails("import ;");
    }

    #[test]
    fn test_parse_error_import_missing_semicolon() {
        assert_program_fails("import std::Math fn main() { }");
    }

    // =====================================================================
    // SYNTAX VARIATIONS - LET DECLARATIONS
    // Tests that different valid syntaxes parse correctly
    // =====================================================================

    #[test]
    fn test_parse_let_with_type_annotation() {
        assert_parses("let x: Int = 42;");
        assert_parses("let x: Float = 3.14;");
        assert_parses("let x: Str = \"hello\";");
        assert_parses("let x: Bool = true;");
        assert_parses("let x: [Int] = [1, 2, 3];");
        assert_parses("let x: {Str: Int} = {\"a\": 1};");
    }

    #[test]
    fn test_parse_let_without_type_annotation() {
        assert_parses("let x = 42;");
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

    // =====================================================================
    // SYNTAX VARIATIONS - FOR LOOPS
    // =====================================================================

    #[test]
    fn test_parse_for_range_exclusive() {
        assert_parses("for i in 0..10 { }");
    }

    #[test]
    fn test_parse_for_range_inclusive() {
        assert_parses("for i in 0..=10 { }");
    }

    #[test]
    fn test_parse_for_array_iteration() {
        assert_parses("for x in arr { }");
    }

    #[test]
    fn test_parse_for_with_wildcard() {
        assert_parses("for _ in arr { }");
        assert_parses("for _ in 0..10 { }");
    }

    #[test]
    fn test_parse_for_map_destructure() {
        assert_parses("for k, v in map1 { }");
    }

    #[test]
    fn test_parse_for_map_destructure_wildcards() {
        assert_parses("for _, _ in map1 { }");
        assert_parses("for _, v in map1 { }");
        assert_parses("for k, _ in map1 { }");
    }

    #[test]
    fn test_parse_for_infinite_loop() {
        assert_parses("for { }");
    }

    // =====================================================================
    // SYNTAX VARIATIONS - MATCH EXPRESSIONS
    // =====================================================================

    #[test]
    fn test_parse_match_int_arms() {
        assert_parses("match x { 1 => print(1), _ => print(0), }");
    }

    #[test]
    fn test_parse_match_string_arms() {
        assert_parses(r#"match s { "hello" => print(1), "world" => print(2), _ => print(0), }"#);
    }

    #[test]
    fn test_parse_match_bool_arms() {
        assert_parses("match flag { true => print(\"yes\"), false => print(\"no\"), }");
    }

    #[test]
    fn test_parse_match_enum_variant() {
        assert_parses("match status { Status::Active => print(1), Status::Inactive => print(0), }");
    }

    #[test]
    fn test_parse_match_enum_with_binding() {
        assert_parses(
            "match result { Result::Success(val) => print(val), Result::Failure(msg) => print(msg), }",
        );
    }

    #[test]
    fn test_parse_match_as_expression() {
        assert_parses("let msg = match x { 1 => \"one\", _ => \"other\", };");
    }

    #[test]
    fn test_parse_match_with_block_body() {
        assert_parses("match x { 1 => { let a = 1; print(a); }, _ => print(0), }");
    }

    // =====================================================================
    // SYNTAX VARIATIONS - ERROR HANDLING
    // =====================================================================

    #[test]
    fn test_parse_result_return_type() {
        assert_program_parses(
            "fn divide(a: Int, b: Int) -> Int ! Str { return a / b; } fn main() { }",
        );
    }

    #[test]
    fn test_parse_ok_expression() {
        assert_parses("Ok 42;");
    }

    #[test]
    fn test_parse_err_expression() {
        assert_parses(r#"Err "error message";"#);
    }

    #[test]
    fn test_parse_error_propagation() {
        assert_parses("let val = getValue()?;");
    }

    #[test]
    fn test_parse_manual_error_extract() {
        assert_parses("let result, err = someFunction();");
        assert_parses("let _, err = someFunction();");
    }

    // =====================================================================
    // SYNTAX VARIATIONS - TUPLE
    // =====================================================================

    #[test]
    fn test_parse_tuple_return_type() {
        assert_program_parses("fn getData() -> Int, Str { return 1, \"hello\"; } fn main() { }");
    }

    #[test]
    fn test_parse_tuple_destructuring() {
        assert_parses("let a, b = getData();");
        assert_parses("let a, b, c = getTriple();");
    }

    // =====================================================================
    // SYNTAX VARIATIONS - ARRAY SPREAD & SLICE
    // =====================================================================

    #[test]
    fn test_parse_array_spread() {
        assert_parses("let arr2 = [...arr1, 4, 5];");
        assert_parses("let combined = [...arr1, ...arr2];");
        assert_parses("let arr = [1, 2, ...rest];");
        assert_parses("let arr = [...prefix, 3, 4];");
    }

    #[test]
    fn test_parse_array_slice_exclusive() {
        assert_parses("let slice = arr[1..4];");
    }

    #[test]
    fn test_parse_array_slice_inclusive() {
        assert_parses("let slice = arr[1..=4];");
    }

    #[test]
    fn test_parse_array_slice_with_variables() {
        assert_parses("let slice = arr[start..end];");
    }

    // =====================================================================
    // SYNTAX VARIATIONS - IMPORTS
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

    #[test]
    fn test_parse_import_module_no_braces() {
        assert_program_parses("import std::File; fn main() { }");
    }

    #[test]
    fn test_parse_import_module_alias() {
        assert_program_parses("import std::File as F; fn main() { }");
    }

    #[test]
    fn test_parse_import_module_wildcard() {
        assert_program_parses("import std::File::*; fn main() { }");
    }

    #[test]
    fn test_parse_import_top_level_multiple() {
        assert_program_parses("import std::{File, Math, Json}; fn main() { }");
    }

    // =====================================================================
    // SYNTAX VARIATIONS - LAMBDAS
    // =====================================================================

    #[test]
    fn test_parse_lambda_simple() {
        assert_parses("let a = arr.map((n) => n * 2);");
    }

    #[test]
    fn test_parse_lambda_filter() {
        assert_parses("let a = arr.filter((n) => n > 1);");
    }

    #[test]
    fn test_parse_lambda_reduce() {
        assert_parses("let a = arr.reduce((acc, n) => acc + n);");
    }

    #[test]
    fn test_parse_lambda_block() {
        assert_parses("let doubled = arr.map((x) => { let y = x * 2; return y; });");
    }

    #[test]
    fn test_parse_lambda_block_multiline() {
        assert_parses(
            "let result = arr.filter((x) => { if x > 0 { return true; } return false; });",
        );
    }

    // =====================================================================
    // SYNTAX VARIATIONS - STRING INTERPOLATION
    // =====================================================================

    #[test]
    fn test_parse_string_interpolation_simple() {
        assert_parses(r#"let msg = "Hello ${name}!";"#);
    }

    #[test]
    fn test_parse_string_interpolation_expression() {
        assert_parses(r#"let msg = "Result: ${x + y}";"#);
    }

    #[test]
    fn test_parse_string_interpolation_multiple() {
        assert_parses(r#"let msg = "${first} ${last}";"#);
    }

    // =====================================================================
    // SYNTAX VARIATIONS - TYPE CAST
    // =====================================================================

    #[test]
    fn test_parse_type_cast() {
        assert_parses("let x = value as Int;");
        assert_parses("let x = value as Float;");
        assert_parses("let x = value as Str;");
        assert_parses("let x = a + b as Float;");
    }

    // =====================================================================
    // SYNTAX VARIATIONS - INLINE IF
    // =====================================================================

    #[test]
    fn test_parse_inline_if_expression() {
        assert_parses("let x = if condition { 1 } else { 0 };");
    }

    #[test]
    fn test_parse_conditional_assignment() {
        assert_parses("let result = if x > 0 { x } else { -x };");
    }

    // =====================================================================
    // SYNTAX VARIATIONS - IN OPERATOR
    // =====================================================================

    #[test]
    fn test_parse_in_operator() {
        assert_parses("if 3 in arr { print(\"found\"); }");
        assert_parses(r#"if "key" in map { print("exists"); }"#);
    }

    // =====================================================================
    // SYNTAX VARIATIONS - INCREMENT/DECREMENT
    // =====================================================================

    #[test]
    fn test_parse_increment_decrement() {
        assert_parses("x++;");
        assert_parses("x--;");
    }

    // =====================================================================
    // AST STRUCTURE VERIFICATION
    // Verify that parser produces correct AST for specific constructs
    // =====================================================================

    #[test]
    fn test_parse_let_produces_ast_node() {
        let result = parse_input("let x: Int = 42;");
        assert!(result.is_ok());
        match result.unwrap() {
            AstNode::LetDecl { .. } => (),
            _ => panic!("Expected LetDecl"),
        }
    }

    #[test]
    fn test_parse_program_structure() {
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

    // =====================================================================
    // HAPPY PATH SANITY CHECKS (MINIMAL)
    // Keep very few to ensure parser doesn't reject valid code
    // =====================================================================

    #[test]
    fn test_parse_basic_program_ok() {
        assert_program_parses("fn main() { }");
    }

    #[test]
    fn test_parse_struct_declaration_ok() {
        assert_program_parses("struct User { name: Str, age: Int } fn main() { }");
    }

    #[test]
    fn test_parse_enum_declaration_ok() {
        assert_program_parses("enum Status { Active, Inactive, Pending } fn main() { }");
    }

    #[test]
    fn test_parse_enum_with_payload_ok() {
        assert_program_parses("enum Result { Success(Int), Failure(Str) } fn main() { }");
    }

    #[test]
    fn test_parse_struct_method_ok() {
        assert_program_parses(
            "struct User { age: Int } fn User.isAdult(self) -> Bool { return self.age >= 18; } fn main() { }",
        );
    }
}
