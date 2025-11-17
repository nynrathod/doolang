//! Basic Integration Tests
//! Tests fundamental language features through full compilation pipeline
//! Focus: Entry-level feature coverage without extensive duplication

use super::super::common::assert_compiles;

// =====================================================================
// MINIMAL PROGRAMS
// =====================================================================

#[test]
fn test_empty_main() {
    assert_compiles("fn main() { }");
}

#[test]
fn test_hello_world() {
    assert_compiles(r#"fn main() { print("Hello, World!"); }"#);
}

// =====================================================================
// VARIABLE DECLARATIONS
// =====================================================================

#[test]
fn test_let_int() {
    assert_compiles("fn main() { let x = 42; }");
}

#[test]
fn test_let_string() {
    assert_compiles(r#"fn main() { let s = "hello"; }"#);
}

#[test]
fn test_let_bool() {
    assert_compiles("fn main() { let b = true; }");
}

#[test]
fn test_let_float() {
    assert_compiles("fn main() { let f = 3.14; }");
}

#[test]
fn test_let_mutable() {
    assert_compiles("fn main() { let mut x = 5; x = 10; }");
}

#[test]
fn test_let_with_type_annotation() {
    assert_compiles("fn main() { let x: Int = 42; }");
}

// =====================================================================
// SIMPLE FUNCTIONS
// =====================================================================

#[test]
fn test_function_no_params() {
    assert_compiles("fn greet() { print(1); } fn main() { }");
}

#[test]
fn test_function_with_params() {
    assert_compiles("fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { }");
}

#[test]
fn test_function_call() {
    assert_compiles("fn greet() { print(1); } fn main() { greet(); }");
}

#[test]
fn test_function_call_with_args() {
    assert_compiles(
        "fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { let x = add(5, 3); }",
    );
}

// =====================================================================
// ARITHMETIC (REPRESENTATIVE ONLY)
// =====================================================================

#[test]
fn test_arithmetic_operations() {
    assert_compiles("fn main() { let a = 5 + 3; let b = 10 - 3; let c = 4 * 7; let d = 20 / 4; }");
}

#[test]
fn test_operator_precedence() {
    assert_compiles("fn main() { let x = 5 + 3 * 2; }");
}

// =====================================================================
// COMPARISONS (REPRESENTATIVE ONLY)
// =====================================================================

#[test]
fn test_comparison_operators() {
    assert_compiles("fn main() { let a = 5 == 5; let b = 5 != 3; let c = 3 < 5; let d = 5 > 3; }");
}

// =====================================================================
// BOOLEAN LOGIC
// =====================================================================

#[test]
fn test_boolean_operators() {
    assert_compiles("fn main() { let x = true && false; let y = true || false; let z = !false; }");
}

// =====================================================================
// CONTROL FLOW (MINIMAL)
// =====================================================================

#[test]
fn test_if_statement() {
    assert_compiles("fn main() { if true { } }");
}

#[test]
fn test_if_else() {
    assert_compiles("fn main() { if true { } else { } }");
}

#[test]
fn test_for_loop() {
    assert_compiles("fn main() { for i in 0..10 { } }");
}

// =====================================================================
// COLLECTIONS
// =====================================================================

#[test]
fn test_array_literal() {
    assert_compiles("fn main() { let arr = [1, 2, 3]; }");
}

#[test]
fn test_array_access() {
    assert_compiles("fn main() { let arr = [1, 2, 3]; let x = arr[0]; }");
}

#[test]
fn test_map_literal() {
    assert_compiles(r#"fn main() { let m = {"a": 1, "b": 2}; }"#);
}

// =====================================================================
// PRINT STATEMENTS
// =====================================================================

#[test]
fn test_print_basic() {
    assert_compiles(r#"fn main() { print("hello"); }"#);
}

#[test]
fn test_print_multiple_args() {
    assert_compiles(r#"fn main() { print("x", 5, "y", 10); }"#);
}

// =====================================================================
// COMPOUND ASSIGNMENT
// =====================================================================

#[test]
fn test_compound_assignment() {
    assert_compiles("fn main() { let mut x = 5; x += 3; x -= 1; x *= 2; x /= 2; }");
}

// =====================================================================
// STRING OPERATIONS
// =====================================================================

#[test]
fn test_string_concat() {
    assert_compiles(r#"fn main() { let s = "hello" + " " + "world"; }"#);
}

// =====================================================================
// COMBINED FEATURES
// =====================================================================

#[test]
fn test_function_and_variables() {
    assert_compiles(
        "fn double(x: Int) -> Int { return x * 2; } fn main() { let a = 5; let b = double(a); }",
    );
}

#[test]
fn test_loop_with_accumulator() {
    assert_compiles("fn main() { let mut sum = 0; for i in 0..10 { sum += i; } }");
}

#[test]
fn test_if_else_with_variables() {
    assert_compiles(
        "fn main() { let x = 10; if x > 5 { let y = x * 2; } else { let z = x / 2; } }",
    );
}

#[test]
fn test_multiple_functions() {
    assert_compiles("fn f1() { } fn f2() { } fn f3() { } fn main() { }");
}

#[test]
fn test_nested_function_calls() {
    assert_compiles("fn helper() -> Int { return 42; } fn caller() -> Int { return helper(); } fn main() { let x = caller(); }");
}
