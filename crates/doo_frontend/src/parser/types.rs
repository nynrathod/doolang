use crate::ast::*;
use crate::lexer::TokenKind;
use doo_core::{Span, CompilerError, ErrorCode};
use super::{Parser, ParseResult};

/// Trait for parsing type expressions and patterns.
pub trait ParserTypes {
    fn parse_type_expr(&mut self) -> ParseResult<TypeExpr>;
    fn parse_pattern(&mut self) -> ParseResult<Pattern>;
}

impl ParserTypes for Parser {
    // === Types ===

    fn parse_type_expr(&mut self) -> ParseResult<TypeExpr> {
        let start = self.current_span();

        // Array type: [T]
        if self.check(TokenKind::LBracket) {
            self.advance();
            let element = self.parse_type_expr()?;
            self.expect(TokenKind::RBracket)?;
            let end = self.prev_span();
            return Ok(TypeExpr::array(element, start.merge(&end)));
        }

        // Map type: {K: V}
        if self.check(TokenKind::LBrace) {
            self.advance();
            let key = self.parse_type_expr()?;
            self.expect(TokenKind::Colon)?;
            let value = self.parse_type_expr()?;
            self.expect(TokenKind::RBrace)?;
            let end = self.prev_span();
            return Ok(TypeExpr::new(
                TypeExprKind::Map(Box::new(key), Box::new(value)),
                start.merge(&end),
            ));
        }

        // Tuple type: (T1, T2, ...)
        if self.check(TokenKind::LParen) {
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
            return Ok(TypeExpr::new(TypeExprKind::Tuple(types), start.merge(&end)));
        }

        // Named type
        let name = self.expect_ident()?;

        // Check for optional: T?
        if self.check(TokenKind::Question) {
            self.advance();
            let end = self.prev_span();
            let inner = TypeExpr::named(&name, start);
            return Ok(TypeExpr::optional(inner, start.merge(&end)));
        }

        let end = self.prev_span();
        Ok(TypeExpr::named(name, start.merge(&end)))
    }

    // === Patterns ===

    fn parse_pattern(&mut self) -> ParseResult<Pattern> {
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
            return Ok(Pattern::new(PatternKind::Tuple(patterns), start.merge(&end)));
        }

        let name = self.expect_ident()?;
        let end = self.prev_span();
        Ok(Pattern::ident(name, start.merge(&end)))
    }
}
