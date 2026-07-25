//! # Parser Module
//!
//! Hand-written recursive descent parser with Pratt parsing (precedence climbing)
//! for expressions. Pre-tokenizes the entire input for O(1) lookahead.

pub mod expr;
pub mod helpers;

use crate::ast::*;
use crate::lexer::{Lexer, Token, TokenKind};
use doo_core::{CompilerError, ErrorCode, Span};

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
        let tokens: Vec<Token> = lexer.collect();
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
            match self.parse_item() {
                Ok(item) => items.push(item),
                Err(e) => {
                    self.errors.push(e);
                    self.synchronize();
                }
            }
        }

        if !self.errors.is_empty() {
            return Err(self.errors.clone());
        }

        let end_span = self.prev_span();
        Ok(Program::new(items, start_span.merge(end_span)))
    }

    // === Top-Level Items ===

    pub fn parse_item(&mut self) -> ParseResult<Item> {
        match self.current().kind {
            TokenKind::Fn => self.parse_function().map(Item::Function),
            TokenKind::Struct => self.parse_struct().map(Item::Struct),
            TokenKind::Enum => self.parse_enum().map(Item::Enum),
            _ => self.parse_statement().map(Item::Statement),
        }
    }

    fn parse_function(&mut self) -> ParseResult<FunctionDecl> {
        let span = self.current().span;
        self.expect(TokenKind::Fn)?;
        let name = self.expect_ident()?;
        self.expect(TokenKind::LParen)?;
        let params = self.parse_list(TokenKind::RParen, |p| {
            let name = p.expect_ident()?;
            let type_ann = if p.match_token(TokenKind::Colon) {
                Some(p.parse_type_expr()?)
            } else {
                None
            };
            Ok((name, type_ann))
        })?;
        self.expect(TokenKind::RParen)?;

        self.fn_depth += 1;
        let body = self.parse_block()?;
        self.fn_depth -= 1;

        Ok(FunctionDecl {
            name,
            is_public: false,
            type_params: Vec::new(),
            params,
            return_type: None,
            error_type: None,
            body,
            decorators: Vec::new(),
            receiver: None,
            associated_type: None,
            is_expr_fn: false,
            is_async: false,
            span: span.merge(self.prev_span()),
        })
    }

    fn parse_struct(&mut self) -> ParseResult<StructDecl> {
        let span = self.current().span;
        self.expect(TokenKind::Struct)?;
        let name = self.expect_ident()?;
        self.expect(TokenKind::LBrace)?;
        let mut fields = Vec::new();

        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            let field_name = self.expect_ident()?;
            self.expect(TokenKind::Colon)?;
            let type_expr = self.parse_type_expr()?;
            fields.push(FieldDecl {
                name: field_name,
                type_expr,
                is_public: false,
                is_optional: false,
                default: None,
                decorators: Vec::new(),
                span: self.prev_span(),
            });
            self.match_token(TokenKind::Comma);
        }
        self.expect(TokenKind::RBrace)?;

        Ok(StructDecl {
            name,
            is_public: false,
            type_params: Vec::new(),
            fields,
            decorators: Vec::new(),
            span: span.merge(self.prev_span()),
        })
    }

    fn parse_enum(&mut self) -> ParseResult<EnumDecl> {
        let span = self.current().span;
        self.expect(TokenKind::Enum)?;
        let name = self.expect_ident()?;
        self.expect(TokenKind::LBrace)?;
        let mut variants = Vec::new();

        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            let var_name = self.expect_ident()?;
            variants.push(VariantDecl {
                name: var_name,
                payload: None,
                decorators: Vec::new(),
                span: self.prev_span(),
            });
            self.match_token(TokenKind::Comma);
        }
        self.expect(TokenKind::RBrace)?;

        Ok(EnumDecl {
            name,
            is_public: false,
            variants,
            span: span.merge(self.prev_span()),
        })
    }

    // === Token Navigation ===

    #[inline]
    pub fn current(&self) -> &Token {
        &self.tokens[self.pos]
    }

    #[inline]
    pub fn peek(&self) -> &Token {
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
                ErrorCode::UnexpectedToken,
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
                ErrorCode::ExpectedIdentifier,
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

    /// Error recovery: skip tokens until we hit a statement boundary.
    pub fn synchronize(&mut self) {
        while !self.is_at_end() {
            match self.current().kind {
                TokenKind::Semi => {
                    self.advance();
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

    // === Statements ===

    pub fn parse_statement(&mut self) -> ParseResult<Stmt> {
        let span = self.current().span;
        match self.current().kind {
            TokenKind::Let => self.parse_let(),
            TokenKind::Return => self.parse_return(),
            TokenKind::If => {
                let expr = self.parse_if_expression(span)?;
                Ok(Stmt::new(
                    StmtKind::Expr(expr),
                    span.merge(self.prev_span()),
                ))
            }
            TokenKind::For => self.parse_for(),
            _ => {
                let expr = self.parse_expression()?;
                Ok(Stmt::new(
                    StmtKind::Expr(expr),
                    span.merge(self.prev_span()),
                ))
            }
        }
    }

    fn parse_let(&mut self) -> ParseResult<Stmt> {
        let span = self.current().span;
        self.expect(TokenKind::Let)?;
        let mutable = self.match_token(TokenKind::Mut);
        let pattern = self.parse_pattern()?;
        let type_ann = if self.match_token(TokenKind::Colon) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        self.expect(TokenKind::Eq)?;
        let value = self.parse_expression()?;
        Ok(Stmt::new(
            StmtKind::Let {
                mutable,
                pattern,
                type_ann,
                value,
            },
            span.merge(self.prev_span()),
        ))
    }

    fn parse_return(&mut self) -> ParseResult<Stmt> {
        let span = self.current().span;
        self.expect(TokenKind::Return)?;
        let mut values = Vec::new();
        if !self.is_at_end() && !self.check(TokenKind::Semi) && !self.check(TokenKind::RBrace) {
            values.push(self.parse_expression()?);
            while self.match_token(TokenKind::Comma) {
                values.push(self.parse_expression()?);
            }
        }
        Ok(Stmt::new(
            StmtKind::Return(values),
            span.merge(self.prev_span()),
        ))
    }

    fn parse_for(&mut self) -> ParseResult<Stmt> {
        let span = self.current().span;
        self.expect(TokenKind::For)?;
        let pattern = self.parse_pattern()?;
        let iterable = if self.match_token(TokenKind::In) {
            Some(self.parse_expression()?)
        } else {
            None
        };
        self.loop_depth += 1;
        let body = self.parse_block()?;
        self.loop_depth -= 1;
        Ok(Stmt::new(
            StmtKind::For {
                pattern,
                iterable,
                body,
            },
            span.merge(self.prev_span()),
        ))
    }

    pub fn parse_block(&mut self) -> ParseResult<Vec<Stmt>> {
        self.expect(TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            match self.parse_statement() {
                Ok(stmt) => {
                    if stmt.kind.needs_semicolon() {
                        self.match_token(TokenKind::Semi);
                    }
                    stmts.push(stmt);
                }
                Err(e) => {
                    self.errors.push(e);
                    self.synchronize();
                }
            }
        }
        self.expect(TokenKind::RBrace)?;
        Ok(stmts)
    }

    // Stubs for types/patterns
    pub fn parse_type_expr(&mut self) -> ParseResult<TypeExpr> {
        let span = self.current().span;
        let name = self.expect_ident()?;
        Ok(TypeExpr::named(name, span))
    }

    pub fn parse_pattern(&mut self) -> ParseResult<Pattern> {
        let span = self.current().span;
        if self.match_token(TokenKind::Underscore) {
            return Ok(Pattern::wildcard(span));
        }
        let name = self.expect_ident()?;
        Ok(Pattern::ident(name, span))
    }
}
