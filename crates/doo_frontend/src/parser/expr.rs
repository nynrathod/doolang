//! Expression Parser - Thin Dispatch Layer
//!
//! Minimal trait-based parser that delegates to centralized helpers.
//! No duplicate logic, all utilities in helpers.rs.

use super::stmt::ParserStmt;
use super::types::ParserTypes;
use super::{
    helpers::{self, BraceType},
    ParseResult, Parser,
};
use crate::ast::*;
use crate::lexer::TokenKind;
use doo_core::{CompilerError, ErrorCode, Span};

/// Expression parsing trait.
pub trait ParserExpr {
    fn parse_expression(&mut self) -> ParseResult<Expr>;
    fn parse_unary(&mut self) -> ParseResult<Expr>;
    fn parse_primary(&mut self) -> ParseResult<Expr>;
    fn parse_postfix(&mut self, expr: Expr) -> ParseResult<Expr>;
}

impl ParserExpr for Parser {
    /// Entry point for expression parsing.
    fn parse_expression(&mut self) -> ParseResult<Expr> {
        self.parse_expression_prec(0)
    }

    /// Parse unary expressions: -x, !x
    fn parse_unary(&mut self) -> ParseResult<Expr> {
        if let Some(op) = UnaryOp::from_token(self.current().kind) {
            let start = self.current_span();
            self.advance();
            let expr = self.parse_unary()?;
            let span = start.merge(&expr.span);
            return Ok(Expr::new(
                ExprKind::Unary {
                    op,
                    expr: Box::new(expr),
                },
                span,
            ));
        }
        self.parse_primary()
    }

    /// Parse primary expressions.
    fn parse_primary(&mut self) -> ParseResult<Expr> {
        let start = self.current_span();

        match self.current().kind {
            TokenKind::Integer | TokenKind::Float => self.parse_number_literal(start),

            TokenKind::String => {
                let text = self.current().text.clone();
                self.advance();
                Ok(Expr::new(
                    ExprKind::StrLit(Self::process_escapes(&text)),
                    start,
                ))
            }

            TokenKind::StringTemplate => {
                let text = self.current().text.clone();
                self.advance();
                self.parse_string_interpolation(&text, start)
            }

            TokenKind::True => {
                self.advance();
                Ok(Expr::new(ExprKind::BoolLit(true), start))
            }

            TokenKind::False => {
                self.advance();
                Ok(Expr::new(ExprKind::BoolLit(false), start))
            }

            TokenKind::Nil => {
                self.advance();
                Ok(Expr::new(ExprKind::Nil, start))
            }

            TokenKind::Spread => {
                self.advance();
                let inner = self.parse_expression()?;
                Ok(Expr::new(ExprKind::Spread(Box::new(inner)), start))
            }

            TokenKind::Ident => parse_ident(self, start),

            TokenKind::LParen => {
                if self.lookahead_closure() {
                    parse_closure(self, start)
                } else {
                    parse_group_or_tuple(self, start)
                }
            }

            TokenKind::LBracket => parse_array(self, start),

            TokenKind::LBrace => match self.lookahead_brace_type() {
                BraceType::Object => parse_object(self, start),
                BraceType::Map => parse_map(self, start),
                BraceType::Block => parse_block(self, start),
                BraceType::RouteBlock => parse_route_block(self, start),
            },

            TokenKind::If => parse_if_expr(self, start),
            TokenKind::Match => parse_match(self, start),
            TokenKind::Ok => parse_ok(self, start),
            TokenKind::Err => parse_err(self, start),

            _ => Err(CompilerError::new(
                ErrorCode::UnexpectedToken,
                format!("Expected expression, got {}", self.current().kind),
                start,
            )),
        }
    }

