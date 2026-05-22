//! Comprehensive lexer unit tests for the Doo compiler frontend.
//!
//! Tests cover: keywords, identifiers, integers, floats, strings, operators,
//! delimiters, punctuation, comments, whitespace, complex expressions,
//! error handling, edge cases, and TokenKind methods.

use doo_frontend::lexer::{Lexer, Token, TokenKind};

// ============================================================================
// Helper functions
// ============================================================================

fn lex(source: &str) -> Vec<Token> {
    let mut lexer = Lexer::new(source, 0);
    lexer.tokenize()
}

fn lex_kinds(source: &str) -> Vec<TokenKind> {
    lex(source)
        .into_iter()
        .map(|t| t.kind)
        .filter(|k| *k != TokenKind::Eof)
        .collect()
}

fn lex_first(source: &str) -> Token {
    let tokens = lex(source);
    tokens.into_iter().next().unwrap()
}

fn lex_texts(source: &str) -> Vec<String> {
    lex(source)
        .into_iter()
        .filter(|t| t.kind != TokenKind::Eof)
        .map(|t| t.text)
        .collect()
}

// ============================================================================
// 1. Keywords (~21 tests)
// ============================================================================

#[test]
fn test_lex_keyword_let() {
    let t = lex_first("let");
    assert_eq!(t.kind, TokenKind::Let);
    assert_eq!(t.text, "let");
}

#[test]
fn test_lex_keyword_mut() {
    let t = lex_first("mut");
    assert_eq!(t.kind, TokenKind::Mut);
    assert_eq!(t.text, "mut");
}

#[test]
fn test_lex_keyword_fn() {
    let t = lex_first("fn");
    assert_eq!(t.kind, TokenKind::Fn);
    assert_eq!(t.text, "fn");
}

#[test]
fn test_lex_keyword_import() {
    let t = lex_first("import");
    assert_eq!(t.kind, TokenKind::Import);
    assert_eq!(t.text, "import");
}

#[test]
fn test_lex_keyword_as() {
    let t = lex_first("as");
    assert_eq!(t.kind, TokenKind::As);
    assert_eq!(t.text, "as");
}

#[test]
fn test_lex_keyword_struct() {
    let t = lex_first("struct");
    assert_eq!(t.kind, TokenKind::Struct);
    assert_eq!(t.text, "struct");
}

#[test]
fn test_lex_keyword_enum() {
    let t = lex_first("enum");
    assert_eq!(t.kind, TokenKind::Enum);
    assert_eq!(t.text, "enum");
}

#[test]
fn test_lex_keyword_if() {
    let t = lex_first("if");
    assert_eq!(t.kind, TokenKind::If);
    assert_eq!(t.text, "if");
}

#[test]
fn test_lex_keyword_else() {
    let t = lex_first("else");
    assert_eq!(t.kind, TokenKind::Else);
    assert_eq!(t.text, "else");
}

#[test]
fn test_lex_keyword_for() {
    let t = lex_first("for");
    assert_eq!(t.kind, TokenKind::For);
    assert_eq!(t.text, "for");
}

#[test]
fn test_lex_keyword_in() {
    let t = lex_first("in");
    assert_eq!(t.kind, TokenKind::In);
    assert_eq!(t.text, "in");
}

#[test]
fn test_lex_keyword_return() {
    let t = lex_first("return");
    assert_eq!(t.kind, TokenKind::Return);
    assert_eq!(t.text, "return");
}

#[test]
fn test_lex_keyword_break() {
    let t = lex_first("break");
    assert_eq!(t.kind, TokenKind::Break);
    assert_eq!(t.text, "break");
}

#[test]
fn test_lex_keyword_continue() {
    let t = lex_first("continue");
    assert_eq!(t.kind, TokenKind::Continue);
    assert_eq!(t.text, "continue");
}

#[test]
fn test_lex_keyword_print() {
    let t = lex_first("print");
    assert_eq!(t.kind, TokenKind::Print);
    assert_eq!(t.text, "print");
}

#[test]
fn test_lex_keyword_ok() {
    let t = lex_first("Ok");
    assert_eq!(t.kind, TokenKind::Ok);
    assert_eq!(t.text, "Ok");
}

#[test]
fn test_lex_keyword_err() {
    let t = lex_first("Err");
    assert_eq!(t.kind, TokenKind::Err);
    assert_eq!(t.text, "Err");
}

#[test]
fn test_lex_keyword_nil() {
    let t = lex_first("nil");
    assert_eq!(t.kind, TokenKind::Nil);
    assert_eq!(t.text, "nil");
}

#[test]
fn test_lex_keyword_match() {
    let t = lex_first("match");
    assert_eq!(t.kind, TokenKind::Match);
    assert_eq!(t.text, "match");
}

#[test]
fn test_lex_keyword_true() {
    let t = lex_first("true");
    assert_eq!(t.kind, TokenKind::True);
    assert_eq!(t.text, "true");
}

#[test]
fn test_lex_keyword_false() {
    let t = lex_first("false");
    assert_eq!(t.kind, TokenKind::False);
    assert_eq!(t.text, "false");
}

// ============================================================================
// 2. Identifiers (~25 tests)
// ============================================================================

#[test]
fn test_lex_ident_simple() {
    let t = lex_first("foo");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "foo");
}

#[test]
fn test_lex_ident_camel_case() {
    let t = lex_first("myVariable");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "myVariable");
}

#[test]
fn test_lex_ident_snake_case() {
    let t = lex_first("my_variable");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "my_variable");
}

#[test]
fn test_lex_ident_pascal_case() {
    let t = lex_first("MyStruct");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "MyStruct");
}

#[test]
fn test_lex_ident_with_numbers() {
    let t = lex_first("x1");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "x1");
}

#[test]
fn test_lex_ident_with_trailing_numbers() {
    let t = lex_first("item42");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "item42");
}

#[test]
fn test_lex_ident_single_char() {
    let t = lex_first("x");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "x");
}

#[test]
fn test_lex_ident_single_uppercase() {
    let t = lex_first("T");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "T");
}

#[test]
fn test_lex_ident_long_name() {
    let t = lex_first("thisIsAVeryLongIdentifierNameForTesting");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "thisIsAVeryLongIdentifierNameForTesting");
}

#[test]
fn test_lex_ident_starts_with_underscore() {
    let t = lex_first("_private");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "_private");
}

#[test]
fn test_lex_ident_double_underscore_prefix() {
    let t = lex_first("__internal");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "__internal");
}

#[test]
fn test_lex_ident_underscore_only_is_wildcard() {
    let t = lex_first("_");
    assert_eq!(t.kind, TokenKind::Underscore);
    assert_eq!(t.text, "_");
}

#[test]
fn test_lex_ident_keyword_prefix_letter() {
    // "letter" starts with "let" but should be an identifier
    let t = lex_first("letter");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "letter");
}

#[test]
fn test_lex_ident_keyword_prefix_format() {
    // "format" starts with "for" but should be an identifier
    let t = lex_first("format");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "format");
}

#[test]
fn test_lex_ident_keyword_prefix_import_ant() {
    let t = lex_first("important");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "important");
}

#[test]
fn test_lex_ident_keyword_prefix_returns() {
    let t = lex_first("returns");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "returns");
}

#[test]
fn test_lex_ident_keyword_prefix_matching() {
    let t = lex_first("matching");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "matching");
}

#[test]
fn test_lex_ident_keyword_prefix_structural() {
    let t = lex_first("structural");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "structural");
}

#[test]
fn test_lex_ident_mixed_case_not_keyword() {
    // "Let" (uppercase L) is not a keyword, it is an identifier
    let t = lex_first("Let");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "Let");
}

#[test]
fn test_lex_ident_all_uppercase_not_keyword() {
    let t = lex_first("TRUE");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "TRUE");
}

#[test]
fn test_lex_ident_all_uppercase_fn() {
    let t = lex_first("FN");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "FN");
}

#[test]
fn test_lex_ident_numbers_in_middle() {
    let t = lex_first("a1b2c3");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "a1b2c3");
}

#[test]
fn test_lex_ident_multiple_underscores() {
    let t = lex_first("a__b__c");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "a__b__c");
}

#[test]
fn test_lex_ident_trailing_underscore() {
    let t = lex_first("value_");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "value_");
}

#[test]
fn test_lex_ident_ok_lowercase_not_keyword() {
    let t = lex_first("ok");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.text, "ok");
}

// ============================================================================
// 3. Integer literals (~20 tests)
// ============================================================================

#[test]
fn test_lex_int_zero() {
    let t = lex_first("0");
    assert_eq!(t.kind, TokenKind::Integer);
    assert_eq!(t.text, "0");
}

#[test]
fn test_lex_int_single_digit() {
    let t = lex_first("5");
    assert_eq!(t.kind, TokenKind::Integer);
    assert_eq!(t.text, "5");
}

#[test]
fn test_lex_int_two_digits() {
    let t = lex_first("42");
    assert_eq!(t.kind, TokenKind::Integer);
    assert_eq!(t.text, "42");
}

#[test]
fn test_lex_int_three_digits() {
    let t = lex_first("100");
    assert_eq!(t.kind, TokenKind::Integer);
    assert_eq!(t.text, "100");
}

#[test]
fn test_lex_int_large_number() {
    let t = lex_first("9999999");
    assert_eq!(t.kind, TokenKind::Integer);
    assert_eq!(t.text, "9999999");
}

#[test]
fn test_lex_int_max_i64_like() {
    let t = lex_first("9223372036854775807");
    assert_eq!(t.kind, TokenKind::Integer);
    assert_eq!(t.text, "9223372036854775807");
}

