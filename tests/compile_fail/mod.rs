//! Compile-fail tests — programs that MUST fail to compile
//! Uses the real compiler pipeline via common::assert_fails / assert_fails_with
//! Only tests REAL user-facing errors (type safety, scope, immutability)
//! NOT borrow/ownership — those are auto-managed in Doo

use crate::common::{assert_compiles, assert_fails, assert_fails_with, assert_doo_file_suite, DooTestMode};

// ===========================================================================
// Type Safety — Annotations
// ===========================================================================

#[test]
fn type_mismatch_int_str() {
    assert_fails_with(r#"fn main() { let x: Int = "hello"; }"#, "expected");
}

#[test]
fn type_mismatch_bool_int() {
    assert_fails_with(r#"fn main() { let x: Bool = 42; }"#, "expected");
}

#[test]
fn type_mismatch_str_int() {
    assert_fails_with(r#"fn main() { let x: Str = 42; }"#, "expected");
}

#[test]
fn type_mismatch_float_str() {
    assert_fails_with(r#"fn main() { let x: Float = "nope"; }"#, "expected");
}

#[test]
fn type_mismatch_int_bool() {
    assert_fails_with(r#"fn main() { let x: Int = true; }"#, "expected");
}

#[test]
fn type_mismatch_str_bool() {
    assert_fails_with(r#"fn main() { let x: Str = false; }"#, "expected");
}

#[test]
fn unknown_type_annotation() {
    assert_fails(r#"fn main() { let x: Foo = 42; }"#);
}

// ===========================================================================
// Type Safety — Return Types
// ===========================================================================

#[test]
fn return_type_mismatch_str_for_int() {
    assert_fails_with(
        r#"fn get() -> Int { return "oops"; } fn main() { }"#,
        "expected",
    );
}

#[test]
fn return_type_mismatch_bool_for_str() {
    assert_fails_with(
        r#"fn get() -> Str { return true; } fn main() { }"#,
        "expected",
    );
}

#[test]
fn return_type_mismatch_int_for_bool() {
    assert_fails_with(
        r#"fn get() -> Bool { return 42; } fn main() { }"#,
        "expected",
    );
}

#[test]
fn missing_return_value() {
    assert_fails(r#"fn get() -> Int { print("no return"); } fn main() { }"#);
}

// ===========================================================================
// Type Safety — Function Arguments
// ===========================================================================

#[test]
fn wrong_argument_count_fewer() {
    assert_fails_with(
        r#"fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { add(5); }"#,
        "ArgMismatch",
    );
}

#[test]
fn wrong_argument_count_more() {
    assert_fails_with(
        r#"fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { add(1, 2, 3); }"#,
        "ArgMismatch",
    );
}

#[test]
fn wrong_argument_count_zero() {
    assert_fails_with(
        r#"fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { add(); }"#,
        "ArgMismatch",
    );
}

#[test]
fn wrong_argument_type() {
    assert_fails(
        r#"fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { add(5, "hello"); }"#,
    );
}

#[test]
fn wrong_argument_type_bool_for_int() {
    assert_fails(r#"fn take(x: Int) { print(x); } fn main() { take(true); }"#);
}

// ===========================================================================
// Type Safety — Binary Operators
// ===========================================================================

#[test]
fn binary_op_int_plus_str() {
    assert_fails(r#"fn main() { let x = 42 + "hello"; }"#);
}

#[test]
fn binary_op_bool_plus_int() {
    assert_fails(r#"fn main() { let x = true + 1; }"#);
}

#[test]
fn binary_op_str_minus_str() {
    assert_fails(r#"fn main() { let x = "a" - "b"; }"#);
}

#[test]
fn binary_op_str_multiply() {
    assert_fails(r#"fn main() { let x = "a" * "b"; }"#);
}

#[test]
fn comparison_int_str() {
    assert_fails(r#"fn main() { let x = 42 > "hello"; }"#);
}

// ===========================================================================
// Type Safety — Collections
// ===========================================================================

#[test]
fn array_mixed_types() {
    assert_fails(r#"fn main() { let arr = [1, "two", 3]; }"#);
}

#[test]
fn array_mixed_int_bool() {
    assert_fails(r#"fn main() { let arr = [1, true, 3]; }"#);
}

#[test]
fn array_annotated_wrong_element() {
    assert_fails(r#"fn main() { let arr: [Int] = ["hello"]; }"#);
}

#[test]
fn map_mixed_value_types() {
    assert_fails(r#"fn main() { let m = {"a": 1, "b": "two"}; }"#);
}

#[test]
fn map_mixed_key_types() {
    assert_fails(r#"fn main() { let m = {"a": 1, 2: 3}; }"#);
}

// ===========================================================================
// Type Safety — Structs
// ===========================================================================

#[test]
fn struct_wrong_field_type() {
    assert_fails(
        r#"
struct Point { x: Int, y: Int }
fn main() { let p = Point { x: "hello", y: 10 }; }
"#,
    );
}

#[test]
fn struct_missing_field() {
    assert_fails(
        r#"
struct Point { x: Int, y: Int }
fn main() { let p = Point { x: 10 }; }
"#,
    );
}

#[test]
fn struct_extra_field() {
    assert_fails(
        r#"
struct Point { x: Int, y: Int }
fn main() { let p = Point { x: 1, y: 2, z: 3 }; }
"#,
    );
}

#[test]
fn struct_nonexistent_field_access() {
    assert_fails(
        r#"
struct Point { x: Int, y: Int }
fn main() { let p = Point { x: 1, y: 2 }; print(p.z); }
"#,
    );
}

#[test]
fn struct_method_on_wrong_type() {
    assert_fails(
        r#"
struct Cat { name: Str }
struct Dog { name: Str }
fn Cat.meow(self) -> Str { return "meow"; }
fn main() { let d = Dog { name: "Rex" }; d.meow(); }
"#,
    );
}

// ===========================================================================
// Type Safety — Enums
// ===========================================================================

#[test]
fn enum_nonexistent_variant() {
    assert_fails(r#"enum Color { Red, Blue } fn main() { let c = Color::Green; }"#);
}

#[test]
fn enum_wrong_payload_type() {
    assert_fails(
        r#"
enum Wrapper { Val(Int) }
fn main() { let w = Wrapper::Val("hello"); }
"#,
    );
}

#[test]
fn enum_missing_payload() {
    assert_fails(
        r#"
enum Wrapper { Val(Int) }
fn main() { let w = Wrapper::Val; }
"#,
    );
}

// ===========================================================================
// Type Safety — Match
// ===========================================================================

#[test]
fn match_arm_type_mismatch() {
    assert_fails(
        r#"
fn main() {
    let x = 5;
    let result: Str = match {
        x > 0 => "positive",
        _ => 42,
    };
}
"#,
    );
}

// ===========================================================================
// Type Safety — Error Handling
// ===========================================================================

#[test]
fn error_propagation_outside_error_fn() {
    // The compiler allows ? in non-error functions (panics at runtime on Err)
    assert_compiles(
        r#"
fn risky() -> Int ! Str { Ok 42; }
fn main() { let v = risky()?; }
"#,
    );
}

#[test]
fn ok_in_non_error_function() {
    assert_fails(
        r#"
fn get() -> Int { Ok 42; }
fn main() { }
"#,
    );
}

#[test]
fn err_in_non_error_function() {
    assert_fails(
        r#"
fn get() -> Int { Err "bad"; }
fn main() { }
"#,
    );
}

// ===========================================================================
// Scope Errors
// ===========================================================================

#[test]
fn undefined_variable() {
    assert_fails_with(r#"fn main() { print(undefined_var); }"#, "undefined");
}

#[test]
fn undefined_function() {
    assert_fails(r#"fn main() { doesNotExist(); }"#);
}

#[test]
fn variable_out_of_scope_if() {
    assert_fails(
        r#"
fn main() {
    if true { let inner = 42; }
    print(inner);
}
"#,
    );
}

#[test]
fn variable_out_of_scope_for() {
    assert_fails(
        r#"
fn main() {
    for i in 0..5 { let x = i; }
    print(x);
}
"#,
    );
}

#[test]
fn variable_out_of_scope_block() {
    assert_fails(
        r#"
fn main() {
    { let secret = 42; }
    print(secret);
}
"#,
    );
}

#[test]
fn undeclared_struct() {
    assert_fails(r#"fn main() { let p = UnknownStruct { x: 1 }; }"#);
}

#[test]
fn undeclared_enum() {
    assert_fails(r#"fn main() { let c = UnknownEnum::Variant; }"#);
}

#[test]
fn use_before_declaration() {
    assert_fails(r#"fn main() { print(x); let x = 42; }"#);
}

// ===========================================================================
// Immutability Errors
// ===========================================================================

#[test]
fn immutable_reassignment() {
    assert_fails_with(r#"fn main() { let x = 42; x = 10; }"#, "immutable");
}

#[test]
fn immutable_compound_assign() {
    assert_fails(r#"fn main() { let x = 42; x += 10; }"#);
}

#[test]
fn immutable_increment() {
    assert_fails(r#"fn main() { let x = 42; x++; }"#);
}

#[test]
fn immutable_decrement() {
    assert_fails(r#"fn main() { let x = 42; x--; }"#);
}

#[test]
fn immutable_array_push() {
    assert_fails(r#"fn main() { let arr = [1, 2, 3]; arr.push(4); }"#);
}

#[test]
fn immutable_array_index_set() {
    assert_fails(r#"fn main() { let arr = [1, 2, 3]; arr[0] = 99; }"#);
}

#[test]
fn immutable_map_insert() {
    assert_fails(r#"fn main() { let m = {"a": 1}; m["b"] = 2; }"#);
}

#[test]
fn immutable_struct_field_set() {
    assert_fails(
        r#"
struct Point { x: Int, y: Int }
fn main() {
    let p = Point { x: 1, y: 2 };
    p.x = 10;
}
"#,
    );
}

// ===========================================================================
// Duplicate Declarations
// ===========================================================================

#[test]
fn duplicate_variable() {
    assert_fails(r#"fn main() { let x = 1; let x = 2; }"#);
}

#[test]
fn duplicate_function() {
    assert_fails(
        r#"
fn foo() { } fn foo() { } fn main() { }
"#,
    );
}

#[test]
fn duplicate_struct() {
    assert_fails(
        r#"
struct Foo { x: Int }
struct Foo { y: Str }
fn main() { }
"#,
    );
}

#[test]
fn duplicate_enum() {
    assert_fails(
        r#"
enum Color { Red }
enum Color { Blue }
fn main() { }
"#,
    );
}

// ===========================================================================
// Type Safety — Closures
// ===========================================================================

#[test]
fn closure_wrong_return_in_map() {
    assert_fails(
        r#"
fn main() {
    let nums = [1, 2, 3];
    let result: [Str] = nums.map((x) => x * 2);
}
"#,
    );
}

// ===========================================================================
// Type Safety — Nil Safety
// ===========================================================================

#[test]
fn assign_nil_to_non_optional() {
    assert_fails(r#"fn main() { let x: Int = nil; }"#);
}

#[test]
fn assign_nil_to_non_optional_str() {
    assert_fails(r#"fn main() { let x: Str = nil; }"#);
}

// ===========================================================================
// Auto-discovered .doo compile-fail files
// ===========================================================================

#[test]
fn compile_fail_doo_files() {
    let dir = std::path::Path::new("tests/compile_fail");
    assert_doo_file_suite(dir, DooTestMode::CompileFail, "compile_fail");
}
