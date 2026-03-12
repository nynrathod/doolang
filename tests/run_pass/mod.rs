//! Run-pass tests — programs that compile, execute, and produce correct output
//! This is the GOLD STANDARD: verifies end-to-end correctness
//! Modeled after rustc's run-pass and Go's "// run" tests

use crate::common::{assert_doo_file_suite, DooTestMode};
use std::fs;
use std::path::Path;
use std::process::Command;

fn run_doo(file: &str, expected: &str) {
    let path = Path::new("tests/run_pass").join(file);
    assert!(path.exists(), "Test file not found: {:?}", path);

    // Use pre-built binary directly — never use `cargo run` (causes lock contention).
    let doo_bin: &[&str] = if cfg!(target_os = "windows") {
        &["target-windows/release/doo.exe"]
    } else if cfg!(target_os = "macos") {
        &["target/release/doo"]
    } else {
        // Linux: WSL or native
        &[
            r"\\wsl.localhost\Ubuntu\home\nayan\doo-builds\linux\release\doo",
            "target-linux/release/doo",
        ]
    };

    let bin = doo_bin
        .iter()
        .find(|p| Path::new(p).exists())
        .unwrap_or_else(|| {
            panic!(
            "No doo binary found. Build first with: cargo build --release --workspace\nTried: {:?}",
            doo_bin
        )
        });

    let output = Command::new(bin)
        .args(&["run", path.to_str().unwrap()])
        .output()
        .expect("Failed to run doo");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "Compilation/execution failed for {}:\nstderr: {}",
        file,
        stderr
    );
    assert!(
        stdout.contains(expected),
        "Expected '{}' in output of {}.\nGot: '{}'",
        expected,
        file,
        stdout
    );
}

/// Helper: write code to file then run it
fn run_code(category: &str, name: &str, code: &str, expected: &str) {
    let dir = format!("tests/run_pass/{}", category);
    fs::create_dir_all(&dir).ok();
    let file = format!("{}/{}.doo", category, name);
    fs::write(Path::new("tests/run_pass").join(&file), code).unwrap();
    run_doo(&file, expected);
}

// ===========================================================================
// Basic — Primitives & Print
// ===========================================================================

#[test]
fn hello_world() {
    run_code(
        "basic",
        "hello",
        r#"
fn main() {
    print("Hello, World!");
}
"#,
        "Hello, World!",
    );
}

#[test]
fn integer_arithmetic() {
    run_code(
        "basic",
        "arithmetic",
        r#"
fn main() {
    let x = 10 + 20;
    print(x);
}
"#,
        "30",
    );
}

#[test]
fn float_arithmetic() {
    run_code(
        "basic",
        "float_arith",
        r#"
fn main() {
    let x = 2.5 + 3.5;
    print(x);
}
"#,
        "6",
    );
}

#[test]
fn boolean_print() {
    run_code(
        "basic",
        "bool",
        r#"
fn main() {
    let x = true;
    print(x);
}
"#,
        "true",
    );
}

#[test]
fn string_interpolation() {
    run_code(
        "basic",
        "interpolation",
        r#"
fn main() {
    let name = "Alice";
    print("Hello ${name}!");
}
"#,
        "Hello Alice!",
    );
}

#[test]
fn string_interp_expr() {
    run_code(
        "basic",
        "interp_expr",
        r#"
fn main() {
    let x = 10;
    let y = 20;
    print("${x} + ${y} = ${x + y}");
}
"#,
        "10 + 20 = 30",
    );
}

#[test]
fn mutable_variable() {
    run_code(
        "basic",
        "mutable",
        r#"
fn main() {
    let mut x = 10;
    x = 20;
    print(x);
}
"#,
        "20",
    );
}

#[test]
fn compound_assign() {
    run_code(
        "basic",
        "compound",
        r#"
fn main() {
    let mut x = 10;
    x += 5;
    print(x);
}
"#,
        "15",
    );
}

#[test]
fn increment_decrement() {
    run_code(
        "basic",
        "inc_dec",
        r#"
fn main() {
    let mut x = 10;
    x++;
    print(x);
}
"#,
        "11",
    );
}

