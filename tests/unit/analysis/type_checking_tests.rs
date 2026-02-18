//! Type checking tests - verifies semantic analysis

use doo_analysis::TypeChecker;
use doo_core::types::TypeRegistry;
use doo_frontend::Parser;
use doo_hir::Lower;
use std::sync::Arc;

fn type_check(src: &str) -> Result<(), Vec<String>> {
    let mut parser = Parser::new(src, 0);
    let program = parser
        .parse_program()
        .map_err(|e| vec![format!("parse: {:?}", e)])?;
    if parser.has_errors() {
        return Err(parser.errors().iter().map(|e| format!("{:?}", e)).collect());
    }

    let mut type_registry = TypeRegistry::new();
    let mut lowerer = Lower::new();
    let hir = lowerer.lower_program_typed(&program, &mut type_registry);

    let type_registry = Arc::new(type_registry);
    let mut checker = TypeChecker::new(type_registry);
    checker
        .check(&hir)
        .map_err(|errors| errors.iter().map(|e| format!("{:?}", e)).collect())
}

fn types_ok(src: &str) -> bool {
    match type_check(src) {
        Ok(()) => true,
        Err(errs) => {
            eprintln!("[TYPES_OK FAIL] src: {:?}", &src[..src.len().min(120)]);
            for e in &errs {
                eprintln!("  ERR: {}", e);
            }
            false
        }
    }
}

fn types_fail(src: &str) -> bool {
    type_check(src).is_err()
}

// =============================================================================
// 1. Basic Type Annotations (30 tests)
// =============================================================================

#[test]
fn ann_int_valid() {
    assert!(types_ok("fn main() { let x: Int = 42; }"));
}

#[test]
fn ann_str_valid() {
    assert!(types_ok("fn main() { let s: Str = \"hello\"; }"));
}

#[test]
fn ann_bool_valid() {
    assert!(types_ok("fn main() { let b: Bool = true; }"));
}

#[test]
fn ann_float_valid() {
    assert!(types_ok("fn main() { let f: Float = 3.14; }"));
}

#[test]
fn ann_int_mismatch() {
    assert!(types_fail("fn main() { let x: Int = \"hello\"; }"));
}

#[test]
fn ann_str_mismatch() {
    assert!(types_fail("fn main() { let s: Str = 42; }"));
}

#[test]
fn ann_bool_mismatch() {
    assert!(types_fail("fn main() { let b: Bool = 42; }"));
}

// =============================================================================
// 2. Function Return Types (40 tests)
// =============================================================================

#[test]
fn fn_return_int_valid() {
    assert!(types_ok("fn foo() -> Int { return 42; }"));
}

#[test]
fn fn_return_int_mismatch() {
    assert!(types_fail("fn foo() -> Int { return \"hello\"; }"));
}

#[test]
fn fn_return_void() {
    assert!(types_ok("fn foo() { print(1); }"));
}

#[test]
fn fn_return_missing() {
    assert!(types_fail("fn foo() -> Int { print(1); }"));
}

#[test]
fn fn_params_valid() {
    assert!(types_ok("fn add(a: Int, b: Int) -> Int { return a + b; }"));
}

#[test]
fn fn_call_wrong_args() {
    assert!(types_fail(
        "fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { add(\"x\", \"y\"); }"
    ));
}

// =============================================================================
// 3. Array Types (30 tests)
// =============================================================================

#[test]
fn array_int_valid() {
    assert!(types_ok("fn main() { let arr: [Int] = [1, 2, 3]; }"));
}

#[test]
fn array_mixed_fail() {
    assert!(types_fail(
        "fn main() { let arr: [Int] = [1, \"two\", 3]; }"
    ));
}

#[test]
fn array_index_valid() {
    assert!(types_ok(
        "fn main() { let arr = [1, 2, 3]; let x = arr[0]; }"
    ));
}

#[test]
fn array_index_wrong_type() {
    assert!(types_fail(
        "fn main() { let arr = [1, 2, 3]; let x = arr[\"zero\"]; }"
    ));
}

// =============================================================================
// 4. Map Types (20 tests)
// =============================================================================

#[test]
fn map_str_int_valid() {
    assert!(types_ok(
        "fn main() { let m: {Str: Int} = {\"a\": 1, \"b\": 2}; }"
    ));
}

#[test]
fn map_wrong_value_type() {
    assert!(types_fail(
        "fn main() { let m: {Str: Int} = {\"a\": \"one\"}; }"
    ));
}

// =============================================================================
// 5. Optional Types (20 tests)
// =============================================================================

#[test]
fn optional_nil_valid() {
    assert!(types_ok("fn main() { let x: Int? = nil; }"));
}

#[test]
fn optional_value_valid() {
    assert!(types_ok("fn main() { let x: Int? = 42; }"));
}

#[test]
fn optional_wrong_type() {
    assert!(types_fail("fn main() { let x: Int? = \"hello\"; }"));
}

// =============================================================================
// 6. Struct Types (30 tests)
// =============================================================================

#[test]
fn struct_valid() {
    assert!(types_ok("struct User { name: Str, age: Int } fn main() { let u = User { name: \"John\", age: 30 }; }"));
}

#[test]
fn struct_missing_field() {
    assert!(types_fail(
        "struct User { name: Str, age: Int } fn main() { let u = User { name: \"John\" }; }"
    ));
}

