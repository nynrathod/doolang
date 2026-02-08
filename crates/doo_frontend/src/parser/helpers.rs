//! Parser Helper Utilities
//!
//! Centralized parsing utilities following single-source-of-truth principle.
//! All repetitive parsing logic is extracted here to avoid duplication.

use super::{ParseResult, Parser};
use crate::ast::*;
use crate::lexer::TokenKind;
use doo_core::{CompilerError, ErrorCode, Span};

impl Parser {
    /// Parse comma-separated items until end token.
    pub(super) fn parse_list<T, F>(
        &mut self,
        end: TokenKind,
        mut parse_fn: F,
    ) -> ParseResult<Vec<T>>
    where
        F: FnMut(&mut Self) -> ParseResult<T>,
    {
        let mut items = Vec::new();
        while !self.check(end) && !self.is_at_end() {
            items.push(parse_fn(self)?);
            if !self.check(end) {
                self.expect(TokenKind::Comma)?;
            }
        }
        Ok(items)
    }

    /// Parse binary operations with precedence climbing.
    pub(super) fn parse_binary_op(&mut self, mut left: Expr, min_prec: u8) -> ParseResult<Expr> {
        while let Some(op) = BinaryOp::from_token(self.current().kind) {
            let op_prec = op.precedence();
            if op_prec < min_prec {
                break;
            }
            let op_span = self.current_span();
            self.advance();
            let right = self.parse_expression_prec(op_prec + 1).map_err(|_| {
                CompilerError::new(
                    ErrorCode::ExpectedExprAfterOp,
                    format!("expected expression after `{}`", op),
                    op_span,
                )
            })?;
            let span = left.span.merge(&right.span);
            left = Expr::new(
                ExprKind::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            );
        }
        Ok(left)
    }

    /// Parse range operators (.. or ..=).
    pub(super) fn parse_range(&mut self, left: Expr) -> ParseResult<Expr> {
        if !self.check(TokenKind::DotDot) && !self.check(TokenKind::DotDotEq) {
            return Ok(left);
        }

        let inclusive = self.check(TokenKind::DotDotEq);
        self.advance();
        let end = self.parse_expression_prec(8)?;
        let span = left.span.merge(&end.span);

        Ok(Expr::new(
            ExprKind::Range {
                start: Box::new(left),
                end: Box::new(end),
                inclusive,
            },
            span,
        ))
    }

