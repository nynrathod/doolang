//! Expression Parser (Pratt Parsing / Precedence Climbing)
//!
//! Parses expressions with correct operator precedence and right-associativity.

use super::helpers::BraceType;
use super::{ParseResult, Parser};
use crate::ast::*;
use crate::lexer::TokenKind;
use doo_core::{CompilerError, ErrorCode, Span};

impl Parser {
    /// Entry point for expression parsing.
    pub fn parse_expression(&mut self) -> ParseResult<Expr> {
        self.parse_expression_prec(1) // Minimum precedence = 1 (lowest)
    }

    /// Pratt parser core loop.
    pub fn parse_expression_prec(&mut self, min_prec: u8) -> ParseResult<Expr> {
        let mut left = self.parse_unary()?;

        loop {
            // Handle Range operators explicitly since they have unique AST nodes
            if self.check(TokenKind::DotDot) || self.check(TokenKind::DotDotEq) {
                let op_prec = 10; // Range precedence
                if op_prec < min_prec {
                    break;
                }
                let inclusive = self.check(TokenKind::DotDotEq);
                let op_span = self.current().span;
                self.advance();

                let right = self.parse_expression_prec(op_prec + 1).map_err(|_| {
                    CompilerError::new(
                        ErrorCode::ExpectedExprAfterOp,
                        "expected expression after range operator",
                        op_span,
                    )
                })?;

                let span = left.span.merge(right.span);
                left = Expr::new(
                    ExprKind::Range {
                        start: Box::new(left),
                        end: Box::new(right),
                        inclusive,
                    },
                    span,
                );
                continue;
            }

            // Handle standard binary operators
            if let Some(op) = BinaryOp::from_token(self.current().kind) {
                let op_prec = op.precedence();
                if op_prec < min_prec {
                    break;
                }

                let op_span = self.current().span;
                self.advance();

                let right = self.parse_expression_prec(op_prec + 1).map_err(|_| {
                    CompilerError::new(
                        ErrorCode::ExpectedExprAfterOp,
                        format!("expected expression after `{}`", op),
                        op_span,
                    )
                })?;

                let span = left.span.merge(right.span);
                left = Expr::new(
                    ExprKind::Binary {
                        left: Box::new(left),
                        op,
                        right: Box::new(right),
                    },
                    span,
                );
            } else {
                break;
            }
        }

        Ok(left)
    }

    /// Parse unary operators (`-x`, `!x`).
    fn parse_unary(&mut self) -> ParseResult<Expr> {
        if let Some(op) = UnaryOp::from_token(self.current().kind) {
            let op_span = self.current().span;
            self.advance();
            let expr = self.parse_unary()?; // Right associative
            let span = op_span.merge(expr.span);
            return Ok(Expr::new(
                ExprKind::Unary {
                    op,
                    expr: Box::new(expr),
                },
                span,
            ));
        }
        // Fix borrow checker: evaluate primary first, then pass it to postfix
        let primary = self.parse_primary()?;
        self.parse_postfix(primary)
    }