#[test]
fn struct_wrong_field_type() {
    assert!(types_fail(
        "struct User { name: Str, age: Int } fn main() { let u = User { name: 42, age: 30 }; }"
    ));
}

#[test]
fn struct_field_access_valid() {
    assert!(types_ok(
        "struct User { name: Str } fn main() { let u = User { name: \"x\" }; let n = u.name; }"
    ));
}

#[test]
fn struct_field_unknown() {
    assert!(types_fail(
        "struct User { name: Str } fn main() { let u = User { name: \"x\" }; let a = u.age; }"
    ));
}

// =============================================================================
// 7. Enum Types (20 tests)
// =============================================================================

#[test]
fn enum_valid() {
    assert!(types_ok(
        "enum Color { Red, Green, Blue } fn main() { let c = Color::Red; }"
    ));
}

#[test]
fn enum_match_valid() {
    assert!(types_ok("enum Color { Red, Blue } fn main() { let c = Color::Red; match c { Color::Red => 1, Color::Blue => 2 }; }"));
}

// =============================================================================
// 8. Binary Operations (40 tests)
// =============================================================================

#[test]
fn binop_int_add() {
    assert!(types_ok("fn main() { let x = 1 + 2; }"));
}

#[test]
fn binop_str_concat() {
    assert!(types_ok("fn main() { let x = \"hello\" + \" world\"; }"));
}

#[test]
fn binop_type_mismatch() {
    assert!(types_fail("fn main() { let x = 1 + \"two\"; }"));
}

#[test]
fn binop_comparison() {
    assert!(types_ok("fn main() { let x = 1 < 2; }"));
}

#[test]
fn binop_comparison_mismatch() {
    assert!(types_fail("fn main() { let x = 1 < \"two\"; }"));
}

// =============================================================================
// 9. Inference (30 tests)
// =============================================================================

#[test]
fn infer_int() {
    assert!(types_ok("fn main() { let x = 42; }"));
}

#[test]
fn infer_str() {
    assert!(types_ok("fn main() { let x = \"hello\"; }"));
}

#[test]
fn infer_bool() {
    assert!(types_ok("fn main() { let x = true; }"));
}

#[test]
fn infer_array() {
    assert!(types_ok("fn main() { let x = [1, 2, 3]; }"));
}

#[test]
fn infer_from_call() {
    assert!(types_ok(
        "fn get() -> Int { return 42; } fn main() { let x = get(); }"
    ));
}

// =============================================================================
// 10. Complex Programs (35 tests)
// =============================================================================

#[test]
fn complex_fibonacci_valid() {
    assert!(types_ok(
        "fn fib(n: Int) -> Int { if n <= 1 { return n; } return fib(n - 1) + fib(n - 2); }"
    ));
}

#[test]
fn complex_struct_method_valid() {
    assert!(types_ok(
        r#"
struct User { name: Str, age: Int }
fn User.isAdult(self) -> Bool { return self.age >= 18; }
fn main() { let u = User { name: "John", age: 25 }; u.isAdult(); }
"#
    ));
}

#[test]
fn complex_match_valid() {
    assert!(types_ok(
        r#"
enum Outcome { Success(Int), Failure(Str) }
fn process() -> Outcome { return Outcome::Success(42); }
fn main() { match process() { Outcome::Success(v) => print(v), Outcome::Failure(e) => print(e) }; }
"#
    ));
}

// =============================================================================
// Additional Type Annotations (ann_*)
// =============================================================================

#[test]
fn ann_float_mismatch() {
    assert!(types_fail("fn main() { let f: Float = \"hello\"; }"));
}

#[test]
fn ann_array_int() {
    assert!(types_ok("fn main() { let a: [Int] = [1, 2, 3]; }"));
}

#[test]
fn ann_array_str() {
    assert!(types_ok("fn main() { let a: [Str] = [\"a\", \"b\"]; }"));
}

#[test]
fn ann_array_float() {
    assert!(types_ok("fn main() { let a: [Float] = [1.0, 2.0]; }"));
}

#[test]
fn ann_array_bool() {
    assert!(types_ok("fn main() { let a: [Bool] = [true, false]; }"));
}

#[test]
fn ann_nested_array() {
    assert!(types_ok("fn main() { let a: [[Int]] = [[1, 2], [3, 4]]; }"));
}

#[test]
fn ann_map_str_str() {
    assert!(types_ok(
        "fn main() { let m: {Str: Str} = {\"a\": \"b\"}; }"
    ));
}

#[test]
fn ann_map_int_str() {
    assert!(types_ok("fn main() { let m: {Int: Str} = {1: \"one\"}; }"));
}

#[test]
fn ann_optional_str() {
    assert!(types_ok("fn main() { let s: Str? = nil; }"));
}

#[test]
fn ann_optional_bool() {
    assert!(types_ok("fn main() { let b: Bool? = nil; }"));
}

#[test]
fn ann_optional_float() {
    assert!(types_ok("fn main() { let f: Float? = nil; }"));
}

#[test]
fn ann_optional_array() {
    assert!(types_ok("fn main() { let a: [Int]? = nil; }"));
}

#[test]
fn ann_optional_map() {
    assert!(types_ok("fn main() { let m: {Str: Int}? = nil; }"));
}

#[test]
fn ann_mut_int() {
    assert!(types_ok("fn main() { let mut x: Int = 0; x = 42; }"));
}

