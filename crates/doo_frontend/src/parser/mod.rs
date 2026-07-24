//! Main Parser Implementation
//!
//! Recursive descent parser with Pratt parsing for expressions.
//! Parses Doo tokens into an AST.

pub mod expr;
pub mod helpers;
pub mod items;
pub mod stmt;
pub mod types;

use crate::ast::*;
use crate::lexer::{Lexer, Token, TokenKind};
use doo_core::{CompilerError, ErrorCode, Span};

pub use expr::ParserExpr;
pub use items::ParserItems;
pub use stmt::ParserStmt;
pub use types::ParserTypes;

/// Maximum expression recursion depth (stack overflow protection).
/// Kept conservative (128) for Windows 1MB default thread stack in debug builds.
const MAX_EXPR_DEPTH: u32 = 128;

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
    /// Loop nesting depth (for break/continue validation).
    pub(super) loop_depth: u32,
    /// Function nesting depth (for return validation).
    pub(super) fn_depth: u32,
    /// Expression recursion depth (stack overflow protection).
    pub(super) expr_depth: u32,
}

impl Parser {
    /// Create a new parser from source code.
    pub fn new(source: &str, file_id: u32) -> Self {
        let mut lexer = Lexer::new(source, file_id);
        let tokens = lexer.tokenize();

        let mut parser = Self {
            tokens,
            pos: 0,
            file_id,
            errors: Vec::new(),
            eof_token: Token::new(TokenKind::Eof, "", Span::dummy()),
            loop_depth: 0,
            fn_depth: 0,
            expr_depth: 0,
        };

        // Pre-scan: convert any lexer Error tokens to CompilerErrors
        parser.collect_lexer_errors();
        parser
    }

    /// Create a parser from pre-lexed tokens.
    pub fn from_tokens(tokens: Vec<Token>, file_id: u32) -> Self {
        let mut parser = Self {
            tokens,
            pos: 0,
            file_id,
            errors: Vec::new(),
            eof_token: Token::new(TokenKind::Eof, "", Span::dummy()),
            loop_depth: 0,
            fn_depth: 0,
            expr_depth: 0,
        };
        parser.collect_lexer_errors();
        parser
    }

    /// Scan tokens for lexer Error markers and collect as CompilerErrors.
    fn collect_lexer_errors(&mut self) {
        for token in &self.tokens {
            if token.kind == TokenKind::Error {
                let (code, msg) = if token.text.contains("Unterminated string") {
                    (
                        ErrorCode::UnterminatedString,
                        "unterminated string literal — missing closing `\"`".to_string(),
                    )
                } else if token.text.starts_with("Invalid escape sequence") {
                    (ErrorCode::InvalidEscapeSequence, token.text.clone())
                } else if token.text.contains("String literal too long") {
                    (ErrorCode::InvalidStringLiteral, token.text.clone())
                } else if token.text.contains("Unexpected character")
                    || token.text.contains("Invalid character")
                {
                    (
                        ErrorCode::InvalidCharacter,
                        format!("invalid character in source: {}", token.text),
                    )
                } else if token.text.contains("too large") || token.text.contains("too many") {
                    (ErrorCode::InternalError, token.text.clone())
                } else {
                    (
                        ErrorCode::InvalidExpression,
                        format!("lexer error: {}", token.text),
                    )
                };
                self.errors.push(CompilerError::new(code, msg, token.span));
            }
        }
    }

