//! Control Flow Integration Tests
//! Tests if/else, for loops, break/continue, nested control structures

use super::super::common::{assert_compiles, assert_fails};

// ========================================
// IF STATEMENTS
// ========================================

#[test]
fn test_if_basic() {
    assert_compiles("fn main() { if true { } }");
    assert_compiles("fn main() { if false { } }");
}

#[test]
fn test_if_with_body() {
    assert_compiles("fn main() { if true { let x = 5; } }");
    assert_compiles("fn main() { if true { let x = 5; let y = 10; } }");
}

#[test]
fn test_if_else() {
    let code = r#"
        fn main() {
            if true {
                let x = 1;
            } else {
                let y = 2;
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_if_elif_else_chain() {
    let code = r#"
        fn main() {
            let x = 10;
            if x > 10 {
                print("greater");
            } else if x < 10 {
                print("less");
            } else {
                print("equal");
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_if_with_comparison() {
    assert_compiles("fn main() { if 5 > 3 { } }");
    assert_compiles("fn main() { if 5 < 3 { } else { } }");
    assert_compiles("fn main() { let x = 5; if x >= 3 { } }");
}

#[test]
fn test_if_with_boolean_logic() {
    assert_compiles("fn main() { if true && false { } }");
    assert_compiles("fn main() { if true || false { } }");
    assert_compiles("fn main() { if !false { } }");
}

#[test]
fn test_if_with_complex_condition() {
    let code = r#"
        fn main() {
            let x = 5;
            let y = 10;
            if x > 0 && y > 0 || x == y {
                print("complex condition works");
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_nested_if() {
    let code = r#"
        fn main() {
            if true {
                if true {
                    if true {
                        let x = 1;
                    }
                }
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_if_condition_must_be_bool() {
    assert_fails("fn main() { if 42 { } }");
    assert_fails(r#"fn main() { if "hello" { } }"#);
    assert_fails("fn main() { if 3.14 { } }");
}

// ========================================
// FOR LOOPS
// ========================================

#[test]
fn test_for_loop_basic() {
    assert_compiles("fn main() { for i in 0..10 { } }");
    assert_compiles("fn main() { for i in 0..=10 { } }");
}

#[test]
fn test_for_loop_with_body() {
    let code = r#"
        fn main() {
            for i in 0..5 {
                let x = i;
                print(x);
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_for_loop_over_array() {
    let code = r#"
        fn main() {
            let arr = [1, 2, 3, 4, 5];
            for val in arr {
                print(val);
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_for_loop_over_map() {
    let code = r#"
        fn main() {
            let maps = {"a": 1, "b": 2};
            for key, val in maps {
                print(key);
                print(val);
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_for_loop_with_range_variables() {
    let code = r#"
        fn main() {
            let start = 0;
            let end = 10;
            for i in start..end {
                print(i);
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_nested_for_loops() {
    let code = r#"
        fn main() {
            for i in 0..5 {
                for j in 0..5 {
                    let sum = i + j;
                }
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_deeply_nested_for_loops() {
    let code = r#"
        fn main() {
            for i in 0..3 {
                for j in 0..3 {
                    for k in 0..3 {
                        let product = i * j * k;
                    }
                }
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_for_loop_range_types() {
    assert_fails("fn main() { for i in 0.5..10 { } }");
    assert_fails("fn main() { for i in 0..10.5 { } }");
    assert_fails(r#"fn main() { for i in "a".."z" { } }"#);
}

#[test]
fn test_for_loop_non_iterable() {
    assert_fails("fn main() { for i in 42 { } }");
    assert_fails("fn main() { for i in true { } }");
}

// ========================================
// BREAK AND CONTINUE
// ========================================

#[test]
fn test_break_in_loop() {
    let code = r#"
        fn main() {
            for i in 0..10 {
                if i == 5 {
                    break;
                }
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_continue_in_loop() {
    let code = r#"
        fn main() {
            for i in 0..10 {
                if i == 5 {
                    continue;
                }
                print(i);
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_break_and_continue_together() {
    let code = r#"
        fn main() {
            for i in 0..10 {
                if i < 3 {
                    continue;
                }
                if i > 7 {
                    break;
                }
                print(i);
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_break_in_nested_loop() {
    let code = r#"
        fn main() {
            for i in 0..5 {
                for j in 0..5 {
                    if j == 2 {
                        break;
                    }
                }
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_continue_in_nested_loop() {
    let code = r#"
        fn main() {
            for i in 0..5 {
                for j in 0..5 {
                    if j == 2 {
                        continue;
                    }
                    print(j);
                }
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_break_outside_loop() {
    assert_fails("fn main() { break; }");
}

#[test]
fn test_continue_outside_loop() {
    assert_fails("fn main() { continue; }");
}

#[test]
fn test_break_in_if_outside_loop() {
    assert_fails("fn main() { if true { break; } }");
}

#[test]
fn test_continue_in_if_outside_loop() {
    assert_fails("fn main() { if true { continue; } }");
}

// ========================================
// COMPLEX CONTROL FLOW
// ========================================

#[test]
fn test_nested_loops_and_if() {
    let code = r#"
        fn main() {
            let mut total = 0;
            for i in 0..5 {
                for j in 0..5 {
                    if i > 0 && j > 0 {
                        total ++;
                    }
                }
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_loop_with_multiple_conditions() {
    let code = r#"
        fn main() {
            for i in 0..10 {
                if i < 3 {
                    continue;
                } else if i > 7 {
                    break;
                } else {
                    print(i);
                }
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_if_inside_loop_inside_if() {
    let code = r#"
        fn main() {
            let condition = true;
            if condition {
                for i in 0..5 {
                    if i > 2 {
                        print(i);
                    }
                }
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_accumulator_pattern() {
    let code = r#"
        fn main() {
            let mut sum = 0;
            for i in 0..10 {
                sum += i;
            }
            print(sum);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_early_return_in_loop() {
    let code = r#"
        fn findValue(arr: [Int], target: Int) -> Bool {
            for val in arr {
                if val == target {
                    return true;
                }
            }
            return false;
        }
        fn main() { }
    "#;
    assert_compiles(code);
}

#[test]
fn test_complex_nested_control_flow() {
    let code = r#"
        fn main() {
            let mut result = 0;
            for i in 0..10 {
                if i > 5 {
                    for j in 0..i {
                        if j % 2 == 0 {
                            result += j;
                        } else {
                            continue;
                        }
                    }
                } else {
                    result += i;
                }
            }
        }
    "#;
    assert_compiles(code);
}

// ========================================
// BOOLEAN LOGIC IN CONDITIONS
// ========================================

#[test]
fn test_boolean_and() {
    assert_compiles("fn main() { if true && true { } }");
    assert_compiles("fn main() { let x = true && false; }");
}

#[test]
fn test_boolean_or() {
    assert_compiles("fn main() { if true || false { } }");
    assert_compiles("fn main() { let x = false || false; }");
}

#[test]
fn test_boolean_not() {
    assert_compiles("fn main() { if !false { } }");
    assert_compiles("fn main() { let x = !true; }");
}

#[test]
fn test_complex_boolean_expression() {
    let code = r#"
        fn main() {
            let a = true;
            let b = false;
            let c = true;
            if a && b || !b && c {
                print("complex boolean");
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_comparison_boolean_result() {
    assert_compiles("fn main() { let x = 5 > 3; }");
    assert_compiles("fn main() { let x = 5 < 3; }");
    assert_compiles("fn main() { let x = 5 == 5; }");
    assert_compiles("fn main() { let x = 5 != 3; }");
}

#[test]
fn test_boolean_logic_type_error() {
    assert_fails("fn main() { let x = 1 && 2; }");
    assert_fails("fn main() { let x = true || 5; }");
    assert_fails("fn main() { let x = !42; }");
}