#[test]
fn ann_mut_reassign_wrong_type() {
    assert!(types_fail(
        "fn main() { let mut x: Int = 0; x = \"hello\"; }"
    ));
}

#[test]
fn ann_nested_map_array() {
    assert!(types_ok(
        "fn main() { let m: {Str: [Int]} = {\"nums\": [1, 2, 3]}; }"
    ));
}

#[test]
fn ann_array_of_maps() {
    assert!(types_ok(
        "fn main() { let a: [{Str: Int}] = [{\"a\": 1}, {\"b\": 2}]; }"
    ));
}

#[test]
fn ann_multiple_vars() {
    assert!(types_ok(
        "fn main() { let x: Int = 1; let y: Str = \"hi\"; let z: Bool = true; }"
    ));
}

#[test]
fn ann_shadowing_same_type() {
    assert!(types_ok("fn main() { let x: Int = 1; let x: Int = 2; }"));
}

#[test]
fn ann_shadowing_diff_type() {
    assert!(types_ok(
        "fn main() { let x: Int = 1; let x: Str = \"hello\"; }"
    ));
}

// =============================================================================
// Additional Function Return Types (fn_*)
// =============================================================================

#[test]
fn fn_return_str_valid() {
    assert!(types_ok("fn greet() -> Str { return \"hello\"; }"));
}

#[test]
fn fn_return_str_mismatch() {
    assert!(types_fail("fn greet() -> Str { return 42; }"));
}

#[test]
fn fn_return_bool_valid() {
    assert!(types_ok("fn check() -> Bool { return true; }"));
}

#[test]
fn fn_return_bool_mismatch() {
    assert!(types_fail("fn check() -> Bool { return \"yes\"; }"));
}

#[test]
fn fn_return_float_valid() {
    assert!(types_ok("fn pi() -> Float { return 3.14; }"));
}

#[test]
fn fn_return_array() {
    assert!(types_ok("fn nums() -> [Int] { return [1, 2, 3]; }"));
}

#[test]
fn fn_return_map() {
    assert!(types_ok("fn cfg() -> {Str: Int} { return {\"a\": 1}; }"));
}

#[test]
fn fn_return_optional() {
    assert!(types_ok("fn find() -> Int? { return nil; }"));
}

#[test]
fn fn_return_optional_value() {
    assert!(types_ok("fn find() -> Int? { return 42; }"));
}

#[test]
fn fn_expression_body() {
    assert!(types_ok("fn double(x: Int) -> Int => x * 2;"));
}

#[test]
fn fn_expr_body_wrong_type() {
    assert!(types_fail("fn double(x: Int) -> Int => \"hello\";"));
}

#[test]
fn fn_error_return_type() {
    assert!(types_ok(
        "fn divide(a: Int, b: Int) -> Int ! Str { if b == 0 { Err \"zero\"; } Ok a / b; }"
    ));
}

#[test]
fn fn_multiple_params() {
    assert!(types_ok("fn calc(a: Int, b: Float, c: Str) { print(a); }"));
}

#[test]
fn fn_call_too_few_args() {
    assert!(types_fail(
        "fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { add(1); }"
    ));
}

#[test]
fn fn_call_too_many_args() {
    assert!(types_fail(
        "fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { add(1, 2, 3); }"
    ));
}

#[test]
fn fn_recursive_valid() {
    assert!(types_ok(
        "fn fact(n: Int) -> Int { if n <= 1 { return 1; } return n * fact(n - 1); }"
    ));
}

#[test]
fn fn_mutual_recursion() {
    assert!(types_ok("fn isEven(n: Int) -> Bool { if n == 0 { return true; } return isOdd(n - 1); } fn isOdd(n: Int) -> Bool { if n == 0 { return false; } return isEven(n - 1); }"));
}

#[test]
fn fn_nested_calls() {
    assert!(types_ok("fn inner() -> Int { return 5; } fn outer(x: Int) -> Int { return x; } fn main() { outer(inner()); }"));
}

#[test]
fn fn_void_no_return() {
    assert!(types_ok("fn greet(name: Str) { print(name); }"));
}

#[test]
fn fn_conditional_return() {
    assert!(types_ok(
        "fn abs(x: Int) -> Int { if x < 0 { return -x; } return x; }"
    ));
}

// =============================================================================
// Additional Array Types (array_*)
// =============================================================================

#[test]
fn array_empty() {
    assert!(types_ok("fn main() { let arr: [Int] = []; }"));
}

#[test]
fn array_str_valid() {
    assert!(types_ok("fn main() { let arr = [\"hello\", \"world\"]; }"));
}

#[test]
fn array_bool_valid() {
    assert!(types_ok("fn main() { let arr = [true, false, true]; }"));
}

#[test]
fn array_float_valid() {
    assert!(types_ok("fn main() { let arr = [1.0, 2.5, 3.7]; }"));
}

#[test]
fn array_nested_valid() {
    assert!(types_ok("fn main() { let arr = [[1, 2], [3, 4]]; }"));
}

#[test]
fn array_nested_mixed_fail() {
    assert!(types_fail(
        "fn main() { let arr = [[1, 2], [\"a\", \"b\"]]; }"
    ));
}

#[test]
fn array_push_valid() {
    assert!(types_ok(
        "fn main() { let mut arr: [Int] = []; arr.push(1); }"
    ));
}