    /// Parse the entire program.
    pub fn parse_program(&mut self) -> ParseResult<Program> {
        let start = self.current_span();
        let mut items = Vec::new();

        while !self.is_at_end() {
            match self.parse_item() {
                Ok(item) => {
                    // Enforce mandatory semicolons for top-level items that need them
                    if item.needs_semicolon() {
                        if self.check(TokenKind::Semi) {
                            self.advance();
                        } else {
                            self.errors.push(
                                CompilerError::new(
                                    ErrorCode::MissingSemicolon,
                                    "expected `;` after declaration",
                                    self.prev_span(),
                                )
                                .with_suggestion("add `;` at the end of this declaration"),
                            );
                        }
                    } else {
                        // Block-ending items: consume optional semicolon
                        if self.check(TokenKind::Semi) {
                            self.advance();
                        }
                    }
                    items.push(item);
                }
                Err(e) => {
                    self.errors.push(e);
                    self.synchronize();
                }
            }
        }

        let end = self.current_span();
        Ok(Program::new(items, start.merge(end)))
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
            // Detect unexpected EOF
            if self.is_at_end() {
                let code = match kind {
                    TokenKind::RParen => ErrorCode::MissingClosingParen,
                    TokenKind::RBrace => ErrorCode::MissingClosingBrace,
                    TokenKind::RBracket => ErrorCode::MissingClosingBracket,
                    _ => ErrorCode::UnexpectedEof,
                };
                return Err(CompilerError::new(
                    code,
                    format!("unexpected end of file, expected `{}`", kind),
                    self.current_span(),
                ));
            }

            // Use specific error codes for closing delimiters
            let (code, msg) = match kind {
                TokenKind::RParen => (
                    ErrorCode::MissingClosingParen,
                    format!("Missing `)`, got `{}`", self.current().kind),
                ),
                TokenKind::RBrace => (
                    ErrorCode::MissingClosingBrace,
                    format!("Missing `}}`, got `{}`", self.current().kind),
                ),
                TokenKind::RBracket => (
                    ErrorCode::MissingClosingBracket,
                    format!("Missing `]`, got `{}`", self.current().kind),
                ),
                TokenKind::FatArrow => (
                    ErrorCode::UnexpectedToken,
                    format!("Expected `=>`, got `{}`", self.current().kind),
                ),
                TokenKind::Eq => (
                    ErrorCode::UnexpectedToken,
                    format!("Expected `=`, got `{}`", self.current().kind),
                ),
                TokenKind::Colon => (
                    ErrorCode::ExpectedTypeAnnotation,
                    format!(
                        "Expected `:` for type annotation, got `{}`",
                        self.current().kind
                    ),
                ),
                TokenKind::LBrace => (
                    ErrorCode::ExpectedBlock,
                    format!(
                        "Expected `{{` to start block, got `{}`",
                        self.current().kind
                    ),
                ),
                TokenKind::Semi => (
                    ErrorCode::MissingSemicolon,
                    format!("missing `;` — got `{}`", self.current().kind),
                ),
                TokenKind::Comma => (
                    ErrorCode::UnexpectedToken,
                    format!("Expected `,`, got `{}`", self.current().kind),
                ),
                _ => (
                    ErrorCode::UnexpectedToken,
                    format!("Expected `{}`, got `{}`", kind, self.current().kind),
                ),
            };
            Err(CompilerError::new(code, msg, self.current_span()))
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
                // Inside a function body, `fn` is a nested declaration,
                // not a recovery point. Skip it to avoid consuming the
                // next top-level function as part of the current body.
                if self.current().kind == TokenKind::Fn && self.fn_depth > 0 {
                    self.advance();
                    continue;
                }
                return;
            }
            // Also stop at `}` — the closing brace of the current block
            // is a natural recovery point.
            if self.current().kind == TokenKind::RBrace {
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

    /// Peek at next token (for lookahead).
    pub(super) fn peek_next(&self) -> &Token {
        self.tokens.get(self.pos + 1).unwrap_or(&self.eof_token)
    }

    /// Check if the next token (after current) is of the given kind.
    pub(super) fn peek_is(&self, kind: TokenKind) -> bool {
        self.peek_next().kind == kind
    }

    /// Check if the `{` after an uppercase identifier contains struct fields
    /// (e.g., `Ident: value` or `Ident,` or `}`) rather than code statements.
    /// Used to disambiguate struct literals from block openings like if-bodies.
    pub(super) fn is_struct_literal_body(&self) -> bool {
        // After `{`, if next token is `}` (empty struct), it's a struct.
        if self.peek_is(TokenKind::RBrace) {
            return true;
        }
        // Check 2 ahead: after `{`, the first token should be an identifier
        // followed by `:` or `,` or `}` for it to be a struct field.
        // If it's `return`, `let`, `if`, `for`, etc., it's a code block.
        let after_brace = self.peek_next();
        if after_brace.kind == TokenKind::Ident {
            // Peek 2 ahead: after the identifier, check for `:` (typed field)
            // or `,`/`}` (shorthand field / end of struct)
            if self.pos + 2 < self.tokens.len() {
                let after_ident = &self.tokens[self.pos + 2];
                return matches!(after_ident.kind, TokenKind::Colon | TokenKind::Comma | TokenKind::RBrace);
            }
            return true;
        }
        // Not an identifier — must be a block (return, let, if, etc.)
        false
    }

    /// Parse expression with precedence (internal helper).
    pub(super) fn parse_expression_prec(&mut self, min_prec: u8) -> ParseResult<Expr> {
        // Stack overflow protection
        self.expr_depth += 1;
        if self.expr_depth > MAX_EXPR_DEPTH {
            self.expr_depth -= 1;
            return Err(CompilerError::new(
                ErrorCode::InternalError,
                "expression nesting too deep (max 256)",
                self.current_span(),
            )
            .with_suggestion("simplify the expression or break it into smaller parts"));
        }

        use expr::ParserExpr;
        let mut left = self.parse_unary()?;
        left = self.parse_postfix(left)?;
        left = self.parse_binary_op(left, min_prec)?;
        left = self.parse_range(left)?;

        self.expr_depth -= 1;
        Ok(left)
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