#[test]
fn type_cast_int_float() {
    run_code(
        "basic",
        "cast",
        r#"
fn main() {
    let i = 42;
    let f = (i as Float);
    print(f);
}
"#,
        "42",
    );
}

// ===========================================================================
// Functions
// ===========================================================================

#[test]
fn function_call() {
    run_code(
        "functions",
        "call",
        r#"
fn add(a: Int, b: Int) -> Int {
    return a + b;
}
fn main() {
    print(add(10, 20));
}
"#,
        "30",
    );
}

#[test]
fn function_expression_body() {
    run_code(
        "functions",
        "expr_body",
        r#"
fn double(x: Int) -> Int => x * 2;
fn main() {
    print(double(21));
}
"#,
        "42",
    );
}

#[test]
fn function_recursive() {
    run_code(
        "functions",
        "recursive",
        r#"
fn fib(n: Int) -> Int {
    if n <= 1 { return n; }
    return fib(n - 1) + fib(n - 2);
}
fn main() {
    print(fib(10));
}
"#,
        "55",
    );
}

#[test]
fn function_multiple_returns() {
    run_code(
        "functions",
        "multi_return",
        r#"
fn classify(x: Int) -> Str {
    if x > 100 { return "big"; }
    if x > 10 { return "medium"; }
    return "small";
}
fn main() {
    print(classify(50));
}
"#,
        "medium",
    );
}

// ===========================================================================
// Control Flow
// ===========================================================================

#[test]
fn if_else() {
    run_code(
        "control",
        "if_else",
        r#"
fn main() {
    let x = 10;
    if x > 0 {
        print("positive");
    } else {
        print("negative");
    }
}
"#,
        "positive",
    );
}

#[test]
fn else_if_chain() {
    run_code(
        "control",
        "else_if",
        r#"
fn main() {
    let x = 50;
    if x > 100 {
        print("big");
    } else if x > 10 {
        print("medium");
    } else {
        print("small");
    }
}
"#,
        "medium",
    );
}

#[test]
fn for_range_sum() {
    run_code(
        "control",
        "for_range",
        r#"
fn main() {
    let mut sum = 0;
    for i in 1..=10 {
        sum += i;
    }
    print(sum);
}
"#,
        "55",
    );
}

#[test]
fn for_break() {
    run_code(
        "control",
        "for_break",
        r#"
fn main() {
    let mut last = 0;
    for i in 0..100 {
        if i >= 5 { break; }
        last = i;
    }
    print(last);
}
"#,
        "4",
    );
}

#[test]
fn for_continue() {
    run_code(
        "control",
        "for_continue",
        r#"
fn main() {
    let mut sum = 0;
    for i in 0..10 {
        if i % 2 == 0 { continue; }
        sum += i;
    }
    print(sum);
}
"#,
        "25",
    );
}

#[test]
fn nested_loops() {
    run_code(
        "control",
        "nested_loops",
        r#"
fn main() {
    let mut count = 0;
    for i in 0..3 {
        for j in 0..3 {
            count++;
        }
    }
    print(count);
}
"#,
        "9",
    );
}

#[test]
fn while_loop() {
    run_code(
        "control",
        "while_loop",
        r#"
fn main() {
    let mut i = 0;
    for _ in 0..5 {
        i++;
    }
    print(i);
}
"#,
        "5",
    );
}

#[test]
fn match_conditional() {
    run_code(
        "control",
        "match_cond",
        r#"
fn main() {
    let x = 42;
    let label = match {
        x > 100 => "big",
        x > 10 => "medium",
        _ => "small",
    };
    print(label);
}
"#,
        "medium",
    );
}

#[test]
fn fizzbuzz() {
    run_code(
        "control",
        "fizzbuzz",
        r#"
fn main() {
    for i in 1..=15 {
        if i % 15 == 0 {
            print("FizzBuzz");
        } else if i % 3 == 0 {
            print("Fizz");
        } else if i % 5 == 0 {
            print("Buzz");
        } else {
            print(i);
        }
    }
}
"#,
        "FizzBuzz",
    );
}

// ===========================================================================
// Collections
// ===========================================================================

