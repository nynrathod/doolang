//! Token types for the Doo lexer.
//!
//! All tokens produced by the lexer, including keywords, operators, and literals.

use doo_core::Span;
use serde::{Deserialize, Serialize};

/// A token produced by the lexer.
#[derive(Debug, Clone)]
pub struct Token {
    /// The kind of token.
    pub kind: TokenKind,
    /// The source text of the token.
    pub text: String,
    /// Location in source code.
    pub span: Span,
}

impl Token {
    /// Create a new token.
    pub fn new(kind: TokenKind, text: impl Into<String>, span: Span) -> Self {
        Self {
            kind,
            text: text.into(),
            span,
        }
    }

    /// Check if this is an EOF token.
    pub fn is_eof(&self) -> bool {
        self.kind == TokenKind::Eof
    }

    /// Check if this is an error token.
    pub fn is_error(&self) -> bool {
        self.kind == TokenKind::Error
    }

    /// Get the line number (1-indexed).
    pub fn line(&self) -> u32 {
        // Will be calculated from span using SourceMap
        0
    }
}

/// All token types in Doo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TokenKind {
    // === Special ===
    /// End of file
    Eof,
    /// Lexer error
    Error,

    // === Keywords ===
    /// `let`
    Let,
    /// `mut`
    Mut,
    /// `fn`
    Fn,
    /// `import`
    Import,
    /// `as`
    As,
    /// `struct`
    Struct,
    /// `enum`
    Enum,
    /// `if`
    If,
    /// `else`
    Else,
    /// `for`
    For,
    /// `in`
    In,
    /// `return`
    Return,
    /// `break`
    Break,
    /// `continue`
    Continue,
    /// `print`
    Print,
    /// `Ok`
    Ok,
    /// `Err`
    Err,
    /// `nil`
    Nil,
    /// `match`
    Match,
    /// `true`
    True,
    /// `false`
    False,

    // === Literals ===
    /// Integer literal: `123`, `0`, `-42`
    Integer,
    /// Float literal: `3.14`, `1e10`, `2.5e-3`
    Float,
    /// String literal: `"hello"`
    String,
    /// String template with interpolation: `"Hello ${name}"`
    StringTemplate,
    /// Identifier: `foo`, `myVar`, `User`
    Ident,

    // === Operators: Arithmetic ===
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `%`
    Percent,

    // === Operators: Comparison ===
    /// `==`
    EqEq,
    /// `!=`
    NotEq,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    LtEq,
    /// `>=`
    GtEq,

    // === Operators: Logical ===
    /// `!`
    Bang,
    /// `&&`
    AndAnd,
    /// `||`
    OrOr,
    /// `&`
    And,
    /// `|`
    Or,

    // === Operators: Assignment ===
    /// `=`
    Eq,
    /// `+=`
    PlusEq,
    /// `-=`
    MinusEq,
    /// `*=`
    StarEq,
    /// `/=`
    SlashEq,
    /// `%=`
    PercentEq,

    // === Operators: Increment/Decrement ===
    /// `++`
    PlusPlus,
    /// `--`
    MinusMinus,

    // === Operators: Arrows ===
    /// `->`
    Arrow,
    /// `=>`
    FatArrow,

    // === Delimiters ===
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `[`
    LBracket,
    /// `]`
    RBracket,

    // === Punctuation ===
    /// `,`
    Comma,
    /// `;`
    Semi,
    /// `.`
    Dot,
    /// `..`
    DotDot,
    /// `..=`
    DotDotEq,
    /// `...`
    Spread,
    /// `:`
    Colon,
    /// `::`
    ColonColon,
    /// `?`
    Question,
    /// `??`
    QuestionQuestion,
    /// `@`
    At,
    /// `#`
    Hash,
    /// `~`
    Tilde,
    /// `$`
    Dollar,
    /// `_`
    Underscore,
}

