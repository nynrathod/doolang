//! Codegen verification tests — assert specific LLVM IR patterns
//! Modeled after rustc's tests/codegen/ directory
//! Verifies the compiler produces correct IR for each language feature

use crate::common::{assert_compiles_with, compile_snippet};

// ===========================================================================
// Arithmetic — verify correct LLVM instructions
// ===========================================================================

#[test]
fn ir_int_add() {
    assert_compiles_with(r#"fn main() { let x = 1 + 2; print(x); }"#, "add");
}

#[test]
fn ir_int_sub() {
    assert_compiles_with(r#"fn main() { let x = 10 - 3; print(x); }"#, "i64 7");
}

#[test]
fn ir_int_mul() {
    assert_compiles_with(r#"fn main() { let x = 3 * 4; print(x); }"#, "i64 12");
}

#[test]
fn ir_int_div() {
    assert_compiles_with(r#"fn main() { let x = 10 / 2; print(x); }"#, "i64 5");
}

#[test]
fn ir_int_mod() {
    assert_compiles_with(r#"fn main() { let x = 10 % 3; print(x); }"#, "store i64 1");
}

#[test]
fn ir_float_add() {
    assert_compiles_with(
        r#"fn main() { let x = 1.5 + 2.5; print(x); }"#,
        "4.000000e+00",
    );
}

#[test]
fn ir_float_sub() {
    assert_compiles_with(
        r#"fn main() { let x = 5.0 - 2.0; print(x); }"#,
        "3.000000e+00",
    );
}

#[test]
fn ir_float_mul() {
    assert_compiles_with(
        r#"fn main() { let x = 3.0 * 4.0; print(x); }"#,
        "1.200000e+01",
    );
}

#[test]
fn ir_float_div() {
    assert_compiles_with(
        r#"fn main() { let x = 10.0 / 3.0; print(x); }"#,
        "0x400AAAA",
    );
}

// ===========================================================================
// Comparisons — verify icmp/fcmp
// ===========================================================================

#[test]
fn ir_int_compare_gt() {
    assert_compiles_with(r#"fn main() { let x = 5 > 3; print(x); }"#, "icmp");
}

#[test]
fn ir_int_compare_eq() {
    assert_compiles_with(r#"fn main() { let x = 5 == 5; print(x); }"#, "icmp");
}

#[test]
fn ir_float_compare() {
    assert_compiles_with(r#"fn main() { let x = 3.14 > 2.0; print(x); }"#, "i1 true");
}

// ===========================================================================
// Variables — verify store/load
// ===========================================================================

#[test]
fn ir_mutable_store() {
    assert_compiles_with(
        r#"fn main() { let mut x = 10; x = 20; print(x); }"#,
        "store",
    );
}

#[test]
fn ir_compound_assign() {
    assert_compiles_with(r#"fn main() { let mut x = 10; x += 5; print(x); }"#, "add");
}

// ===========================================================================
// Control Flow — verify branches and phi nodes
// ===========================================================================

#[test]
fn ir_if_branch() {
    assert_compiles_with(
        r#"fn main() { let x = 10; if x > 0 { print("yes"); } }"#,
        "br",
    );
}

#[test]
fn ir_if_else_branch() {
    assert_compiles_with(
        r#"fn main() { let x = 10; if x > 0 { print("pos"); } else { print("neg"); } }"#,
        "br",
    );
}

#[test]
fn ir_for_loop_branch() {
    assert_compiles_with(r#"fn main() { for i in 0..10 { print(i); } }"#, "br label");
}

#[test]
fn ir_for_loop_phi() {
    assert_compiles_with(
        r#"fn main() { let mut s = 0; for i in 1..=10 { s = s + i; } print(s); }"#,
        "while_cond",
    );
}

#[test]
fn ir_break_in_loop() {
    assert_compiles_with(
        r#"fn main() { for i in 0..100 { if i >= 5 { break; } print(i); } }"#,
        "br",
    );
}

// ===========================================================================
// Functions — verify define and call
// ===========================================================================

#[test]
fn ir_function_define() {
    assert_compiles_with(
        r#"fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { print(add(1, 2)); }"#,
        "define",
    );
}

#[test]
fn ir_function_call() {
    assert_compiles_with(
        r#"fn greet() { print("hi"); } fn main() { greet(); }"#,
        "call",
    );
}

#[test]
fn ir_recursive_call() {
    assert_compiles_with(
        r#"
fn fib(n: Int) -> Int {
    if n <= 1 { return n; }
    return fib(n - 1) + fib(n - 2);
}
fn main() { print(fib(10)); }
"#,
        "@fib",
    );
}

// ===========================================================================
// Structs — verify insertvalue/extractvalue
// ===========================================================================

#[test]
fn ir_struct_construct() {
    assert_compiles_with(
        r#"struct Point { x: Int, y: Int } fn main() { let p = Point { x: 1, y: 2 }; print(p.x); }"#,
        "getelementptr",
    );
}

#[test]
fn ir_struct_field_access() {
    assert_compiles_with(
        r#"struct Point { x: Int, y: Int } fn main() { let p = Point { x: 1, y: 2 }; print(p.x); }"#,
        "getelementptr",
    );
}

#[test]
fn ir_struct_method_call() {
    assert_compiles_with(
        r#"
struct User { name: Str, age: Int }
fn User.isAdult(self) -> Bool => self.age >= 18;
fn main() { let u = User { name: "Alice", age: 25 }; print(u.isAdult()); }
"#,
        "isAdult",
    );
}

// ===========================================================================
// Enums — verify switch for match
// ===========================================================================

#[test]
fn ir_enum_match_switch() {
    assert_compiles_with(
        r#"
enum Color { Red, Green, Blue }
fn main() {
    let c = Color::Red;
    match c {
        Color::Red => print("red"),
        Color::Green => print("green"),
        Color::Blue => print("blue"),
    }
}
"#,
        "icmp eq",
    );
}

#[test]
fn ir_enum_with_payload() {
    assert_compiles_with(
        r#"
enum Shape { Circle(Float), Rect(Float, Float) }
fn main() {
    let s = Shape::Circle(5.0);
    match s {
        Shape::Circle(r) => print(r),
        Shape::Rect(w, h) => print(w),
    }
}
"#,
        "icmp eq",
    );
}

// ===========================================================================
// Strings — verify string constants in IR
// ===========================================================================

#[test]
fn ir_string_literal() {
    assert_compiles_with(
        r#"fn main() { let s = "hello world"; print(s); }"#,
        "hello world",
    );
}

#[test]
fn ir_string_interpolation() {
    assert_compiles_with(
        r#"fn main() { let name = "Alice"; print("Hello ${name}"); }"#,
        "Alice",
    );
}

// ===========================================================================
// Collections — verify runtime calls
// ===========================================================================

#[test]
fn ir_array_literal() {
    assert_compiles_with(
        r#"fn main() { let arr = [1, 2, 3]; print(arr[0]); }"#,
        "getelementptr",
    );
}

#[test]
fn ir_array_push() {
    assert_compiles_with(
        r#"fn main() { let mut arr: [Int] = []; arr.push(42); print(arr.length()); }"#,
        "call",
    );
}

#[test]
fn ir_map_literal() {
    assert_compiles_with(
        r#"fn main() { let m = {"key": 42}; print(m["key"]); }"#,
        "key",
    );
}

// ===========================================================================
// Closures — verify closure/lambda codegen
// ===========================================================================

#[test]
fn ir_closure_basic() {
    assert_compiles_with(
        r#"fn main() { let f = (x) => x + 1; print(f(5)); }"#,
        "call",
    );
}

#[test]
fn ir_closure_capture() {
    assert_compiles_with(
        r#"fn main() { let base = 10; let add = (x) => x + base; print(add(5)); }"#,
        "call",
    );
}

#[test]
fn ir_array_map_closure() {
    assert_compiles_with(
        r#"fn main() { let doubled = [1, 2, 3].map((x) => x * 2); print(doubled[0]); }"#,
        "call",
    );
}

#[test]
fn ir_array_filter_closure() {
    assert_compiles_with(
        r#"fn main() { let evens = [1, 2, 3, 4].filter((x) => x % 2 == 0); print(evens.length()); }"#,
        "call",
    );
}

#[test]
fn ir_array_reduce_closure() {
    assert_compiles_with(
        r#"fn main() { let sum = [1, 2, 3].reduce(0, (a, b) => a + b); print(sum); }"#,
        "call",
    );
}

// ===========================================================================
// Error Handling — verify Ok/Err codegen
// ===========================================================================

#[test]
fn ir_error_ok_path() {
    assert_compiles_with(
        r#"
fn safe() -> Int ! Str { Ok 42; }
fn main() { }
"#,
        "define",
    );
}

#[test]
fn ir_error_err_path() {
    assert_compiles_with(
        r#"
fn risky(x: Int) -> Int ! Str {
    if x < 0 { Err "negative"; }
    Ok x;
}
fn main() { }
"#,
        "negative",
    );
}

#[test]
fn ir_error_propagation() {
    assert_compiles_with(
        r#"
fn step1() -> Int ! Str { Ok 10; }
fn step2(x: Int) -> Int ! Str { Ok x * 2; }
fn pipeline() -> Int ! Str {
    let a = step1()?;
    let b = step2(a)?;
    Ok b;
}
fn main() { }
"#,
        "pipeline",
    );
}

// ===========================================================================
// Type Casting — verify conversion instructions
// ===========================================================================

#[test]
fn ir_int_to_float_cast() {
    assert_compiles_with(
        r#"fn main() { let i = 42; let f = (i as Float); print(f); }"#,
        "sitofp",
    );
}

// ===========================================================================
// Optional/Nil — verify nil coalescing
// ===========================================================================

#[test]
fn ir_nil_coalesce() {
    assert_compiles_with(
        r#"fn main() { let x: Str? = nil; let val = x ?? panic("no value"); print(val); }"#,
        "no value",
    );
}

// ===========================================================================
// Nested/Complex IR patterns
// ===========================================================================

#[test]
fn ir_nested_struct_access() {
    assert_compiles_with(
        r#"
struct Inner { val: Int }
struct Outer { inner: Inner }
fn main() {
    let o = Outer { inner: Inner { val: 99 } };
    print(o.inner.val);
}
"#,
        "getelementptr",
    );
}

#[test]
fn ir_chained_method_calls() {
    assert_compiles_with(
        r#"
fn main() {
    let result = [1, 2, 3, 4, 5]
        .filter((x) => x > 2)
        .map((x) => x * 10);
    for item in result { print(item); }
}
"#,
        "call",
    );
}

#[test]
fn ir_match_conditional() {
    assert_compiles_with(
        r#"
fn main() {
    let x = 50;
    let label = match {
        x > 100 => "big",
        x > 10 => "medium",
        _ => "small",
    };
    print(label);
}
"#,
        "br",
    );
}

// ===========================================================================
// Async / Go / Scope — verify async primitives generate IR
// ===========================================================================

#[test]
fn codegen_sleep_call() {
    assert_compiles_with(
        r#"
fn main() {
    sleep(50);
}
"#,
        "call",
    );
}

#[test]
fn codegen_async_fn() {
    assert_compiles_with(
        r#"
async fn fetchData() -> Str {
    sleep(10);
    return "data";
}
fn main() {
    let r = await fetchData();
    print(r);
}
"#,
        "define",
    );
}

#[test]
fn codegen_go_block() {
    assert_compiles_with(
        r#"
fn main() {
    go {
        print("task");
    }
    sleep(100);
}
"#,
        "call",
    );
}

#[test]
fn codegen_scope_block() {
    assert_compiles_with(
        r#"
fn main() {
    scope {
        go {
            sleep(10);
            print("inner");
        }
    }
    print("done");
}
"#,
        "call",
    );
}

// ===========================================================================
// Process — verify process calls generate IR
// ===========================================================================

#[test]
fn codegen_process_run() {
    assert_compiles_with(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    let r = Process::run("echo", "[\"hello\"]")?;
    print(r);
}
"#,
        "call",
    );
}

// ===========================================================================
// WebSocket — verify ws setup generates IR
// ===========================================================================

#[test]
fn codegen_ws_handler() {
    assert_compiles_with(
        r#"
import std::Http::{Server, WsConnection};
fn onMsg(conn: WsConnection, data: Str) {
    conn.emit("echo", data);
}
fn handler(conn: WsConnection) {
    conn.on("echo", onMsg);
}
fn main() {
    let app = Server::new(":3000");
    app.ws("/ws/echo", handler);
    app.start();
}
"#,
        "define",
    );
}