    /// Parse postfix operations.
    fn parse_postfix(&mut self, mut expr: Expr) -> ParseResult<Expr> {
        loop {
            match self.current().kind {
                TokenKind::LParen => {
                    expr = parse_call(self, expr)?;
                }
                TokenKind::Dot => {
                    expr = parse_field_or_method(self, expr)?;
                }
                TokenKind::LBracket => {
                    expr = parse_index(self, expr)?;
                }
                TokenKind::As => {
                    expr = parse_cast(self, expr)?;
                }
                TokenKind::QuestionQuestion => {
                    // Unwrap or panic operator: expr ?? panic(msg)
                    let start = expr.span;
                    self.advance();

                    // Expect 'panic' keyword and message
                    if !self.check(TokenKind::Ident) || self.current().text != "panic" {
                        return Err(CompilerError::new(
                            ErrorCode::UnexpectedToken,
                            "Expected `panic` after `??`",
                            self.current().span,
                        ));
                    }
                    self.advance();

                    self.expect(TokenKind::LParen)?;
                    let message = Box::new(self.parse_expression()?);
                    self.expect(TokenKind::RParen)?;

                    let span = start.merge(&self.prev_span());
                    expr = Expr::new(
                        ExprKind::UnwrapOrPanic {
                            expr: Box::new(expr),
                            message,
                        },
                        span,
                    );
                }
                TokenKind::Question => {
                    // Error propagation operator: expr?
                    let start = expr.span;
                    self.advance();
                    let span = start.merge(&self.prev_span());
                    expr = Expr::new(ExprKind::Try(Box::new(expr)), span);
                }
                _ => break,
            }
        }
        Ok(expr)
    }
}

// === Primary Expression Parsers ===

fn parse_ident(parser: &mut Parser, start: Span) -> ParseResult<Expr> {
    let name = parser.current().text.clone();
    parser.advance();

    if parser.check(TokenKind::ColonColon) {
        parser.advance();
        let variant = parser.expect_ident()?;
        let payload = if parser.check(TokenKind::LParen) {
            parser.advance();
            let args = parser.parse_list(TokenKind::RParen, |p| p.parse_expression())?;
            parser.expect(TokenKind::RParen)?;
            args
        } else {
            vec![]
        };
        return Ok(Expr::new(
            ExprKind::EnumVariant {
                enum_name: name,
                variant,
                payload,
            },
            start.merge(&parser.prev_span()),
        ));
    }

    if parser.check(TokenKind::LBrace) && name.chars().next().map_or(false, |c| c.is_uppercase()) {
        parser.advance();
        let fields = parser.parse_list(TokenKind::RBrace, |p| {
            let field = p.expect_ident()?;
            if p.check(TokenKind::Colon) {
                p.advance();
                let value = p.parse_expression()?;
                Ok((field, value))
            } else {
                let span = p.prev_span();
                Ok((field.clone(), Expr::new(ExprKind::Ident(field), span)))
            }
        })?;
        parser.expect(TokenKind::RBrace)?;
        return Ok(Expr::new(
            ExprKind::StructLit { name, fields },
            start.merge(&parser.prev_span()),
        ));
    }

    Ok(Expr::new(ExprKind::Ident(name), start))
}

fn parse_group_or_tuple(parser: &mut Parser, start: Span) -> ParseResult<Expr> {
    parser.expect(TokenKind::LParen)?;
    if parser.check(TokenKind::RParen) {
        parser.advance();
        return Ok(Expr::new(ExprKind::TupleLit(Vec::new()), start));
    }

    let first = parser.parse_expression()?;
    if parser.check(TokenKind::Comma) {
        let mut elements = vec![first];
        parser.advance();
        if !parser.check(TokenKind::RParen) {
            elements.extend(parser.parse_list(TokenKind::RParen, |p| p.parse_expression())?);
        }
        parser.expect(TokenKind::RParen)?;
        Ok(Expr::new(
            ExprKind::TupleLit(elements),
            start.merge(&parser.prev_span()),
        ))
    } else {
        parser.expect(TokenKind::RParen)?;
        Ok(first)
    }
}

