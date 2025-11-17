//! Memory and Stress Tests
//! Tests compiler behavior under heavy load and realistic edge cases
//! Focus: Real-world stress scenarios, not excessive duplication

use super::super::common::assert_compiles;

// ========================================
// LARGE VARIABLE/FUNCTION COUNTS
// ========================================

#[test]
fn test_many_variables() {
    let mut code = String::from("fn main() {\n");
    for i in 0..100 {
        code.push_str(&format!("    let x{} = {};\n", i, i));
    }
    code.push_str("}");
    assert_compiles(&code);
}

#[test]
fn test_many_functions() {
    let mut code = String::new();
    for i in 0..50 {
        code.push_str(&format!("fn func{}() {{ }}\n", i));
    }
    code.push_str("fn main() { }");
    assert_compiles(&code);
}

#[test]
fn test_many_functions_with_parameters() {
    let mut code = String::new();
    for i in 0..30 {
        code.push_str(&format!(
            "fn func{}(a: Int, b: Str, c: Float) -> Int {{ return {}; }}\n",
            i, i
        ));
    }
    code.push_str("fn main() { }");
    assert_compiles(&code);
}

// ========================================
// LARGE COLLECTIONS
// ========================================

#[test]
fn test_large_array() {
    let elements: Vec<String> = (0..1000).map(|i| i.to_string()).collect();
    let code = format!("fn main() {{ let arr = [{}]; }}", elements.join(", "));
    assert_compiles(&code);
}

#[test]
fn test_large_map() {
    let mut entries = Vec::new();
    for i in 0..100 {
        entries.push(format!("\"key{}\": {}", i, i));
    }
    let code = format!("fn main() {{ let m = {{{}}}; }}", entries.join(", "));
    assert_compiles(&code);
}