#[test]
fn test_lex_int_leading_zeros() {
    let t = lex_first("007");
    assert_eq!(t.kind, TokenKind::Integer);
    assert_eq!(t.text, "007");
}

#[test]
fn test_lex_int_negative_is_minus_plus_int() {
    let kinds = lex_kinds("-42");
    assert_eq!(kinds, vec![TokenKind::Minus, TokenKind::Integer]);
}

#[test]
fn test_lex_int_negative_text() {
    let tokens = lex("-42");
    assert_eq!(tokens[0].text, "-");
    assert_eq!(tokens[1].text, "42");
}

#[test]
fn test_lex_int_multiple_integers() {
    let kinds = lex_kinds("1 2 3");
    assert_eq!(
        kinds,
        vec![TokenKind::Integer, TokenKind::Integer, TokenKind::Integer]
    );
}

#[test]
fn test_lex_int_followed_by_ident() {
    let kinds = lex_kinds("123abc");
    // The lexer scans 123 as integer, then abc as ident
    assert_eq!(kinds, vec![TokenKind::Integer, TokenKind::Ident]);
}

#[test]
fn test_lex_int_all_digits() {
    let t = lex_first("1234567890");
    assert_eq!(t.kind, TokenKind::Integer);
    assert_eq!(t.text, "1234567890");
}

#[test]
fn test_lex_int_before_dot_dot_is_not_float() {
    // 1..10 should be Int DotDot Int, not Float
    let kinds = lex_kinds("1..10");
    assert_eq!(
        kinds,
        vec![TokenKind::Integer, TokenKind::DotDot, TokenKind::Integer]
    );
}

#[test]
fn test_lex_int_before_dot_dot_eq_is_not_float() {
    let kinds = lex_kinds("1..=10");
    assert_eq!(
        kinds,
        vec![TokenKind::Integer, TokenKind::DotDotEq, TokenKind::Integer]
    );
}

#[test]
fn test_lex_int_in_expression() {
    let kinds = lex_kinds("1+2");
    assert_eq!(
        kinds,
        vec![TokenKind::Integer, TokenKind::Plus, TokenKind::Integer]
    );
}

#[test]
fn test_lex_int_with_parens() {
    let kinds = lex_kinds("(42)");
    assert_eq!(
        kinds,
        vec![TokenKind::LParen, TokenKind::Integer, TokenKind::RParen]
    );
}

#[test]
fn test_lex_int_very_long() {
    let t = lex_first("99999999999999999999999999999999");
    assert_eq!(t.kind, TokenKind::Integer);
    assert_eq!(t.text, "99999999999999999999999999999999");
}

#[test]
fn test_lex_int_followed_by_semi() {
    let kinds = lex_kinds("42;");
    assert_eq!(kinds, vec![TokenKind::Integer, TokenKind::Semi]);
}

#[test]
fn test_lex_int_before_dot_ident_is_not_float() {
    // "1.method" - dot followed by non-digit is not float
    let kinds = lex_kinds("1.method");
    assert_eq!(
        kinds,
        vec![TokenKind::Integer, TokenKind::Dot, TokenKind::Ident]
    );
}

#[test]
fn test_lex_int_multiple_zeros() {
    let t = lex_first("000");
    assert_eq!(t.kind, TokenKind::Integer);
    assert_eq!(t.text, "000");
}

// ============================================================================
// 4. Float literals (~20 tests)
// ============================================================================

#[test]
fn test_lex_float_simple() {
    let t = lex_first("3.14");
    assert_eq!(t.kind, TokenKind::Float);
    assert_eq!(t.text, "3.14");
}

#[test]
fn test_lex_float_one_point_zero() {
    let t = lex_first("1.0");
    assert_eq!(t.kind, TokenKind::Float);
    assert_eq!(t.text, "1.0");
}

#[test]
fn test_lex_float_zero_point_five() {
    let t = lex_first("0.5");
    assert_eq!(t.kind, TokenKind::Float);
    assert_eq!(t.text, "0.5");
}

#[test]
fn test_lex_float_zero_point_zero() {
    let t = lex_first("0.0");
    assert_eq!(t.kind, TokenKind::Float);
    assert_eq!(t.text, "0.0");
}

#[test]
fn test_lex_float_leading_zero() {
    let t = lex_first("0.123");
    assert_eq!(t.kind, TokenKind::Float);
    assert_eq!(t.text, "0.123");
}

#[test]
fn test_lex_float_many_decimals() {
    let t = lex_first("3.14159265358979");
    assert_eq!(t.kind, TokenKind::Float);
    assert_eq!(t.text, "3.14159265358979");
}

#[test]
fn test_lex_float_scientific_lowercase() {
    let t = lex_first("1e10");
    assert_eq!(t.kind, TokenKind::Float);
    assert_eq!(t.text, "1e10");
}

#[test]
fn test_lex_float_scientific_uppercase() {
    let t = lex_first("1E10");
    assert_eq!(t.kind, TokenKind::Float);
    assert_eq!(t.text, "1E10");
}

#[test]
fn test_lex_float_scientific_with_decimal() {
    let t = lex_first("2.5e3");
    assert_eq!(t.kind, TokenKind::Float);
    assert_eq!(t.text, "2.5e3");
}

#[test]
fn test_lex_float_scientific_negative_exp() {
    let t = lex_first("2.5e-3");
    assert_eq!(t.kind, TokenKind::Float);
    assert_eq!(t.text, "2.5e-3");
}

#[test]
fn test_lex_float_scientific_positive_exp() {
    let t = lex_first("2.5e+3");
    assert_eq!(t.kind, TokenKind::Float);
    assert_eq!(t.text, "2.5e+3");
}

#[test]
fn test_lex_float_scientific_zero_exp() {
    let t = lex_first("1e0");
    assert_eq!(t.kind, TokenKind::Float);
    assert_eq!(t.text, "1e0");
}

#[test]
fn test_lex_float_large_value() {
    let t = lex_first("999999.999999");
    assert_eq!(t.kind, TokenKind::Float);
    assert_eq!(t.text, "999999.999999");
}

#[test]
fn test_lex_float_small_value() {
    let t = lex_first("0.000001");
    assert_eq!(t.kind, TokenKind::Float);
    assert_eq!(t.text, "0.000001");
}

#[test]
fn test_lex_float_negative_is_minus_float() {
    let kinds = lex_kinds("-3.14");
    assert_eq!(kinds, vec![TokenKind::Minus, TokenKind::Float]);
}

#[test]
fn test_lex_float_in_expression() {
    let kinds = lex_kinds("1.5+2.5");
    assert_eq!(
        kinds,
        vec![TokenKind::Float, TokenKind::Plus, TokenKind::Float]
    );
}

#[test]
fn test_lex_float_scientific_large_exp() {
    let t = lex_first("1e100");
    assert_eq!(t.kind, TokenKind::Float);
    assert_eq!(t.text, "1e100");
}

#[test]
fn test_lex_float_invalid_exp_rewinds() {
    // "1eX" - invalid exponent, should rewind to integer 1 then ident eX
    let kinds = lex_kinds("1eX");
    assert_eq!(kinds, vec![TokenKind::Integer, TokenKind::Ident]);
}

#[test]
fn test_lex_float_double_dot_after_float() {
    // "1.5..2.5" should be Float DotDot Float
    let kinds = lex_kinds("1.5..2.5");
    assert_eq!(
        kinds,
        vec![TokenKind::Float, TokenKind::DotDot, TokenKind::Float]
    );
}

#[test]
fn test_lex_float_two_decimal_parts() {
    let t = lex_first("12.34");
    assert_eq!(t.kind, TokenKind::Float);
    assert_eq!(t.text, "12.34");
}

// ============================================================================
// 5. String literals (~30 tests)
// ============================================================================

#[test]
fn test_lex_string_empty() {
    let t = lex_first(r#""""#);
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text, "");
}

#[test]
fn test_lex_string_simple() {
    let t = lex_first(r#""hello""#);
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text, "hello");
}

#[test]
fn test_lex_string_with_spaces() {
    let t = lex_first(r#""hello world""#);
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text, "hello world");
}

#[test]
fn test_lex_string_single_char() {
    let t = lex_first(r#""a""#);
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text, "a");
}

#[test]
fn test_lex_string_with_numbers() {
    let t = lex_first(r#""abc123""#);
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text, "abc123");
}

#[test]
fn test_lex_string_escape_newline() {
    let t = lex_first(r#""line1\nline2""#);
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text, "line1\nline2");
}

#[test]
fn test_lex_string_escape_tab() {
    let t = lex_first(r#""col1\tcol2""#);
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text, "col1\tcol2");
}

#[test]
fn test_lex_string_escape_backslash() {
    let t = lex_first(r#""path\\file""#);
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text, "path\\file");
}

#[test]
fn test_lex_string_escape_quote() {
    let t = lex_first(r#""say \"hi\"""#);
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text, "say \"hi\"");
}

#[test]
fn test_lex_string_escape_carriage_return() {
    let t = lex_first(r#""line\r""#);
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text, "line\r");
}

#[test]
fn test_lex_string_escape_null() {
    let t = lex_first(r#""null\0char""#);
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text, "null\0char");
}

#[test]
fn test_lex_string_escape_dollar() {
    let t = lex_first(r#""cost \$5""#);
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text, "cost $5");
}

#[test]
fn test_lex_string_multiple_escapes() {
    let t = lex_first(r#""\t\n\\""#);
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text, "\t\n\\");
}