#[test]
fn array_push_wrong_type() {
    assert!(types_fail(
        "fn main() { let mut arr: [Int] = []; arr.push(\"hello\"); }"
    ));
}

#[test]
fn array_length() {
    assert!(types_ok(
        "fn main() { let arr = [1, 2, 3]; let n = arr.length(); }"
    ));
}

#[test]
fn array_map_closure() {
    assert!(types_ok(
        "fn main() { let arr = [1, 2, 3]; let doubled = arr.map((x) => x * 2); }"
    ));
}

#[test]
fn array_filter_closure() {
    assert!(types_ok(
        "fn main() { let arr = [1, 2, 3, 4]; let evens = arr.filter((x) => x % 2 == 0); }"
    ));
}

#[test]
fn array_reduce_closure() {
    assert!(types_ok(
        "fn main() { let arr = [1, 2, 3]; let sum = arr.reduce(0, (a, b) => a + b); }"
    ));
}

#[test]
fn array_spread() {
    assert!(types_ok(
        "fn main() { let a = [1, 2]; let b = [...a, 3, 4]; }"
    ));
}

#[test]
fn array_in_function_param() {
    assert!(types_ok(
        "fn sum(nums: [Int]) -> Int { return 0; } fn main() { sum([1, 2, 3]); }"
    ));
}

#[test]
fn array_in_function_return() {
    assert!(types_ok(
        "fn nums() -> [Int] { return [1, 2, 3]; } fn main() { let a = nums(); }"
    ));
}

#[test]
fn array_iterate() {
    assert!(types_ok(
        "fn main() { let arr = [1, 2, 3]; for item in arr { print(item); } }"
    ));
}

#[test]
fn array_index_with_var() {
    assert!(types_ok(
        "fn main() { let arr = [10, 20]; let i = 0; let v = arr[i]; }"
    ));
}

#[test]
fn array_assign_element() {
    assert!(types_ok(
        "fn main() { let mut arr = [1, 2, 3]; arr[0] = 10; }"
    ));
}

#[test]
fn array_assign_wrong_type() {
    assert!(types_fail(
        "fn main() { let mut arr = [1, 2, 3]; arr[0] = \"hello\"; }"
    ));
}

// =============================================================================
// Additional Map Types (map_*)
// =============================================================================

#[test]
fn map_empty() {
    assert!(types_ok("fn main() { let m: {Str: Int} = {}; }"));
}

#[test]
fn map_access_valid() {
    assert!(types_ok(
        "fn main() { let m = {\"a\": 1}; let v = m[\"a\"]; }"
    ));
}

#[test]
fn map_access_wrong_key_type() {
    assert!(types_fail(
        "fn main() { let m: {Str: Int} = {\"a\": 1}; let v = m[42]; }"
    ));
}

#[test]
fn map_insert_valid() {
    assert!(types_ok(
        "fn main() { let mut m: {Str: Int} = {}; m[\"key\"] = 42; }"
    ));
}

#[test]
fn map_insert_wrong_type() {
    assert!(types_fail(
        "fn main() { let mut m: {Str: Int} = {}; m[\"key\"] = \"hello\"; }"
    ));
}

#[test]
fn map_nested() {
    assert!(types_ok(
        "fn main() { let m: {Str: {Str: Int}} = {\"a\": {\"b\": 1}}; }"
    ));
}

#[test]
fn map_nested_access() {
    assert!(types_ok(
        "fn main() { let m = {\"a\": {\"b\": 1}}; let v = m[\"a\"][\"b\"]; }"
    ));
}

#[test]
fn map_with_array_values() {
    assert!(types_ok(
        "fn main() { let m: {Str: [Int]} = {\"nums\": [1, 2, 3]}; }"
    ));
}

#[test]
fn map_keys_method() {
    assert!(types_ok(
        "fn main() { let m = {\"a\": 1, \"b\": 2}; let keys = m.keys(); }"
    ));
}

#[test]
fn map_in_function_param() {
    assert!(types_ok(
        "fn process(m: {Str: Int}) { print(m); } fn main() { process({\"a\": 1}); }"
    ));
}

#[test]
fn map_in_function_return() {
    assert!(types_ok(
        "fn config() -> {Str: Int} { return {\"timeout\": 30}; }"
    ));
}

#[test]
fn map_dynamic_key() {
    assert!(types_ok(
        "fn main() { let key = \"a\"; let m = {\"a\": 1}; let v = m[key]; }"
    ));
}

#[test]
fn map_contains_check() {
    assert!(types_ok(
        "fn main() { let m = {\"a\": 1}; if \"a\" in m { print(\"found\"); } }"
    ));
}

// =============================================================================
// Additional Optional Types (opt_*)
// =============================================================================

#[test]
fn opt_nil_coalesce() {
    assert!(types_ok(
        "fn main() { let x: Int? = nil; let y = x ?? panic(\"nil\"); }"
    ));
}

#[test]
fn opt_nil_coalesce_mismatch() {
    assert!(types_fail(
        "fn main() { let x: Int? = nil; let y = x ?? \"hello\"; }"
    ));
}

#[test]
fn opt_chaining() {
    assert!(types_ok("fn main() { let x: Int? = 42; let y: Int? = x; }"));
}

#[test]
fn opt_in_struct() {
    assert!(types_ok(
        "struct User { email: Str? } fn main() { let u = User { email: nil }; }"
    ));
}