    /// Parse primary expressions (literals, identifiers, blocks).
    fn parse_primary(&mut self) -> ParseResult<Expr> {
        let span = self.current().span;

        match self.current().kind {
            TokenKind::Integer => {
                let text = self.current().text.clone();
                self.advance();
                let val = text.parse::<i64>().map_err(|_| {
                    CompilerError::new(
                        ErrorCode::InvalidNumberLiteral,
                        format!("Invalid integer: {}", text),
                        span,
                    )
                })?;
                Ok(Expr::new(ExprKind::IntLit(val), span))
            }
            TokenKind::Float => {
                let text = self.current().text.clone();
                self.advance();
                let val = text.parse::<f64>().map_err(|_| {
                    CompilerError::new(
                        ErrorCode::InvalidNumberLiteral,
                        format!("Invalid float: {}", text),
                        span,
                    )
                })?;
                Ok(Expr::new(ExprKind::FloatLit(val), span))
            }
            TokenKind::String | TokenKind::StringTemplate => {
                let text = self.current().text.clone();
                self.advance();
                self.parse_string_literal(text, span)
            }
            TokenKind::True => {
                self.advance();
                Ok(Expr::new(ExprKind::BoolLit(true), span))
            }
            TokenKind::False => {
                self.advance();
                Ok(Expr::new(ExprKind::BoolLit(false), span))
            }
            TokenKind::Nil => {
                self.advance();
                Ok(Expr::new(ExprKind::Nil, span))
            }
            TokenKind::Ident => {
                let name = self.current().text.clone();
                self.advance();
                Ok(Expr::new(ExprKind::Ident(name), span))
            }
            TokenKind::LParen => {
                if self.lookahead_closure() {
                    self.parse_closure(span)
                } else {
                    self.advance(); // consume (
                    let expr = self.parse_expression()?;

                    if self.check(TokenKind::Comma) {
                        // Tuple literal
                        let mut elements = vec![expr];
                        while self.match_token(TokenKind::Comma) {
                            if self.check(TokenKind::RParen) {
                                break;
                            } // trailing comma
                            elements.push(self.parse_expression()?);
                        }
                        let end_span = self.expect(TokenKind::RParen)?;
                        Ok(Expr::new(
                            ExprKind::TupleLit(elements),
                            span.merge(end_span),
                        ))
                    } else {
                        let _end_span = self.expect(TokenKind::RParen)?;
                        Ok(expr) // Just a parenthesized expression
                    }
                }
            }
            TokenKind::LBracket => self.parse_array_literal(span),
            TokenKind::LBrace => match self.lookahead_brace_type() {
                BraceType::Object => self.parse_object_literal(span),
                BraceType::Map => self.parse_map_literal(span),
                BraceType::Block => self.parse_block_expression(span),
            },
            TokenKind::If => self.parse_if_expression(span),
            TokenKind::Match => self.parse_match_expression(span),
            TokenKind::Async => {
                self.advance();
                let block = self.parse_block()?;
                Ok(Expr::new(
                    ExprKind::Block(block, None),
                    span.merge(self.prev_span()),
                ))
            }
            _ => Err(CompilerError::new(
                ErrorCode::InvalidExpression,
                format!(
                    "expected expression, got `{}`",
                    self.current().kind.description()
                ),
                span,
            )),
        }
    }