#[test]
fn test_array_of_simple_maps() {
    let code = r#"
        fn main() {
            let arr = [1, 2, 3, 4, 5];
            let mapped = arr.map((x) => x * 2);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_map_with_various_values() {
    let mut entries = Vec::new();
    for i in 0..30 {
        entries.push(format!("\"key{}\": {}", i, i));
    }
    let code = format!("fn main() {{ let m = {{{}}}; }}", entries.join(", "));
    assert_compiles(&code);
}

#[test]
fn test_large_string_array() {
    let mut entries = Vec::new();
    for i in 0..100 {
        entries.push(format!("\"string{}\"", i));
    }
    let code = format!("fn main() {{ let arr = [{}]; }}", entries.join(", "));
    assert_compiles(&code);
}

#[test]
fn test_large_float_array() {
    let elements: Vec<String> = (0..500)
        .map(|i| format!("{}.{}", i % 100, i % 10))
        .collect();
    let code = format!("fn main() {{ let arr = [{}]; }}", elements.join(", "));
    assert_compiles(&code);
}

// ========================================
// DEEP NESTING
// ========================================

#[test]
fn test_deeply_nested_expressions() {
    let mut expr = String::from("1");
    for _ in 0..50 {
        expr = format!("({} + 1)", expr);
    }
    let code = format!("fn main() {{ let x = {}; }}", expr);
    assert_compiles(&code);
}

#[test]
fn test_deeply_nested_blocks() {
    let mut code = String::from("fn main() {\n");
    for _ in 0..30 {
        code.push_str("    if true {\n");
    }
    code.push_str("        let x = 1;\n");
    for _ in 0..30 {
        code.push_str("    }\n");
    }
    code.push_str("}");
    assert_compiles(&code);
}

#[test]
fn test_deeply_nested_with_variables() {
    let mut code = String::from("fn main() {\n");
    for i in 0..3 {
        code.push_str(&format!("    let v{} = {};\n", i, i));
        code.push_str("    if true {\n");
    }
    code.push_str("        let finalVar = 42;\n");
    for _ in 0..3 {
        code.push_str("    }\n");
    }
    code.push_str("}");
    assert_compiles(&code);
}

#[test]
fn test_deeply_nested_function_calls() {
    let mut code = String::from("fn add(a: Int, b: Int) -> Int { return a + b; }\n");
    code.push_str("fn main() {\n");

    let mut expr = String::from("1");
    for i in 0..20 {
        expr = format!("add({}, {})", expr, i);
    }
    code.push_str(&format!("    let result = {};\n", expr));
    code.push_str("}");
    assert_compiles(&code);
}

// ========================================
// LOOPS WITH MANY ITERATIONS
// ========================================

#[test]
fn test_many_loop_iterations() {
    let code = r#"
        fn main() {
            let mut sum = 0;
            for i in 0..10000 {
                sum += i;
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_nested_loops_many_iterations() {
    let code = r#"
        fn main() {
            let mut total = 0;
            for i in 0..100 {
                for j in 0..100 {
                    total += i * j;
                }
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_triple_nested_loops() {
    let code = r#"
        fn main() {
            let mut result = 0;
            for i in 0..20 {
                for j in 0..20 {
                    for k in 0..20 {
                        result += i + j + k;
                    }
                }
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_loop_with_array_operations() {
    let code = r#"
        fn main() {
            let mut arr: [Int] = [];
            for i in 0..1000 {
                arr.push(i);
            }
        }
    "#;
    assert_compiles(code);
}

// ========================================
// COMPLEX METHOD CHAINING
// ========================================

#[test]
fn test_complex_method_chain() {
    let code = r#"
        fn main() {
            let arr = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
            let result = arr
                .map((x) => x * 2)
                .filter((x) => x > 5)
                .map((x) => x - 1)
                .filter((x) => x % 2 == 0)
                .map((x) => x / 2)
                .reduce(0, (acc, x) => acc + x);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_long_method_chain() {
    let code = r#"
        fn main() {
            let arr = [1, 2, 3, 4, 5];
            let result = arr
                .map((x) => x * 2)
                .map((x) => x + 1)
                .map((x) => x - 1)
                .filter((x) => x > 0)
                .map((x) => x * x)
                .filter((x) => x < 100)
                .map((x) => x / 2);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_chained_operations_with_large_array() {
    let elements: Vec<String> = (0..100).map(|i| i.to_string()).collect();
    let code = format!(
        r#"fn main() {{
            let arr = [{}];
            let filtered = arr.filter((x) => x > 5);
            let mapped = filtered.map((x) => x * 2);
            let result = mapped.reduce(0, (a, b) => a + b);
        }}"#,
        elements.join(", ")
    );
    assert_compiles(&code);
}

// ========================================
// DEEP RECURSION
// ========================================

#[test]
fn test_deeply_recursive_function() {
    let code = r#"
        fn recurse(n: Int) -> Int {
            if n <= 0 {
                return 0;
            }
            return 1 + recurse(n - 1);
        }
        fn main() {
            let result = recurse(100);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_mutual_recursion() {
    let code = r#"
        fn even(n: Int) -> Bool {
            if n == 0 {
                return true;
            }
            return odd(n - 1);
        }

        fn odd(n: Int) -> Bool {
            if n == 0 {
                return false;
            }
            return even(n - 1);
        }

        fn main() {
            let result1 = even(10);
            let result2 = odd(10);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_recursive_function_with_array() {
    let code = r#"
        fn sumRecursive(arr: [Int], idx: Int) -> Int {
            if idx >= arr.len() {
                return 0;
            }
            return arr[idx] + sumRecursive(arr, idx + 1);
        }

        fn main() {
            let arr = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
            let total = sumRecursive(arr, 0);
        }
    "#;
    assert_compiles(code);
}

// ========================================
// EDGE CASES
// ========================================

#[test]
fn test_empty_array_operations() {
    let code = r#"
        fn main() {
            let mut empty: [Int] = [];
            let mapped = empty.map((x) => x * 2);
            let filtered = empty.filter((x) => x > 0);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_single_element_collections() {
    let code = r#"
        fn main() {
            let arr = [42];
            let val = arr[0];

            let m = {"key": 1};
            let v = m.get("key");
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_empty_map_operations() {
    let code = r#"
        fn main() {
            let mut m: {Str: Int} = {};
        }
    "#;
    assert_compiles(code);
}

// ========================================
// COMBINED STRESS SCENARIOS
// ========================================

#[test]
fn test_many_variables_with_large_arrays() {
    let mut code = String::from("fn main() {\n");
    for i in 0..20 {
        let elements: Vec<String> = (0..50).map(|j| (i * 50 + j).to_string()).collect();
        code.push_str(&format!("    let arr{} = [{}];\n", i, elements.join(", ")));
    }
    code.push_str("}");
    assert_compiles(&code);
}

#[test]
fn test_many_variables_with_large_maps() {
    let mut code = String::from("fn main() {\n");
    for i in 0..15 {
        let mut entries = Vec::new();
        for j in 0..30 {
            entries.push(format!("\"k{}\": {}", j, i * 100 + j));
        }
        code.push_str(&format!("    let m{} = {{{}}};\n", i, entries.join(", ")));
    }
    code.push_str("}");
    assert_compiles(&code);
}

#[test]
fn test_nested_loops_with_array_operations() {
    let code = r#"
        fn main() {
            let mut result = 0;
            for i in 0..10 {
                for j in 0..20 {
                    result = result + i + j;
                }
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_function_returning_large_collection() {
    let code = r#"
        fn createDict() -> {Str: Int} {
            return {"a": 1, "b": 2, "c": 3};
        }

        fn main() {
            let result = createDict();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_function_returning_large_array() {
    let code = r#"
        fn createArr() -> [Int] {
            return [1, 2, 3, 4, 5];
        }

        fn main() {
            let result = createArr();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_multiple_nested_scopes() {
    let code = r#"
        fn main() {
            let x = 1;
            if true {
                let y = 2;
                if true {
                    let z = 3;
                }
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_many_conditional_branches() {
    let mut code = String::from("fn main() {\n");
    code.push_str("    let mut x = 0;\n");
    for i in 0..10 {
        code.push_str(&format!("    if x == {} {{ x = {}; }} else ", i, i + 1));
    }
    code.push_str("{ x = 100; }\n}");
    assert_compiles(&code);
}

// ========================================
// MEMORY ALLOCATION PATTERNS
// ========================================

#[test]
fn test_repeated_array_allocation() {
    let code = r#"
        fn main() {
            for i in 0..1000000 {
                let arr = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_repeated_map_allocation() {
    let code = r#"
        fn main() {
            for i in 0..50 {
                let m = {"a": 1, "b": 2, "c": 3};
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_array_growth_pattern() {
    let code = r#"
        fn main() {
            let mut arr: [Int] = [];
            for i in 0..100 {
                arr.push(i);
                for j in 0..10 {
                    arr.push(i * 10 + j);
                }
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_string_operations() {
    let code = r#"
        fn main() {
            let s1 = "hello";
            let s2 = "world";
            let s3 = s1;
            let s4 = s2;
        }
    "#;
    assert_compiles(code);
}

// ========================================
// TYPE COMPLEXITY
// ========================================

#[test]
fn test_complex_map_types() {
    let code = r#"
        fn main() {
            let m1: {Str: Int} = {"a": 1};
            let m2: {Str: Float} = {"b": 2.0};
            let m3: {Str: Bool} = {"c": true};
            let m4: {Int: Str} = {1: "x"};
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_many_type_annotations() {
    let mut code = String::from("fn main() {\n");
    code.push_str("    let v1: Int = 1;\n");
    code.push_str("    let v2: Float = 1.0;\n");
    code.push_str("    let v3: Str = \"str\";\n");
    code.push_str("    let v4: Bool = true;\n");
    for i in 0..30 {
        match i % 4 {
            0 => code.push_str(&format!("    let v{}: [Int] = [{}];\n", i + 5, i)),
            1 => code.push_str(&format!("    let v{}: [Float] = [{}.0];\n", i + 5, i)),
            2 => code.push_str(&format!(
                "    let v{}: {{Str: Int}} = {{\"key\": {}}};\n",
                i + 5,
                i
            )),
            _ => code.push_str(&format!(
                "    let v{}: {{Str: Str}} = {{\"a\": \"b\"}};\n",
                i + 5
            )),
        }
    }
    code.push_str("}");
    assert_compiles(&code);
}

// ========================================
// STRESS WITH FUNCTION DEFINITIONS
// ========================================

#[test]
fn test_many_function_definitions() {
    let mut code = String::new();
    code.push_str("fn processInt(x: Int) -> Int { return x * 2; }\n");
    code.push_str("fn processFloat(x: Float) -> Float { return x * 2.0; }\n");
    code.push_str("fn processStr(x: Str) -> Str { return x; }\n");
    code.push_str("fn processBool(x: Bool) -> Bool { return x; }\n");

    for i in 0..5 {
        code.push_str(&format!("fn helper{}(a: Int) -> Int {{ return a; }}\n", i));
    }
    code.push_str("fn main() { }");
    assert_compiles(&code);
}

#[test]
fn test_stress_function_calls() {
    let mut code = String::from("fn add(a: Int, b: Int) -> Int { return a + b; }\n");
    code.push_str("fn main() {\n");
    code.push_str("    let mut result = 0;\n");
    for i in 0..1000 {
        code.push_str(&format!("    result = add(result, {});\n", i % 10));
    }
    code.push_str("}");
    assert_compiles(&code);
}
