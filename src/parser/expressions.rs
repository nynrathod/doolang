use crate::lexer::token::TokenType;
use crate::limits::{PARSER_MAX_ARRAY_SIZE, PARSER_MAX_DEPTH, PARSER_MAX_MAP_SIZE};
use crate::parser::ast::AstNode;
use crate::parser::{ParseError, ParseResult, Parser};

impl<'a> Parser<'a> {
    /// Entry point for parsing any expression.
    /// Delegates to precedence-based parser.
    pub fn parse_expression(&mut self) -> ParseResult<AstNode> {
        self.depth += 1;
        if self.depth > PARSER_MAX_DEPTH {
            self.depth -= 1;
            return Err(ParseError::UnexpectedTokenAt {
                msg: "Expression nesting too deep (recursion limit exceeded)".to_string(),
                line: self.peek().map(|t| t.line).unwrap_or(0),
                col: self.peek().map(|t| t.col).unwrap_or(0),
            });
        }
        let result = self.parse_expression_prec(0);
        self.depth -= 1;
        result
    }

    /// Parses an expression with operator precedence.
    /// Uses precedence climbing for correct operator grouping.
    /// - `min_prec`: minimum precedence to consider (used for recursion).
    /// Returns the parsed AST node for the expression.
    fn parse_expression_prec(&mut self, min_prec: u8) -> ParseResult<AstNode> {
        // First parse unary and primary (postfix is now handled inside parse_unary_primary
        // so that method calls bind tighter than unary operators)
        let mut left = self.parse_unary_primary()?;

        // Apply postfix operations for unary expressions
        // This handles cases like (-42) as Str
        left = self.parse_postfix(left)?;

        // Binary operator expressions:
        // Handles: a + b, x * y - z, a < b, a <= b, a > b, a >= b
        // Groups operators according to precedence and left-to-right associativity.
        while let Some(tok) = self.peek() {
            // Get the precedence of the current operator token
            let prec = Self::get_precedence(tok.kind);

            // If the operator's precedence is lower than the minimum required,
            // or if it's not an operator (prec == 0), stop parsing further binary operators
            if prec < min_prec || prec == 0 {
                break;
            }

            let op = tok.kind;
            self.advance();

            // Recursively parse the right-hand side of the expression,
            // using higher precedence to ensure correct grouping
            let right = self.parse_expression_prec(prec + 1)?;

            // Build a BinaryExpr AST node with the current left and right expressions
            left = AstNode::BinaryExpr {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        // Handle ternary, try-propagate, and unwrap-or-panic at lowest precedence (after all binary operators)
        // Only parse when at minimum precedence level (0) to ensure correct precedence
        // This ensures: x > y ? a : b parses as (x > y) ? a : b

        // Check for ?? (unwrap or panic operator) FIRST since lexer tokenizes ?? as DoubleQuestion
        if min_prec == 0 && self.peek_is(TokenType::DoubleQuestion) {
            self.advance(); // consume '??'

            // Parse the fallback expression (typically panic("message"))
            let fallback = self.parse_expression_prec(0)?;

            left = AstNode::UnwrapOrPanic {
                expr: Box::new(left),
                panic_msg: Box::new(fallback),
            };
        } else if min_prec == 0 && self.peek_is(TokenType::Question) {
            self.advance(); // consume '?'

            // Distinguish between ternary and try-propagate by looking ahead
            if self.peek_is(TokenType::Semi)
                || self.peek_is(TokenType::CloseParen)
                || self.peek_is(TokenType::CloseBracket)
                || self.peek_is(TokenType::CloseBrace)
                || self.peek_is(TokenType::Comma)
            {
                // This is try propagate: expr?
                left = AstNode::TryPropagate {
                    expr: Box::new(left),
                };
            } else {
                // This is ternary: condition ? true_expr : false_expr
                let true_expr = self.parse_expression_prec(0)?;
                self.expect(TokenType::Colon)?;
                let false_expr = self.parse_expression_prec(0)?;
                left = AstNode::TernaryExpr {
                    condition: Box::new(left),
                    true_expr: Box::new(true_expr),
                    false_expr: Box::new(false_expr),
                };
            }
        }

        Ok(left)
    }

    /// Parse unary operators and primary expressions.
    /// Unary operators (-, !, +) are right-associative, so !!x is parsed as !(!(x))
    /// Method calls and field access bind tighter than unary operators.
    /// So !t.IsDone() is parsed as !(t.IsDone()), not (!t).IsDone()
    fn parse_unary_primary(&mut self) -> ParseResult<AstNode> {
        if let Some(tok) = self.peek() {
            match tok.kind {
                // Allow unary operators: !, -, +
                TokenType::Bang | TokenType::Minus | TokenType::Plus => {
                    let op = tok.kind;
                    self.advance(); // consume operator
                                    // Recursively parse unary for chaining: !!x, ---x, etc.
                                    // The recursive call handles the operand, which may include postfix operations
                    let expr = self.parse_unary_primary()?;
                    Ok(AstNode::UnaryExpr {
                        op,
                        expr: Box::new(expr),
                    })
                }
                // Primary expressions:
                // Handles: number, identifier, function call foo(a + b), string, boolean, array, map
                // Apply postfix operations here so method calls bind tighter than unary
                _ => {
                    let primary = self.parse_primary()?;
                    self.parse_postfix(primary)
                }
            }
        } else {
            Err(ParseError::EndOfInput)
        }
    }

    /// Parses postfix operations on an expression.
    /// Handles array/map element access: arr[0], map["key"], nested[i][j]
    /// Also handles method calls: obj.method(args)
    /// Can be chained: arr[0][1][2] or obj.method1().method2()
    fn parse_postfix(&mut self, mut expr: AstNode) -> ParseResult<AstNode> {
        loop {
            if self.peek_is(TokenType::OpenBracket) {
                if self.depth >= PARSER_MAX_DEPTH {
                    return Err(ParseError::UnexpectedToken(
                        "Expression too deeply nested".to_string(),
                    ));
                }
                self.advance(); // consume '['

                // Check for slicing: [start..end] or [..end] or [start..]
                // Also handle negative indexing: [-1], [-2]
                let index = self.parse_expression()?;

                self.expect(TokenType::CloseBracket)?;

                // Convert BinaryExpr with range operators to Range node for slicing
                let index = match index {
                    AstNode::BinaryExpr { op, left, right }
                        if op == TokenType::RangeExc || op == TokenType::RangeInc =>
                    {
                        AstNode::Range {
                            start: left,
                            end: right,
                            inclusive: op == TokenType::RangeInc,
                        }
                    }
                    other => other,
                };

                expr = AstNode::ElementAccess {
                    array: Box::new(expr),
                    index: Box::new(index),
                };
            } else if self.peek_is(TokenType::Dot) {
                if self.depth >= PARSER_MAX_DEPTH {
                    return Err(ParseError::UnexpectedToken(
                        "Expression too deeply nested".to_string(),
                    ));
                }
                self.advance(); // consume '.'
                let field_tok = self.expect(TokenType::Identifier)?;
                let field_name = field_tok.value.to_string();

                if self.peek_is(TokenType::OpenParen) {
                    // Method call: obj.method(args)
                    self.advance(); // consume '('
                    let args = self
                        .parse_comma_separated(|p| p.parse_expression(), TokenType::CloseParen)?;
                    self.expect(TokenType::CloseParen)?;
                    expr = AstNode::MethodCall {
                        object: Box::new(expr),
                        method: field_name,
                        args,
                    };
                } else {
                    // Field access: obj.field
                    expr = AstNode::FieldAccess {
                        object: Box::new(expr),
                        field: field_name,
                    };
                }
            } else if self.peek_is(TokenType::As) {
                // Handle type casting: expr as Int, expr as Float, expr as String
                self.advance(); // consume 'as'
                let type_tok = self.expect(TokenType::Identifier)?;
                let type_str = type_tok.value.to_string();

                let target_type = match type_str.as_str() {
                    "Int" => crate::parser::ast::TypeNode::Int,
                    "Float" => crate::parser::ast::TypeNode::Float,
                    "Str" => crate::parser::ast::TypeNode::String,
                    "Bool" => crate::parser::ast::TypeNode::Bool,
                    _ => {
                        return Err(ParseError::UnexpectedTokenAt {
                            msg: format!("Unsupported cast target type: {}", type_str),
                            line: type_tok.line,
                            col: type_tok.col,
                        });
                    }
                };

                expr = AstNode::Cast {
                    expr: Box::new(expr),
                    target_type,
                };
            } else if self.peek_is(TokenType::PlusPlus) || self.peek_is(TokenType::MinusMinus) {
                // Handle postfix increment/decrement: x++, x--
                let op = self.peek().unwrap().kind;
                self.advance(); // consume ++ or --

                // Extract identifier from expr for increment/decrement
                if let AstNode::Identifier(name) = expr {
                    expr = AstNode::IncrementDecrement { variable: name, op };
                } else {
                    return Err(ParseError::UnexpectedToken(
                        "Only single-variable increment/decrement is allowed".into(),
                    ));
                }
            } else {
                break;
            }
        }
        Ok(expr)
    }

    /// Handles literals (number, string, boolean), identifiers
    /// function calls, arrays, and maps.
    fn parse_primary(&mut self) -> ParseResult<AstNode> {
        if let Some(tok) = self.peek() {
            let tok_kind = tok.kind;
            let tok_line = tok.line;
            let tok_col = tok.col;
            match tok_kind {
                TokenType::Number => {
                    let tok = self.advance().unwrap();
                    match tok.value.parse::<i32>() {
                        Ok(num) => Ok(AstNode::NumberLiteral(num)),
                        Err(e) => Err(ParseError::UnexpectedTokenAt {
                            msg: format!("Invalid integer literal: {}", e),
                            line: tok.line,
                            col: tok.col,
                        }),
                    }
                }
                TokenType::Float => {
                    let tok = self.advance().unwrap();
                    match tok.value.parse::<f64>() {
                        Ok(num) => Ok(AstNode::FloatLiteral(num)),
                        Err(e) => Err(ParseError::UnexpectedTokenAt {
                            msg: format!("Invalid float literal: {}", e),
                            line: tok.line,
                            col: tok.col,
                        }),
                    }
                }
                TokenType::Identifier => {
                    let tok = self.advance().unwrap();
                    let name = tok.value.to_string();

                    // If followed by '::', parse as EnumVariant (could be enum variant or namespaced function)
                    // The analyzer will determine if it's actually an enum or a function call
                    // This allows both Status::Pending(25) and File::Write(...) to work correctly
                    if self.peek_is(TokenType::ColonColon) {
                        self.advance(); // consume '::'
                        let second_tok = self.expect(TokenType::Identifier)?;
                        let second_name = second_tok.value.to_string();

                        // Check for payload/arguments in parentheses
                        let payload = if self.peek_is(TokenType::OpenParen) {
                            self.advance(); // consume '('
                            let args = if !self.peek_is(TokenType::CloseParen) {
                                self.parse_comma_separated(
                                    |p| p.parse_expression(),
                                    TokenType::CloseParen,
                                )?
                            } else {
                                vec![]
                            };
                            self.expect(TokenType::CloseParen)?;
                            args
                        } else {
                            vec![]
                        };

                        // Return as EnumVariant - analyzer will check if it's enum or function
                        return Ok(AstNode::EnumVariant {
                            enum_name: name,
                            variant: second_name,
                            payload,
                        });
                    }

                    // If followed by '(', parse as function call
                    if self.peek_is(TokenType::OpenParen) {
                        self.advance(); // consume '('
                        let args = self.parse_comma_separated(
                            |p| p.parse_expression(),
                            TokenType::CloseParen,
                        )?;
                        self.expect(TokenType::CloseParen)?;
                        return Ok(AstNode::FunctionCall {
                            func: Box::new(AstNode::Identifier(name)),
                            args,
                        });
                    }

                    // If followed by '{', parse as struct literal
                    // BUT only if the identifier starts with uppercase (struct types use PascalCase)
                    // This prevents parsing `flag { ... }` as a struct when it's an expression + block
                    if self.peek_is(TokenType::OpenBrace)
                        && name
                            .chars()
                            .next()
                            .map(|c| c.is_uppercase())
                            .unwrap_or(false)
                    {
                        self.advance(); // consume '{'
                        let mut fields = Vec::new();

                        // Parse field: value pairs, supporting shorthand syntax
                        while !self.peek_is(TokenType::CloseBrace) {
                            let field_tok = self.expect(TokenType::Identifier)?;
                            let field_name = field_tok.value.to_string();

                            // Check for shorthand: {name, age} instead of {name: name, age: age}
                            if self.peek_is(TokenType::Comma) || self.peek_is(TokenType::CloseBrace)
                            {
                                // Shorthand syntax - field name is also variable name
                                fields.push((
                                    field_name.clone(),
                                    Box::new(AstNode::Identifier(field_name)),
                                ));
                            } else if self.peek_is(TokenType::Colon) {
                                // Explicit field: value syntax
                                self.advance(); // consume ':'
                                let value = self.parse_expression()?;
                                fields.push((field_name, Box::new(value)));
                            } else {
                                return Err(ParseError::UnexpectedTokenAt {
                                    msg: format!(
                                        "Expected ':' or ',' after field name in struct literal"
                                    ),
                                    line: self.peek().map(|t| t.line).unwrap_or(0),
                                    col: self.peek().map(|t| t.col).unwrap_or(0),
                                });
                            }

                            if !self.peek_is(TokenType::CloseBrace) {
                                self.expect(TokenType::Comma)?;
                            }
                        }
                        self.expect(TokenType::CloseBrace)?;
                        return Ok(AstNode::StructLiteral { name, fields });
                    }

                    Ok(AstNode::Identifier(name))
                }
                TokenType::String => {
                    let tok = self.advance().unwrap();
                    let raw_value = tok.value.to_string();

                    // Check for string interpolation ${...}
                    if raw_value.contains("${") {
                        // Parse string interpolation and convert to concatenation
                        Ok(self.parse_string_interpolation(&raw_value)?)
                    } else {
                        // Process escape sequences
                        let string_value = Parser::process_escape_sequences(&raw_value);
                        Ok(AstNode::StringLiteral(string_value))
                    }
                }
                TokenType::Boolean => {
                    let tok = self.advance().unwrap();
                    let value = tok.value == "true";
                    Ok(AstNode::BoolLiteral(value))
                }
                TokenType::Nil => {
                    self.advance(); // consume 'nil'
                    Ok(AstNode::NilLiteral)
                }
                TokenType::OpenBracket => self.parse_array_literal(),
                TokenType::OpenBrace => self.parse_map_literal(),
                TokenType::If => {
                    // Inline if-else expression: if condition { stmts; expr } else { stmts; expr }
                    // Also supports else-if chains: if cond { expr } else if cond { expr } else { expr }
                    // Supports multiple statements (with semicolons) followed by a final expression (no semicolon)
                    self.advance(); // consume 'if'
                    let condition = self.parse_expression()?;

                    // Parse then block expression
                    let then_expr = self.parse_block_expr()?;

                    // Parse else branch (or else-if chain)
                    self.expect(TokenType::Else)?;
                    let else_expr = self.parse_inline_else_branch()?;

                    Ok(AstNode::ConditionalExpr {
                        condition: Box::new(condition),
                        then_expr: Box::new(then_expr),
                        else_expr: Box::new(else_expr),
                    })
                }
                TokenType::OpenParen => {
                    // Check if this is an arrow function () => or (x) =>
                    if self.is_arrow_function() {
                        self.parse_arrow_closure()
                    } else {
                        // Otherwise, treat as grouping parentheses: (expr)
                        self.advance(); // consume '('
                        let expr = self.parse_expression()?;
                        self.expect(TokenType::CloseParen)?;
                        Ok(expr)
                    }
                }
                TokenType::Ok => {
                    self.advance(); // consume 'Ok'
                                    // Ok values - parentheses are optional but not recommended
                                    // Ok 42  or  Ok(42)  or  Ok 10, 20  or  Ok(10, 20)
                    let mut values = Vec::new();

                    if self.peek_is(TokenType::OpenParen) {
                        // With parentheses: Ok(values)
                        self.advance(); // consume '('
                        if !self.peek_is(TokenType::CloseParen) {
                            values = self.parse_comma_separated(
                                |p| p.parse_expression(),
                                TokenType::CloseParen,
                            )?;
                        }
                        self.expect(TokenType::CloseParen)?;
                    } else {
                        // Without parentheses: Ok values
                        // Parse comma-separated values until semicolon or other delimiter
                        values.push(self.parse_expression()?);
                        while self.peek_is(TokenType::Comma) {
                            self.advance(); // consume ','
                            values.push(self.parse_expression()?);
                        }
                    }

                    Ok(AstNode::OkExpr { values })
                }
                TokenType::Err => {
                    self.advance(); // consume 'Err'
                                    // Err value - parentheses are optional but not recommended
                                    // Err "message"  or  Err("message")
                    let value = if self.peek_is(TokenType::OpenParen) {
                        // With parentheses: Err(value)
                        self.advance(); // consume '('
                        let expr = self.parse_expression()?;
                        self.expect(TokenType::CloseParen)?;
                        expr
                    } else {
                        // Without parentheses: Err value
                        self.parse_expression()?
                    };

                    Ok(AstNode::ErrExpr {
                        value: Box::new(value),
                    })
                }
                TokenType::Match => {
                    // Parse match expression
                    self.parse_match_expr()
                }
                _ => Err(ParseError::UnexpectedTokenAt {
                    msg: format!("Expected primary expression, got {:?}", tok_kind),
                    line: tok_line,
                    col: tok_col,
                }),
            }
        } else {
            Err(ParseError::EndOfInput)
        }
    }

    /// Example: `[1, 2, 3]` or `[...arr1, 4, 5]`
    /// Uses parse_comma_separated to parse elements until ']'.
    /// Supports spread operator: [...arr]
    fn parse_array_literal(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::OpenBracket)?;

        let mut elements = Vec::new();

        // Parse elements, checking for spread operator
        while !self.peek_is(TokenType::CloseBracket) {
            if self.peek_is(TokenType::Spread) {
                // Spread operator: ...expr
                self.advance(); // consume '...'
                let spread_expr = self.parse_expression()?;
                elements.push(AstNode::SpreadElement(Box::new(spread_expr)));
            } else {
                elements.push(self.parse_expression()?);
            }

            if !self.peek_is(TokenType::CloseBracket) {
                self.expect(TokenType::Comma)?;
            }
        }

        // Validate array size doesn't exceed limit
        if elements.len() > PARSER_MAX_ARRAY_SIZE {
            return Err(ParseError::UnexpectedTokenAt {
                msg: format!(
                    "Array literal exceeds maximum size of {}",
                    PARSER_MAX_ARRAY_SIZE
                ),
                line: self.peek().map(|t| t.line).unwrap_or(0),
                col: self.peek().map(|t| t.col).unwrap_or(0),
            });
        }

        self.expect(TokenType::CloseBracket)?;
        Ok(AstNode::ArrayLiteral(elements))
    }

    /// Parses a map/dictionary literal.
    /// Example: `{ "a": 1, "b": 2 }` or `{...obj, "c": 3}`
    /// Each entry is a key-value pair separated by ':' and entries separated by ','.
    /// Supports spread operator: {...map}
    fn parse_map_literal(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::OpenBrace)?;

        let mut entries = Vec::new();

        // Parse entries, checking for spread operator
        while !self.peek_is(TokenType::CloseBrace) {
            if self.peek_is(TokenType::Spread) {
                // Spread operator: ...expr
                self.advance(); // consume '...'
                let spread_expr = self.parse_expression()?;
                // Store spread as a special entry with SpreadElement as key
                entries.push((
                    AstNode::SpreadElement(Box::new(spread_expr)),
                    AstNode::NilLiteral,
                ));
            } else {
                let key = self.parse_expression()?;
                self.expect(TokenType::Colon)?;
                let value = self.parse_expression()?;
                entries.push((key, value));
            }

            if !self.peek_is(TokenType::CloseBrace) {
                self.expect(TokenType::Comma)?;
            }
        }

        // Validate map size doesn't exceed limit
        if entries.len() > PARSER_MAX_MAP_SIZE {
            return Err(ParseError::UnexpectedTokenAt {
                msg: format!(
                    "Map literal exceeds maximum size of {}",
                    PARSER_MAX_MAP_SIZE
                ),
                line: self.peek().map(|t| t.line).unwrap_or(0),
                col: self.peek().map(|t| t.col).unwrap_or(0),
            });
        }

        self.expect(TokenType::CloseBrace)?;
        Ok(AstNode::MapLiteral(entries))
    }

