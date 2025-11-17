//! Collections Integration Tests
//! Tests arrays, maps, and their methods (map, filter, reduce, etc.)

use super::super::common::{assert_compiles, assert_fails};

#[test]
fn test_array_literal_empty() {
    assert_compiles("fn main() { let mut x: [Int] = []; }");
    assert_compiles("fn main() { let mut x: [Float] = []; }");
    assert_compiles("fn main() { let mut x: [Str] = []; }");
    assert_compiles("fn main() { let mut x: [Bool] = []; }");
    assert_compiles("fn main() { let mut x = []; }");
}

#[test]
fn test_array_literal_single_element() {
    assert_compiles("fn main() { let x = [42]; }");
    assert_compiles("fn main() { let x: [Int] = [42]; }");
    assert_compiles("fn main() { let x = [3.14]; }");
    assert_compiles("fn main() { let x: [Float] = [3.14]; }");
    assert_compiles(r#"fn main() { let x = ["hello"]; }"#);
    assert_compiles(r#"fn main() { let x: [Str] = ["hello"]; }"#);
    assert_compiles("fn main() { let x = [true]; }");
    assert_compiles("fn main() { let x: [Bool] = [true]; }");
}

#[test]
fn test_array_literal_multiple_elements() {
    assert_compiles("fn main() { let x = [1, 2, 3, 4, 5]; }");
    assert_compiles("fn main() { let x: [Int] = [1, 2, 3, 4, 5]; }");
    assert_compiles("fn main() { let x = [1.1, 2.2, 3.3, 4.4, 5.5]; }");
    assert_compiles("fn main() { let x: [Float] = [1.1, 2.2, 3.3, 4.4, 5.5]; }");
    assert_compiles(r#"fn main() { let x = ["a", "b", "c", "d", "e"]; }"#);
    assert_compiles(r#"fn main() { let x: [Str] = ["a", "b", "c", "d", "e"]; }"#);
    assert_compiles("fn main() { let x = [true, false, true, false]; }");
    assert_compiles("fn main() { let x: [Bool] = [true, false, true, false]; }");
}

#[test]
fn test_array_literal_with_expressions() {
    assert_compiles("fn main() { let x = [1 + 1, 2 * 2, 3 - 1]; }");
    assert_compiles("fn main() { let x: [Int] = [1 + 1, 2 * 2, 3 - 1]; }");
    assert_compiles("fn main() { let x = [1.0 + 2.0, 2.5 * 2.0, 3.14 - 1.0]; }");
    assert_compiles("fn main() { let x: [Float] = [1.0 + 2.0, 2.5 * 2.0, 3.14 - 1.0]; }");
    assert_compiles(r#"fn main() { let x = ["a" + "b", "c" + "d"]; }"#);
    assert_compiles(r#"fn main() { let x: [Str] = ["a" + "b", "c" + "d"]; }"#);
    assert_compiles("fn main() { let x = [true && false, !false, true || false]; }");
    assert_compiles("fn main() { let x: [Bool] = [true && false, !false, true || false]; }");
}

#[test]
fn test_array_of_strings() {
    assert_compiles(r#"fn main() { let x = ["hello", "world"]; }"#);
    assert_compiles(r#"fn main() { let x: [Str] = ["hello", "world"]; }"#);
}

#[test]
fn test_array_of_floats() {
    assert_compiles("fn main() { let x = [1.0, 2.5, 3.14]; }");
    assert_compiles("fn main() { let x: [Float] = [1.0, 2.5, 3.14]; }");
}

#[test]
fn test_array_of_bools() {
    assert_compiles("fn main() { let x = [true, false, true]; }");
    assert_compiles("fn main() { let x: [Bool] = [true, false, true]; }");
}

#[test]
fn test_array_of_ints() {
    assert_compiles("fn main() { let x = [1, 2, 3]; }");
    assert_compiles("fn main() { let x: [Int] = [1, 2, 3]; }");
}

#[test]
fn test_array_type_mismatch() {
    assert_fails("fn main() { let x = [1, \"hello\", 3]; }");
    assert_fails("fn main() { let x = [1, 2.5, 3]; }");
    assert_fails("fn main() { let x = [true, 1]; }");
    assert_fails("fn main() { let x = [\"a\", false]; }");
    assert_fails("fn main() { let x = [1.0, \"b\"]; }");
    assert_fails("fn main() { let x = [true, 2.0]; }");
    assert_fails("fn main() { let x = [\"str\", 1.0]; }");
}

// ========================================
// ARRAYS - ACCESS
// ========================================

#[test]
fn test_array_access_literal_index() {
    assert_compiles("fn main() { let arr = [1, 2, 3]; let x = arr[0]; }");
    assert_compiles("fn main() { let arr: [Int] = [1, 2, 3]; let x = arr[2]; }");
    assert_compiles("fn main() { let arr = [1.1, 2.2, 3.3]; let x = arr[1]; }");
    assert_compiles("fn main() { let arr: [Float] = [1.1, 2.2, 3.3]; let x = arr[2]; }");
    assert_compiles(r#"fn main() { let arr = ["a", "b", "c"]; let x = arr[0]; }"#);
    assert_compiles(r#"fn main() { let arr: [Str] = ["a", "b", "c"]; let x = arr[2]; }"#);
    assert_compiles("fn main() { let arr = [true, false, true]; let x = arr[1]; }");
    assert_compiles("fn main() { let arr: [Bool] = [true, false, true]; let x = arr[2]; }");
}

#[test]
fn test_array_access_variable_index() {
    let code = r#"
        fn main() {
            let arr = [10, 20, 30];
            let idx = 1;
            let x = arr[idx];
        }
    "#;
    assert_compiles(code);

    let code = r#"
        fn main() {
            let arr: [Float] = [1.1, 2.2, 3.3];
            let idx = 2;
            let x = arr[idx];
        }
    "#;
    assert_compiles(code);

    let code = r#"
        fn main() {
            let arr: [Str] = ["a", "b", "c"];
            let idx = 0;
            let x = arr[idx];
        }
    "#;
    assert_compiles(code);

    let code = r#"
        fn main() {
            let arr: [Bool] = [true, false, true];
            let idx = 1;
            let x = arr[idx];
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_access_expression_index() {
    assert_compiles("fn main() { let arr = [1, 2, 3]; let x = arr[1 + 1]; }");
    assert_compiles("fn main() { let arr: [Int] = [1, 2, 3]; let x = arr[2 - 1]; }");
    assert_compiles("fn main() { let arr = [1.1, 2.2, 3.3]; let x = arr[1 + 1]; }");
    assert_compiles("fn main() { let arr: [Float] = [1.1, 2.2, 3.3]; let x = arr[2 - 1]; }");
    assert_compiles(r#"fn main() { let arr = ["a", "b", "c"]; let x = arr[1 + 1]; }"#);
    assert_compiles(r#"fn main() { let arr: [Str] = ["a", "b", "c"]; let x = arr[2 - 1]; }"#);
    assert_compiles("fn main() { let arr = [true, false, true]; let x = arr[1 + 1]; }");
    assert_compiles("fn main() { let arr: [Bool] = [true, false, true]; let x = arr[2 - 1]; }");
}

#[test]
fn test_array_access_in_loop() {
    let code = r#"
        fn main() {
            let arr = [10, 20, 30, 40];
            for i in 0..4 {
                let val = arr[i];
                print(val);
            }
        }
    "#;
    assert_compiles(code);

    let code = r#"
        fn main() {
            let arr: [Float] = [1.1, 2.2, 3.3, 4.4];
            for i in 0..4 {
                let val = arr[i];
                print(val);
            }
        }
    "#;
    assert_compiles(code);

    let code = r#"
        fn main() {
            let arr: [Str] = ["a", "b", "c", "d"];
            for i in 0..4 {
                let val = arr[i];
                print(val);
            }
        }
    "#;
    assert_compiles(code);

    let code = r#"
        fn main() {
            let arr: [Bool] = [true, false, true, false];
            for i in 0..4 {
                let val = arr[i];
                print(val);
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_access_invalid_index_type() {
    assert_fails("fn main() { let arr = [1, 2, 3]; let x = arr[\"bad\"]; }");
    assert_fails("fn main() { let arr = [1, 2, 3]; let x = arr[1.5]; }");
    assert_fails("fn main() { let arr = [1, 2, 3]; let x = arr[true]; }");
    assert_fails("fn main() { let arr: [Float] = [1.1, 2.2]; let x = arr[\"bad\"]; }");
    assert_fails("fn main() { let arr: [Str] = [\"a\", \"b\"]; let x = arr[1.5]; }");
    assert_fails("fn main() { let arr: [Bool] = [true, false]; let x = arr[true]; }");
}

// ========================================
// MAPS - BASIC
// ========================================

#[test]
fn test_map_literal_empty() {
    assert_compiles("fn main() { let mut m: {Str: Int} = {}; }");
    assert_compiles("fn main() { let mut m: {Str: Float} = {}; }");
    assert_compiles("fn main() { let mut m: {Str: Bool} = {}; }");
    assert_compiles("fn main() { let mut m: {Str: Str} = {}; }");
    assert_compiles("fn main() { let mut m: {Int: Int} = {}; }");
    assert_compiles("fn main() { let mut m: {Int: Float} = {}; }");
    assert_compiles("fn main() { let mut m: {Int: Bool} = {}; }");
    assert_compiles("fn main() { let mut m: {Int: Str} = {}; }");
    assert_compiles("fn main() { let mut m: {Float: Int} = {}; }");
    assert_compiles("fn main() { let mut m: {Float: Float} = {}; }");
    assert_compiles("fn main() { let mut m: {Float: Bool} = {}; }");
    assert_compiles("fn main() { let mut m: {Float: Str} = {}; }");
    assert_compiles("fn main() { let mut m: {Bool: Int} = {}; }");
    assert_compiles("fn main() { let mut m: {Bool: Float} = {}; }");
    assert_compiles("fn main() { let mut m: {Bool: Bool} = {}; }");
    assert_compiles("fn main() { let mut m: {Bool: Str} = {}; }");
}

#[test]
fn test_map_literal_single_entry() {
    // {Str: Int}
    assert_compiles(r#"fn main() { let m = {"key": 42}; }"#);
    assert_compiles(r#"fn main() { let m: {Str: Int} = {"key": 42}; }"#);

    // {Str: Float}
    assert_compiles(r#"fn main() { let m = {"pi": 3.14}; }"#);
    assert_compiles(r#"fn main() { let m: {Str: Float} = {"pi": 3.14}; }"#);

    // {Str: Bool}
    assert_compiles(r#"fn main() { let m = {"yes": true}; }"#);
    assert_compiles(r#"fn main() { let m: {Str: Bool} = {"yes": true}; }"#);

    // {Str: Str}
    assert_compiles(r#"fn main() { let m = {"hello": "world"}; }"#);
    assert_compiles(r#"fn main() { let m: {Str: Str} = {"hello": "world"}; }"#);

    // {Int: Int}
    assert_compiles(r#"fn main() { let m = {1: 42}; }"#);
    assert_compiles(r#"fn main() { let m: {Int: Int} = {1: 42}; }"#);

    // {Int: Float}
    assert_compiles(r#"fn main() { let m = {1: 3.14}; }"#);
    assert_compiles(r#"fn main() { let m: {Int: Float} = {1: 3.14}; }"#);

    // {Int: Bool}
    assert_compiles(r#"fn main() { let m = {1: true}; }"#);
    assert_compiles(r#"fn main() { let m: {Int: Bool} = {1: true}; }"#);

    // {Int: Str}
    assert_compiles(r#"fn main() { let m = {1: "hello"}; }"#);
    assert_compiles(r#"fn main() { let m: {Int: Str} = {1: "hello"}; }"#);

    // {Float: Int}
    assert_compiles(r#"fn main() { let m = {3.14: 42}; }"#);
    assert_compiles(r#"fn main() { let m: {Float: Int} = {3.14: 42}; }"#);

    // {Float: Float}
    assert_compiles(r#"fn main() { let m = {3.14: 2.71}; }"#);
    assert_compiles(r#"fn main() { let m: {Float: Float} = {3.14: 2.71}; }"#);

    // {Float: Bool}
    assert_compiles(r#"fn main() { let m = {3.14: true}; }"#);
    assert_compiles(r#"fn main() { let m: {Float: Bool} = {3.14: true}; }"#);

    // {Float: Str}
    assert_compiles(r#"fn main() { let m = {3.14: "hello"}; }"#);
    assert_compiles(r#"fn main() { let m: {Float: Str} = {3.14: "hello"}; }"#);

    // {Bool: Int}
    assert_compiles(r#"fn main() { let m = {true: 42}; }"#);
    assert_compiles(r#"fn main() { let m: {Bool: Int} = {true: 42}; }"#);

    // {Bool: Float}
    assert_compiles(r#"fn main() { let m = {false: 3.14}; }"#);
    assert_compiles(r#"fn main() { let m: {Bool: Float} = {false: 3.14}; }"#);

    // {Bool: Bool}
    assert_compiles(r#"fn main() { let m = {true: false}; }"#);
    assert_compiles(r#"fn main() { let m: {Bool: Bool} = {true: false}; }"#);

    // {Bool: Str}
    assert_compiles(r#"fn main() { let m = {false: "hello"}; }"#);
    assert_compiles(r#"fn main() { let m: {Bool: Str} = {false: "hello"}; }"#);
}

#[test]
fn test_map_literal_multiple_entries() {
    // {Str: Int}
    assert_compiles(r#"fn main() { let m = {"a": 1, "b": 2, "c": 3}; }"#);
    assert_compiles(r#"fn main() { let m: {Str: Int} = {"a": 1, "b": 2, "c": 3}; }"#);

    // {Str: Float}
    assert_compiles(r#"fn main() { let m = {"x": 1.1, "y": 2.2, "z": 3.3}; }"#);
    assert_compiles(r#"fn main() { let m: {Str: Float} = {"x": 1.1, "y": 2.2, "z": 3.3}; }"#);

    // {Str: Bool}
    assert_compiles(r#"fn main() { let m = {"yes": true, "no": false}; }"#);
    assert_compiles(r#"fn main() { let m: {Str: Bool} = {"yes": true, "no": false}; }"#);

    // {Str: Str}
    assert_compiles(r#"fn main() { let m = {"hello": "world", "foo": "bar"}; }"#);
    assert_compiles(r#"fn main() { let m: {Str: Str} = {"hello": "world", "foo": "bar"}; }"#);

    // {Int: Int}
    assert_compiles(r#"fn main() { let m = {1: 10, 2: 20, 3: 30}; }"#);
    assert_compiles(r#"fn main() { let m: {Int: Int} = {1: 10, 2: 20, 3: 30}; }"#);

    // {Int: Float}
    assert_compiles(r#"fn main() { let m = {1: 1.1, 2: 2.2, 3: 3.3}; }"#);
    assert_compiles(r#"fn main() { let m: {Int: Float} = {1: 1.1, 2: 2.2, 3: 3.3}; }"#);

    // {Int: Bool}
    assert_compiles(r#"fn main() { let m = {1: true, 2: false}; }"#);
    assert_compiles(r#"fn main() { let m: {Int: Bool} = {1: true, 2: false}; }"#);

    // {Int: Str}
    assert_compiles(r#"fn main() { let m = {1: "one", 2: "two"}; }"#);
    assert_compiles(r#"fn main() { let m: {Int: Str} = {1: "one", 2: "two"}; }"#);

    // {Float: Int}
    assert_compiles(r#"fn main() { let m = {1.1: 10, 2.2: 20}; }"#);
    assert_compiles(r#"fn main() { let m: {Float: Int} = {1.1: 10, 2.2: 20}; }"#);

    // {Float: Float}
    assert_compiles(r#"fn main() { let m = {1.1: 2.2, 3.3: 4.4}; }"#);
    assert_compiles(r#"fn main() { let m: {Float: Float} = {1.1: 2.2, 3.3: 4.4}; }"#);

    // {Float: Bool}
    assert_compiles(r#"fn main() { let m = {1.1: true, 2.2: false}; }"#);
    assert_compiles(r#"fn main() { let m: {Float: Bool} = {1.1: true, 2.2: false}; }"#);

    // {Float: Str}
    assert_compiles(r#"fn main() { let m = {1.1: "one", 2.2: "two"}; }"#);
    assert_compiles(r#"fn main() { let m: {Float: Str} = {1.1: "one", 2.2: "two"}; }"#);

    // {Bool: Int}
    assert_compiles(r#"fn main() { let m = {true: 1, false: 0}; }"#);
    assert_compiles(r#"fn main() { let m: {Bool: Int} = {true: 1, false: 0}; }"#);

    // {Bool: Float}
    assert_compiles(r#"fn main() { let m = {true: 1.1, false: 0.0}; }"#);
    assert_compiles(r#"fn main() { let m: {Bool: Float} = {true: 1.1, false: 0.0}; }"#);

    // {Bool: Bool}
    assert_compiles(r#"fn main() { let m = {true: false, false: true}; }"#);
    assert_compiles(r#"fn main() { let m: {Bool: Bool} = {true: false, false: true}; }"#);

    // {Bool: Str}
    assert_compiles(r#"fn main() { let m = {true: "yes", false: "no"}; }"#);
    assert_compiles(r#"fn main() { let m: {Bool: Str} = {true: "yes", false: "no"}; }"#);
}

#[test]
fn test_map_with_expressions() {
    assert_compiles(r#"fn main() { let m = {"sum": 1 + 2, "product": 3 * 4}; }"#);
    assert_compiles(r#"fn main() { let m: {Str: Int} = {"sum": 1 + 2, "product": 3 * 4}; }"#);

    assert_compiles(r#"fn main() { let m = {"f": 1.1 + 2.2, "g": 3.3 * 4.4}; }"#);
    assert_compiles(r#"fn main() { let m: {Str: Float} = {"f": 1.1 + 2.2, "g": 3.3 * 4.4}; }"#);

    assert_compiles(r#"fn main() { let m = {"b": true && false, "c": !false}; }"#);
    assert_compiles(r#"fn main() { let m: {Str: Bool} = {"b": true && false, "c": !false}; }"#);

    assert_compiles(r#"fn main() { let m = {"s": "a" + "b", "t": "c" + "d"}; }"#);
    assert_compiles(r#"fn main() { let m: {Str: Str} = {"s": "a" + "b", "t": "c" + "d"}; }"#);

    assert_compiles(r#"fn main() { let m = {1: 2 + 3, 2: 4 * 5}; }"#);
    assert_compiles(r#"fn main() { let m: {Int: Int} = {1: 2 + 3, 2: 4 * 5}; }"#);

    assert_compiles(r#"fn main() { let m = {1: 1.1 + 2.2, 2: 3.3 * 4.4}; }"#);
    assert_compiles(r#"fn main() { let m: {Int: Float} = {1: 1.1 + 2.2, 2: 3.3 * 4.4}; }"#);

    assert_compiles(r#"fn main() { let m = {1: true && false, 2: !false}; }"#);
    assert_compiles(r#"fn main() { let m: {Int: Bool} = {1: true && false, 2: !false}; }"#);

    assert_compiles(r#"fn main() { let m = {1: "a" + "b", 2: "c" + "d"}; }"#);
    assert_compiles(r#"fn main() { let m: {Int: Str} = {1: "a" + "b", 2: "c" + "d"}; }"#);

    assert_compiles(r#"fn main() { let m = {1.1: 2 + 3, 2.2: 4 * 5}; }"#);
    assert_compiles(r#"fn main() { let m: {Float: Int} = {1.1: 2 + 3, 2.2: 4 * 5}; }"#);

    assert_compiles(r#"fn main() { let m = {1.1: 1.1 + 2.2, 2.2: 3.3 * 4.4}; }"#);
    assert_compiles(r#"fn main() { let m: {Float: Float} = {1.1: 1.1 + 2.2, 2.2: 3.3 * 4.4}; }"#);

    assert_compiles(r#"fn main() { let m = {1.1: true && false, 2.2: !false}; }"#);
    assert_compiles(r#"fn main() { let m: {Float: Bool} = {1.1: true && false, 2.2: !false}; }"#);

    assert_compiles(r#"fn main() { let m = {1.1: "a" + "b", 2.2: "c" + "d"}; }"#);
    assert_compiles(r#"fn main() { let m: {Float: Str} = {1.1: "a" + "b", 2.2: "c" + "d"}; }"#);

    assert_compiles(r#"fn main() { let m = {true: 2 + 3, false: 4 * 5}; }"#);
    assert_compiles(r#"fn main() { let m: {Bool: Int} = {true: 2 + 3, false: 4 * 5}; }"#);

    assert_compiles(r#"fn main() { let m = {true: 1.1 + 2.2, false: 3.3 * 4.4}; }"#);
    assert_compiles(r#"fn main() { let m: {Bool: Float} = {true: 1.1 + 2.2, false: 3.3 * 4.4}; }"#);

    assert_compiles(r#"fn main() { let m = {true: true && false, false: !false}; }"#);
    assert_compiles(r#"fn main() { let m: {Bool: Bool} = {true: true && false, false: !false}; }"#);

    assert_compiles(r#"fn main() { let m = {true: "a" + "b", false: "c" + "d"}; }"#);
    assert_compiles(r#"fn main() { let m: {Bool: Str} = {true: "a" + "b", false: "c" + "d"}; }"#);
}

#[test]
fn test_map_string_to_string() {
    assert_compiles(r#"fn main() { let m = {"hello": "world", "foo": "bar"}; }"#);
    assert_compiles(r#"fn main() { let m: {Str: Str} = {"hello": "world", "foo": "bar"}; }"#);
}

#[test]
fn test_map_string_to_float() {
    assert_compiles(r#"fn main() { let m = {"pi": 3.14, "e": 2.71}; }"#);
    assert_compiles(r#"fn main() { let m: {Str: Float} = {"pi": 3.14, "e": 2.71}; }"#);
}

#[test]
fn test_map_string_to_bool() {
    assert_compiles(r#"fn main() { let m = {"yes": true, "no": false}; }"#);
    assert_compiles(r#"fn main() { let m: {Str: Bool} = {"yes": true, "no": false}; }"#);
}

#[test]
fn test_nested_maps() {
    assert_compiles(r#"fn main() { let m = {"outer": {"inner": 42}}; }"#);
    assert_compiles(r#"fn main() { let m: {Str: {Str: Int}} = {"outer": {"inner": 42}}; }"#);
}

#[test]
fn test_map_key_type_mismatch() {
    assert_fails(r#"fn main() { let m = {"a": 1, 2: 3}; }"#);
    assert_fails(r#"fn main() { let m: {Str: Int} = {"a": 1, 2: 3}; }"#);
    assert_fails(r#"fn main() { let m: {Int: Int} = {1: 1, "b": 2}; }"#);
}

#[test]
fn test_map_value_type_mismatch() {
    assert_fails(r#"fn main() { let m = {"a": 1, "b": "hello"}; }"#);
    assert_fails(r#"fn main() { let m: {Str: Int} = {"a": 1, "b": "hello"}; }"#);
    assert_fails(r#"fn main() { let m: {Str: Float} = {"a": 1.1, "b": true}; }"#);
    assert_fails(r#"fn main() { let m: {Int: Bool} = {1: true, 2: "no"}; }"#);
}

// ========================================
// MAPS - ACCESS
// ========================================

#[test]
fn test_map_access_literal_key() {
    // {Str: Int}
    assert_compiles(r#"fn main() { let m = {"key": 42}; let x = m.get("key"); }"#);
    assert_compiles(r#"fn main() { let m: {Str: Int} = {"key": 42}; let x = m.get("key"); }"#);

    // {Str: Float}
    assert_compiles(r#"fn main() { let m = {"pi": 3.14}; let x = m.get("pi"); }"#);
    assert_compiles(r#"fn main() { let m: {Str: Float} = {"pi": 3.14}; let x = m.get("pi"); }"#);

    // {Str: Bool}
    assert_compiles(r#"fn main() { let m = {"yes": true}; let x = m.get("yes"); }"#);
    assert_compiles(r#"fn main() { let m: {Str: Bool} = {"yes": true}; let x = m.get("yes"); }"#);

    // {Str: Str}
    assert_compiles(r#"fn main() { let m = {"hello": "world"}; let x = m.get("hello"); }"#);
    assert_compiles(
        r#"fn main() { let m: {Str: Str} = {"hello": "world"}; let x = m.get("hello"); }"#,
    );

    // {Int: Int}
    assert_compiles(r#"fn main() { let m = {1: 42}; let x = m.get(1); }"#);
    assert_compiles(r#"fn main() { let m: {Int: Int} = {1: 42}; let x = m.get(1); }"#);

    // {Int: Float}
    assert_compiles(r#"fn main() { let m = {1: 3.14}; let x = m.get(1); }"#);
    assert_compiles(r#"fn main() { let m: {Int: Float} = {1: 3.14}; let x = m.get(1); }"#);

    // {Int: Bool}
    assert_compiles(r#"fn main() { let m = {1: true}; let x = m.get(1); }"#);
    assert_compiles(r#"fn main() { let m: {Int: Bool} = {1: true}; let x = m.get(1); }"#);

    // {Int: Str}
    assert_compiles(r#"fn main() { let m = {1: "hello"}; let x = m.get(1); }"#);
    assert_compiles(r#"fn main() { let m: {Int: Str} = {1: "hello"}; let x = m.get(1); }"#);

    // {Float: Int}
    assert_compiles(r#"fn main() { let m = {3.14: 42}; let x = m.get(3.14); }"#);
    assert_compiles(r#"fn main() { let m: {Float: Int} = {3.14: 42}; let x = m.get(3.14); }"#);

    // {Float: Float}
    assert_compiles(r#"fn main() { let m = {3.14: 2.71}; let x = m.get(3.14); }"#);
    assert_compiles(r#"fn main() { let m: {Float: Float} = {3.14: 2.71}; let x = m.get(3.14); }"#);

    // {Float: Bool}
    assert_compiles(r#"fn main() { let m = {3.14: true}; let x = m.get(3.14); }"#);
    assert_compiles(r#"fn main() { let m: {Float: Bool} = {3.14: true}; let x = m.get(3.14); }"#);

    // {Float: Str}
    assert_compiles(r#"fn main() { let m = {3.14: "hello"}; let x = m.get(3.14); }"#);
    assert_compiles(r#"fn main() { let m: {Float: Str} = {3.14: "hello"}; let x = m.get(3.14); }"#);

    // {Bool: Int}
    assert_compiles(r#"fn main() { let m = {true: 42}; let x = m.get(true); }"#);
    assert_compiles(r#"fn main() { let m: {Bool: Int} = {true: 42}; let x = m.get(true); }"#);

    // {Bool: Float}
    assert_compiles(r#"fn main() { let m = {false: 3.14}; let x = m.get(false); }"#);
    assert_compiles(r#"fn main() { let m: {Bool: Float} = {false: 3.14}; let x = m.get(false); }"#);

    // {Bool: Bool}
    assert_compiles(r#"fn main() { let m = {true: false}; let x = m.get(true); }"#);
    assert_compiles(r#"fn main() { let m: {Bool: Bool} = {true: false}; let x = m.get(true); }"#);

    // {Bool: Str}
    assert_compiles(r#"fn main() { let m = {false: "hello"}; let x = m.get(false); }"#);
    assert_compiles(
        r#"fn main() { let m: {Bool: Str} = {false: "hello"}; let x = m.get(false); }"#,
    );
}

#[test]
fn test_map_access_variable_key() {
    // {Str: Int}
    let code = r#"
       fn main() {
           let m = {"key": 42};
           let k = "key";
           let x = m.get(k);
       }
   "#;
    assert_compiles(code);

    let code = r#"
       fn main() {
           let m: {Str: Int} = {"key": 42};
           let k = "key";
           let x = m.get(k);
       }
   "#;
    assert_compiles(code);

    // {Int: Float}
    let code = r#"
       fn main() {
           let m = {1: 3.14};
           let k = 1;
           let x = m.get(k);
       }
   "#;
    assert_compiles(code);

    let code = r#"
       fn main() {
           let m: {Int: Float} = {1: 3.14};
           let k = 1;
           let x = m.get(k);
       }
   "#;
    assert_compiles(code);

    // {Bool: Str}
    let code = r#"
       fn main() {
           let m = {true: "yes"};
           let k = true;
           let x = m.get(k);
       }
   "#;
    assert_compiles(code);

    let code = r#"
       fn main() {
           let m: {Bool: Str} = {true: "yes"};
           let k = true;
           let x = m.get(k);
       }
   "#;
    assert_compiles(code);
}

#[test]
fn test_map_access_multiple_keys() {
    // {Str: Int}
    let code = r#"
       fn main() {
           let m = {"a": 1, "b": 2, "c": 3};
           let x = m.get("a");
           let y = m.get("b");
           let z = m.get("c");
       }
   "#;
    assert_compiles(code);

    let code = r#"
       fn main() {
           let m: {Str: Int} = {"a": 1, "b": 2, "c": 3};
           let x = m.get("a");
           let y = m.get("b");
           let z = m.get("c");
       }
   "#;
    assert_compiles(code);

    // {Int: Bool}
    let code = r#"
       fn main() {
           let m = {1: true, 2: false};
           let x = m.get(1);
           let y = m.get(2);
       }
   "#;
    assert_compiles(code);

    let code = r#"
       fn main() {
           let m: {Int: Bool} = {1: true, 2: false};
           let x = m.get(1);
           let y = m.get(2);
       }
   "#;
    assert_compiles(code);

    // {Float: Str}
    let code = r#"
       fn main() {
           let m = {1.1: "one", 2.2: "two"};
           let x = m.get(1.1);
           let y = m.get(2.2);
       }
   "#;
    assert_compiles(code);

    let code = r#"
       fn main() {
           let m: {Float: Str} = {1.1: "one", 2.2: "two"};
           let x = m.get(1.1);
           let y = m.get(2.2);
       }
   "#;
    assert_compiles(code);
}

#[test]
fn test_map_iteration() {
    // {Str: Int}
    let code = r#"
       fn main() {
           let m = {"a": 1, "b": 2};
           for (key, val) in m {
               print(key);
               print(val);
           }
       }
   "#;
    assert_compiles(code);

    let code = r#"
       fn main() {
           let m: {Str: Int} = {"a": 1, "b": 2};
           for (key, val) in m {
               print(key);
               print(val);
           }
       }
   "#;
    assert_compiles(code);

    // {Int: Float}
    let code = r#"
       fn main() {
           let m = {1: 1.1, 2: 2.2};
           for (key, val) in m {
               print(key);
               print(val);
           }
       }
   "#;
    assert_compiles(code);

    let code = r#"
       fn main() {
           let m: {Int: Float} = {1: 1.1, 2: 2.2};
           for (key, val) in m {
               print(key);
               print(val);
           }
       }
   "#;
    assert_compiles(code);

    // {Bool: Str}
    let code = r#"
       fn main() {
           let m = {true: "yes", false: "no"};
           for (key, val) in m {
               print(key);
               print(val);
           }
       }
   "#;
    assert_compiles(code);

    let code = r#"
       fn main() {
           let m: {Bool: Str} = {true: "yes", false: "no"};
           for (key, val) in m {
               print(key);
               print(val);
           }
       }
   "#;
    assert_compiles(code);
}
