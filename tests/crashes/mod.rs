//! Crash tests — verify compiler doesn't panic/crash on malformed input
//! Modeled after rustc's crash test suite and Go's test/fixedbugs
//! Every test: compile garbage → must NOT panic (errors are fine)
//! Auto-discovers .doo files in tests/crashes/ and runs them as crash tests

use crate::common::{compile_snippet, assert_doo_file_suite, DooTestMode};
use std::fs;

fn must_not_crash(code: &str) {
    let _ = compile_snippet(code); // Result doesn't matter, just no panic
}

// ===========================================================================
// Incomplete / Truncated Syntax
// ===========================================================================

#[test]
fn crash_incomplete_let() {
    must_not_crash("fn main() { let x =");
}

#[test]
fn crash_incomplete_fn() {
    must_not_crash("fn foo(");
}

#[test]
fn crash_incomplete_struct() {
    must_not_crash("struct Foo {");
}

#[test]
fn crash_incomplete_enum() {
    must_not_crash("enum Bar {");
}

#[test]
fn crash_incomplete_if() {
    must_not_crash("fn main() { if true {");
}

#[test]
fn crash_incomplete_for() {
    must_not_crash("fn main() { for i in");
}

#[test]
fn crash_incomplete_match() {
    must_not_crash("fn main() { match x {");
}

#[test]
fn crash_incomplete_return() {
    must_not_crash("fn main() -> Int { return");
}

