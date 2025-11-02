// --- VALID TESTS ---
#[cfg(test)]
mod lexer_tests {
    use bumpalo::Bump;
    use crate::lexar::lexer::lex;
    use crate::lexar::token::TokenType;

    // =====================
    // Valid Token Tests
    // =====================

    #[test]
    fn test_basic_tokens() {
        let input = "let x = 42;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::Let);
        assert_eq!(tokens[1].kind, TokenType::Identifier);
        assert_eq!(tokens[1].value, "x");
        assert_eq!(tokens[2].kind, TokenType::Eq);
        assert_eq!(tokens[3].kind, TokenType::Number);
        assert_eq!(tokens[3].value, "42");
        assert_eq!(tokens[4].kind, TokenType::Semi);
    }

    #[test]
    fn test_string_literals() {
        let input = r#"let s = "hello world";"#;
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[3].kind, TokenType::String);
        assert_eq!(tokens[3].value, "hello world");
    }

    #[test]
    fn test_boolean_literals() {
        let input = "let a = true; let b = false;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[3].kind, TokenType::Boolean);
        assert_eq!(tokens[3].value, "true");
        assert_eq!(tokens[8].kind, TokenType::Boolean);
        assert_eq!(tokens[8].value, "false");
    }

    #[test]
    fn test_arithmetic_operators() {
        let input = "+ - * / %";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::Plus);
        assert_eq!(tokens[1].kind, TokenType::Minus);
        assert_eq!(tokens[2].kind, TokenType::Star);
        assert_eq!(tokens[3].kind, TokenType::Slash);
        assert_eq!(tokens[4].kind, TokenType::Percent);
    }

    #[test]
    fn test_assignment_operators() {
        let input = "= += -= *= /= %=";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::Eq);
        assert_eq!(tokens[1].kind, TokenType::PlusEq);
        assert_eq!(tokens[2].kind, TokenType::MinusEq);
        assert_eq!(tokens[3].kind, TokenType::StarEq);
        assert_eq!(tokens[4].kind, TokenType::SlashEq);
        assert_eq!(tokens[5].kind, TokenType::PercentEq);
    }

    // =====================
    // Edge Case Tests
    // =====================
    #[test]
    fn test_max_int_value() {
        let input = "2147483647";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::Number);
    }

    #[test]
    fn test_negative_numbers() {
        let input = "-42";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::Minus);
        assert_eq!(tokens[0].value, "-");
        assert_eq!(tokens[1].kind, TokenType::Number);
        assert_eq!(tokens[1].value, "42");
    }

    #[test]
    fn test_floating_point_supported() {
        let input = "3.14";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenType::Float);
        assert_eq!(tokens[0].value, "3.14");
    }

    #[test]
    fn test_very_long_string() {
        let input = format!(r#"let s = "{}";"#, "a".repeat(10000));
        let arena = Bump::new();
        let tokens = lex(&input, &arena);
        let string_token = tokens.iter().find(|t| t.kind == TokenType::String);
        assert!(string_token.is_some());
    }

    #[test]
    fn test_string_with_escapes() {
        let input = r#"let s = "Hello\nWorld\t!";"#;
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::String));
    }

    #[test]
    fn test_empty_string() {
        let input = r#"let s = "";"#;
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let string_token = tokens.iter().find(|t| t.kind == TokenType::String);
        assert_eq!(string_token.unwrap().value, "");
    }

    #[test]
    fn test_string_with_quotes_inside() {
        let input = r#"let s = "He said \"hi\"";"#;
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::String));
    }

    // =====================
    // Array Access Lexing Tests
    // =====================
    #[test]
    fn test_lex_array_access_basic() {
        let input = "arr[0]";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::Identifier);
        assert_eq!(tokens[0].value, "arr");
        assert_eq!(tokens[1].kind, TokenType::OpenBracket);
        assert_eq!(tokens[2].kind, TokenType::Number);
        assert_eq!(tokens[2].value, "0");
        assert_eq!(tokens[3].kind, TokenType::CloseBracket);
    }

    #[test]
    fn test_lex_array_access_variable_index() {
        let input = "arr[idx]";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::Identifier);
        assert_eq!(tokens[1].kind, TokenType::OpenBracket);
        assert_eq!(tokens[2].kind, TokenType::Identifier);
        assert_eq!(tokens[2].value, "idx");
        assert_eq!(tokens[3].kind, TokenType::CloseBracket);
    }

    #[test]
    fn test_lex_array_access_expression_index() {
        let input = "arr[idx+1]";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::Identifier);
        assert_eq!(tokens[1].kind, TokenType::OpenBracket);
        assert_eq!(tokens[2].kind, TokenType::Identifier);
        assert_eq!(tokens[2].value, "idx");
        assert_eq!(tokens[3].kind, TokenType::Plus);
        assert_eq!(tokens[4].kind, TokenType::Number);
        assert_eq!(tokens[4].value, "1");
        assert_eq!(tokens[5].kind, TokenType::CloseBracket);
    }

    #[test]
    fn test_lex_array_access_nested() {
        let input = "matrix[0][1]";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::Identifier);
        assert_eq!(tokens[1].kind, TokenType::OpenBracket);
        assert_eq!(tokens[2].kind, TokenType::Number);
        assert_eq!(tokens[2].value, "0");
        assert_eq!(tokens[3].kind, TokenType::CloseBracket);
        assert_eq!(tokens[4].kind, TokenType::OpenBracket);
        assert_eq!(tokens[5].kind, TokenType::Number);
        assert_eq!(tokens[5].value, "1");
        assert_eq!(tokens[6].kind, TokenType::CloseBracket);
    }

    #[test]
    fn test_lex_array_access_invalid_empty_index() {
        let input = "arr[]";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::Identifier);
        assert_eq!(tokens[1].kind, TokenType::OpenBracket);
        // Should produce CloseBracket immediately after OpenBracket
        assert_eq!(tokens[2].kind, TokenType::CloseBracket);
    }

    #[test]
    fn test_multiple_line_comments() {
        let input = "// comment 1\n// comment 2\nlet x = 1;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens.len(), 5); // let, x, =, 1, ;
    }

    #[test]
    fn test_comment_at_end_of_line() {
        let input = "let x = 42; // inline comment";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[3].value, "42");
    }

    #[test]
    fn test_identifier_with_numbers() {
        let input = "let var123 = 1;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[1].value, "var123");
    }

    #[test]
    fn test_identifier_with_underscore() {
        let input = "let my_var = 1;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[1].value, "my_var");
    }

    #[test]
    fn test_all_keywords() {
        let input = "let mut fn if else for in return break continue struct enum import print";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens.len(), 14);
    }

    #[test]
    fn test_range_operators() {
        let input = "0..10 0..=10";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::RangeExc));
        assert!(tokens.iter().any(|t| t.kind == TokenType::RangeInc));
    }

    #[test]
    fn test_double_colon() {
        let input = "import http::Client;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        // Should tokenize :: as two colons or specific token
        let colon_count = tokens.iter().filter(|t| t.kind == TokenType::Colon).count();
        assert!(colon_count >= 2);
    }

    #[test]
    fn test_arrow_vs_minus_gt() {
        let input = "fn foo() -> Int";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::Arrow));
    }

    #[test]
    fn test_fat_arrow() {
        let input = "x => y";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::FatArrow));
    }

    #[test]
    fn test_compound_assignment() {
        let input = "+= -=";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::PlusEq);
        assert_eq!(tokens[1].kind, TokenType::MinusEq);
    }

    #[test]
    fn test_triple_equals() {
        let input = "===";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::EqEqEq);
    }

    #[test]
    fn test_not_double_equals() {
        let input = "!==";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::NotEqEq);
    }

    // =====================
    // Stress Tests
    // =====================
    #[test]
    fn test_1000_tokens() {
        let input = "let x = 1; ".repeat(200); // 1000 tokens
        let arena = Bump::new();
        let tokens = lex(&input, &arena);
        assert!(tokens.len() >= 1000);
    }

    #[test]
    fn test_deeply_nested_brackets() {
        let input = "[[[[[[[[[[1]]]]]]]]]]";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let open_count = tokens
            .iter()
            .filter(|t| t.kind == TokenType::OpenBracket)
            .count();
        let close_count = tokens
            .iter()
            .filter(|t| t.kind == TokenType::CloseBracket)
            .count();
        assert_eq!(open_count, close_count);
    }

    #[test]
    fn test_very_long_identifier() {
        let long_name = "a".repeat(1000);
        let input = format!("let {} = 1;", long_name);
        let arena = Bump::new();
        let tokens = lex(&input, &arena);
        assert!(tokens.iter().any(|t| t.value.len() == 1000));
    }

    #[test]
    fn test_many_operators_in_sequence() {
        let input = "+ - * / % == != > < >= <=".repeat(50);
        let arena = Bump::new();
        let tokens = lex(&input, &arena);
        assert!(tokens.len() > 500);
    }

    // =====================
    // Unicode Tests
    // =====================
    // #[test]
    // fn test_unicode_in_string() {
    //     let input = r#"let s = "Hello 世界 🚀";"#;
    //     let tokens = lex(input);
    //     let string_token = tokens.iter().find(|t| t.kind == TokenType::String);
    //     assert!(string_token.unwrap().value.contains("世界"));
    // }

    // TODO: check unicode test
    // #[test]
    // fn test_emoji_in_identifier() {
    //     // Most lexers reject emojis in identifiers, but test behavior
    //     let input = "let x🚀 = 1;";
    //     let tokens = lex(input);
    //     // Should either accept or reject gracefully
    //     assert!(!tokens.is_empty());
    // }

    // =====================
    // Whitespace Handling Tests
    // =====================
    #[test]
    fn test_mixed_whitespace() {
        let input = "let\tx\n=\r\n42;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens.len(), 5);
    }

    #[test]
    fn test_no_whitespace() {
        let input = "let x=42;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens.len(), 5);
    }

    #[test]
    fn test_excessive_whitespace() {
        let input = "let     x     =     42     ;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens.len(), 5);
    }

    #[test]
    fn test_tabs_vs_spaces() {
        let input1 = "let x = 1;";
        let input2 = "let\tx\t=\t1;";
        let arena = Bump::new();
        let tokens1 = lex(input1, &arena);
        let tokens2 = lex(input2, &arena);
        assert_eq!(tokens1.len(), tokens2.len());
    }

    // =====================
    // Invalid Input Tests
    // =====================
    #[test]
    fn test_invalid_char_at_symbol() {
        let input = "@";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::Unknown));
    }

    #[test]
    fn test_invalid_char_backtick() {
        let input = "`";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::Unknown));
    }

    #[test]
    fn test_invalid_char_caret() {
        let input = "^";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::Unknown));
    }

    #[test]
    fn test_invalid_string_unterminated() {
        let input = r#"let s = "hello"#;
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let has_string = tokens.iter().any(|t| t.kind == TokenType::String);
        assert!(
            !has_string,
            "Should not produce String token for unterminated string"
        );
    }

    #[test]
    fn test_invalid_string_newline_in_middle() {
        let input = "let s = \"hello\nworld\";";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        // Behavior depends on lexer - should handle gracefully
        assert!(!tokens.is_empty());
    }

    #[test]
    fn test_number_with_leading_zeros() {
        let input = "00042";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens[0].kind == TokenType::Number);
    }

    #[test]
    fn test_number_followed_immediately_by_letter() {
        let input = "123abc";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        // Should produce Number then Identifier
        assert!(tokens.len() >= 2);
    }

    #[test]
    fn test_invalid_operator_sequence() {
        let input = "+++";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::Plus);
    }

    // Additional invalid/malformed input tests
    #[test]
    fn test_tokenize_all_operators_and_keywords_from_root_tests() {
        let input = r#"
            let x = 1;
            let mut y = 2;
            x += 3;
            y -= 4;
            x *= 5;
            y /= 6;
            let b1 = true && false || true;
            let b2 = 1 < 2 && 3 >= 2;
            let b3 = 4 > 3 || 2 <= 1;
            let eq = 5 == 5;
            let neq = 6 != 7;
            for i in 0..10 {
                if i > 0 && i < 5 {
                    print("i in range:", i);
                } else {
                    print("i out of range:", i);
                }
            }
        "#;
        let arena = Bump::new();
        let tokens = lex(input, &arena);

        // Check for presence of all relevant tokens
        let kinds: Vec<TokenType> = tokens.iter().map(|t| t.kind).collect();
        assert!(kinds.contains(&TokenType::Let));
        assert!(kinds.contains(&TokenType::Mut));
        assert!(kinds.contains(&TokenType::PlusEq));
        assert!(kinds.contains(&TokenType::MinusEq));
        assert!(kinds.contains(&TokenType::StarEq));
        assert!(kinds.contains(&TokenType::SlashEq));
        assert!(kinds.contains(&TokenType::AndAnd));
        assert!(kinds.contains(&TokenType::OrOr));
        assert!(kinds.contains(&TokenType::Lt));
        assert!(kinds.contains(&TokenType::Gt));
        assert!(kinds.contains(&TokenType::LtEq));
        assert!(kinds.contains(&TokenType::GtEq));
        assert!(kinds.contains(&TokenType::EqEq));
        assert!(kinds.contains(&TokenType::NotEq));
        assert!(kinds.contains(&TokenType::For));
        assert!(kinds.contains(&TokenType::If));
        assert!(kinds.contains(&TokenType::Else));
        assert!(kinds.contains(&TokenType::Print));
        assert!(tokens.iter().any(|t| t.value == "true"));
        assert!(tokens.iter().any(|t| t.value == "false"));
    }
    #[test]
    fn test_invalid_char_dollar() {
        let input = "$";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::Dollar));
    }

    #[test]
    fn test_invalid_char_tilde() {
        let input = "~";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::Tilde));
    }

    #[test]
    fn test_invalid_char_pipe() {
        let input = "|";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::Or));
    }

    #[test]
    fn test_invalid_char_backslash() {
        let input = "\\";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::Unknown));
    }

    #[test]
    fn test_invalid_char_brace() {
        let input = "{";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        // Should be recognized as OpenBrace or Unknown
        assert!(tokens.iter().any(|t| t.kind == TokenType::OpenBrace));
    }

    #[test]
    fn test_invalid_char_bracket() {
        let input = "]";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        // Should be recognized as CloseBracket or Unknown
        assert!(tokens.iter().any(|t| t.kind == TokenType::CloseBracket));
    }

    #[test]
    fn test_invalid_char_angle() {
        let input = "<";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        // Should be recognized as Less or Unknown
        assert!(tokens.iter().any(|t| t.kind == TokenType::Lt));
    }

    #[test]
    fn test_invalid_char_percent() {
        let input = "%";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::Percent));
    }

    #[test]
    fn test_invalid_string_escaped_newline() {
        let input = "let s = \"hello\\\nworld\";";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(!tokens.is_empty());
    }

    #[test]
    fn test_invalid_string_escaped_quote() {
        let input = "let s = \"hello\\\"world\";";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::String));
    }

    #[test]
    fn test_invalid_number_alpha() {
        let input = "42abc";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.len() >= 2);
    }

    #[test]
    fn test_lexer_number_dot() {
        let input = "42.";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].kind, TokenType::Number);
        assert_eq!(tokens[1].kind, TokenType::Dot);
    }

    #[test]
    fn test_lexer_number_double_dot() {
        let input = "42..";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].kind, TokenType::Number);
        assert_eq!(tokens[1].kind, TokenType::RangeExc);
    }

    #[test]
    fn test_lexer_number_triple_dot() {
        let input = "42...";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].kind, TokenType::Number);
        assert_eq!(tokens[1].kind, TokenType::RangeExc);
        assert_eq!(tokens[2].kind, TokenType::Dot);
    }

    #[test]
    fn test_invalid_empty_input() {
        let input = "";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(
            tokens.is_empty(),
            "Expected no tokens for empty input, got {:?}",
            tokens
        );
    }

    #[test]
    fn test_invalid_only_whitespace() {
        let input = "    \t\n";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(
            tokens.is_empty(),
            "Expected no tokens for whitespace-only input, got {:?}",
            tokens
        );
    }

    #[test]
    fn test_invalid_comment_only() {
        let input = "// just a comment";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(
            tokens.is_empty(),
            "Expected no tokens for comment-only input, got {:?}",
            tokens
        );
    }

    #[test]
    fn test_invalid_string_only_quote() {
        let input = "\"";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(!tokens.iter().any(|t| t.kind == TokenType::String));
    }

    #[test]
    fn test_invalid_string_only_double_quote() {
        let input = "\"\"";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::String));
    }

    #[test]
    fn test_invalid_string_odd_quotes() {
        let input = "\"hello";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(!tokens.iter().any(|t| t.kind == TokenType::String));
    }

    #[test]
    fn test_invalid_string_odd_quotes2() {
        let input = "hello\"";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(!tokens.iter().any(|t| t.kind == TokenType::String));
    }

    #[test]
    fn test_invalid_string_escaped_backslash() {
        let input = "let s = \"hello\\\\world\";";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::String));
    }

    #[test]
    fn test_invalid_string_escaped_tab() {
        let input = "let s = \"hello\\tworld\";";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::String));
    }

    #[test]
    fn test_invalid_string_escaped_unicode() {
        let input = "let s = \"hello\\u1234world\";";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::String));
    }

    #[test]
    fn test_invalid_string_escaped_hex() {
        let input = "let s = \"hello\\x41world\";";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::String));
    }

    #[test]
    fn test_invalid_string_escaped_null() {
        let input = "let s = \"hello\\0world\";";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::String));
    }

    #[test]
    fn test_invalid_string_escaped_bell() {
        let input = "let s = \"hello\\aworld\";";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens.iter().any(|t| t.kind == TokenType::String));
    }

    #[test]
    fn test_comparison_operators() {
        let input = "== === != !== > < >= <=";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::EqEq);
        assert_eq!(tokens[1].kind, TokenType::EqEqEq);
        assert_eq!(tokens[2].kind, TokenType::NotEq);
        assert_eq!(tokens[3].kind, TokenType::NotEqEq);
        assert_eq!(tokens[4].kind, TokenType::Gt);
        assert_eq!(tokens[5].kind, TokenType::Lt);
        assert_eq!(tokens[6].kind, TokenType::GtEq);
        assert_eq!(tokens[7].kind, TokenType::LtEq);
    }

    #[test]
    fn test_logical_operators() {
        let input = "! & | && ||";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::Bang);
        assert_eq!(tokens[1].kind, TokenType::And);
        assert_eq!(tokens[2].kind, TokenType::Or);
        assert_eq!(tokens[3].kind, TokenType::AndAnd);
        assert_eq!(tokens[4].kind, TokenType::OrOr);
    }

    #[test]
    fn test_arrow_operators() {
        let input = "-> =>";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::Arrow);
        assert_eq!(tokens[1].kind, TokenType::FatArrow);
    }

    #[test]
    fn test_delimiters_and_punctuation() {
        let input = "( ) { } [ ] , ; ..= .. . : # ~ ? $";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::OpenParen);
        assert_eq!(tokens[1].kind, TokenType::CloseParen);
        assert_eq!(tokens[2].kind, TokenType::OpenBrace);
        assert_eq!(tokens[3].kind, TokenType::CloseBrace);
        assert_eq!(tokens[4].kind, TokenType::OpenBracket);
        assert_eq!(tokens[5].kind, TokenType::CloseBracket);
        assert_eq!(tokens[6].kind, TokenType::Comma);
        assert_eq!(tokens[7].kind, TokenType::Semi);
        assert_eq!(tokens[8].kind, TokenType::RangeInc);
        assert_eq!(tokens[9].kind, TokenType::RangeExc);
        assert_eq!(tokens[10].kind, TokenType::Dot);
        assert_eq!(tokens[11].kind, TokenType::Colon);
        assert_eq!(tokens[12].kind, TokenType::Pound);
        assert_eq!(tokens[13].kind, TokenType::Tilde);
        assert_eq!(tokens[14].kind, TokenType::Question);
        assert_eq!(tokens[15].kind, TokenType::Dollar);
    }

    #[test]
    fn test_keywords() {
        let input = "let mut fn if else for in return break continue struct enum import print";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::Let);
        assert_eq!(tokens[1].kind, TokenType::Mut);
        assert_eq!(tokens[2].kind, TokenType::Function);
        assert_eq!(tokens[3].kind, TokenType::If);
        assert_eq!(tokens[4].kind, TokenType::Else);
        assert_eq!(tokens[5].kind, TokenType::For);
        assert_eq!(tokens[6].kind, TokenType::In);
        assert_eq!(tokens[7].kind, TokenType::Return);
        assert_eq!(tokens[8].kind, TokenType::Break);
        assert_eq!(tokens[9].kind, TokenType::Continue);
        assert_eq!(tokens[10].kind, TokenType::Struct);
        assert_eq!(tokens[11].kind, TokenType::Enum);
        assert_eq!(tokens[12].kind, TokenType::Import);
        assert_eq!(tokens[13].kind, TokenType::Print);
    }

    #[test]
    fn test_array_literal() {
        let input = "[1, 2, 3]";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::OpenBracket);
        assert_eq!(tokens[1].kind, TokenType::Number);
        assert_eq!(tokens[2].kind, TokenType::Comma);
        assert_eq!(tokens[3].kind, TokenType::Number);
        assert_eq!(tokens[4].kind, TokenType::Comma);
        assert_eq!(tokens[5].kind, TokenType::Number);
        assert_eq!(tokens[6].kind, TokenType::CloseBracket);
    }

    #[test]
    fn test_map_literal() {
        let input = r#"{"key": 42}"#;
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::OpenBrace);
        assert_eq!(tokens[1].kind, TokenType::String);
        assert_eq!(tokens[2].kind, TokenType::Colon);
        assert_eq!(tokens[3].kind, TokenType::Number);
        assert_eq!(tokens[4].kind, TokenType::CloseBrace);
    }

    #[test]
    fn test_function_declaration() {
        let input = "fn add(x: Int, y: Int) -> Int { return x + y; }";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[0].kind, TokenType::Function);
        assert_eq!(tokens[1].kind, TokenType::Identifier);
        assert_eq!(tokens[1].value, "add");
        assert_eq!(tokens[2].kind, TokenType::OpenParen);
    }

    #[test]
    fn test_type_annotations() {
        let input = "let x: Int = 42; let s: Str = \"hi\"; let b: Bool = true;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens[2].kind, TokenType::Colon);
        assert_eq!(tokens[3].kind, TokenType::Identifier);
        assert_eq!(tokens[3].value, "Int");
    }

    #[test]
    fn test_whitespace_only() {
        let input = "    \t\n  ";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(
            tokens.len(),
            0,
            "Whitespace-only input should produce no tokens"
        );
    }

    #[test]
    fn test_empty_input() {
        let input = "";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert_eq!(tokens.len(), 0, "Empty input should produce no tokens");
    }

    #[test]
    fn test_long_identifier() {
        let input = "let thisIsAVeryLongIdentifierName123 = 1;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        assert!(tokens
            .iter()
            .any(|t| t.value == "thisIsAVeryLongIdentifierName123"));
    }

    #[test]
    fn test_multiple_semicolons() {
        let input = "let x = 1;;;;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let semi_count = tokens.iter().filter(|t| t.kind == TokenType::Semi).count();
        assert_eq!(semi_count, 4);
    }

    #[test]
    fn test_comments_ignored() {
        let input = "let x = 42; // this is a comment\nlet y = 10;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        // Comments should be filtered out
        let expected_kinds = [
            TokenType::Let,
            TokenType::Identifier,
            TokenType::Eq,
            TokenType::Number,
            TokenType::Semi,
            TokenType::Let,
            TokenType::Identifier,
            TokenType::Eq,
            TokenType::Number,
            TokenType::Semi,
        ];
        assert_eq!(
            tokens.iter().map(|t| t.kind).collect::<Vec<_>>(),
            expected_kinds
        );
    }

    // =====================
    // Invalid tests
    // =====================

    #[test]
    fn test_invalid_token() {
        let input = "@";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        // Should produce an Unknown token or similar for invalid character
        let has_unknown = tokens.iter().any(|t| matches!(t.kind, TokenType::Unknown));
        assert!(
            has_unknown,
            "Lexer should produce Unknown token for invalid input"
        );
    }

    #[test]
    fn test_invalid_number() {
        let input = "123abc";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        // Should produce a Number token followed by Identifier or Unknown
        let has_number = tokens.iter().any(|t| t.kind == TokenType::Number);
        let has_identifier = tokens.iter().any(|t| t.kind == TokenType::Identifier);
        assert!(has_number, "Lexer should produce Number token");
        assert!(
            has_identifier,
            "Lexer should produce Identifier token after invalid number"
        );
    }

    #[test]
    fn test_unterminated_string() {
        let input = "let s = \"unterminated;";
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        // Should not produce a String token for unterminated string
        let has_string = tokens.iter().any(|t| t.kind == TokenType::String);
        assert!(
            !has_string,
            "Lexer should not produce String token for unterminated string"
        );
    }
}
