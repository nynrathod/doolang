//! UI tests — verify diagnostic messages and error formatting
//! Modeled after rustc's tests/ui/ (15,000+ tests)
//! Each test verifies the compiler produces the RIGHT error message

use crate::common::{compile_snippet, assert_doo_file_suite, DooTestMode};

fn assert_error_contains(code: &str, expected_error: &str) {
    let result = compile_snippet(code);
    assert!(
        result.is_err(),
        "Expected error but compilation succeeded for code:\n{}",
        code
    );

    let error_msg = result.err().unwrap();
    assert!(
        error_msg
            .to_lowercase()
            .contains(&expected_error.to_lowercase()),
        "Expected error message containing '{}' but got: '{}'\nCode:\n{}",
        expected_error,
        error_msg,
        code
    );
}

fn assert_compiles_fine(code: &str) {
    let result = compile_snippet(code);
    assert!(
        result.is_ok(),
        "Expected compilation to succeed but got error: {:?}\nCode:\n{}",
        result.err(),
        code
    );
}

// ===========================================================================
// Type Mismatch Messages
// ===========================================================================

#[test]
fn ui_type_mismatch_int_str() {
    assert_error_contains(r#"fn main() { let x: Int = "string"; }"#, "type");
}

#[test]
fn ui_type_mismatch_bool_int() {
    assert_error_contains(r#"fn main() { let x: Bool = 42; }"#, "type");
}

#[test]
fn ui_type_mismatch_return() {
    assert_error_contains(
        r#"fn get() -> Int { return "oops"; } fn main() { }"#,
        "type",
    );
}

#[test]
fn ui_type_mismatch_binary_op() {
    assert_error_contains(r#"fn main() { let x = 42 + "hello"; }"#, "type");
}

#[test]
fn ui_type_mismatch_array_element() {
    assert_error_contains(r#"fn main() { let arr = [1, "two", 3]; }"#, "type");
}

// ===========================================================================
// Undefined Symbol Messages
// ===========================================================================

#[test]
fn ui_undefined_variable_message() {
    assert_error_contains(r#"fn main() { print(undefined); }"#, "undefined");
}

#[test]
fn ui_undefined_function_message() {
    assert_error_contains(r#"fn main() { doesNotExist(); }"#, "undefined");
}

#[test]
fn ui_undefined_struct_message() {
    assert_error_contains(r#"fn main() { let p = Unknown { x: 1 }; }"#, "undefined");
}

// ===========================================================================
// Immutability Messages
// ===========================================================================

#[test]
fn ui_immutable_assignment_message() {
    assert_error_contains(r#"fn main() { let x = 42; x = 10; }"#, "immutable");
}

#[test]
fn ui_immutable_array_mutation() {
    assert_error_contains(
        r#"fn main() { let arr = [1, 2]; arr.push(3); }"#,
        "immutable",
    );
}

#[test]
fn ui_immutable_compound_assign() {
    assert_error_contains(r#"fn main() { let x = 5; x += 1; }"#, "immutable");
}

// ===========================================================================
// Argument Messages
// ===========================================================================

#[test]
fn ui_wrong_arg_count_message() {
    assert_error_contains(
        r#"fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { add(1); }"#,
        "ArgMismatch",
    );
}

#[test]
fn ui_wrong_arg_count_too_many() {
    assert_error_contains(
        r#"fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { add(1, 2, 3); }"#,
        "ArgMismatch",
    );
}

// ===========================================================================
// Struct Error Messages
// ===========================================================================

#[test]
fn ui_struct_missing_field() {
    assert_error_contains(
        r#"struct Point { x: Int, y: Int } fn main() { let p = Point { x: 1 }; }"#,
        "field",
    );
}

#[test]
fn ui_struct_wrong_field_type() {
    assert_error_contains(
        r#"struct Point { x: Int, y: Int } fn main() { let p = Point { x: "bad", y: 2 }; }"#,
        "type",
    );
}

// ===========================================================================
// Enum Error Messages
// ===========================================================================

#[test]
fn ui_enum_bad_variant() {
    assert_error_contains(
        r#"enum Color { Red, Blue } fn main() { let c = Color::Green; }"#,
        "variant",
    );
}

// ===========================================================================
// Scope Error Messages
// ===========================================================================

#[test]
fn ui_out_of_scope_variable() {
    assert_error_contains(
        r#"fn main() { if true { let x = 1; } print(x); }"#,
        "undefined",
    );
}

// ===========================================================================
// Error Handling Messages
// ===========================================================================

#[test]
fn ui_propagation_in_non_error_fn() {
    // The compiler allows ? in non-error functions (panics at runtime on Err)
    // Verify it compiles without error
    assert_compiles_fine(
        r#"fn risky() -> Int ! Str { Ok 42; } fn main() { let v = risky()?; }"#,
    );
}

// ===========================================================================
// Auto-Ownership Success Tests (Doo magic — must NOT error)
// ===========================================================================

#[test]
fn ui_auto_clone_no_error() {
    assert_compiles_fine(
        r#"
fn take(s: Str) { print(s); }
fn main() {
    let s = "hello";
    take(s);
    take(s);
    print(s);
}
"#,
    );
}

#[test]
fn ui_auto_clone_array_no_error() {
    assert_compiles_fine(
        r#"
fn process(arr: [Int]) { print(arr.length()); }
fn main() {
    let data = [1, 2, 3];
    process(data);
    process(data);
    print(data.length());
}
"#,
    );
}

#[test]
fn ui_auto_clone_struct_no_error() {
    assert_compiles_fine(
        r#"
struct Point { x: Int, y: Int }
fn show(p: Point) { print(p.x); }
fn main() {
    let p = Point { x: 1, y: 2 };
    show(p);
    print(p.y);
}
"#,
    );
}

#[test]
fn ui_auto_clone_closure_capture() {
    assert_compiles_fine(
        r#"
fn main() {
    let x = 42;
    let f = () => x + 1;
    print(f());
    print(x);
}
"#,
    );
}

#[test]
fn ui_auto_clone_loop_reuse() {
    assert_compiles_fine(
        r#"
fn process(s: Str) { print(s); }
fn main() {
    let msg = "hello";
    for i in 0..5 {
        process(msg);
    }
    print(msg);
}
"#,
    );
}

#[test]
fn ui_auto_clone_multiple_fn_passes() {
    assert_compiles_fine(
        r#"
fn f1(s: Str) { print(s); }
fn f2(s: Str) { print(s); }
fn f3(s: Str) { print(s); }
fn main() {
    let val = "shared";
    f1(val);
    f2(val);
    f3(val);
    print(val);
}
"#,
    );
}

#[test]
fn ui_auto_clone_struct_method_then_use() {
    assert_compiles_fine(
        r#"
struct User { name: Str, age: Int }
fn User.greet(self) -> Str => "Hi ${self.name}";
fn main() {
    let u = User { name: "Alice", age: 30 };
    let g = u.greet();
    print(g);
    print(u.name);
    print(u.age);
}
"#,
    );
}

#[test]
fn ui_auto_clone_nested_struct() {
    assert_compiles_fine(
        r#"
struct Inner { val: Int }
struct Outer { inner: Inner, name: Str }
fn consume(o: Outer) { print(o.name); }
fn main() {
    let o = Outer { inner: Inner { val: 42 }, name: "test" };
    consume(o);
    print(o.inner.val);
}
"#,
    );
}

#[test]
fn ui_auto_clone_map_pass() {
    assert_compiles_fine(
        r#"
fn read(m: {Str: Int}) { print(m["a"]); }
fn main() {
    let m = {"a": 1, "b": 2};
    read(m);
    read(m);
    print(m["b"]);
}
"#,
    );
}

#[test]
fn ui_auto_clone_in_closure_and_after() {
    assert_compiles_fine(
        r#"
fn main() {
    let data = [1, 2, 3];
    let sum = data.reduce(0, (a, b) => a + b);
    print(sum);
    print(data.length());
}
"#,
    );
}

// ===========================================================================
// Mutable Ownership Tests (must compile)
// ===========================================================================

#[test]
fn ui_mut_reassign_after_use() {
    assert_compiles_fine(
        r#"
fn consume(x: Str) { print(x); }
fn main() {
    let mut s = "first";
    consume(s);
    s = "second";
    print(s);
}
"#,
    );
}

#[test]
fn ui_mut_array_operations() {
    assert_compiles_fine(
        r#"
fn main() {
    let mut arr = [1, 2, 3];
    arr.push(4);
    arr[0] = 99;
    print(arr.length());
}
"#,
    );
}

#[test]
fn ui_mut_map_operations() {
    assert_compiles_fine(
        r#"
fn main() {
    let mut m = {"a": 1};
    m["b"] = 2;
    print(m["a"]);
}
"#,
    );
}

// ===========================================================================
// Auto-discovered .doo UI diagnostic files
// ===========================================================================

#[test]
fn ui_doo_files() {
    let dir = std::path::Path::new("tests/ui");
    assert_doo_file_suite(dir, DooTestMode::UiDiagnostic, "ui");
}