#[test]
fn opt_in_struct_with_value() {
    assert!(types_ok(
        "struct User { email: Str? } fn main() { let u = User { email: \"a@b.com\" }; }"
    ));
}

#[test]
fn opt_in_function_return() {
    assert!(types_ok(
        "fn find(id: Int) -> Str? { if id > 0 { return \"found\"; } return nil; }"
    ));
}

#[test]
fn opt_in_function_param() {
    assert!(types_ok(
        "fn greet(name: Str?) { print(name ?? panic(\"nil\")); }"
    ));
}

#[test]
fn opt_non_optional_assign_nil_fail() {
    assert!(types_fail("fn main() { let x: Int = nil; }"));
}

#[test]
fn opt_ternary() {
    assert!(types_ok(
        "fn main() { let x: Int? = 5; let v = x ?? panic(\"nil\"); }"
    ));
}

#[test]
fn opt_array() {
    assert!(types_ok("fn main() { let x: [Int]? = nil; }"));
}

#[test]
fn opt_map() {
    assert!(types_ok("fn main() { let x: {Str: Int}? = nil; }"));
}

// =============================================================================
// Additional Struct Types (struct_*)
// =============================================================================

#[test]
fn struct_optional_field() {
    assert!(types_ok("struct User { name: Str, email: Str? } fn main() { let u = User { name: \"A\", email: nil }; }"));
}

#[test]
fn struct_multiple_fields() {
    assert!(types_ok("struct Point { x: Float, y: Float, z: Float } fn main() { let p = Point { x: 1.0, y: 2.0, z: 3.0 }; }"));
}

#[test]
fn struct_field_in_expression() {
    assert!(types_ok("struct Point { x: Int, y: Int } fn main() { let p = Point { x: 10, y: 20 }; let sum = p.x + p.y; }"));
}

#[test]
fn struct_method_self() {
    assert!(types_ok("struct Counter { count: Int } fn Counter.increment(self) -> Int { return self.count + 1; }"));
}

#[test]
fn struct_method_params() {
    assert!(types_ok("struct User { name: Str } fn User.greetWith(self, prefix: Str) -> Str { return prefix + self.name; }"));
}

#[test]
fn struct_method_wrong_return() {
    assert!(types_fail(
        "struct User { name: Str } fn User.getName(self) -> Int { return self.name; }"
    ));
}

#[test]
fn struct_nested_field_access() {
    assert!(types_ok("struct Address { city: Str } struct User { address: Address } fn main() { let u = User { address: Address { city: \"NYC\" } }; print(u.address.city); }"));
}

#[test]
fn struct_array_field() {
    assert!(types_ok(
        "struct Team { members: [Str] } fn main() { let t = Team { members: [\"A\", \"B\"] }; }"
    ));
}

#[test]
fn struct_map_field() {
    assert!(types_ok("struct Config { settings: {Str: Int} } fn main() { let c = Config { settings: {\"timeout\": 30} }; }"));
}

#[test]
fn struct_used_in_array() {
    assert!(types_ok("struct User { name: Str } fn main() { let users = [User { name: \"A\" }, User { name: \"B\" }]; }"));
}

#[test]
fn struct_used_in_map() {
    assert!(types_ok("struct User { name: Str } fn main() { let m: {Str: User} = {\"admin\": User { name: \"A\" }}; }"));
}

#[test]
fn struct_extra_field_fail() {
    assert!(types_fail(
        "struct User { name: Str } fn main() { let u = User { name: \"A\", age: 30 } }"
    ));
}

#[test]
fn struct_mut_field_update() {
    assert!(types_ok(
        "struct User { name: Str } fn main() { let mut u = User { name: \"A\" }; u.name = \"B\"; }"
    ));
}

#[test]
fn struct_mut_field_wrong_type() {
    assert!(types_fail(
        "struct User { name: Str } fn main() { let mut u = User { name: \"A\" }; u.name = 42; }"
    ));
}

#[test]
fn struct_with_enum_field() {
    assert!(types_ok("enum Status { Active, Inactive } struct User { name: Str, status: Status } fn main() { let u = User { name: \"A\", status: Status::Active }; }"));
}

#[test]
fn struct_as_function_param() {
    assert!(types_ok("struct User { name: Str } fn greet(u: User) { print(u.name); } fn main() { greet(User { name: \"A\" }); }"));
}

#[test]
fn struct_as_function_return() {
    assert!(types_ok(
        "struct User { name: Str } fn createUser() -> User { return User { name: \"A\" }; }"
    ));
}

#[test]
fn struct_default_value() {
    assert!(types_ok(
        "struct Config { timeout: Int = 30 } fn main() { let c = Config {}; }"
    ));
}

#[test]
fn struct_immutable_update_fail() {
    assert!(types_fail(
        "struct User { name: Str } fn main() { let u = User { name: \"A\" }; u.name = \"B\"; }"
    ));
}

// =============================================================================
// Additional Enum Types (enum_*)
// =============================================================================

#[test]
fn enum_three_variants() {
    assert!(types_ok(
        "enum Light { Red, Yellow, Green } fn main() { let l = Light.Red; }"
    ));
}

#[test]
fn enum_with_payload() {
    assert!(types_ok("enum Shape { Circle(Float), Rectangle(Float, Float) } fn main() { let s = Shape::Circle(5.0); }"));
}

