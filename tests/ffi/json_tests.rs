//! JSON FFI Tests — Production-grade coverage
//! Full compiler pipeline: lex → parse → analyze → MIR → codegen
//! Syntax matches dev_test/json/main.doo: JSON.stringify, JSON.parse (built-in)

use super::{assert_ffi_compiles, assert_ffi_compiles_with};

// =============================================================================
// 1. BASIC STRINGIFY / PARSE
// =============================================================================

#[test]
fn json_stringify_struct() {
    assert_ffi_compiles_with(
        r#"
struct User { name: Str, age: Int }
fn main() {
    let u = User { name: "Alice", age: 30 };
    let json = JSON.stringify(u);
    print(json);
}
"#,
        "Alice",
    );
}

#[test]
fn json_parse_struct() {
    assert_ffi_compiles_with(
        r#"
struct User { name: Str, age: Int }
fn main() {
    let json = "{\"name\": \"Alice\", \"age\": 30}";
    let user: User = JSON.parse(json);
    print(user.name);
}
"#,
        "parse",
    );
}

#[test]
fn json_primitive_int() {
    assert_ffi_compiles_with(
        r#"
fn main() {
    let num = 42;
    let json = JSON.stringify(num);
    let parsed: Int = JSON.parse(json);
    print(parsed);
}
"#,
        "call",
    );
}

#[test]
fn json_primitive_float() {
    assert_ffi_compiles(
        r#"
fn main() {
    let pi = 3.14;
    print(JSON.stringify(pi));
}
"#,
    );
}

#[test]
fn json_primitive_bool() {
    assert_ffi_compiles(
        r#"
fn main() {
    let flag = true;
    print(JSON.stringify(flag));
    let parsed: Bool = JSON.parse(JSON.stringify(flag));
    print(parsed);
}
"#,
    );
}

#[test]
fn json_primitive_str() {
    assert_ffi_compiles_with(
        r#"
fn main() {
    let text = "Hello JSON";
    print(JSON.stringify(text));
    let parsed: Str = JSON.parse(JSON.stringify(text));
    print(parsed);
}
"#,
        "Hello JSON",
    );
}

// =============================================================================
// 2. NESTED STRUCTS
// =============================================================================

#[test]
fn json_nested_struct_stringify() {
    assert_ffi_compiles_with(
        r#"
struct Address { city: Str, zip: Str }
struct User { name: Str, address: Address }
fn main() {
    let u = User { name: "Alice", address: Address { city: "NYC", zip: "10001" } };
    let json = JSON.stringify(u);
    print(json);
}
"#,
        "NYC",
    );
}

#[test]
fn json_nested_struct_roundtrip() {
    assert_ffi_compiles_with(
        r#"
struct Address { city: Str, zip: Str }
struct User { name: Str, address: Address }
fn main() {
    let u = User { name: "Alice", address: Address { city: "NYC", zip: "10001" } };
    let json = JSON.stringify(u);
    let u2: User = JSON.parse(json);
    print(u2.address.city);
}
"#,
        "User",
    );
}

// =============================================================================
// 3. ARRAYS AND COLLECTIONS
// =============================================================================

#[test]
fn json_stringify_array() {
    assert_ffi_compiles_with(
        r#"
struct Item { name: Str, price: Float }
fn main() {
    let items = [
        Item { name: "Apple", price: 1.50 },
        Item { name: "Banana", price: 0.75 },
    ];
    let json = JSON.stringify(items);
    print(json);
}
"#,
        "Banana",
    );
}

#[test]
fn json_parse_array() {
    assert_ffi_compiles(
        r#"
fn main() {
    let arr = [1, 2, 3, 4, 5];
    let json = JSON.stringify(arr);
    let parsed: [Int] = JSON.parse(json);
    print(parsed);
}
"#,
    );
}

#[test]
fn json_stringify_map() {
    assert_ffi_compiles_with(
        r#"
fn main() {
    let config = {"host": "localhost", "port": "8080"};
    let json = JSON.stringify(config);
    print(json);
}
"#,
        "localhost",
    );
}

// =============================================================================
// 4. OPTIONAL / NULLABLE FIELDS
// =============================================================================

#[test]
fn json_optional_field_stringify() {
    assert_ffi_compiles_with(
        r#"
struct User { name: Str, bio: Str? }
fn main() {
    let u1 = User { name: "Alice", bio: "Hi there" };
    let u2 = User { name: "Bob", bio: nil };
    print(JSON.stringify(u1));
    print(JSON.stringify(u2));
}
"#,
        "Hi there",
    );
}

