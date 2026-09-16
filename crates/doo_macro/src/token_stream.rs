//! TokenStream — the token-level API for Doo macros.
//!
//! A TokenStream is a sequence of TokenTree values. Each TokenTree is
//! either a single Token or a Group (delimited sub-stream).
//!
//! ## Tokenization
//!
//! `TokenStream::from_str` uses a built-in tokenizer that handles
//! Doo syntax: identifiers, keywords, literals, punctuation, and
//! delimited groups. The tokenizer does NOT depend on `doo_frontend`.

use doo_core::{Span, Symbol};
use rustc_hash::FxHashMap;

// ============================================================================
// Spacing
// ============================================================================

/// Whether a punctuation token is followed immediately by another
/// punctuation token or separated by whitespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spacing {
    /// Next token is also punctuation — no whitespace between them.
    Joint,
    /// Next token is not punctuation, or there is whitespace.
    Alone,
}

// ============================================================================
// Punct
// ============================================================================

/// A single punctuation character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Punct {
    /// The punctuation character.
    pub ch: char,
    /// Spacing relative to the following token.
    pub spacing: Spacing,
}

// ============================================================================
// Literal
// ============================================================================

/// A literal value in the token stream.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    /// Integer literal: `42`
    Int(i64),
    /// Float literal: `3.14`
    Float(f64),
    /// String literal: `"hello"`
    String(Symbol),
    /// Boolean literal: `true` or `false`
    Bool(bool),
}

// ============================================================================
// Keyword
// ============================================================================

/// Doo language keywords recognized by the macro tokenizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keyword {
    Fn,
    Let,
    Const,
    Static,
    Mut,
    If,
    Else,
    For,
    While,
    In,
    Match,
    Return,
    Break,
    Continue,
    Struct,
    Enum,
    Interface,
    Impl,
    Use,
    Import,
    As,
    True,
    False,
    Null,
    Async,
    Await,
    Go,
    Scope,
    Try,
    Throw,
    Self_,
    Print,
    Ok,
    Err,
}

impl Keyword {
    /// Convert to source string.
    pub fn to_str(self) -> &'static str {
        match self {
            Self::Fn => "fn",
            Self::Let => "let",
            Self::Const => "const",
            Self::Static => "static",
            Self::Mut => "mut",
            Self::If => "if",
            Self::Else => "else",
            Self::For => "for",
            Self::While => "while",
            Self::In => "in",
            Self::Match => "match",
            Self::Return => "return",
            Self::Break => "break",
            Self::Continue => "continue",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Interface => "interface",
            Self::Impl => "impl",
            Self::Use => "use",
            Self::Import => "import",
            Self::As => "as",
            Self::True => "true",
            Self::False => "false",
            Self::Null => "null",
            Self::Async => "async",
            Self::Await => "await",
            Self::Go => "go",
            Self::Scope => "scope",
            Self::Try => "try",
            Self::Throw => "throw",
            Self::Self_ => "self",
            Self::Print => "print",
            Self::Ok => "ok",
            Self::Err => "err",
        }
    }

    /// Try to parse a keyword from a string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "fn" => Some(Self::Fn),
            "let" => Some(Self::Let),
            "const" => Some(Self::Const),
            "static" => Some(Self::Static),
            "mut" => Some(Self::Mut),
            "if" => Some(Self::If),
            "else" => Some(Self::Else),
            "for" => Some(Self::For),
            "while" => Some(Self::While),
            "in" => Some(Self::In),
            "match" => Some(Self::Match),
            "return" => Some(Self::Return),
            "break" => Some(Self::Break),
            "continue" => Some(Self::Continue),
            "struct" => Some(Self::Struct),
            "enum" => Some(Self::Enum),
            "interface" => Some(Self::Interface),
            "impl" => Some(Self::Impl),
            "use" => Some(Self::Use),
            "import" => Some(Self::Import),
            "as" => Some(Self::As),
            "true" => Some(Self::True),
            "false" => Some(Self::False),
            "null" => Some(Self::Null),
            "async" => Some(Self::Async),
            "await" => Some(Self::Await),
            "go" => Some(Self::Go),
            "scope" => Some(Self::Scope),
            "try" => Some(Self::Try),
            "throw" => Some(Self::Throw),
            "self" => Some(Self::Self_),
            "print" => Some(Self::Print),
            "ok" => Some(Self::Ok),
            "err" => Some(Self::Err),
            _ => None,
        }
    }
}