#[test]
fn test_lex_string_template_simple() {
    let t = lex_first(r#""Hello ${name}""#);
    assert_eq!(t.kind, TokenKind::StringTemplate);
}

#[test]
fn test_lex_string_template_text_preserved() {
    let t = lex_first(r#""Hello ${name}!""#);
    assert_eq!(t.kind, TokenKind::StringTemplate);
    assert!(t.text.contains("${name}"));
}

#[test]
fn test_lex_string_template_multiple_interpolations() {
    let t = lex_first(r#""${a} and ${b}""#);
    assert_eq!(t.kind, TokenKind::StringTemplate);
}

#[test]
fn test_lex_string_template_expression() {
    let t = lex_first(r#""result: ${x + y}""#);
    assert_eq!(t.kind, TokenKind::StringTemplate);
}

#[test]
fn test_lex_string_no_interpolation_plain_dollar() {
    // Dollar not followed by { is not interpolation
    let t = lex_first(r#""cost $5""#);
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text, "cost $5");
}

#[test]
fn test_lex_string_unterminated() {
    let t = lex_first(r#""hello"#);
    assert_eq!(t.kind, TokenKind::Error);
}

#[test]
fn test_lex_string_unterminated_text() {
    let t = lex_first(r#""hello"#);
    assert_eq!(t.kind, TokenKind::Error);
    assert!(t.text.contains("Unterminated"));
}

#[test]
fn test_lex_string_invalid_escape() {
    let t = lex_first(r#""bad \q escape""#);
    assert_eq!(t.kind, TokenKind::Error);
    assert!(t.text.contains("Invalid escape"));
}

#[test]
fn test_lex_string_multiline_literal() {
    let t = lex_first("\"line1\nline2\"");
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text, "line1\nline2");
}

#[test]
fn test_lex_string_with_punctuation() {
    let t = lex_first(r#""hello, world!""#);
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text, "hello, world!");
}

#[test]
fn test_lex_string_unicode_escape() {
    let t = lex_first(r#""\u{0041}""#);
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text, "A");
}

#[test]
fn test_lex_string_unicode_escape_emoji() {
    let t = lex_first(r#""\u{1F600}""#);
    assert_eq!(t.kind, TokenKind::String);
    // U+1F600 is the grinning face emoji
    assert_eq!(t.text.chars().count(), 1);
}

#[test]
fn test_lex_string_invalid_unicode_escape() {
    let t = lex_first(r#""\u{ZZZZ}""#);
    assert_eq!(t.kind, TokenKind::Error);
}

#[test]
fn test_lex_string_unicode_escape_no_brace() {
    let t = lex_first(r#""\u0041""#);
    assert_eq!(t.kind, TokenKind::Error);
}

#[test]
fn test_lex_string_consecutive() {
    let kinds = lex_kinds(r#""a" "b""#);
    assert_eq!(kinds, vec![TokenKind::String, TokenKind::String]);
}

#[test]
fn test_lex_string_followed_by_token() {
    let kinds = lex_kinds(r#""hello";"#);
    assert_eq!(kinds, vec![TokenKind::String, TokenKind::Semi]);
}

#[test]
fn test_lex_string_long_content() {
    let long = "a".repeat(500);
    let source = format!(r#""{}""#, long);
    let t = lex_first(&source);
    assert_eq!(t.kind, TokenKind::String);
    assert_eq!(t.text.len(), 500);
}

// ============================================================================
// 6. Operators (~40 tests)
// ============================================================================

#[test]
fn test_lex_op_plus() {
    let t = lex_first("+");
    assert_eq!(t.kind, TokenKind::Plus);
    assert_eq!(t.text, "+");
}

#[test]
fn test_lex_op_minus() {
    let t = lex_first("-");
    assert_eq!(t.kind, TokenKind::Minus);
    assert_eq!(t.text, "-");
}

#[test]
fn test_lex_op_star() {
    let t = lex_first("*");
    assert_eq!(t.kind, TokenKind::Star);
    assert_eq!(t.text, "*");
}

#[test]
fn test_lex_op_slash() {
    // Careful: must not be followed by / or * (comment)
    let kinds = lex_kinds("a / b");
    assert_eq!(
        kinds,
        vec![TokenKind::Ident, TokenKind::Slash, TokenKind::Ident]
    );
}

#[test]
fn test_lex_op_percent() {
    let t = lex_first("%");
    assert_eq!(t.kind, TokenKind::Percent);
    assert_eq!(t.text, "%");
}

#[test]
fn test_lex_op_eq_eq() {
    let t = lex_first("==");
    assert_eq!(t.kind, TokenKind::EqEq);
    assert_eq!(t.text, "==");
}

#[test]
fn test_lex_op_not_eq() {
    let t = lex_first("!=");
    assert_eq!(t.kind, TokenKind::NotEq);
    assert_eq!(t.text, "!=");
}

#[test]
fn test_lex_op_lt() {
    let t = lex_first("<");
    assert_eq!(t.kind, TokenKind::Lt);
    assert_eq!(t.text, "<");
}

#[test]
fn test_lex_op_gt() {
    let t = lex_first(">");
    assert_eq!(t.kind, TokenKind::Gt);
    assert_eq!(t.text, ">");
}

#[test]
fn test_lex_op_lt_eq() {
    let t = lex_first("<=");
    assert_eq!(t.kind, TokenKind::LtEq);
    assert_eq!(t.text, "<=");
}

#[test]
fn test_lex_op_gt_eq() {
    let t = lex_first(">=");
    assert_eq!(t.kind, TokenKind::GtEq);
    assert_eq!(t.text, ">=");
}

#[test]
fn test_lex_op_bang() {
    let t = lex_first("!");
    assert_eq!(t.kind, TokenKind::Bang);
    assert_eq!(t.text, "!");
}

#[test]
fn test_lex_op_and_and() {
    let t = lex_first("&&");
    assert_eq!(t.kind, TokenKind::AndAnd);
    assert_eq!(t.text, "&&");
}

#[test]
fn test_lex_op_or_or() {
    let t = lex_first("||");
    assert_eq!(t.kind, TokenKind::OrOr);
    assert_eq!(t.text, "||");
}

#[test]
fn test_lex_op_and() {
    let t = lex_first("&");
    assert_eq!(t.kind, TokenKind::And);
    assert_eq!(t.text, "&");
}

#[test]
fn test_lex_op_or() {
    let t = lex_first("|");
    assert_eq!(t.kind, TokenKind::Or);
    assert_eq!(t.text, "|");
}

#[test]
fn test_lex_op_eq() {
    let t = lex_first("=");
    assert_eq!(t.kind, TokenKind::Eq);
    assert_eq!(t.text, "=");
}

#[test]
fn test_lex_op_plus_eq() {
    let t = lex_first("+=");
    assert_eq!(t.kind, TokenKind::PlusEq);
    assert_eq!(t.text, "+=");
}

#[test]
fn test_lex_op_minus_eq() {
    let t = lex_first("-=");
    assert_eq!(t.kind, TokenKind::MinusEq);
    assert_eq!(t.text, "-=");
}

#[test]
fn test_lex_op_star_eq() {
    let t = lex_first("*=");
    assert_eq!(t.kind, TokenKind::StarEq);
    assert_eq!(t.text, "*=");
}

#[test]
fn test_lex_op_slash_eq() {
    let t = lex_first("/=");
    assert_eq!(t.kind, TokenKind::SlashEq);
    assert_eq!(t.text, "/=");
}

#[test]
fn test_lex_op_percent_eq() {
    let t = lex_first("%=");
    assert_eq!(t.kind, TokenKind::PercentEq);
    assert_eq!(t.text, "%=");
}

#[test]
fn test_lex_op_plus_plus() {
    let t = lex_first("++");
    assert_eq!(t.kind, TokenKind::PlusPlus);
    assert_eq!(t.text, "++");
}

#[test]
fn test_lex_op_minus_minus() {
    let t = lex_first("--");
    assert_eq!(t.kind, TokenKind::MinusMinus);
    assert_eq!(t.text, "--");
}

#[test]
fn test_lex_op_arrow() {
    let t = lex_first("->");
    assert_eq!(t.kind, TokenKind::Arrow);
    assert_eq!(t.text, "->");
}

#[test]
fn test_lex_op_fat_arrow() {
    let t = lex_first("=>");
    assert_eq!(t.kind, TokenKind::FatArrow);
    assert_eq!(t.text, "=>");
}

#[test]
fn test_lex_op_consecutive_plus_eq() {
    // "+=" should be PlusEq, not Plus then Eq
    let kinds = lex_kinds("+=");
    assert_eq!(kinds, vec![TokenKind::PlusEq]);
}

#[test]
fn test_lex_op_plus_space_eq() {
    // "+ =" should be Plus then Eq
    let kinds = lex_kinds("+ =");
    assert_eq!(kinds, vec![TokenKind::Plus, TokenKind::Eq]);
}

#[test]
fn test_lex_op_minus_vs_arrow() {
    // "->" should be Arrow
    let kinds = lex_kinds("->");
    assert_eq!(kinds, vec![TokenKind::Arrow]);
}

#[test]
fn test_lex_op_minus_vs_minus_minus() {
    // "--" should be MinusMinus
    let kinds = lex_kinds("--");
    assert_eq!(kinds, vec![TokenKind::MinusMinus]);
}

#[test]
fn test_lex_op_minus_vs_minus_eq() {
    // "-=" should be MinusEq
    let kinds = lex_kinds("-=");
    assert_eq!(kinds, vec![TokenKind::MinusEq]);
}

#[test]
fn test_lex_op_eq_vs_eq_eq() {
    let kinds = lex_kinds("= ==");
    assert_eq!(kinds, vec![TokenKind::Eq, TokenKind::EqEq]);
}

#[test]
fn test_lex_op_eq_vs_fat_arrow() {
    let kinds = lex_kinds("= =>");
    assert_eq!(kinds, vec![TokenKind::Eq, TokenKind::FatArrow]);
}

#[test]
fn test_lex_op_bang_vs_not_eq() {
    let kinds = lex_kinds("! !=");
    assert_eq!(kinds, vec![TokenKind::Bang, TokenKind::NotEq]);
}

#[test]
fn test_lex_op_lt_vs_lt_eq() {
    let kinds = lex_kinds("< <=");
    assert_eq!(kinds, vec![TokenKind::Lt, TokenKind::LtEq]);
}

#[test]
fn test_lex_op_gt_vs_gt_eq() {
    let kinds = lex_kinds("> >=");
    assert_eq!(kinds, vec![TokenKind::Gt, TokenKind::GtEq]);
}

#[test]
fn test_lex_op_all_arithmetic_together() {
    let kinds = lex_kinds("+ - * / %");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Plus,
            TokenKind::Minus,
            TokenKind::Star,
            TokenKind::Slash,
            TokenKind::Percent,
        ]
    );
}

#[test]
fn test_lex_op_all_comparison_together() {
    let kinds = lex_kinds("== != < > <= >=");
    assert_eq!(
        kinds,
        vec![
            TokenKind::EqEq,
            TokenKind::NotEq,
            TokenKind::Lt,
            TokenKind::Gt,
            TokenKind::LtEq,
            TokenKind::GtEq,
        ]
    );
}

#[test]
fn test_lex_op_all_assignment_together() {
    let kinds = lex_kinds("= += -= *= /= %=");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Eq,
            TokenKind::PlusEq,
            TokenKind::MinusEq,
            TokenKind::StarEq,
            TokenKind::SlashEq,
            TokenKind::PercentEq,
        ]
    );
}

#[test]
fn test_lex_op_no_space_between() {
    let kinds = lex_kinds("+-");
    assert_eq!(kinds, vec![TokenKind::Plus, TokenKind::Minus]);
}

// ============================================================================
// 7. Delimiters (~15 tests)
// ============================================================================

#[test]
fn test_lex_delim_lparen() {
    let t = lex_first("(");
    assert_eq!(t.kind, TokenKind::LParen);
    assert_eq!(t.text, "(");
}

#[test]
fn test_lex_delim_rparen() {
    let t = lex_first(")");
    assert_eq!(t.kind, TokenKind::RParen);
    assert_eq!(t.text, ")");
}

#[test]
fn test_lex_delim_lbrace() {
    let t = lex_first("{");
    assert_eq!(t.kind, TokenKind::LBrace);
    assert_eq!(t.text, "{");
}

#[test]
fn test_lex_delim_rbrace() {
    let t = lex_first("}");
    assert_eq!(t.kind, TokenKind::RBrace);
    assert_eq!(t.text, "}");
}

#[test]
fn test_lex_delim_lbracket() {
    let t = lex_first("[");
    assert_eq!(t.kind, TokenKind::LBracket);
    assert_eq!(t.text, "[");
}

#[test]
fn test_lex_delim_rbracket() {
    let t = lex_first("]");
    assert_eq!(t.kind, TokenKind::RBracket);
    assert_eq!(t.text, "]");
}

#[test]
fn test_lex_delim_matched_parens() {
    let kinds = lex_kinds("()");
    assert_eq!(kinds, vec![TokenKind::LParen, TokenKind::RParen]);
}

#[test]
fn test_lex_delim_matched_braces() {
    let kinds = lex_kinds("{}");
    assert_eq!(kinds, vec![TokenKind::LBrace, TokenKind::RBrace]);
}

#[test]
fn test_lex_delim_matched_brackets() {
    let kinds = lex_kinds("[]");
    assert_eq!(kinds, vec![TokenKind::LBracket, TokenKind::RBracket]);
}

#[test]
fn test_lex_delim_nested_parens() {
    let kinds = lex_kinds("(())");
    assert_eq!(
        kinds,
        vec![
            TokenKind::LParen,
            TokenKind::LParen,
            TokenKind::RParen,
            TokenKind::RParen,
        ]
    );
}

#[test]
fn test_lex_delim_nested_mixed() {
    let kinds = lex_kinds("({[]})");
    assert_eq!(
        kinds,
        vec![
            TokenKind::LParen,
            TokenKind::LBrace,
            TokenKind::LBracket,
            TokenKind::RBracket,
            TokenKind::RBrace,
            TokenKind::RParen,
        ]
    );
}

#[test]
fn test_lex_delim_all_types() {
    let kinds = lex_kinds("( ) { } [ ]");
    assert_eq!(
        kinds,
        vec![
            TokenKind::LParen,
            TokenKind::RParen,
            TokenKind::LBrace,
            TokenKind::RBrace,
            TokenKind::LBracket,
            TokenKind::RBracket,
        ]
    );
}

#[test]
fn test_lex_delim_with_content() {
    let kinds = lex_kinds("(x)");
    assert_eq!(
        kinds,
        vec![TokenKind::LParen, TokenKind::Ident, TokenKind::RParen]
    );
}

#[test]
fn test_lex_delim_array_literal() {
    let kinds = lex_kinds("[1, 2, 3]");
    assert_eq!(
        kinds,
        vec![
            TokenKind::LBracket,
            TokenKind::Integer,
            TokenKind::Comma,
            TokenKind::Integer,
            TokenKind::Comma,
            TokenKind::Integer,
            TokenKind::RBracket,
        ]
    );
}

#[test]
fn test_lex_delim_function_call() {
    let kinds = lex_kinds("f(a, b)");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Ident,
            TokenKind::LParen,
            TokenKind::Ident,
            TokenKind::Comma,
            TokenKind::Ident,
            TokenKind::RParen,
        ]
    );
}

// ============================================================================
// 8. Punctuation (~20 tests)
// ============================================================================

#[test]
fn test_lex_punct_comma() {
    let t = lex_first(",");
    assert_eq!(t.kind, TokenKind::Comma);
    assert_eq!(t.text, ",");
}

#[test]
fn test_lex_punct_semi() {
    let t = lex_first(";");
    assert_eq!(t.kind, TokenKind::Semi);
    assert_eq!(t.text, ";");
}

#[test]
fn test_lex_punct_dot() {
    let t = lex_first(".");
    assert_eq!(t.kind, TokenKind::Dot);
    assert_eq!(t.text, ".");
}

#[test]
fn test_lex_punct_dot_dot() {
    let t = lex_first("..");
    assert_eq!(t.kind, TokenKind::DotDot);
    assert_eq!(t.text, "..");
}

#[test]
fn test_lex_punct_dot_dot_eq() {
    let t = lex_first("..=");
    assert_eq!(t.kind, TokenKind::DotDotEq);
    assert_eq!(t.text, "..=");
}

#[test]
fn test_lex_punct_spread() {
    let t = lex_first("...");
    assert_eq!(t.kind, TokenKind::Spread);
    assert_eq!(t.text, "...");
}

#[test]
fn test_lex_punct_colon() {
    let t = lex_first(":");
    assert_eq!(t.kind, TokenKind::Colon);
    assert_eq!(t.text, ":");
}

#[test]
fn test_lex_punct_colon_colon() {
    let t = lex_first("::");
    assert_eq!(t.kind, TokenKind::ColonColon);
    assert_eq!(t.text, "::");
}

#[test]
fn test_lex_punct_question() {
    let t = lex_first("?");
    assert_eq!(t.kind, TokenKind::Question);
    assert_eq!(t.text, "?");
}

#[test]
fn test_lex_punct_question_question() {
    let t = lex_first("??");
    assert_eq!(t.kind, TokenKind::QuestionQuestion);
    assert_eq!(t.text, "??");
}

#[test]
fn test_lex_punct_at() {
    let t = lex_first("@");
    assert_eq!(t.kind, TokenKind::At);
    assert_eq!(t.text, "@");
}

#[test]
fn test_lex_punct_hash() {
    let t = lex_first("#");
    assert_eq!(t.kind, TokenKind::Hash);
    assert_eq!(t.text, "#");
}

#[test]
fn test_lex_punct_tilde() {
    let t = lex_first("~");
    assert_eq!(t.kind, TokenKind::Tilde);
    assert_eq!(t.text, "~");
}

#[test]
fn test_lex_punct_dollar() {
    let t = lex_first("$");
    assert_eq!(t.kind, TokenKind::Dollar);
    assert_eq!(t.text, "$");
}

#[test]
fn test_lex_punct_underscore_standalone() {
    // Standalone _ in operator context (non-identifier start)
    // Since underscore starts an identifier scan, single _ becomes Underscore
    let t = lex_first("_");
    assert_eq!(t.kind, TokenKind::Underscore);
}

#[test]
fn test_lex_punct_colon_vs_colon_colon() {
    let kinds = lex_kinds(": ::");
    assert_eq!(kinds, vec![TokenKind::Colon, TokenKind::ColonColon]);
}

#[test]
fn test_lex_punct_dot_vs_dot_dot_vs_spread() {
    let kinds = lex_kinds(". .. ...");
    assert_eq!(
        kinds,
        vec![TokenKind::Dot, TokenKind::DotDot, TokenKind::Spread]
    );
}

#[test]
fn test_lex_punct_question_vs_double_question() {
    let kinds = lex_kinds("? ??");
    assert_eq!(
        kinds,
        vec![TokenKind::Question, TokenKind::QuestionQuestion]
    );
}

#[test]
fn test_lex_punct_at_with_ident() {
    let kinds = lex_kinds("@decorator");
    assert_eq!(kinds, vec![TokenKind::At, TokenKind::Ident]);
}

#[test]
fn test_lex_punct_colon_in_type_annotation() {
    let kinds = lex_kinds("x: Int");
    assert_eq!(
        kinds,
        vec![TokenKind::Ident, TokenKind::Colon, TokenKind::Ident]
    );
}

// ============================================================================
// 9. Comments (~15 tests)
// ============================================================================

#[test]
fn test_lex_comment_single_line_skipped() {
    let kinds = lex_kinds("// this is a comment");
    assert_eq!(kinds, vec![]);
}

#[test]
fn test_lex_comment_single_line_before_token() {
    let kinds = lex_kinds("// comment\nx");
    assert_eq!(kinds, vec![TokenKind::Ident]);
}

#[test]
fn test_lex_comment_single_line_after_token() {
    let kinds = lex_kinds("x // comment");
    assert_eq!(kinds, vec![TokenKind::Ident]);
}

#[test]
fn test_lex_comment_single_line_between_tokens() {
    let kinds = lex_kinds("a // comment\nb");
    assert_eq!(kinds, vec![TokenKind::Ident, TokenKind::Ident]);
    let texts = lex_texts("a // comment\nb");
    assert_eq!(texts, vec!["a", "b"]);
}

#[test]
fn test_lex_comment_multiline_basic() {
    let kinds = lex_kinds("/* comment */");
    assert_eq!(kinds, vec![]);
}

#[test]
fn test_lex_comment_multiline_between_tokens() {
    let kinds = lex_kinds("a /* comment */ b");
    assert_eq!(kinds, vec![TokenKind::Ident, TokenKind::Ident]);
}

#[test]
fn test_lex_comment_multiline_spanning_lines() {
    let kinds = lex_kinds("a /* line1\nline2 */ b");
    assert_eq!(kinds, vec![TokenKind::Ident, TokenKind::Ident]);
}

#[test]
fn test_lex_comment_nested_multiline() {
    let kinds = lex_kinds("a /* outer /* inner */ still outer */ b");
    assert_eq!(kinds, vec![TokenKind::Ident, TokenKind::Ident]);
}

#[test]
fn test_lex_comment_only_file() {
    let kinds = lex_kinds("// just a comment");
    assert_eq!(kinds, vec![]);
}

#[test]
fn test_lex_comment_multiline_only_file() {
    let kinds = lex_kinds("/* just a comment */");
    assert_eq!(kinds, vec![]);
}

#[test]
fn test_lex_comment_multiple_single_line() {
    let kinds = lex_kinds("// line 1\n// line 2\nx");
    assert_eq!(kinds, vec![TokenKind::Ident]);
}

#[test]
fn test_lex_comment_single_line_preserves_next_line() {
    let kinds = lex_kinds("a\n// comment\nb\n// another\nc");
    assert_eq!(
        kinds,
        vec![TokenKind::Ident, TokenKind::Ident, TokenKind::Ident]
    );
}

#[test]
fn test_lex_comment_slash_not_comment() {
    // Single / is division, not a comment
    let kinds = lex_kinds("a / b");
    assert_eq!(
        kinds,
        vec![TokenKind::Ident, TokenKind::Slash, TokenKind::Ident]
    );
}

#[test]
fn test_lex_comment_empty_multiline() {
    let kinds = lex_kinds("/**/");
    assert_eq!(kinds, vec![]);
}

#[test]
fn test_lex_comment_multiline_with_stars() {
    let kinds = lex_kinds("a /*** comment ***/ b");
    assert_eq!(kinds, vec![TokenKind::Ident, TokenKind::Ident]);
}

// ============================================================================
// 10. Whitespace (~10 tests)
// ============================================================================

#[test]
fn test_lex_whitespace_spaces() {
    let kinds = lex_kinds("a   b");
    assert_eq!(kinds, vec![TokenKind::Ident, TokenKind::Ident]);
}

#[test]
fn test_lex_whitespace_tabs() {
    let kinds = lex_kinds("a\t\tb");
    assert_eq!(kinds, vec![TokenKind::Ident, TokenKind::Ident]);
}

#[test]
fn test_lex_whitespace_newlines() {
    let kinds = lex_kinds("a\n\nb");
    assert_eq!(kinds, vec![TokenKind::Ident, TokenKind::Ident]);
}

#[test]
fn test_lex_whitespace_carriage_return() {
    let kinds = lex_kinds("a\r\nb");
    assert_eq!(kinds, vec![TokenKind::Ident, TokenKind::Ident]);
}

#[test]
fn test_lex_whitespace_mixed() {
    let kinds = lex_kinds("a \t \n \r\n b");
    assert_eq!(kinds, vec![TokenKind::Ident, TokenKind::Ident]);
}

#[test]
fn test_lex_whitespace_leading() {
    let kinds = lex_kinds("   x");
    assert_eq!(kinds, vec![TokenKind::Ident]);
}

#[test]
fn test_lex_whitespace_trailing() {
    let kinds = lex_kinds("x   ");
    assert_eq!(kinds, vec![TokenKind::Ident]);
}

#[test]
fn test_lex_whitespace_only() {
    let kinds = lex_kinds("   \n\t  \r\n  ");
    assert_eq!(kinds, vec![]);
}

#[test]
fn test_lex_whitespace_no_space_between_tokens() {
    let kinds = lex_kinds("a+b");
    assert_eq!(
        kinds,
        vec![TokenKind::Ident, TokenKind::Plus, TokenKind::Ident]
    );
}

#[test]
fn test_lex_whitespace_blank_lines() {
    let kinds = lex_kinds("a\n\n\n\nb");
    assert_eq!(kinds, vec![TokenKind::Ident, TokenKind::Ident]);
}

// ============================================================================
// 11. Complex expressions (~25 tests)
// ============================================================================

#[test]
fn test_lex_expr_let_statement() {
    let kinds = lex_kinds("let x = 5;");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Let,
            TokenKind::Ident,
            TokenKind::Eq,
            TokenKind::Integer,
            TokenKind::Semi,
        ]
    );
}

#[test]
fn test_lex_expr_let_mut() {
    let kinds = lex_kinds("let mut count = 0;");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Let,
            TokenKind::Mut,
            TokenKind::Ident,
            TokenKind::Eq,
            TokenKind::Integer,
            TokenKind::Semi,
        ]
    );
}

#[test]
fn test_lex_expr_fn_declaration() {
    let kinds = lex_kinds("fn add(a: Int, b: Int) -> Int {");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Fn,
            TokenKind::Ident,
            TokenKind::LParen,
            TokenKind::Ident,
            TokenKind::Colon,
            TokenKind::Ident,
            TokenKind::Comma,
            TokenKind::Ident,
            TokenKind::Colon,
            TokenKind::Ident,
            TokenKind::RParen,
            TokenKind::Arrow,
            TokenKind::Ident,
            TokenKind::LBrace,
        ]
    );
}

