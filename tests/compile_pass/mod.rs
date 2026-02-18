//! Compile-pass tests — programs that MUST compile successfully
//! Uses the real compiler pipeline via common::compile_snippet
//! Verifies IR output contains expected patterns via common::assert_compiles_with

use crate::common::{
    assert_compiles, assert_compiles_with, assert_doo_file_suite, compile_snippet, DooTestMode,
};

// ===========================================================================
// Basic Tests
// ===========================================================================

#[test]
fn basic_hello_world() {
    assert_compiles_with(r#"fn main() { print("Hello, World!"); }"#, "Hello, World!");
}

#[test]
fn basic_arithmetic() {
    // 1 + 2 * 3 = 7. Constant folding might happen, or runtime eval.
    // If constant folded: "7" might be in IR. If not: look for main.
    // Let's assume constant folding isn't rigorous yet, but we check for "print".
    assert_compiles_with(r#"fn main() { let x = 1 + 2 * 3; print(x); }"#, "call");
}

#[test]
fn basic_variables_and_types() {
    assert_compiles_with(
        r#"
fn main() {
    let i: Int = 42;
    let s: Str = "hello";
    let b: Bool = true;
    let f: Float = 3.14;
    print(i);
    print(s);
    print(b);
    print(f);
}
"#,
        "hello",
    );
}

#[test]
fn basic_mutable_variables() {
    assert_compiles_with(
        r#"
fn main() {
    let mut x = 10;
    print(x);
    x = 20;
    print(x);
}
"#,
        "store", // mutation uses store
    );
}

#[test]
fn basic_string_interpolation() {
    assert_compiles_with(
        r#"
fn main() {
    let name = "Alice";
    print("Hello, ${name}!");
}
"#,
        "Alice",
    );
}

#[test]
fn basic_type_inference() {
    assert_compiles_with(
        r#"
fn main() {
    let x = 42;
    let s = "hello";
    print(x);
    print(s);
}
"#,
        "hello",
    );
}

// ===========================================================================
// Functions
// ===========================================================================

#[test]
fn function_add() {
    assert_compiles_with(
        r#"
fn add(a: Int, b: Int) -> Int {
    return a + b;
}
fn main() {
    let result = add(5, 3);
    print(result);
}
"#,
        "define", // Defines a function
    );
}

#[test]
fn function_expression_syntax() {
    assert_compiles_with(
        r#"fn double(x: Int) -> Int => x * 2; fn main() { print(double(5)); }"#,
        "double",
    );
}

#[test]
fn function_recursive() {
    assert_compiles_with(
        r#"
fn factorial(n: Int) -> Int {
    if n <= 1 { return 1; }
    return n * factorial(n - 1);
}
fn main() { print(factorial(5)); }
"#,
        "factorial",
    );
}

#[test]
fn function_multiple_returns() {
    assert_compiles_with(
        r#"
fn classify(x: Int) -> Str {
    if x > 100 { return "big"; }
    if x > 10 { return "medium"; }
    return "small";
}
fn main() { print(classify(50)); }
"#,
        "medium",
    );
}

// ===========================================================================
// Control Flow
// ===========================================================================

#[test]
fn if_else() {
    assert_compiles_with(
        r#"
fn main() {
    let x = 10;
    if x > 0 {
        print("positive");
    } else {
        print("non-positive");
    }
}
"#,
        "positive",
    );
}

#[test]
fn else_if_chain() {
    assert_compiles_with(
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
fn for_range_loop() {
    assert_compiles_with(
        r#"
fn main() {
    let mut sum = 0;
    for i in 1..=10 {
        sum = sum + i;
    }
    print(sum);
}
"#,
        "br label", // Loops use branch
    );
}

#[test]
fn for_array_loop() {
    assert_compiles_with(
        r#"
fn main() {
    let arr = [1, 2, 3];
    let mut sum = 0;
    for item in arr {
        sum = sum + item;
    }
    print(sum);
}
"#,
        "while_cond", // For-in loops compile to index-based while loops
    );
}

#[test]
fn match_expression() {
    assert_compiles_with(
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
fn break_and_continue() {
    assert_compiles_with(
        r#"
fn main() {
    let mut sum = 0;
    for i in 0..100 {
        if i >= 10 { break; }
        if i % 2 == 0 { continue; }
        sum = sum + i;
    }
    print(sum);
}
"#,
        "br",
    );
}

#[test]
fn fizzbuzz() {
    assert_compiles_with(
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
// Structs and Methods
// ===========================================================================

#[test]
fn struct_definition_and_access() {
    assert_compiles_with(
        r#"
struct Point { x: Int, y: Int }
fn main() {
    let p = Point { x: 10, y: 20 };
    print(p.x + p.y);
}
"#,
        "getelementptr", // Struct fields accessed via GEP
    );
}

#[test]
fn struct_method() {
    assert_compiles_with(
        r#"
struct User { name: Str, age: Int }
fn User.greet(self) -> Str => "${self.name} is ${self.age}";
fn main() {
    let u = User { name: "Alice", age: 30 };
    print(u.greet());
}
"#,
        "Alice",
    );
}

#[test]
fn nested_structs() {
    assert_compiles_with(
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
fn struct_optional_field() {
    assert_compiles_with(
        r#"
struct User { name: Str, email: Str? }
fn main() {
    let u = User { name: "Alice", email: "a@b.com" };
    let msg = u.email ?? panic("no email");
    print(msg);
}
"#,
        "unwrap_ok", // Optional unwrap generates unwrap_ok/err branches
    );
}

#[test]
fn struct_multiple_methods() {
    assert_compiles_with(
        r#"
struct Rect { w: Float, h: Float }
fn Rect.area(self) -> Float => self.w * self.h;
fn Rect.perimeter(self) -> Float => 2.0 * (self.w + self.h);
fn Rect.isSquare(self) -> Bool => self.w == self.h;
fn main() {
    let r = Rect { w: 5.0, h: 5.0 };
    print(r.area());
    print(r.perimeter());
    print(r.isSquare());
}
"#,
        "area",
    );
}

// ===========================================================================
// Enums
// ===========================================================================

#[test]
fn enum_simple_match() {
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
        "red",
    );
}

#[test]
fn enum_in_struct() {
    assert_compiles_with(
        r#"
enum Status { Active, Inactive }
struct User { name: Str, status: Status }
fn main() {
    let u = User { name: "Alice", status: Status::Active };
    let label = match u.status {
        Status::Active => "Active",
        Status::Inactive => "Inactive",
    };
    print(label);
}
"#,
        "Active",
    );
}

#[test]
fn enum_with_payload() {
    assert_compiles_with(
        r#"
enum Shape {
    Circle(Float),
    Rectangle(Float, Float),
}
fn area(s: Shape) -> Float {
    match s {
        Shape::Circle(r) => 3.14 * r * r,
        Shape::Rectangle(w, h) => w * h,
    }
}
fn main() {
    print(area(Shape::Circle(5.0)));
    print(area(Shape::Rectangle(3.0, 4.0)));
}
"#,
        "icmp", // Match compiles to icmp+br chains
    );
}

#[test]
fn enum_method() {
    assert_compiles_with(
        r#"
enum Priority { Low, Medium, High }
fn Priority.label(self) -> Str {
    match self {
        Priority::Low => "low",
        Priority::Medium => "medium",
        Priority::High => "high",
    }
}
fn main() {
    let p = Priority::High;
    print(p.label());
}
"#,
        "high",
    );
}

// ===========================================================================
// Collections
// ===========================================================================

#[test]
fn array_operations() {
    assert_compiles_with(
        r#"
fn main() {
    let arr = [10, 20, 30];
    print(arr.length());
    print(arr[0]);
}
"#,
        "getelementptr", // Array access via GEP
    );
}

#[test]
fn mutable_array_push() {
    assert_compiles_with(
        r#"
fn main() {
    let mut arr: [Int] = [];
    arr.push(1);
    arr.push(2);
    arr.push(3);
    print(arr.length());
}
"#,
        "call", // push calls a builtin
    );
}

#[test]
fn map_operations() {
    assert_compiles_with(
        r#"
fn main() {
    let m = {"key": 42, "other": 10};
    print(m["key"]);
}
"#,
        "key",
    );
}

#[test]
fn array_map_filter() {
    assert_compiles_with(
        r#"
fn main() {
    let data = [1, 2, 3, 4, 5];
    let doubled = data.map((x) => x * 2);
    let big = doubled.filter((x) => x > 4);
    for item in big { print(item); }
}
"#,
        "map",
    );
}

// ===========================================================================
// Closures
// ===========================================================================

#[test]
fn closure_capture() {
    assert_compiles_with(
        r#"
fn main() {
    let base = 10;
    let add = (x) => x + base;
    print(add(5));
}
"#,
        "closure", // Assuming closure implementation produces something recognizable
    );
}

#[test]
fn closure_in_map_chain() {
    assert_compiles_with(
        r#"
fn main() {
    let doubled = [1, 2, 3].map((x) => x * 2);
    let mut sum = 0;
    for item in doubled { sum = sum + item; }
    print(sum);
}
"#,
        "call",
    );
}

#[test]
fn nested_closures() {
    assert_compiles_with(
        r#"
fn main() {
    let a = 1;
    let f = () => {
        let b = 2;
        let g = () => a + b;
        g()
    };
    print(f());
}
"#,
        "call",
    );
}

// ===========================================================================
// Error Handling
// ===========================================================================

#[test]
fn error_ok_err() {
    assert_compiles_with(
        r#"
fn divide(a: Int, b: Int) -> Int ! Str {
    if b == 0 { Err "division by zero"; }
    Ok a / b;
}
fn main() {
    let v, e = divide(84, 2);
    if e == nil { print(v); }
    else { print(e); }
}
"#,
        "division by zero",
    );
}

#[test]
fn error_propagation() {
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
        "step1",
    );
}

#[test]
fn error_return_ok_err() {
    assert_compiles_with(
        r#"
fn validate(x: Int) -> Int ! Str {
    if x < 0 { return Err "negative"; }
    if x > 100 { return Err "too large"; }
    return Ok x;
}
fn main() { }
"#,
        "negative",
    );
}

#[test]
fn optional_nil_coalesce() {
    assert_compiles_with(
        r#"
fn main() {
    let x: Str? = "hello";
    let val = x ?? panic("default expected");
    print(val);
}
"#,
        "extractvalue", // Optional unwrap uses extractvalue
    );
}

// ===========================================================================
// Auto-Ownership (compiler manages clone/copy/drop magically)
// ===========================================================================

#[test]
fn auto_clone_use_after_pass() {
    assert_compiles_with(
        r#"
fn greet(name: Str) { print(name); }
fn main() {
    let name = "Alice";
    greet(name);
    print(name);
}
"#,
        "Alice",
    );
}

#[test]
fn auto_clone_multiple_reads() {
    assert_compiles_with(
        r#"
fn main() {
    let s = "hello";
    print(s);
    print(s);
    print(s);
}
"#,
        "hello",
    );
}

#[test]
fn auto_ownership_reassign_after_use() {
    assert_compiles_with(
        r#"
fn consume(x: Str) { print(x); }
fn main() {
    let mut s = "first";
    consume(s);
    s = "second";
    print(s);
}
"#,
        "second",
    );
}

#[test]
fn auto_ownership_closure_capture_reuse() {
    assert_compiles_with(
        r#"
fn main() {
    let name = "Alice";
    let greet = () => "Hello, ${name}!";
    let msg = greet();
    print(msg);
    print(name);
}
"#,
        "Alice",
    );
}

#[test]
fn auto_ownership_array_pass_and_reuse() {
    assert_compiles_with(
        r#"
fn sumAll(arr: [Int]) -> Int {
    let mut s = 0;
    for item in arr { s = s + item; }
    return s;
}
fn main() {
    let data = [1, 2, 3, 4, 5];
    let total = sumAll(data);
    print(total);
    print(data.length());
}
"#,
        "sumAll",
    );
}

#[test]
fn auto_ownership_struct_pass_and_reuse() {
    assert_compiles_with(
        r#"
struct User { name: Str, age: Int }
fn describe(u: User) -> Str { return "${u.name}: ${u.age}"; }
fn main() {
    let u = User { name: "Bob", age: 25 };
    let desc = describe(u);
    print(desc);
    print(u.name);
}
"#,
        "Bob",
    );
}

#[test]
fn auto_ownership_multiple_function_passes() {
    assert_compiles_with(
        r#"
fn first(s: Str) { print(s); }
fn second(s: Str) { print(s); }
fn third(s: Str) { print(s); }
fn main() {
    let msg = "hello";
    first(msg);
    second(msg);
    third(msg);
}
"#,
        "hello",
    );
}

#[test]
fn auto_ownership_nested_struct_pass() {
    assert_compiles_with(
        r#"
struct Inner { val: Int }
struct Outer { inner: Inner, name: Str }
fn getVal(o: Outer) -> Int { return o.inner.val; }
fn main() {
    let o = Outer { inner: Inner { val: 42 }, name: "test" };
    let v = getVal(o);
    print(v);
    print(o.name);
}
"#,
        "test",
    );
}

// ===========================================================================
// Complex Programs
// ===========================================================================

#[test]
fn complex_struct_pipeline() {
    assert_compiles_with(
        r#"
enum Status { Active, Inactive, Pending }
struct User { name: Str, email: Str, status: Status, age: Int }
fn User.isActive(self) -> Bool { match self.status { Status::Active => true, _ => false } }
fn User.describe(self) -> Str => "${self.name} (${self.email})";

fn processUsers(users: [User]) -> [Str] {
    return users.filter((u) => u.isActive()).map((u) => u.describe());
}

fn main() {
    let users = [
        User { name: "Alice", email: "a@x.com", status: Status::Active, age: 25 },
        User { name: "Bob", email: "b@x.com", status: Status::Inactive, age: 30 },
        User { name: "Charlie", email: "c@x.com", status: Status::Active, age: 22 },
    ];
    let active = processUsers(users);
    for desc in active { print(desc); }
}
"#,
        "describe",
    );
}

#[test]
fn complex_error_chain() {
    assert_compiles_with(
        r#"
fn validateName(name: Str) -> Str ! Str {
    if name.length() == 0 { Err "name empty"; }
    Ok name;
}
fn validateAge(age: Int) -> Int ! Str {
    if age < 0 { Err "age negative"; }
    if age > 150 { Err "age too large"; }
    Ok age;
}
fn createUser(name: Str, age: Int) -> Str ! Str {
    let n = validateName(name)?;
    let a = validateAge(age)?;
    Ok "${n} (age ${a})";
}
fn main() { }
"#,
        "negative",
    );
}

// ===========================================================================
// Edge Cases — Operators
// ===========================================================================

#[test]
fn operator_precedence() {
    assert_compiles_with(
        r#"fn main() { let x = 2 + 3 * 4; print(x); }"#,
        "store i64 14", // Compiler constant-folds 2 + 3*4 = 14
    );
}

#[test]
fn modulo_operator() {
    assert_compiles_with(
        r#"fn main() { let x = 10 % 3; print(x); }"#,
        "store i64 1", // Compiler constant-folds 10 % 3 = 1
    );
}

#[test]
fn compound_assignments() {
    assert_compiles_with(
        r#"fn main() { let mut x = 10; x += 5; x -= 2; x *= 3; print(x); }"#,
        "store",
    );
}

#[test]
fn increment_decrement() {
    assert_compiles_with(
        r#"fn main() { let mut x = 10; x++; x--; print(x); }"#,
        "add",
    );
}

#[test]
fn logical_operators() {
    assert_compiles_with(
        r#"fn main() { let x = true && false; let y = true || false; let z = !true; print(x); }"#,
        "i1", // Boolean type in LLVM IR (constants are folded)
    );
}

#[test]
fn comparison_chain() {
    assert_compiles_with(
        r#"
fn main() {
    let a = 5;
    let b = 10;
    let eq = a == b;
    let neq = a != b;
    let lt = a < b;
    let gt = a > b;
    let lte = a <= b;
    let gte = a >= b;
    print(eq);
}
"#,
        "icmp",
    );
}

#[test]
fn float_arithmetic() {
    assert_compiles_with(
        r#"fn main() { let x = 1.5 + 2.5; let y = 3.0 * 4.0; print(x); print(y); }"#,
        "double", // Float type in LLVM IR (constants are folded)
    );
}

// ===========================================================================
// Edge Cases — Variables & Scoping
// ===========================================================================

#[test]
fn variable_shadowing_in_scope() {
    assert_compiles(
        r#"
fn main() {
    let x = 10;
    if true {
        let y = x + 1;
        print(y);
    }
    print(x);
}
"#,
    );
}

#[test]
fn multiple_lets_different_types() {
    assert_compiles(
        r#"
fn main() {
    let i: Int = 42;
    let f: Float = 3.14;
    let s: Str = "hello";
    let b: Bool = true;
    print(i);
    print(f);
    print(s);
    print(b);
}
"#,
    );
}

// ===========================================================================
// Edge Cases — Collections
// ===========================================================================

#[test]
fn empty_array_typed() {
    assert_compiles(r#"fn main() { let arr: [Int] = []; print(arr.length()); }"#);
}

#[test]
fn nested_array() {
    assert_compiles(r#"fn main() { let arr = [[1, 2], [3, 4]]; print(arr[0][0]); }"#);
}

#[test]
fn array_in_struct() {
    assert_compiles(
        r#"
struct Team { name: Str, scores: [Int] }
fn main() {
    let t = Team { name: "A", scores: [10, 20, 30] };
    print(t.name);
    print(t.scores.length());
}
"#,
    );
}

#[test]
fn map_different_key_types() {
    assert_compiles(r#"fn main() { let m = {1: "one", 2: "two"}; print(m[1]); }"#);
}

#[test]
fn map_in_struct() {
    assert_compiles(
        r#"
struct Config { settings: {Str: Str} }
fn main() {
    let c = Config { settings: {"key": "value"} };
    print(c.settings["key"]);
}
"#,
    );
}

#[test]
fn spread_operator() {
    assert_compiles(
        r#"
fn main() {
    let a = [1, 2, 3];
    let b = [4, 5, 6];
    let combined = [...a, ...b];
    print(combined.length());
}
"#,
    );
}

#[test]
fn array_chained_operations() {
    assert_compiles_with(
        r#"
fn main() {
    let result = [1, 2, 3, 4, 5]
        .filter((x) => x > 2)
        .map((x) => x * 10)
        .reduce(0, (a, b) => a + b);
    print(result);
}
"#,
        "call",
    );
}

// ===========================================================================
// Edge Cases — Strings
// ===========================================================================

#[test]
fn empty_string() {
    assert_compiles(r#"fn main() { let s = ""; print(s); }"#);
}

#[test]
fn string_interpolation_expr() {
    assert_compiles_with(
        r#"fn main() { let x = 10; let y = 5; print("${x} + ${y} = ${x + y}"); }"#,
        "call",
    );
}

#[test]
fn string_methods() {
    assert_compiles(
        r#"
fn main() {
    let s = "hello world";
    print(s.length());
}
"#,
    );
}

// ===========================================================================
// Edge Cases — Enums with Payloads
// ===========================================================================

#[test]
fn enum_payload_match_all_variants() {
    assert_compiles_with(
        r#"
enum HttpCode {
    Success(Int),
    NotFound(Int),
    ServerError(Str),
}
fn describe(code: HttpCode) -> Str {
    match code {
        HttpCode::Success(val) => "OK: ${val}",
        HttpCode::NotFound(val) => "404: ${val}",
        HttpCode::ServerError(msg) => "Error: ${msg}",
    }
}
fn main() {
    print(describe(HttpCode::Success(200)));
    print(describe(HttpCode::ServerError("timeout")));
}
"#,
        "icmp", // Match compiles to icmp+br chains
    );
}

#[test]
fn inline_enum() {
    assert_compiles(
        r#"
enum Color: Red | Green | Blue;
fn main() {
    let c = Color::Red;
    print(c);
}
"#,
    );
}

// ===========================================================================
// Edge Cases — Error Handling
// ===========================================================================

#[test]
fn error_tuple_return() {
    assert_compiles(
        r#"
fn getUser(id: Int) -> Str, Int ! Str {
    if id < 0 { Err "invalid"; }
    Ok "Alice", 25;
}
fn main() { }
"#,
    );
}

#[test]
fn error_ignore_value() {
    assert_compiles(
        r#"
fn risky() -> Int ! Str { Ok 42; }
fn main() {
    let _, err = risky();
    if err != nil { print(err); }
}
"#,
    );
}

#[test]
fn error_ignore_error() {
    assert_compiles(
        r#"
fn risky() -> Int ! Str { Ok 42; }
fn main() {
    let val, _ = risky();
    print(val);
}
"#,
    );
}

#[test]
fn error_chain_three_deep() {
    assert_compiles(
        r#"
fn step1() -> Int ! Str { Ok 1; }
fn step2(x: Int) -> Int ! Str { Ok x + 1; }
fn step3(x: Int) -> Int ! Str { Ok x + 1; }
fn pipeline() -> Int ! Str {
    let a = step1()?;
    let b = step2(a)?;
    let c = step3(b)?;
    Ok c;
}
fn main() { }
"#,
    );
}

// ===========================================================================
// Edge Cases — Optional / Nil
// ===========================================================================

#[test]
fn optional_field_nil() {
    assert_compiles(
        r#"
struct User { name: Str, email: Str? }
fn main() {
    let u = User { name: "Alice", email: nil };
    let e = u.email ?? panic("none");
    print(e);
}
"#,
    );
}

#[test]
fn optional_field_value() {
    assert_compiles(
        r#"
struct User { name: Str, email: Str? }
fn main() {
    let u = User { name: "Alice", email: "a@b.com" };
    let e = u.email ?? panic("none");
    print(e);
}
"#,
    );
}

// ===========================================================================
// Edge Cases — Type Casting
// ===========================================================================

#[test]
fn cast_int_to_float() {
    assert_compiles_with(
        r#"fn main() { let i = 42; let f = (i as Float); print(f); }"#,
        "sitofp",
    );
}

// ===========================================================================
// Edge Cases — Closures Advanced
// ===========================================================================

#[test]
fn closure_as_function_param() {
    assert_compiles(
        r#"
fn main() {
    let double = (x) => x * 2;
    let result = double(21);
    print(result);
}
"#,
    );
}

#[test]
fn closure_multi_capture() {
    assert_compiles(
        r#"
fn main() {
    let a = 10;
    let b = 20;
    let c = 30;
    let sum = () => a + b + c;
    print(sum());
}
"#,
    );
}

// ===========================================================================
// Edge Cases — Match Advanced
// ===========================================================================

#[test]
fn match_wildcard_only() {
    assert_compiles(
        r#"
fn main() {
    let x = 42;
    let label = match { _ => "anything" };
    print(label);
}
"#,
    );
}

#[test]
fn match_enum_method_call() {
    assert_compiles_with(
        r#"
enum Priority { Low, Medium, High }
fn Priority.label(self) -> Str {
    match self {
        Priority::Low => "low",
        Priority::Medium => "medium",
        Priority::High => "high",
    }
}
fn main() { print(Priority::High.label()); }
"#,
        "label",
    );
}

// ===========================================================================
// Edge Cases — Struct Advanced
// ===========================================================================

#[test]
fn struct_expression_method() {
    assert_compiles(
        r#"
struct Rect { w: Float, h: Float }
fn Rect.area(self) -> Float => self.w * self.h;
fn Rect.perimeter(self) -> Float => 2.0 * (self.w + self.h);
fn main() {
    let r = Rect { w: 5.0, h: 3.0 };
    print(r.area());
    print(r.perimeter());
}
"#,
    );
}

#[test]
fn struct_with_enum_field() {
    assert_compiles(
        r#"
enum Role { Admin, User, Guest }
struct Account { name: Str, role: Role }
fn main() {
    let a = Account { name: "Alice", role: Role::Admin };
    print(a.name);
}
"#,
    );
}

#[test]
fn struct_array_of_structs() {
    assert_compiles(
        r#"
struct Item { name: Str, price: Int }
fn main() {
    let items = [
        Item { name: "A", price: 10 },
        Item { name: "B", price: 20 },
    ];
    for item in items { print(item.name); }
}
"#,
    );
}

// ===========================================================================
// Edge Cases — For Loop Variants
// ===========================================================================

#[test]
fn for_range_exclusive() {
    assert_compiles(r#"fn main() { for i in 0..5 { print(i); } }"#);
}

#[test]
fn for_range_inclusive() {
    assert_compiles(r#"fn main() { for i in 0..=5 { print(i); } }"#);
}

#[test]
fn for_array_with_index() {
    assert_compiles(
        r#"
fn main() {
    let arr = ["a", "b", "c"];
    for idx, val in arr { print(idx); print(val); }
}
"#,
    );
}

#[test]
fn for_nested_loops() {
    assert_compiles(
        r#"
fn main() {
    for i in 0..3 {
        for j in 0..3 {
            print(i + j);
        }
    }
}
"#,
    );
}

#[test]
fn while_loop_basic() {
    assert_compiles(
        r#"
fn main() {
    let mut i = 0;
    for {
        if i >= 10 { break; }
        print(i);
        i++;
    }
}
"#,
    );
}

// ===========================================================================
// Edge Cases — Complex Realistic Programs
// ===========================================================================

#[test]
fn realistic_todo_list() {
    assert_compiles(
        r#"
enum Status { Pending, Done }
struct Todo { title: Str, status: Status }
fn Todo.isDone(self) -> Bool {
    match self.status {
        Status::Done => true,
        _ => false,
    }
}
fn main() {
    let todos = [
        Todo { title: "Buy groceries", status: Status::Pending },
        Todo { title: "Write code", status: Status::Done },
    ];
    for todo in todos {
        if todo.isDone() { print("DONE: ${todo.title}"); }
        else { print("TODO: ${todo.title}"); }
    }
}
"#,
    );
}

#[test]
fn realistic_calculator() {
    assert_compiles(
        r#"
fn calc(op: Str, a: Int, b: Int) -> Int ! Str {
    if op == "add" { return Ok a + b; }
    if op == "sub" { return Ok a - b; }
    if op == "mul" { return Ok a * b; }
    if op == "div" {
        if b == 0 { return Err "division by zero"; }
        return Ok a / b;
    }
    Err "unknown op";
}
fn main() {
    let result, err = calc("add", 10, 20);
    if err == nil { print(result); }
}
"#,
    );
}

// ===========================================================================
// File-based Test Runner (Runtime Verification)
// ===========================================================================

#[cfg(test)]
mod file_runner {
    use crate::common::compile_snippet;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    #[test]
    #[ignore = "runs .doo files through compiler - some cause hangs"]
    fn run_all_compile_pass_files() {
        let tests_dir = PathBuf::from("tests/compile_pass");
        let mut failed_tests = Vec::new();

        if CodeTester::visit_dirs(&tests_dir, &mut failed_tests).is_err() {
            panic!("Failed to read tests directory");
        }

        if !failed_tests.is_empty() {
            panic!(
                "Some compile-pass file tests failed:\n{}",
                failed_tests.join("\n")
            );
        }
    }

    struct CodeTester;

    impl CodeTester {
        // Recursive directory walker without external crate
        fn visit_dirs(dir: &Path, failed_tests: &mut Vec<String>) -> std::io::Result<()> {
            if dir.is_dir() {
                for entry in fs::read_dir(dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.is_dir() {
                        Self::visit_dirs(&path, failed_tests)?;
                    } else if path.extension().map_or(false, |ext| ext == "doo") {
                        println!("Running test: {:?}", path);
                        if let Err(e) = Self::run_test_file(&path) {
                            failed_tests.push(format!("{:?}: {}", path, e));
                        }
                    }
                }
            }
            Ok(())
        }

        fn run_test_file(path: &Path) -> Result<(), String> {
            let content = fs::read_to_string(path).map_err(|e| e.to_string())?;

            // 1. Parse expected output
            let mut expected_output = String::new();
            let mut has_output_comment = false;

            for line in content.lines() {
                if let Some(comment) = line.trim().strip_prefix("// OUTPUT: ") {
                    expected_output.push_str(comment);
                    expected_output.push('\n');
                    has_output_comment = true;
                } else if let Some(comment) = line.trim().strip_prefix("// OUTPUT:") {
                    expected_output.push_str(comment);
                    expected_output.push('\n');
                    has_output_comment = true;
                }
            }

            // If file has no OUTPUT comment, we skip runtime verification and just check compilation
            // This allows us to incrementally add comments later without breaking everything
            if !has_output_comment {
                // Just compile to ensure it's valid code
                compile_snippet(&content).map_err(|e| format!("Compilation failed: {}", e))?;
                return Ok(());
            }

            // 2. Compile to IR
            let ir = compile_snippet(&content).map_err(|e| format!("Compilation failed: {}", e))?;

            // 3. Write IR to temp file
            let tmp_dir = PathBuf::from("target/tmp_tests");
            fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;

            let file_stem = path.file_stem().unwrap().to_str().unwrap();
            let ir_path = tmp_dir.join(format!("{}.ll", file_stem));
            fs::write(&ir_path, ir).map_err(|e| e.to_string())?;

            // 4. Run lli
            let output = Command::new("lli")
                .arg(&ir_path)
                .output()
                .map_err(|e| format!("Failed to execute lli: {}", e))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!(
                    "Runtime error (lli exit code {:?}):\n{}",
                    output.status.code(),
                    stderr
                ));
            }

            let actual_stdout = String::from_utf8_lossy(&output.stdout);

            // 5. Verify Output
            let expected_clean = expected_output.replace("\r\n", "\n").trim().to_string();
            let actual_clean = actual_stdout.replace("\r\n", "\n").trim().to_string();

            if actual_clean != expected_clean {
                return Err(format!(
                    "Output mismatch.\nExpected:\n---\n{}\n---\nActual:\n---\n{}\n---",
                    expected_clean, actual_clean
                ));
            }

            Ok(())
        }
    }
}

// ===========================================================================
// Auto-discovered .doo compile-pass files
// ===========================================================================
// Async / Go / Scope
// ===========================================================================

#[test]
fn async_fn_and_await() {
    assert_compiles(
        r#"
async fn fetchData() -> Str {
    sleep(10);
    return "data";
}
fn main() {
    let result = await fetchData();
    print(result);
}
"#,
    );
}

#[test]
fn async_go_block_fire_and_forget() {
    assert_compiles(
        r#"
fn main() {
    go {
        print("detached");
    }
    sleep(100);
}
"#,
    );
}

#[test]
fn async_go_with_handle() {
    assert_compiles(
        r#"
fn main() {
    let task = go {
        sleep(30);
        print("done");
    };
    sleep(100);
}
"#,
    );
}

#[test]
fn async_scope_structured() {
    assert_compiles(
        r#"
fn main() {
    scope {
        go {
            sleep(40);
            print("slow");
        }
        go {
            sleep(10);
            print("fast");
        }
    }
    print("all done");
}
"#,
    );
}

#[test]
fn async_await_sleep() {
    assert_compiles(
        r#"
fn main() {
    await sleep(50);
    print("done");
}
"#,
    );
}

#[test]
fn async_nested_go() {
    assert_compiles(
        r#"
fn main() {
    go {
        go {
            sleep(10);
            print("inner");
        }
        sleep(40);
        print("outer");
    }
    sleep(200);
}
"#,
    );
}

#[test]
fn async_for_loop_await() {
    assert_compiles(
        r#"
fn main() {
    for i in 0..3 {
        await sleep(20);
        print("poll ${i}");
    }
}
"#,
    );
}

#[test]
fn async_scope_with_async_fn() {
    assert_compiles(
        r#"
async fn query(name: Str) -> Str {
    sleep(20);
    return "result: ${name}";
}
fn main() {
    scope {
        go {
            let r = await query("users");
            print(r);
        }
        go {
            let r = await query("posts");
            print(r);
        }
    }
}
"#,
    );
}

// ===========================================================================
// Process
// ===========================================================================

#[test]
fn process_run_basic() {
    assert_compiles(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    let result = Process::run("echo", "[\"hello\"]")?;
    print(result);
}
"#,
    );
}

#[test]
fn process_output_basic() {
    assert_compiles(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    let out = Process::output("echo", "[\"test\"]")?;
    print(out);
}
"#,
    );
}

#[test]
fn process_spawn_kill() {
    assert_compiles(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    let handle = Process::spawn("ping", "[\"127.0.0.1\"]")?;
    let running = Process::isRunning(handle);
    Process::kill(handle)?;
}
"#,
    );
}

#[test]
fn process_active_count_and_shutdown() {
    assert_compiles(
        r#"
import std::Process::{Process, ProcessError};
fn main() {
    let h = Process::spawn("ping", "[\"127.0.0.1\"]")?;
    let count = Process::activeCount();
    Process::shutdown();
}
"#,
    );
}

// ===========================================================================
// WebSocket
// ===========================================================================

#[test]
fn ws_echo_handler() {
    assert_compiles(
        r#"
import std::Http::{Server, WsConnection};
fn onEcho(conn: WsConnection, data: Str) {
    conn.emit("echo", data);
}
fn handler(conn: WsConnection) {
    conn.on("echo", onEcho);
}
fn main() {
    let app = Server::new(":3000");
    app.ws("/ws/echo", handler);
    app.start();
}
"#,
    );
}

#[test]
fn ws_room_management() {
    assert_compiles(
        r#"
import std::Http::{Server, WsConnection};
fn onMsg(conn: WsConnection, data: Str, app: Server) {
    app.toRoomEmit("lobby", "message", data);
}
fn handler(conn: WsConnection) {
    conn.join("lobby");
    conn.on("message", onMsg);
}
fn main() {
    let app = Server::new(":3000");
    app.ws("/ws/chat", handler);
    app.start();
}
"#,
    );
}

#[test]
fn ws_lifecycle_hooks() {
    assert_compiles(
        r#"
import std::Http::{Server, WsConnection};
fn onConnect(conn: WsConnection) { print("connected"); }
fn onDisconnect(conn: WsConnection) { print("disconnected"); }
fn onError(conn: WsConnection, err: Str) { print("error"); }
fn handler(conn: WsConnection) {
    conn.onConnect(onConnect);
    conn.onDisconnect(onDisconnect);
    conn.onError(onError);
}
fn main() {
    let app = Server::new(":3000");
    app.ws("/ws/lifecycle", handler);
    app.start();
}
"#,
    );
}

#[test]
fn ws_close_and_is_closed() {
    assert_compiles(
        r#"
import std::Http::{Server, WsConnection};
fn onCheck(conn: WsConnection, data: Str) {
    let closed = conn.isClosed();
    if closed {
        conn.emit("status", "closed");
    } else {
        conn.emit("status", "open");
    }
}
fn onClose(conn: WsConnection, data: Str) {
    conn.close();
}
fn handler(conn: WsConnection) {
    conn.on("check", onCheck);
    conn.on("close", onClose);
}
fn main() {
    let app = Server::new(":3000");
    app.ws("/ws/close", handler);
    app.start();
}
"#,
    );
}

#[test]
fn ws_broadcast_and_active_connections() {
    assert_compiles(
        r#"
import std::Http::{Server, WsConnection};
fn handler(conn: WsConnection) {
    conn.on("msg", (conn: WsConnection, data: Str) => { conn.emit("msg", data); });
}
fn getStatus(app: Server) -> Str {
    let count = app.activeWsConnections();
    return "${count}";
}
fn doBroadcast(app: Server) -> Str {
    app.broadcast("event", "hello");
    return "sent";
}
fn main() {
    let app = Server::new(":3000");
    app.ws("/ws", handler);
    app.get("/status", getStatus);
    app.get("/broadcast", doBroadcast);
    app.start();
}
"#,
    );
}

// ===========================================================================
// Auto-discovered .doo compile-pass files
// ===========================================================================

#[test]
fn compile_pass_doo_files() {
    let dir = std::path::Path::new("tests/compile_pass");
    assert_doo_file_suite(dir, DooTestMode::CompilePass, "compile_pass");
}