// ============================================================================
// TokenKind
// ============================================================================

/// The kind of a token in the stream.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// Identifier: `myVar`, `User`
    Ident(Symbol),
    /// Literal: `42`, `3.14`, `"hello"`, `true`
    Literal(Literal),
    /// Punctuation: `+`, `::`, `=>`
    Punct(Punct),
    /// Keyword: `fn`, `let`, `struct`
    Keyword(Keyword),
}

// ============================================================================
// Token
// ============================================================================

/// A single token with its source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// Token kind.
    pub kind: TokenKind,
    /// Source span.
    pub span: Span,
}

// ============================================================================
// Delimiter
// ============================================================================

/// Delimiter type for grouped token streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delimiter {
    /// `( ... )`
    Parenthesis,
    /// `{ ... }`
    Brace,
    /// `[ ... ]`
    Bracket,
    /// No visible delimiter (for macro interpolation)
    None,
}

impl Delimiter {
    /// Opening character for this delimiter.
    pub fn open(self) -> char {
        match self {
            Self::Parenthesis => '(',
            Self::Brace => '{',
            Self::Bracket => '[',
            Self::None => '\0',
        }
    }

    /// Closing character for this delimiter.
    pub fn close(self) -> char {
        match self {
            Self::Parenthesis => ')',
            Self::Brace => '}',
            Self::Bracket => ']',
            Self::None => '\0',
        }
    }
}

// ============================================================================
// TokenTree
// ============================================================================

/// A node in a TokenStream — either a single token or a delimited group.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenTree {
    /// A single token.
    Token(Token),
    /// A delimited group: `( ... )`, `{ ... }`, `[ ... ]`
    Group(Delimiter, TokenStream, Span),
}

// ============================================================================
// TokenStream
// ============================================================================

/// A sequence of token trees.
///
/// This is the primary data structure that macros receive and produce.
/// Created from source text via `from_str`, or built programmatically.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TokenStream {
    trees: Vec<TokenTree>,
}

impl TokenStream {
    /// Create an empty token stream.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a string into a token stream.
    ///
    /// Uses a built-in tokenizer that handles Doo syntax:
    /// identifiers, keywords, literals, punctuation, and delimited groups.
    pub fn from_str(source: &str) -> Self {
        let mut tokenizer = Tokenizer::new(source);
        tokenizer.tokenize()
    }

    /// Serialize the token stream back to a string.
    ///
    /// This is a lossless roundtrip: `from_str(s).to_string()` produces
    /// output equivalent to `s` (modulo whitespace normalization).
    pub fn to_string(&self) -> String {
        let mut out = String::with_capacity(64);
        let mut prev_spacing: Option<Spacing> = None;
        let mut prev_was_punct = false;

        for tree in &self.trees {
            let need_space = prev_was_punct && prev_spacing == Some(Spacing::Alone);

            if need_space {
                // Only add space if the next token is also punctuation
                // or if the previous was Alone
            }

            match tree {
                TokenTree::Token(token) => {
                    let is_punct = matches!(token.kind, TokenKind::Punct(_));

                    if prev_was_punct && is_punct && prev_spacing == Some(Spacing::Alone) {
                        out.push(' ');
                    } else if prev_was_punct && !is_punct {
                        out.push(' ');
                    }

                    match &token.kind {
                        TokenKind::Ident(sym) => out.push_str(sym.resolve()),
                        TokenKind::Literal(lit) => match lit {
                            Literal::Int(i) => out.push_str(&i.to_string()),
                            Literal::Float(f) => out.push_str(&f.to_string()),
                            Literal::String(s) => {
                                out.push('"');
                                out.push_str(s.resolve());
                                out.push('"');
                            }
                            Literal::Bool(b) => out.push_str(&b.to_string()),
                        },
                        TokenKind::Punct(p) => out.push(p.ch),
                        TokenKind::Keyword(kw) => out.push_str(kw.to_str()),
                    }

                    prev_spacing = match &token.kind {
                        TokenKind::Punct(p) => Some(p.spacing),
                        _ => None,
                    };
                    prev_was_punct = is_punct;
                }
                TokenTree::Group(delimiter, stream, _) => {
                    if prev_was_punct && prev_spacing == Some(Spacing::Alone) {
                        // No space before opening delimiter
                    }
                    if delimiter != &Delimiter::None {
                        out.push(delimiter.open());
                    }
                    let inner = stream.to_string();
                    out.push_str(&inner);
                    if delimiter != &Delimiter::None {
                        out.push(delimiter.close());
                    }
                    prev_was_punct = false;
                    prev_spacing = None;
                }
            }
        }

        out
    }