fn parse_array(parser: &mut Parser, start: Span) -> ParseResult<Expr> {
    parser.expect(TokenKind::LBracket)?;
    let elements = parser.parse_list(TokenKind::RBracket, |p| {
        if p.check(TokenKind::Spread) {
            let span = p.current_span();
            p.advance();
            let inner = p.parse_expression()?;
            Ok(Expr::new(ExprKind::Spread(Box::new(inner)), span))
        } else {
            p.parse_expression()
        }
    })?;
    parser.expect(TokenKind::RBracket)?;
    Ok(Expr::new(
        ExprKind::ArrayLit(elements),
        start.merge(&parser.prev_span()),
    ))
}

fn parse_map(parser: &mut Parser, start: Span) -> ParseResult<Expr> {
    parser.expect(TokenKind::LBrace)?;
    let entries = parser.parse_list(TokenKind::RBrace, |p| {
        if p.check(TokenKind::Spread) {
            let span = p.current_span();
            p.advance();
            let spread = p.parse_expression()?;
            Ok((
                Expr::new(ExprKind::Spread(Box::new(spread.clone())), span),
                Expr::new(ExprKind::Nil, span),
            ))
        } else {
            let key = p.parse_expression()?;
            p.expect(TokenKind::Colon)?;
            let value = p.parse_expression()?;
            Ok((key, value))
        }
    })?;
    parser.expect(TokenKind::RBrace)?;
    Ok(Expr::new(
        ExprKind::MapLit(entries),
        start.merge(&parser.prev_span()),
    ))
}

fn parse_object(parser: &mut Parser, start: Span) -> ParseResult<Expr> {
    parser.expect(TokenKind::LBrace)?;
    let entries = parser.parse_list(TokenKind::RBrace, |p| {
        let key = p.expect_ident()?;
        p.expect(TokenKind::Colon)?;
        let value = p.parse_expression()?;
        Ok((key, value))
    })?;
    parser.expect(TokenKind::RBrace)?;
    Ok(Expr::new(
        ExprKind::ObjectLit(entries),
        start.merge(&parser.prev_span()),
    ))
}

fn parse_block(parser: &mut Parser, start: Span) -> ParseResult<Expr> {
    parser.expect(TokenKind::LBrace)?;
    let mut stmts = Vec::new();
    let mut final_expr = None;

    while !parser.check(TokenKind::RBrace) && !parser.is_at_end() {
        let item = parser.parse_statement()?;
        if parser.check(TokenKind::Semi) {
            parser.advance();
            stmts.push(item);
        } else if parser.check(TokenKind::RBrace) {
            if let StmtKind::Expr(e) = item.kind {
                final_expr = Some(Box::new(e));
            } else {
                stmts.push(item);
            }
        } else {
            stmts.push(item);
        }
    }

    parser.expect(TokenKind::RBrace)?;
    Ok(Expr::new(
        ExprKind::Block(stmts, final_expr),
        start.merge(&parser.prev_span()),
    ))
}

/// Parse a route block: `{ get("/path", Handler), post("/path", Handler) }`
/// Used in app.group() calls for defining multiple routes inline.
fn parse_route_block(parser: &mut Parser, start: Span) -> ParseResult<Expr> {
    parser.expect(TokenKind::LBrace)?;
    let routes = parser.parse_list(TokenKind::RBrace, |p| {
        // Each route is a function call like `get("/path", Handler)`
        p.parse_expression()
    })?;
    parser.expect(TokenKind::RBrace)?;
    Ok(Expr::new(
        ExprKind::RouteBlock { routes },
        start.merge(&parser.prev_span()),
    ))
}

