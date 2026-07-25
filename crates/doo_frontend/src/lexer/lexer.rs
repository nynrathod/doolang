//! Lexer implementation for Doo.
//!
//! Tokenizes UTF-8 source code into a stream of tokens with span information.
//! Supports all Doo syntax including keywords, operators, string/number literals,
//! comments (single-line and multi-line), and proper error handling.
//!
//! ## Architecture
//! Operates directly on `&[u8]` for zero-allocation, rustc-level performance.
//! ASCII characters (which make up 99% of source code) are processed byte-by-byte,
//! only slowing down to decode UTF-8 when encountering multi-byte sequences.

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
        map.insert("const", TokenKind::Const);
        map.insert("static", TokenKind::Static);
        map.insert("impl", TokenKind::Impl);
        map.insert("let", TokenKind::Let);
        map.insert("mut", TokenKind::Mut);
        map.insert("fn", TokenKind::Fn);
        map.insert("use", TokenKind::Use);
        map.insert("import", TokenKind::Import);
        map.insert("as", TokenKind::As);
        map.insert("struct", TokenKind::Struct);
        map.insert("enum", TokenKind::Enum);
        map.insert("interface", TokenKind::Interface);
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
        map.insert("throw", TokenKind::Throw);

        // Error handling & special values
        map.insert("Ok", TokenKind::Ok);
        map.insert("Err", TokenKind::Err);
        map.insert("nil", TokenKind::Nil);
        map.insert("true", TokenKind::True);
        map.insert("false", TokenKind::False);

        // Self type
        map.insert("Self", TokenKind::Self_);

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
    bytes: &'a [u8],
    /// Current byte position in the source.
    pos: usize,
    /// Current byte offset (used for spans).
    byte_offset: u32,
    /// File ID for spans.
    file_id: u32,
}

impl<'a> Lexer<'a> {
    /// Create a new lexer for the given source code.
    pub fn new(source: &'a str, file_id: u32) -> Self {
        Self {
            bytes: source.as_bytes(),
            pos: 0,
            byte_offset: 0,
            file_id,
        }
    }