    /// Returns the precedence value for a given operator token.
    /// Higher numbers mean higher precedence.
    /// Used in precedence climbing for binary expressions.
    fn get_precedence(op: TokenType) -> u8 {
        match op {
            TokenType::OrOr => 1,
            TokenType::AndAnd => 2,
            TokenType::EqEq | TokenType::NotEq => 3,
            TokenType::Lt | TokenType::Gt | TokenType::LtEq | TokenType::GtEq | TokenType::In => 4,
            TokenType::Plus | TokenType::Minus => 5,
            TokenType::Star | TokenType::Slash | TokenType::Percent => 6,
            TokenType::RangeExc | TokenType::RangeInc => 7, // Add range operators with lowest precedence
            _ => 0,
        }
    }

    /// Check if current position is an arrow function: () => or (x) => or (x, y) =>
    fn is_arrow_function(&mut self) -> bool {
        let saved_pos = self.current;

        // Must start with (
        if !self.peek_is(TokenType::OpenParen) {
            self.current = saved_pos;
            return false;
        }
        self.advance(); // consume (

        // Skip parameters until we find )
        let mut depth = 1;
        while depth > 0 && self.current < self.tokens.len() {
            if self.peek_is(TokenType::OpenParen) {
                depth += 1;
            } else if self.peek_is(TokenType::CloseParen) {
                depth -= 1;
            }
            if depth > 0 {
                self.advance();
            }
        }

        if self.current >= self.tokens.len() {
            self.current = saved_pos;
            return false;
        }

        self.advance(); // consume )

        // Check for =>
        let result = self.peek_is(TokenType::FatArrow);
        self.current = saved_pos;
        result
    }