fn parse_if_expr(parser: &mut Parser, start: Span) -> ParseResult<Expr> {
    parser.expect(TokenKind::If)?;
    let condition = parser.parse_expression()?;
    let then_branch = parse_block(parser, parser.current_span())?;
    let else_branch = if parser.check(TokenKind::Else) {
        parser.advance();
        if parser.check(TokenKind::If) {
            Some(Box::new(parse_if_expr(parser, parser.current_span())?))
        } else {
            Some(Box::new(parse_block(parser, parser.current_span())?))
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
        start.merge(&parser.prev_span()),
    ))
}

fn parse_match(parser: &mut Parser, start: Span) -> ParseResult<Expr> {
    parser.expect(TokenKind::Match)?;
    let mut values = Vec::new();
    if !parser.check(TokenKind::LBrace) {
        values.push(parser.parse_expression()?);
        while parser.check(TokenKind::Comma) {
            parser.advance();
            values.push(parser.parse_expression()?);
        }
    }
    parser.expect(TokenKind::LBrace)?;
    let arms = parser.parse_list(TokenKind::RBrace, |p| parse_match_arm(p))?;
    parser.expect(TokenKind::RBrace)?;
    Ok(Expr::new(
        ExprKind::Match { values, arms },
        start.merge(&parser.prev_span()),
    ))
}

fn parse_match_arm(parser: &mut Parser) -> ParseResult<MatchArm> {
    let start = parser.current_span();
    let pattern = parse_match_pattern(parser)?;
    let guard = if parser.check(TokenKind::If) {
        parser.advance();
        Some(parser.parse_expression()?)
    } else {
        None
    };
    parser.expect(TokenKind::FatArrow)?;

    // Parse body: can be an expression OR a statement (wrapped in implicit block)
    // Statement keywords that can appear as match arm bodies
    let body = if matches!(
        parser.current().kind,
        TokenKind::Print
            | TokenKind::Let
            | TokenKind::For
            | TokenKind::Return
            | TokenKind::Break
            | TokenKind::Continue
    ) {
        // Parse as a statement and wrap in a Block expression
        let body_start = parser.current_span();
        let stmt = parser.parse_statement()?;
        let body_span = body_start.merge(&parser.prev_span());
        Expr::new(ExprKind::Block(vec![stmt], None), body_span)
    } else {
        // Parse as a regular expression
        parser.parse_expression()?
    };

    Ok(MatchArm {
        pattern,
        guard,
        body,
        span: start.merge(&parser.prev_span()),
    })
}

fn parse_match_pattern(parser: &mut Parser) -> ParseResult<MatchPattern> {
    match parser.current().kind {
        TokenKind::Underscore => {
            parser.advance();
            Ok(MatchPattern::Wildcard)
        }
        TokenKind::Ident => {
            let name = parser.current().text.clone();
            parser.advance();
            if parser.check(TokenKind::ColonColon) {
                parser.advance();
                let variant = parser.expect_ident()?;
                if parser.check(TokenKind::LParen) {
                    parser.advance();
                    let bindings = parser.parse_list(TokenKind::RParen, |p| p.expect_ident())?;
                    parser.expect(TokenKind::RParen)?;
                    Ok(MatchPattern::EnumVariantPayload {
                        enum_name: name,
                        variant,
                        bindings,
                    })
                } else {
                    Ok(MatchPattern::EnumVariant {
                        enum_name: name,
                        variant,
                    })
                }
            } else {
                parser.pos -= 1;
                let expr = parser.parse_expression()?;
                Ok(MatchPattern::Condition(Box::new(expr)))
            }
        }
        TokenKind::Integer
        | TokenKind::Float
        | TokenKind::String
        | TokenKind::True
        | TokenKind::False => {
            let expr = parser.parse_primary()?;
            Ok(MatchPattern::Literal(Box::new(expr)))
        }
        _ => {
            let expr = parser.parse_expression()?;
            Ok(MatchPattern::Condition(Box::new(expr)))
        }
    }
}

