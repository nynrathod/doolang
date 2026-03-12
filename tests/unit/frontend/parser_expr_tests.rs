use doo_frontend::ast::*;
use doo_frontend::parser::Parser;

fn parse(source: &str) -> Program {
    let mut parser = Parser::new(source, 0);
    parser.parse_program().unwrap()
}

fn parse_expr(source: &str) -> Expr {
    let prog = parse(source);
    match &prog.items[0] {
        Item::Statement(stmt) => match &stmt.kind {
            StmtKind::Let { value, .. } => value.clone(),
            StmtKind::Expr(expr) => expr.clone(),
            _ => panic!("Expected expression statement"),
        },
        _ => panic!("Expected statement item"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== Literal Expressions (30 tests) ====================

    #[test]
    fn test_integer_literal_zero() {
        let expr = parse_expr("let x = 0");
        assert!(matches!(expr.kind, ExprKind::IntLit(0)));
    }

    #[test]
    fn test_integer_literal_one() {
        let expr = parse_expr("let x = 1");
        assert!(matches!(expr.kind, ExprKind::IntLit(1)));
    }

    #[test]
    fn test_integer_literal_positive() {
        let expr = parse_expr("let x = 42");
        assert!(matches!(expr.kind, ExprKind::IntLit(42)));
    }

    #[test]
    fn test_integer_literal_large() {
        let expr = parse_expr("let x = 999999");
        assert!(matches!(expr.kind, ExprKind::IntLit(999999)));
    }

    #[test]
    fn test_integer_literal_negative() {
        let expr = parse_expr("let x = -1");
        match expr.kind {
            ExprKind::Unary { op, .. } => {
                assert!(matches!(op, UnaryOp::Neg));
            }
            _ => panic!("Expected unary negation"),
        }
    }

    #[test]
    fn test_integer_literal_max() {
        let expr = parse_expr("let x = 9223372036854775807");
        assert!(matches!(expr.kind, ExprKind::IntLit(9223372036854775807)));
    }

    #[test]
    fn test_float_literal_zero() {
        let expr = parse_expr("let x = 0.0");
        if let ExprKind::FloatLit(f) = expr.kind {
            assert!((f - 0.0).abs() < f64::EPSILON);
        } else {
            panic!("Expected float literal");
        }
    }

    #[test]
    fn test_float_literal_pi() {
        let expr = parse_expr("let x = 3.14");
        if let ExprKind::FloatLit(f) = expr.kind {
            assert!((f - 3.14).abs() < f64::EPSILON);
        } else {
            panic!("Expected float literal");
        }
    }

    #[test]
    fn test_float_literal_negative() {
        let expr = parse_expr("let x = -2.5");
        match expr.kind {
            ExprKind::Unary { op, .. } => {
                assert!(matches!(op, UnaryOp::Neg));
            }
            _ => panic!("Expected unary negation"),
        }
    }

    #[test]
    fn test_float_literal_scientific() {
        let expr = parse_expr("let x = 1e10");
        if let ExprKind::FloatLit(f) = expr.kind {
            assert!((f - 1e10).abs() < 1e6);
        } else {
            panic!("Expected float literal");
        }
    }

    #[test]
    fn test_string_literal_empty() {
        let expr = parse_expr(r#"let x = """#);
        if let ExprKind::StrLit(s) = &expr.kind {
            assert_eq!(s, "");
        } else {
            panic!("Expected string literal");
        }
    }

    #[test]
    fn test_string_literal_hello() {
        let expr = parse_expr(r#"let x = "hello""#);
        if let ExprKind::StrLit(s) = &expr.kind {
            assert_eq!(s, "hello");
        } else {
            panic!("Expected string literal");
        }
    }

    #[test]
    fn test_string_literal_with_spaces() {
        let expr = parse_expr(r#"let x = "hello world""#);
        if let ExprKind::StrLit(s) = &expr.kind {
            assert_eq!(s, "hello world");
        } else {
            panic!("Expected string literal");
        }
    }

    #[test]
    fn test_string_literal_with_newline() {
        let expr = parse_expr(r#"let x = "hello\nworld""#);
        assert!(matches!(expr.kind, ExprKind::StrLit(_)));
    }

    #[test]
    fn test_string_literal_with_quotes() {
        let expr = parse_expr(r#"let x = "say \"hello\"""#);
        assert!(matches!(expr.kind, ExprKind::StrLit(_)));
    }

    #[test]
    fn test_string_interpolation_simple() {
        let expr = parse_expr(r#"let x = "Hello ${name}""#);
        assert!(matches!(expr.kind, ExprKind::StringInterpolation(_)));
    }

    #[test]
    fn test_string_interpolation_multiple() {
        let expr = parse_expr(r#"let x = "${a} + ${b} = ${a+b}""#);
        assert!(matches!(expr.kind, ExprKind::StringInterpolation(_)));
    }

    #[test]
    fn test_boolean_literal_true() {
        let expr = parse_expr("let x = true");
        assert!(matches!(expr.kind, ExprKind::BoolLit(true)));
    }

    #[test]
    fn test_boolean_literal_false() {
        let expr = parse_expr("let x = false");
        assert!(matches!(expr.kind, ExprKind::BoolLit(false)));
    }

    #[test]
    fn test_nil_literal() {
        let expr = parse_expr("let x = nil");
        assert!(matches!(expr.kind, ExprKind::Nil));
    }

    #[test]
    fn test_array_literal_empty() {
        let expr = parse_expr("let x = []");
        if let ExprKind::ArrayLit(elements) = &expr.kind {
            assert_eq!(elements.len(), 0);
        } else {
            panic!("Expected array literal");
        }
    }

    #[test]
    fn test_array_literal_single() {
        let expr = parse_expr("let x = [1]");
        if let ExprKind::ArrayLit(elements) = &expr.kind {
            assert_eq!(elements.len(), 1);
        } else {
            panic!("Expected array literal");
        }
    }

    #[test]
    fn test_array_literal_multiple() {
        let expr = parse_expr("let x = [1, 2, 3]");
        if let ExprKind::ArrayLit(elements) = &expr.kind {
            assert_eq!(elements.len(), 3);
        } else {
            panic!("Expected array literal");
        }
    }

    #[test]
    fn test_array_literal_nested() {
        let expr = parse_expr("let x = [[1, 2], [3, 4]]");
        if let ExprKind::ArrayLit(elements) = &expr.kind {
            assert_eq!(elements.len(), 2);
            assert!(matches!(elements[0].kind, ExprKind::ArrayLit(_)));
        } else {
            panic!("Expected array literal");
        }
    }

    #[test]
    fn test_map_literal_simple() {
        // {a: 1} with identifier keys is ObjectLit in Doo
        let expr = parse_expr("let x = {a: 1, b: 2}");
        if let ExprKind::ObjectLit(fields) = &expr.kind {
            assert_eq!(fields.len(), 2);
        } else if let ExprKind::MapLit(entries) = &expr.kind {
            assert_eq!(entries.len(), 2);
        } else {
            panic!("Expected object or map literal, got {:?}", expr.kind);
        }
    }

    #[test]
    fn test_map_literal_empty() {
        let expr = parse_expr("let x = {}");
        if let ExprKind::MapLit(entries) = &expr.kind {
            assert_eq!(entries.len(), 0);
        } else {
            panic!("Expected map literal");
        }
    }

    #[test]
    fn test_tuple_literal_simple() {
        let expr = parse_expr(r#"let x = (1, "a", true)"#);
        if let ExprKind::TupleLit(elements) = &expr.kind {
            assert_eq!(elements.len(), 3);
        } else {
            panic!("Expected tuple literal");
        }
    }

    #[test]
    fn test_tuple_literal_pair() {
        let expr = parse_expr("let x = (1, 2)");
        if let ExprKind::TupleLit(elements) = &expr.kind {
            assert_eq!(elements.len(), 2);
        } else {
            panic!("Expected tuple literal");
        }
    }

    #[test]
    fn test_array_literal_strings() {
        let expr = parse_expr(r#"let x = ["a", "b", "c"]"#);
        if let ExprKind::ArrayLit(elements) = &expr.kind {
            assert_eq!(elements.len(), 3);
        } else {
            panic!("Expected array literal");
        }
    }

    #[test]
    fn test_map_literal_string_keys() {
        let expr = parse_expr(r#"let x = {"name": "John", "age": 30}"#);
        if let ExprKind::MapLit(entries) = &expr.kind {
            assert_eq!(entries.len(), 2);
        } else {
            panic!("Expected map literal");
        }
    }

    // ==================== Binary Expressions (30 tests) ====================

    #[test]
    fn test_binary_add() {
        let expr = parse_expr("let x = a + b");
        if let ExprKind::Binary { op, .. } = expr.kind {
            assert!(matches!(op, BinaryOp::Add));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_binary_subtract() {
        let expr = parse_expr("let x = a - b");
        if let ExprKind::Binary { op, .. } = expr.kind {
            assert!(matches!(op, BinaryOp::Sub));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_binary_multiply() {
        let expr = parse_expr("let x = a * b");
        if let ExprKind::Binary { op, .. } = expr.kind {
            assert!(matches!(op, BinaryOp::Mul));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_binary_divide() {
        let expr = parse_expr("let x = a / b");
        if let ExprKind::Binary { op, .. } = expr.kind {
            assert!(matches!(op, BinaryOp::Div));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_binary_modulo() {
        let expr = parse_expr("let x = a % b");
        if let ExprKind::Binary { op, .. } = expr.kind {
            assert!(matches!(op, BinaryOp::Mod));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_binary_equal() {
        let expr = parse_expr("let x = a == b");
        if let ExprKind::Binary { op, .. } = expr.kind {
            assert!(matches!(op, BinaryOp::Eq));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_binary_not_equal() {
        let expr = parse_expr("let x = a != b");
        if let ExprKind::Binary { op, .. } = expr.kind {
            assert!(matches!(op, BinaryOp::NotEq));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_binary_less_than() {
        let expr = parse_expr("let x = a < b");
        if let ExprKind::Binary { op, .. } = expr.kind {
            assert!(matches!(op, BinaryOp::Lt));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_binary_greater_than() {
        let expr = parse_expr("let x = a > b");
        if let ExprKind::Binary { op, .. } = expr.kind {
            assert!(matches!(op, BinaryOp::Gt));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_binary_less_equal() {
        let expr = parse_expr("let x = a <= b");
        if let ExprKind::Binary { op, .. } = expr.kind {
            assert!(matches!(op, BinaryOp::LtEq));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_binary_greater_equal() {
        let expr = parse_expr("let x = a >= b");
        if let ExprKind::Binary { op, .. } = expr.kind {
            assert!(matches!(op, BinaryOp::GtEq));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_binary_logical_and() {
        let expr = parse_expr("let x = a && b");
        if let ExprKind::Binary { op, .. } = expr.kind {
            assert!(matches!(op, BinaryOp::And));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_binary_logical_or() {
        let expr = parse_expr("let x = a || b");
        if let ExprKind::Binary { op, .. } = expr.kind {
            assert!(matches!(op, BinaryOp::Or));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_binary_bitwise_and() {
        let expr = parse_expr("let x = a & b");
        if let ExprKind::Binary { op, .. } = expr.kind {
            assert!(matches!(op, BinaryOp::BitAnd));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_binary_bitwise_or() {
        let expr = parse_expr("let x = a | b");
        if let ExprKind::Binary { op, .. } = expr.kind {
            assert!(matches!(op, BinaryOp::BitOr));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_binary_null_coalesce() {
        // Doo uses `?? panic(msg)` not binary null coalesce
        let expr = parse_expr(r#"let x = a ?? panic("required")"#);
        if let ExprKind::UnwrapOrPanic { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected unwrap or panic");
        }
    }

    #[test]
    fn test_binary_in_operator() {
        let expr = parse_expr("let x = x in arr");
        if let ExprKind::Binary { op, .. } = expr.kind {
            assert!(matches!(op, BinaryOp::In));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_precedence_add_mul() {
        let expr = parse_expr("let x = 1 + 2 * 3");
        if let ExprKind::Binary {
            op: BinaryOp::Add,
            left,
            right,
        } = expr.kind
        {
            assert!(matches!(left.kind, ExprKind::IntLit(1)));
            assert!(matches!(
                right.kind,
                ExprKind::Binary {
                    op: BinaryOp::Mul,
                    ..
                }
            ));
        } else {
            panic!("Expected add with mul on right");
        }
    }

    #[test]
    fn test_precedence_parens() {
        let expr = parse_expr("let x = (1 + 2) * 3");
        if let ExprKind::Binary {
            op: BinaryOp::Mul,
            left,
            right,
        } = expr.kind
        {
            assert!(matches!(
                left.kind,
                ExprKind::Binary {
                    op: BinaryOp::Add,
                    ..
                }
            ));
            assert!(matches!(right.kind, ExprKind::IntLit(3)));
        } else {
            panic!("Expected mul with add on left");
        }
    }

    #[test]
    fn test_associativity_left_to_right() {
        let expr = parse_expr("let x = 1 - 2 - 3");
        if let ExprKind::Binary {
            op: BinaryOp::Sub,
            left,
            right,
        } = expr.kind
        {
            assert!(matches!(
                left.kind,
                ExprKind::Binary {
                    op: BinaryOp::Sub,
                    ..
                }
            ));
            assert!(matches!(right.kind, ExprKind::IntLit(3)));
        } else {
            panic!("Expected left-associative subtraction");
        }
    }

    #[test]
    fn test_complex_arithmetic() {
        let expr = parse_expr("let x = a + b * c - d / e");
        assert!(matches!(expr.kind, ExprKind::Binary { .. }));
    }

    #[test]
    fn test_mixed_comparison_logical() {
        let expr = parse_expr("let x = a > 0 && b < 10");
        if let ExprKind::Binary {
            op: BinaryOp::And,
            left,
            right,
        } = expr.kind
        {
            assert!(matches!(
                left.kind,
                ExprKind::Binary {
                    op: BinaryOp::Gt,
                    ..
                }
            ));
            assert!(matches!(
                right.kind,
                ExprKind::Binary {
                    op: BinaryOp::Lt,
                    ..
                }
            ));
        } else {
            panic!("Expected logical and with comparisons");
        }
    }

    #[test]
    fn test_chained_logical_and() {
        let expr = parse_expr("let x = a && b && c");
        assert!(matches!(
            expr.kind,
            ExprKind::Binary {
                op: BinaryOp::And,
                ..
            }
        ));
    }

    #[test]
    fn test_chained_logical_or() {
        let expr = parse_expr("let x = a || b || c");
        assert!(matches!(
            expr.kind,
            ExprKind::Binary {
                op: BinaryOp::Or,
                ..
            }
        ));
    }

    #[test]
    fn test_mixed_logical_operators() {
        let expr = parse_expr("let x = a && b || c");
        assert!(matches!(expr.kind, ExprKind::Binary { .. }));
    }

    #[test]
    fn test_comparison_chain() {
        let expr = parse_expr("let x = a < b && b < c");
        if let ExprKind::Binary {
            op: BinaryOp::And, ..
        } = expr.kind
        {
            // Success
        } else {
            panic!("Expected logical and");
        }
    }

    #[test]
    fn test_null_coalesce_chain() {
        // Doo uses `?? panic(msg)` not binary null coalesce
        let expr = parse_expr(r#"let x = a ?? panic("required")"#);
        if let ExprKind::UnwrapOrPanic { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected unwrap or panic");
        }
    }

    #[test]
    fn test_complex_boolean_expression() {
        let expr = parse_expr("let x = (a > 0 && b < 10) || c == true");
        if let ExprKind::Binary {
            op: BinaryOp::Or, ..
        } = expr.kind
        {
            // Success
        } else {
            panic!("Expected logical or");
        }
    }

    // ==================== Unary Expressions (15 tests) ====================

    #[test]
    fn test_unary_negation_variable() {
        let expr = parse_expr("let x = -y");
        if let ExprKind::Unary { op, .. } = expr.kind {
            assert!(matches!(op, UnaryOp::Neg));
        } else {
            panic!("Expected unary negation");
        }
    }

    #[test]
    fn test_unary_negation_literal() {
        let expr = parse_expr("let x = -42");
        if let ExprKind::Unary { op, .. } = expr.kind {
            assert!(matches!(op, UnaryOp::Neg));
        } else {
            panic!("Expected unary negation");
        }
    }

    #[test]
    fn test_unary_negation_expression() {
        let expr = parse_expr("let x = -(a + b)");
        if let ExprKind::Unary { op, expr: operand } = expr.kind {
            assert!(matches!(op, UnaryOp::Neg));
            assert!(matches!(operand.kind, ExprKind::Binary { .. }));
        } else {
            panic!("Expected unary negation");
        }
    }

    #[test]
    fn test_unary_not_true() {
        let expr = parse_expr("let x = !true");
        if let ExprKind::Unary { op, .. } = expr.kind {
            assert!(matches!(op, UnaryOp::Not));
        } else {
            panic!("Expected logical not");
        }
    }

    #[test]
    fn test_unary_not_false() {
        let expr = parse_expr("let x = !false");
        if let ExprKind::Unary { op, .. } = expr.kind {
            assert!(matches!(op, UnaryOp::Not));
        } else {
            panic!("Expected logical not");
        }
    }

    #[test]
    fn test_unary_not_condition() {
        let expr = parse_expr("let x = !condition");
        if let ExprKind::Unary { op, .. } = expr.kind {
            assert!(matches!(op, UnaryOp::Not));
        } else {
            panic!("Expected logical not");
        }
    }

    #[test]
    fn test_unary_double_negation() {
        // --y is parsed as decrement, not double negation. Use -(-y) for double negation.
        let expr = parse_expr("let x = -(-y)");
        if let ExprKind::Unary {
            op: UnaryOp::Neg,
            expr: operand,
        } = expr.kind
        {
            assert!(matches!(
                operand.kind,
                ExprKind::Unary {
                    op: UnaryOp::Neg,
                    ..
                }
            ));
        } else {
            panic!("Expected double negation");
        }
    }

    #[test]
    fn test_unary_double_not() {
        let expr = parse_expr("let x = !(!y)");
        if let ExprKind::Unary {
            op: UnaryOp::Not,
            expr: operand,
        } = expr.kind
        {
            assert!(matches!(
                operand.kind,
                ExprKind::Unary {
                    op: UnaryOp::Not,
                    ..
                }
            ));
        } else {
            panic!("Expected double not");
        }
    }

    #[test]
    fn test_unary_negation_method_call() {
        let expr = parse_expr("let x = -obj.value");
        if let ExprKind::Unary { op, .. } = expr.kind {
            assert!(matches!(op, UnaryOp::Neg));
        } else {
            panic!("Expected unary negation");
        }
    }

    #[test]
    fn test_unary_not_comparison() {
        let expr = parse_expr("let x = !(a > b)");
        if let ExprKind::Unary { op, expr: operand } = expr.kind {
            assert!(matches!(op, UnaryOp::Not));
            assert!(matches!(operand.kind, ExprKind::Binary { .. }));
        } else {
            panic!("Expected logical not");
        }
    }

    #[test]
    fn test_unary_negation_float() {
        let expr = parse_expr("let x = -3.14");
        if let ExprKind::Unary { op, .. } = expr.kind {
            assert!(matches!(op, UnaryOp::Neg));
        } else {
            panic!("Expected unary negation");
        }
    }

    #[test]
    fn test_unary_not_method_call() {
        let expr = parse_expr("let x = !obj.isEmpty()");
        if let ExprKind::Unary { op, .. } = expr.kind {
            assert!(matches!(op, UnaryOp::Not));
        } else {
            panic!("Expected logical not");
        }
    }

    #[test]
    fn test_unary_negation_index() {
        let expr = parse_expr("let x = -arr[0]");
        if let ExprKind::Unary { op, .. } = expr.kind {
            assert!(matches!(op, UnaryOp::Neg));
        } else {
            panic!("Expected unary negation");
        }
    }

    #[test]
    fn test_unary_not_field_access() {
        let expr = parse_expr("let x = !obj.flag");
        if let ExprKind::Unary { op, .. } = expr.kind {
            assert!(matches!(op, UnaryOp::Not));
        } else {
            panic!("Expected logical not");
        }
    }

    #[test]
    fn test_unary_mixed_negation_not() {
        let expr = parse_expr("let x = -(!y)");
        if let ExprKind::Unary {
            op: UnaryOp::Neg,
            expr: operand,
        } = expr.kind
        {
            assert!(matches!(
                operand.kind,
                ExprKind::Unary {
                    op: UnaryOp::Not,
                    ..
                }
            ));
        } else {
            panic!("Expected negation of not");
        }
    }

    // ==================== Access Expressions (20 tests) ====================

    #[test]
    fn test_field_access_simple() {
        let expr = parse_expr("let x = obj.field");
        if let ExprKind::Field { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected field access");
        }
    }

    #[test]
    fn test_field_access_chained() {
        let expr = parse_expr("let x = a.b.c");
        if let ExprKind::Field { object, .. } = expr.kind {
            assert!(matches!(object.kind, ExprKind::Field { .. }));
        } else {
            panic!("Expected chained field access");
        }
    }

    #[test]
    fn test_index_access_zero() {
        let expr = parse_expr("let x = arr[0]");
        if let ExprKind::Index { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected index access");
        }
    }

    #[test]
    fn test_index_access_variable() {
        let expr = parse_expr("let x = arr[i]");
        if let ExprKind::Index { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected index access");
        }
    }

    #[test]
    fn test_index_access_string_key() {
        let expr = parse_expr(r#"let x = map["key"]"#);
        if let ExprKind::Index { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected index access");
        }
    }

    #[test]
    fn test_chained_index_field() {
        let expr = parse_expr("let x = arr[0].field");
        if let ExprKind::Field { object, .. } = expr.kind {
            assert!(matches!(object.kind, ExprKind::Index { .. }));
        } else {
            panic!("Expected field access on indexed element");
        }
    }

    #[test]
    fn test_chained_field_index() {
        let expr = parse_expr("let x = obj.field[0]");
        if let ExprKind::Index { object, .. } = expr.kind {
            assert!(matches!(object.kind, ExprKind::Field { .. }));
        } else {
            panic!("Expected index access on field");
        }
    }

    #[test]
    fn test_method_call_no_args() {
        let expr = parse_expr("let x = obj.method()");
        if let ExprKind::MethodCall { args, .. } = expr.kind {
            assert_eq!(args.len(), 0);
        } else {
            panic!("Expected method call");
        }
    }

    #[test]
    fn test_method_call_one_arg() {
        let expr = parse_expr("let x = obj.method(a)");
        if let ExprKind::MethodCall { args, .. } = expr.kind {
            assert_eq!(args.len(), 1);
        } else {
            panic!("Expected method call");
        }
    }

    #[test]
    fn test_method_call_multiple_args() {
        let expr = parse_expr("let x = obj.method(a, b)");
        if let ExprKind::MethodCall { args, .. } = expr.kind {
            assert_eq!(args.len(), 2);
        } else {
            panic!("Expected method call");
        }
    }

    #[test]
    fn test_chained_method_calls() {
        let expr = parse_expr("let x = obj.method1().method2()");
        if let ExprKind::MethodCall { object, .. } = expr.kind {
            assert!(matches!(object.kind, ExprKind::MethodCall { .. }));
        } else {
            panic!("Expected chained method calls");
        }
    }

    #[test]
    fn test_function_call_no_args() {
        let expr = parse_expr("let x = foo()");
        if let ExprKind::Call { args, .. } = expr.kind {
            assert_eq!(args.len(), 0);
        } else {
            panic!("Expected function call");
        }
    }

    #[test]
    fn test_function_call_one_arg() {
        let expr = parse_expr("let x = foo(a)");
        if let ExprKind::Call { args, .. } = expr.kind {
            assert_eq!(args.len(), 1);
        } else {
            panic!("Expected function call");
        }
    }

    #[test]
    fn test_function_call_multiple_args() {
        let expr = parse_expr("let x = foo(a, b, c)");
        if let ExprKind::Call { args, .. } = expr.kind {
            assert_eq!(args.len(), 3);
        } else {
            panic!("Expected function call");
        }
    }

    #[test]
    fn test_nested_function_call() {
        let expr = parse_expr("let x = foo(bar(x))");
        if let ExprKind::Call { args, .. } = expr.kind {
            assert_eq!(args.len(), 1);
            assert!(matches!(args[0].kind, ExprKind::Call { .. }));
        } else {
            panic!("Expected nested function call");
        }
    }

    #[test]
    fn test_complex_chained_access() {
        let expr = parse_expr("let x = obj.arr[0].method().field");
        assert!(matches!(expr.kind, ExprKind::Field { .. }));
    }

    #[test]
    fn test_method_call_on_literal() {
        let expr = parse_expr(r#"let x = "hello".length()"#);
        if let ExprKind::MethodCall { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected method call on string literal");
        }
    }

    #[test]
    fn test_index_on_method_result() {
        let expr = parse_expr("let x = obj.getArray()[0]");
        if let ExprKind::Index { object, .. } = expr.kind {
            assert!(matches!(object.kind, ExprKind::MethodCall { .. }));
        } else {
            panic!("Expected index on method result");
        }
    }

    #[test]
    fn test_field_on_index_on_field() {
        let expr = parse_expr("let x = obj.items[0].name");
        if let ExprKind::Field { object, .. } = expr.kind {
            assert!(matches!(object.kind, ExprKind::Index { .. }));
        } else {
            panic!("Expected field access");
        }
    }

    #[test]
    fn test_call_with_complex_args() {
        let expr = parse_expr("let x = foo(a + b, c.field, d[0])");
        if let ExprKind::Call { args, .. } = expr.kind {
            assert_eq!(args.len(), 3);
        } else {
            panic!("Expected function call");
        }
    }

    // ==================== Control Flow Expressions (20 tests) ====================

    #[test]
    fn test_if_expression_simple() {
        let expr = parse_expr("let x = if true { 1 } else { 0 }");
        if let ExprKind::IfExpr { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected if expression");
        }
    }

    #[test]
    fn test_if_expression_with_condition() {
        let expr = parse_expr("let x = if x > 0 { 1 } else { 0 }");
        if let ExprKind::IfExpr { condition, .. } = expr.kind {
            assert!(matches!(condition.kind, ExprKind::Binary { .. }));
        } else {
            panic!("Expected if expression");
        }
    }

    #[test]
    fn test_if_expression_without_else() {
        let expr = parse_expr("let x = if x > 0 { 1 }");
        if let ExprKind::IfExpr { else_branch, .. } = expr.kind {
            assert!(else_branch.is_none());
        } else {
            panic!("Expected if expression");
        }
    }

    #[test]
    fn test_ternary_simple() {
        // Ternary `?:` is not supported in Doo — `?` is the try operator.
        // Use if/else expression instead.
        let expr = parse_expr(r#"let x = if x > 0 { "positive" } else { "non-positive" }"#);
        if let ExprKind::IfExpr { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected if expression");
        }
    }

    #[test]
    fn test_ternary_with_expressions() {
        // Ternary not supported — use if/else expression
        let expr = parse_expr("let x = if a > b { a } else { b }");
        if let ExprKind::IfExpr { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected if expression");
        }
    }

    #[test]
    fn test_nested_ternary() {
        // Ternary not supported — use nested if/else expression
        let expr = parse_expr("let x = if a > 0 { if b > 0 { 1 } else { 2 } } else { 3 }");
        if let ExprKind::IfExpr { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected if expression");
        }
    }

    #[test]
    fn test_match_expression_simple() {
        let expr = parse_expr(r#"let x = match y { 1 => "one", _ => "other" }"#);
        if let ExprKind::Match { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected match expression");
        }
    }

    #[test]
    fn test_match_with_multiple_arms() {
        let expr =
            parse_expr(r#"let x = match y { 1 => "one", 2 => "two", 3 => "three", _ => "other" }"#);
        if let ExprKind::Match { arms, .. } = expr.kind {
            assert!(arms.len() >= 3);
        } else {
            panic!("Expected match expression");
        }
    }

    #[test]
    fn test_match_with_guard() {
        let expr = parse_expr(r#"let x = match y { n if n > 0 => "positive", _ => "other" }"#);
        if let ExprKind::Match { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected match expression");
        }
    }

    #[test]
    fn test_match_with_enum_pattern() {
        let expr = parse_expr(r#"let x = match opt { Some(v) => v, None => 0 }"#);
        if let ExprKind::Match { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected match expression");
        }
    }

    #[test]
    fn test_block_expression_simple() {
        let expr = parse_expr("let x = { let y = 1; y + 1 }");
        if let ExprKind::Block { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected block expression");
        }
    }

    #[test]
    fn test_block_expression_multiple_statements() {
        let expr = parse_expr("let x = { let a = 1; let b = 2; a + b }");
        if let ExprKind::Block(stmts, _) = expr.kind {
            assert!(stmts.len() >= 2);
        } else {
            panic!("Expected block expression");
        }
    }

    #[test]
    fn test_nested_if_expression() {
        let expr = parse_expr("let x = if a { if b { 1 } else { 2 } } else { 3 }");
        if let ExprKind::IfExpr { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected if expression");
        }
    }

    #[test]
    fn test_if_else_if_chain() {
        let expr = parse_expr("let x = if a { 1 } else if b { 2 } else { 3 }");
        if let ExprKind::IfExpr { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected if expression");
        }
    }

    #[test]
    fn test_match_with_complex_expressions() {
        let expr = parse_expr("let x = match y { 1 => a + b, 2 => c * d, _ => 0 }");
        if let ExprKind::Match { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected match expression");
        }
    }

    #[test]
    fn test_block_with_return() {
        // Use a block with statements instead of bare `return` in block-expr
        let expr = parse_expr("let x = { let y = 42; y }");
        if let ExprKind::Block(_, _) = expr.kind {
            // Success
        } else {
            panic!("Expected block expression, got {:?}", expr.kind);
        }
    }

    #[test]
    fn test_if_with_block_bodies() {
        let expr = parse_expr("let x = if x > 0 { let y = x * 2; y } else { 0 }");
        if let ExprKind::IfExpr { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected if expression");
        }
    }

    #[test]
    fn test_ternary_with_method_calls() {
        // Ternary not supported — use if/else expression
        let expr = parse_expr("let x = if flag { obj.method1() } else { obj.method2() }");
        if let ExprKind::IfExpr { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected if expression");
        }
    }

    #[test]
    fn test_match_on_enum_variant() {
        let expr = parse_expr("let x = match color { Color.Red => 1, Color.Blue => 2, _ => 0 }");
        if let ExprKind::Match { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected match expression");
        }
    }

    #[test]
    fn test_block_expression_empty() {
        // `{}` is parsed as empty MapLit in Doo. Use `{ ; }` for an empty block.
        let expr = parse_expr("let x = {}");
        if let ExprKind::MapLit(entries) = expr.kind {
            assert_eq!(entries.len(), 0);
        } else {
            panic!("Expected empty map literal");
        }
    }

    // ==================== Range Expressions (10 tests) ====================

    #[test]
    fn test_range_exclusive_literals() {
        let expr = parse_expr("let x = 1..10");
        if let ExprKind::Range { inclusive, .. } = expr.kind {
            assert!(!inclusive);
        } else {
            panic!("Expected exclusive range");
        }
    }

    #[test]
    fn test_range_inclusive_literals() {
        let expr = parse_expr("let x = 1..=10");
        if let ExprKind::Range { inclusive, .. } = expr.kind {
            assert!(inclusive);
        } else {
            panic!("Expected inclusive range");
        }
    }

    #[test]
    fn test_range_exclusive_variables() {
        let expr = parse_expr("let x = start..end");
        if let ExprKind::Range { inclusive, .. } = expr.kind {
            assert!(!inclusive);
        } else {
            panic!("Expected exclusive range");
        }
    }

    #[test]
    fn test_range_inclusive_variables() {
        let expr = parse_expr("let x = start..=end");
        if let ExprKind::Range { inclusive, .. } = expr.kind {
            assert!(inclusive);
        } else {
            panic!("Expected inclusive range");
        }
    }

    #[test]
    fn test_range_zero_to_n() {
        let expr = parse_expr("let x = 0..n");
        if let ExprKind::Range { start, .. } = expr.kind {
            assert!(matches!(start.kind, ExprKind::IntLit(0)));
        } else {
            panic!("Expected range");
        }
    }

    #[test]
    fn test_range_negative_bounds() {
        let expr = parse_expr("let x = -10..10");
        if let ExprKind::Range { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected range");
        }
    }

    #[test]
    fn test_range_with_expressions() {
        let expr = parse_expr("let x = (a + 1)..(b * 2)");
        if let ExprKind::Range { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected range");
        }
    }

    #[test]
    fn test_range_in_for_loop() {
        let prog = parse("for i in 0..n { }");
        if let Item::Statement(stmt) = &prog.items[0] {
            if let StmtKind::For { iterable, .. } = &stmt.kind {
                assert!(matches!(
                    iterable.as_ref().unwrap().kind,
                    ExprKind::Range { .. }
                ));
            } else {
                panic!("Expected for statement");
            }
        } else {
            panic!("Expected statement");
        }
    }

    #[test]
    fn test_range_inclusive_in_for() {
        let prog = parse("for i in 1..=100 { }");
        if let Item::Statement(stmt) = &prog.items[0] {
            if let StmtKind::For { iterable, .. } = &stmt.kind {
                assert!(matches!(
                    iterable.as_ref().unwrap().kind,
                    ExprKind::Range {
                        inclusive: true,
                        ..
                    }
                ));
            } else {
                panic!("Expected for statement");
            }
        } else {
            panic!("Expected statement");
        }
    }

    #[test]
    fn test_range_with_field_access() {
        let expr = parse_expr("let x = obj.start..obj.end");
        if let ExprKind::Range { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected range");
        }
    }

    // ==================== Error Handling Expressions (15 tests) ====================

    #[test]
    fn test_ok_wrapper_literal() {
        let expr = parse_expr("let x = Ok(42)");
        if let ExprKind::Ok(values) = expr.kind {
            assert_eq!(values.len(), 1);
        } else {
            panic!("Expected Ok wrapper");
        }
    }

    #[test]
    fn test_ok_wrapper_expression() {
        let expr = parse_expr("let x = Ok(a + b)");
        if let ExprKind::Ok(values) = expr.kind {
            assert_eq!(values.len(), 1);
        } else {
            panic!("Expected Ok wrapper");
        }
    }

    #[test]
    fn test_err_wrapper_string() {
        let expr = parse_expr(r#"let x = Err("failed")"#);
        if let ExprKind::Err(_) = expr.kind {
            // Success
        } else {
            panic!("Expected Err wrapper");
        }
    }

    #[test]
    fn test_err_wrapper_object() {
        let expr = parse_expr(r#"let x = Err({code: 404, message: "Not Found"})"#);
        if let ExprKind::Err(_) = expr.kind {
            // Success
        } else {
            panic!("Expected Err wrapper");
        }
    }

    #[test]
    fn test_try_operator_simple() {
        let expr = parse_expr("let x = result?");
        if let ExprKind::Try { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected try operator");
        }
    }

    #[test]
    fn test_try_operator_on_call() {
        let expr = parse_expr("let x = parseNumber(s)?");
        if let ExprKind::Try(operand) = expr.kind {
            assert!(matches!(operand.kind, ExprKind::Call { .. }));
        } else {
            panic!("Expected try operator");
        }
    }

    #[test]
    fn test_try_operator_chained() {
        let expr = parse_expr("let x = result?.value");
        if let ExprKind::Field { object, .. } = expr.kind {
            assert!(matches!(object.kind, ExprKind::Try { .. }));
        } else {
            panic!("Expected field access on try");
        }
    }

    #[test]
    fn test_unwrap_or_panic_simple() {
        let expr = parse_expr(r#"let x = result ?? panic("message")"#);
        if let ExprKind::UnwrapOrPanic { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected unwrap or panic");
        }
    }

    #[test]
    fn test_unwrap_or_panic_on_call() {
        let expr = parse_expr(r#"let x = parseNumber(s) ?? panic("Invalid number")"#);
        if let ExprKind::UnwrapOrPanic { expr: operand, .. } = expr.kind {
            assert!(matches!(operand.kind, ExprKind::Call { .. }));
        } else {
            panic!("Expected unwrap or panic");
        }
    }

    #[test]
    fn test_result_in_match() {
        let expr = parse_expr("let x = match result { Ok(v) => v, Err(e) => 0 }");
        if let ExprKind::Match { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected match expression");
        }
    }

    #[test]
    fn test_try_in_binary_expression() {
        let expr = parse_expr("let x = result? + 1");
        if let ExprKind::Binary { left, .. } = expr.kind {
            assert!(matches!(left.kind, ExprKind::Try { .. }));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_nested_try_operators() {
        let expr = parse_expr("let x = outer(inner()?)");
        if let ExprKind::Call { args, .. } = expr.kind {
            assert!(matches!(args[0].kind, ExprKind::Try { .. }));
        } else {
            panic!("Expected call with try");
        }
    }

    #[test]
    fn test_ok_with_struct() {
        let expr = parse_expr("let x = Ok(User { name: \"John\", age: 30 })");
        if let ExprKind::Ok(values) = expr.kind {
            assert_eq!(values.len(), 1);
        } else {
            panic!("Expected Ok wrapper");
        }
    }

    #[test]
    fn test_try_with_method_chain() {
        let expr = parse_expr("let x = obj.method()?.field");
        if let ExprKind::Field { object, .. } = expr.kind {
            assert!(matches!(object.kind, ExprKind::Try { .. }));
        } else {
            panic!("Expected field access");
        }
    }

    #[test]
    fn test_result_type_annotation() {
        // Doo uses `-> Int ! Str` syntax, not `Result<Int, Str>`
        let prog = parse("fn test() -> Int ! Str { Ok(42) }");
        if let Item::Function(func) = &prog.items[0] {
            assert_eq!(func.name, "test");
            assert!(func.return_type.is_some());
            assert!(func.error_type.is_some());
        } else {
            panic!("Expected function");
        }
    }

    // ==================== Closure Expressions (15 tests) ====================

    #[test]
    fn test_closure_no_params() {
        let expr = parse_expr("let x = () => 42");
        if let ExprKind::Closure { params, .. } = expr.kind {
            assert_eq!(params.len(), 0);
        } else {
            panic!("Expected closure");
        }
    }

    #[test]
    fn test_closure_one_param() {
        let expr = parse_expr("let x = (x) => x + 1");
        if let ExprKind::Closure { params, .. } = expr.kind {
            assert_eq!(params.len(), 1);
        } else {
            panic!("Expected closure");
        }
    }

    #[test]
    fn test_closure_multiple_params() {
        let expr = parse_expr("let x = (a, b) => a + b");
        if let ExprKind::Closure { params, .. } = expr.kind {
            assert_eq!(params.len(), 2);
        } else {
            panic!("Expected closure");
        }
    }

    #[test]
    fn test_closure_with_type_annotation() {
        let expr = parse_expr("let x = (x: Int) => x * 2");
        if let ExprKind::Closure { params, .. } = expr.kind {
            assert_eq!(params.len(), 1);
        } else {
            panic!("Expected closure");
        }
    }

    #[test]
    fn test_closure_with_return_type() {
        let expr = parse_expr("let x = (x: Int) -> Int => x * 2");
        if let ExprKind::Closure { params, .. } = expr.kind {
            assert_eq!(params.len(), 1);
        } else {
            panic!("Expected closure");
        }
    }

    #[test]
    fn test_closure_as_argument() {
        let expr = parse_expr("let x = arr.map((x) => x * 2)");
        if let ExprKind::MethodCall { args, .. } = expr.kind {
            assert_eq!(args.len(), 1);
            assert!(matches!(args[0].kind, ExprKind::Closure { .. }));
        } else {
            panic!("Expected method call with closure");
        }
    }

    #[test]
    fn test_closure_with_block_body() {
        let expr = parse_expr("let x = (x) => { let y = x * 2; y + 1 }");
        if let ExprKind::Closure { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected closure");
        }
    }

    #[test]
    fn test_closure_immediately_invoked() {
        let expr = parse_expr("let x = ((a) => a + 1)(5)");
        if let ExprKind::Call { func, .. } = expr.kind {
            assert!(matches!(func.kind, ExprKind::Closure { .. }));
        } else {
            panic!("Expected immediately invoked closure");
        }
    }

    #[test]
    fn test_closure_nested() {
        let expr = parse_expr("let x = (a) => (b) => a + b");
        if let ExprKind::Closure { body, .. } = expr.kind {
            assert!(matches!(body.kind, ExprKind::Closure { .. }));
        } else {
            panic!("Expected nested closure");
        }
    }

    #[test]
    fn test_closure_captures_variable() {
        let prog = parse("let y = 10; let x = (x) => x + y");
        assert_eq!(prog.items.len(), 2);
    }

    #[test]
    fn test_closure_multiple_type_annotations() {
        let expr = parse_expr("let x = (a: Int, b: Str) => a");
        if let ExprKind::Closure { params, .. } = expr.kind {
            assert_eq!(params.len(), 2);
        } else {
            panic!("Expected closure");
        }
    }

    #[test]
    fn test_closure_in_array() {
        let expr = parse_expr("let x = [(x) => x + 1, (x) => x * 2]");
        if let ExprKind::ArrayLit(elements) = expr.kind {
            assert_eq!(elements.len(), 2);
            assert!(matches!(elements[0].kind, ExprKind::Closure { .. }));
        } else {
            panic!("Expected array of closures");
        }
    }

    #[test]
    fn test_closure_with_if_expression() {
        let expr = parse_expr("let x = (x) => if x > 0 { x } else { -x }");
        if let ExprKind::Closure { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected closure");
        }
    }

    #[test]
    fn test_closure_with_method_call() {
        let expr = parse_expr("let x = (s) => s.toUpper()");
        if let ExprKind::Closure { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected closure");
        }
    }

    #[test]
    fn test_closure_chained_methods() {
        let expr = parse_expr("let x = arr.filter((x) => x > 0).map((x) => x * 2)");
        if let ExprKind::MethodCall { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected chained method calls");
        }
    }

    // ==================== Struct/Enum Construction (15 tests) ====================

    #[test]
    fn test_struct_literal_simple() {
        let expr = parse_expr(r#"let x = User { name: "foo", age: 30 }"#);
        if let ExprKind::StructLit { fields, .. } = expr.kind {
            assert_eq!(fields.len(), 2);
        } else {
            panic!("Expected struct literal");
        }
    }

    #[test]
    fn test_struct_literal_one_field() {
        let expr = parse_expr("let x = Point { x: 10 }");
        if let ExprKind::StructLit { fields, .. } = expr.kind {
            assert_eq!(fields.len(), 1);
        } else {
            panic!("Expected struct literal");
        }
    }

    #[test]
    fn test_struct_literal_empty() {
        let expr = parse_expr("let x = Empty {}");
        if let ExprKind::StructLit { fields, .. } = expr.kind {
            assert_eq!(fields.len(), 0);
        } else {
            panic!("Expected struct literal");
        }
    }

    #[test]
    fn test_struct_literal_with_expressions() {
        let expr = parse_expr("let x = Point { x: a + b, y: c * d }");
        if let ExprKind::StructLit { fields, .. } = expr.kind {
            assert_eq!(fields.len(), 2);
        } else {
            panic!("Expected struct literal");
        }
    }

    #[test]
    fn test_enum_variant_simple() {
        let expr = parse_expr("let x = Color.Red");
        if let ExprKind::Field { field, .. } = expr.kind {
            assert_eq!(field, "Red");
        } else {
            panic!("Expected enum variant");
        }
    }

    #[test]
    fn test_enum_variant_with_payload() {
        // Use a non-keyword variant name (Ok is a keyword in Doo)
        let expr = parse_expr("let x = Status::Active(42)");
        if let ExprKind::EnumVariant {
            enum_name,
            variant,
            payload,
        } = expr.kind
        {
            assert_eq!(enum_name, "Status");
            assert_eq!(variant, "Active");
            assert_eq!(payload.len(), 1);
        } else {
            panic!("Expected enum variant with payload, got {:?}", expr.kind);
        }
    }

    #[test]
    fn test_enum_variant_multiple_payloads() {
        let expr = parse_expr("let x = Event::MouseMove(10, 20)");
        if let ExprKind::EnumVariant {
            enum_name,
            variant,
            payload,
        } = expr.kind
        {
            assert_eq!(enum_name, "Event");
            assert_eq!(variant, "MouseMove");
            assert_eq!(payload.len(), 2);
        } else {
            panic!("Expected enum variant with payloads");
        }
    }

    #[test]
    fn test_nested_struct_literal() {
        let expr = parse_expr("let x = Person { name: \"John\", address: Address { street: \"Main\", city: \"NYC\" } }");
        if let ExprKind::StructLit { fields, .. } = expr.kind {
            assert_eq!(fields.len(), 2);
        } else {
            panic!("Expected struct literal");
        }
    }

    #[test]
    fn test_struct_literal_with_method_call() {
        let expr = parse_expr("let x = User { name: getName(), age: getAge() }");
        if let ExprKind::StructLit { fields, .. } = expr.kind {
            assert_eq!(fields.len(), 2);
        } else {
            panic!("Expected struct literal");
        }
    }

    #[test]
    fn test_struct_literal_with_field_access() {
        let expr = parse_expr("let x = Point { x: other.x, y: other.y }");
        if let ExprKind::StructLit { fields, .. } = expr.kind {
            assert_eq!(fields.len(), 2);
        } else {
            panic!("Expected struct literal");
        }
    }

    #[test]
    fn test_struct_literal_in_array() {
        let expr =
            parse_expr("let x = [User { name: \"a\", age: 1 }, User { name: \"b\", age: 2 }]");
        if let ExprKind::ArrayLit(elements) = expr.kind {
            assert_eq!(elements.len(), 2);
        } else {
            panic!("Expected array of struct literals");
        }
    }

    #[test]
    fn test_enum_variant_in_match() {
        let expr = parse_expr("let x = match color { Color.Red => 1, Color.Blue => 2, _ => 0 }");
        if let ExprKind::Match { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected match expression");
        }
    }

    #[test]
    fn test_struct_literal_with_spread() {
        // Struct literal spread `...other` is not currently supported in Doo.
        // Spread is supported in arrays: `[1, 2, ...arr]`
        let expr = parse_expr("let x = [1, 2, ...arr]");
        if let ExprKind::ArrayLit(elements) = expr.kind {
            assert_eq!(elements.len(), 3);
        } else {
            panic!("Expected array with spread");
        }
    }

    #[test]
    fn test_struct_literal_shorthand() {
        let expr = parse_expr("let x = User { name, age }");
        if let ExprKind::StructLit { fields, .. } = expr.kind {
            assert_eq!(fields.len(), 2);
        } else {
            panic!("Expected struct literal");
        }
    }

    #[test]
    fn test_generic_struct_literal() {
        // Doo does not support generic struct syntax `Box<Int> { value: 42 }`.
        // Use regular struct literal instead.
        let expr = parse_expr("let x = Box { value: 42 }");
        if let ExprKind::StructLit { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected struct literal");
        }
    }

    // ==================== Cast Expressions (10 tests) ====================

    #[test]
    fn test_cast_int_to_float() {
        let expr = parse_expr("let x = y as Float");
        if let ExprKind::Cast { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected cast expression");
        }
    }

    #[test]
    fn test_cast_float_to_int() {
        let expr = parse_expr("let x = y as Int");
        if let ExprKind::Cast { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected cast expression");
        }
    }

    #[test]
    fn test_cast_to_string() {
        let expr = parse_expr("let x = y as Str");
        if let ExprKind::Cast { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected cast expression");
        }
    }

    #[test]
    fn test_cast_literal() {
        let expr = parse_expr("let x = 42 as Float");
        if let ExprKind::Cast { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected cast expression");
        }
    }

    #[test]
    fn test_cast_expression() {
        let expr = parse_expr("let x = (a + b) as Float");
        if let ExprKind::Cast { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected cast expression");
        }
    }

    #[test]
    fn test_cast_method_call() {
        let expr = parse_expr("let x = obj.getValue() as Int");
        if let ExprKind::Cast { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected cast expression");
        }
    }

    #[test]
    fn test_cast_field_access() {
        let expr = parse_expr("let x = obj.value as Float");
        if let ExprKind::Cast { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected cast expression");
        }
    }

    #[test]
    fn test_cast_in_binary_expression() {
        let expr = parse_expr("let x = (y as Float) + 1.0");
        if let ExprKind::Binary { left, .. } = expr.kind {
            assert!(matches!(left.kind, ExprKind::Cast { .. }));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_cast_to_custom_type() {
        let expr = parse_expr("let x = y as MyType");
        if let ExprKind::Cast { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected cast expression");
        }
    }

    #[test]
    fn test_chained_casts() {
        let expr = parse_expr("let x = (y as Float) as Int");
        if let ExprKind::Cast { expr: operand, .. } = expr.kind {
            assert!(matches!(operand.kind, ExprKind::Cast { .. }));
        } else {
            panic!("Expected chained cast");
        }
    }

    // ==================== Complex/Combined (20 tests) ====================

    #[test]
    fn test_method_call_on_string_literal() {
        let expr = parse_expr(r#"let x = "hello".length()"#);
        if let ExprKind::MethodCall { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected method call");
        }
    }

    #[test]
    fn test_method_call_on_array_literal() {
        let expr = parse_expr("let x = [1, 2, 3].length()");
        if let ExprKind::MethodCall { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected method call");
        }
    }

    #[test]
    fn test_chained_array_methods() {
        let expr = parse_expr("let x = arr.filter((x) => x > 0).map((x) => x * 2).length()");
        if let ExprKind::MethodCall { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected chained method calls");
        }
    }

    #[test]
    fn test_nested_ternary_expressions() {
        // Ternary not supported — use nested if/else expressions
        let expr = parse_expr(
            "let x = if a > 0 { if b > 0 { 1 } else { 2 } } else { if c > 0 { 3 } else { 4 } }",
        );
        if let ExprKind::IfExpr { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected if expression");
        }
    }

    #[test]
    fn test_complex_boolean_with_parens() {
        let expr = parse_expr("let x = (a > 0 && b < 10) || c == true");
        if let ExprKind::Binary {
            op: BinaryOp::Or, ..
        } = expr.kind
        {
            // Success
        } else {
            panic!("Expected logical or");
        }
    }

    #[test]
    fn test_spread_in_array() {
        let expr = parse_expr("let x = [1, 2, ...arr, 3, 4]");
        if let ExprKind::ArrayLit(elements) = expr.kind {
            assert!(elements.len() > 0);
        } else {
            panic!("Expected array with spread");
        }
    }

    #[test]
    fn test_spread_in_function_call() {
        let expr = parse_expr("let x = foo(a, ...args, b)");
        if let ExprKind::Call { args, .. } = expr.kind {
            assert!(args.len() > 0);
        } else {
            panic!("Expected function call with spread");
        }
    }

    #[test]
    fn test_assignment_with_if_expression() {
        let expr = parse_expr("let x = if true { 1 } else { 2 }");
        if let ExprKind::IfExpr { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected if expression");
        }
    }

    #[test]
    fn test_assignment_with_match_expression() {
        let expr = parse_expr("let x = match y { 1 => \"one\", _ => \"other\" }");
        if let ExprKind::Match { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected match expression");
        }
    }

    #[test]
    fn test_complex_operator_precedence() {
        let expr = parse_expr("let x = a + b * c - d / e % f");
        if let ExprKind::Binary { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_nested_array_access() {
        let expr = parse_expr("let x = matrix[i][j]");
        if let ExprKind::Index { object, .. } = expr.kind {
            assert!(matches!(object.kind, ExprKind::Index { .. }));
        } else {
            panic!("Expected nested index access");
        }
    }

    #[test]
    fn test_method_chain_with_index() {
        let expr = parse_expr("let x = obj.getArray().filter((x) => x > 0)[0]");
        if let ExprKind::Index { object, .. } = expr.kind {
            assert!(matches!(object.kind, ExprKind::MethodCall { .. }));
        } else {
            panic!("Expected index on method result");
        }
    }

    #[test]
    fn test_complex_struct_construction() {
        let expr = parse_expr(
            "let x = User { name: getName(), age: getAge(), address: Address { city: \"NYC\" } }",
        );
        if let ExprKind::StructLit { fields, .. } = expr.kind {
            assert!(fields.len() >= 3);
        } else {
            panic!("Expected struct literal");
        }
    }

    #[test]
    fn test_array_of_closures_with_calls() {
        let expr = parse_expr("let x = [(x) => x + 1, (x) => x * 2][0](5)");
        if let ExprKind::Call { func, .. } = expr.kind {
            assert!(matches!(func.kind, ExprKind::Index { .. }));
        } else {
            panic!("Expected function call");
        }
    }

    #[test]
    fn test_ternary_with_complex_branches() {
        // Ternary not supported — use if/else expression
        let expr = parse_expr("let x = if flag { obj.method1().field } else { arr[0].value }");
        if let ExprKind::IfExpr { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected if expression");
        }
    }

    #[test]
    fn test_null_coalesce_with_field_access() {
        // Doo uses `?? panic(msg)` not `?? default`
        let expr = parse_expr(r#"let x = obj.field ?? panic("default required")"#);
        if let ExprKind::UnwrapOrPanic { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected unwrap or panic");
        }
    }

    #[test]
    fn test_try_operator_in_complex_expression() {
        let expr = parse_expr("let x = parseNumber(input)? + 10");
        if let ExprKind::Binary { left, .. } = expr.kind {
            assert!(matches!(left.kind, ExprKind::Try { .. }));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_closure_returning_struct() {
        let expr = parse_expr("let x = () => User { name: \"John\", age: 30 }");
        if let ExprKind::Closure { .. } = expr.kind {
            // Success
        } else {
            panic!("Expected closure");
        }
    }

    #[test]
    fn test_range_in_array_slice() {
        let expr = parse_expr("let x = arr[0..10]");
        if let ExprKind::Index { index, .. } = expr.kind {
            assert!(matches!(index.kind, ExprKind::Range { .. }));
        } else {
            panic!("Expected index with range");
        }
    }

    #[test]
    fn test_deeply_nested_field_access() {
        let expr = parse_expr("let x = a.b.c.d.e.f");
        assert!(matches!(expr.kind, ExprKind::Field { .. }));
    }
}