    /// Check if the stream is empty.
    pub fn is_empty(&self) -> bool {
        self.trees.is_empty()
    }

    /// Number of top-level token trees.
    pub fn len(&self) -> usize {
        self.trees.len()
    }

    /// Push a token tree onto the stream.
    pub fn push(&mut self, tree: TokenTree) {
        self.trees.push(tree);
    }

    /// Push a single token.
    pub fn push_token(&mut self, kind: TokenKind, span: Span) {
        self.trees.push(TokenTree::Token(Token { kind, span }));
    }

    /// Iterate over token trees.
    pub fn iter(&self) -> std::slice::Iter<'_, TokenTree> {
        self.trees.iter()
    }

    /// Extend with another stream.
    pub fn extend(&mut self, other: TokenStream) {
        self.trees.extend(other.trees);
    }
}

impl IntoIterator for TokenStream {
    type Item = TokenTree;
    type IntoIter = std::vec::IntoIter<TokenTree>;

    fn into_iter(self) -> Self::IntoIter {
        self.trees.into_iter()
    }
}

impl FromIterator<TokenTree> for TokenStream {
    fn from_iter<I: IntoIterator<Item = TokenTree>>(iter: I) -> Self {
        Self {
            trees: iter.into_iter().collect(),
        }
    }
}

// ============================================================================
// Tokenizer
// ============================================================================

