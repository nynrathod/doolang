use crate::lexer::token::TokenType;
use crate::parser::ast::{AstNode, MatchArm, MatchPattern, Pattern};
use crate::parser::{ParseError, ParseResult, Parser};

impl<'a> Parser<'a> {
    /// Syntax:
    ///   - `if condition { ... }`
    ///   - `if condition { ... } else { ... }`
    ///   - `if condition { ... } else if ...`
    /// Supports nested else-if branches recursively.
    pub fn parse_conditional_stmt(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::If)?;

        // Parse condition expression
        let condition = self.parse_expression()?;

        // Parse then block
        let then_block = self.parse_braced_block()?; // parse statements until '}'

        // Parse optional else or else-if branch
        let mut else_branch = None;
        if let Some(tok) = self.peek() {
            if tok.kind == TokenType::Else {
                self.advance(); // consume 'else'

                if let Some(next) = self.peek() {
                    if next.kind == TokenType::If {
                        // else if: recursively parse another conditional
                        let elseif = self.parse_conditional_stmt()?;
                        else_branch = Some(Box::new(elseif));
                    } else {
                        // else: parse block
                        let else_block = self.parse_braced_block()?;
                        else_branch = Some(Box::new(AstNode::Block(else_block)));
                    }
                }
            }
        }

