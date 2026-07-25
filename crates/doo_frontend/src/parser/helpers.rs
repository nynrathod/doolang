//! Parser Helper Utilities
//!
//! Centralized parsing utilities following single-source-of-truth principle.

use super::{ParseResult, Parser};
use crate::ast::*;
use crate::lexer::TokenKind;
use doo_core::{CompilerError, ErrorCode, Span};

impl Parser {
    /// Parse comma-separated items until end token.
    pub fn parse_list<T, F>(&mut self, end: TokenKind, mut parse_fn: F) -> ParseResult<Vec<T>>
    where
        F: FnMut(&mut Parser) -> ParseResult<T>,
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

    /// Process escape sequences in strings (centralized).
    pub fn process_escapes(s: &str) -> String {
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
    /// Checks if we are at `(params) =>` or `(params) ->`.
    pub fn lookahead_closure(&mut self) -> bool {
        let saved_pos = self.pos;
        if !self.check(TokenKind::LParen) {
            return false;
        }
        self.advance(); // consume (

        // Empty params: `() =>`
        if self.check(TokenKind::RParen) {
            self.advance(); // consume )
            let result = self.check(TokenKind::FatArrow) || self.check(TokenKind::Arrow);
            self.pos = saved_pos;
            return result;
        }

        // Try to match `(name` or `(name: Type`
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

        if self.check(TokenKind::RParen) {
            self.advance(); // consume )
            let result = self.check(TokenKind::FatArrow) || self.check(TokenKind::Arrow);
            self.pos = saved_pos;
            return result;
        }

        self.pos = saved_pos;
        false
    }

    /// Lookahead helper for object vs map vs block vs route block.
    pub fn lookahead_brace_type(&mut self) -> BraceType {
        let saved_pos = self.pos;
        if !self.check(TokenKind::LBrace) {
            return BraceType::Block;
        }
        self.advance(); // consume {

        // Empty braces `{}` → Map
        if self.check(TokenKind::RBrace) {
            self.pos = saved_pos;
            return BraceType::Map;
        }

        // Spread operator `{...x}` → Map
        if self.check(TokenKind::Spread) {
            self.pos = saved_pos;
            return BraceType::Map;
        }

        // String/Int/Float key `{ "key": v }` or `{ 1: v }` → Map
        if matches!(
            self.current().kind,
            TokenKind::String
                | TokenKind::Integer
                | TokenKind::Float
                | TokenKind::True
                | TokenKind::False
        ) {
            self.pos = saved_pos;
            return BraceType::Map;
        }

        // Identifier - could be Object, Block, or RouteBlock
        if self.check(TokenKind::Ident) {
            let ident_text = self.current().text.clone();
            self.advance();

            // Object literal: `{ key: value }`
            if self.check(TokenKind::Colon) {
                self.pos = saved_pos;
                return BraceType::Object;
            }

            // Route block: `{ get("/path", handler) }`
            if matches!(
                ident_text.as_str(),
                "get" | "post" | "put" | "delete" | "patch" | "head" | "options"
            ) {
                if self.check(TokenKind::LParen) {
                    self.pos = saved_pos;
                    return BraceType::RouteBlock;
                }
            }
        }

        self.pos = saved_pos;
        BraceType::Block
    }
}

/// Brace disambiguation result.
pub enum BraceType {
    Object,     // {key: value}
    Map,        // {"str": value} or {...}
    Block,      // { stmt; }
    RouteBlock, // { get("/path", Handler), post(...) }
}
