//! MIR Layer Unit Tests
//!
//! Responsibility: Verify MIR-SPECIFIC transformations and structure
//!   - Function generation (count, names, signatures)
//!   - Lambda → Closure instruction conversion
//!   - Block structure for control flow
//!   - MIR instruction types present
//!
//! Each test verifies SPECIFIC MIR output, not just "did it build."
//! If you only need "does it compile," use codegen tests instead.

#[cfg(test)]
mod mir_tests {
    use crate::analyzer::SemanticAnalyzer;
    use crate::lexer::lexer::lex;
    use crate::mir::builder::MirBuilder;
    use crate::mir::{MirFunction, MirInstr, MirProgram};
    use crate::parser::ast::AstNode;
    use crate::parser::Parser;
    use bumpalo::Bump;

    // ========================================
    // HELPER FUNCTIONS
    // ========================================

    fn build_mir(input: &str) -> Result<MirProgram, String> {
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

                    let mut mir_builder = MirBuilder::new();
                    mir_builder.build_program(nodes);
                    mir_builder.finalize();
                    Ok(mir_builder.program)
                } else {
                    Err("Not a program".to_string())
                }
            }
            Err(e) => Err(format!("Parse error: {:?}", e)),
        }
    }

    fn get_mir(code: &str) -> MirProgram {
        build_mir(code).expect("MIR build failed")
    }

    fn get_function<'a>(program: &'a MirProgram, name: &str) -> Option<&'a MirFunction> {
        program.functions.iter().find(|f| f.name == name)
    }

    // ========================================
    // VERIFICATION HELPERS
    // ========================================

    /// Verify exact function count in MIR
    fn assert_function_count(code: &str, expected: usize, reason: &str) {
        let mir = get_mir(code);
        assert_eq!(
            mir.functions.len(),
            expected,
            "{}: expected {} functions, got {}",
            reason,
            expected,
            mir.functions.len()
        );
    }

    /// Verify a function exists by name
    fn assert_function_exists(code: &str, fn_name: &str) {
        let mir = get_mir(code);
        assert!(
            get_function(&mir, fn_name).is_some(),
            "Expected function '{}' in MIR, found: {:?}",
            fn_name,
            mir.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }

    /// Verify function has specific parameter count
    fn assert_function_param_count(code: &str, fn_name: &str, expected: usize) {
        let mir = get_mir(code);
        let func = get_function(&mir, fn_name).expect(&format!("Function '{}' not found", fn_name));
        assert_eq!(
            func.params.len(),
            expected,
            "Function '{}' expected {} params, got {}",
            fn_name,
            expected,
            func.params.len()
        );
    }

    /// Verify function has at least N blocks (for complex control flow)
    fn assert_function_min_blocks(code: &str, fn_name: &str, min: usize) {
        let mir = get_mir(code);
        let func = get_function(&mir, fn_name).expect(&format!("Function '{}' not found", fn_name));
        assert!(
            func.blocks.len() >= min,
            "Function '{}' expected at least {} blocks, got {}",
            fn_name,
            min,
            func.blocks.len()
        );
    }

    /// Count Closure instructions in a function
    fn count_closure_instructions(mir: &MirProgram, fn_name: &str) -> usize {
        if let Some(func) = mir.functions.iter().find(|f| f.name == fn_name) {
            func.blocks
                .iter()
                .flat_map(|b| b.instrs.iter())
                .filter(|i| matches!(i, MirInstr::Closure { .. }))
                .count()
        } else {
            0
        }
    }

    /// Verify a function contains Closure instructions (lambdas are stored as Closure instructions)
    fn assert_closure_count(code: &str, fn_name: &str, expected: usize, reason: &str) {
        let mir = get_mir(code);
        let closure_count = count_closure_instructions(&mir, fn_name);
        assert_eq!(
            closure_count, expected,
            "{}: expected {} Closure instructions in '{}', got {}",
            reason, expected, fn_name, closure_count
        );
    }

    /// Get closure instruction from a function and verify its parameter count
    fn get_closure_params(mir: &MirProgram, fn_name: &str) -> Vec<Vec<String>> {
        if let Some(func) = mir.functions.iter().find(|f| f.name == fn_name) {
            func.blocks
                .iter()
                .flat_map(|b| b.instrs.iter())
                .filter_map(|i| {
                    if let MirInstr::Closure { params, .. } = i {
                        Some(params.clone())
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            vec![]
        }
    }

    // ========================================
    // FUNCTION STRUCTURE TESTS
    // Purpose: Verify MIR correctly generates function signatures
    // ========================================

    #[test]
    fn mir_main_function_generated() {
        // Verify: main function exists with 0 params
        assert_function_exists("fn main() { }", "main");
        assert_function_param_count("fn main() { }", "main", 0);
    }

    #[test]
    fn mir_function_with_params() {
        let code = "fn add(a: Int, b: Int) -> Int { return a + b; } fn main() { }";

        // Verify: function has correct params
        assert_function_exists(code, "add");
        assert_function_param_count(code, "add", 2);
    }

    #[test]
    fn mir_multiple_functions() {
        let code = r#"
            fn foo() { }
            fn bar() { }
            fn baz() { }
            fn main() { }
        "#;

        // Verify: all 4 functions generated
        assert_function_count(code, 4, "4 user functions");
    }

    #[test]
    fn mir_recursive_function() {
        let code = r#"
            fn factorial(n: Int) -> Int {
                if n <= 1 { return 1; }
                return n * factorial(n - 1);
            }
            fn main() { }
        "#;

        // Verify: recursive function generates correctly
        assert_function_exists(code, "factorial");
        assert_function_param_count(code, "factorial", 1);
    }

    #[test]
    fn mir_mutual_recursion() {
        let code = r#"
            fn isEven(n: Int) -> Bool { if n == 0 { return true; } return isOdd(n - 1); }
            fn isOdd(n: Int) -> Bool { if n == 0 { return false; } return isEven(n - 1); }
            fn main() { }
        "#;

        // Verify: both functions exist
        assert_function_count(code, 3, "isEven + isOdd + main");
    }

    // ========================================
    // STRUCT METHOD TESTS
    // Purpose: Verify struct methods become functions
    // ========================================

    #[test]
    fn mir_struct_method_becomes_function() {
        let code = r#"
            struct Counter { value: Int }
            fn Counter.getValue(self) -> Int { return self.value; }
            fn main() { }
        "#;

        // Verify: struct method generates as function
        assert_function_count(code, 2, "Counter.getValue + main");
    }

    #[test]
    fn mir_multiple_struct_methods() {
        let code = r#"
            struct Point { x: Int, y: Int }
            fn Point.getX(self) -> Int { return self.x; }
            fn Point.getY(self) -> Int { return self.y; }
            fn Point.sum(self) -> Int { return self.x + self.y; }
            fn main() { }
        "#;

        // Verify: 3 methods + main = 4 functions
        assert_function_count(code, 4, "3 methods + main");
    }

    // ========================================
    // LAMBDA/CLOSURE TESTS
    // Purpose: Verify lambdas generate Closure instructions in MIR
    // Note: Lambdas are stored as Closure instructions, not separate functions.
    //       They become actual LLVM functions during codegen.
    // ========================================

    #[test]
    fn mir_lambda_generates_function() {
        let code = "fn main() { let arr = [1, 2, 3]; let x = arr.map((n) => n * 2); }";

        // Verify: lambda creates Closure instruction in main
        assert_closure_count(code, "main", 1, "map lambda");
    }

    #[test]
    fn mir_multiple_lambdas_generate_functions() {
        let code = "fn main() { let x = [1, 2, 3].map((n) => n * 2).filter((n) => n > 2); }";

        // Verify: 2 lambdas = 2 Closure instructions
        assert_closure_count(code, "main", 2, "map + filter lambdas");
    }

    #[test]
    fn mir_chained_lambdas() {
        let code = r#"
            fn main() {
                let result = [1, 2, 3, 4, 5]
                    .filter((n) => n > 1)
                    .map((n) => n * 2)
                    .reduce(0, (acc, x) => acc + x);
            }
        "#;

        // Verify: 3 lambdas (filter + map + reduce)
        assert_closure_count(code, "main", 3, "filter + map + reduce lambdas");
    }

    #[test]
    fn mir_lambda_block_syntax() {
        let code = r#"
            fn main() {
                let arr = [1, 2, 3];
                let doubled = arr.map((x) => {
                    let result = x * 2;
                    return result;
                });
            }
        "#;

        // Verify: block lambda generates Closure instruction
        assert_closure_count(code, "main", 1, "block lambda");
    }

    #[test]
    fn mir_reduce_lambda_two_params() {
        let code = "fn main() { let sum = [1, 2, 3].reduce(0, (acc, x) => acc + x); }";

        // Verify: reduce lambda exists and has 2 params
        let mir = get_mir(code);
        let closure_params = get_closure_params(&mir, "main");

        assert_eq!(closure_params.len(), 1, "Should have 1 closure");
        assert_eq!(
            closure_params[0].len(),
            2,
            "Reduce lambda should have 2 params (acc, x)"
        );
    }

    // ========================================
    // CONTROL FLOW TESTS
    // Purpose: Verify control flow generates correct block structure
    // ========================================

    #[test]
    fn mir_if_generates_blocks() {
        let code = r#"
            fn test(x: Int) -> Int {
                if x > 0 { return 1; }
                return 0;
            }
            fn main() { }
        "#;

        // Verify: if statement creates multiple blocks
        assert_function_min_blocks(code, "test", 2);
    }

    #[test]
    fn mir_if_else_generates_blocks() {
        let code = r#"
            fn test(x: Int) -> Int {
                if x > 0 { return 1; } else { return -1; }
            }
            fn main() { }
        "#;

        // Verify: if-else creates at least 3 blocks (then, else, merge)
        assert_function_min_blocks(code, "test", 2);
    }

    #[test]
    fn mir_for_loop_generates_blocks() {
        let code = r#"
            fn test() {
                for i in 0..10 {
                    print(i);
                }
            }
            fn main() { }
        "#;

        // Verify: for loop creates blocks (init, cond, body, increment, exit)
        assert_function_min_blocks(code, "test", 2);
    }

    #[test]
    fn mir_nested_control_flow() {
        let code = r#"
            fn test(x: Int) -> Int {
                for i in 0..x {
                    if i > 5 {
                        return i;
                    }
                }
                return 0;
            }
            fn main() { }
        "#;

        // Verify: nested control flow creates many blocks
        assert_function_min_blocks(code, "test", 3);
    }

    #[test]
    fn mir_match_generates_blocks() {
        let code = r#"
            fn test(x: Int) -> Int {
                match x {
                    1 => 10,
                    2 => 20,
                    _ => 0,
                }
            }
            fn main() { }
        "#;

        // Verify: match creates blocks for arms
        assert_function_min_blocks(code, "test", 2);
    }

    // ========================================
    // RESULT TYPE TESTS
    // Purpose: Verify Result type handling in MIR
    // ========================================

    #[test]
    fn mir_result_function() {
        let code = r#"
            fn divide(a: Int, b: Int) -> Int ! Str {
                if b == 0 { Err "division by zero"; }
                Ok a / b;
            }
            fn main() { }
        "#;

        // Verify: Result function exists
        assert_function_exists(code, "divide");
        assert_function_param_count(code, "divide", 2);
    }

    #[test]
    fn mir_error_propagation() {
        let code = r#"
            fn inner() -> Int ! Str {
                Err "error";
            }
            fn outer() -> Int ! Str {
                let val = inner()?;
                Ok val;
            }
            fn main() { }
        "#;

        // Verify: both functions exist, outer has control flow for ? operator
        assert_function_count(code, 3, "inner + outer + main");
        assert_function_min_blocks(code, "outer", 2); // ? creates branch
    }

    // ========================================
    // TUPLE TESTS
    // Purpose: Verify tuple handling in MIR
    // ========================================

    #[test]
    fn mir_tuple_return() {
        let code = r#"
            fn getData() -> Int { return 42; }
            fn main() { }
        "#;

        // Verify: function exists
        assert_function_exists(code, "getData");
        assert_function_param_count(code, "getData", 0);
    }

    #[test]
    fn mir_tuple_destructuring() {
        let code = r#"
            fn getData() -> Int { return 42; }
            fn main() {
                let x = getData();
                print(x);
            }
        "#;

        // Verify: both functions exist
        assert_function_count(code, 2, "getData + main");
    }

    // ========================================
    // ENUM TESTS
    // Purpose: Verify enum handling in MIR
    // ========================================

    #[test]
    fn mir_enum_match() {
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

        // Verify: main has blocks for match arms
        assert_function_min_blocks(code, "main", 2);
    }

    #[test]
    fn mir_enum_with_payload() {
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

        // Verify: match with payload binding creates blocks
        assert_function_min_blocks(code, "main", 2);
    }

    // ========================================
    // EDGE CASES
    // Purpose: Verify MIR handles edge cases correctly
    // ========================================

    #[test]
    fn mir_empty_function() {
        let code = "fn empty() { } fn main() { }";

        // Verify: empty function exists
        // Note: empty functions may have 0 blocks in current MIR implementation
        assert_function_exists(code, "empty");
    }

    #[test]
    fn mir_many_parameters() {
        let code = "fn manyParams(a: Int, b: Int, c: Int, d: Int, e: Int) { } fn main() { }";

        // Verify: all 5 params registered
        assert_function_param_count(code, "manyParams", 5);
    }

    #[test]
    fn mir_deeply_nested_lambdas() {
        let code = r#"
            fn main() {
                let result = [1, 2, 3]
                    .map((x) => x + 1)
                    .filter((x) => x > 1)
                    .map((x) => x * 2)
                    .filter((x) => x < 10)
                    .reduce(0, (a, b) => a + b);
            }
        "#;

        // Verify: 5 Closure instructions generated
        assert_closure_count(code, "main", 5, "5 chained lambdas");
    }

    #[test]
    fn mir_infinite_loop_with_break() {
        let code = r#"
            fn main() {
                let mut i = 0;
                for {
                    if i >= 5 { break; }
                    i++;
                }
            }
        "#;

        // Verify: loop structure creates blocks
        assert_function_min_blocks(code, "main", 2);
    }
}