#[test]
fn enum_match_exhaustive() {
    assert!(types_ok("enum Dir { Up, Down, Left, Right } fn dir(d: Dir) -> Str { match d { Dir.Up => \"up\", Dir.Down => \"down\", Dir.Left => \"left\", Dir.Right => \"right\" }; }"));
}

#[test]
fn enum_match_wildcard() {
    assert!(types_ok("enum Color { Red, Green, Blue } fn main() { let c = Color::Red; match c { Color::Red => print(\"red\"), _ => print(\"other\") } }"));
}

#[test]
fn enum_in_array() {
    assert!(types_ok(
        "enum Color { Red, Blue } fn main() { let colors = [Color::Red, Color::Blue]; }"
    ));
}

#[test]
fn enum_unknown_variant_fail() {
    assert!(types_fail(
        "enum Color { Red, Blue } fn main() { let c = Color::Green; }"
    ));
}

#[test]
fn enum_method() {
    assert!(types_ok("enum Priority { Low, High } fn Priority.isHigh(self) -> Bool { match self { Priority::High => true, _ => false } }"));
}

#[test]
fn enum_in_function_param() {
    assert!(types_ok(
        "enum Color { Red, Blue } fn paint(c: Color) { print(c); }"
    ));
}

#[test]
fn enum_returned_from_function() {
    assert!(types_ok(
        "enum Status { Success, Failure } fn check() -> Status { return Status::Success; }"
    ));
}

// =============================================================================
// Additional Binary Operations (binop_*)
// =============================================================================

#[test]
fn binop_int_sub() {
    assert!(types_ok("fn main() { let x = 10 - 3; }"));
}

#[test]
fn binop_int_mul() {
    assert!(types_ok("fn main() { let x = 4 * 5; }"));
}

#[test]
fn binop_int_div() {
    assert!(types_ok("fn main() { let x = 10 / 2; }"));
}

#[test]
fn binop_int_mod() {
    assert!(types_ok("fn main() { let x = 10 % 3; }"));
}

#[test]
fn binop_float_add() {
    assert!(types_ok("fn main() { let x = 1.5 + 2.5; }"));
}

#[test]
fn binop_float_sub() {
    assert!(types_ok("fn main() { let x = 5.0 - 2.5; }"));
}

#[test]
fn binop_float_mul() {
    assert!(types_ok("fn main() { let x = 2.0 * 3.0; }"));
}

#[test]
fn binop_float_div() {
    assert!(types_ok("fn main() { let x = 10.0 / 3.0; }"));
}

#[test]
fn binop_bool_and() {
    assert!(types_ok("fn main() { let x = true && false; }"));
}

#[test]
fn binop_bool_or() {
    assert!(types_ok("fn main() { let x = true || false; }"));
}

#[test]
fn binop_bool_not() {
    assert!(types_ok("fn main() { let x = !true; }"));
}

#[test]
fn binop_eq() {
    assert!(types_ok("fn main() { let x = 1 == 1; }"));
}

#[test]
fn binop_neq() {
    assert!(types_ok("fn main() { let x = 1 != 2; }"));
}

#[test]
fn binop_gt() {
    assert!(types_ok("fn main() { let x = 5 > 3; }"));
}

#[test]
fn binop_gte() {
    assert!(types_ok("fn main() { let x = 5 >= 5; }"));
}

#[test]
fn binop_lt() {
    assert!(types_ok("fn main() { let x = 3 < 5; }"));
}

#[test]
fn binop_lte() {
    assert!(types_ok("fn main() { let x = 3 <= 3; }"));
}

#[test]
fn binop_str_eq() {
    assert!(types_ok("fn main() { let x = \"a\" == \"b\"; }"));
}

#[test]
fn binop_chained() {
    assert!(types_ok("fn main() { let x = 1 + 2 + 3 + 4; }"));
}

#[test]
fn binop_mixed_precedence() {
    assert!(types_ok("fn main() { let x = 1 + 2 * 3; }"));
}

#[test]
fn binop_parens() {
    assert!(types_ok("fn main() { let x = (1 + 2) * 3; }"));
}

#[test]
fn binop_logical_chain() {
    assert!(types_ok("fn main() { let x = true && false || true; }"));
}

#[test]
fn binop_comparison_chain() {
    assert!(types_ok("fn main() { let x = 1 < 2 && 3 > 1; }"));
}

#[test]
fn binop_int_float_fail() {
    assert!(types_fail("fn main() { let x = 1 + 2.5; }"));
}

#[test]
fn binop_str_int_fail() {
    assert!(types_fail("fn main() { let x = \"hello\" - 1; }"));
}

#[test]
fn binop_bool_add_fail() {
    assert!(types_fail("fn main() { let x = true + false; }"));
}

#[test]
fn binop_assignment_ok() {
    assert!(types_ok("fn main() { let mut x = 0; x += 5; }"));
}

#[test]
fn binop_assignment_sub() {
    assert!(types_ok("fn main() { let mut x = 10; x -= 3; }"));
}

#[test]
fn binop_assignment_mul() {
    assert!(types_ok("fn main() { let mut x = 3; x *= 2; }"));
}

#[test]
fn binop_assignment_div() {
    assert!(types_ok("fn main() { let mut x = 10; x /= 2; }"));
}

#[test]
fn binop_assignment_mod() {
    assert!(types_ok("fn main() { let mut x = 10; x %= 3; }"));
}

#[test]
fn binop_increment() {
    assert!(types_ok("fn main() { let mut x = 0; x++; }"));
}

