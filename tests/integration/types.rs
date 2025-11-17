//! Auto-generated: Type Cast Test Suite (1 case per function)
//! Converted from main.doo:testAllTypeCasts()
//!
//! Uses:
//!   assert_expr_type(expr, expected_type)

use super::super::common::{assert_expr_type, assert_fails, assert_typeof};

//
// ──────────────────────────────────────────────────────────────
//   PART 1 — ALLOWED CASTS: INT
// ──────────────────────────────────────────────────────────────
//

#[test]
fn test_int_to_int() {
    assert_expr_type("42 as Int", "Int");
}

#[test]
fn test_int_to_float() {
    assert_expr_type("42 as Float", "Float");
}

#[test]
fn test_int_to_str() {
    assert_expr_type("42 as Str", "Str");
}

//
// ──────────────────────────────────────────────────────────────
//   PART 1 — ALLOWED CASTS: FLOAT
// ──────────────────────────────────────────────────────────────
//

#[test]
fn test_float_to_int() {
    assert_expr_type("3.14 as Int", "Int");
}

#[test]
fn test_float_to_float() {
    assert_expr_type("3.14 as Float", "Float");
}

#[test]
fn test_float_to_str() {
    assert_expr_type("3.14 as Str", "Str");
}

//
// ──────────────────────────────────────────────────────────────
//   PART 1 — ALLOWED CASTS: BOOL
// ──────────────────────────────────────────────────────────────
//

#[test]
fn test_true_to_int() {
    assert_expr_type("true as Int", "Int");
}

#[test]
fn test_false_to_int() {
    assert_expr_type("false as Int", "Int");
}

#[test]
fn test_true_to_str() {
    assert_expr_type("true as Str", "Str");
}

#[test]
fn test_false_to_str() {
    assert_expr_type("false as Str", "Str");
}

//
// ──────────────────────────────────────────────────────────────
//   PART 1 — ALLOWED CASTS: STRING
// ──────────────────────────────────────────────────────────────
//