// =============================================================================
// 5. ENUM SERIALIZATION
// =============================================================================

#[test]
fn json_enum_stringify() {
    assert_ffi_compiles_with(
        r#"
enum Status { Active, Inactive, Pending }
struct User { name: Str, status: Status }
fn main() {
    let u = User { name: "Alice", status: Status::Active };
    let json = JSON.stringify(u);
    print(json);
}
"#,
        "Alice",
    );
}

#[test]
fn json_enum_roundtrip() {
    assert_ffi_compiles(
        r#"
enum Status { Active, Inactive, Pending }
fn main() {
    let s: Status = JSON.parse(JSON.stringify(Status::Active));
    print(JSON.stringify(s));
}
"#,
    );
}

#[test]
fn json_enum_payload() {
    assert_ffi_compiles_with(
        r#"
enum ApiResult {
    Success(Int),
    Error(Str),
}
fn main() {
    print(JSON.stringify(ApiResult::Success(200)));
    print(JSON.stringify(ApiResult::Error("not found")));
    let r: ApiResult = JSON.parse(JSON.stringify(ApiResult::Success(42)));
    print(JSON.stringify(r));
}
"#,
        "not found",
    );
}

// =============================================================================
// 6. COMPLEX JSON SCENARIOS
// =============================================================================

#[test]
fn json_roundtrip() {
    assert_ffi_compiles_with(
        r#"
struct Config { host: Str, port: Int, debug: Bool }
fn main() {
    let original = Config { host: "localhost", port: 8080, debug: true };
    let json = JSON.stringify(original);
    let restored: Config = JSON.parse(json);
    print(restored.host);
    print(restored.port);
    print(restored.debug);
}
"#,
        "Config",
    );
}

#[test]
fn json_struct_with_collections() {
    assert_ffi_compiles_with(
        r#"
struct Config {
    tags: [Str],
    limits: [Int],
    options: {Str: Bool},
}
fn main() {
    let cfg = Config {
        tags: ["api", "prod"],
        limits: [100, 200],
        options: {"verbose": true, "debug": false},
    };
    let json = JSON.stringify(cfg);
    let cfg2: Config = JSON.parse(json);
    print(cfg2.tags);
    print(cfg2.limits);
}
"#,
        "Config",
    );
}

#[test]
fn json_deeply_nested() {
    assert_ffi_compiles_with(
        r#"
struct Coord { lat: Float, lng: Float }
struct Location { name: Str, coord: Coord }
struct Branch { id: Int, location: Location }
struct Company { name: Str, branches: [Branch] }
fn main() {
    let c = Company {
        name: "Corp",
        branches: [
            Branch { id: 1, location: Location { name: "HQ", coord: Coord { lat: 40.7, lng: -74.0 } } },
            Branch { id: 2, location: Location { name: "West", coord: Coord { lat: 34.0, lng: -118.2 } } },
        ],
    };
    let json = JSON.stringify(c);
    print(json);
}
"#,
        "Branch",
    );
}

#[test]
fn json_parse_and_process_with_file() {
    assert_ffi_compiles_with(
        r#"
import std::File;
struct Config { host: Str, port: Int, debug: Bool }
fn loadConfig(path: Str) -> Config ! Str {
    let content = File::Read(path)?;
    let cfg: Config = JSON.parse(content);
    Ok cfg;
}
fn main() {
    let cfg, err = loadConfig("config.json");
    if err != nil {
        print("Using defaults");
    } else {
        print("Server: ${cfg.host}:${cfg.port}");
    }
}
"#,
        "config.json",
    );
}

#[test]
fn json_variable_roundtrip() {
    assert_ffi_compiles_with(
        r#"
struct Point { x: Int, y: Int }
fn main() {
    let iVal: Int = JSON.parse(JSON.stringify(12345));
    print(iVal);
    let fVal: Float = JSON.parse(JSON.stringify(1.23));
    print(fVal);
    let bVal: Bool = JSON.parse(JSON.stringify(true));
    print(bVal);
    let sVal: Str = JSON.parse(JSON.stringify("roundtrip"));
    print(sVal);
    let ptVal: Point = JSON.parse(JSON.stringify(Point { x: 7, y: 8 }));
    print(ptVal.x, ptVal.y);
}
"#,
        "roundtrip",
    );
}
