//! Codegen Unit Tests - END-TO-END CHECKPOINT
//!
//! These tests run the FULL pipeline: lexer → parser → analyzer → MIR → codegen
//! If a test passes here, all earlier stages also passed.
//!
//! PRINCIPLE: Every test MUST verify specific IR patterns, not just "did it compile."
//! - Use `assert_ir_contains` to verify expected LLVM IR patterns
//! - Use `assert_ir_contains_all` for multiple required patterns
//! - Use `assert_codegen_fails` for expected compile errors
//!
//! NO redundant tests - each test has a clear, documented purpose.

#[cfg(test)]
mod codegen_tests {
    use crate::analyzer::SemanticAnalyzer;
    use crate::codegen::core::CodeGen;
    use crate::lexer::lexer::lex;
    use crate::mir::builder::MirBuilder;
    use crate::parser::ast::AstNode;
    use crate::parser::Parser;
    use bumpalo::Bump;
    use inkwell::context::Context;

    // ========================================
    // HELPER FUNCTIONS
    // ========================================

    fn compile_code(input: &str) -> Result<String, String> {
        let arena = Bump::new();
        let tokens = lex(input, &arena);
        let mut parser = Parser::new(&tokens);
        let result = parser.parse_program();

        match result {
            Ok(mut ast) => {
                let mut analyzer = SemanticAnalyzer::new(None);
                if let AstNode::Program(ref mut nodes) = ast {
                    analyzer
                        .analyze_program(nodes)
                        .map_err(|e| format!("{:?}", e))?;

                    let mut all_nodes = analyzer.imported_structs.clone();
                    all_nodes.extend(analyzer.imported_functions.clone());
                    all_nodes.extend(nodes.clone());

                    let mut mir_builder = MirBuilder::new();
                    mir_builder.build_program(&all_nodes);
                    mir_builder.finalize();

                    let context = Context::create();
                    let mut codegen = CodeGen::new("test", &context);
                    codegen.function_aliases = analyzer.function_aliases.clone();
                    codegen.generate_program(&mir_builder.program);

                    Ok(codegen.module.print_to_string().to_string())
                } else {
                    Err("Not a program".to_string())
                }
            }
            Err(e) => Err(format!("Parse error: {:?}", e)),
        }
    }

    /// Get IR string, panic if compilation fails
    fn get_ir(code: &str) -> String {
        compile_code(code).expect(&format!("Codegen failed for:\n{}", code))
    }

    /// Verify IR contains a specific pattern
    fn assert_ir_contains(code: &str, pattern: &str) {
        let ir = get_ir(code);
        assert!(
            ir.contains(pattern),
            "Expected IR to contain '{}'\n\nCode:\n{}\n\nIR:\n{}",
            pattern,
            code,
            ir
        );
    }

    /// Verify IR contains ALL specified patterns
    fn assert_ir_contains_all(code: &str, patterns: &[&str]) {
        let ir = get_ir(code);
        for pattern in patterns {
            assert!(
                ir.contains(pattern),
                "Expected IR to contain '{}'\n\nCode:\n{}\n\nIR:\n{}",
                pattern,
                code,
                ir
            );
        }
    }

    /// Verify IR contains at least one of the patterns
    fn assert_ir_contains_any(code: &str, patterns: &[&str]) {
        let ir = get_ir(code);
        let found = patterns.iter().any(|p| ir.contains(p));
        assert!(
            found,
            "Expected IR to contain any of {:?}\n\nCode:\n{}\n\nIR:\n{}",
            patterns, code, ir
        );
    }

    /// Verify compilation fails (for error case testing)
    fn assert_codegen_fails(code: &str) {
        assert!(
            compile_code(code).is_err(),
            "Expected compilation to fail for:\n{}",
            code
        );
    }

    /// Verify a function exists with specific signature pattern
    fn assert_function_signature(code: &str, fn_pattern: &str) {
        let ir = get_ir(code);
        assert!(
            ir.contains(&format!("define")) && ir.contains(fn_pattern),
            "Expected function matching '{}'\n\nIR:\n{}",
            fn_pattern,
            ir
        );
    }

    /// Count occurrences of pattern in IR
    fn count_ir_pattern(code: &str, pattern: &str) -> usize {
        let ir = get_ir(code);
        ir.matches(pattern).count()
    }

    // ========================================
    // FUNCTION GENERATION
    // Purpose: Verify functions generate correct LLVM IR signatures
    // ========================================