        Ok(AstNode::ConditionalStmt {
            condition: Box::new(condition),
            then_block,
            else_branch,
        })
    }

    /// Supports tuple patterns and optional iterable expressions.
    /// Syntax:
    ///   - `for a, b or (a, b) in iterable { ... }`
    ///   - `for { ... }` (infinite loop)
    /// Returns a ForLoopStmt AST node.
    pub fn parse_for_stmt(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::For)?;

        // Parse loop variable pattern(s)
        let pattern = if self.peek_is(TokenType::OpenBrace) {
            Pattern::Wildcard // `for { ... }` infinite loop
        } else {
            // Parse comma-separated patterns for tuple destructuring
            let patterns = self.parse_comma_separated(
                |p| p.parse_pattern(),
                TokenType::In, // Stop at 'in' or at the start of the iterable expression
            )?;

            if patterns.len() == 1 {
                patterns.into_iter().next().unwrap()
            } else {
                Pattern::Tuple(patterns)
            }
        };

        // Parse optional iterable expression after 'in'
        let iterable = if self.peek_is(TokenType::In) {
            self.advance(); // consume 'in'
            Some(Box::new(self.parse_expression()?))
        } else if self.peek_is(TokenType::OpenBrace) {
            None // infinite loop, no iterable
        } else {
            None
        };

        // Parse loop body block
        let body = self.parse_braced_block()?;

        Ok(AstNode::ForLoopStmt {
            pattern,
            iterable,
            body,
        })
    }

    /// Parse match expression
    /// Syntax:
    ///   - `match value { pattern => expr, ... }`
    ///   - `match { condition => expr, ... }`
    pub fn parse_match_expr(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::Match)?;

        // Check if we have a value or condition-based match
        let value = if self.peek_is(TokenType::OpenBrace) {
            None // condition-based match
        } else {
            Some(Box::new(self.parse_expression()?))
        };

        self.expect(TokenType::OpenBrace)?;

        let mut arms = Vec::new();

        while !self.peek_is(TokenType::CloseBrace) {
            // Parse pattern
            let pattern = if self.peek_is(TokenType::Underscore) {
                self.advance();
                MatchPattern::Wildcard
            } else if value.is_none() {
                // Condition-based match: parse expression as condition
                let condition = self.parse_expression()?;
                MatchPattern::Condition(Box::new(condition))
            } else {
                // Check if it's an enum pattern (e.g., Status::Active or HttpCode::Success(code))
                if let Some(first_tok) = self.peek() {
                    if first_tok.kind == TokenType::Identifier {
                        // Look ahead for :: to detect enum pattern
                        if self.peek_ahead(1).map(|t| t.kind) == Some(TokenType::ColonColon) {
                            // Parse enum pattern
                            let enum_name = self.advance().unwrap().value.to_string();
                            self.expect(TokenType::ColonColon)?;
                            let variant = self.expect(TokenType::Identifier)?.value.to_string();

                            // Check if there's a payload with binding
                            if self.peek_is(TokenType::OpenParen) {
                                self.advance(); // consume '('
                                let binding = self.expect(TokenType::Identifier)?.value.to_string();
                                self.expect(TokenType::CloseParen)?;
                                MatchPattern::EnumVariantWithPayload {
                                    enum_name,
                                    variant,
                                    binding,
                                }
                            } else {
                                MatchPattern::EnumVariant { enum_name, variant }
                            }
                        } else {
                            // Regular literal pattern
                            let literal = self.parse_expression()?;
                            MatchPattern::Literal(Box::new(literal))
                        }
                    } else {
                        // Value-based match: parse literal pattern
                        let literal = self.parse_expression()?;
                        MatchPattern::Literal(Box::new(literal))
                    }
                } else {
                    return Err(ParseError::EndOfInput);
                }
            };

            self.expect(TokenType::FatArrow)?;

            // Parse body - handle both expressions and statements like print
            let body = if let Some(tok) = self.peek() {
                match tok.kind {
                    TokenType::Print => {
                        // Parse print statement as body
                        Box::new(self.parse_print()?)
                    }
                    TokenType::OpenBrace => {
                        // Parse block as body
                        let block = self.parse_braced_block()?;
                        Box::new(AstNode::Block(block))
                    }
                    _ => {
                        // Parse regular expression
                        Box::new(self.parse_expression()?)
                    }
                }
            } else {
                return Err(ParseError::EndOfInput);
            };

            arms.push(MatchArm { pattern, body });

            // Optional comma
            if self.peek_is(TokenType::Comma) {
                self.advance();
            } else if !self.peek_is(TokenType::CloseBrace) {
                return Err(ParseError::UnexpectedToken(
                    "Expected ',' or '}' after match arm".to_string(),
                ));
            }
        }

        self.expect(TokenType::CloseBrace)?;

        Ok(AstNode::MatchExpr { value, arms })
    }

    /// Parses a return statement.
    /// Syntax: `return expr1, expr2, ...;`
    /// Consumes 'return', then parses one or more expressions separated by commas, ending with a semicolon.
    pub fn parse_return(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::Return)?; // consume 'return'

        let mut values = Vec::new();

        loop {
            let expr = self.parse_expression()?;
            values.push(expr);

            match self.peek() {
                Some(tok) if tok.kind == TokenType::Comma => {
                    self.advance(); // consume ',' and continue parsing next expression
                }
                _ => break, // no more expressions
            }
        }

        // Return statements MUST end with semicolon
        self.expect(TokenType::Semi)?;

        Ok(AstNode::Return { values })
    }

    /// Syntax: `break;`
    /// Returns a Break AST node.
    pub fn parse_break(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::Break)?;
        self.expect(TokenType::Semi)?;
        Ok(AstNode::Break)
    }

    /// Syntax: `continue;`
    /// Returns a Continue AST node.
    pub fn parse_continue(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::Continue)?;
        self.expect(TokenType::Semi)?;
        Ok(AstNode::Continue)
    }

    /// Syntax: `print(expr1, expr2, ...);` or `print expr1, expr2, ...;`
    /// Supports both with and without parentheses.
    /// Returns a Print AST node.
    pub fn parse_print(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::Print)?;

        let args = if self.peek().map(|t| t.kind) == Some(TokenType::OpenParen) {
            // Parse with parentheses: print(expr1, expr2)
            self.advance(); // consume '('
            let exprs =
                self.parse_comma_separated(|p| p.parse_expression(), TokenType::CloseParen)?;
            self.expect(TokenType::CloseParen)?;
            exprs
        } else {
            // Parse without parentheses: print expr1, expr2
            let mut exprs = Vec::new();
            loop {
                exprs.push(self.parse_expression()?);

                if let Some(tok) = self.peek() {
                    if tok.kind == TokenType::Comma {
                        self.advance(); // consume ','
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            exprs
        };

        // Semicolon is optional in match arms (followed by comma or close brace)
        if let Some(tok) = self.peek() {
            if tok.kind == TokenType::Semi {
                self.advance();
            }
        }

        Ok(AstNode::Print { exprs: args })
    }

    /// Parses an assignment statement.
    /// Supports tuple destructuring on the left-hand side (e.g., `a, b = ...;`).
    /// Uses parse_comma_separated for LHS patterns, then expects '=' and parses the RHS expression.
    /// Returns an Assignment AST node.
    pub fn parse_assignment(&mut self) -> ParseResult<AstNode> {
        // Parse comma-separated patterns for the left-hand side
        let patterns = self.parse_comma_separated(|p| p.parse_pattern(), TokenType::Eq)?;

        // Only allow assignment to a single identifier (not tuple, not wildcard)
        // Ex., let a, _ = ...; Allowed
        if patterns.len() != 1 {
            return Err(ParseError::UnexpectedToken(
                "Tuple assignment is only allowed in 'let' declarations".into(),
            ));
        }

        let lhs_pattern = patterns.into_iter().next().unwrap();
        match lhs_pattern {
            Pattern::Identifier(_) => {}
            _ => {
                // Disallow assignment to wildcard or tuple
                // Ex., a, _ = ...; Not allowed without let
                return Err(ParseError::UnexpectedToken(
                    "Only single-variable assignment is allowed without 'let'".into(),
                ));
            }
        }

        self.expect(TokenType::Eq)?;
        let rhs = self.parse_expression()?;
        self.expect(TokenType::Semi)?;

        Ok(AstNode::Assignment {
            pattern: lhs_pattern,
            value: Box::new(rhs),
        })
    }

    /// Parses a pattern for use in assignments, for loops, and match arms.
    /// Supports:
    ///   - Identifiers: `x`
    ///   - Tuple patterns: `(x, y, z) or without () x,y,z`
    ///   - Wildcard: `_`
    /// Returns a Pattern enum variant.
    pub fn parse_pattern(&mut self) -> ParseResult<Pattern> {
        // Prevent stack overflow from deeply nested patterns
        if self.depth >= crate::parser::parser::MAX_DEPTH {
            return Err(ParseError::UnexpectedTokenAt {
                msg: "Pattern nesting too deep (recursion limit exceeded)".to_string(),
                line: self.peek().map(|t| t.line).unwrap_or(0),
                col: self.peek().map(|t| t.col).unwrap_or(0),
            });
        }

        self.depth += 1;
        let result = if let Some(tok) = self.peek() {
            match tok.kind {
                // Identifier or enum variant pattern
                TokenType::Identifier => {
                    let name = tok.value.to_string();
                    self.advance();
                    Ok(Pattern::Identifier(name))
                }

                // Optional:Tuple pattern for assignment and for loops e.g., (x, y)
                TokenType::OpenParen => {
                    self.advance(); // consume '('
                    let elements =
                        self.parse_comma_separated(|p| p.parse_pattern(), TokenType::CloseParen)?;
                    self.expect(TokenType::CloseParen)?;
                    Ok(Pattern::Tuple(elements))
                }

                // Wildcard pattern, e.g., _
                TokenType::Underscore => {
                    self.advance();
                    Ok(Pattern::Wildcard)
                }

                // Unexpected token in pattern context
                _ => Err(ParseError::UnexpectedTokenAt {
                    msg: format!("Unexpected token {:?} in pattern", tok.kind),
                    line: tok.line,
                    col: tok.col,
                }),
            }
        } else {
            Err(ParseError::EndOfInput)
        };
        self.depth -= 1;
        result
    }

    /// Parses a block of statements enclosed in braces `{ ... }`.
    /// Returns a vector of AST nodes for each statement in the block.
    /// Ok/Err statements MUST end with semicolons.
    fn parse_block(&mut self) -> ParseResult<Vec<AstNode>> {
        let mut stmts = Vec::new();
        while let Some(tok) = self.peek() {
            if tok.kind == TokenType::CloseBrace {
                break;
            }

            // Regular statement - use parse_statement which handles Ok/Err
            stmts.push(self.parse_statement()?);
        }
        self.expect(TokenType::CloseBrace)?; // consume '}'
        Ok(stmts)
    }

    /// Parses a block of statements, expecting an opening brace first.
    /// Returns a vector of AST nodes for the block.
    /// Used in conditionals, functions, and loops
    pub fn parse_braced_block(&mut self) -> ParseResult<Vec<AstNode>> {
        self.expect(TokenType::OpenBrace)?;
        self.parse_block()
    }
}