#[test]
fn binop_decrement() {
    assert!(types_ok("fn main() { let mut x = 10; x--; }"));
}

// =============================================================================
// Additional Inference (infer_*)
// =============================================================================

#[test]
fn infer_float() {
    assert!(types_ok("fn main() { let x = 3.14; }"));
}

#[test]
fn infer_string_interpolation() {
    assert!(types_ok(
        "fn main() { let name = \"world\"; let msg = \"Hello ${name; }\"; }"
    ));
}

#[test]
fn infer_array_of_ints() {
    assert!(types_ok("fn main() { let nums = [1, 2, 3]; }"));
}

#[test]
fn infer_array_of_strs() {
    assert!(types_ok("fn main() { let strs = [\"a\", \"b\", \"c\"]; }"));
}

#[test]
fn infer_map() {
    assert!(types_ok("fn main() { let m = {\"key\": 42}; }"));
}

#[test]
fn infer_binary_expr() {
    assert!(types_ok("fn main() { let x = 1 + 2 * 3; }"));
}

#[test]
fn infer_comparison_result() {
    assert!(types_ok("fn main() { let x = 5 > 3; }"));
}

#[test]
fn infer_ternary() {
    assert!(types_ok(
        "fn main() { let x = if true { 1; } else { 0; }; }"
    ));
}

#[test]
fn infer_nil_coalesce() {
    assert!(types_ok(
        "fn main() { let x: Int? = nil; let y = x ?? panic(\"nil\"); }"
    ));
}

#[test]
fn infer_closure() {
    assert!(types_ok("fn main() { let f = (x) => x + 1; }"));
}

#[test]
fn infer_closure_multi_param() {
    assert!(types_ok("fn main() { let f = (a, b) => a + b; }"));
}

#[test]
fn infer_chained_methods() {
    assert!(types_ok(
        "fn main() { let result = [1, 2, 3].map((x) => x * 2); }"
    ));
}

#[test]
fn infer_struct_construction() {
    assert!(types_ok(
        "struct Point { x: Int, y: Int } fn main() { let p = Point { x: 10, y: 20 }; }"
    ));
}

#[test]
fn infer_field_access() {
    assert!(types_ok(
        "struct User { name: Str } fn main() { let u = User { name: \"A\" }; let n = u.name; }"
    ));
}

#[test]
fn infer_method_call() {
    assert!(types_ok(
        "fn main() { let arr = [1, 2, 3]; let len = arr.length(); }"
    ));
}

#[test]
fn infer_cast() {
    assert!(types_ok("fn main() { let x = 42; let f = x as Float; }"));
}

#[test]
fn infer_conditional() {
    assert!(types_ok(
        "fn main() { let x = 5; let y = if x > 0 { x; } else { 0; }; }"
    ));
}

// =============================================================================
// Additional Complex Programs (complex_*)
// =============================================================================

#[test]
fn complex_for_loop_accumulate() {
    assert!(types_ok(
        "fn main() { let mut sum = 0; for i in 0..10 { sum += i; } }"
    ));
}

#[test]
fn complex_nested_if() {
    assert!(types_ok("fn classify(x: Int) -> Str { if x > 100 { return \"big\"; } else if x > 10 { return \"medium\"; } else { return \"small\"; } }"));
}

#[test]
fn complex_struct_with_methods() {
    assert!(types_ok(
        r#"
struct Point { x: Int, y: Int }
fn Point.distanceTo(self, other: Point) -> Int { return (self.x - other.x) + (self.y - other.y); }
fn main() { let a = Point { x: 0, y: 0 }; let b = Point { x: 3, y: 4 }; a.distanceTo(b); }
"#
    ));
}

#[test]
fn complex_enum_with_match() {
    assert!(types_ok(
        r#"
enum Shape { Circle(Float), Rect(Float, Float) }
fn area(s: Shape) -> Float {
    match s {
        Shape::Circle(r) => 3.14 * r * r,
        Shape::Rect(w, h) => w * h
    }
}
"#
    ));
}

#[test]
fn complex_array_pipeline() {
    assert!(types_ok(
        "fn main() { let result = [1, 2, 3, 4, 5].filter((x) => x > 2).map((x) => x * 10); }"
    ));
}

#[test]
fn complex_error_handling() {
    assert!(types_ok(
        r#"
enum Outcome { Success(Int), Failure(Str) }
fn divide(a: Int, b: Int) -> Outcome {
    if b == 0 { return Outcome::Failure("division by zero"); }
    return Outcome::Success(a / b);
}
fn main() {
    let result = divide(10, 2);
    match result { Outcome::Success(v) => print(v), Outcome::Failure(e) => print(e) };
}
"#
    ));
}

#[test]
fn complex_closure_capture() {
    assert!(types_ok(
        "fn main() { let base = 100; let add = (x) => x + base; let result = add(42); }"
    ));
}

#[test]
fn complex_multi_struct() {
    assert!(types_ok(
        r#"
struct Address { city: Str, zip: Str }
struct User { name: Str, age: Int, address: Address }
fn main() {
    let u = User { name: "Alice", age: 30, address: Address { city: "NYC", zip: "10001" } };
    print(u.address.city);
}
"#
    ));
}

#[test]
fn complex_loop_with_break() {
    assert!(types_ok("fn main() { let mut found = false; for i in 0..100 { if i == 42 { found = true; break; } }; }"));
}