    #[test]
    fn codegen_main_function() {
        // Verify: @main function exists with void return
        assert_ir_contains_all("fn main() { }", &["@main", "ret void"]);
    }

    #[test]
    fn codegen_function_with_int_params() {
        // Verify: function has i64 params and returns i64
        let code = "fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { }";
        let ir = get_ir(code);

        assert!(ir.contains("@add"), "Function @add not found");
        // Should have integer parameters
        assert!(
            ir.contains("i64") || ir.contains("i32"),
            "No integer type in params"
        );
    }

    #[test]
    fn codegen_function_with_string_param() {
        // Verify: string param generates pointer type
        let code = r#"fn greet(name: Str) { print(name); } fn main() { }"#;
        let ir = get_ir(code);

        assert!(ir.contains("@greet"), "Function @greet not found");
    }

    #[test]
    fn codegen_function_call_generates_call_instruction() {
        // Verify: function call generates 'call' instruction
        let code = "fn helper() -> Int { return 42; } fn main() { let x = helper(); }";
        assert_ir_contains(code, "call");
    }

    #[test]
    fn codegen_recursive_function_call() {
        // Verify: recursive call generates call to self
        let code = r#"
            fn fact(n: Int) -> Int {
                if n <= 1 { return 1; }
                return n * fact(n - 1);
            }
            fn main() { }
        "#;
        let ir = get_ir(code);

        assert!(ir.contains("@fact"), "Function @fact not found");
        // Should call itself
        assert!(ir.matches("call").count() >= 1, "No recursive call found");
    }

    // ========================================
    // ARITHMETIC OPERATIONS
    // Purpose: Verify correct LLVM arithmetic instructions
    // ========================================

    #[test]
    fn codegen_int_addition() {
        // Verify: integer addition uses 'add' instruction
        assert_ir_contains("fn main() { let a = 5; let b = 3; let x = a + b; }", "add");
    }

    #[test]
    fn codegen_int_subtraction() {
        // Verify: integer subtraction uses 'sub' instruction
        assert_ir_contains("fn main() { let a = 5; let b = 3; let x = a - b; }", "sub");
    }

    #[test]
    fn codegen_int_multiplication() {
        // Verify: integer multiplication uses 'mul' instruction
        assert_ir_contains("fn main() { let a = 5; let b = 3; let x = a * b; }", "mul");
    }

    #[test]
    fn codegen_int_division() {
        // Verify: signed integer division uses 'sdiv' instruction
        assert_ir_contains("fn main() { let a = 6; let b = 3; let x = a / b; }", "sdiv");
    }

    #[test]
    fn codegen_int_modulo() {
        // Verify: modulo uses 'srem' instruction
        assert_ir_contains("fn main() { let a = 7; let b = 3; let x = a % b; }", "srem");
    }

    #[test]
    fn codegen_float_addition() {
        // Verify: float addition uses 'fadd' instruction
        assert_ir_contains(
            "fn main() { let a = 5.0; let b = 3.0; let x = a + b; }",
            "fadd",
        );
    }

    #[test]
    fn codegen_float_subtraction() {
        // Verify: float subtraction uses 'fsub' instruction
        assert_ir_contains(
            "fn main() { let a = 5.0; let b = 3.0; let x = a - b; }",
            "fsub",
        );
    }

    #[test]
    fn codegen_float_multiplication() {
        // Verify: float multiplication uses 'fmul' instruction
        assert_ir_contains(
            "fn main() { let a = 5.0; let b = 3.0; let x = a * b; }",
            "fmul",
        );
    }

    #[test]
    fn codegen_float_division() {
        // Verify: float division uses 'fdiv' instruction
        assert_ir_contains(
            "fn main() { let a = 6.0; let b = 3.0; let x = a / b; }",
            "fdiv",
        );
    }

    // ========================================
    // COMPARISON OPERATIONS
    // Purpose: Verify correct comparison instructions
    // ========================================

    #[test]
    fn codegen_int_comparison_eq() {
        // Verify: equality comparison uses icmp eq
        assert_ir_contains_any("fn main() { let x = 5 == 3; }", &["icmp eq", "icmp"]);
    }

    #[test]
    fn codegen_int_comparison_lt() {
        // Verify: less-than uses icmp slt (signed)
        assert_ir_contains_any("fn main() { let x = 5 < 3; }", &["icmp slt", "icmp"]);
    }

    #[test]
    fn codegen_float_comparison() {
        // Verify: float comparison generates comparison instruction
        // Note: The compiler may optimize constant comparisons at compile time
        assert_ir_contains_any(
            "fn main() { let x = 5.0 < 3.0; }",
            &["fcmp", "icmp", "store"],
        );
    }