    /// Parse arrow function: () => expr or (x) => expr or (x, y) => expr
    fn parse_arrow_closure(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::OpenParen)?;

        let mut params = Vec::new();

        // Parse parameters
        if !self.peek_is(TokenType::CloseParen) {
            loop {
                let param_tok = self.expect(TokenType::Identifier)?;
                let param_name = param_tok.value.to_string();

                // Check for optional type annotation
                let param_type = if self.peek_is(TokenType::Colon) {
                    self.advance(); // consume ':'
                    Some(self.parse_type_annotation()?)
                } else {
                    None
                };

                params.push((param_name, param_type));

                if self.peek_is(TokenType::Comma) {
                    self.advance(); // consume ','
                } else {
                    break;
                }
            }
        }

        self.expect(TokenType::CloseParen)?;
        self.expect(TokenType::FatArrow)?;

        // Parse optional return type annotation
        let return_type = if self.peek_is(TokenType::Arrow) {
            self.advance(); // consume '->'
            Some(self.parse_type_annotation()?)
        } else {
            None
        };

        // Parse body - either a single expression or a block
        let body = if self.peek_is(TokenType::OpenBrace) {
            let statements = self.parse_braced_block()?;
            Box::new(AstNode::Block(statements))
        } else {
            Box::new(self.parse_expression()?)
        };

