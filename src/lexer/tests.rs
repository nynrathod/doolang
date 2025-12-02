//! Lexer Unit Tests
//! Tests token generation in isolation - NO parser/analyzer/mir/codegen dependencies

#[cfg(test)]
mod lexer_tests {
    use crate::lexer::lexer::lex;
    use crate::lexer::token::{Token, TokenType};
    use bumpalo::Bump;

    fn lex_and_check<F>(input: &str, check: F)
    where
        F: FnOnce(&[Token]),
    {
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        check(&tokens);
    }

    fn has_token_kind(tokens: &[Token], token_type: TokenType) -> bool {
        tokens.iter().any(|t| t.kind == token_type)
    }

    fn count_token_kind(tokens: &[Token], token_type: TokenType) -> usize {
        tokens.iter().filter(|t| t.kind == token_type).count()
    }

    fn first_token_is(input: &str, expected: TokenType) {
        lex_and_check(input, |tokens| {
            assert!(!tokens.is_empty(), "Expected tokens but got empty");
            assert_eq!(
                tokens[0].kind, expected,
                "Expected {:?} but got {:?}",
                expected, tokens[0].kind
            );
        });
    }

    // ========================================
    // KEYWORDS
    // ========================================

    #[test]
    fn test_keywords() {
        // Common list of all keywords and their TokenType
        let keywords: &[(&str, TokenType)] = &[
            ("let", TokenType::Let),
            ("mut", TokenType::Mut),
            ("fn", TokenType::Function),
            ("import", TokenType::Import),
            ("as", TokenType::As),
            ("struct", TokenType::Struct),
            ("enum", TokenType::Enum),
            ("if", TokenType::If),
            ("else", TokenType::Else),
            ("for", TokenType::For),
            ("in", TokenType::In),
            ("return", TokenType::Return),
            ("break", TokenType::Break),
            ("continue", TokenType::Continue),
            ("print", TokenType::Print),
            ("Ok", TokenType::Ok),
            ("Err", TokenType::Err),
            ("nil", TokenType::Nil),
            ("match", TokenType::Match),
        ];

        // Check each keyword individually
        for &(kw, kind) in keywords {
            first_token_is(kw, kind);
        }

        // Check that all keywords together are not identifiers
        let all_keywords = keywords
            .iter()
            .map(|(kw, _)| *kw)
            .collect::<Vec<_>>()
            .join(" ");
        let all_keywords_with_bools = format!("{} true false", all_keywords);

        lex_and_check(&all_keywords_with_bools, |tokens| {
            assert_eq!(count_token_kind(tokens, TokenType::Identifier), 0);
        });
    }

    #[test]
    fn test_boolean_literals() {
        lex_and_check("true false", |tokens| {
            assert!(has_token_kind(tokens, TokenType::Boolean));
            assert_eq!(count_token_kind(tokens, TokenType::Boolean), 2);
        });
    }

    // ========================================
    // IDENTIFIERS
    // ========================================

    #[test]
    fn test_identifiers() {
        let identifiers = ["foo", "bar123", "VarName", "x"];

        // Check each identifier individually
        for &ident in &identifiers {
            first_token_is(ident, TokenType::Identifier);
        }

        // Check all identifiers together
        let all_idents = identifiers.join(" ");
        lex_and_check(&all_idents, |tokens| {
            assert_eq!(
                count_token_kind(tokens, TokenType::Identifier),
                identifiers.len()
            );
        });
    }