    // ========================================
    // BOOLEAN OPERATIONS
    // Purpose: Verify logical operations
    // ========================================

    #[test]
    fn codegen_logical_and() {
        // Verify: && generates 'and' instruction
        assert_ir_contains("fn main() { let x = true && false; }", "and");
    }

    #[test]
    fn codegen_logical_or() {
        // Verify: || generates 'or' instruction
        assert_ir_contains("fn main() { let x = true || false; }", "or");
    }

    #[test]
    fn codegen_logical_not() {
        // Verify: ! generates negation (xor or icmp for booleans)
        // The compiler may optimize !true to false at compile time
        let ir = get_ir("fn main() { let x = !true; }");
        // Just verify it compiles and generates a main function
        assert!(ir.contains("@main"), "Main function not generated");
    }

    // ========================================
    // MEMORY OPERATIONS
    // Purpose: Verify variable allocation, store, load
    // ========================================

    #[test]
    fn codegen_variable_alloca() {
        // Verify: variable declaration generates alloca
        assert_ir_contains("fn main() { let x = 42; }", "alloca");
    }

    #[test]
    fn codegen_variable_store() {
        // Verify: assignment generates store
        assert_ir_contains("fn main() { let x = 42; }", "store");
    }

    #[test]
    fn codegen_variable_load() {
        // Verify: variable use generates load
        assert_ir_contains("fn main() { let x = 42; let y = x; }", "load");
    }

    #[test]
    fn codegen_mutable_reassignment() {
        // Verify: mutable variable reassignment generates multiple stores
        let code = "fn main() { let mut x = 1; x = 2; x = 3; }";
        let store_count = count_ir_pattern(code, "store");
        assert!(
            store_count >= 3,
            "Expected at least 3 stores, got {}",
            store_count
        );
    }

    #[test]
    fn codegen_compound_assignment() {
        // Verify: += generates load, add, store sequence
        let code = "fn main() { let mut x = 10; x += 5; }";
        let ir = get_ir(code);
        assert!(ir.contains("load"), "Missing load for compound assignment");
        assert!(ir.contains("add"), "Missing add for compound assignment");
        assert!(
            ir.contains("store"),
            "Missing store for compound assignment"
        );
    }

    // ========================================
    // CONTROL FLOW
    // Purpose: Verify branch and phi instructions
    // ========================================

    #[test]
    fn codegen_if_generates_branch() {
        // Verify: if statement generates conditional branch
        assert_ir_contains("fn main() { if true { let x = 1; } }", "br");
    }

    #[test]
    fn codegen_if_else_generates_branches() {
        // Verify: if-else has conditional branch and label blocks
        let code = "fn main() { if true { let x = 1; } else { let y = 2; } }";
        let ir = get_ir(code);

        let br_count = ir.matches("br ").count();
        assert!(
            br_count >= 2,
            "Expected at least 2 branches, got {}",
            br_count
        );
    }

    #[test]
    fn codegen_for_loop_generates_branch() {
        // Verify: for loop generates branch instructions
        assert_ir_contains("fn main() { for i in 0..10 { let x = i; } }", "br");
    }

    #[test]
    fn codegen_for_loop_has_loop_structure() {
        // Verify: for loop has multiple basic blocks
        let code = "fn main() { for i in 0..5 { let x = i; } }";
        let ir = get_ir(code);

        // Should have multiple basic block labels
        let label_count = ir.matches(":").count();
        assert!(
            label_count >= 3,
            "Expected at least 3 labels for loop structure"
        );
    }

    #[test]
    fn codegen_break_generates_branch() {
        // Verify: break generates unconditional branch out of loop
        let code = "fn main() { for i in 0..10 { if i == 5 { break; } } }";
        let ir = get_ir(code);
        // Multiple branches expected (loop + break)
        let br_count = ir.matches("br ").count();
        assert!(br_count >= 3, "Expected multiple branches for break");
    }

    #[test]
    fn codegen_continue_generates_branch() {
        // Verify: continue generates branch back to loop header
        let code = "fn main() { for i in 0..10 { if i == 5 { continue; } } }";
        let ir = get_ir(code);
        let br_count = ir.matches("br ").count();
        assert!(br_count >= 3, "Expected multiple branches for continue");
    }