    /// Parse postfix operations (`.`, `()`, `[]`, `await`).
    fn parse_postfix(&mut self, mut expr: Expr) -> ParseResult<Expr> {
        loop {
            match self.current().kind {
                TokenKind::Dot => {
                    self.advance();
                    if self.check(TokenKind::Await) {
                        let span = expr.span.merge(self.current().span);
                        self.advance();
                        expr = Expr::new(ExprKind::Await(Box::new(expr)), span);
                    } else {
                        let field = self.expect_ident()?;
                        if self.check(TokenKind::LParen) {
                            self.advance();
                            let args =
                                self.parse_list(TokenKind::RParen, |p| p.parse_expression())?;
                            let end_span = self.expect(TokenKind::RParen)?;
                            let span = expr.span.merge(end_span);
                            expr = Expr::new(
                                ExprKind::MethodCall {
                                    object: Box::new(expr),
                                    method: field,
                                    args,
                                },
                                span,
                            );
                        } else {
                            let span = expr.span.merge(self.prev_span());
                            expr = Expr::new(
                                ExprKind::Field {
                                    object: Box::new(expr),
                                    field,
                                },
                                span,
                            );
                        }
                    }
                }
                TokenKind::LParen => {
                    self.advance();
                    let args = self.parse_list(TokenKind::RParen, |p| p.parse_expression())?;
                    let end_span = self.expect(TokenKind::RParen)?;
                    let span = expr.span.merge(end_span);
                    expr = Expr::new(
                        ExprKind::Call {
                            func: Box::new(expr),
                            args,
                        },
                        span,
                    );
                }
                TokenKind::LBracket => {
                    self.advance();
                    let index = self.parse_expression()?;
                    let end_span = self.expect(TokenKind::RBracket)?;
                    let span = expr.span.merge(end_span);
                    expr = Expr::new(
                        ExprKind::Index {
                            object: Box::new(expr),
                            index: Box::new(index),
                        },
                        span,
                    );
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    // === Compound Parsers ===

    fn parse_string_literal(&mut self, text: String, span: Span) -> ParseResult<Expr> {
        if !text.contains("${") {
            return Ok(Expr::new(
                ExprKind::StrLit(Self::process_escapes(&text)),
                span,
            ));
        }

        let mut parts = Vec::new();
        let mut chars = text.chars().peekable();
        let mut current_literal = String::new();

        while let Some(c) = chars.next() {
            if c == '$' && chars.peek() == Some(&'{') {
                chars.next(); // consume {
                if !current_literal.is_empty() {
                    parts.push(StringPart::Literal(std::mem::take(&mut current_literal)));
                }
                let mut expr_str = String::new();
                let mut depth = 1;
                while let Some(c) = chars.next() {
                    if c == '{' {
                        depth += 1;
                    } else if c == '}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    expr_str.push(c);
                }

                // --- FIX IS HERE ---
                // Parser::new takes &str directly now, no need to create Lexer manually
                let mut parser = Parser::new(&expr_str, self.file_id);
                if let Ok(expr) = parser.parse_expression() {
                    parts.push(StringPart::Expr(Box::new(expr)));
                }
            } else {
                current_literal.push(c);
            }
        }
        if !current_literal.is_empty() {
            parts.push(StringPart::Literal(current_literal));
        }

        Ok(Expr::new(ExprKind::StringInterpolation(parts), span))
    }

    fn parse_closure(&mut self, span: Span) -> ParseResult<Expr> {
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        while !self.check(TokenKind::RParen) && !self.is_at_end() {
            let name = self.expect_ident()?;
            let type_ann = if self.match_token(TokenKind::Colon) {
                Some(self.parse_type_expr()?)
            } else {
                None
            };
            params.push((name, type_ann));
            if !self.check(TokenKind::RParen) {
                self.expect(TokenKind::Comma)?;
            }
        }
        self.expect(TokenKind::RParen)?;

        let return_type = if self.check(TokenKind::Arrow) {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };

        let error_type = if self.check(TokenKind::Bang) {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };

        self.expect(TokenKind::FatArrow)?;
        let body = self.parse_expression()?;
        let end_span = self.prev_span();

        Ok(Expr::new(
            ExprKind::Closure {
                params,
                body: Box::new(body),
                return_type,
                error_type,
            },
            span.merge(end_span),
        ))
    }

    fn parse_array_literal(&mut self, span: Span) -> ParseResult<Expr> {
        self.expect(TokenKind::LBracket)?;
        let mut elements = Vec::new();
        while !self.check(TokenKind::RBracket) && !self.is_at_end() {
            if self.check(TokenKind::Spread) {
                self.advance();
                let expr = self.parse_expression()?;
                let expr_span = expr.span; // Fix use of moved value
                elements.push(Expr::new(ExprKind::Spread(Box::new(expr)), expr_span));
            } else {
                elements.push(self.parse_expression()?);
            }
            if !self.check(TokenKind::RBracket) {
                self.expect(TokenKind::Comma)?;
            }
        }
        let end_span = self.expect(TokenKind::RBracket)?;
        Ok(Expr::new(
            ExprKind::ArrayLit(elements),
            span.merge(end_span),
        ))
    }

    fn parse_object_literal(&mut self, span: Span) -> ParseResult<Expr> {
        self.expect(TokenKind::LBrace)?;
        let mut fields = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            let key = self.expect_ident()?;
            self.expect(TokenKind::Colon)?;
            let value = self.parse_expression()?;
            fields.push((key, value));
            if !self.check(TokenKind::RBrace) {
                self.expect(TokenKind::Comma)?;
            }
        }
        let end_span = self.expect(TokenKind::RBrace)?;
        Ok(Expr::new(ExprKind::ObjectLit(fields), span.merge(end_span)))
    }

    fn parse_map_literal(&mut self, span: Span) -> ParseResult<Expr> {
        self.expect(TokenKind::LBrace)?;
        let mut entries = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            let key = self.parse_expression()?;
            self.expect(TokenKind::Colon)?;
            let value = self.parse_expression()?;
            entries.push((key, value));
            if !self.check(TokenKind::RBrace) {
                self.expect(TokenKind::Comma)?;
            }
        }
        let end_span = self.expect(TokenKind::RBrace)?;
        Ok(Expr::new(ExprKind::MapLit(entries), span.merge(end_span)))
    }

    fn parse_block_expression(&mut self, span: Span) -> ParseResult<Expr> {
        self.expect(TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        let mut expr = None;

        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            let stmt = self.parse_statement()?;
            if stmt.kind.needs_semicolon() {
                self.match_token(TokenKind::Semi);
                stmts.push(stmt);
            } else {
                // If it's an expression that doesn't need a semicolon, it's the block tail
                if let StmtKind::Expr(e) = stmt.kind {
                    expr = Some(Box::new(e));
                    break;
                } else {
                    stmts.push(stmt);
                }
            }
        }
        let end_span = self.expect(TokenKind::RBrace)?;
        Ok(Expr::new(
            ExprKind::Block(stmts, expr),
            span.merge(end_span),
        ))
    }

    pub fn parse_if_expression(&mut self, span: Span) -> ParseResult<Expr> {
        self.expect(TokenKind::If)?;
        let condition = self.parse_expression()?;

        let then_block = self.parse_block()?;
        let then_branch = Expr::new(ExprKind::Block(then_block, None), self.prev_span());

        let else_branch = if self.match_token(TokenKind::Else) {
            if self.check(TokenKind::If) {
                Some(Box::new(self.parse_if_expression(self.current().span)?))
            } else {
                let else_block = self.parse_block()?;
                Some(Box::new(Expr::new(
                    ExprKind::Block(else_block, None),
                    self.prev_span(),
                )))
            }
        } else {
            None
        };

        Ok(Expr::new(
            ExprKind::IfExpr {
                condition: Box::new(condition),
                then_branch: Box::new(then_branch),
                else_branch,
            },
            span.merge(self.prev_span()),
        ))
    }

    fn parse_match_expression(&mut self, span: Span) -> ParseResult<Expr> {
        self.expect(TokenKind::Match)?;
        let mut values = vec![self.parse_expression()?];
        while self.match_token(TokenKind::Comma) {
            values.push(self.parse_expression()?);
        }

        self.expect(TokenKind::LBrace)?;
        let mut arms = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            let arm_span = self.current_span();
            let pattern = self.parse_match_pattern()?;

            let guard = if self.match_token(TokenKind::If) {
                Some(self.parse_expression()?)
            } else {
                None
            };

            self.expect(TokenKind::FatArrow)?;
            let body = self.parse_expression()?;

            arms.push(MatchArm {
                pattern,
                guard,
                body,
                span: arm_span.merge(self.prev_span()),
            });

            if !self.check(TokenKind::RBrace) {
                self.expect(TokenKind::Comma)?;
            }
        }
        let end_span = self.expect(TokenKind::RBrace)?;
        Ok(Expr::new(
            ExprKind::Match { values, arms },
            span.merge(end_span),
        ))
    }

    fn parse_match_pattern(&mut self) -> ParseResult<MatchPattern> {
        if self.match_token(TokenKind::Underscore) {
            return Ok(MatchPattern::Wildcard);
        }
        if matches!(
            self.current().kind,
            TokenKind::Integer
                | TokenKind::Float
                | TokenKind::String
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Minus
        ) {
            let expr = self.parse_expression()?;
            return Ok(MatchPattern::Literal(Box::new(expr)));
        }
        if self.check(TokenKind::Ident) {
            let name = self.current().text.clone();
            self.advance();
            if self.match_token(TokenKind::Dot) {
                let variant = self.expect_ident()?;
                if self.check(TokenKind::LParen) {
                    self.advance();
                    let mut bindings = Vec::new();
                    while !self.check(TokenKind::RParen) && !self.is_at_end() {
                        bindings.push(self.expect_ident()?);
                        if !self.check(TokenKind::RParen) {
                            self.expect(TokenKind::Comma)?;
                        }
                    }
                    self.expect(TokenKind::RParen)?;
                    return Ok(MatchPattern::EnumVariantPayload {
                        enum_name: name,
                        variant,
                        bindings,
                    });
                }
                return Ok(MatchPattern::EnumVariant {
                    enum_name: name,
                    variant,
                });
            }
            return Ok(MatchPattern::Literal(Box::new(Expr::new(
                ExprKind::Ident(name),
                self.prev_span(),
            ))));
        }
        Err(CompilerError::new(
            ErrorCode::InvalidPattern,
            "invalid match pattern",
            self.current().span,
        ))
    }
}