#[test]
fn complex_nested_collections() {
    assert!(types_ok(
        "fn main() { let matrix: [[Int]] = [[1, 2], [3, 4]]; let v = matrix[0][1]; }"
    ));
}

#[test]
fn complex_string_operations() {
    assert!(types_ok(
        "fn main() { let s = \"hello\"; let upper = s.toUpperCase(); let len = s.length(); }"
    ));
}

#[test]
fn complex_map_iteration() {
    assert!(types_ok("fn main() { let m = {\"a\": 1, \"b\": 2}; let keys = m.keys(); for k in keys { print(k); } }"));
}

#[test]
fn complex_optional_chain() {
    assert!(types_ok("fn find(id: Int) -> Str? { if id > 0 { return \"found\"; } return nil; } fn main() { let result = find(1) ?? panic(\"not found\"); print(result); }"));
}

#[test]
fn complex_user_pipeline() {
    assert!(types_ok(
        r#"
struct User { name: Str, age: Int }
fn User.isAdult(self) -> Bool => self.age >= 18;
fn User.greeting(self) -> Str => "Hello, ${self.name}";
fn main() {
    let users = [User { name: "Alice", age: 25 }, User { name: "Bob", age: 15 }];
    for u in users {
        if u.isAdult() { print(u.greeting()); }
    }
}
"#
    ));
}

#[test]
fn complex_recursive_tree() {
    assert!(types_ok(
        r#"
fn fib(n: Int) -> Int {
    if n <= 0 { return 0; }
    if n == 1 { return 1; }
    return fib(n - 1) + fib(n - 2);
}
fn main() { print(fib(10)); }
"#
    ));
}

#[test]
fn complex_task_manager() {
    assert!(types_ok(
        r#"
enum Priority { Low, Medium, High }
enum Status { Todo, Done }
struct Task { title: Str, priority: Priority, status: Status }
fn Task.isDone(self) -> Bool { match self.status { Status::Done => true, _ => false } }
fn Task.isUrgent(self) -> Bool { match self.priority { Priority::High => true, _ => false } }
fn main() {
    let tasks = [
        Task { title: "Fix bug", priority: Priority::High, status: Status::Todo },
        Task { title: "Write docs", priority: Priority::Low, status: Status::Done }
    ];
    for t in tasks {
        if t.isUrgent() && !t.isDone() { print(t.title); }
    }
}
"#
    ));
}

#[test]
fn complex_error_chain() {
    assert!(types_ok(
        r#"
fn parseInt(s: Str) -> Int ! Str {
    Ok 42;
}
fn parseAndDouble(s: Str) -> Int ! Str {
    let v = parseInt(s)?;
    Ok v * 2;
}
"#
    ));
}

#[test]
fn complex_variable_shadowing() {
    assert!(types_ok(
        "fn main() { let x = 1; let x = \"hello\"; let x = [1, 2, 3]; print(x); }"
    ));
}

#[test]
fn complex_scope_nesting() {
    assert!(types_ok("fn main() { let x = 5; if x > 0 { let y = x + 1; if y > 3 { let z = y * 2; print(z); } } }"));
}

#[test]
fn complex_closure_in_loop() {
    assert!(types_ok(
        "fn main() { let fns = []; for i in 0..5 { let f = () => i; print(f()); } }"
    ));
}

#[test]
fn complex_multi_return_match() {
    assert!(types_ok(
        r#"
fn classify(x: Int) -> Str {
    if x > 100 { return "huge"; }
    if x > 50 { return "big"; }
    if x > 10 { return "medium"; }
    if x > 0 { return "small"; }
    return "zero or negative";
}
"#
    ));
}

#[test]
fn complex_while_pattern() {
    assert!(types_ok(
        "fn main() { let mut count = 0; for { count++; if count >= 10 { break; } } }"
    ));
}

#[test]
fn complex_decorated_struct() {
    assert!(types_ok("@table struct User { name: Str, age: Int } fn main() { let u = User { name: \"A\", age: 30 }; }"));
}

#[test]
fn complex_cast_operations() {
    assert!(types_ok(
        "fn main() { let x = 42; let f = x as Float; let s = x as Str; }"
    ));
}

#[test]
fn complex_range_inclusive() {
    assert!(types_ok("fn main() { for i in 1..=10 { print(i); }; }"));
}

#[test]
fn complex_range_exclusive() {
    assert!(types_ok("fn main() { for i in 0..10 { print(i); }; }"));
}

#[test]
fn complex_range_with_vars() {
    assert!(types_ok(
        "fn main() { let start = 0; let end = 10; for i in start..end { print(i); } }"
    ));
}

#[test]
fn complex_indexed_loop() {
    assert!(types_ok(
        "fn main() { let arr = [10, 20, 30]; for i, val in arr { print(i, val); } }"
    ));
}

#[test]
fn complex_multiline_program() {
    assert!(types_ok(
        r#"
struct Config { timeout: Int, retries: Int }
fn Config.isValid(self) -> Bool => self.timeout > 0 && self.retries > 0;
fn loadConfig() -> Config {
    return Config { timeout: 30, retries: 3 };
}
fn main() {
    let mut cfg = loadConfig();
    if !cfg.isValid() {
        cfg.timeout = 60;
        cfg.retries = 5;
    }
    print(cfg.timeout);
}
"#
    ));
}