    #[test]
    fn test_data_types() {
        lex_and_check(
            "
            let a: Int = 10;
            let a: [Int] = [10, 20];
            let a: {Str: Int} = {\"key\": 10};
            let a: Bool = true;
            let a: Float = 3.14;
            ",
            |tokens| {
                assert!(has_token_kind(tokens, TokenType::Identifier));
                assert!(has_token_kind(tokens, TokenType::Colon));
            },
        );
    }

    #[test]
    fn test_import_typeof_identifier() {
        lex_and_check("typeOf(a);", |tokens| {
            assert!(has_token_kind(tokens, TokenType::Identifier));
        });
    }

    // ========================================
    // NUMBERS
    // ========================================

    #[test]
    fn test_number_literals() {
        first_token_is("0", TokenType::Number);
        first_token_is("42", TokenType::Number);
        first_token_is("00042", TokenType::Number);
        first_token_is("2147483647", TokenType::Number);

        first_token_is("3.14", TokenType::Float);
        first_token_is("0.5", TokenType::Float);
        first_token_is("123.456", TokenType::Float);
        first_token_is("0.0", TokenType::Float);
        first_token_is("1.0", TokenType::Float);
        first_token_is("0.123456789", TokenType::Float);
    }

    #[test]
    fn test_negative_numbers() {
        lex_and_check("-42", |tokens| {
            assert!(has_token_kind(tokens, TokenType::Minus));
            assert!(has_token_kind(tokens, TokenType::Number));
        });
    }

    // ========================================
    // STRINGS
    // ========================================

    #[test]
    fn test_string_literals() {
        first_token_is(r#""hello""#, TokenType::String);

        first_token_is(r#""""#, TokenType::String);
        first_token_is(r#""hello world""#, TokenType::String);
        first_token_is(r#""hello\nworld""#, TokenType::String);
        first_token_is(r#""tab\there""#, TokenType::String);
        first_token_is(r#""say \"hello\"""#, TokenType::String);
        first_token_is(r#""test123""#, TokenType::String);
    }

    // ========================================
    // OPERATORS - ARITHMETIC
    // ========================================

    #[test]
    fn test_arithmetic_operators() {
        first_token_is("+", TokenType::Plus);
        first_token_is("-", TokenType::Minus);
        first_token_is("*", TokenType::Star);
        first_token_is("/", TokenType::Slash);
        first_token_is("%", TokenType::Percent);
    }

    #[test]
    fn test_arithmetic_expression() {
        lex_and_check("1 + 2 - 3 * 4 / 5", |tokens| {
            assert!(has_token_kind(tokens, TokenType::Plus));
            assert!(has_token_kind(tokens, TokenType::Minus));
            assert!(has_token_kind(tokens, TokenType::Star));
            assert!(has_token_kind(tokens, TokenType::Slash));
            assert_eq!(count_token_kind(tokens, TokenType::Number), 5);
        });
    }

    // ========================================
    // OPERATORS - COMPARISON
    // ========================================

    #[test]
    fn test_comparison_operators() {
        first_token_is("==", TokenType::EqEq);
        first_token_is("!=", TokenType::NotEq);
        first_token_is("<", TokenType::Lt);
        first_token_is(">", TokenType::Gt);
        first_token_is("<=", TokenType::LtEq);
        first_token_is(">=", TokenType::GtEq);
    }

    #[test]
    fn test_comparison_expression() {
        lex_and_check("x == y != z < a", |tokens| {
            assert!(has_token_kind(tokens, TokenType::EqEq));
            assert!(has_token_kind(tokens, TokenType::NotEq));
            assert!(has_token_kind(tokens, TokenType::Lt));
        });
    }

    // ========================================
    // OPERATORS - LOGICAL
    // ========================================

    #[test]
    fn test_logical_operators() {
        first_token_is("&&", TokenType::AndAnd);
        first_token_is("||", TokenType::OrOr);
        first_token_is("!", TokenType::Bang);
    }

    // ========================================
    // OPERATORS - ASSIGNMENT
    // ========================================

    #[test]
    fn test_assignment_operators() {
        first_token_is("=", TokenType::Eq);
        first_token_is("+=", TokenType::PlusEq);
        first_token_is("-=", TokenType::MinusEq);
        first_token_is("*=", TokenType::StarEq);
        first_token_is("/=", TokenType::SlashEq);
        first_token_is("%=", TokenType::PercentEq);
        first_token_is("++", TokenType::PlusPlus);
        first_token_is("--", TokenType::MinusMinus);
    }

    #[test]
    fn test_compound_assignment_expression() {
        lex_and_check("x += 1; y -= 2; z *= 3; p /= 4; q %= 5; ", |tokens| {
            assert!(has_token_kind(tokens, TokenType::PlusEq));
            assert!(has_token_kind(tokens, TokenType::MinusEq));
            assert!(has_token_kind(tokens, TokenType::StarEq));
            assert!(has_token_kind(tokens, TokenType::SlashEq));
            assert!(has_token_kind(tokens, TokenType::PercentEq));
        });
    }

    // ========================================
    // Symbols
    // ========================================

    #[test]
    fn test_symbols() {
        first_token_is("&", TokenType::At);
        first_token_is("$", TokenType::Dollar);
    }

    // ========================================
    // DELIMITERS
    // ========================================
    /// Test delimiters: parentheses, braces, brackets
    #[test]
    fn test_delimiters() {
        // Parentheses
        first_token_is("(", TokenType::OpenParen);
        first_token_is(")", TokenType::CloseParen);

        // Braces
        first_token_is("{", TokenType::OpenBrace);
        first_token_is("}", TokenType::CloseBrace);

        // Brackets
        first_token_is("[", TokenType::OpenBracket);
        first_token_is("]", TokenType::CloseBracket);
    }

    #[test]
    fn test_punctuation() {
        first_token_is(";", TokenType::Semi);
        first_token_is(",", TokenType::Comma);
        first_token_is(":", TokenType::Colon);
        first_token_is(".", TokenType::Dot);
    }

    #[test]
    fn test_delimiters_in_expression() {
        lex_and_check("(a, b, c)", |tokens| {
            assert_eq!(count_token_kind(tokens, TokenType::OpenParen), 1);
            assert_eq!(count_token_kind(tokens, TokenType::CloseParen), 1);
            assert_eq!(count_token_kind(tokens, TokenType::Comma), 2);
        });
    }

    // ========================================
    // SPECIAL OPERATORS
    // ========================================

    #[test]
    fn test_arrow_operators() {
        first_token_is("->", TokenType::Arrow);
        first_token_is("=>", TokenType::FatArrow);
    }

    #[test]
    fn test_range_operators() {
        first_token_is("..", TokenType::RangeExc);
        first_token_is("..=", TokenType::RangeInc);
        first_token_is("...", TokenType::Spread);
    }

    #[test]
    fn test_range_in_expression() {
        lex_and_check("0..10", |tokens| {
            assert!(has_token_kind(tokens, TokenType::RangeExc));
        });
        lex_and_check("0..=10", |tokens| {
            assert!(has_token_kind(tokens, TokenType::RangeInc));
        });
    }

    // ========================================
    // WHITESPACE & COMMENTS
    // ========================================

    #[test]
    fn test_whitespace_ignored() {
        let arena1 = Bump::new();
        let tokens1 = lex("let x = 1;", &arena1);
        let kinds1: Vec<_> = tokens1.iter().map(|t| t.kind).collect();

        let arena2 = Bump::new();
        let tokens2 = lex("let    x    =    1   ;", &arena2);
        let kinds2: Vec<_> = tokens2.iter().map(|t| t.kind).collect();

        let arena3 = Bump::new();
        let tokens3 = lex("let\tx\t=\t1\t;", &arena3);
        let kinds3: Vec<_> = tokens3.iter().map(|t| t.kind).collect();

        // All should produce the same token sequence
        assert_eq!(kinds1, kinds2);
        assert_eq!(kinds1, kinds3);
    }

    #[test]
    fn test_newlines_ignored() {
        lex_and_check("let\nx\n=\n1\n;", |tokens| {
            assert!(has_token_kind(tokens, TokenType::Let));
            assert!(has_token_kind(tokens, TokenType::Eq));
        });
    }

    // ========================================
    // COMPLEX EXPRESSIONS
    // ========================================

    #[test]
    fn test_function_definition() {
        lex_and_check("fn add(x, y) { return x + y; }", |tokens| {
            assert!(has_token_kind(tokens, TokenType::Function));
            assert!(has_token_kind(tokens, TokenType::Identifier));
            assert!(has_token_kind(tokens, TokenType::Return));
            assert!(has_token_kind(tokens, TokenType::OpenBrace));
            assert!(has_token_kind(tokens, TokenType::CloseBrace));
        });
    }

    #[test]
    fn test_array_literal() {
        lex_and_check("[1, 2, 3]", |tokens| {
            assert_eq!(count_token_kind(tokens, TokenType::OpenBracket), 1);
            assert_eq!(count_token_kind(tokens, TokenType::CloseBracket), 1);
            assert_eq!(count_token_kind(tokens, TokenType::Comma), 2);
        });
    }

    #[test]
    fn test_map_literal() {
        lex_and_check(r#"{"key": 42}"#, |tokens| {
            assert!(has_token_kind(tokens, TokenType::OpenBrace));
            assert!(has_token_kind(tokens, TokenType::CloseBrace));
            assert!(has_token_kind(tokens, TokenType::String));
            assert!(has_token_kind(tokens, TokenType::Colon));
        });
    }

    #[test]
    fn test_if_statement() {
        lex_and_check("if x > 0 { }", |tokens| {
            assert!(has_token_kind(tokens, TokenType::If));
            assert!(has_token_kind(tokens, TokenType::Gt));
        });
    }

    #[test]
    fn test_for_loop() {
        lex_and_check("for i in 0..10 { } for i in 0..=10 { }", |tokens| {
            assert!(has_token_kind(tokens, TokenType::For));
            assert!(has_token_kind(tokens, TokenType::In));
            assert!(has_token_kind(tokens, TokenType::RangeExc));
            assert!(has_token_kind(tokens, TokenType::RangeInc));
        });
    }

    #[test]
    fn test_lambda_expression() {
        lex_and_check("(x) => x * 2", |tokens| {
            assert!(has_token_kind(tokens, TokenType::OpenParen));
            assert!(has_token_kind(tokens, TokenType::CloseParen));
            assert!(has_token_kind(tokens, TokenType::FatArrow));
            assert!(has_token_kind(tokens, TokenType::Star));
        });
    }

    // ========================================
    // MIXED TYPES
    // ========================================

    #[test]
    fn test_int_and_float_in_expression() {
        lex_and_check("10 + 5.5", |tokens| {
            assert!(has_token_kind(tokens, TokenType::Number));
            assert!(has_token_kind(tokens, TokenType::Float));
            assert!(has_token_kind(tokens, TokenType::Plus));
        });
    }

    #[test]
    fn test_string_concatenation() {
        lex_and_check(r#""hello" + " world""#, |tokens| {
            assert_eq!(count_token_kind(tokens, TokenType::String), 2);
            assert!(has_token_kind(tokens, TokenType::Plus));
        });
    }

    #[test]
    fn test_string_with_number() {
        lex_and_check(r#""value: " + 42"#, |tokens| {
            assert!(has_token_kind(tokens, TokenType::String));
            assert!(has_token_kind(tokens, TokenType::Number));
        });
    }

    // ========================================
    // COMPREHENSIVE PROGRAM
    // ========================================

    #[test]
    fn test_complete_function() {
        let code = r#"
            fn factorial(n) {
                if n <= 1 {
                    return 1;
                }
                return n * factorial(n - 1);
            }
        "#;
        lex_and_check(code, |tokens| {
            assert!(has_token_kind(tokens, TokenType::Function));
            assert!(has_token_kind(tokens, TokenType::If));
            assert!(has_token_kind(tokens, TokenType::Return));
            assert!(has_token_kind(tokens, TokenType::LtEq));
            assert!(has_token_kind(tokens, TokenType::Minus));
            assert!(has_token_kind(tokens, TokenType::Star));
        });
    }

    #[test]
    fn test_variable_declaration() {
        lex_and_check("let x = 42;", |tokens| {
            assert_eq!(tokens[0].kind, TokenType::Let);
            assert_eq!(tokens[1].kind, TokenType::Identifier);
            assert_eq!(tokens[2].kind, TokenType::Eq);
            assert_eq!(tokens[3].kind, TokenType::Number);
            assert_eq!(tokens[4].kind, TokenType::Semi);
        });
    }

    #[test]
    fn test_variable_type_cast() {
        lex_and_check("let x = data as Int;", |tokens| {
            assert!(has_token_kind(tokens, TokenType::As));
        });
    }

    #[test]
    fn test_array_access() {
        lex_and_check("arr[0]", |tokens| {
            assert!(has_token_kind(tokens, TokenType::Identifier));
            assert!(has_token_kind(tokens, TokenType::OpenBracket));
            assert!(has_token_kind(tokens, TokenType::Number));
            assert!(has_token_kind(tokens, TokenType::CloseBracket));
        });
    }

    #[test]
    fn test_map_access() {
        lex_and_check(r#"map.get("key")"#, |tokens| {
            assert!(has_token_kind(tokens, TokenType::Identifier));
            assert!(has_token_kind(tokens, TokenType::Dot));
            assert!(has_token_kind(tokens, TokenType::Identifier));
            assert!(has_token_kind(tokens, TokenType::OpenParen));
            assert!(has_token_kind(tokens, TokenType::String));
            assert!(has_token_kind(tokens, TokenType::CloseParen));
        });
    }

    #[test]
    fn test_array_update() {
        lex_and_check("arr[0] = 42", |tokens| {
            assert!(has_token_kind(tokens, TokenType::Identifier));
            assert!(has_token_kind(tokens, TokenType::OpenBracket));
            assert!(has_token_kind(tokens, TokenType::Number));
            assert!(has_token_kind(tokens, TokenType::CloseBracket));
            assert!(has_token_kind(tokens, TokenType::Eq));
            assert!(has_token_kind(tokens, TokenType::Number));
        });
    }

    // ========================================
    // TOKEN COUNTS
    // ========================================

    #[test]
    fn test_token_count_simple() {
        lex_and_check("let x = 42;", |tokens| {
            assert_eq!(tokens.len(), 5);
        });
    }

    #[test]
    fn test_token_count_expression() {
        lex_and_check("1 + 2 * 3", |tokens| {
            assert_eq!(tokens.len(), 5);
        });
    }

    // =====================================================================
    // IMPORTS
    // =====================================================================

    #[test]
    fn test_import_single() {
        lex_and_check("import std::Math::Abs;", |tokens| {
            assert!(has_token_kind(tokens, TokenType::Import));
            assert!(has_token_kind(tokens, TokenType::Identifier));
            assert!(has_token_kind(tokens, TokenType::Semi));
        });
    }

    #[test]
    fn test_import_multiple() {
        lex_and_check("import std::Math::*;", |tokens| {
            assert!(has_token_kind(tokens, TokenType::Star));
        });
    }

    #[test]
    fn test_import_aliased() {
        lex_and_check("import std::Math::{Abs as AbsValue};", |tokens| {
            assert!(has_token_kind(tokens, TokenType::As));
        });
    }
}
