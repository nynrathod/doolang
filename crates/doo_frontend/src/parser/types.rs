use super::{ParseResult, Parser};
use crate::ast::*;
use crate::lexer::TokenKind;
use doo_core::{CompilerError, ErrorCode};

impl Parser {
    // === Types ===

    pub(crate) fn parse_type_expr(&mut self) -> ParseResult<TypeExpr> {
        let start = self.current_span();

        // Array type: [T] or [T]?
        let base = if self.check(TokenKind::LBracket) {
            self.advance();
            let element = self.parse_type_expr()?;
            self.expect(TokenKind::RBracket)?;
            let end = self.prev_span();
            TypeExpr::array(element, start.merge(end))

        // Map type: {K: V} or {K: V}?
        } else if self.check(TokenKind::LBrace) {
            self.advance();
            let key = self.parse_type_expr()?;
            self.expect(TokenKind::Colon)?;
            let value = self.parse_type_expr()?;
            self.expect(TokenKind::RBrace)?;
            let end = self.prev_span();
            TypeExpr::new(
                TypeExprKind::Map(Box::new(key), Box::new(value)),
                start.merge(end),
            )

        // Tuple type: (T1, T2, ...) or (T1, T2)?
        } else if self.check(TokenKind::LParen) {
            self.advance();
            let mut types = Vec::new();
            while !self.check(TokenKind::RParen) && !self.is_at_end() {
                types.push(self.parse_type_expr()?);
                if !self.check(TokenKind::RParen) {
                    self.expect(TokenKind::Comma)?;
                }
            }
            self.expect(TokenKind::RParen)?;
            let end = self.prev_span();
            TypeExpr::new(TypeExprKind::Tuple(types), start.merge(end))

        // Function type: fn(T) -> U  or  fn(T1, T2) -> U  or  fn() -> U
        } else if self.check(TokenKind::Fn) {
            self.advance();
            self.expect(TokenKind::LParen)?;
            let mut params = Vec::new();
            while !self.check(TokenKind::RParen) && !self.is_at_end() {
                params.push(self.parse_type_expr()?);
                if !self.check(TokenKind::RParen) {
                    self.expect(TokenKind::Comma)?;
                }
            }
            self.expect(TokenKind::RParen)?;
            // Optional: -> ReturnType
            let returns = if self.check(TokenKind::Arrow) {
                self.advance();
                self.parse_type_expr()?
            } else {
                // Default to Void if no return type specified
                TypeExpr::void(self.current_span())
            };
            let end = self.prev_span();
            TypeExpr::new(
                TypeExprKind::Function {
                    params,
                    returns: Box::new(returns),
                },
                start.merge(end),
            )

        // Named type: T or T?
        } else {
            let name = self.expect_ident().map_err(|_| {
                CompilerError::new(
                    ErrorCode::InvalidTypeExpr,
                    format!("expected type expression, got `{}`", self.current().kind),
                    self.current_span(),
                )
                .with_suggestion("expected a type like Int, String, [T], {K: V}")
            })?;

            let end = self.prev_span();
            TypeExpr::named(name, start.merge(end))
        };

        // Check for optional suffix: T?, [T]?, {K: V}?, (T1, T2)?
        if self.check(TokenKind::Question) {
            self.advance();
            let end = self.prev_span();
            return Ok(TypeExpr::optional(base, start.merge(end)));
        }

        Ok(base)
    }

    // === Patterns ===

    pub(crate) fn parse_pattern(&mut self) -> ParseResult<Pattern> {
        let start = self.current_span();

        if self.check(TokenKind::Underscore) {
            self.advance();
            return Ok(Pattern::wildcard(start));
        }

        if self.check(TokenKind::LParen) {
            self.advance();
            let mut patterns = Vec::new();
            while !self.check(TokenKind::RParen) && !self.is_at_end() {
                patterns.push(self.parse_pattern()?);
                if !self.check(TokenKind::RParen) {
                    self.expect(TokenKind::Comma)?;
                }
            }
            self.expect(TokenKind::RParen)?;
            let end = self.prev_span();
            return Ok(Pattern::new(PatternKind::Tuple(patterns), start.merge(end)));
        }

        let name = self.expect_ident()?;
        let end = self.prev_span();
        Ok(Pattern::ident(name, start.merge(end)))
    }
}