fn parse_closure(parser: &mut Parser, start: Span) -> ParseResult<Expr> {
    parser.expect(TokenKind::LParen)?;
    let params = parser.parse_list(TokenKind::RParen, |p| {
        let name = p.expect_ident()?;
        let type_ann = if p.check(TokenKind::Colon) {
            p.advance();
            Some(p.parse_type_expr()?)
        } else {
            None
        };
        Ok((name, type_ann))
    })?;
    parser.expect(TokenKind::RParen)?;

    let mut return_type = None;
    let mut error_type = None;

    if parser.check(TokenKind::FatArrow) {
        parser.advance();
    } else if parser.check(TokenKind::Arrow) {
        parser.advance();
        return_type = Some(parser.parse_type_expr()?);
        if parser.check(TokenKind::Bang) {
            parser.advance();
            error_type = Some(parser.parse_type_expr()?);
        }
    } else {
        return Err(CompilerError::new(
            ErrorCode::UnexpectedToken,
            "Expected => or -> in closure",
            parser.current_span(),
        ));
    }

    let body = if parser.check(TokenKind::LBrace) {
        parse_block(parser, parser.current_span())?
    } else {
        parser.parse_expression()?
    };

    Ok(Expr::new(
        ExprKind::Closure {
            params,
            body: Box::new(body),
            return_type,
            error_type,
        },
        start.merge(&parser.prev_span()),
    ))
}

fn parse_ok(parser: &mut Parser, start: Span) -> ParseResult<Expr> {
    parser.expect(TokenKind::Ok)?;
    let values = if parser.check(TokenKind::LParen) {
        // Ok(value) or Ok(v1, v2, ...)
        parser.advance();
        let args = parser.parse_list(TokenKind::RParen, |p| p.parse_expression())?;
        parser.expect(TokenKind::RParen)?;
        args
    } else {
        // Ok value or Ok v1, v2, ...
        // Parse first value, then continue with comma-separated values
        let mut values = vec![parser.parse_expression()?];
        while parser.check(TokenKind::Comma) {
            parser.advance();
            // Stop if next token looks like a statement start
            if parser.check(TokenKind::Semi) || parser.check(TokenKind::RBrace) {
                break;
            }
            values.push(parser.parse_expression()?);
        }
        values
    };
    Ok(Expr::new(
        ExprKind::Ok(values),
        start.merge(&parser.prev_span()),
    ))
}

fn parse_err(parser: &mut Parser, start: Span) -> ParseResult<Expr> {
    parser.expect(TokenKind::Err)?;
    let value = if parser.check(TokenKind::LParen) {
        parser.advance();
        let expr = parser.parse_expression()?;
        parser.expect(TokenKind::RParen)?;
        expr
    } else {
        parser.parse_expression()?
    };
    Ok(Expr::new(
        ExprKind::Err(Box::new(value)),
        start.merge(&parser.prev_span()),
    ))
}

// === Postfix Expression Parsers ===

fn parse_call(parser: &mut Parser, func: Expr) -> ParseResult<Expr> {
    let start = func.span;
    parser.expect(TokenKind::LParen)?;
    let args = parser.parse_list(TokenKind::RParen, |p| p.parse_expression())?;
    parser.expect(TokenKind::RParen)?;
    Ok(Expr::new(
        ExprKind::Call {
            func: Box::new(func),
            args,
        },
        start.merge(&parser.prev_span()),
    ))
}

fn parse_field_or_method(parser: &mut Parser, object: Expr) -> ParseResult<Expr> {
    let start = object.span;
    parser.expect(TokenKind::Dot)?;
    let field = parser.expect_ident()?;
    if parser.check(TokenKind::LParen) {
        parser.advance();
        let args = parser.parse_list(TokenKind::RParen, |p| p.parse_expression())?;
        parser.expect(TokenKind::RParen)?;
        Ok(Expr::new(
            ExprKind::MethodCall {
                object: Box::new(object),
                method: field,
                args,
            },
            start.merge(&parser.prev_span()),
        ))
    } else {
        Ok(Expr::new(
            ExprKind::Field {
                object: Box::new(object),
                field,
            },
            start.merge(&parser.prev_span()),
        ))
    }
}

