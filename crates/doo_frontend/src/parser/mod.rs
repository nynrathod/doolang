//! # Parser Module
//!
//! Hand-written recursive descent parser with Pratt parsing (precedence climbing)
//! for expressions. Pre-tokenizes the entire input for O(1) lookahead.

pub mod expr;
pub mod helpers;
pub mod items;
pub mod stmt;
pub mod types;

use crate::ast::*;
use crate::lexer::{Lexer, Token, TokenKind};
use doo_core::{CompilerError, Span};

pub type ParseResult<T> = Result<T, CompilerError>;

/// The Doo parser.
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    errors: Vec<CompilerError>,
    file_id: u32,
    loop_depth: usize,
    fn_depth: usize,
}

impl Parser {
    /// Create a new parser from a lexer. Pre-tokenizes the entire input.
    pub fn new(source: &str, file_id: u32) -> Self {
        let lexer = Lexer::new(source, file_id);
        let mut tokens: Vec<Token> = lexer.collect();

        // Architectural Invariant: The parser requires a trailing `Eof` token
        // as a sentinel to safely bound all lookahead operations (e.g., `current()`).
        // The `Iterator` implementation of `Lexer` drops the `Eof` token,
        // so we enforce the invariant here at the parser boundary.
        if tokens.last().map_or(true, |t| t.kind != TokenKind::Eof) {
            let eof_span = tokens.last().map_or(Span::dummy(), |t| t.span);
            tokens.push(Token::new(TokenKind::Eof, "", eof_span));
        }

        Self {
            tokens,
            pos: 0,
            errors: Vec::new(),
            file_id,
            loop_depth: 0,
            fn_depth: 0,
        }
    }

    /// Get any non-fatal errors collected during parsing.
    pub fn errors(&self) -> &[CompilerError] {
        &self.errors
    }

    /// Parse a complete program.
    pub fn parse_program(&mut self) -> Result<Program, Vec<CompilerError>> {
        let start_span = self.current().span;
        let mut items = Vec::new();

        while !self.is_at_end() {
            // Top-level recovery: skip stray closing delimiters to prevent infinite loops
            if matches!(
                self.current().kind,
                TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket
            ) {
                self.errors.push(CompilerError::new(
                    doo_core::ErrorCode::UnexpectedToken,
                    format!(
                        "unexpected `{}` at top level",
                        self.current().kind.description()
                    ),
                    self.current().span,
                ));
                self.advance();
                continue;
            }

            let pos_before = self.pos;
            match self.parse_item() {
                Ok(item) => items.push(item),
                Err(e) => {
                    self.errors.push(e);
                    self.synchronize();

                    // GUARANTEE PROGRESS: if synchronize didn't advance, force advance
                    // This is the ultimate safeguard against infinite loops.
                    if self.pos == pos_before && !self.is_at_end() {
                        self.advance();
                    }
                }
            }
        }

        if !self.errors.is_empty() {
            return Err(self.errors.clone());
        }

        let end_span = self.prev_span();
        Ok(Program::new(items, start_span.merge(end_span)))
    }

    // === Token Navigation ===

    #[inline]
    pub fn current(&self) -> &Token {
        // Safety net: pos should never exceed bounds if advance/is_at_end are correct,
        // but we return the last token (guaranteed to be Eof) to absolutely prevent
        // index-out-of-bounds panics.
        if self.pos >= self.tokens.len() {
            return self
                .tokens
                .last()
                .expect("tokens must contain at least Eof");
        }
        &self.tokens[self.pos]
    }

    #[inline]
    pub fn peek_next(&self) -> &Token {
        if self.pos + 1 < self.tokens.len() {
            &self.tokens[self.pos + 1]
        } else {
            &self.tokens[self.tokens.len() - 1]
        }
    }

    #[inline]
    pub fn advance(&mut self) -> &Token {
        if !self.is_at_end() {
            self.pos += 1;
        }

        // If pos is 0 (empty input, already at EOF), return the EOF token itself
        // to prevent usize underflow on `self.pos - 1`.
        if self.pos == 0 {
            return self.current();
        }

        &self.tokens[self.pos - 1]
    }

    #[inline]
    pub fn is_at_end(&self) -> bool {
        self.current().kind == TokenKind::Eof
    }

    #[inline]
    pub fn check(&self, kind: TokenKind) -> bool {
        self.current().kind == kind
    }

    #[inline]
    pub fn match_token(&mut self, kind: TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn expect(&mut self, kind: TokenKind) -> ParseResult<Span> {
        if self.check(kind) {
            let span = self.current().span;
            self.advance();
            Ok(span)
        } else {
            Err(CompilerError::new(
                doo_core::ErrorCode::UnexpectedToken,
                format!(
                    "expected `{}`, got `{}`",
                    kind.description(),
                    self.current().kind.description()
                ),
                self.current().span,
            ))
        }
    }

    #[inline]
    pub fn expect_ident(&mut self) -> ParseResult<String> {
        if self.check(TokenKind::Ident) {
            let text = self.current().text.clone();
            self.advance();
            Ok(text)
        } else {
            Err(CompilerError::new(
                doo_core::ErrorCode::ExpectedIdentifier,
                format!(
                    "expected identifier, got `{}`",
                    self.current().kind.description()
                ),
                self.current().span,
            ))
        }
    }

    #[inline]
    pub fn prev_span(&self) -> Span {
        if self.pos > 0 {
            self.tokens[self.pos - 1].span
        } else {
            Span::dummy()
        }
    }

    #[inline]
    pub fn current_span(&self) -> Span {
        self.current().span
    }

    /// Error recovery: skip tokens until we hit a statement boundary.
    pub fn synchronize(&mut self) {
        while !self.is_at_end() {
            match self.current().kind {
                TokenKind::Semi => {
                    self.advance(); // Consume the semicolon and continue
                    return;
                }
                TokenKind::RBrace => {
                    // Do NOT consume the closing brace here!
                    // Let the block parser consume it so blocks close correctly.
                    return;
                }
                TokenKind::Fn
                | TokenKind::Struct
                | TokenKind::Enum
                | TokenKind::Interface
                | TokenKind::Const
                | TokenKind::Static
                | TokenKind::Impl => return,
                _ => {
                    self.advance();
                }
            }
        }
    }

    #[inline]
    pub fn is_at_stmt_end(&self) -> bool {
        self.check(TokenKind::Semi) || self.check(TokenKind::RBrace) || self.is_at_end()
    }
}