        Ok(AstNode::Closure {
            params,
            body,
            return_type,
        })
    }

    /// Parse string interpolation: "Hello ${name}!" -> "Hello " + name + "!"
    fn parse_string_interpolation(&mut self, template: &str) -> ParseResult<AstNode> {
        let mut result: Option<AstNode> = None;
        let chars: Vec<char> = template.chars().collect();
        let mut current_pos = 0;

        while current_pos < chars.len() {
            // Find next ${ using character positions
            let remaining: String = chars[current_pos..].iter().collect();
            if let Some(start) = remaining.find("${") {
                // Convert byte position to character position
                let byte_start = start;
                let char_start = remaining[..byte_start].chars().count();
                let abs_start = current_pos + char_start;

                // Add the literal part before ${
                if char_start > 0 {
                    let literal = chars[current_pos..abs_start].iter().collect::<String>();
                    let literal_node = AstNode::StringLiteral(literal);
                    result = Some(if let Some(prev) = result {
                        AstNode::BinaryExpr {
                            op: crate::lexer::token::TokenType::Plus,
                            left: Box::new(prev),
                            right: Box::new(literal_node),
                        }
                    } else {
                        literal_node
                    });
                }

                // Find matching }
                let expr_start = abs_start + 2;
                let mut brace_count = 1;
                let mut expr_end = expr_start;

                while expr_end < chars.len() && brace_count > 0 {
                    if chars[expr_end] == '{' {
                        brace_count += 1;
                    } else if chars[expr_end] == '}' {
                        brace_count -= 1;
                    }
                    if brace_count > 0 {
                        expr_end += 1;
                    }
                }

                if brace_count != 0 {
                    return Err(ParseError::UnexpectedToken(
                        "Unclosed ${} in string interpolation".to_string(),
                    ));
                }

                // Parse the expression inside ${}
                let expr_str: String = chars[expr_start..expr_end].iter().collect();

                // Create a temporary parser for the expression
                let arena = bumpalo::Bump::new();
                let expr_tokens = crate::lexer::lexer::lex(&expr_str, &arena);
                let mut expr_parser = Parser::new(&expr_tokens);
                let expr_node = expr_parser.parse_expression()?;

                // Add the expression to result
                result = Some(if let Some(prev) = result {
                    AstNode::BinaryExpr {
                        op: crate::lexer::token::TokenType::Plus,
                        left: Box::new(prev),
                        right: Box::new(expr_node),
                    }
                } else {
                    expr_node
                });

                current_pos = expr_end + 1;
            } else {
                // No more ${}, add remaining literal
                let literal: String = chars[current_pos..].iter().collect();
                if !literal.is_empty() {
                    let literal_node = AstNode::StringLiteral(literal);
                    result = Some(if let Some(prev) = result {
                        AstNode::BinaryExpr {
                            op: crate::lexer::token::TokenType::Plus,
                            left: Box::new(prev),
                            right: Box::new(literal_node),
                        }
                    } else {
                        literal_node
                    });
                }
                break;
            }
        }

        result.ok_or(ParseError::UnexpectedToken(
            "Empty string interpolation".to_string(),
        ))
    }

    /// Helper method to parse the else branch of an inline if-else expression.
    /// Handles both `else { expr }` and `else if ...` chains.
    /// Supports both expressions (no semicolon) and statements (with semicolon).
    /// Returns the else expression AST node.
    fn parse_inline_else_branch(&mut self) -> ParseResult<AstNode> {
        if self.peek_is(TokenType::If) {
            // else if: recursively parse another conditional expression
            self.advance(); // consume 'if'
            let condition = self.parse_expression()?;

            // Parse then block expression
            let then_expr = self.parse_block_expr()?;

            // Recursively parse the else branch (might be else if or else)
            self.expect(TokenType::Else)?;
            let else_expr = self.parse_inline_else_branch()?;

            Ok(AstNode::ConditionalExpr {
                condition: Box::new(condition),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
            })
        } else {
            // else: parse the final block expression
            self.parse_block_expr()
        }
    }

    /// Parse a block expression: { statements; final_expr }
    /// Supports multiple statements (with semicolons) followed by a final expression (no semicolon).
    /// Returns either a BlockExpr (if there are statements) or just the expression (if no statements).
    fn parse_block_expr(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::OpenBrace)?;

        let mut statements = Vec::new();

        // Keep parsing statements until we hit the closing brace
        loop {
            // Check for closing brace
            if self.peek_is(TokenType::CloseBrace) {
                self.advance(); // consume '}'
                                // Empty block or block with only statements - return unit/last statement
                if statements.is_empty() {
                    // Empty block - return a nil literal as default
                    return Ok(AstNode::NilLiteral);
                } else {
                    // Block ended with semicolon, last statement is not an expression
                    // Return BlockExpr with nil as result
                    return Ok(AstNode::BlockExpr {
                        statements,
                        result: Box::new(AstNode::NilLiteral),
                    });
                }
            }

            // Try to parse a statement or expression
            let item = self.parse_block_item()?;

            // Check if followed by semicolon
            if self.peek_is(TokenType::Semi) {
                self.advance(); // consume ';'
                statements.push(item);
            } else if self.peek_is(TokenType::CloseBrace) {
                // No semicolon and closing brace follows - this is the result expression
                self.advance(); // consume '}'
                if statements.is_empty() {
                    // Just a single expression
                    return Ok(item);
                } else {
                    // Multiple statements with a final expression
                    return Ok(AstNode::BlockExpr {
                        statements,
                        result: Box::new(item),
                    });
                }
            } else {
                // No semicolon and not closing brace - error or multi-line expression
                // For now, treat as a statement and continue
                statements.push(item);
            }
        }
    }

    /// Parse a single item in a block (statement or expression)
    fn parse_block_item(&mut self) -> ParseResult<AstNode> {
        if self.peek_is(TokenType::Let) {
            self.parse_let_decl()
        } else if self.peek_is(TokenType::Print) {
            self.advance(); // consume 'print'
            let exprs = self.parse_comma_separated(|p| p.parse_expression(), TokenType::Semi)?;
            Ok(AstNode::Print { exprs })
        } else {
            self.parse_expression()
        }
    }
}