#[test]
fn test_lex_expr_return_statement() {
    let kinds = lex_kinds("return a + b");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Return,
            TokenKind::Ident,
            TokenKind::Plus,
            TokenKind::Ident,
        ]
    );
}

#[test]
fn test_lex_expr_if_else() {
    let kinds = lex_kinds("if x > 0 { true } else { false }");
    assert_eq!(
        kinds,
        vec![
            TokenKind::If,
            TokenKind::Ident,
            TokenKind::Gt,
            TokenKind::Integer,
            TokenKind::LBrace,
            TokenKind::True,
            TokenKind::RBrace,
            TokenKind::Else,
            TokenKind::LBrace,
            TokenKind::False,
            TokenKind::RBrace,
        ]
    );
}

#[test]
fn test_lex_expr_for_loop() {
    let kinds = lex_kinds("for i in 0..10 {");
    assert_eq!(
        kinds,
        vec![
            TokenKind::For,
            TokenKind::Ident,
            TokenKind::In,
            TokenKind::Integer,
            TokenKind::DotDot,
            TokenKind::Integer,
            TokenKind::LBrace,
        ]
    );
}

#[test]
fn test_lex_expr_struct_declaration() {
    let kinds = lex_kinds("struct User { name: String, age: Int }");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Struct,
            TokenKind::Ident,
            TokenKind::LBrace,
            TokenKind::Ident,
            TokenKind::Colon,
            TokenKind::Ident,
            TokenKind::Comma,
            TokenKind::Ident,
            TokenKind::Colon,
            TokenKind::Ident,
            TokenKind::RBrace,
        ]
    );
}

