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
        // First parse unary and primary
        let mut left = self.parse_unary_primary()?;

        // Then apply postfix operations (array access, method calls, casts)
        // This ensures that -42 as Str parses as (-42) as Str, not -(42 as Str)
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

        Ok(left)
    }

    /// Parse unary operators and primary expressions.
    /// Unary operators (-, !, +) are right-associative, so !!x is parsed as !(!(x))
    /// This is called before postfix operations, so postfix operators apply to the result of unary.
    fn parse_unary_primary(&mut self) -> ParseResult<AstNode> {
        if let Some(tok) = self.peek() {
            match tok.kind {
                // Allow unary operators: !, -, +
                TokenType::Bang | TokenType::Minus | TokenType::Plus => {
                    let op = tok.kind;
                    self.advance(); // consume operator
                                    // Recursively parse unary for chaining: !!x, ---x, etc.
                    let expr = self.parse_unary_primary()?;
                    Ok(AstNode::UnaryExpr {
                        op,
                        expr: Box::new(expr),
                    })
                }
                // Primary expressions:
                // Handles: number, identifier, function call foo(a + b), string, boolean, array, map
                _ => self.parse_primary(),
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
                let index = self.parse_expression()?;
                self.expect(TokenType::CloseBracket)?;
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
            } else if self.peek_is(TokenType::Question) {
                // Handle ? operator (try propagate): expr?
                self.advance(); // consume '?'
                expr = AstNode::TryPropagate {
                    expr: Box::new(expr),
                };
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

                    // If followed by '::', parse as enum variant
                    if self.peek_is(TokenType::ColonColon) {
                        self.advance(); // consume '::'
                        let variant_tok = self.expect(TokenType::Identifier)?;
                        let variant = variant_tok.value.to_string();

                        // Check for payload
                        let payload = if self.peek_is(TokenType::OpenParen) {
                            self.advance(); // consume '('
                            let expr = self.parse_expression()?;
                            self.expect(TokenType::CloseParen)?;
                            Some(Box::new(expr))
                        } else {
                            None
                        };

                        return Ok(AstNode::EnumVariant {
                            enum_name: name,
                            variant,
                            payload,
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

                        // Parse field: value pairs
                        while !self.peek_is(TokenType::CloseBrace) {
                            let field_tok = self.expect(TokenType::Identifier)?;
                            let field_name = field_tok.value.to_string();
                            self.expect(TokenType::Colon)?;
                            let value = self.parse_expression()?;
                            fields.push((field_name, Box::new(value)));

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
                    Ok(AstNode::StringLiteral(tok.value.to_string()))
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

    /// Example: `[1, 2, 3]`
    /// Uses parse_comma_separated to parse elements until ']'.
    fn parse_array_literal(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::OpenBracket)?;

        let elements = self
            .parse_comma_separated(|parser| parser.parse_expression(), TokenType::CloseBracket)?;

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
    /// Example: `{ "a": 1, "b": 2 }`
    /// Each entry is a key-value pair separated by ':' and entries separated by ','.
    fn parse_map_literal(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::OpenBrace)?;

        let entries = self.parse_comma_separated(
            |p| {
                let key = p.parse_expression()?; // parse key
                p.expect(TokenType::Colon)?; // expect ':'
                let value = p.parse_expression()?; // parse value
                Ok((key, value))
            },
            TokenType::CloseBrace,
        )?;

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
            TokenType::Lt | TokenType::Gt | TokenType::LtEq | TokenType::GtEq => 4,
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
}