#[test]
fn test_string_int_to_int() {
    assert_expr_type(r#""123" as Int"#, "Int");
}

#[test]
fn test_string_float_to_float() {
    assert_expr_type(r#""3.14" as Float"#, "Float");
}

#[test]
fn test_string_to_string() {
    assert_expr_type(r#""hello" as Str"#, "Str");
}

//
// ──────────────────────────────────────────────────────────────
//   PART 2 — EDGE CASES: ZERO
// ──────────────────────────────────────────────────────────────
//

#[test]
fn test_zero_int_to_float() {
    assert_expr_type("0 as Float", "Float");
}

#[test]
fn test_zero_int_to_str() {
    assert_expr_type("0 as Str", "Str");
}

#[test]
fn test_zero_float_to_int() {
    assert_expr_type("0.0 as Int", "Int");
}

#[test]
fn test_zero_float_to_str() {
    assert_expr_type("0.0 as Str", "Str");
}

//
// ──────────────────────────────────────────────────────────────
//   PART 2 — EDGE CASES: NEGATIVE NUMBERS
// ──────────────────────────────────────────────────────────────
//

#[test]
fn test_negative_int_to_float() {
    assert_expr_type("-42 as Float", "Float");
}

#[test]
fn test_negative_int_to_str() {
    assert_expr_type("-42 as Str", "Str");
}

#[test]
fn test_negative_float_to_int() {
    assert_expr_type("-3.14 as Int", "Int");
}

#[test]
fn test_negative_float_to_str() {
    assert_expr_type("-3.14 as Str", "Str");
}

//
// ──────────────────────────────────────────────────────────────
//   PART 2 — FRACTIONAL TRUNCATION
// ──────────────────────────────────────────────────────────────
//

#[test]
fn test_fraction_0_5_to_int() {
    assert_expr_type("0.5 as Int", "Int");
}

#[test]
fn test_fraction_0_9_to_int() {
    assert_expr_type("0.9 as Int", "Int");
}

#[test]
fn test_fraction_1_1_to_int() {
    assert_expr_type("1.1 as Int", "Int");
}

#[test]
fn test_fraction_large_to_int() {
    assert_expr_type("999.99 as Int", "Int");
}

#[test]
fn test_fraction_negative_0_5_to_int() {
    assert_expr_type("-0.5 as Int", "Int");
}

#[test]
fn test_fraction_negative_1_9_to_int() {
    assert_expr_type("-1.9 as Int", "Int");
}

//
// ──────────────────────────────────────────────────────────────
//   PART 2 — LARGE NUMBERS
// ──────────────────────────────────────────────────────────────
//

#[test]
fn test_large_int_to_float() {
    assert_expr_type("999999 as Float", "Float");
}

#[test]
fn test_large_int_to_str() {
    assert_expr_type("999999 as Str", "Str");
}

#[test]
fn test_large_float_to_int() {
    assert_expr_type("999999.99 as Int", "Int");
}

#[test]
fn test_large_float_to_str() {
    assert_expr_type("999999.99 as Str", "Str");
}

//
// ──────────────────────────────────────────────────────────────
//   PART 2 — STRING PARSING EDGE CASES
// ──────────────────────────────────────────────────────────────
//

#[test]
fn test_string_leading_zero() {
    assert_expr_type(r#""007" as Int"#, "Int");
}

#[test]
fn test_string_negative_integer() {
    assert_expr_type(r#""-123" as Int"#, "Int");
}

#[test]
fn test_string_negative_float() {
    assert_expr_type(r#""-3.14" as Float"#, "Float");
}

//
// ──────────────────────────────────────────────────────────────
//   PART 2 — WHITESPACE STRINGS
// ──────────────────────────────────────────────────────────────
//
// These compile but cannot be cast to int/float.
// Only compilation is tested, not a cast.
//

#[test]
fn test_string_whitespace_literal_compiles() {
    // this expression is a raw string literal by itself, no cast
    assert_expr_type(r#""  123  ""#, "Str");
}

//
// ──────────────────────────────────────────────────────────────
//   PART 3 — BLOCKED CASTS (compile-time errors)
// ──────────────────────────────────────────────────────────────
//

#[test]
fn test_int_0_to_bool_blocked() {
    assert_fails("0 as Bool");
}

#[test]
fn test_int_1_to_bool_blocked() {
    assert_fails("1 as Bool");
}

#[test]
fn test_int_42_to_bool_blocked() {
    assert_fails("42 as Bool");
}

#[test]
fn test_float_0_0_to_bool_blocked() {
    assert_fails("0.0 as Bool");
}

#[test]
fn test_float_1_0_to_bool_blocked() {
    assert_fails("1.0 as Bool");
}

#[test]
fn test_float_3_14_to_bool_blocked() {
    assert_fails("3.14 as Bool");
}

#[test]
fn test_str_true_to_bool_blocked() {
    assert_fails(r#""true" as Bool"#);
}

#[test]
fn test_str_false_to_bool_blocked() {
    assert_fails(r#""false" as Bool"#);
}

#[test]
fn test_str_1_to_bool_blocked() {
    assert_fails(r#""1" as Bool"#);
}

#[test]
fn test_bool_true_to_float_blocked() {
    assert_fails("true as Float");
}

#[test]
fn test_bool_false_to_float_blocked() {
    assert_fails("false as Float");
}

#[test]
fn test_str_true_to_bool_blocked_duplicate() {
    assert_fails(r#""true" as Bool"#);
}

#[test]
fn test_str_false_to_bool_blocked_duplicate() {
    assert_fails(r#""false" as Bool"#);
}

//
// ──────────────────────────────────────────────────────────────
//   PART 4 — RUNTIME PANIC CASES (string parsing)
// ──────────────────────────────────────────────────────────────
//

#[test]
fn test_invalid_string_hello_to_int() {
    assert_fails(r#""hello" as Int"#);
}

#[test]
fn test_invalid_string_abc123_to_int() {
    assert_fails(r#""abc123" as Int"#);
}

#[test]
fn test_invalid_string_decimal_to_int() {
    assert_fails(r#""12.34" as Int"#);
}

#[test]
fn test_invalid_empty_string_to_int() {
    assert_fails(r#"""" as Int"#);
}

#[test]
fn test_invalid_whitespace_string_to_int() {
    assert_fails(r#""  " as Int"#);
}

#[test]
fn test_invalid_string_letters_after_number_to_int() {
    assert_fails(r#""123abc" as Int"#);
}

#[test]
fn test_invalid_string_hello_to_float() {
    assert_fails(r#""hello" as Float"#);
}

#[test]
fn test_invalid_string_abc_to_float() {
    assert_fails(r#""abc" as Float"#);
}

#[test]
fn test_invalid_empty_string_to_float() {
    assert_fails(r#"""" as Float"#);
}

#[test]
fn test_invalid_string_multiple_dots_to_float() {
    assert_fails(r#""3.14.15" as Float"#);
}

//
// ──────────────────────────────────────────────────────────────
//   PART 5 — SPECIAL FLOAT VALUES
// ──────────────────────────────────────────────────────────────
//

#[test]
fn test_infinity_and_nan_types() {
    assert_expr_type("(1.0 / 0.0) as Int", "Int");
    assert_expr_type("(-1.0 / 0.0) as Int", "Int");
    assert_expr_type("(0.0 / 0.0) as Int", "Int");
    assert_expr_type("(1.0 / 0.0) as Str", "Str");
    assert_expr_type("(0.0 / 0.0) as Str", "Str");
}

//
// ──────────────────────────────────────────────────────────────
//   PART 6 — OVERFLOW CASES
// ──────────────────────────────────────────────────────────────
//

#[test]
fn test_large_float_overflow_to_int() {
    assert_expr_type("9999999999.0 as Int", "Int");
}

#[test]
fn test_max_int_and_plus_one_to_float() {
    assert_expr_type("2147483647 as Float", "Float");
    assert_expr_type("2147483647 + 1 as Float", "Float");
}

//
// ──────────────────────────────────────────────────────────────
//   typeOf
// ──────────────────────────────────────────────────────────────
//
//
#[test]
fn test_typeof_int() {
    assert_typeof("42", "Int");
}

#[test]
fn test_typeof_float() {
    assert_typeof("3.14", "Float");
}

#[test]
fn test_typeof_bool() {
    assert_typeof("true", "Bool");
}

#[test]
fn test_typeof_array() {
    assert_typeof("[1,2,3]", "[Int]");
}

#[test]
fn test_typeof_map() {
    assert_typeof("{\"a\": 10}", "{Str:Int}");
}

#[test]
fn test_string_to_int_cast() {
    assert_typeof(r#""123" as Int"#, "Int");
}
