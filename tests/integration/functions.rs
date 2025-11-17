//! Function Integration Tests
//! Tests function features through full compilation pipeline
//! Focus: Function declarations, calls, recursion, and composition

use super::super::common::{assert_compiles, assert_fails};

// =====================================================================
// FUNCTION DECLARATIONS
// =====================================================================

#[test]
fn test_function_declarations() {
    assert_compiles("fn greet() { print(1); } fn main() { }");
    assert_compiles("fn getValue() -> Int { return 42; } fn main() { }");
    assert_compiles("fn double(x: Int) -> Int { return x * 2; } fn main() { }");
    assert_compiles("fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { }");
    assert_compiles("fn sum3(a: Int, b: Int, c: Int) -> Int { return a + b + c; } fn main() { }");
}

#[test]
fn test_function_parameter_types() {
    assert_compiles("fn process(x: Int, y: Str, z: Bool) -> Int { return x; } fn main() { }");
    assert_compiles("fn sumArr(arr: [Int]) -> Int { return arr[0]; } fn main() { }");
    assert_compiles("fn getVal(m: {Str: Int}) -> Int { return 0; } fn main() { }");
}

#[test]
fn test_function_empty_body() {
    assert_compiles("fn test() { } fn main() { }");
}

// =====================================================================
// FUNCTION CALLS
// =====================================================================

#[test]
fn test_function_calls_basic() {
    assert_compiles("fn greet() { print(1); } fn main() { greet(); }");
    assert_compiles(
        "fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { let x = add(5, 3); }",
    );
}

#[test]
fn test_function_calls_various_args() {
    assert_compiles(
        "fn mul(a: Int, b: Int) -> Int { return a * b; } fn main() { let x = mul(4, 7); }",
    );
    assert_compiles("fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { let x = 5; let y = 3; let z = add(x, y); }");
}

#[test]
fn test_function_calls_in_expressions() {
    assert_compiles("fn get() -> Int { return 10; } fn main() { let x = get() + 5; }");
    assert_compiles("fn get() -> Int { return 42; } fn main() { print(get()); }");
}

#[test]
fn test_function_calls_nested() {
    assert_compiles("fn get() -> Int { return 10; } fn process(x: Int) -> Int { return x * 2; } fn main() { let x = process(get()); }");
}

// =====================================================================
// CHAINED & NESTED CALLS
// =====================================================================

#[test]
fn test_nested_function_calls() {
    assert_compiles("fn inner(x: Int) -> Int { return x * 2; } fn outer(x: Int) -> Int { return inner(x) + 1; } fn main() { let x = outer(5); }");
}

#[test]
fn test_deep_nesting() {
    assert_compiles("fn l3(x: Int) -> Int { return x * 2; } fn l2(x: Int) -> Int { return l3(x) + 1; } fn l1(x: Int) -> Int { return l2(x) + 1; } fn main() { let x = l1(5); }");
}

#[test]
fn test_function_composition() {
    assert_compiles("fn double(x: Int) -> Int { return x * 2; } fn addTen(x: Int) -> Int { return x + 10; } fn main() { let x = addTen(double(5)); }");
}

// =====================================================================
// RECURSION
// =====================================================================

#[test]
fn test_recursion_simple() {
    assert_compiles("fn countdown(n: Int) { if n > 0 { countdown(n - 1); } } fn main() { }");
}

#[test]
fn test_recursion_with_return() {
    assert_compiles(
        "fn fact(n: Int) -> Int { if n <= 1 { return 1; } return n * fact(n - 1); } fn main() { }",
    );
}

#[test]
fn test_recursion_fibonacci() {
    assert_compiles("fn fib(n: Int) -> Int { if n <= 1 { return n; } return fib(n - 1) + fib(n - 2); } fn main() { }");
}

#[test]
fn test_recursion_with_accumulator() {
    assert_compiles("fn sumTo(n: Int, acc: Int) -> Int { if n <= 0 { return acc; } return sumTo(n - 1, acc + n); } fn main() { }");
}

// =====================================================================
// RETURN STATEMENTS
// =====================================================================

#[test]
fn test_return_statements() {
    assert_compiles("fn test(x: Int) -> Int { if x < 0 { return 0; } return x; } fn main() { }");
}

#[test]
fn test_return_multiple_paths() {
    assert_compiles("fn classify(x: Int) -> Int { if x > 0 { return 1; } if x < 0 { return -1; } return 0; } fn main() { }");
}

#[test]
fn test_return_in_loop() {
    assert_compiles("fn find(arr: [Int]) -> Int { for i in 1..5 { if arr[i] == 0 { return i; } } return -1; } fn main() { }");
}

#[test]
fn test_return_expression() {
    assert_compiles("fn calc(x: Int, y: Int) -> Int { return x * 2 + y * 3; } fn main() { }");
}

// =====================================================================
// MULTIPLE FUNCTIONS
// =====================================================================

#[test]
fn test_multiple_functions() {
    assert_compiles("fn add(a: Int, b: Int) -> Int { return a + b; } fn sub(a: Int, b: Int) -> Int { return a - b; } fn main() { }");
}

#[test]
fn test_functions_different_signatures() {
    assert_compiles("fn f1() { } fn f2(x: Int) -> Int { return x; } fn f3(x: Int, y: Int) -> Int { return x + y; } fn main() { }");
}

#[test]
fn test_functions_calling_each_other() {
    assert_compiles("fn helper() -> Int { return 42; } fn processor() -> Int { return helper() * 2; } fn main() { let x = processor(); }");
}

#[test]
fn test_functions_different_return_types() {
    assert_compiles("fn getInt() -> Int { return 42; } fn getStr() -> Str { return \"hello\"; } fn getBool() -> Bool { return true; } fn main() { }");
}

// =====================================================================
// LOCAL VARIABLES IN FUNCTIONS
// =====================================================================

#[test]
fn test_function_local_variables() {
    assert_compiles("fn calc(x: Int) -> Int { let temp = x * 2; let result = temp + 10; return result; } fn main() { }");
}

#[test]
fn test_function_mutable_local() {
    assert_compiles("fn sumRange(n: Int) -> Int { let mut sum = 0; for i in 1..n { sum = sum + i; } return sum; } fn main() { }");
}

// =====================================================================
// FUNCTION ERROR CASES
// =====================================================================

#[test]
fn test_undefined_function() {
    assert_fails("fn main() { undefinedFunc(); }");
}

#[test]
fn test_wrong_argument_count() {
    assert_fails("fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { let x = add(5); }");
    assert_fails(
        "fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { let x = add(5, 3, 7); }",
    );
}

#[test]
fn test_wrong_argument_type() {
    assert_fails(
        "fn process(x: Int) -> Int { return x; } fn main() { let x = process(\"string\"); }",
    );
}

#[test]
fn test_return_type_mismatch() {
    assert_fails("fn get() -> Int { return \"string\"; } fn main() { }");
}

#[test]
fn test_missing_return() {
    assert_fails("fn get() -> Int { let x = 42; } fn main() { }");
}

#[test]
fn test_duplicate_parameter() {
    assert_fails("fn test(x: Int, x: Int) -> Int { return x; } fn main() { }");
}

#[test]
fn test_duplicate_function() {
    assert_fails("fn test() { } fn test() { } fn main() { }");
}