#[test]
fn array_access() {
    run_code(
        "collections",
        "array_access",
        r#"
fn main() {
    let arr = [10, 20, 30];
    print(arr[1]);
}
"#,
        "20",
    );
}

#[test]
fn array_length() {
    run_code(
        "collections",
        "array_len",
        r#"
fn main() {
    let arr = [1, 2, 3, 4, 5];
    print(arr.len());
}
"#,
        "5",
    );
}

#[test]
fn array_push() {
    run_code(
        "collections",
        "array_push",
        r#"
fn main() {
    let mut arr: [Int] = [];
    arr.push(1);
    arr.push(2);
    arr.push(3);
    print(arr.len());
}
"#,
        "3",
    );
}

#[test]
fn array_iterate() {
    run_code(
        "collections",
        "array_iter",
        r#"
fn main() {
    let mut sum = 0;
    let arr = [10, 20, 30];
    for item in arr {
        sum += item;
    }
    print(sum);
}
"#,
        "60",
    );
}

#[test]
fn array_iterate_index() {
    run_code(
        "collections",
        "array_idx_iter",
        r#"
fn main() {
    let arr = ["a", "b", "c"];
    for idx, val in arr {
        print("${idx}:${val}");
    }
}
"#,
        "0:a",
    );
}

#[test]
fn array_map() {
    run_code(
        "collections",
        "array_map",
        r#"
fn main() {
    let nums = [1, 2, 3];
    let doubled = nums.map((x) => x * 2);
    print(doubled[0]);
    print(doubled[1]);
    print(doubled[2]);
}
"#,
        "6",
    );
}

#[test]
fn array_filter() {
    run_code(
        "collections",
        "array_filter",
        r#"
fn main() {
    let nums = [1, 2, 3, 4, 5];
    let big = nums.filter((x) => x > 3);
    print(big.len());
}
"#,
        "2",
    );
}

#[test]
fn array_reduce() {
    run_code(
        "collections",
        "array_reduce",
        r#"
fn main() {
    let sum = [1, 2, 3, 4, 5].reduce(0, (a, b) => a + b);
    print(sum);
}
"#,
        "15",
    );
}

#[test]
fn array_chain() {
    run_code(
        "collections",
        "array_chain",
        r#"
fn main() {
    let result = [1, 2, 3, 4, 5]
        .filter((x) => x > 2)
        .map((x) => x * 10);
    print(result[0]);
}
"#,
        "30",
    );
}

#[test]
fn map_access() {
    run_code(
        "collections",
        "map_access",
        r#"
fn main() {
    let m = {"name": "Alice", "city": "NYC"};
    print(m["name"]);
}
"#,
        "Alice",
    );
}

#[test]
fn map_mutate() {
    run_code(
        "collections",
        "map_mutate",
        r#"
fn main() {
    let mut m = {"a": 1};
    m["b"] = 2;
    print(m["b"]);
}
"#,
        "2",
    );
}

#[test]
fn spread_operator() {
    run_code(
        "collections",
        "spread",
        r#"
fn main() {
    let a = [1, 2, 3];
    let b = [4, 5];
    let c = [...a, ...b];
    print(c.len());
}
"#,
        "5",
    );
}

// ===========================================================================
// Structs
// ===========================================================================

#[test]
fn struct_fields() {
    run_code(
        "structs",
        "fields",
        r#"
struct Point { x: Int, y: Int }
fn main() {
    let p = Point { x: 10, y: 20 };
    print(p.x + p.y);
}
"#,
        "30",
    );
}

#[test]
fn struct_method() {
    run_code(
        "structs",
        "method",
        r#"
struct Rect { w: Float, h: Float }
fn Rect.area(self) -> Float => self.w * self.h;
fn main() {
    let r = Rect { w: 5.0, h: 3.0 };
    print(r.area());
}
"#,
        "15",
    );
}

#[test]
fn struct_nested() {
    run_code(
        "structs",
        "nested",
        r#"
struct Address { city: Str }
struct User { name: Str, address: Address }
fn main() {
    let u = User { name: "Alice", address: Address { city: "NYC" } };
    print(u.address.city);
}
"#,
        "NYC",
    );
}