    /// Tokenize all source code into a vector of tokens.
    pub fn tokenize(&mut self) -> Vec<Token> {
        if self.bytes.len() > MAX_INPUT_SIZE {
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

        let b = match self.peek_byte() {
            Some(b) => b,
            None => return self.make_token(TokenKind::Eof, ""),
        };

        // String literals
        if b == b'"' {
            // Check for triple-quoted multi-line string
            if self.peek_byte_at(1) == Some(b'"') && self.peek_byte_at(2) == Some(b'"') {
                return self.scan_multiline_string();
            }
            return self.scan_string();
        }

        // Numbers
        if b.is_ascii_digit() {
            return self.scan_number();
        }

        // Identifiers and keywords
        if b.is_ascii_alphabetic() || b == b'_' {
            return self.scan_identifier();
        }

        // Operators and punctuation
        self.scan_operator()
    }

    fn skip_whitespace_and_comments(&mut self) {
        while let Some(b) = self.peek_byte() {
            match b {
                b' ' | b'\t' | b'\r' | b'\n' => {
                    self.advance();
                }
                // Single-line comment
                b'/' if self.peek_byte_at(1) == Some(b'/') => {
                    self.advance(); // '/'
                    self.advance(); // '/'
                    while let Some(b) = self.peek_byte() {
                        if b == b'\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                // Multi-line comment
                b'/' if self.peek_byte_at(1) == Some(b'*') => {
                    self.advance(); // '/'
                    self.advance(); // '*'
                    let mut depth = 1;
                    while depth > 0 {
                        if self.is_at_end() {
                            break;
                        }
                        let curr = self.advance().unwrap();
                        if curr == b'/' && self.peek_byte() == Some(b'*') {
                            self.advance();
                            depth += 1;
                        } else if curr == b'*' && self.peek_byte() == Some(b'/') {
                            self.advance();
                            depth -= 1;
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

        while let Some(b) = self.peek_byte() {
            if b == b'"' {
                self.advance(); // Skip closing quote
                let kind = if has_interpolation {
                    TokenKind::StringTemplate
                } else {
                    TokenKind::String
                };
                return self.make_token_at(kind, &value, start_offset);
            }

            length += 1;
            if length > MAX_STRING_LENGTH {
                while let Some(b) = self.peek_byte() {
                    if b == b'"' {
                        self.advance();
                        break;
                    }
                    self.advance();
                }
                return self.make_token_at(
                    TokenKind::Error,
                    "String literal too long",
                    start_offset,
                );
            }

            // Check for interpolation ${...}
            if b == b'$' && self.peek_byte_at(1) == Some(b'{') {
                has_interpolation = true;
                value.push('$');
                value.push('{');
                self.advance();
                self.advance();

                let mut brace_depth = 1;
                while brace_depth > 0 {
                    if self.is_at_end() {
                        return self.make_token_at(
                            TokenKind::Error,
                            "Unterminated string interpolation",
                            start_offset,
                        );
                    }
                    let curr = self.advance().unwrap();
                    match curr {
                        b'{' => brace_depth += 1,
                        b'}' => brace_depth -= 1,
                        _ => {}
                    }
                    value.push(curr as char);
                }
                continue;
            }

            // Handle escape sequences
            if b == b'\\' {
                self.advance(); // Skip backslash
                if let Some(esc) = self.peek_byte() {
                    self.advance();
                    match esc {
                        b'n' => value.push('\n'),
                        b'r' => value.push('\r'),
                        b't' => value.push('\t'),
                        b'\\' => value.push('\\'),
                        b'"' => value.push('"'),
                        b'\'' => value.push('\''),
                        b'0' => value.push('\0'),
                        b'$' => value.push('$'),
                        b'x' => {
                            // \xNN hex escape
                            let mut hex = String::new();
                            for _ in 0..2 {
                                if let Some(h) = self.peek_byte() {
                                    if h.is_ascii_hexdigit() {
                                        hex.push(h as char);
                                        self.advance();
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if let Ok(code) = u8::from_str_radix(&hex, 16) {
                                value.push(code as char);
                            } else {
                                value.push('\\');
                                value.push('x');
                                value.push_str(&hex);
                            }
                        }
                        b'u' => {
                            // \u{NNNN} unicode escape
                            if self.peek_byte() == Some(b'{') {
                                self.advance();
                                let mut hex = String::new();
                                while let Some(h) = self.peek_byte() {
                                    if h == b'}' {
                                        self.advance();
                                        break;
                                    }
                                    if h.is_ascii_hexdigit() && hex.len() < 6 {
                                        hex.push(h as char);
                                        self.advance();
                                    } else {
                                        break;
                                    }
                                }
                                if let Ok(code) = u32::from_str_radix(&hex, 16) {
                                    if let Some(c) = char::from_u32(code) {
                                        value.push(c);
                                    }
                                }
                            }
                        }
                        _ => {
                            value.push('\\');
                            value.push(esc as char);
                        }
                    }
                }
            } else {
                // Normal character
                // We need to handle UTF-8 properly here.
                // Since we are iterating bytes, we must collect multi-byte sequences.
                if b < 0x80 {
                    value.push(b as char);
                    self.advance();
                } else {
                    // UTF-8 multi-byte sequence
                    let utf8_len = if b >= 0xF0 {
                        4
                    } else if b >= 0xE0 {
                        3
                    } else {
                        2
                    };
                    let start = self.pos;
                    for _ in 0..utf8_len {
                        self.advance();
                    }
                    if let Ok(s) = std::str::from_utf8(&self.bytes[start..self.pos]) {
                        value.push_str(s);
                    } else {
                        value.push('?'); // Invalid UTF-8
                    }
                }
            }
        }

        self.make_token_at(TokenKind::Error, "Unterminated string", start_offset)
    }

    fn scan_multiline_string(&mut self) -> Token {
        let start_offset = self.byte_offset;

        // Skip opening """
        self.advance();
        self.advance();
        self.advance();

        // Skip immediate newline after opening """
        if self.peek_byte() == Some(b'\n') {
            self.advance();
        } else if self.peek_byte() == Some(b'\r') {
            self.advance();
            if self.peek_byte() == Some(b'\n') {
                self.advance();
            }
        }

        let mut value = String::new();
        let mut has_interpolation = false;

        loop {
            if self.is_at_end() {
                return self.make_token_at(
                    TokenKind::Error,
                    "Unterminated multi-line string",
                    start_offset,
                );
            }

            // Check for closing """
            if self.peek_byte() == Some(b'"')
                && self.peek_byte_at(1) == Some(b'"')
                && self.peek_byte_at(2) == Some(b'"')
            {
                self.advance();
                self.advance();
                self.advance();
                break;
            }

            // Check for interpolation ${...}
            if self.peek_byte() == Some(b'$') && self.peek_byte_at(1) == Some(b'{') {
                has_interpolation = true;
                value.push('$');
                value.push('{');
                self.advance();
                self.advance();

                let mut brace_depth = 1;
                while brace_depth > 0 {
                    if self.is_at_end() {
                        return self.make_token_at(
                            TokenKind::Error,
                            "Unterminated string interpolation",
                            start_offset,
                        );
                    }
                    let curr = self.advance().unwrap();
                    match curr {
                        b'{' => brace_depth += 1,
                        b'}' => brace_depth -= 1,
                        _ => {}
                    }
                    value.push(curr as char);
                }
                continue;
            }

            // Handle escapes
            if self.peek_byte() == Some(b'\\') {
                self.advance();
                if let Some(esc) = self.peek_byte() {
                    self.advance();
                    match esc {
                        b'n' => value.push('\n'),
                        b'r' => value.push('\r'),
                        b't' => value.push('\t'),
                        b'\\' => value.push('\\'),
                        b'"' => value.push('"'),
                        b'\'' => value.push('\''),
                        b'0' => value.push('\0'),
                        b'$' => value.push('$'),
                        b'x' => {
                            let mut hex = String::new();
                            for _ in 0..2 {
                                if let Some(h) = self.peek_byte() {
                                    if h.is_ascii_hexdigit() {
                                        hex.push(h as char);
                                        self.advance();
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if let Ok(code) = u8::from_str_radix(&hex, 16) {
                                value.push(code as char);
                            }
                        }
                        b'u' => {
                            if self.peek_byte() == Some(b'{') {
                                self.advance();
                                let mut hex = String::new();
                                while let Some(h) = self.peek_byte() {
                                    if h == b'}' {
                                        self.advance();
                                        break;
                                    }
                                    if h.is_ascii_hexdigit() && hex.len() < 6 {
                                        hex.push(h as char);
                                        self.advance();
                                    } else {
                                        break;
                                    }
                                }
                                if let Ok(code) = u32::from_str_radix(&hex, 16) {
                                    if let Some(c) = char::from_u32(code) {
                                        value.push(c);
                                    }
                                }
                            }
                        }
                        _ => {
                            value.push('\\');
                            value.push(esc as char);
                        }
                    }
                }
            } else {
                // Normal character
                let b = self.peek_byte().unwrap();
                if b < 0x80 {
                    value.push(b as char);
                    self.advance();
                } else {
                    let utf8_len = if b >= 0xF0 {
                        4
                    } else if b >= 0xE0 {
                        3
                    } else {
                        2
                    };
                    let start = self.pos;
                    for _ in 0..utf8_len {
                        self.advance();
                    }
                    if let Ok(s) = std::str::from_utf8(&self.bytes[start..self.pos]) {
                        value.push_str(s);
                    } else {
                        value.push('?');
                    }
                }
            }
        }

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

        // Check for hex, binary, octal prefixes
        if self.peek_byte() == Some(b'0') {
            if let Some(prefix) = self.peek_byte_at(1) {
                match prefix {
                    b'x' | b'X' => {
                        self.advance();
                        self.advance();
                        while let Some(b) = self.peek_byte() {
                            if b.is_ascii_hexdigit() {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        let text =
                            std::str::from_utf8(&self.bytes[start_pos..self.pos]).unwrap_or("0");
                        return self.make_token_at(TokenKind::Integer, text, start_offset);
                    }
                    b'b' | b'B' => {
                        self.advance();
                        self.advance();
                        while let Some(b) = self.peek_byte() {
                            if b == b'0' || b == b'1' {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        let text =
                            std::str::from_utf8(&self.bytes[start_pos..self.pos]).unwrap_or("0");
                        return self.make_token_at(TokenKind::Integer, text, start_offset);
                    }
                    b'o' | b'O' => {
                        self.advance();
                        self.advance();
                        while let Some(b) = self.peek_byte() {
                            if b >= b'0' && b <= b'7' {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        let text =
                            std::str::from_utf8(&self.bytes[start_pos..self.pos]).unwrap_or("0");
                        return self.make_token_at(TokenKind::Integer, text, start_offset);
                    }
                    _ => {}
                }
            }
        }

        // Standard integer part
        while let Some(b) = self.peek_byte() {
            if b.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }

        let mut is_float = false;

        // Check for decimal part
        if self.peek_byte() == Some(b'.') {
            if let Some(next) = self.peek_byte_at(1) {
                if next.is_ascii_digit() {
                    is_float = true;
                    self.advance();
                    while let Some(b) = self.peek_byte() {
                        if b.is_ascii_digit() {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
            }
        }

        // Exponent part
        if let Some(b) = self.peek_byte() {
            if b == b'e' || b == b'E' {
                let exp_pos = self.pos;
                self.advance();
                if let Some(b2) = self.peek_byte() {
                    if b2 == b'+' || b2 == b'-' {
                        self.advance();
                    }
                }
                if let Some(b2) = self.peek_byte() {
                    if b2.is_ascii_digit() {
                        is_float = true;
                        while let Some(b) = self.peek_byte() {
                            if b.is_ascii_digit() {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                    } else {
                        self.pos = exp_pos;
                    }
                }
            }
        }

        let text = std::str::from_utf8(&self.bytes[start_pos..self.pos]).unwrap_or("");
        let kind = if is_float {
            TokenKind::Float
        } else {
            TokenKind::Integer
        };
        self.make_token_at(kind, text, start_offset)
    }

    fn scan_identifier(&mut self) -> Token {
        let start_offset = self.byte_offset;
        let start_pos = self.pos;

        while let Some(b) = self.peek_byte() {
            if b.is_ascii_alphanumeric() || b == b'_' {
                self.advance();
                if self.pos - start_pos > MAX_IDENTIFIER_LENGTH {
                    while let Some(b) = self.peek_byte() {
                        if b.is_ascii_alphanumeric() || b == b'_' {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    return self.make_token_at(
                        TokenKind::Error,
                        "Identifier too long",
                        start_offset,
                    );
                }
            } else {
                break;
            }
        }

        let text = std::str::from_utf8(&self.bytes[start_pos..self.pos]).unwrap_or("");

        if text == "_" {
            return self.make_token_at(TokenKind::Underscore, "_", start_offset);
        }

        let kind = keyword_map().get(text).copied().unwrap_or(TokenKind::Ident);

        self.make_token_at(kind, text, start_offset)
    }

    fn scan_operator(&mut self) -> Token {
        let start_offset = self.byte_offset;
        let b = self.peek_byte().unwrap();

        // Try 3-character operators first
        if let Some(b2) = self.peek_byte_at(1) {
            if let Some(b3) = self.peek_byte_at(2) {
                let three = [b, b2, b3];
                let kind = match three {
                    [b'.', b'.', b'='] => Some(TokenKind::DotDotEq),
                    [b'.', b'.', b'.'] => Some(TokenKind::Spread),
                    _ => None,
                };
                if let Some(k) = kind {
                    self.advance();
                    self.advance();
                    self.advance();
                    return self.make_token_at(k, "", start_offset);
                }
            }
        }

        // Try 2-character operators
        if let Some(b2) = self.peek_byte_at(1) {
            let two = [b, b2];
            let kind = match two {
                [b'=', b'='] => Some(TokenKind::EqEq),
                [b'!', b'='] => Some(TokenKind::NotEq),
                [b'<', b'='] => Some(TokenKind::LtEq),
                [b'>', b'='] => Some(TokenKind::GtEq),
                [b'&', b'&'] => Some(TokenKind::AndAnd),
                [b'|', b'|'] => Some(TokenKind::OrOr),
                [b'+', b'+'] => Some(TokenKind::PlusPlus),
                [b'-', b'-'] => Some(TokenKind::MinusMinus),
                [b'+', b'='] => Some(TokenKind::PlusEq),
                [b'-', b'='] => Some(TokenKind::MinusEq),
                [b'*', b'='] => Some(TokenKind::StarEq),
                [b'/', b'='] => Some(TokenKind::SlashEq),
                [b'%', b'='] => Some(TokenKind::PercentEq),
                [b'-', b'>'] => Some(TokenKind::Arrow),
                [b'=', b'>'] => Some(TokenKind::FatArrow),
                [b'.', b'.'] => Some(TokenKind::DotDot),
                [b':', b':'] => Some(TokenKind::ColonColon),
                [b'?', b'?'] => Some(TokenKind::QuestionQuestion),

                _ => None,
            };
            if let Some(k) = kind {
                self.advance();
                self.advance();
                return self.make_token_at(k, "", start_offset);
            }
        }

        // Single character operators
        self.advance();
        let kind = match b {
            b'+' => TokenKind::Plus,
            b'-' => TokenKind::Minus,
            b'*' => TokenKind::Star,
            b'/' => TokenKind::Slash,
            b'%' => TokenKind::Percent,
            b'=' => TokenKind::Eq,
            b'!' => TokenKind::Bang,
            b'<' => TokenKind::Lt,
            b'>' => TokenKind::Gt,
            b'&' => TokenKind::And,
            b'|' => TokenKind::Or,
            b'^' => TokenKind::Caret,
            b'(' => TokenKind::LParen,
            b')' => TokenKind::RParen,
            b'{' => TokenKind::LBrace,
            b'}' => TokenKind::RBrace,
            b'[' => TokenKind::LBracket,
            b']' => TokenKind::RBracket,
            b',' => TokenKind::Comma,
            b';' => TokenKind::Semi,
            b'.' => TokenKind::Dot,
            b':' => TokenKind::Colon,
            b'?' => TokenKind::Question,
            b'@' => TokenKind::At,
            b'#' => TokenKind::Hash,
            b'~' => TokenKind::Tilde,
            b'$' => TokenKind::Dollar,
            _ => TokenKind::Error,
        };

        self.make_token_at(kind, "", start_offset)
    }

    // Helper methods

    #[inline]
    fn is_at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    #[inline]
    fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    #[inline]
    fn peek_byte_at(&self, n: usize) -> Option<u8> {
        self.bytes.get(self.pos + n).copied()
    }

    #[inline]
    fn advance(&mut self) -> Option<u8> {
        let b = self.peek_byte()?;
        self.pos += 1;
        self.byte_offset += 1;
        Some(b)
    }

    #[inline]
    fn make_token(&self, kind: TokenKind, text: &str) -> Token {
        Token {
            kind,
            text: text.to_string(),
            span: Span::new(self.byte_offset, self.byte_offset + text.len() as u32),
        }
    }

    #[inline]
    fn make_token_at(&self, kind: TokenKind, text: &str, start_offset: u32) -> Token {
        Token {
            kind,
            text: text.to_string(),
            span: Span::new(start_offset, self.byte_offset),
        }
    }

    #[inline]
    fn error_token(&self, message: &str) -> Token {
        Token {
            kind: TokenKind::Error,
            text: message.to_string(),
            span: Span::new(self.byte_offset, self.byte_offset),
        }
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Token;

    fn next(&mut self) -> Option<Self::Item> {
        if self.is_at_end() {
            return None;
        }

        let token = self.next_token();

        if token.kind == TokenKind::Eof {
            None
        } else {
            Some(token)
        }
    }
}
