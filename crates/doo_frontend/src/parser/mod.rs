//! Main Parser Implementation
//!
//! Recursive descent parser with Pratt parsing for expressions.
//! Parses Doo tokens into an AST.

pub mod items;
pub mod stmt;
pub mod expr;
pub mod types;

use crate::ast::*;
use crate::lexer::{Token, TokenKind, Lexer};
use doo_core::{Span, CompilerError, ErrorCode};

pub use items::ParserItems;
pub use stmt::ParserStmt;
pub use expr::ParserExpr;
pub use types::ParserTypes;

/// Result type for parser operations.
pub type ParseResult<T> = Result<T, CompilerError>;

/// The Doo parser.
pub struct Parser {
    /// All tokens from lexer.
    pub(super) tokens: Vec<Token>,
    /// Current position in tokens.
    pub(super) pos: usize,
    /// File ID for spans.
    pub(super) file_id: u32,
    /// Collected errors.
    pub(super) errors: Vec<CompilerError>,
    /// EOF token for when we're past the end.
    pub(super) eof_token: Token,
}

impl Parser {
    /// Create a new parser from source code.
    pub fn new(source: &str, file_id: u32) -> Self {
        let mut lexer = Lexer::new(source, file_id);
        let tokens = lexer.tokenize();
        
        Self {
            tokens,
            pos: 0,
            file_id,
            errors: Vec::new(),
            eof_token: Token::new(TokenKind::Eof, "", Span::dummy()),
        }
    }

    /// Create a parser from pre-lexed tokens.
    pub fn from_tokens(tokens: Vec<Token>, file_id: u32) -> Self {
        Self {
            tokens,
            pos: 0,
            file_id,
            errors: Vec::new(),
            eof_token: Token::new(TokenKind::Eof, "", Span::dummy()),
        }
    }

    /// Parse the entire program.
    pub fn parse_program(&mut self) -> ParseResult<Program> {
        let start = self.current_span();
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

        let end = self.current_span();
        Ok(Program::new(items, start.merge(&end)))
    }

    // === Helpers (Internal logic exposed to submodules) ===

    pub(super) fn current(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&self.eof_token)
    }

    pub(super) fn current_span(&self) -> Span {
        self.current().span
    }

    pub(super) fn prev_span(&self) -> Span {
        if self.pos > 0 {
            self.tokens[self.pos - 1].span
        } else {
            Span::dummy()
        }
    }

    pub(super) fn is_at_end(&self) -> bool {
        self.current().kind == TokenKind::Eof
    }

    pub(super) fn check(&self, kind: TokenKind) -> bool {
        self.current().kind == kind
    }

    pub(super) fn advance(&mut self) {
        if !self.is_at_end() {
            self.pos += 1;
        }
    }

    pub(super) fn expect(&mut self, kind: TokenKind) -> ParseResult<()> {
        if self.check(kind) {
            self.advance();
            Ok(())
        } else {
            Err(CompilerError::new(
                ErrorCode::UnexpectedToken,
                format!("Expected {}, got {}", kind, self.current().kind),
                self.current_span(),
            ))
        }
    }

    pub(super) fn expect_ident(&mut self) -> ParseResult<String> {
        if self.check(TokenKind::Ident) {
            let name = self.current().text.clone();
            self.advance();
            Ok(name)
        } else {
            Err(CompilerError::new(
                ErrorCode::ExpectedIdentifier,
                format!("Expected identifier, got {}", self.current().kind),
                self.current_span(),
            ))
        }
    }

    pub(super) fn is_at_stmt_end(&self) -> bool {
        matches!(
            self.current().kind,
            TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof
        )
    }

    pub(super) fn synchronize(&mut self) {
        self.advance();
        while !self.is_at_end() {
            if matches!(
                self.current().kind,
                TokenKind::Fn
                    | TokenKind::Struct
                    | TokenKind::Enum
                    | TokenKind::Import
                    | TokenKind::Let
                    | TokenKind::If
                    | TokenKind::For
                    | TokenKind::Return
            ) {
                return;
            }
            self.advance();
        }
    }

    /// Get all collected errors.
    pub fn errors(&self) -> &[CompilerError] {
        &self.errors
    }

    /// Check if parsing succeeded without errors.
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Program {
        let mut parser = Parser::new(source, 0);
        parser.parse_program().unwrap()
    }

    #[test]
    fn test_empty_program() {
        let prog = parse("");
        assert!(prog.items.is_empty());
    }

    #[test]
    fn test_function_decl() {
        let prog = parse("fn add(a: Int, b: Int) -> Int { return a + b }");
        assert_eq!(prog.items.len(), 1);
        if let Item::Function(f) = &prog.items[0] {
            assert_eq!(f.name, "add");
            assert_eq!(f.params.len(), 2);
        } else {
            panic!("Expected function");
        }
    }

    #[test]
    fn test_struct_decl() {
        let prog = parse("struct User { name: Str, age: Int }");
        assert_eq!(prog.items.len(), 1);
        if let Item::Struct(s) = &prog.items[0] {
            assert_eq!(s.name, "User");
            assert_eq!(s.fields.len(), 2);
        } else {
            panic!("Expected struct");
        }
    }

    #[test]
    fn test_let_statement() {
        let prog = parse("let x = 42");
        assert_eq!(prog.items.len(), 1);
    }

    #[test]
    fn test_binary_expr() {
        let prog = parse("let x = 1 + 2 * 3");
        assert_eq!(prog.items.len(), 1);
    }

    #[test]
    fn test_function_call() {
        let prog = parse("foo(1, 2, 3)");
        assert_eq!(prog.items.len(), 1);
    }

    #[test]
    fn test_method_call() {
        let prog = parse("obj.method(arg)");
        assert_eq!(prog.items.len(), 1);
    }

    #[test]
    fn test_array_literal() {
        let prog = parse("let arr = [1, 2, 3]");
        assert_eq!(prog.items.len(), 1);
    }

    #[test]
    fn test_if_statement() {
        let prog = parse("if x > 0 { print(x) }");
        assert_eq!(prog.items.len(), 1);
    }

    #[test]
    fn test_for_loop() {
        let prog = parse("for i in 1..10 { print(i) }");
        assert_eq!(prog.items.len(), 1);
    }

    #[test]
    fn test_decorator() {
        let prog = parse("@table struct User { name: Str }");
        assert_eq!(prog.items.len(), 1);
        if let Item::Struct(s) = &prog.items[0] {
            assert_eq!(s.decorators.len(), 1);
            assert_eq!(s.decorators[0].name, "table");
        }
    }
}
