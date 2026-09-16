use super::{ParseResult, Parser};
use crate::ast::*;
use crate::lexer::TokenKind;
use doo_core::{CompilerError, ErrorCode};

impl Parser {
    // === Types ===

    pub fn parse_type_expr(&mut self) -> ParseResult<TypeExpr> {
        let start = self.current_span();

        // Array shorthand: [T]
        if self.check(TokenKind::LBracket) {
            self.advance();
            let elem = self.parse_type_expr()?;
            self.expect(TokenKind::RBracket)?;
            return Ok(TypeExpr::new(TypeExprKind::Array(Box::new(elem)), start));
        }

        // Map shorthand: {K: V}
        if self.check(TokenKind::LBrace) {
            self.advance();
            let key = self.parse_type_expr()?;
            self.expect(TokenKind::Colon)?;
            let val = self.parse_type_expr()?;
            self.expect(TokenKind::RBrace)?;
            return Ok(TypeExpr::new(
                TypeExprKind::Map(Box::new(key), Box::new(val)),
                start,
            ));
        }

        let name = self.expect_ident()?;

        //  Check for generic type arguments: Name<T, U>
        if self.check(TokenKind::Lt) {
            self.advance(); // consume <
            let mut args = Vec::new();
            while !self.check(TokenKind::Gt) && !self.is_at_end() {
                args.push(self.parse_type_expr()?);
                if !self.check(TokenKind::Gt) {
                    self.expect(TokenKind::Comma)?;
                }
            }
            self.expect(TokenKind::Gt)?; // consume >

            // Map specific generic types to their AST kinds
            match name.as_str() {
                "Array" if args.len() == 1 => {
                    return Ok(TypeExpr::new(
                        TypeExprKind::Array(Box::new(args.remove(0))),
                        start,
                    ));
                }
                "Map" if args.len() == 2 => {
                    let val = args.remove(1);
                    let key = args.remove(0);
                    return Ok(TypeExpr::new(
                        TypeExprKind::Map(Box::new(key), Box::new(val)),
                        start,
                    ));
                }
                "Option" if args.len() == 1 => {
                    return Ok(TypeExpr::new(
                        TypeExprKind::Optional(Box::new(args.remove(0))),
                        start,
                    ));
                }
                "Result" if args.len() == 2 => {
                    let err = args.remove(1);
                    let ok = args.remove(0);
                    return Ok(TypeExpr::new(
                        TypeExprKind::Result(Box::new(ok), Box::new(err)),
                        start,
                    ));
                }
                _ => {
                    // Unknown generic type — discard args and return Named(name)
                    return Ok(TypeExpr::new(TypeExprKind::Named(name), start));
                }
            }
        }

        // Optional marker: T?
        if self.check(TokenKind::Question) {
            self.advance();
            return Ok(TypeExpr::new(
                TypeExprKind::Optional(Box::new(TypeExpr::new(TypeExprKind::Named(name), start))),
                start,
            ));
        }

        Ok(TypeExpr::new(TypeExprKind::Named(name), start))
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