    /// Process escape sequences in strings (centralized).
    /// Invalid escape sequences like `\z` are preserved as-is (lexer may catch them).
    pub(super) fn process_escapes(s: &str) -> String {
        let mut result = String::new();
        let mut chars = s.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(&next) = chars.peek() {
                    chars.next();
                    match next {
                        'n' => result.push('\n'),
                        't' => result.push('\t'),
                        'r' => result.push('\r'),
                        '\\' => result.push('\\'),
                        '"' => result.push('"'),
                        '\'' => result.push('\''),
                        '0' => result.push('\0'),
                        '$' => result.push('$'),
                        _ => {
                            // Unknown escape — preserve as-is
                            // The lexer handles the error reporting for truly invalid escapes
                            result.push('\\');
                            result.push(next);
                        }
                    }
                } else {
                    result.push('\\');
                }
            } else {
                result.push(c);
            }
        }
        result
    }

    /// Lookahead helper for closure detection.
    pub(super) fn lookahead_closure(&mut self) -> bool {
        let saved = self.pos;
        if !self.check(TokenKind::LParen) {
            return false;
        }
        self.advance();

        let mut depth = 1;
        while depth > 0 && !self.is_at_end() {
            match self.current().kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => depth -= 1,
                _ => {}
            }
            if depth > 0 {
                self.advance();
            }
        }
        self.advance();

        let result = self.check(TokenKind::FatArrow) || self.check(TokenKind::Arrow);
        self.pos = saved;
        result
    }

    /// Check if identifier is an HTTP route method name.
    fn is_route_method_name(name: &str) -> bool {
        matches!(
            name,
            "get" | "post" | "put" | "delete" | "patch" | "head" | "options"
        )
    }

    /// Lookahead helper for object vs map vs block vs route block.
    pub(super) fn lookahead_brace_type(&mut self) -> BraceType {
        let saved = self.pos;
        if !self.check(TokenKind::LBrace) {
            self.pos = saved;
            return BraceType::Block;
        }
        self.advance();

        // Empty braces `{}` → Map
        if self.check(TokenKind::RBrace) {
            self.pos = saved;
            return BraceType::Map;
        }

        // Spread operator `{...x}` → Map
        if self.check(TokenKind::Spread) {
            self.pos = saved;
            return BraceType::Map;
        }

        // String key `{"key": value}` → Map
        if self.check(TokenKind::String) {
            self.pos = saved;
            return BraceType::Map;
        }

        // Integer key `{1: value}` → Map
        if self.check(TokenKind::Integer) {
            // Look ahead for colon to confirm it's a map
            self.advance();
            if self.check(TokenKind::Colon) {
                self.pos = saved;
                return BraceType::Map;
            }
            self.pos = saved;
            return BraceType::Block;
        }

        // Float key `{1.5: value}` → Map
        if self.check(TokenKind::Float) {
            // Look ahead for colon to confirm it's a map
            self.advance();
            if self.check(TokenKind::Colon) {
                self.pos = saved;
                return BraceType::Map;
            }
            self.pos = saved;
            return BraceType::Block;
        }

        // Bool key `{true: value}` → Map
        if self.check(TokenKind::True) || self.check(TokenKind::False) {
            // Look ahead for colon to confirm it's a map
            self.advance();
            if self.check(TokenKind::Colon) {
                self.pos = saved;
                return BraceType::Map;
            }
            self.pos = saved;
            return BraceType::Block;
        }

        // Identifier - could be Object, Block, or RouteBlock
        if !self.check(TokenKind::Ident) {
            self.pos = saved;
            return BraceType::Block;
        }

        // Check for RouteBlock pattern: `{ get(...)` or `{ post(...)` etc.
        let ident_text = self.current().text.clone();
        if Self::is_route_method_name(&ident_text) {
            self.advance();
            if self.check(TokenKind::LParen) {
                self.pos = saved;
                return BraceType::RouteBlock;
            }
            // Reset and continue with normal logic
            self.pos = saved;
            self.advance(); // skip LBrace again
        }

        let mut depth = 0;
        let mut found_colon = false;
        let mut found_semi = false;

        while !self.is_at_end() {
            match self.current().kind {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace if depth == 0 => break,
                TokenKind::RBrace => depth -= 1,
                TokenKind::Colon if depth == 0 => {
                    found_colon = true;
                    break;
                }
                TokenKind::Semi if depth == 0 => {
                    found_semi = true;
                    break;
                }
                _ => {}
            }
            self.advance();
        }

        self.pos = saved;

        if found_semi {
            BraceType::Block
        } else if found_colon {
            BraceType::Object
        } else {
            BraceType::Block
        }
    }

    /// Parse number literal (int or float).
    pub(super) fn parse_number_literal(&mut self, start: Span) -> ParseResult<Expr> {
        let text = self.current().text.clone();
        let kind = self.current().kind;
        self.advance();

        match kind {
            TokenKind::Integer => {
                let val = text.parse::<i64>().map_err(|_| {
                    CompilerError::new(
                        ErrorCode::InvalidNumberLiteral,
                        format!("Invalid integer: {}", text),
                        start,
                    )
                })?;
                Ok(Expr::new(ExprKind::IntLit(val), start))
            }
            TokenKind::Float => {
                let val = text.parse::<f64>().map_err(|_| {
                    CompilerError::new(
                        ErrorCode::InvalidNumberLiteral,
                        format!("Invalid float: {}", text),
                        start,
                    )
                })?;
                Ok(Expr::new(ExprKind::FloatLit(val), start))
            }
            _ => Err(CompilerError::new(
                ErrorCode::InvalidExpression,
                "Expected number",
                start,
            )),
        }
    }
}

/// Brace disambiguation result.
pub enum BraceType {
    Object,     // {key: value}
    Map,        // {"str": value} or {...}
    Block,      // { stmt; }
    RouteBlock, // { get("/path", Handler), post(...) }
}
