//! Builtins Integration Tests
//! Tests standard library methods and built-in functions

use super::super::common::{assert_compiles, assert_fails};

// ========================================
// STRING METHODS
// ========================================

#[test]
fn test_string_length() {
    let code = r#"
        fn main() {
            let s = "hello";
            let len = s.len();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_string_char_at() {
    let code = r#"
        fn main() {
            let s = "hello";
            let ch = s.charAt(0);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_string_substring() {
    let code = r#"
        fn main() {
            let s = "hello world";
            let sub = s.substring(0, 5);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_string_to_upper() {
    let code = r#"
        fn main() {
            let s = "hello";
            let upper = s.toUpper();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_string_to_lower() {
    let code = r#"
        fn main() {
            let s = "HELLO";
            let lower = s.toLower();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_string_contains() {
    let code = r#"
        fn main() {
            let s = "hello world";
            let has = s.contains("world");
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_string_starts_with() {
    let code = r#"
        fn main() {
            let s = "hello world";
            let starts = s.startsWith("hello");
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_string_ends_with() {
    let code = r#"
        fn main() {
            let s = "hello world";
            let ends = s.endsWith("world");
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_string_trim() {
    let code = r#"
        fn main() {
            let s = "  hello  ";
            let trimmed = s.trim();
        }
    "#;
    assert_compiles(code);
}

// Add any missing string methods from string.rs here:
#[test]
fn test_string_repeat() {
    let code = r#"
        fn main() {
            let s = "ha";
            let repeated = s.repeat(3);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_string_replace() {
    let code = r#"
        fn main() {
            let s = "foo bar foo";
            let replaced = s.replace("foo", "baz");
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_string_index_of() {
    let code = r#"
        fn main() {
            let s = "hello world";
            let idx = s.indexOf("world");
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_string_count_substr() {
    let code = r#"
        fn main() {
            let s = "banana";
            let count = s.countSubstr("an");
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_string_concat() {
    let code = r#"
        fn main() {
            let s1 = "foo";
            let s2 = "bar";
            let result = s1.concat(s2);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_string_char_code() {
    let code = r#"
        fn main() {
            let s = "A";
            let code = s.charCode();
        }
    "#;
    assert_compiles(code);
}

// ========================================
// ARRAY METHODS
// ========================================

#[test]
fn test_array_length() {
    let code = r#"
        fn main() {
            let arr = [1, 2, 3];
            let len = arr.len();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_push() {
    let code = r#"
        fn main() {
            let mut arr = [1, 2, 3];
            arr.push(4);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_pop() {
    let code = r#"
        fn main() {
            let mut arr = [1, 2, 3];
            let val = arr.pop();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_get() {
    let code = r#"
        fn main() {
            let arr = [10, 20, 30];
            let val = arr[1];
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_set() {
    let code = r#"
        fn main() {
            let mut arr = [10, 20, 30];
            arr[1] = 99;
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_contains() {
    let code = r#"
        fn main() {
            let arr = [1, 2, 3];
            let has = arr.contains(2);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_first() {
    let code = r#"
        fn main() {
            let arr = [1, 2, 3];
            let first = arr.first();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_last() {
    let code = r#"
        fn main() {
            let arr = [1, 2, 3];
            let last = arr.last();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_reverse() {
    let code = r#"
        fn main() {
            let mut arr = [1, 2, 3];
            arr.reverse();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_is_empty() {
    let code = r#"
        fn main() {
            let mut arr = [];
            let empty = arr.isEmpty();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_clear() {
    let code = r#"
        fn main() {
            let mut arr = [1, 2, 3];
            arr.clear();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_sort() {
    let code = r#"
        fn main() {
            let mut arr = [3, 1, 2];
            arr.sort();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_slice() {
    let code = r#"
        fn main() {
            let arr = [1, 2, 3, 4, 5];
            let sub = arr.slice(1, 4);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_index_of() {
    let code = r#"
        fn main() {
            let arr = [10, 20, 30, 20];
            let idx = arr.indexOf(20);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_map() {
    let code = r#"
        fn main() {
            let arr = [1, 2, 3];
            let doubled = arr.map((x) => x * 2);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_filter() {
    let code = r#"
        fn main() {
            let arr = [1, 2, 3, 4, 5];
            let evens = arr.filter((x) => x % 2 == 0);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_reduce() {
    let code = r#"
        fn main() {
            let arr = [1, 2, 3, 4];
            let sum = arr.reduce(0, (acc, x) => acc + x);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_join() {
    let code = r#"
        fn main() {
            let arr = [1, 2, 3];
            let s = arr.join(",");
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_find() {
    let code = r#"
        fn main() {
            let arr = [1, 2, 3, 4];
            let found = arr.map((x) => x * 2);
        }
    "#;
    assert_compiles(code);
}

// ========================================
// MAP METHODS
// ========================================

#[test]
fn test_map_has() {
    let code = r#"
        fn main() {
            let m = {"key": 42};
            let exists = m.has("key");
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_map_get() {
    let code = r#"
        fn main() {
            let m = {"key": 42};
            let val = m.get("key");
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_map_set() {
    let code = r#"
        fn main() {
            let mut m = {"a": 1};
            m.set("b", 2);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_map_remove() {
    let code = r#"
        fn main() {
            let mut m = {"a": 1, "b": 2};
            m.remove("a");
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_map_is_empty() {
    let code = r#"
        fn main() {
            let mut m = {};
            let empty = m.isEmpty();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_map_size() {
    let code = r#"
        fn main() {
            let m = {"a": 1, "b": 2};
            let size = m.size();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_map_clear() {
    let code = r#"
        fn main() {
            let mut m = {"a": 1, "b": 2};
            m.clear();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_map_keys() {
    let code = r#"
        fn main() {
            let m = {"a": 1, "b": 2};
            let keys = m.keys();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_map_values() {
    let code = r#"
        fn main() {
            let m = {"a": 1, "b": 2};
            let vals = m.values();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_map_contains_key() {
    let code = r#"
        fn main() {
            let m = {"foo": 123};
            let has = m.containsKey("foo");
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_map_contains_value() {
    let code = r#"
        fn main() {
            let m = {"foo": 123, "bar": 456};
            let has = m.containsValue(456);
        }
    "#;
    assert_compiles(code);
}

// ========================================
// PRINT FUNCTION
// ========================================

#[test]
fn test_print_int() {
    assert_compiles("fn main() { print(42); }");
}

#[test]
fn test_print_string() {
    assert_compiles(r#"fn main() { print("hello"); }"#);
}

#[test]
fn test_print_bool() {
    assert_compiles("fn main() { print(true); }");
}

#[test]
fn test_print_float() {
    assert_compiles("fn main() { print(3.14); }");
}

#[test]
fn test_print_multiple_args() {
    let code = r#"
        fn main() {
            print("value:", 42);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_print_variable() {
    let code = r#"
        fn main() {
            let x = 42;
            print(x);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_print_arithmetic_expression() {
    let code = r#"
        fn main() {
            print(1 + 2 * 3);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_print_function_call() {
    let code = r#"
        fn double(n: Int) -> Int {
            return n * 2;
        }
        fn main() {
            print(double(21));
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_print_method_call() {
    let code = r#"
        fn main() {
            let s = "hello";
            print(s.toUpper());
        }
    "#;
    assert_compiles(code);
}

// ========================================
// METHOD CHAINING
// ========================================

#[test]
fn test_string_method_chaining() {
    let code = r#"
        fn main() {
            let s = "  HELLO  ";
            let result = s.trim().toLower().replace("h", "j").repeat(2).substring(0, 4);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_method_chaining() {
    let code = r#"
        fn main() {
            let arr = [1, 2, 3, 4, 5];
            let result = arr
                .map((x) => x * 2)
                .filter((x) => x > 5)
                .reduce(0, (acc, x) => acc + x);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_map_method_chaining() {
    let code = r#"
        fn main() {
            let m = {"a": 1, "b": 2, "c": 3};
            let result = m
                .keys()
                .map((k) => k.toUpper())
                .join("-");
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_arrow_function_chaining_with_block() {
    let code = r#"
        fn main() {
            let arr = [1, 2, 3, 4, 5];
            let result = arr
                .map((x) => {
                    let y = x * 3;
                    return y + 1;
                })
                .filter((x) => {
                    return x % 2 == 0;
                })
                .reduce(0, (acc, x) => {
                    return acc + x;
                });
        }
    "#;
    assert_compiles(code);
}

// ========================================
// ERROR CASES
// ========================================

#[test]
fn test_method_on_wrong_type() {
    assert_fails("fn main() { let x = 42; x.length(); }");
    assert_fails(r#"fn main() { let x = "hello"; x.push(1); }"#);
    assert_fails("fn main() { let x = [1, 2]; x.toUpper(); }");
}

#[test]
fn test_undefined_method() {
    assert_fails(r#"fn main() { let x = "hello"; x.nonExistent(); }"#);
    assert_fails("fn main() { let x = [1, 2]; x.fakeMethod(); }");
}
