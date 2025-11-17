//! Fixed Bugs - Regression Tests
//! Tests for bugs that were previously broken and are now fixed
//! All tests in this file should PASS

use super::super::common::{assert_compiles, assert_fails};

// Bug: Duplicate variable declarations in same scope were allowed
#[test]
fn test_fixed_duplicate_variable_in_scope() {
    assert_fails("fn main() { let x = 1; let x = 2; }");
}

// Bug: break/continue were allowed outside loops
#[test]
fn test_fixed_break_outside_loop() {
    assert_fails("fn main() { break; }");
}

#[test]
fn test_fixed_continue_outside_loop() {
    assert_fails("fn main() { continue; }");
}

// Bug: Non-boolean if conditions were allowed
#[test]
fn test_fixed_if_condition_type() {
    assert_fails("fn main() { if 42 { } }");
    assert_fails(r#"fn main() { if "string" { } }"#);
    assert_fails("fn main() { if 3.14 { } }");
}

// Bug: Non-integer array indices were allowed
#[test]
fn test_fixed_array_index_type() {
    assert_fails(r#"fn main() { let arr = [1, 2]; let x = arr["bad"]; }"#);
}

// Bug: Type mismatches in arithmetic were not caught
#[test]
fn test_fixed_type_mismatch_arithmetic() {
    assert_fails(r#"fn main() { let x = "hello" - 5; }"#);
    assert_fails(r#"fn main() { let x = "hello" * 2; }"#);
    assert_fails(r#"fn main() { let x = "hello" / 2; }"#);
}

// Bug: Variable shadowing in inner scopes didn't work correctly
// Fixed: Now properly allows shadowing in nested scopes
#[test]
fn test_fixed_variable_shadowing() {
    let code = r#"
        fn main() {
            let x = 1;
            if true {
                let x = 2;
                print(x);
            }
            print(x);
        }
    "#;
    assert_compiles(code);
}

// Bug: Mixed int/float arithmetic didn't coerce properly
// Fixed: Now properly promotes int to float
#[test]
fn test_fixed_mixed_arithmetic() {
    let code = r#"
        fn main() {
            let x: Float = 10 + 5.5;
            let y: Float = 5 * 2.5;
            let z: Float = 10 / 2.5;
        }
    "#;
    assert_compiles(code);
}

// Bug: String + number concatenation didn't work
// Fixed: Now properly converts numbers to strings
#[test]
fn test_fixed_string_number_concat() {
    let code = r#"
        fn main() {
            let msg = "value: " + 42;
            let msg2 = "pi: " + 3.14;
            let msg3 = "negative: " + -15;
        }
    "#;
    assert_compiles(code);
}

// Bug: Nested loops had scope/control flow issues
#[test]
fn test_fixed_nested_loops() {
    let code = r#"
        fn main() {
            for i in 0..3 {
                for j in 0..3 {
                    if j == 1 { break; }
                    print(i + j);
                }
            }
        }
    "#;
    assert_compiles(code);
}

// Bug: Lambda parameter/return types weren't inferred correctly
#[test]
fn test_fixed_lambda_type_inference() {
    let code = r#"
        fn main() {
            let arr = [1, 2, 3];
            let doubled = arr.map((x) => x * 2);
            let evens = arr.filter((x) => x % 2 == 0);
        }
    "#;
    assert_compiles(code);
}

// Bug: Compound assignment operators didn't work correctly
#[test]
fn test_fixed_compound_assignment() {
    let code = r#"
        fn main() {
            let mut x = 10;
            x += 5;
            x -= 2;
            x *= 3;
            x /= 4;
            x++;
            x--;
        }
    "#;
    assert_compiles(code);
}

// Bug: Recursive function calls had stack/scope issues
#[test]
fn test_fixed_recursive_functions() {
    let code = r#"
        fn factorial(n: Int) -> Int {
            if n <= 1 {
                return 1;
            }
            return n * factorial(n - 1);
        }
        fn main() {
            let result = factorial(5);
        }
    "#;
    assert_compiles(code);
}

// Bug: Mutable variables in functions weren't properly tracked
#[test]
fn test_fixed_mutable_locals() {
    let code = r#"
        fn sumRange(n: Int) -> Int {
            let mut sum = 0;
            for i in 0..n {
                sum += i;
            }
            return sum;
        }
        fn main() {
            let result = sumRange(10);
        }
    "#;
    assert_compiles(code);
}

// Bug: Arrays could contain mixed types
#[test]
fn test_fixed_array_type_consistency() {
    assert_fails("fn main() { let arr = [1, \"hello\", 3]; }");
    assert_fails("fn main() { let arr = [1, 2, 3.14]; }");
}

// Bug: Boolean comparison not throwing errors
#[test]
fn test_fixed_boolean_comparison() {
    assert_fails("fn main() { if true > false {}}");
}

// Bug: Empty maps and arrays can contain mixed types
#[test]
fn test_fixed_empty_map_all_types() {
    assert_compiles(
        r#"
        fn main() {
            let mut mstrint: {Str: Int} = {};
            let mut mstrfloat: {Str: Float} = {};
            let mut mstrbool: {Str: Bool} = {};
            let mut mstrstr: {Str: Str} = {};
            let mut mintint: {Int: Int} = {};
            let mut mintfloat: {Int: Float} = {};
            let mut mintbool: {Int: Bool} = {};
            let mut mintstr: {Int: Str} = {};
            let mut mfloatint: {Float: Int} = {};
            let mut mfloatfloat: {Float: Float} = {};
            let mut mfloatbool: {Float: Bool} = {};
            let mut mfloatstr: {Float: Str} = {};
            let mut mboolint: {Bool: Int} = {};
            let mut mboolfloat: {Bool: Float} = {};
            let mut mboolbool: {Bool: Bool} = {};
            let mut mboolstr: {Bool: Str} = {};
        }
    "#,
    );
}

#[test]
fn test_fixed_empty_array_all_types() {
    assert_compiles(
        r#"
        fn main() {
            let mut arrInt: [Int] = [];
            let mut arrFloat: [Float] = [];
            let mut arrBool: [Bool] = [];
            let mut arrStr: [Str] = [];
        }
    "#,
    );
}

// Bug: Import allowed without main function
#[test]
fn test_fixed_missing_main() {
    assert_fails(r#"import std::Math::{Abs}; { }"#);
}

// Bug: builtin method not working inside variables
#[test]
fn test_fixed_builtin_in_variable() {
    assert_compiles(
        r#"
        fn main() {
            let s = "ha";
            let repeated = s.repeat(3);
            let s2 = "banana";
            let count = s2.countSubstr("an");
        }
        "#,
    );
}

// Bug: Negative as type cast not working inside variable
#[test]
fn test_fixed_typecast_in_variable() {
    assert_compiles(
        r#"
        fn main() { let a = -3.14;
            let b = a as Str;
        }
        "#,
    );
}
