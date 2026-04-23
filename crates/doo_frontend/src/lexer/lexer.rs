//! Lexer implementation for Doo.
//!
//! Tokenizes UTF-8 source code into a stream of tokens with span information.
//! Supports all Doo syntax including keywords, operators, string/number literals,
//! comments (single-line and multi-line), and proper error handling.

use super::token::{Token, TokenKind};
use doo_core::Span;
use rustc_hash::FxHashMap;
use std::sync::OnceLock;

/// Maximum input size (10MB) to prevent DoS attacks.
const MAX_INPUT_SIZE: usize = 10 * 1024 * 1024;
/// Maximum token count to prevent memory exhaustion.
const MAX_TOKEN_COUNT: usize = 1_000_000;
/// Maximum string literal length.
const MAX_STRING_LENGTH: usize = 100_000;
/// Maximum identifier length.
const MAX_IDENTIFIER_LENGTH: usize = 1000;

/// Global static keyword map — built once, shared across all lexer instances.
fn keyword_map() -> &'static FxHashMap<&'static str, TokenKind> {
    static KEYWORDS: OnceLock<FxHashMap<&'static str, TokenKind>> = OnceLock::new();
    KEYWORDS.get_or_init(|| {
        let mut map = FxHashMap::default();

        // Declaration keywords
        map.insert("let", TokenKind::Let);
        map.insert("mut", TokenKind::Mut);
        map.insert("fn", TokenKind::Fn);
        map.insert("import", TokenKind::Import);
        map.insert("as", TokenKind::As);
        map.insert("struct", TokenKind::Struct);
        map.insert("enum", TokenKind::Enum);
        map.insert("match", TokenKind::Match);

        // Control flow
        map.insert("if", TokenKind::If);
        map.insert("else", TokenKind::Else);
        map.insert("for", TokenKind::For);
        map.insert("in", TokenKind::In);

        // Statement keywords
        map.insert("return", TokenKind::Return);
        map.insert("break", TokenKind::Break);
        map.insert("continue", TokenKind::Continue);
        map.insert("print", TokenKind::Print);

        // Error handling & special values
        map.insert("Ok", TokenKind::Ok);
        map.insert("Err", TokenKind::Err);
        map.insert("nil", TokenKind::Nil);
        map.insert("true", TokenKind::True);
        map.insert("false", TokenKind::False);

        // RBAC
        map.insert("policy", TokenKind::Policy);

        // Async & concurrency
        map.insert("async", TokenKind::Async);
        map.insert("await", TokenKind::Await);
        map.insert("go", TokenKind::Go);
        map.insert("scope", TokenKind::Scope);

        map
    })
}

/// The Doo lexer.
pub struct Lexer<'a> {
    /// Source code as UTF-8 bytes.
    source: &'a str,
    /// Characters for iteration (handles UTF-8 properly).
    chars: Vec<char>,
    /// Current position in chars.
    pos: usize,
    /// Current line (1-indexed).
    line: u32,
    /// Current column (1-indexed).
    col: u32,
    /// Byte offset for spans.
    byte_offset: u32,
    /// File ID for spans.
    file_id: u32,
}

impl<'a> Lexer<'a> {
    /// Create a new lexer for the given source code.
    pub fn new(source: &'a str, file_id: u32) -> Self {
        let chars: Vec<char> = source.chars().collect();

        Self {
            source,
            chars,
            pos: 0,
            line: 1,
            col: 1,
            byte_offset: 0,
            file_id,
        }
    }

    /// Tokenize all source code into a vector of tokens.
    pub fn tokenize(&mut self) -> Vec<Token> {
        // Validate input size
        if self.source.len() > MAX_INPUT_SIZE {
            return vec![self.error_token("Input too large")];
        }

        let mut tokens = Vec::new();

        loop {
            if tokens.len() >= MAX_TOKEN_COUNT {
                tokens.push(self.error_token("Too many tokens"));
                break;
            }

            let token = self.next_token();
            let is_eof = token.kind == TokenKind::Eof;
            tokens.push(token);

            if is_eof {
                break;
            }
        }

        tokens
    }

    /// Get the next token.
    pub fn next_token(&mut self) -> Token {
        self.skip_whitespace_and_comments();

        if self.is_at_end() {
            return self.make_token(TokenKind::Eof, "");
        }

        let c = self.current();

        // String literals
        if c == '"' {
            return self.scan_string();
        }

        // Numbers
        if c.is_ascii_digit() {
            return self.scan_number();
        }

        // Identifiers and keywords
        if c.is_alphabetic() || c == '_' {
            return self.scan_identifier();
        }

        // Operators and punctuation
        self.scan_operator()
    }

