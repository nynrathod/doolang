//! Import Integration Tests
//! Tests the module import system with stdlib and custom modules

use super::super::common::{assert_compiles, assert_fails};

// ========================================
// BASIC SINGLE IMPORTS
// ========================================

#[test]
fn test_simple_import() {
    let code = r#"
        import std::Math::Abs;
        fn main() {
            let val = Abs(-42);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_import_multiple_functions() {
    let code = r#"
        import std::Math::Abs;
        import std::Math::Max;
        import std::Math::Min;
        fn main() {
            let a = Abs(-10);
            let b = Max(5, 10);
            let c = Min(5, 10);
        }
    "#;
    assert_compiles(code);
}

// ========================================
// BRACES SYNTAX (MULTI-IMPORT)
// ========================================

#[test]
fn test_import_with_braces() {
    let code = r#"
        import std::Math::{Abs, Max, Min};
        fn main() {
            let a = Abs(-10);
            let b = Max(5, 10);
            let c = Min(5, 10);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_import_with_mixed_items_in_braces() {
    let code = r#"
        import std::Math::{Abs, Max, Min, Pow, Sqrt};
        fn main() {
            let a = Abs(-10);
            let b = Max(5, 10);
            let c = Min(5, 10);
            let d = Pow(2, 3);
            let e = Sqrt(16);
        }
    "#;
    assert_compiles(code);
}

// ========================================
// WILDCARD IMPORTS
// ========================================

#[test]
fn test_import_wildcard() {
    let code = r#"
        import std::Math::*;
        fn main() {
            let a = Abs(-10);
            let b = Max(5, 10);
            let c = Min(5, 10);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_wildcard_imports_multiple_functions() {
    let code = r#"
        import std::Math::*;
        fn main() {
            let a = Abs(-10);
            let b = Max(5, 10);
            let c = Pow(2, 8);
        }
    "#;
    assert_compiles(code);
}

// ========================================
// MATH MODULE FUNCTIONS
// ========================================

#[test]
fn test_import_math_module() {
    let code = r#"
        import std::Math::{Abs, Max, Min, Pow, Sqrt};
        fn main() {
            let a = Abs(-10);
            let b = Max(5, 10);
            let c = Min(5, 10);
            let d = Pow(2, 3);
            let e = Sqrt(16);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_import_abs() {
    let code = r#"
        import std::Math::Abs;
        fn main() {
            let a = Abs(-42);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_import_max_min() {
    let code = r#"
        import std::Math::Max;
        import std::Math::Min;
        fn main() {
            let a = Max(10, 20);
            let b = Min(10, 20);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_import_pow() {
    let code = r#"
        import std::Math::Pow;
        fn main() {
            let a = Pow(2, 8);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_import_sqrt() {
    let code = r#"
        import std::Math::Sqrt;
        fn main() {
            let a = Sqrt(16);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_import_clamp() {
    let code = r#"
        import std::Math::Clamp;
        fn main() {
            let a = Clamp(15, 10, 20);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_import_floor() {
    let code = r#"
        import std::Math::Floor;
        fn main() {
            let a = Floor(3.7);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_import_ceil() {
    let code = r#"
        import std::Math::Ceil;
        fn main() {
            let a = Ceil(3.2);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_import_round() {
    let code = r#"
        import std::Math::Round;
        fn main() {
            let a = Round(3.5);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_import_is_even() {
    let code = r#"
        import std::Math::IsEven;
        fn main() {
            let a = IsEven(4);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_import_is_odd() {
    let code = r#"
        import std::Math::IsOdd;
        fn main() {
            let a = IsOdd(5);
        }
    "#;
    assert_compiles(code);
}

// ========================================
// IMPORT ORDERING
// ========================================

#[test]
fn test_imports_before_functions() {
    let code = r#"
        import std::Math::Abs;

        fn helper() -> Int {
            return Abs(-10);
        }

        fn main() {
            let val = helper();
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_multiple_import_statements() {
    let code = r#"
        import std::Math::Abs;
        import std::Math::Max;

        fn main() {
            let a = Abs(-10);
            let b = Max(5, 10);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_imports_from_multiple_modules() {
    let code = r#"
        import std::Math::Abs;
        import std::Math::Max;
        import std::Math::Min;
        fn main() {
            let a = Abs(-10);
            let b = Max(5, 10);
            let c = Min(5, 10);
        }
    "#;
    assert_compiles(code);
}

// ========================================
// ERROR CASES - UNDEFINED SYMBOLS
// ========================================

#[test]
fn test_use_unimported_function() {
    let code = r#"
        fn main() {
            let val = Abs(-10);
        }
    "#;
    assert_fails(code);
}

#[test]
fn test_import_undefined_symbol() {
    let code = r#"
        import std::Math::NonExistentFunction;
        fn main() { }
    "#;
    assert_fails(code);
}

#[test]
fn test_import_undefined_module() {
    let code = r#"
        import undefined::module::Function;
        fn main() { }
    "#;
    assert_fails(code);
}

// ========================================
// IMPORT WITH USER-DEFINED FUNCTIONS
// ========================================

#[test]
fn test_import_with_user_functions() {
    let code = r#"
        import std::Math::Abs;

        fn myabs(x: Int) -> Int {
            return Abs(x);
        }

        fn main() {
            let val = myabs(-42);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_import_in_helper_function() {
    let code = r#"
        import std::Math::Max;

        fn getmax(a: Int, b: Int) -> Int {
            return Max(a, b);
        }

        fn main() {
            let result = getmax(10, 20);
        }
    "#;
    assert_compiles(code);
}

// ========================================
// NESTED FUNCTION CALLS WITH IMPORTS
// ========================================

#[test]
fn test_nested_calls_with_import() {
    let code = r#"
        import std::Math::Abs;

        fn processabs(x: Int) -> Int {
            return Abs(x);
        }

        fn main() {
            let val = processabs(-42);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_multiple_imported_functions_in_call() {
    let code = r#"
        import std::Math::{Max, Min};

        fn main() {
            let a = 5;
            let b = 10;
            let maxval = Max(a, b);
            let minval = Min(a, b);
        }
    "#;
    assert_compiles(code);
}

// ========================================
// IMPORTS WITH LOOPS AND CONDITIONALS
// ========================================

#[test]
fn test_import_in_loop() {
    let code = r#"
        import std::Math::Abs;

        fn main() {
            for i in 0..5 {
                let val = Abs(0 - i);
            }
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_import_in_conditional() {
    let code = r#"
        import std::Math::Max;

        fn main() {
            let a = 5;
            let b = 10;
            if Max(a, b) > 7 {
                print("yes");
            }
        }
    "#;
    assert_compiles(code);
}

// ========================================
// FORWARD DECLARATIONS WITH IMPORTS
// ========================================

#[test]
fn test_function_call_ordering_with_import() {
    let code = r#"
        import std::Math::Abs;

        fn main() {
            let val = helper();
        }

        fn helper() -> Int {
            return Abs(-42);
        }
    "#;
    assert_compiles(code);
}

// ========================================
// COMPLEX IMPORT PATTERNS
// ========================================

#[test]
fn test_multiple_function_definitions_with_imports() {
    let code = r#"
        import std::Math::Abs;
        import std::Math::Max;

        fn add(a: Int, b: Int) -> Int {
            return a + b;
        }

        fn multiply(a: Int, b: Int) -> Int {
            return a * b;
        }

        fn main() {
            let sum = add(5, 3);
            let product = multiply(4, 2);
            let a = Abs(-10);
            let b = Max(5, 10);
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_imports_with_all_math_functions() {
    let code = r#"
        import std::Math::{Abs, Pow, Sqrt};

        fn main() {
            let intval = Abs(-42);
            let floatval = Sqrt(16);
            let result = Pow(2, 5);
        }
    "#;
    assert_compiles(code);
}

// ========================================
// BUILTIN FUNCTIONS WITHOUT IMPORTS
// ========================================

#[test]
fn test_builtin_print() {
    let code = r#"
        fn main() {
            print(42);
            print("hello");
        }
    "#;
    assert_compiles(code);
}

#[test]
fn test_builtin_print_multiple_args() {
    let code = r#"
        fn main() {
            print("x", 5, "y", 10);
        }
    "#;
    assert_compiles(code);
}

// ========================================
// RELATIVE IMPORTS - NOT SUPPORTED
// ========================================

#[test]
fn test_relative_imports_not_supported() {
    let code = r#"
        import ./module::Function;
        fn main() { }
    "#;
    assert_fails(code);
}

#[test]
fn test_parent_relative_imports_not_supported() {
    let code = r#"
        import ../parent::Function;
        fn main() { }
    "#;
    assert_fails(code);
}

// ========================================
// QUALIFIED NAMES - NOT SUPPORTED
// ========================================

#[test]
fn test_qualified_names_not_supported() {
    let code = r#"
        fn main() {
            let val = Math::Abs(-10);
        }
    "#;
    assert_fails(code);
}