#[test]
fn crash_incomplete_string() {
    must_not_crash(r#"fn main() { let s = "unterminated"#);
}

#[test]
fn crash_incomplete_interpolation() {
    must_not_crash(r#"fn main() { let s = "hello ${"; }"#);
}

#[test]
fn crash_incomplete_array() {
    must_not_crash("fn main() { let a = [1, 2,");
}

#[test]
fn crash_incomplete_map() {
    must_not_crash(r#"fn main() { let m = {"a": 1,"#);
}

#[test]
fn crash_incomplete_closure() {
    must_not_crash("fn main() { let f = (x) =>");
}

// ===========================================================================
// Empty / Minimal Input
// ===========================================================================

#[test]
fn crash_empty_input() {
    must_not_crash("");
}

#[test]
fn crash_whitespace_only() {
    must_not_crash("   \n\n\t  \n  ");
}

#[test]
fn crash_single_keyword() {
    must_not_crash("fn");
}

#[test]
fn crash_single_brace() {
    must_not_crash("{");
}

#[test]
fn crash_single_paren() {
    must_not_crash("(");
}

#[test]
fn crash_single_bracket() {
    must_not_crash("[");
}

#[test]
fn crash_just_main() {
    must_not_crash("fn main()");
}

#[test]
fn crash_empty_main() {
    must_not_crash("fn main() { }");
}

// ===========================================================================
// Garbage / Random Tokens
// ===========================================================================

#[test]
fn crash_random_operators() {
    must_not_crash("+ - * / % == != < > <= >= && || !");
}

#[test]
fn crash_stacked_keywords() {
    must_not_crash("fn fn fn struct enum let let mut return break continue");
}

#[test]
fn crash_mismatched_braces() {
    must_not_crash("fn main() { { { } }}}");
}

#[test]
fn crash_mismatched_parens() {
    must_not_crash("fn main() { ((()) }");
}

#[test]
fn crash_mismatched_brackets() {
    must_not_crash("fn main() { [[[]] }");
}

#[test]
fn crash_semicolons_only() {
    must_not_crash(";;;;;;;;");
}

#[test]
fn crash_dots_only() {
    must_not_crash(".....");
}

#[test]
fn crash_arrows_only() {
    must_not_crash("=> => => ->");
}

// ===========================================================================
// Unicode Edge Cases
// ===========================================================================

#[test]
fn crash_unicode_identifiers() {
    must_not_crash("fn main() { let \u{00E9} = 42; }");
}

#[test]
fn crash_unicode_in_string() {
    must_not_crash(r#"fn main() { let s = "Hello 世界 🌍"; print(s); }"#);
}

#[test]
fn crash_null_byte_in_source() {
    must_not_crash("fn main() { let x\0 = 42; }");
}

#[test]
fn crash_bom_prefix() {
    must_not_crash("\u{FEFF}fn main() { }");
}

#[test]
fn crash_mixed_line_endings() {
    must_not_crash("fn main() {\r\n    let x = 1;\r    let y = 2;\n}");
}

// ===========================================================================
// Deeply Nested (moderate — won't stack overflow)
// ===========================================================================

#[test]
fn crash_nested_ifs_moderate() {
    let mut code = String::from("fn main() {\n");
    for i in 0..50 {
        code.push_str(&format!("{}if true {{\n", "    ".repeat(i + 1)));
    }
    code.push_str(&format!("{}print(\"deep\");\n", "    ".repeat(51)));
    for i in (0..50).rev() {
        code.push_str(&format!("{}}}\n", "    ".repeat(i + 1)));
    }
    code.push_str("}\n");
    must_not_crash(&code);
}

#[test]
fn crash_nested_arrays_moderate() {
    // [[[[[[1]]]]]]
    let mut code = String::from("fn main() { let x = ");
    for _ in 0..20 {
        code.push('[');
    }
    code.push('1');
    for _ in 0..20 {
        code.push(']');
    }
    code.push_str("; }");
    must_not_crash(&code);
}

#[test]
fn crash_deeply_nested_expressions() {
    // Parser's MAX_EXPR_DEPTH (128) catches deep nesting before stack overflow
    let mut code = String::from("fn main() { let x = ");
    for _ in 0..1000 {
        code.push_str("(1 + ");
    }
    code.push('1');
    for _ in 0..1000 {
        code.push(')');
    }
    code.push_str("; }");
    must_not_crash(&code);
}

// ===========================================================================
// Semantic Nonsense (valid tokens, invalid semantics)
// ===========================================================================

#[test]
fn crash_return_outside_function() {
    must_not_crash("return 42;");
}

#[test]
fn crash_break_outside_loop() {
    must_not_crash("fn main() { break; }");
}

#[test]
fn crash_continue_outside_loop() {
    must_not_crash("fn main() { continue; }");
}

#[test]
fn crash_double_return() {
    must_not_crash("fn main() { return; return; }");
}

#[test]
fn crash_method_on_literal() {
    must_not_crash(r#"fn main() { 42.toString(); }"#);
}

#[test]
fn crash_call_non_function() {
    must_not_crash("fn main() { let x = 42; x(); }");
}

#[test]
fn crash_index_non_array() {
    must_not_crash("fn main() { let x = 42; x[0]; }");
}

#[test]
fn crash_field_on_int() {
    must_not_crash("fn main() { let x = 42; x.field; }");
}

#[test]
fn crash_assign_to_literal() {
    must_not_crash("fn main() { 42 = 10; }");
}

#[test]
fn crash_struct_as_value() {
    must_not_crash("struct Foo { x: Int } fn main() { print(Foo); }");
}

#[test]
fn crash_enum_as_value() {
    must_not_crash("enum Color { Red } fn main() { print(Color); }");
}

// ===========================================================================
// Large Input
// ===========================================================================

#[test]
fn crash_many_variables() {
    let mut code = String::from("fn main() {\n");
    for i in 0..200 {
        code.push_str(&format!("    let var_{} = {};\n", i, i));
    }
    code.push_str("}\n");
    must_not_crash(&code);
}

#[test]
fn crash_many_functions() {
    let mut code = String::new();
    for i in 0..100 {
        code.push_str(&format!("fn func_{}() {{ print({}); }}\n", i, i));
    }
    code.push_str("fn main() { }\n");
    must_not_crash(&code);
}

#[test]
fn crash_very_long_string() {
    let long = "a".repeat(10_000);
    let code = format!(r#"fn main() {{ let s = "{}"; print(s); }}"#, long);
    must_not_crash(&code);
}

#[test]
fn crash_very_long_identifier() {
    let long_id = "x".repeat(1_000);
    let code = format!("fn main() {{ let {} = 42; }}", long_id);
    must_not_crash(&code);
}

// ===========================================================================
// Edge Cases in Expressions
// ===========================================================================

#[test]
fn crash_empty_block() {
    must_not_crash("fn main() { {} }");
}

#[test]
fn crash_trailing_comma_array() {
    must_not_crash("fn main() { let arr = [1, 2, 3,]; }");
}

#[test]
fn crash_trailing_comma_struct() {
    must_not_crash("struct P { x: Int, y: Int, } fn main() { }");
}

#[test]
fn crash_double_semicolons() {
    must_not_crash("fn main() { let x = 1;; let y = 2;; }");
}

#[test]
fn crash_negative_literal() {
    must_not_crash("fn main() { let x = -42; }");
}

#[test]
fn crash_double_negative() {
    must_not_crash("fn main() { let x = --42; }");
}

#[test]
fn crash_empty_struct() {
    must_not_crash("struct Empty { } fn main() { }");
}

#[test]
fn crash_empty_enum() {
    must_not_crash("enum Empty { } fn main() { }");
}

#[test]
fn crash_single_variant_enum() {
    must_not_crash("enum Single { Only } fn main() { let x = Single::Only; }");
}

// ===========================================================================
// Async / Go / Scope Edge Cases
// ===========================================================================

#[test]
fn crash_async_fn_no_body() {
    must_not_crash("async fn broken()");
}

#[test]
fn crash_go_no_block() {
    must_not_crash("fn main() { go }");
}

#[test]
fn crash_scope_no_block() {
    must_not_crash("fn main() { scope }");
}

#[test]
fn crash_await_nothing() {
    must_not_crash("fn main() { await; }");
}

#[test]
fn crash_nested_scope_go() {
    must_not_crash("fn main() { scope { go { scope { go { } } } } }");
}

#[test]
fn crash_go_in_go() {
    must_not_crash("fn main() { go { go { go { } } } }");
}

// ===========================================================================
// Process Edge Cases
// ===========================================================================

#[test]
fn crash_process_incomplete_call() {
    must_not_crash(r#"import std::Process::{Process}; fn main() { Process::run("echo", }"#);
}

// ===========================================================================
// WebSocket Edge Cases
// ===========================================================================

#[test]
fn crash_ws_incomplete_handler() {
    must_not_crash(r#"import std::Http::{WsConnection}; fn handler(conn: WsConnection) { conn.on("#);
}

// ===========================================================================
// Auto-discovered .doo crash test files
// ===========================================================================

#[test]
fn crash_doo_files() {
    let dir = std::path::Path::new("tests/crashes");
    assert_doo_file_suite(dir, DooTestMode::CrashTest, "crashes");
}