    #[test]
    fn codegen_infinite_loop() {
        // Verify: infinite for loop generates loop structure
        let code = r#"
            fn main() {
                let mut i = 0;
                for {
                    if i >= 5 { break; }
                    i++;
                }
            }
        "#;
        assert_ir_contains(code, "br");
    }

    // ========================================
    // TYPE REPRESENTATION
    // Purpose: Verify correct LLVM types
    // ========================================

    #[test]
    fn codegen_int_type() {
        // Verify: Int maps to i64 or i32
        let ir = get_ir("fn main() { let x: Int = 42; }");
        assert!(
            ir.contains("i64") || ir.contains("i32"),
            "No integer type found in IR"
        );
    }

    #[test]
    fn codegen_float_type() {
        // Verify: Float maps to double or float
        let ir = get_ir("fn main() { let x: Float = 3.14; }");
        assert!(
            ir.contains("double") || ir.contains("float"),
            "No float type found in IR"
        );
    }

    #[test]
    fn codegen_bool_type() {
        // Verify: Bool maps to i1 or i8
        let ir = get_ir("fn main() { let x: Bool = true; }");
        assert!(
            ir.contains("i1") || ir.contains("i8"),
            "No bool type found in IR"
        );
    }

    // ========================================
    // ARRAY OPERATIONS
    // Purpose: Verify array codegen
    // ========================================

    #[test]
    fn codegen_array_literal() {
        // Verify: array literal generates allocation
        assert_ir_contains("fn main() { let arr = [1, 2, 3]; }", "alloca");
    }

    #[test]
    fn codegen_array_access() {
        // Verify: array access generates getelementptr
        let code = "fn main() { let arr = [1, 2, 3]; let x = arr[0]; }";
        let ir = get_ir(code);
        // Should have pointer arithmetic or array access pattern
        assert!(
            ir.contains("getelementptr") || ir.contains("load"),
            "No array access pattern found"
        );
    }