#[test]
fn struct_mut_field() {
    run_code(
        "structs",
        "mut_field",
        r#"
struct Counter { val: Int }
fn main() {
    let mut c = Counter { val: 0 };
    c.val = 42;
    print(c.val);
}
"#,
        "42",
    );
}

#[test]
fn struct_in_array() {
    run_code(
        "structs",
        "in_array",
        r#"
struct Item { name: Str, price: Int }
fn main() {
    let items = [
        Item { name: "A", price: 10 },
        Item { name: "B", price: 20 },
    ];
    print(items[1].price);
}
"#,
        "20",
    );
}

#[test]
fn struct_method_string() {
    run_code(
        "structs",
        "method_str",
        r#"
struct User { name: Str, age: Int }
fn User.greet(self) -> Str => "Hi, I'm ${self.name}";
fn main() {
    let u = User { name: "Bob", age: 30 };
    print(u.greet());
}
"#,
        "Hi, I'm Bob",
    );
}

// ===========================================================================
// Enums & Match
// ===========================================================================

#[test]
fn enum_match() {
    run_code(
        "enums",
        "basic_match",
        r#"
enum Color { Red, Green, Blue }
fn main() {
    let c = Color::Green;
    match c {
        Color::Red => print("red"),
        Color::Green => print("green"),
        Color::Blue => print("blue"),
    }
}
"#,
        "green",
    );
}

#[test]
fn enum_with_payload() {
    run_code(
        "enums",
        "payload",
        r#"
enum Shape {
    Circle(Int),
    Rectangle(Int, Int),
}
fn area(s: Shape) -> Int {
    match s {
        Shape::Circle(r) => r * r,
        Shape::Rectangle(w, h) => w * h,
    }
}
fn main() {
    print(area(Shape::Rectangle(3, 4)));
}
"#,
        "12",
    );
}

#[test]
fn enum_method() {
    run_code(
        "enums",
        "method",
        r#"
enum Priority { Low, Medium, High }
fn Priority.label(self) -> Str {
    return match self {
        Priority::Low => "low",
        Priority::Medium => "medium",
        Priority::High => "high",
    };
}
fn main() {
    print(Priority::High.label());
}
"#,
        "high",
    );
}

#[test]
fn enum_in_struct() {
    run_code(
        "enums",
        "in_struct",
        r#"
enum Status { Active, Inactive }
struct User { name: Str, status: Status }
fn main() {
    let u = User { name: "Alice", status: Status::Active };
    let label = match u.status {
        Status::Active => "active",
        Status::Inactive => "inactive",
    };
    print(label);
}
"#,
        "active",
    );
}

// ===========================================================================
// Error Handling
// ===========================================================================

#[test]
fn error_ok_path() {
    run_code(
        "errors",
        "ok_path",
        r#"
fn divide(a: Int, b: Int) -> Int ! Str {
    if b == 0 { Err "division by zero"; }
    Ok a / b;
}
fn main() {
    let v, e = divide(10, 2);
    if e == nil { print(v); }
}
"#,
        "5",
    );
}

#[test]
fn error_err_path() {
    run_code(
        "errors",
        "err_path",
        r#"
fn divide(a: Int, b: Int) -> Int ! Str {
    if b == 0 { Err "division by zero"; }
    Ok a / b;
}
fn main() {
    let _, e = divide(10, 0);
    if e != nil { print(e); }
}
"#,
        "division by zero",
    );
}

#[test]
fn error_propagation() {
    run_code(
        "errors",
        "propagation",
        r#"
fn step1() -> Int ! Str { Ok 10; }
fn step2(x: Int) -> Int ! Str { Ok x * 2; }
fn pipeline() -> Int ! Str {
    let a = step1()?;
    let b = step2(a)?;
    Ok b;
}
fn main() {
    let v, e = pipeline();
    if e == nil { print(v); }
}
"#,
        "20",
    );
}

#[test]
fn error_nil_coalesce() {
    run_code(
        "errors",
        "nil_coalesce",
        r#"
fn getValue() -> Str ! Str {
    Ok "hello";
}
fn main() {
    let v, _ = getValue();
    print(v);
}
"#,
        "hello",
    );
}

