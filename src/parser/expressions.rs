use crate::lexar::token::TokenType;
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
        let mut left = if let Some(tok) = self.peek() {
            match tok.kind {
                // Disallow unary '!' operator
                TokenType::Bang => {
                    let tok = self.advance().unwrap();
                    return Err(ParseError::UnexpectedTokenAt {
                        msg: "Unary '!' operator is not allowed in doolang".to_string(),
                        line: tok.line,
                        col: tok.col,
                    });
                }
                // Allow unary minus and plus if desired
                TokenType::Minus | TokenType::Plus => {
                    let op = tok.kind;
                    self.advance(); // consume operator
                    let expr = self.parse_expression_prec(7)?; // unary has high precedence
                    AstNode::UnaryExpr {
                        op,
                        expr: Box::new(expr),
                    }
                }
                // Primary expressions:
                // Handles: number, identifier, function call foo(a + b), string, boolean, array, map
                _ => self.parse_primary()?,
            }
        } else {
            return Err(ParseError::EndOfInput);
        };

        // Postfix operations: array/map element access
        // Handles: arr[0], map["key"], nested[i][j], etc.
        left = self.parse_postfix(left)?;

        // Binary operator expressions:
        // Handles: a + b, x * y - z, a < b, a <= b, a > b, a >= b
        // 🟡 TODO: Operators && , || not supported yet
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
            let mut right = self.parse_expression_prec(prec + 1)?;

            // Build a BinaryExpr AST node with the current left and right expressions
            left = AstNode::BinaryExpr {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// Parses postfix operations on an expression.
    /// Handles array/map element access: arr[0], map["key"], nested[i][j]
    /// Can be chained: arr[0][1][2]
    fn parse_postfix(&mut self, mut expr: AstNode) -> ParseResult<AstNode> {
        while self.peek_is(TokenType::OpenBracket) {
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
        }
        Ok(expr)
    }

    /// Handles literals (number, string, boolean), identifiers
    /// function calls, arrays, and maps.
    fn parse_primary(&mut self) -> ParseResult<AstNode> {
        if let Some(tok) = self.peek() {
            match tok.kind {
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
                TokenType::OpenBracket => self.parse_array_literal(),
                TokenType::OpenBrace => self.parse_map_literal(),
                TokenType::OpenParen => Err(ParseError::UnexpectedTokenAt {
                    msg: "Parentheses are not allowed in expressions in mtlang".to_string(),
                    line: tok.line,
                    col: tok.col,
                }),
                _ => Err(ParseError::UnexpectedTokenAt {
                    msg: format!("Expected primary expression, got {:?}", tok.kind),
                    line: tok.line,
                    col: tok.col,
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
}