#[test]
fn test_lex_expr_method_chain() {
    let kinds = lex_kinds("obj.method().field");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Ident,
            TokenKind::Dot,
            TokenKind::Ident,
            TokenKind::LParen,
            TokenKind::RParen,
            TokenKind::Dot,
            TokenKind::Ident,
        ]
    );
}

#[test]
fn test_lex_expr_array_literal() {
    let kinds = lex_kinds("[1, 2, 3]");
    assert_eq!(
        kinds,
        vec![
            TokenKind::LBracket,
            TokenKind::Integer,
            TokenKind::Comma,
            TokenKind::Integer,
            TokenKind::Comma,
            TokenKind::Integer,
            TokenKind::RBracket,
        ]
    );
}

#[test]
fn test_lex_expr_map_literal() {
    let kinds = lex_kinds(r#"{"key": "value"}"#);
    assert_eq!(
        kinds,
        vec![
            TokenKind::LBrace,
            TokenKind::String,
            TokenKind::Colon,
            TokenKind::String,
            TokenKind::RBrace,
        ]
    );
}

#[test]
fn test_lex_expr_import_statement() {
    let kinds = lex_kinds("import http as Http");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Import,
            TokenKind::Ident,
            TokenKind::As,
            TokenKind::Ident,
        ]
    );
}