fn parse_index(parser: &mut Parser, object: Expr) -> ParseResult<Expr> {
    let start = object.span;
    parser.expect(TokenKind::LBracket)?;
    let index = parser.parse_expression()?;
    parser.expect(TokenKind::RBracket)?;
    Ok(Expr::new(
        ExprKind::Index {
            object: Box::new(object),
            index: Box::new(index),
        },
        start.merge(&parser.prev_span()),
    ))
}

fn parse_cast(parser: &mut Parser, expr: Expr) -> ParseResult<Expr> {
    let start = expr.span;
    parser.expect(TokenKind::As)?;
    let target = parser.parse_type_expr()?;
    Ok(Expr::new(
        ExprKind::Cast {
            expr: Box::new(expr),
            target,
        },
        start.merge(&parser.prev_span()),
    ))
}

// ============================================================================
// STRING INTERPOLATION PARSER
// Parses "hello ${expr} world" into StringInterpolation(Vec<StringPart>)
// ============================================================================

impl Parser {
    /// Parse a string template into StringInterpolation expression
    /// Template format: "Hello ${name}, you have ${count} messages"
    pub(super) fn parse_string_interpolation(
        &mut self,
        template: &str,
        span: Span,
    ) -> ParseResult<Expr> {
        use crate::ast::StringPart;
        use crate::lexer::Lexer;

        let mut parts: Vec<StringPart> = Vec::new();
        let chars: Vec<char> = template.chars().collect();
        let mut pos = 0;

        while pos < chars.len() {
            // Find next ${
            let remaining: String = chars[pos..].iter().collect();
            if let Some(dollar_idx) = remaining.find("${") {
                // Add literal part before ${
                if dollar_idx > 0 {
                    let literal: String = chars[pos..pos + dollar_idx].iter().collect();
                    parts.push(StringPart::Literal(Self::process_escapes(&literal)));
                }

                // Find matching closing brace
                let expr_start = pos + dollar_idx + 2;
                let mut brace_depth = 1;
                let mut expr_end = expr_start;

                while expr_end < chars.len() && brace_depth > 0 {
                    match chars[expr_end] {
                        '{' => brace_depth += 1,
                        '}' => brace_depth -= 1,
                        _ => {}
                    }
                    if brace_depth > 0 {
                        expr_end += 1;
                    }
                }

                if brace_depth != 0 {
                    return Err(CompilerError::new(
                        ErrorCode::UnexpectedToken,
                        "Unclosed ${} in string interpolation",
                        span,
                    ));
                }

                // Extract and parse the expression inside ${}
                let expr_str: String = chars[expr_start..expr_end].iter().collect();

                // Create a new lexer and parser for the embedded expression
                let tokens = Lexer::new(&expr_str, 0).tokenize();
                let mut expr_parser = Parser::from_tokens(tokens, 0);
                let expr = expr_parser.parse_expression()?;

                parts.push(StringPart::Expr(Box::new(expr)));
                pos = expr_end + 1;
            } else {
                // No more interpolations, add remaining as literal
                let literal: String = chars[pos..].iter().collect();
                parts.push(StringPart::Literal(Self::process_escapes(&literal)));
                break;
            }
        }

        // If no parts (empty string), add empty literal
        if parts.is_empty() {
            parts.push(StringPart::Literal(String::new()));
        }

        // If only one literal part, return as simple string
        if parts.len() == 1 {
            if let StringPart::Literal(s) = &parts[0] {
                return Ok(Expr::new(ExprKind::StrLit(s.clone()), span));
            }
        }

        Ok(Expr::new(ExprKind::StringInterpolation(parts), span))
    }
}