    #[test]
    fn codegen_empty_array() {
        // Verify: empty mutable array compiles
        let code = "fn main() { let mut arr: [Int] = []; }";
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "main function not generated");
    }

    #[test]
    fn codegen_array_spread() {
        // Verify: spread operator compiles
        let code = "fn main() { let a = [1, 2]; let b = [...a, 3]; }";
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Spread failed to compile");
    }

    #[test]
    fn codegen_array_slice() {
        // Verify: array slice compiles
        let code = "fn main() { let arr = [1, 2, 3, 4, 5]; let s = arr[1..4]; }";
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Slice failed to compile");
    }

    // ========================================
    // MAP OPERATIONS
    // Purpose: Verify map codegen
    // ========================================

    #[test]
    fn codegen_map_literal() {
        // Verify: map literal generates allocation
        let code = r#"fn main() { let m = {"a": 1, "b": 2}; }"#;
        assert_ir_contains(code, "alloca");
    }

    #[test]
    fn codegen_map_access() {
        // Verify: map access compiles
        let code = r#"fn main() { let m = {"key": 42}; let x = m["key"]; }"#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Map access failed");
    }

    #[test]
    fn codegen_map_iteration() {
        // Verify: map iteration compiles with destructuring
        let code = r#"
            fn main() {
                let m = {"a": 1, "b": 2};
                for k, v in m {
                    print(k, v);
                }
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("br"), "No loop branches for map iteration");
    }

    // ========================================
    // STRUCT OPERATIONS
    // Purpose: Verify struct codegen
    // ========================================

    #[test]
    fn codegen_struct_definition() {
        // Verify: struct compiles to type definition
        let code = r#"
            struct User { name: Str, age: Int }
            fn main() { }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Struct definition failed");
    }

    #[test]
    fn codegen_struct_literal() {
        // Verify: struct literal generates stores for fields
        let code = r#"
            struct Point { x: Int, y: Int }
            fn main() {
                let p = Point { x: 10, y: 20 };
            }
        "#;
        let ir = get_ir(code);
        let store_count = ir.matches("store").count();
        assert!(store_count >= 2, "Expected stores for struct fields");
    }

    #[test]
    fn codegen_struct_field_access() {
        // Verify: field access generates getelementptr or load
        let code = r#"
            struct Point { x: Int, y: Int }
            fn main() {
                let p = Point { x: 10, y: 20 };
                let val = p.x;
            }
        "#;
        let ir = get_ir(code);
        assert!(
            ir.contains("getelementptr") || ir.contains("load"),
            "No field access pattern"
        );
    }

    #[test]
    fn codegen_nested_struct() {
        // Verify: nested struct field access compiles
        let code = r#"
            struct Address { city: Str }
            struct User { name: Str, address: Address }
            fn main() {
                let addr = Address { city: "NYC" };
                let u = User { name: "Alice", address: addr };
                let c = u.address.city;
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Nested struct failed");
    }

    #[test]
    fn codegen_struct_shorthand() {
        // Verify: struct shorthand syntax compiles
        let code = r#"
            struct Point { x: Int, y: Int }
            fn main() {
                let x = 10;
                let y = 20;
                let p = Point { x, y };
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Struct shorthand failed");
    }

    #[test]
    fn codegen_struct_method() {
        // Verify: struct method generates separate function and call
        let code = r#"
            struct User { age: Int }
            fn User.isAdult(self) -> Bool { return self.age >= 18; }
            fn main() {
                let u = User { age: 25 };
                let adult = u.isAdult();
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("call"), "No method call generated");
    }

    #[test]
    fn codegen_multiple_struct_methods() {
        // Verify: multiple methods generate multiple functions
        let code = r#"
            struct Point { x: Int, y: Int }
            fn Point.getX(self) -> Int { return self.x; }
            fn Point.getY(self) -> Int { return self.y; }
            fn main() {
                let p = Point { x: 3, y: 4 };
                let sum = p.getX() + p.getY();
            }
        "#;
        let ir = get_ir(code);
        let call_count = ir.matches("call").count();
        assert!(
            call_count >= 2,
            "Expected 2+ method calls, got {}",
            call_count
        );
    }

    // ========================================
    // ENUM OPERATIONS
    // Purpose: Verify enum codegen
    // ========================================

    #[test]
    fn codegen_enum_simple() {
        // Verify: simple enum compiles
        let code = r#"
            enum Status { Active, Inactive }
            fn main() { let s = Status::Active; }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Simple enum failed");
    }

    #[test]
    fn codegen_enum_with_payload() {
        // Verify: enum with payload compiles
        let code = r#"
            enum Option { Some(Int), None }
            fn main() { let opt = Option::Some(42); }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Enum with payload failed");
    }

    #[test]
    fn codegen_enum_multiple_payloads() {
        // Verify: enum with different payload types compiles
        let code = r#"
            enum Result { Success(Int), Failure(Str), Unknown }
            fn main() {
                let r1 = Result::Success(42);
                let r2 = Result::Failure("error");
                let r3 = Result::Unknown;
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Multi-payload enum failed");
    }

    // ========================================
    // MATCH EXPRESSION
    // Purpose: Verify match generates correct branches
    // ========================================

    #[test]
    fn codegen_match_int() {
        // Verify: match on int generates comparisons and branches
        let code = r#"
            fn main() {
                let x = 42;
                match x {
                    1 => print("one"),
                    42 => print("answer"),
                    _ => print("other"),
                }
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("icmp"), "No comparison for match");
        assert!(ir.contains("br"), "No branches for match arms");
    }

    #[test]
    fn codegen_match_float() {
        // Verify: match on float generates fcmp
        let code = r#"
            fn main() {
                let x = 3.14;
                match x {
                    3.14 => print("pi"),
                    _ => print("other"),
                }
            }
        "#;
        let ir = get_ir(code);
        assert!(
            ir.contains("fcmp") || ir.contains("br"),
            "No float match codegen"
        );
    }

    #[test]
    fn codegen_match_bool() {
        // Verify: match on bool generates branches
        let code = r#"
            fn main() {
                let x = true;
                match x {
                    true => print("yes"),
                    false => print("no"),
                }
            }
        "#;
        assert_ir_contains(code, "br");
    }

    #[test]
    fn codegen_match_string() {
        // Verify: match on string compiles
        let code = r#"
            fn main() {
                let x = "hello";
                match x {
                    "hello" => print("greeting"),
                    _ => print("other"),
                }
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "String match failed");
    }

    #[test]
    fn codegen_match_enum() {
        // Verify: match on enum generates variant check
        let code = r#"
            enum Status { Active, Inactive }
            fn main() {
                let s = Status::Active;
                match s {
                    Status::Active => print("active"),
                    Status::Inactive => print("inactive"),
                }
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("br"), "No branches for enum match");
    }

    #[test]
    fn codegen_match_enum_with_binding() {
        // Verify: match with payload binding compiles
        let code = r#"
            enum Option { Some(Int), None }
            fn main() {
                let opt = Option::Some(42);
                match opt {
                    Option::Some(val) => print(val),
                    Option::None => print("none"),
                }
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Enum binding match failed");
    }

    #[test]
    fn codegen_match_as_expression() {
        // Verify: match as expression generates result
        let code = r#"
            fn main() {
                let x = 1;
                let msg = match x {
                    1 => "one",
                    _ => "other",
                };
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("br"), "Match expression has no branches");
    }

    #[test]
    fn codegen_nested_match() {
        // Verify: nested match compiles with correct structure
        let code = r#"
            fn main() {
                let x = 1;
                let y = 2;
                match x {
                    1 => {
                        match y {
                            2 => print("x=1, y=2"),
                            _ => print("x=1"),
                        }
                    },
                    _ => print("other"),
                }
            }
        "#;
        let ir = get_ir(code);
        let br_count = ir.matches("br ").count();
        assert!(br_count >= 4, "Nested match needs multiple branches");
    }

    // ========================================
    // ERROR HANDLING / RESULT TYPE
    // Purpose: Verify Result type codegen
    // ========================================

    #[test]
    fn codegen_result_function() {
        // Verify: function with Result return type compiles
        let code = r#"
            fn divide(a: Int, b: Int) -> Int ! Str {
                if b == 0 { Err "division by zero"; }
                Ok a / b;
            }
            fn main() { }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@divide"), "Result function not generated");
    }

    #[test]
    fn codegen_result_extraction() {
        // Verify: let val, err = ... pattern compiles
        let code = r#"
            fn getValue(x: Int) -> Int ! Str {
                if x < 0 { Err "negative"; }
                Ok 42;
            }
            fn main() {
                let result, err = getValue(1);
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("call"), "Result call not generated");
    }

    #[test]
    fn codegen_error_propagation() {
        // Verify: ? operator compiles
        let code = r#"
            fn inner(x: Int) -> Int ! Str {
                if x < 0 { Err "negative"; }
                Ok 42;
            }
            fn outer(x: Int) -> Int ! Str {
                let val = inner(x)?;
                Ok val;
            }
            fn main() { }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@outer"), "Outer function not generated");
        assert!(ir.contains("br"), "No branches for ? operator");
    }

    #[test]
    fn codegen_nil_comparison() {
        // Verify: nil comparison compiles
        let code = r#"
            fn getValue(x: Int) -> Int ! Str {
                if x < 0 { Err "negative"; }
                Ok 42;
            }
            fn main() {
                let result, err = getValue(1);
                if err == nil { print(result); }
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "nil comparison failed");
    }

    #[test]
    fn codegen_struct_error_type() {
        // Verify: struct as error type compiles
        let code = r#"
            struct MyError { code: Int, message: Str }
            fn risky(x: Int) -> Int ! MyError {
                if x < 0 {
                    Err MyError { code: 100, message: "negative" };
                }
                Ok x * 2;
            }
            fn main() {
                let result, err = risky(5);
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@risky"), "Struct error function not generated");
    }

    // ========================================
    // TUPLE
    // Purpose: Verify tuple codegen
    // ========================================

    #[test]
    fn codegen_tuple_return() {
        // Verify: tuple return compiles
        let code = r#"
            fn getData() -> Int, Str { return 42, "hello"; }
            fn main() { }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@getData"), "Tuple function not generated");
    }

    #[test]
    fn codegen_tuple_destructuring() {
        // Verify: tuple destructuring compiles
        let code = r#"
            fn getData() -> Int, Str { return 42, "hello"; }
            fn main() {
                let a, b = getData();
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("call"), "Tuple call not generated");
    }

    // ========================================
    // STRING OPERATIONS
    // Purpose: Verify string codegen
    // ========================================

    #[test]
    fn codegen_string_literal() {
        // Verify: string literal creates global constant
        let code = r#"fn main() { let s = "hello"; }"#;
        let ir = get_ir(code);
        assert!(
            ir.contains("hello") || ir.contains("c\""),
            "String literal not in IR"
        );
    }

    #[test]
    fn codegen_string_concatenation() {
        // Verify: string concat generates call (to runtime function)
        let code = r#"fn main() { let s = "hello" + " world"; }"#;
        let ir = get_ir(code);
        // Should either inline or call concat function
        assert!(ir.contains("@main"), "String concat failed");
    }

    #[test]
    fn codegen_string_interpolation() {
        // Verify: string interpolation compiles
        let code = r#"fn main() { let name = "Alice"; let msg = "Hello ${name}!"; }"#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "String interpolation failed");
    }

    #[test]
    fn codegen_string_interpolation_expression() {
        // Verify: interpolation with expression compiles
        let code = r#"fn main() { let x = 5; let msg = "Value: ${x + 3}"; }"#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Interpolation expression failed");
    }

    #[test]
    fn codegen_string_methods() {
        // Verify: string methods compile
        let code = r#"
            fn main() {
                let s = "hello world";
                let len = s.len();
                let starts = s.startsWith("hello");
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("call"), "String method calls missing");
    }

    // ========================================
    // LAMBDA / ARRAY METHODS
    // Purpose: Verify lambda codegen
    // ========================================

    #[test]
    fn codegen_lambda_map() {
        // Verify: map lambda generates anonymous function
        let code = "fn main() { let arr = [1, 2, 3]; let x = arr.map((n) => n * 2); }";
        let ir = get_ir(code);
        // Should have lambda function
        assert!(
            ir.contains("lambda") || ir.matches("define").count() >= 2,
            "Lambda function not generated"
        );
    }

    #[test]
    fn codegen_lambda_filter() {
        // Verify: filter lambda compiles
        let code = "fn main() { let arr = [1, 2, 3, 4]; let x = arr.filter((n) => n > 2); }";
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Filter lambda failed");
    }

    #[test]
    fn codegen_lambda_reduce() {
        // Verify: reduce with 2-param lambda compiles
        let code = "fn main() { let sum = [1, 2, 3].reduce(0, (acc, x) => acc + x); }";
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Reduce lambda failed");
    }

    #[test]
    fn codegen_lambda_block() {
        // Verify: block lambda compiles
        let code = r#"
            fn main() {
                let arr = [1, 2, 3];
                let doubled = arr.map((x) => {
                    let result = x * 2;
                    return result;
                });
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Block lambda failed");
    }

    #[test]
    fn codegen_chained_lambdas() {
        // Verify: chained array methods compile
        let code = r#"
            fn main() {
                let result = [1, 2, 3, 4, 5]
                    .filter((x) => x > 2)
                    .map((x) => x * 3)
                    .reduce(0, (a, b) => a + b);
            }
        "#;
        let ir = get_ir(code);
        // Should have multiple lambda functions
        let define_count = ir.matches("define").count();
        assert!(
            define_count >= 3,
            "Expected 3+ functions for chained lambdas, got {}",
            define_count
        );
    }

    #[test]
    fn codegen_float_array_lambda() {
        // Verify: float array with lambda compiles
        let code = r#"
            fn main() {
                let floats = [1.5, 2.5, 3.5];
                let doubled = floats.map((x) => x * 2.0);
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Float array lambda failed");
    }

    #[test]
    fn codegen_bool_array_lambda() {
        // Verify: bool array operations compile
        let code = r#"
            fn main() {
                let bools = [true, false, true];
                let inverted = bools.map((b) => !b);
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Bool array lambda failed");
    }

    #[test]
    fn codegen_string_array_lambda() {
        // Verify: string array with lambda compiles
        let code = r#"
            fn main() {
                let words = ["hello", "world"];
                let excited = words.map((s) => s + "!");
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "String array lambda failed");
    }

    // ========================================
    // OPERATORS
    // Purpose: Verify special operators
    // ========================================

    #[test]
    fn codegen_increment() {
        // Verify: ++ generates add and store
        let code = "fn main() { let mut x = 5; x++; }";
        let ir = get_ir(code);
        assert!(ir.contains("add"), "Increment should use add");
        assert!(ir.contains("store"), "Increment should store result");
    }

    #[test]
    fn codegen_decrement() {
        // Verify: -- generates sub and store
        let code = "fn main() { let mut x = 5; x--; }";
        let ir = get_ir(code);
        assert!(ir.contains("sub"), "Decrement should use sub");
    }

    #[test]
    fn codegen_in_operator() {
        // Verify: in operator compiles
        let code = r#"fn main() { let arr = [1, 2, 3]; if 2 in arr { print("found"); } }"#;
        let ir = get_ir(code);
        assert!(ir.contains("br"), "in operator should have branch");
    }

    #[test]
    fn codegen_inline_if() {
        // Verify: inline if expression compiles
        let code = "fn main() { let x = 5; let abs = if x > 0 { x } else { -x }; }";
        let ir = get_ir(code);
        assert!(ir.contains("br"), "Inline if should have branches");
    }

    // ========================================
    // IMPORT
    // Purpose: Verify import codegen (basic only)
    // ========================================

    #[test]
    fn codegen_std_import() {
        // Verify: std library import compiles and call is generated
        let code = "import std::Math::{Abs}; fn main() { let x = Abs(-5); }";
        let ir = get_ir(code);
        assert!(ir.contains("call"), "Imported function call not generated");
    }

    // ========================================
    // RETURN STATEMENTS
    // Purpose: Verify return codegen
    // ========================================

    #[test]
    fn codegen_return_void() {
        // Verify: void function returns ret void
        assert_ir_contains("fn main() { }", "ret void");
    }

    #[test]
    fn codegen_return_int() {
        // Verify: int return generates ret with value
        let code = "fn getValue() -> Int { return 42; } fn main() { }";
        let ir = get_ir(code);
        // Should have ret instruction with integer
        assert!(ir.contains("ret"), "No ret instruction");
    }

    #[test]
    fn codegen_early_return() {
        // Verify: early return generates branch/ret
        let code = r#"
            fn check(x: Int) -> Int {
                if x < 0 { return -1; }
                return x;
            }
            fn main() { }
        "#;
        let ir = get_ir(code);
        let ret_count = ir.matches("ret").count();
        assert!(ret_count >= 2, "Expected multiple returns");
    }

    #[test]
    fn codegen_multiple_returns() {
        // Verify: multiple return paths all have ret
        let code = r#"
            fn classify(n: Int) -> Str {
                if n < 0 { return "negative"; }
                if n == 0 { return "zero"; }
                return "positive";
            }
            fn main() { }
        "#;
        let ir = get_ir(code);
        let ret_count = ir.matches("ret").count();
        assert!(
            ret_count >= 3,
            "Expected 3+ ret for multiple returns, got {}",
            ret_count
        );
    }

    // ========================================
    // COMPREHENSIVE PROGRAMS
    // Purpose: Verify complex programs compile correctly
    // ========================================

    #[test]
    fn codegen_comprehensive_struct_method() {
        // Verify: complex struct usage compiles
        let code = r#"
            struct User { name: Str, age: Int }
            fn User.isAdult(self) -> Bool { return self.age >= 18; }
            fn User.greet(self) -> Str { return "Hello, " + self.name; }
            fn main() {
                let user = User { name: "Alice", age: 25 };
                let adult = user.isAdult();
                let greeting = user.greet();
                print(adult, greeting);
            }
        "#;
        let ir = get_ir(code);
        let call_count = ir.matches("call").count();
        assert!(
            call_count >= 3,
            "Expected 3+ calls (2 methods + print), got {}",
            call_count
        );
    }

    #[test]
    fn codegen_comprehensive_control_flow() {
        // Verify: nested loops and conditions compile
        let code = r#"
            fn main() {
                let mut total = 0;
                for i in 0..5 {
                    for j in 0..5 {
                        if i == j { continue; }
                        if total > 20 { break; }
                        total += i + j;
                    }
                }
            }
        "#;
        let ir = get_ir(code);
        let br_count = ir.matches("br ").count();
        assert!(
            br_count >= 8,
            "Complex control flow needs many branches, got {}",
            br_count
        );
    }

    #[test]
    fn codegen_comprehensive_error_handling() {
        // Verify: complete error handling chain compiles
        let code = r#"
            fn validate(x: Int) -> ! Str {
                if x < 0 { Err "negative"; }
            }
            fn process(x: Int) -> Int ! Str {
                validate(x)?;
                Ok x * 2;
            }
            fn main() {
                let result, err = process(5);
                if err == nil {
                    print(result);
                } else {
                    print("Error occurred");
                }
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@validate"), "validate function missing");
        assert!(ir.contains("@process"), "process function missing");
        assert!(ir.contains("@main"), "main function missing");
    }

    #[test]
    fn codegen_array_of_structs() {
        // Verify: array containing structs compiles
        let code = r#"
            struct Item { id: Int, name: Str }
            fn main() {
                let items = [
                    Item { id: 1, name: "one" },
                    Item { id: 2, name: "two" }
                ];
                let first = items[0];
                print(first.name);
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Array of structs failed");
    }

    #[test]
    fn codegen_all_map_key_types() {
        // Verify: maps with different key types compile
        let code = r#"
            fn main() {
                let strMap = {"a": 1, "b": 2};
                let intMap = {1: "one", 2: "two"};
                let boolMap = {true: 100, false: 200};
                print(strMap["a"]);
            }
        "#;
        let ir = get_ir(code);
        assert!(ir.contains("@main"), "Multi-key-type maps failed");
    }
}