#[test]
fn test_lex_expr_enum_declaration() {
    let kinds = lex_kinds("enum Color { Red, Green, Blue }");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Enum,
            TokenKind::Ident,
            TokenKind::LBrace,
            TokenKind::Ident,
            TokenKind::Comma,
            TokenKind::Ident,
            TokenKind::Comma,
            TokenKind::Ident,
            TokenKind::RBrace,
        ]
    );
}

#[test]
fn test_lex_expr_match_statement() {
    let kinds = lex_kinds("match x { 1 => true, _ => false }");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Match,
            TokenKind::Ident,
            TokenKind::LBrace,
            TokenKind::Integer,
            TokenKind::FatArrow,
            TokenKind::True,
            TokenKind::Comma,
            TokenKind::Underscore,
            TokenKind::FatArrow,
            TokenKind::False,
            TokenKind::RBrace,
        ]
    );
}

#[test]
fn test_lex_expr_comparison_chain() {
    let kinds = lex_kinds("a == b && c != d");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Ident,
            TokenKind::EqEq,
            TokenKind::Ident,
            TokenKind::AndAnd,
            TokenKind::Ident,
            TokenKind::NotEq,
            TokenKind::Ident,
        ]
    );
}

#[test]
fn test_lex_expr_compound_assignment() {
    let kinds = lex_kinds("x += 1;");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Ident,
            TokenKind::PlusEq,
            TokenKind::Integer,
            TokenKind::Semi,
        ]
    );
}

#[test]
fn test_lex_expr_increment() {
    let kinds = lex_kinds("i++");
    assert_eq!(kinds, vec![TokenKind::Ident, TokenKind::PlusPlus]);
}

#[test]
fn test_lex_expr_decorator_with_args() {
    let kinds = lex_kinds("@min(1)");
    assert_eq!(
        kinds,
        vec![
            TokenKind::At,
            TokenKind::Ident,
            TokenKind::LParen,
            TokenKind::Integer,
            TokenKind::RParen,
        ]
    );
}

#[test]
fn test_lex_expr_scope_resolution() {
    let kinds = lex_kinds("Module::method");
    assert_eq!(
        kinds,
        vec![TokenKind::Ident, TokenKind::ColonColon, TokenKind::Ident,]
    );
}

#[test]
fn test_lex_expr_nil_coalescing() {
    let kinds = lex_kinds("x ?? y");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Ident,
            TokenKind::QuestionQuestion,
            TokenKind::Ident,
        ]
    );
}

#[test]
fn test_lex_expr_optional_chaining() {
    let kinds = lex_kinds("x?.y");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Ident,
            TokenKind::Question,
            TokenKind::Dot,
            TokenKind::Ident,
        ]
    );
}

#[test]
fn test_lex_expr_spread_in_call() {
    let kinds = lex_kinds("f(...args)");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Ident,
            TokenKind::LParen,
            TokenKind::Spread,
            TokenKind::Ident,
            TokenKind::RParen,
        ]
    );
}

#[test]
fn test_lex_expr_inline_closure() {
    let kinds = lex_kinds("(x) => x + 1");
    assert_eq!(
        kinds,
        vec![
            TokenKind::LParen,
            TokenKind::Ident,
            TokenKind::RParen,
            TokenKind::FatArrow,
            TokenKind::Ident,
            TokenKind::Plus,
            TokenKind::Integer,
        ]
    );
}

#[test]
fn test_lex_expr_string_template_in_print() {
    let kinds = lex_kinds(r#"print("Hello ${name}")"#);
    assert_eq!(
        kinds,
        vec![
            TokenKind::Print,
            TokenKind::LParen,
            TokenKind::StringTemplate,
            TokenKind::RParen,
        ]
    );
}

#[test]
fn test_lex_expr_negation() {
    let kinds = lex_kinds("!flag");
    assert_eq!(kinds, vec![TokenKind::Bang, TokenKind::Ident]);
}

#[test]
fn test_lex_expr_result_type() {
    let kinds = lex_kinds("Ok(value)");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Ok,
            TokenKind::LParen,
            TokenKind::Ident,
            TokenKind::RParen,
        ]
    );
}

// ============================================================================
// 12. Error handling (~15 tests)
// ============================================================================