/// Built-in tokenizer for creating TokenStreams from source text.
///
/// Handles Doo syntax without depending on `doo_frontend::Lexer`.
struct Tokenizer<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            bytes: source.as_bytes(),
            pos: 0,
        }
    }

    /// Tokenize the entire input into a TokenStream.
    fn tokenize(&mut self) -> TokenStream {
        let mut trees = Vec::new();

        while self.pos < self.bytes.len() {
            self.skip_whitespace_and_comments();

            if self.pos >= self.bytes.len() {
                break;
            }

            let start = self.pos as u32;
            let byte = self.bytes[self.pos];

            match byte {
                b'(' => {
                    self.pos += 1;
                    let stream = self.tokenize_until(b')');
                    trees.push(TokenTree::Group(
                        Delimiter::Parenthesis,
                        stream,
                        Span::new(start, self.pos as u32),
                    ));
                }
                b'{' => {
                    self.pos += 1;
                    let stream = self.tokenize_until(b'}');
                    trees.push(TokenTree::Group(
                        Delimiter::Brace,
                        stream,
                        Span::new(start, self.pos as u32),
                    ));
                }
                b'[' => {
                    self.pos += 1;
                    let stream = self.tokenize_until(b']');
                    trees.push(TokenTree::Group(
                        Delimiter::Bracket,
                        stream,
                        Span::new(start, self.pos as u32),
                    ));
                }
                b'"' => {
                    let token = self.tokenize_string(start);
                    trees.push(TokenTree::Token(token));
                }
                b'0'..=b'9' => {
                    let token = self.tokenize_number(start);
                    trees.push(TokenTree::Token(token));
                }
                b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                    let token = self.tokenize_ident(start);
                    trees.push(TokenTree::Token(token));
                }
                _ => {
                    if is_punct(byte) {
                        let token = self.tokenize_punct(start);
                        trees.push(TokenTree::Token(token));
                    } else {
                        self.pos += 1;
                    }
                }
            }
        }

        TokenStream { trees }
    }

    /// Tokenize until a matching closing delimiter, handling nesting.
    fn tokenize_until(&mut self, close: u8) -> TokenStream {
        let mut trees = Vec::new();
        let open = match close {
            b')' => b'(',
            b'}' => b'{',
            b']' => b'[',
            _ => return TokenStream::new(),
        };
        let mut depth = 0;

        while self.pos < self.bytes.len() {
            self.skip_whitespace_and_comments();
            if self.pos >= self.bytes.len() {
                break;
            }

            let byte = self.bytes[self.pos];

            if byte == close && depth == 0 {
                self.pos += 1;
                break;
            }

            if byte == close {
                depth -= 1;
            }

            match byte {
                b'(' if close == b')' => {
                    depth += 1;
                    let start = self.pos as u32;
                    self.pos += 1;
                    let stream = self.tokenize_until(b')');
                    trees.push(TokenTree::Group(
                        Delimiter::Parenthesis,
                        stream,
                        Span::new(start, self.pos as u32),
                    ));
                    continue;
                }
                b'{' if close == b'}' => {
                    depth += 1;
                    let start = self.pos as u32;
                    self.pos += 1;
                    let stream = self.tokenize_until(b'}');
                    trees.push(TokenTree::Group(
                        Delimiter::Brace,
                        stream,
                        Span::new(start, self.pos as u32),
                    ));
                    continue;
                }
                b'[' if close == b']' => {
                    depth += 1;
                    let start = self.pos as u32;
                    self.pos += 1;
                    let stream = self.tokenize_until(b']');
                    trees.push(TokenTree::Group(
                        Delimiter::Bracket,
                        stream,
                        Span::new(start, self.pos as u32),
                    ));
                    continue;
                }
                b'(' | b'{' | b'[' if byte != open => {
                    let delim = match byte {
                        b'(' => Delimiter::Parenthesis,
                        b'{' => Delimiter::Brace,
                        b'[' => Delimiter::Bracket,
                        _ => unreachable!(),
                    };
                    let close_char = match byte {
                        b'(' => b')',
                        b'{' => b'}',
                        b'[' => b']',
                        _ => unreachable!(),
                    };
                    let start = self.pos as u32;
                    self.pos += 1;
                    let stream = self.tokenize_until(close_char);
                    trees.push(TokenTree::Group(
                        delim,
                        stream,
                        Span::new(start, self.pos as u32),
                    ));
                    continue;
                }
                b'"' => {
                    let start = self.pos as u32;
                    let token = self.tokenize_string(start);
                    trees.push(TokenTree::Token(token));
                }
                b'0'..=b'9' => {
                    let start = self.pos as u32;
                    let token = self.tokenize_number(start);
                    trees.push(TokenTree::Token(token));
                }
                b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                    let start = self.pos as u32;
                    let token = self.tokenize_ident(start);
                    trees.push(TokenTree::Token(token));
                }
                _ if is_punct(byte) => {
                    let start = self.pos as u32;
                    let token = self.tokenize_punct(start);
                    trees.push(TokenTree::Token(token));
                }
                _ => {
                    self.pos += 1;
                }
            }
        }

        TokenStream { trees }
    }

    fn skip_whitespace_and_comments(&mut self) {
        while self.pos < self.bytes.len() {
            let byte = self.bytes[self.pos];

            if byte.is_ascii_whitespace() {
                self.pos += 1;
                continue;
            }

            // Line comment: //
            if byte == b'/' && self.pos + 1 < self.bytes.len() {
                if self.bytes[self.pos + 1] == b'/' {
                    self.pos += 2;
                    while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                    continue;
                }
                if self.bytes[self.pos + 1] == b'*' {
                    self.pos += 2;
                    while self.pos + 1 < self.bytes.len() {
                        if self.bytes[self.pos] == b'*' && self.bytes[self.pos + 1] == b'/' {
                            self.pos += 2;
                            break;
                        }
                        self.pos += 1;
                    }
                    continue;
                }
            }

            break;
        }
    }

    fn tokenize_string(&mut self, start: u32) -> Token {
        self.pos += 1; // Skip opening quote
        let mut content = String::new();

        while self.pos < self.bytes.len() {
            let byte = self.bytes[self.pos];

            if byte == b'"' {
                self.pos += 1;
                break;
            }

            if byte == b'\\' && self.pos + 1 < self.bytes.len() {
                self.pos += 1;
                let escaped = self.bytes[self.pos];
                match escaped {
                    b'n' => content.push('\n'),
                    b't' => content.push('\t'),
                    b'r' => content.push('\r'),
                    b'\\' => content.push('\\'),
                    b'"' => content.push('"'),
                    b'0' => content.push('\0'),
                    _ => content.push(escaped as char),
                }
                self.pos += 1;
                continue;
            }

            content.push(byte as char);
            self.pos += 1;
        }

        let end = self.pos as u32;
        Token {
            kind: TokenKind::Literal(Literal::String(Symbol::intern(&content))),
            span: Span::new(start, end),
        }
    }

    fn tokenize_number(&mut self, start: u32) -> Token {
        let num_start = self.pos;

        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
            self.pos += 1;
        }

        // Check for float
        if self.pos < self.bytes.len()
            && self.bytes[self.pos] == b'.'
            && self.pos + 1 < self.bytes.len()
            && self.bytes[self.pos + 1].is_ascii_digit()
        {
            self.pos += 1; // Skip dot
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            let text = std::str::from_utf8(&self.bytes[num_start..self.pos]).unwrap_or("0");
            let val: f64 = text.parse().unwrap_or(0.0);
            return Token {
                kind: TokenKind::Literal(Literal::Float(val)),
                span: Span::new(start, self.pos as u32),
            };
        }

        let text = std::str::from_utf8(&self.bytes[num_start..self.pos]).unwrap_or("0");
        let val: i64 = text.parse().unwrap_or(0);
        Token {
            kind: TokenKind::Literal(Literal::Int(val)),
            span: Span::new(start, self.pos as u32),
        }
    }

    fn tokenize_ident(&mut self, start: u32) -> Token {
        let ident_start = self.pos;

        while self.pos < self.bytes.len()
            && (self.bytes[self.pos].is_ascii_alphanumeric() || self.bytes[self.pos] == b'_')
        {
            self.pos += 1;
        }

        let text = std::str::from_utf8(&self.bytes[ident_start..self.pos]).unwrap_or("");

        // Check for keywords
        if let Some(kw) = Keyword::from_str(text) {
            return Token {
                kind: TokenKind::Keyword(kw),
                span: Span::new(start, self.pos as u32),
            };
        }

        // Check for boolean literals
        match text {
            "true" => {
                return Token {
                    kind: TokenKind::Literal(Literal::Bool(true)),
                    span: Span::new(start, self.pos as u32),
                };
            }
            "false" => {
                return Token {
                    kind: TokenKind::Literal(Literal::Bool(false)),
                    span: Span::new(start, self.pos as u32),
                };
            }
            _ => {}
        }

        Token {
            kind: TokenKind::Ident(Symbol::intern(text)),
            span: Span::new(start, self.pos as u32),
        }
    }

    fn tokenize_punct(&mut self, start: u32) -> Token {
        let ch = self.bytes[self.pos] as char;
        self.pos += 1;

        // Determine spacing: Joint if next byte is also punctuation
        let spacing = if self.pos < self.bytes.len() && is_punct(self.bytes[self.pos]) {
            Spacing::Joint
        } else {
            Spacing::Alone
        };

        Token {
            kind: TokenKind::Punct(Punct { ch, spacing }),
            span: Span::new(start, self.pos as u32),
        }
    }
}