    fn skip_whitespace_and_comments(&mut self) {
        while !self.is_at_end() {
            let c = self.current();

            match c {
                // Whitespace
                ' ' | '\t' | '\r' => {
                    self.advance();
                }
                '\n' => {
                    self.advance();
                    self.line += 1;
                    self.col = 1;
                }
                // Single-line comment
                '/' if self.peek() == Some('/') => {
                    self.advance(); // '/'
                    self.advance(); // '/'
                    while !self.is_at_end() && self.current() != '\n' {
                        self.advance();
                    }
                }
                // Multi-line comment
                '/' if self.peek() == Some('*') => {
                    self.advance(); // '/'
                    self.advance(); // '*'
                    let mut depth = 1;
                    while !self.is_at_end() && depth > 0 {
                        if self.current() == '/' && self.peek() == Some('*') {
                            self.advance();
                            self.advance();
                            depth += 1;
                        } else if self.current() == '*' && self.peek() == Some('/') {
                            self.advance();
                            self.advance();
                            depth -= 1;
                        } else if self.current() == '\n' {
                            self.advance();
                            self.line += 1;
                            self.col = 1;
                        } else {
                            self.advance();
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn scan_string(&mut self) -> Token {
        let start_offset = self.byte_offset;
        self.advance(); // Skip opening quote

        let mut value = String::new();
        let mut length = 0;
        let mut has_interpolation = false;

        while !self.is_at_end() && self.current() != '"' {
            length += 1;
            if length > MAX_STRING_LENGTH {
                // Skip to end of string and return error
                while !self.is_at_end() && self.current() != '"' {
                    if self.current() == '\n' {
                        self.line += 1;
                        self.col = 1;
                    }
                    self.advance();
                }
                if !self.is_at_end() {
                    self.advance(); // Skip closing quote
                }
                return self.make_token_at(
                    TokenKind::Error,
                    "String literal too long",
                    start_offset,
                );
            }

            // Check for interpolation ${...}
            if self.current() == '$' && self.peek() == Some('{') {
                has_interpolation = true;
                value.push(self.current());
                self.advance();
                value.push(self.current());
                self.advance();
                // Include everything until matching }
                let mut brace_depth = 1;
                while !self.is_at_end() && brace_depth > 0 {
                    let c = self.current();
                    if c == '{' {
                        brace_depth += 1;
                    } else if c == '}' {
                        brace_depth -= 1;
                    } else if c == '\n' {
                        self.line += 1;
                        self.col = 1;
                    }
                    value.push(c);
                    self.advance();
                }
                continue;
            }

            if self.current() == '\\' && !self.is_at_end() {
                self.advance(); // Skip backslash
                if !self.is_at_end() {
                    let escaped = match self.current() {
                        'n' => Some('\n'),
                        'r' => Some('\r'),
                        't' => Some('\t'),
                        '\\' => Some('\\'),
                        '"' => Some('"'),
                        '0' => Some('\0'),
                        '$' => Some('$'), // Allow escaping $ to prevent interpolation
                        'u' => {
                            // Unicode escape: \u{XXXX}
                            self.advance(); // skip 'u'
                            if !self.is_at_end() && self.current() == '{' {
                                self.advance(); // skip '{'
                                let mut hex = String::new();
                                while !self.is_at_end() && self.current() != '}' && hex.len() < 6 {
                                    hex.push(self.current());
                                    self.advance();
                                }
                                if !self.is_at_end() && self.current() == '}' {
                                    self.advance(); // skip '}'
                                    if let Ok(code) = u32::from_str_radix(&hex, 16) {
                                        if let Some(c) = char::from_u32(code) {
                                            value.push(c);
                                            continue; // already advanced past '}'
                                        }
                                    }
                                }
                                // Invalid unicode escape — skip to end of string
                                while !self.is_at_end() && self.current() != '"' {
                                    if self.current() == '\n' {
                                        self.line += 1;
                                        self.col = 1;
                                    }
                                    self.advance();
                                }
                                if !self.is_at_end() {
                                    self.advance();
                                }
                                return self.make_token_at(
                                    TokenKind::Error,
                                    &format!("Invalid unicode escape: \\u{{{}}}", hex),
                                    start_offset,
                                );
                            } else {
                                // \u without { — invalid
                                while !self.is_at_end() && self.current() != '"' {
                                    if self.current() == '\n' {
                                        self.line += 1;
                                        self.col = 1;
                                    }
                                    self.advance();
                                }
                                if !self.is_at_end() {
                                    self.advance();
                                }
                                return self.make_token_at(
                                    TokenKind::Error,
                                    "Invalid escape sequence: \\u (expected \\u{XXXX})",
                                    start_offset,
                                );
                            }
                        }
                        _ => None,
                    };
                    if let Some(c) = escaped {
                        value.push(c);
                        self.advance();
                    } else {
                        // Invalid escape sequence — skip to end of string and return error
                        let bad = self.current();
                        while !self.is_at_end() && self.current() != '"' {
                            if self.current() == '\n' {
                                self.line += 1;
                                self.col = 1;
                            }
                            self.advance();
                        }
                        if !self.is_at_end() {
                            self.advance(); // Skip closing quote
                        }
                        return self.make_token_at(
                            TokenKind::Error,
                            &format!("Invalid escape sequence: \\{}", bad),
                            start_offset,
                        );
                    }
                }
            } else if self.current() == '\n' {
                self.line += 1;
                self.col = 1;
                value.push(self.current());
                self.advance();
            } else {
                value.push(self.current());
                self.advance();
            }
        }

        if self.is_at_end() {
            return self.make_token_at(TokenKind::Error, "Unterminated string", start_offset);
        }

        self.advance(); // Skip closing quote

        // Return StringTemplate if has interpolation, otherwise regular String
        let kind = if has_interpolation {
            TokenKind::StringTemplate
        } else {
            TokenKind::String
        };

        self.make_token_at(kind, &value, start_offset)
    }

    fn scan_number(&mut self) -> Token {
        let start_offset = self.byte_offset;
        let start_pos = self.pos;

        // Integer part
        while !self.is_at_end() && self.current().is_ascii_digit() {
            self.advance();
        }

        let mut is_float = false;

        // Check for decimal part (but not range operators)
        if !self.is_at_end() && self.current() == '.' {
            // Look ahead to distinguish float from range
            if let Some(next) = self.peek() {
                if next.is_ascii_digit() {
                    is_float = true;
                    self.advance(); // '.'
                    while !self.is_at_end() && self.current().is_ascii_digit() {
                        self.advance();
                    }
                }
                // If next is '.', this is range operator, don't consume
            }
        }

        // Exponent part
        if !self.is_at_end() && (self.current() == 'e' || self.current() == 'E') {
            let exp_pos = self.pos;
            self.advance();

            if !self.is_at_end() && (self.current() == '+' || self.current() == '-') {
                self.advance();
            }

            if !self.is_at_end() && self.current().is_ascii_digit() {
                is_float = true;
                while !self.is_at_end() && self.current().is_ascii_digit() {
                    self.advance();
                }
            } else {
                // Invalid exponent, rewind
                self.pos = exp_pos;
            }
        }

        let text: String = self.chars[start_pos..self.pos].iter().collect();
        let kind = if is_float {
            TokenKind::Float
        } else {
            TokenKind::Integer
        };

        self.make_token_at(kind, &text, start_offset)
    }

    fn scan_identifier(&mut self) -> Token {
        let start_offset = self.byte_offset;
        let start_pos = self.pos;
        let mut length = 0;

        while !self.is_at_end() && (self.current().is_alphanumeric() || self.current() == '_') {
            length += 1;
            if length > MAX_IDENTIFIER_LENGTH {
                // Skip rest and return error
                while !self.is_at_end()
                    && (self.current().is_alphanumeric() || self.current() == '_')
                {
                    self.advance();
                }
                return self.make_token_at(TokenKind::Error, "Identifier too long", start_offset);
            }
            self.advance();
        }

        let text: String = self.chars[start_pos..self.pos].iter().collect();

        // Check for underscore pattern (wildcard)
        if text == "_" {
            return self.make_token_at(TokenKind::Underscore, "_", start_offset);
        }

        // Look up keyword from global static map
        let kind = keyword_map()
            .get(text.as_str())
            .copied()
            .unwrap_or(TokenKind::Ident);

        self.make_token_at(kind, &text, start_offset)
    }

    fn scan_operator(&mut self) -> Token {
        let start_offset = self.byte_offset;
        let c = self.current();

        // Try 3-character operators first
        if self.pos + 2 < self.chars.len() {
            let three: String = self.chars[self.pos..self.pos + 3].iter().collect();
            let kind = match three.as_str() {
                "..=" => Some(TokenKind::DotDotEq),
                "..." => Some(TokenKind::Spread),
                _ => None,
            };
            if let Some(k) = kind {
                self.advance();
                self.advance();
                self.advance();
                return self.make_token_at(k, &three, start_offset);
            }
        }

        // Try 2-character operators
        if let Some(next) = self.peek() {
            let two = format!("{}{}", c, next);
            let kind = match two.as_str() {
                "==" => Some(TokenKind::EqEq),
                "!=" => Some(TokenKind::NotEq),
                "<=" => Some(TokenKind::LtEq),
                ">=" => Some(TokenKind::GtEq),
                "&&" => Some(TokenKind::AndAnd),
                "||" => Some(TokenKind::OrOr),
                "++" => Some(TokenKind::PlusPlus),
                "--" => Some(TokenKind::MinusMinus),
                "+=" => Some(TokenKind::PlusEq),
                "-=" => Some(TokenKind::MinusEq),
                "*=" => Some(TokenKind::StarEq),
                "/=" => Some(TokenKind::SlashEq),
                "%=" => Some(TokenKind::PercentEq),
                "->" => Some(TokenKind::Arrow),
                "=>" => Some(TokenKind::FatArrow),
                ".." => Some(TokenKind::DotDot),
                "::" => Some(TokenKind::ColonColon),
                "??" => Some(TokenKind::QuestionQuestion),
                _ => None,
            };
            if let Some(k) = kind {
                self.advance();
                self.advance();
                return self.make_token_at(k, &two, start_offset);
            }
        }

        // Single character operators
        self.advance();
        let kind = match c {
            '+' => TokenKind::Plus,
            '-' => TokenKind::Minus,
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,
            '%' => TokenKind::Percent,
            '=' => TokenKind::Eq,
            '!' => TokenKind::Bang,
            '<' => TokenKind::Lt,
            '>' => TokenKind::Gt,
            '&' => TokenKind::And,
            '|' => TokenKind::Or,
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            ',' => TokenKind::Comma,
            ';' => TokenKind::Semi,
            '.' => TokenKind::Dot,
            ':' => TokenKind::Colon,
            '?' => TokenKind::Question,
            '@' => TokenKind::At,
            '#' => TokenKind::Hash,
            '~' => TokenKind::Tilde,
            '$' => TokenKind::Dollar,
            '_' => TokenKind::Underscore,
            _ => TokenKind::Error,
        };

        let text = c.to_string();
        self.make_token_at(kind, &text, start_offset)
    }

    // Helper methods

    fn is_at_end(&self) -> bool {
        self.pos >= self.chars.len()
    }

    fn current(&self) -> char {
        self.chars.get(self.pos).copied().unwrap_or('\0')
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn advance(&mut self) {
        if !self.is_at_end() {
            let c = self.current();
            self.byte_offset += c.len_utf8() as u32;
            self.col += 1;
            self.pos += 1;
        }
    }

    fn make_token(&self, kind: TokenKind, text: &str) -> Token {
        let span = Span::new(
            self.file_id,
            self.byte_offset,
            self.byte_offset + text.len() as u32,
        );
        Token::new(kind, text, span)
    }

    fn make_token_at(&self, kind: TokenKind, text: &str, start_offset: u32) -> Token {
        let span = Span::new(self.file_id, start_offset, self.byte_offset);
        Token::new(kind, text, span)
    }

    fn error_token(&self, message: &str) -> Token {
        let span = Span::new(self.file_id, self.byte_offset, self.byte_offset);
        Token::new(TokenKind::Error, message, span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(source: &str) -> Vec<Token> {
        let mut lexer = Lexer::new(source, 0);
        lexer.tokenize()
    }

    fn token_kinds(tokens: &[Token]) -> Vec<TokenKind> {
        tokens.iter().map(|t| t.kind).collect()
    }

    #[test]
    fn test_empty() {
        let tokens = lex("");
        assert_eq!(token_kinds(&tokens), vec![TokenKind::Eof]);
    }

    #[test]
    fn test_whitespace() {
        let tokens = lex("   \n\t  ");
        assert_eq!(token_kinds(&tokens), vec![TokenKind::Eof]);
    }

    #[test]
    fn test_keywords() {
        let tokens = lex("let mut fn if else for in return break continue");
        assert_eq!(
            token_kinds(&tokens),
            vec![
                TokenKind::Let,
                TokenKind::Mut,
                TokenKind::Fn,
                TokenKind::If,
                TokenKind::Else,
                TokenKind::For,
                TokenKind::In,
                TokenKind::Return,
                TokenKind::Break,
                TokenKind::Continue,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_identifiers() {
        let tokens = lex("foo bar MyStruct camelCase");
        assert_eq!(tokens[0].kind, TokenKind::Ident);
        assert_eq!(tokens[0].text, "foo");
        assert_eq!(tokens[1].text, "bar");
        assert_eq!(tokens[2].text, "MyStruct");
        assert_eq!(tokens[3].text, "camelCase");
    }

    #[test]
    fn test_numbers() {
        let tokens = lex("123 3.14 1e10 2.5e-3");
        assert_eq!(tokens[0].kind, TokenKind::Integer);
        assert_eq!(tokens[0].text, "123");
        assert_eq!(tokens[1].kind, TokenKind::Float);
        assert_eq!(tokens[1].text, "3.14");
        assert_eq!(tokens[2].kind, TokenKind::Float);
        assert_eq!(tokens[3].kind, TokenKind::Float);
    }

    #[test]
    fn test_string() {
        let tokens = lex(r#""hello world""#);
        assert_eq!(tokens[0].kind, TokenKind::String);
        assert_eq!(tokens[0].text, "hello world");
    }

    #[test]
    fn test_string_escape() {
        let tokens = lex(r#""say \"hello\"""#);
        assert_eq!(tokens[0].kind, TokenKind::String);
        assert_eq!(tokens[0].text, "say \"hello\"");
    }

    #[test]
    fn test_operators() {
        let tokens = lex("+ - * / % == != < > <= >= && ||");
        let kinds = token_kinds(&tokens);
        assert!(kinds.contains(&TokenKind::Plus));
        assert!(kinds.contains(&TokenKind::EqEq));
        assert!(kinds.contains(&TokenKind::AndAnd));
    }

    #[test]
    fn test_delimiters() {
        let tokens = lex("( ) { } [ ]");
        assert_eq!(tokens[0].kind, TokenKind::LParen);
        assert_eq!(tokens[1].kind, TokenKind::RParen);
        assert_eq!(tokens[2].kind, TokenKind::LBrace);
        assert_eq!(tokens[3].kind, TokenKind::RBrace);
        assert_eq!(tokens[4].kind, TokenKind::LBracket);
        assert_eq!(tokens[5].kind, TokenKind::RBracket);
    }

    #[test]
    fn test_range_operators() {
        let tokens = lex("1..10 1..=10 ...arr");
        assert_eq!(tokens[0].kind, TokenKind::Integer);
        assert_eq!(tokens[1].kind, TokenKind::DotDot);
        assert_eq!(tokens[2].kind, TokenKind::Integer);
        assert_eq!(tokens[3].kind, TokenKind::Integer);
        assert_eq!(tokens[4].kind, TokenKind::DotDotEq);
        assert_eq!(tokens[5].kind, TokenKind::Integer);
        assert_eq!(tokens[6].kind, TokenKind::Spread);
    }

    #[test]
    fn test_arrows() {
        let tokens = lex("-> =>");
        assert_eq!(tokens[0].kind, TokenKind::Arrow);
        assert_eq!(tokens[1].kind, TokenKind::FatArrow);
    }

    #[test]
    fn test_comments() {
        let tokens = lex("a // comment\nb");
        assert_eq!(tokens[0].kind, TokenKind::Ident);
        assert_eq!(tokens[0].text, "a");
        assert_eq!(tokens[1].kind, TokenKind::Ident);
        assert_eq!(tokens[1].text, "b");
    }

    #[test]
    fn test_multiline_comments() {
        let tokens = lex("a /* block\ncomment */ b");
        assert_eq!(tokens[0].kind, TokenKind::Ident);
        assert_eq!(tokens[0].text, "a");
        assert_eq!(tokens[1].kind, TokenKind::Ident);
        assert_eq!(tokens[1].text, "b");
    }

    #[test]
    fn test_decorator() {
        let tokens = lex("@email @min(8)");
        assert_eq!(tokens[0].kind, TokenKind::At);
        assert_eq!(tokens[1].kind, TokenKind::Ident);
        assert_eq!(tokens[1].text, "email");
    }

    #[test]
    fn test_real_code() {
        let code = r#"
fn add(a: Int, b: Int) -> Int {
    return a + b
}
"#;
        let tokens = lex(code);
        let kinds = token_kinds(&tokens);
        assert!(kinds.contains(&TokenKind::Fn));
        assert!(kinds.contains(&TokenKind::Ident));
        assert!(kinds.contains(&TokenKind::Arrow));
        assert!(kinds.contains(&TokenKind::Return));
    }
}
