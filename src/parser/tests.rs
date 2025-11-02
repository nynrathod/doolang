#[cfg(test)]
mod parser_tests {
    use bumpalo::Bump;
    use crate::lexar::lexer::lex;
    use crate::parser::ast::AstNode;
    use crate::parser::Parser;

    // =====================
    // Declarations
    // =====================

    #[test]
    fn test_variable_declaration() {
        let input = "let x: Int = 42;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
        match result.unwrap() {
            AstNode::LetDecl { .. } => (),
            _ => panic!("Expected LetDecl"),
        }
    }

    #[test]
    fn test_mutable_variable() {
        let input = "let mut x = 10;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
        match result.unwrap() {
            AstNode::LetDecl { mutable, .. } => assert!(mutable),
            _ => panic!("Expected mutable LetDecl"),
        }
    }

    // =====================
    // Functions
    // =====================

    #[test]
    fn test_function_declaration() {
        let input = "fn add(x: Int, y: Int) -> Int { return x + y; }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
        match result.unwrap() {
            AstNode::FunctionDecl { name, params, .. } => {
                assert_eq!(name, "add");
                assert_eq!(params.len(), 2);
            }
            _ => panic!("Expected FunctionDecl"),
        }
    }

    #[test]
    fn test_function_no_params_no_return() {
        let input = "fn hello() { print(1); }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_function_multiple_params() {
        let input = "fn add(a: Int, b: Int, c: Int) -> Int { return a + b + c; }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_function_with_array_param() {
        let input = "fn process(arr: [Int]) -> Int { return arr[0]; }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_function_with_map_param() {
        let input = "fn process(map: {Str: Int}) -> Int { return 0; }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
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
            "#;
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_function_with_return_type() {
        let input = "fn foo() -> Int { return 1; }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_function_with_empty_body() {
        let input = "fn foo() {}";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_function_with_multiple_return_types() {
        let input = "fn foo() -> (Int, Str) { return 1, \"a\"; }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_function_with_doc_comment() {
        let input = "/// This is a doc comment\nfn foo() {}";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    // ---------------------
    // Invalid Function Tests
    // ---------------------

    #[test]
    fn test_invalid_function_missing_param_type() {
        let input = "fn foo(x) {}";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_function_missing_body() {
        let input = "fn foo(x: Int)";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_function_with_default_param() {
        let input = "fn foo(x: Int = 5) {}";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_function_with_varargs() {
        let input = "fn foo(...args: [Int]) { print(args); }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_function_with_tuple_param() {
        let input = "fn foo((x, y): (Int, Int)) {}";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_function_with_multiple_return_types_with_paren() {
        let input = "fn foo() -> (Int, Str { return 1, \"a\"; }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_function_with_no_body() {
        let input = "fn foo(x: Int);";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    // =====================
    // Expressions
    // =====================

    #[test]
    fn test_mixed_operators_precedence() {
        let input = "let x = 1 + 2 * 3 - 4 / 2;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_comparison_chains() {
        let input = "let b = x > 5 && y < 10 || z == 3;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_unary_minus() {
        let input = "let x = -42;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_string_concatenation_chain() {
        let input = r#"let s = "a" + "b" + "c" + "d";"#;
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_function_call_with_expressions() {
        let input = "print(5 + 3, x * 2, \"hello\" + \" world\");";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_nested_function_calls() {
        let input = "let x = foo(bar(baz(1)));";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    // ---------------------
    // Invalid Expression Tests
    // ---------------------

    #[test]
    fn test_invalid_assignment_to_literal() {
        let input = "5 = x;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_expression_in_statement() {
        let input = "let x = ;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    // =====================
    // Control Flow
    // =====================

    #[test]
    fn test_if_statement() {
        let input = "if x > 5 { print(x); }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
        match result.unwrap() {
            AstNode::ConditionalStmt { .. } => (),
            _ => panic!("Expected ConditionalStmt"),
        }
    }

    #[test]
    fn test_if_elif_else_chain() {
        let input = r#"
                if x > 10 {
                    print(1);
                } else if x > 5 {
                    print(2);
                } else {
                    print(3);
                }
            "#;
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_nested_if_statements() {
        let input = r#"
                if x {
                    if y {
                        if z {
                            print(1);
                        }
                    }
                }
            "#;
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_for_loop_with_break() {
        let input = "for i in 0..10 { if i == 5 { break; } }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_for_loop_with_continue() {
        let input = "for i in 0..10 { if i == 5 { continue; } print(i); }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_nested_for_loops() {
        let input = r#"
                for i in 0..5 {
                    for j in 0..5 {
                        print(i, j);
                    }
                }
            "#;
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_for_loop_inclusive_range() {
        let input = "for i in 0..=10 { print(i); }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_for_loop_over_map_destructuring() {
        let input = r#"for (key, val) in map { print(key, val); }"#;
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_if_with_logical_and() {
        let input = "if x > 0 && y < 5 { print(x, y); }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_if_with_logical_or() {
        let input = "if x == 0 || y == 0 { print(x, y); }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_for_loop_with_empty_body() {
        let input = "for i in 0..10 {}";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    // ---------------------
    // Invalid Control Flow Tests
    // ---------------------

    #[test]
    fn test_invalid_missing_semicolon() {
        let input = "let x = 42";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_if_with_not() {
        let input = "if !x { print(x); }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_unary_not() {
        let input = "let x = !true;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_unclosed_paren() {
        let input = "if (x > 5 { print(x); }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_unclosed_brace() {
        let input = "if x > 5 { print(x); ";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    // =====================
    // Collections (Arrays & Maps)
    // =====================

    #[test]
    fn test_array_empty() {
        let input = "let arr: [Int] = [];";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_array_single_element() {
        let input = "let arr = [42];";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_array_mixed_expressions_same_type() {
        let input = "let arr = [1, 2, 3];";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_map_empty() {
        let input = "let m: {Str: Int} = {};";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_map_single_entry() {
        let input = "let m = {\"a\": 1};";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_map_multiple_entries() {
        let input = "let m = {\"a\": 1, \"b\": 2};";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_map_with_expressions() {
        let input = "let m = {\"a\": 1 + 2, \"b\": 3 * 4};";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    // ---------------------
    // Invalid Collection Tests
    // ---------------------

    #[test]
    fn test_invalid_tuple_declaration() {
        let input = "let t = (1, 2, 3;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_deeply_nested_expressions() {
        let input = "let x = (((((((((1)))))))));";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_map_missing_colon() {
        let input = "let m = {\"a\" 1};";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_array_missing_comma() {
        let input = "let arr = [1 2 3];";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    // =====================
    // Array/Map Element Access
    // =====================

    #[test]
    fn test_array_element_access_literal() {
        let input = "let arr = [1, 2, 3]; let x = arr[0];";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_array_element_access_variable() {
        let input = "let arr = [1, 2, 3]; let i = 1; let x = arr[i];";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_array_element_access_expression() {
        let input = "let arr = [1, 2, 3]; let x = arr[1 + 1];";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    #[test]
    fn test_array_element_access_in_function_call() {
        let input = "let arr = [1,2,3]; print(arr[0]);";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_ok());
    }

    // ---------------------
    // Invalid Element Access Tests
    // ---------------------

    #[test]
    fn test_parser_array_access_invalid_string_index() {
        let input = "let arr = [1,2,3]; let x = arr[\"bad\"];";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        // Parser should accept this; analyzer will reject
        assert!(result.is_ok());
    }

    #[test]
    fn test_parser_array_access_invalid_float_index() {
        let input = "let arr = [1,2,3]; let x = arr[1.5];";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        // Parser should accept this; analyzer will reject
        assert!(result.is_ok());
    }

    // =====================
    // Miscellaneous & Edge Cases
    // =====================

    #[test]
    fn test_parenthesized_expression() {
        let input = "let x = (1 + 2) * 3;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    // ---------------------
    // Invalid Miscellaneous Tests
    // ---------------------

    #[test]
    fn test_invalid_missing_variable_name() {
        let input = "let = 42;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_unterminated_string() {
        let input = "let s = \"hello;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_function_missing_paren() {
        let input = "fn foo( { print(1); }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_statement();
        assert!(result.is_err());
    }
}