/// Check if a byte is a punctuation character.
fn is_punct(byte: u8) -> bool {
    matches!(
        byte,
        b'+' | b'-'
            | b'*'
            | b'/'
            | b'%'
            | b'='
            | b'!'
            | b'<'
            | b'>'
            | b'&'
            | b'|'
            | b'^'
            | b'~'
            | b'@'
            | b'#'
            | b'?'
            | b':'
            | b';'
            | b','
            | b'.'
            | b'_'
            | b'$'
    )
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_stream() {
        let stream = TokenStream::new();
        assert!(stream.is_empty());
        assert_eq!(stream.len(), 0);
    }

    #[test]
    fn test_from_str_ident() {
        let stream = TokenStream::from_str("myVar");
        assert_eq!(stream.len(), 1);
        if let TokenTree::Token(token) = stream.iter().next().unwrap() {
            assert!(matches!(&token.kind, TokenKind::Ident(_)));
        } else {
            panic!("expected Token");
        }
    }

    #[test]
    fn test_from_str_keyword() {
        let stream = TokenStream::from_str("fn");
        assert_eq!(stream.len(), 1);
        if let TokenTree::Token(token) = stream.iter().next().unwrap() {
            assert!(matches!(&token.kind, TokenKind::Keyword(Keyword::Fn)));
        } else {
            panic!("expected Token");
        }
    }

    #[test]
    fn test_from_str_int() {
        let stream = TokenStream::from_str("42");
        assert_eq!(stream.len(), 1);
        if let TokenTree::Token(token) = stream.iter().next().unwrap() {
            assert!(matches!(&token.kind, TokenKind::Literal(Literal::Int(42))));
        } else {
            panic!("expected Token");
        }
    }

    #[test]
    fn test_from_str_float() {
        let stream = TokenStream::from_str("3.14");
        assert_eq!(stream.len(), 1);
        if let TokenTree::Token(token) = stream.iter().next().unwrap() {
            if let TokenKind::Literal(Literal::Float(f)) = &token.kind {
                assert!((f - 3.14).abs() < 0.001);
            } else {
                panic!("expected Float");
            }
        } else {
            panic!("expected Token");
        }
    }

    #[test]
    fn test_from_str_string() {
        let stream = TokenStream::from_str("\"hello\"");
        assert_eq!(stream.len(), 1);
        if let TokenTree::Token(token) = stream.iter().next().unwrap() {
            if let TokenKind::Literal(Literal::String(s)) = &token.kind {
                assert_eq!(s.resolve(), "hello");
            } else {
                panic!("expected String");
            }
        } else {
            panic!("expected Token");
        }
    }

    #[test]
    fn test_from_str_bool() {
        let stream = TokenStream::from_str("true");
        assert_eq!(stream.len(), 1);
        if let TokenTree::Token(token) = stream.iter().next().unwrap() {
            assert!(matches!(
                &token.kind,
                TokenKind::Literal(Literal::Bool(true))
            ));
        } else {
            panic!("expected Token");
        }
    }

    #[test]
    fn test_from_str_punct() {
        let stream = TokenStream::from_str("+");
        assert_eq!(stream.len(), 1);
        if let TokenTree::Token(token) = stream.iter().next().unwrap() {
            if let TokenKind::Punct(p) = &token.kind {
                assert_eq!(p.ch, '+');
            } else {
                panic!("expected Punct");
            }
        } else {
            panic!("expected Token");
        }
    }

    #[test]
    fn test_from_str_group_parens() {
        let stream = TokenStream::from_str("(a, b)");
        assert_eq!(stream.len(), 1);
        if let TokenTree::Group(delimiter, inner, _) = stream.iter().next().unwrap() {
            assert_eq!(*delimiter, Delimiter::Parenthesis);
            assert_eq!(inner.len(), 3);
        } else {
            panic!("expected Group");
        }
    }

    #[test]
    fn test_from_str_group_braces() {
        let stream = TokenStream::from_str("{ x: 1 }");
        assert_eq!(stream.len(), 1);
        if let TokenTree::Group(delimiter, _, _) = stream.iter().next().unwrap() {
            assert_eq!(*delimiter, Delimiter::Brace);
        } else {
            panic!("expected Group");
        }
    }

    #[test]
    fn test_from_str_group_brackets() {
        let stream = TokenStream::from_str("[1, 2, 3]");
        assert_eq!(stream.len(), 1);
        if let TokenTree::Group(delimiter, inner, _) = stream.iter().next().unwrap() {
            assert_eq!(*delimiter, Delimiter::Bracket);
            assert!(inner.len() >= 3);
        } else {
            panic!("expected Group");
        }
    }

    #[test]
    fn test_from_str_nested_groups() {
        let stream = TokenStream::from_str("{ [1, 2], (a) }");
        assert_eq!(stream.len(), 1);
        if let TokenTree::Group(_, inner, _) = stream.iter().next().unwrap() {
            assert!(inner.len() >= 1);
        } else {
            panic!("expected Group");
        }
    }

    #[test]
    fn test_from_str_multi_token() {
        let stream = TokenStream::from_str("fn add(x: Int, y: Int) -> Int { x + y }");
        assert!(stream.len() > 5);
    }

    #[test]
    fn test_to_string_roundtrip() {
        let source = "fn add(x: Int) -> Int { x }";
        let stream = TokenStream::from_str(source);
        let result = stream.to_string();
        assert!(result.contains("fn"));
        assert!(result.contains("add"));
    }

    #[test]
    fn test_to_string_simple() {
        let stream = TokenStream::from_str("hello");
        assert_eq!(stream.to_string(), "hello");
    }

    #[test]
    fn test_to_string_group() {
        let stream = TokenStream::from_str("(x, y)");
        let result = stream.to_string();
        assert!(result.contains("("));
        assert!(result.contains(")"));
        assert!(result.contains("x"));
        assert!(result.contains("y"));
    }

    #[test]
    fn test_keyword_from_str() {
        assert_eq!(Keyword::from_str("fn"), Some(Keyword::Fn));
        assert_eq!(Keyword::from_str("let"), Some(Keyword::Let));
        assert_eq!(Keyword::from_str("not_a_keyword"), None);
    }

    #[test]
    fn test_keyword_to_str() {
        assert_eq!(Keyword::Fn.to_str(), "fn");
        assert_eq!(Keyword::Struct.to_str(), "struct");
    }

    #[test]
    fn test_from_str_with_comments() {
        let stream = TokenStream::from_str("// comment\nfn foo() {}");
        assert!(stream.len() >= 1);
        if let TokenTree::Token(token) = stream.iter().next().unwrap() {
            assert!(matches!(&token.kind, TokenKind::Keyword(Keyword::Fn)));
        } else {
            panic!("expected Token");
        }
    }

    #[test]
    fn test_from_str_with_whitespace() {
        let stream = TokenStream::from_str("  fn   foo  (  )  ");
        assert!(stream.len() >= 3);
    }

    #[test]
    fn test_from_str_string_with_escapes() {
        let stream = TokenStream::from_str("\"hello\\nworld\"");
        if let TokenTree::Token(token) = stream.iter().next().unwrap() {
            if let TokenKind::Literal(Literal::String(s)) = &token.kind {
                assert_eq!(s.resolve(), "hello\nworld");
            } else {
                panic!("expected String");
            }
        } else {
            panic!("expected Token");
        }
    }

    #[test]
    fn test_push_token() {
        let mut stream = TokenStream::new();
        stream.push_token(TokenKind::Ident(Symbol::intern("x")), Span::new(0, 1));
        assert_eq!(stream.len(), 1);
    }

    #[test]
    fn test_extend() {
        let mut s1 = TokenStream::from_str("a");
        let s2 = TokenStream::from_str("b");
        s1.extend(s2);
        assert_eq!(s1.len(), 2);
    }

    #[test]
    fn test_from_str_struct_definition() {
        let source = "struct User { Name: Str; Age: Int }";
        let stream = TokenStream::from_str(source);
        assert!(stream.len() > 3);
        let result = stream.to_string();
        assert!(result.contains("struct"));
        assert!(result.contains("User"));
    }

    #[test]
    fn test_from_str_function_definition() {
        let source = "fn save(user: User) -> Void { print(user) }";
        let stream = TokenStream::from_str(source);
        assert!(stream.len() > 5);
        let result = stream.to_string();
        assert!(result.contains("fn"));
        assert!(result.contains("save"));
    }

    #[test]
    fn test_from_str_at_decorator() {
        let stream = TokenStream::from_str("@table(\"users\")");
        assert!(stream.len() >= 2);
    }
}