impl TokenKind {
    /// Get the keyword for this token kind, if it's a keyword.
    pub fn keyword_str(&self) -> Option<&'static str> {
        match self {
            Self::Let => Some("let"),
            Self::Mut => Some("mut"),
            Self::Fn => Some("fn"),
            Self::Import => Some("import"),
            Self::As => Some("as"),
            Self::Struct => Some("struct"),
            Self::Enum => Some("enum"),
            Self::If => Some("if"),
            Self::Else => Some("else"),
            Self::For => Some("for"),
            Self::In => Some("in"),
            Self::Return => Some("return"),
            Self::Break => Some("break"),
            Self::Continue => Some("continue"),
            Self::Print => Some("print"),
            Self::Ok => Some("Ok"),
            Self::Err => Some("Err"),
            Self::Nil => Some("nil"),
            Self::Match => Some("match"),
            Self::True => Some("true"),
            Self::False => Some("false"),
            _ => None,
        }
    }

    /// Check if this is a keyword.
    pub fn is_keyword(&self) -> bool {
        self.keyword_str().is_some()
    }

    /// Check if this is a literal (number, string, bool).
    pub fn is_literal(&self) -> bool {
        matches!(
            self,
            Self::Integer | Self::Float | Self::String | Self::StringTemplate | Self::True | Self::False | Self::Nil
        )
    }

    /// Check if this is an operator.
    pub fn is_operator(&self) -> bool {
        matches!(
            self,
            Self::Plus
                | Self::Minus
                | Self::Star
                | Self::Slash
                | Self::Percent
                | Self::EqEq
                | Self::NotEq
                | Self::Lt
                | Self::Gt
                | Self::LtEq
                | Self::GtEq
                | Self::Bang
                | Self::AndAnd
                | Self::OrOr
                | Self::And
                | Self::Or
                | Self::Eq
                | Self::PlusEq
                | Self::MinusEq
                | Self::StarEq
                | Self::SlashEq
                | Self::PercentEq
                | Self::PlusPlus
                | Self::MinusMinus
                | Self::Arrow
                | Self::FatArrow
        )
    }

    /// Check if this is a delimiter.
    pub fn is_delimiter(&self) -> bool {
        matches!(
            self,
            Self::LParen | Self::RParen | Self::LBrace | Self::RBrace | Self::LBracket | Self::RBracket
        )
    }

    /// Get a human-readable description.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Eof => "end of file",
            Self::Error => "error",
            Self::Let => "`let`",
            Self::Mut => "`mut`",
            Self::Fn => "`fn`",
            Self::Import => "`import`",
            Self::As => "`as`",
            Self::Struct => "`struct`",
            Self::Enum => "`enum`",
            Self::If => "`if`",
            Self::Else => "`else`",
            Self::For => "`for`",
            Self::In => "`in`",
            Self::Return => "`return`",
            Self::Break => "`break`",
            Self::Continue => "`continue`",
            Self::Print => "`print`",
            Self::Ok => "`Ok`",
            Self::Err => "`Err`",
            Self::Nil => "`nil`",
            Self::Match => "`match`",
            Self::True => "`true`",
            Self::False => "`false`",
            Self::Integer => "integer",
            Self::Float => "float",
            Self::String => "string",
            Self::StringTemplate => "string template",
            Self::Ident => "identifier",
            Self::Plus => "`+`",
            Self::Minus => "`-`",
            Self::Star => "`*`",
            Self::Slash => "`/`",
            Self::Percent => "`%`",
            Self::EqEq => "`==`",
            Self::NotEq => "`!=`",
            Self::Lt => "`<`",
            Self::Gt => "`>`",
            Self::LtEq => "`<=`",
            Self::GtEq => "`>=`",
            Self::Bang => "`!`",
            Self::AndAnd => "`&&`",
            Self::OrOr => "`||`",
            Self::And => "`&`",
            Self::Or => "`|`",
            Self::Eq => "`=`",
            Self::PlusEq => "`+=`",
            Self::MinusEq => "`-=`",
            Self::StarEq => "`*=`",
            Self::SlashEq => "`/=`",
            Self::PercentEq => "`%=`",
            Self::PlusPlus => "`++`",
            Self::MinusMinus => "`--`",
            Self::Arrow => "`->`",
            Self::FatArrow => "`=>`",
            Self::LParen => "`(`",
            Self::RParen => "`)`",
            Self::LBrace => "`{`",
            Self::RBrace => "`}`",
            Self::LBracket => "`[`",
            Self::RBracket => "`]`",
            Self::Comma => "`,`",
            Self::Semi => "`;`",
            Self::Dot => "`.`",
            Self::DotDot => "`..`",
            Self::DotDotEq => "`..=`",
            Self::Spread => "`...`",
            Self::Colon => "`:`",
            Self::ColonColon => "`::`",
            Self::Question => "`?`",
            Self::QuestionQuestion => "`??`",
            Self::At => "`@`",
            Self::Hash => "`#`",
            Self::Tilde => "`~`",
            Self::Dollar => "`$`",
            Self::Underscore => "`_`",
        }
    }
}

impl std::fmt::Display for TokenKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.description())
    }
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}({})", self.kind, self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_kind_is_keyword() {
        assert!(TokenKind::Let.is_keyword());
        assert!(TokenKind::Fn.is_keyword());
        assert!(!TokenKind::Plus.is_keyword());
        assert!(!TokenKind::Ident.is_keyword());
    }

    #[test]
    fn test_token_kind_is_literal() {
        assert!(TokenKind::Integer.is_literal());
        assert!(TokenKind::Float.is_literal());
        assert!(TokenKind::String.is_literal());
        assert!(TokenKind::True.is_literal());
        assert!(!TokenKind::Ident.is_literal());
    }

    #[test]
    fn test_token_kind_description() {
        assert_eq!(TokenKind::Let.description(), "`let`");
        assert_eq!(TokenKind::Plus.description(), "`+`");
        assert_eq!(TokenKind::Integer.description(), "integer");
    }
}