// ===========================================================================
// Closures
// ===========================================================================

#[test]
fn closure_capture() {
    run_code(
        "closures",
        "capture",
        r#"
fn main() {
    let nums = [10, 20, 30];
    let result = nums.map((x) => x * 2);
    print(result[1]);
}
"#,
        "40",
    );
}

#[test]
fn closure_in_map() {
    run_code(
        "closures",
        "in_map",
        r#"
fn main() {
    let nums = [1, 2, 3];
    let doubled = nums.map((x) => x * 2);
    for item in doubled { print(item); }
}
"#,
        "6",
    );
}

// ===========================================================================
// Auto-Ownership (Doo magic — use after pass)
// ===========================================================================

#[test]
fn auto_clone_str() {
    run_code(
        "ownership",
        "clone_str",
        r#"
fn take(s: Str) { print(s); }
fn main() {
    let s = "hello";
    take(s);
    take(s);
    print(s);
}
"#,
        "hello",
    );
}

#[test]
fn auto_clone_struct() {
    run_code(
        "ownership",
        "clone_struct",
        r#"
struct Point { x: Int, y: Int }
fn show(p: Point) { print(p.x); }
fn main() {
    let p = Point { x: 42, y: 10 };
    show(p);
    print(p.y);
}
"#,
        "10",
    );
}

#[test]
fn auto_clone_array() {
    run_code(
        "ownership",
        "clone_array",
        r#"
fn process(arr: [Int]) { print(arr.len()); }
fn main() {
    let data = [1, 2, 3];
    process(data);
    process(data);
    print(data[0]);
}
"#,
        "1",
    );
}

// ===========================================================================
// Realistic Programs
// ===========================================================================

#[test]
fn realistic_factorial() {
    run_code(
        "realistic",
        "factorial",
        r#"
fn fact(n: Int) -> Int {
    if n <= 1 { return 1; }
    return n * fact(n - 1);
}
fn main() {
    print(fact(10));
}
"#,
        "3628800",
    );
}

#[test]
fn realistic_todo() {
    run_code(
        "realistic",
        "todo",
        r#"
enum Status { Pending, Done }
struct Todo { title: Str, status: Status }
fn Todo.isDone(self) -> Bool {
    return match self.status {
        Status::Done => true,
        _ => false,
    };
}
fn main() {
    let todos = [
        Todo { title: "Buy milk", status: Status::Done },
        Todo { title: "Write code", status: Status::Pending },
    ];
    for todo in todos {
        if todo.isDone() { print("DONE: ${todo.title}"); }
        else { print("TODO: ${todo.title}"); }
    }
}
"#,
        "DONE: Buy milk",
    );
}

#[test]
fn realistic_calculator() {
    run_code(
        "realistic",
        "calc",
        r#"
fn calc(op: Str, a: Int, b: Int) -> Int ! Str {
    if op == "add" { Ok a + b; }
    if op == "sub" { Ok a - b; }
    if op == "mul" { Ok a * b; }
    if op == "div" {
        if b == 0 { Err "division by zero"; }
        Ok a / b;
    }
    Err "unknown op";
}
fn main() {
    let v, e = calc("mul", 6, 7);
    if e == nil { print(v); }
}
"#,
        "42",
    );
}

#[test]
fn realistic_user_filter() {
    run_code(
        "realistic",
        "user_filter",
        r#"
struct User { name: Str, age: Int }
fn User.isAdult(self) -> Bool => self.age >= 18;
fn main() {
    let users = [
        User { name: "Alice", age: 25 },
        User { name: "Bob", age: 15 },
        User { name: "Charlie", age: 30 },
    ];
    let adults = users.filter((u) => u.isAdult());
    print(adults.len());
}
"#,
        "2",
    );
}

// ===========================================================================
// Auto-discovered .doo run-pass files (compile verification)
// ===========================================================================

#[test]
fn run_pass_doo_files_compile() {
    let dir = std::path::Path::new("tests/run_pass");
    assert_doo_file_suite(dir, DooTestMode::CompilePass, "run_pass");
}