#[test]
fn test_lex_error_unterminated_string() {
    let t = lex_first(r#""hello"#);
    assert_eq!(t.kind, TokenKind::Error);
}

#[test]
fn test_lex_error_unterminated_string_with_newline() {
    // String with actual newline but no closing quote
    let t = lex_first("\"hello\nworld");
    assert_eq!(t.kind, TokenKind::Error);
}

#[test]
fn test_lex_error_invalid_escape_a() {
    let t = lex_first(r#""\a""#);
    assert_eq!(t.kind, TokenKind::Error);
}

#[test]
fn test_lex_error_invalid_escape_b() {
    let t = lex_first(r#""\b""#);
    assert_eq!(t.kind, TokenKind::Error);
}

#[test]
fn test_lex_error_invalid_escape_x() {
    let t = lex_first(r#""\x""#);
    assert_eq!(t.kind, TokenKind::Error);
}

#[test]
fn test_lex_error_invalid_character_backtick() {
    let t = lex_first("`");
    assert_eq!(t.kind, TokenKind::Error);
}

#[test]
fn test_lex_error_continues_after_error() {
    // After an error token, lexing should continue
    let tokens = lex("` x");
    assert_eq!(tokens[0].kind, TokenKind::Error);
    assert_eq!(tokens[1].kind, TokenKind::Ident);
    assert_eq!(tokens[1].text, "x");
}

#[test]
fn test_lex_error_unterminated_string_continues() {
    // After unterminated string error, lexing can continue
    let tokens = lex("\"hello\nx");
    // The unterminated string becomes error, then newline processing, then x
    let has_error = tokens.iter().any(|t| t.kind == TokenKind::Error);
    assert!(has_error);
}

#[test]
fn test_lex_error_invalid_escape_message() {
    let t = lex_first(r#""\q""#);
    assert_eq!(t.kind, TokenKind::Error);
    assert!(t.text.contains("Invalid escape"));
    assert!(t.text.contains("q"));
}

#[test]
fn test_lex_error_unterminated_string_message() {
    let t = lex_first(r#""hello"#);
    assert!(t.text.contains("Unterminated"));
}

#[test]
fn test_lex_error_invalid_unicode_escape_message() {
    let t = lex_first(r#""\u{GGGG}""#);
    assert_eq!(t.kind, TokenKind::Error);
    assert!(t.text.contains("Invalid unicode"));
}

#[test]
fn test_lex_error_unicode_escape_no_braces() {
    let t = lex_first(r#""\u1234""#);
    assert_eq!(t.kind, TokenKind::Error);
}

#[test]
fn test_lex_error_multiple_errors() {
    let tokens = lex("` `");
    let errors: Vec<_> = tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Error)
        .collect();
    assert_eq!(errors.len(), 2);
}

#[test]
fn test_lex_error_invalid_char_caret() {
    let t = lex_first("^");
    assert_eq!(t.kind, TokenKind::Error);
}

#[test]
fn test_lex_error_string_with_only_backslash() {
    let t = lex_first("\"\\");
    assert_eq!(t.kind, TokenKind::Error);
}

// ============================================================================
// 13. Edge cases (~20 tests)
// ============================================================================

#[test]
fn test_lex_edge_empty_input() {
    let tokens = lex("");
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, TokenKind::Eof);
}

#[test]
fn test_lex_edge_single_char_ident() {
    let t = lex_first("a");
    assert_eq!(t.kind, TokenKind::Ident);
}

#[test]
fn test_lex_edge_single_digit() {
    let t = lex_first("0");
    assert_eq!(t.kind, TokenKind::Integer);
}

#[test]
fn test_lex_edge_single_operator() {
    let t = lex_first("+");
    assert_eq!(t.kind, TokenKind::Plus);
}

#[test]
fn test_lex_edge_eof_token_present() {
    let tokens = lex("x");
    let last = tokens.last().unwrap();
    assert_eq!(last.kind, TokenKind::Eof);
}

#[test]
fn test_lex_edge_eof_always_last() {
    let tokens = lex("let x = 5;");
    let last = tokens.last().unwrap();
    assert_eq!(last.kind, TokenKind::Eof);
}

#[test]
fn test_lex_edge_consecutive_operators() {
    let kinds = lex_kinds("+-*/%");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Plus,
            TokenKind::Minus,
            TokenKind::Star,
            TokenKind::Slash,
            TokenKind::Percent,
        ]
    );
}

#[test]
fn test_lex_edge_many_tokens() {
    let source = "x ".repeat(100);
    let kinds = lex_kinds(&source);
    assert_eq!(kinds.len(), 100);
    assert!(kinds.iter().all(|k| *k == TokenKind::Ident));
}

#[test]
fn test_lex_edge_span_start_at_zero() {
    let t = lex_first("hello");
    assert_eq!(t.span.start, 0);
}

#[test]
fn test_lex_edge_span_end_correct() {
    let t = lex_first("hello");
    assert_eq!(t.span.end, 5);
}

#[test]
fn test_lex_edge_span_second_token() {
    let tokens = lex("ab cd");
    // "ab" is at 0..2, space at 2, "cd" at 3..5
    assert_eq!(tokens[0].span.start, 0);
    assert_eq!(tokens[0].span.end, 2);
    assert_eq!(tokens[1].span.start, 3);
    assert_eq!(tokens[1].span.end, 5);
}

#[test]
fn test_lex_edge_span_file_id() {
    let mut lexer = Lexer::new("x", 42);
    let tokens = lexer.tokenize();
    assert_eq!(tokens[0].span.file_id, 42);
}

#[test]
fn test_lex_edge_span_operator() {
    let tokens = lex("==");
    assert_eq!(tokens[0].span.start, 0);
    assert_eq!(tokens[0].span.end, 2);
}

#[test]
fn test_lex_edge_span_three_char_operator() {
    let tokens = lex("..=");
    assert_eq!(tokens[0].span.start, 0);
    assert_eq!(tokens[0].span.end, 3);
}

#[test]
fn test_lex_edge_no_whitespace_ident_operator_ident() {
    let kinds = lex_kinds("a+b");
    assert_eq!(
        kinds,
        vec![TokenKind::Ident, TokenKind::Plus, TokenKind::Ident]
    );
}

#[test]
fn test_lex_edge_keyword_immediately_followed_by_paren() {
    let kinds = lex_kinds("if(x)");
    assert_eq!(
        kinds,
        vec![
            TokenKind::If,
            TokenKind::LParen,
            TokenKind::Ident,
            TokenKind::RParen,
        ]
    );
}

#[test]
fn test_lex_edge_number_dot_number_dot_number() {
    // "1.2.3" -> Float(1.2), Dot, Integer(3)
    let kinds = lex_kinds("1.2.3");
    assert_eq!(
        kinds,
        vec![TokenKind::Float, TokenKind::Dot, TokenKind::Integer]
    );
}

#[test]
fn test_lex_edge_operator_then_string() {
    let kinds = lex_kinds(r#"+"hello""#);
    assert_eq!(kinds, vec![TokenKind::Plus, TokenKind::String]);
}

#[test]
fn test_lex_edge_string_then_operator() {
    let kinds = lex_kinds(r#""hello"+"#);
    assert_eq!(kinds, vec![TokenKind::String, TokenKind::Plus]);
}

#[test]
fn test_lex_edge_multiple_file_ids() {
    let mut lexer1 = Lexer::new("x", 0);
    let mut lexer2 = Lexer::new("y", 1);
    let tokens1 = lexer1.tokenize();
    let tokens2 = lexer2.tokenize();
    assert_eq!(tokens1[0].span.file_id, 0);
    assert_eq!(tokens2[0].span.file_id, 1);
}

// ============================================================================
// 14. TokenKind methods (~25 tests)
// ============================================================================

#[test]
fn test_tokenkind_is_keyword_let() {
    assert!(TokenKind::Let.is_keyword());
}

#[test]
fn test_tokenkind_is_keyword_mut() {
    assert!(TokenKind::Mut.is_keyword());
}

#[test]
fn test_tokenkind_is_keyword_fn() {
    assert!(TokenKind::Fn.is_keyword());
}

#[test]
fn test_tokenkind_is_keyword_import() {
    assert!(TokenKind::Import.is_keyword());
}

#[test]
fn test_tokenkind_is_keyword_as() {
    assert!(TokenKind::As.is_keyword());
}

#[test]
fn test_tokenkind_is_keyword_struct() {
    assert!(TokenKind::Struct.is_keyword());
}

#[test]
fn test_tokenkind_is_keyword_enum() {
    assert!(TokenKind::Enum.is_keyword());
}

#[test]
fn test_tokenkind_is_keyword_true_false() {
    assert!(TokenKind::True.is_keyword());
    assert!(TokenKind::False.is_keyword());
}

#[test]
fn test_tokenkind_is_keyword_nil() {
    assert!(TokenKind::Nil.is_keyword());
}

#[test]
fn test_tokenkind_is_keyword_not_for_ident() {
    assert!(!TokenKind::Ident.is_keyword());
}

#[test]
fn test_tokenkind_is_keyword_not_for_integer() {
    assert!(!TokenKind::Integer.is_keyword());
}

#[test]
fn test_tokenkind_is_keyword_not_for_plus() {
    assert!(!TokenKind::Plus.is_keyword());
}

#[test]
fn test_tokenkind_is_keyword_not_for_eof() {
    assert!(!TokenKind::Eof.is_keyword());
}

#[test]
fn test_tokenkind_is_literal_integer() {
    assert!(TokenKind::Integer.is_literal());
}

#[test]
fn test_tokenkind_is_literal_float() {
    assert!(TokenKind::Float.is_literal());
}

#[test]
fn test_tokenkind_is_literal_string() {
    assert!(TokenKind::String.is_literal());
}

#[test]
fn test_tokenkind_is_literal_string_template() {
    assert!(TokenKind::StringTemplate.is_literal());
}

#[test]
fn test_tokenkind_is_literal_true() {
    assert!(TokenKind::True.is_literal());
}

#[test]
fn test_tokenkind_is_literal_false() {
    assert!(TokenKind::False.is_literal());
}

#[test]
fn test_tokenkind_is_literal_nil() {
    assert!(TokenKind::Nil.is_literal());
}

#[test]
fn test_tokenkind_is_literal_not_for_ident() {
    assert!(!TokenKind::Ident.is_literal());
}

#[test]
fn test_tokenkind_is_literal_not_for_let() {
    assert!(!TokenKind::Let.is_literal());
}

#[test]
fn test_tokenkind_is_operator_plus() {
    assert!(TokenKind::Plus.is_operator());
}

#[test]
fn test_tokenkind_is_operator_minus() {
    assert!(TokenKind::Minus.is_operator());
}

#[test]
fn test_tokenkind_is_operator_star() {
    assert!(TokenKind::Star.is_operator());
}

#[test]
fn test_tokenkind_is_operator_slash() {
    assert!(TokenKind::Slash.is_operator());
}

#[test]
fn test_tokenkind_is_operator_percent() {
    assert!(TokenKind::Percent.is_operator());
}

#[test]
fn test_tokenkind_is_operator_eq_eq() {
    assert!(TokenKind::EqEq.is_operator());
}

#[test]
fn test_tokenkind_is_operator_not_eq() {
    assert!(TokenKind::NotEq.is_operator());
}

#[test]
fn test_tokenkind_is_operator_comparisons() {
    assert!(TokenKind::Lt.is_operator());
    assert!(TokenKind::Gt.is_operator());
    assert!(TokenKind::LtEq.is_operator());
    assert!(TokenKind::GtEq.is_operator());
}

#[test]
fn test_tokenkind_is_operator_logical() {
    assert!(TokenKind::Bang.is_operator());
    assert!(TokenKind::AndAnd.is_operator());
    assert!(TokenKind::OrOr.is_operator());
    assert!(TokenKind::And.is_operator());
    assert!(TokenKind::Or.is_operator());
}

#[test]
fn test_tokenkind_is_operator_assignment() {
    assert!(TokenKind::Eq.is_operator());
    assert!(TokenKind::PlusEq.is_operator());
    assert!(TokenKind::MinusEq.is_operator());
    assert!(TokenKind::StarEq.is_operator());
    assert!(TokenKind::SlashEq.is_operator());
    assert!(TokenKind::PercentEq.is_operator());
}

#[test]
fn test_tokenkind_is_operator_increment_decrement() {
    assert!(TokenKind::PlusPlus.is_operator());
    assert!(TokenKind::MinusMinus.is_operator());
}

#[test]
fn test_tokenkind_is_operator_arrows() {
    assert!(TokenKind::Arrow.is_operator());
    assert!(TokenKind::FatArrow.is_operator());
}

#[test]
fn test_tokenkind_is_operator_not_for_ident() {
    assert!(!TokenKind::Ident.is_operator());
}

#[test]
fn test_tokenkind_is_operator_not_for_lparen() {
    assert!(!TokenKind::LParen.is_operator());
}

#[test]
fn test_tokenkind_is_operator_not_for_comma() {
    assert!(!TokenKind::Comma.is_operator());
}

#[test]
fn test_tokenkind_is_delimiter_all() {
    assert!(TokenKind::LParen.is_delimiter());
    assert!(TokenKind::RParen.is_delimiter());
    assert!(TokenKind::LBrace.is_delimiter());
    assert!(TokenKind::RBrace.is_delimiter());
    assert!(TokenKind::LBracket.is_delimiter());
    assert!(TokenKind::RBracket.is_delimiter());
}

#[test]
fn test_tokenkind_is_delimiter_not_for_comma() {
    assert!(!TokenKind::Comma.is_delimiter());
}

#[test]
fn test_tokenkind_is_delimiter_not_for_operator() {
    assert!(!TokenKind::Plus.is_delimiter());
}

#[test]
fn test_tokenkind_is_delimiter_not_for_keyword() {
    assert!(!TokenKind::Let.is_delimiter());
}

#[test]
fn test_tokenkind_description_keyword() {
    assert_eq!(TokenKind::Let.description(), "`let`");
    assert_eq!(TokenKind::Fn.description(), "`fn`");
    assert_eq!(TokenKind::Return.description(), "`return`");
}

#[test]
fn test_tokenkind_description_literal() {
    assert_eq!(TokenKind::Integer.description(), "integer");
    assert_eq!(TokenKind::Float.description(), "float");
    assert_eq!(TokenKind::String.description(), "string");
    assert_eq!(TokenKind::StringTemplate.description(), "string template");
}

#[test]
fn test_tokenkind_description_operator() {
    assert_eq!(TokenKind::Plus.description(), "`+`");
    assert_eq!(TokenKind::EqEq.description(), "`==`");
    assert_eq!(TokenKind::Arrow.description(), "`->`");
}

#[test]
fn test_tokenkind_description_delimiter() {
    assert_eq!(TokenKind::LParen.description(), "`(`");
    assert_eq!(TokenKind::RBrace.description(), "`}`");
}

#[test]
fn test_tokenkind_description_punctuation() {
    assert_eq!(TokenKind::Comma.description(), "`,`");
    assert_eq!(TokenKind::Semi.description(), "`;`");
    assert_eq!(TokenKind::Dot.description(), "`.`");
    assert_eq!(TokenKind::DotDot.description(), "`..`");
    assert_eq!(TokenKind::DotDotEq.description(), "`..=`");
    assert_eq!(TokenKind::Spread.description(), "`...`");
    assert_eq!(TokenKind::Colon.description(), "`:`");
    assert_eq!(TokenKind::ColonColon.description(), "`::`");
}

#[test]
fn test_tokenkind_description_special() {
    assert_eq!(TokenKind::Eof.description(), "end of file");
    assert_eq!(TokenKind::Error.description(), "error");
    assert_eq!(TokenKind::Ident.description(), "identifier");
}

#[test]
fn test_tokenkind_keyword_str_let() {
    assert_eq!(TokenKind::Let.keyword_str(), Some("let"));
}

#[test]
fn test_tokenkind_keyword_str_fn() {
    assert_eq!(TokenKind::Fn.keyword_str(), Some("fn"));
}

#[test]
fn test_tokenkind_keyword_str_ok_err() {
    assert_eq!(TokenKind::Ok.keyword_str(), Some("Ok"));
    assert_eq!(TokenKind::Err.keyword_str(), Some("Err"));
}

#[test]
fn test_tokenkind_keyword_str_all_keywords() {
    assert_eq!(TokenKind::Let.keyword_str(), Some("let"));
    assert_eq!(TokenKind::Mut.keyword_str(), Some("mut"));
    assert_eq!(TokenKind::Fn.keyword_str(), Some("fn"));
    assert_eq!(TokenKind::Import.keyword_str(), Some("import"));
    assert_eq!(TokenKind::As.keyword_str(), Some("as"));
    assert_eq!(TokenKind::Struct.keyword_str(), Some("struct"));
    assert_eq!(TokenKind::Enum.keyword_str(), Some("enum"));
    assert_eq!(TokenKind::If.keyword_str(), Some("if"));
    assert_eq!(TokenKind::Else.keyword_str(), Some("else"));
    assert_eq!(TokenKind::For.keyword_str(), Some("for"));
    assert_eq!(TokenKind::In.keyword_str(), Some("in"));
    assert_eq!(TokenKind::Return.keyword_str(), Some("return"));
    assert_eq!(TokenKind::Break.keyword_str(), Some("break"));
    assert_eq!(TokenKind::Continue.keyword_str(), Some("continue"));
    assert_eq!(TokenKind::Print.keyword_str(), Some("print"));
    assert_eq!(TokenKind::Ok.keyword_str(), Some("Ok"));
    assert_eq!(TokenKind::Err.keyword_str(), Some("Err"));
    assert_eq!(TokenKind::Nil.keyword_str(), Some("nil"));
    assert_eq!(TokenKind::Match.keyword_str(), Some("match"));
    assert_eq!(TokenKind::True.keyword_str(), Some("true"));
    assert_eq!(TokenKind::False.keyword_str(), Some("false"));
}

#[test]
fn test_tokenkind_keyword_str_none_for_non_keywords() {
    assert_eq!(TokenKind::Ident.keyword_str(), None);
    assert_eq!(TokenKind::Integer.keyword_str(), None);
    assert_eq!(TokenKind::Plus.keyword_str(), None);
    assert_eq!(TokenKind::LParen.keyword_str(), None);
    assert_eq!(TokenKind::Comma.keyword_str(), None);
    assert_eq!(TokenKind::Eof.keyword_str(), None);
    assert_eq!(TokenKind::Error.keyword_str(), None);
}

#[test]
fn test_tokenkind_display_uses_description() {
    let display = format!("{}", TokenKind::Let);
    assert_eq!(display, "`let`");
}

#[test]
fn test_tokenkind_display_operator() {
    let display = format!("{}", TokenKind::Plus);
    assert_eq!(display, "`+`");
}

#[test]
fn test_token_display_format() {
    let tokens = lex("hello");
    let display = format!("{}", tokens[0]);
    assert!(display.contains("Ident"));
    assert!(display.contains("hello"));
}

#[test]
fn test_token_is_eof() {
    let tokens = lex("");
    assert!(tokens[0].is_eof());
}

#[test]
fn test_token_is_eof_false() {
    let tokens = lex("x");
    assert!(!tokens[0].is_eof());
}

#[test]
fn test_token_is_error() {
    let tokens = lex("`");
    assert!(tokens[0].is_error());
}

#[test]
fn test_token_is_error_false() {
    let tokens = lex("x");
    assert!(!tokens[0].is_error());
}

// ============================================================================
// Additional edge case and robustness tests
// ============================================================================

#[test]
fn test_lex_edge_all_keywords_in_sequence() {
    let kinds = lex_kinds(
        "let mut fn import as struct enum if else for in return break continue print Ok Err nil match true false",
    );
    assert_eq!(
        kinds,
        vec![
            TokenKind::Let,
            TokenKind::Mut,
            TokenKind::Fn,
            TokenKind::Import,
            TokenKind::As,
            TokenKind::Struct,
            TokenKind::Enum,
            TokenKind::If,
            TokenKind::Else,
            TokenKind::For,
            TokenKind::In,
            TokenKind::Return,
            TokenKind::Break,
            TokenKind::Continue,
            TokenKind::Print,
            TokenKind::Ok,
            TokenKind::Err,
            TokenKind::Nil,
            TokenKind::Match,
            TokenKind::True,
            TokenKind::False,
        ]
    );
}

#[test]
fn test_lex_edge_all_single_char_operators() {
    let kinds = lex_kinds("( ) { } [ ] , ; . : ? @ # ~ $");
    assert_eq!(
        kinds,
        vec![
            TokenKind::LParen,
            TokenKind::RParen,
            TokenKind::LBrace,
            TokenKind::RBrace,
            TokenKind::LBracket,
            TokenKind::RBracket,
            TokenKind::Comma,
            TokenKind::Semi,
            TokenKind::Dot,
            TokenKind::Colon,
            TokenKind::Question,
            TokenKind::At,
            TokenKind::Hash,
            TokenKind::Tilde,
            TokenKind::Dollar,
        ]
    );
}

#[test]
fn test_lex_edge_all_two_char_operators() {
    let kinds = lex_kinds("== != <= >= && || ++ -- += -= *= /= %= -> => .. :: ??");
    assert_eq!(
        kinds,
        vec![
            TokenKind::EqEq,
            TokenKind::NotEq,
            TokenKind::LtEq,
            TokenKind::GtEq,
            TokenKind::AndAnd,
            TokenKind::OrOr,
            TokenKind::PlusPlus,
            TokenKind::MinusMinus,
            TokenKind::PlusEq,
            TokenKind::MinusEq,
            TokenKind::StarEq,
            TokenKind::SlashEq,
            TokenKind::PercentEq,
            TokenKind::Arrow,
            TokenKind::FatArrow,
            TokenKind::DotDot,
            TokenKind::ColonColon,
            TokenKind::QuestionQuestion,
        ]
    );
}

#[test]
fn test_lex_edge_all_three_char_operators() {
    let kinds = lex_kinds("..= ...");
    assert_eq!(kinds, vec![TokenKind::DotDotEq, TokenKind::Spread]);
}

#[test]
fn test_lex_edge_real_function_body() {
    let code = r#"fn greet(name: String) -> String {
    let greeting = "Hello, ${name}!"
    return greeting
}"#;
    let kinds = lex_kinds(code);
    assert_eq!(kinds[0], TokenKind::Fn);
    assert_eq!(kinds[1], TokenKind::Ident); // greet
    assert!(kinds.contains(&TokenKind::Arrow));
    assert!(kinds.contains(&TokenKind::Let));
    assert!(kinds.contains(&TokenKind::StringTemplate));
    assert!(kinds.contains(&TokenKind::Return));
}

#[test]
fn test_lex_edge_struct_with_decorators() {
    let code = r#"struct User {
    @email
    email: String
    @min(1)
    name: String
}"#;
    let kinds = lex_kinds(code);
    assert_eq!(kinds[0], TokenKind::Struct);
    assert!(kinds.contains(&TokenKind::At));
}

#[test]
fn test_lex_edge_complex_expression() {
    let kinds = lex_kinds("a && b || !c == d != e");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Ident,
            TokenKind::AndAnd,
            TokenKind::Ident,
            TokenKind::OrOr,
            TokenKind::Bang,
            TokenKind::Ident,
            TokenKind::EqEq,
            TokenKind::Ident,
            TokenKind::NotEq,
            TokenKind::Ident,
        ]
    );
}

#[test]
fn test_lex_edge_token_count() {
    // Simple verification that we get expected token count (including Eof)
    let tokens = lex("let x = 5;");
    assert_eq!(tokens.len(), 6); // let, x, =, 5, ;, Eof
}

#[test]
fn test_lex_edge_only_eof_for_empty() {
    let tokens = lex("");
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, TokenKind::Eof);
}
