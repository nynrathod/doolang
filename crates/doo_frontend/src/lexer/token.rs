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
    /// `interface`
    Interface,
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

    /// `const`
    Const,
    /// `static`
    Static,
    /// `impl`
    Impl,

    /// `use`
    Use,
    /// `throw`
    Throw,
    /// `Self` (type reference, not the `self` parameter)
    Self_,

    // === RBAC ===
    /// `policy`
    Policy,

    // === Async & Concurrency ===
    /// `async`
    Async,
    /// `await`
    Await,
    /// `go`
    Go,
    /// `scope`
    Scope,

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

    // === Operators: Bitwise (ADD THESE) ===
    /// `^` (bitwise XOR)
    Caret,
}

impl TokenKind {
    /// Get the keyword for this token kind, if it's a keyword.
    pub fn keyword_str(&self) -> Option<&'static str> {
        match self {
            Self::Const => Some("const"),
            Self::Static => Some("static"),
            Self::Impl => Some("impl"),
            Self::Let => Some("let"),
            Self::Mut => Some("mut"),
            Self::Fn => Some("fn"),
            Self::Use => Some("use"),
            Self::Import => Some("import"),
            Self::As => Some("as"),
            Self::Struct => Some("struct"),
            Self::Enum => Some("enum"),
            Self::Interface => Some("interface"),
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
            Self::Policy => Some("policy"),
            Self::Async => Some("async"),
            Self::Await => Some("await"),
            Self::Go => Some("go"),
            Self::Scope => Some("scope"),
            Self::Throw => Some("throw"),
            Self::Self_ => Some("Self"),
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
            Self::Integer
                | Self::Float
                | Self::String
                | Self::StringTemplate
                | Self::True
                | Self::False
                | Self::Nil
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
                | Self::Caret
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
            Self::LParen
                | Self::RParen
                | Self::LBrace
                | Self::RBrace
                | Self::LBracket
                | Self::RBracket
        )
    }

    /// Get a human-readable description.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Eof => "end of file",
            Self::Error => "error",
            Self::Const => "`const`",
            Self::Static => "`static`",
            Self::Impl => "`impl`",
            Self::Let => "`let`",
            Self::Mut => "`mut`",
            Self::Fn => "`fn`",
            Self::Import => "`import`",
            Self::As => "`as`",
            Self::Struct => "`struct`",
            Self::Enum => "`enum`",
            Self::Interface => "`interface`",
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
            Self::Policy => "`policy`",
            Self::Async => "`async`",
            Self::Await => "`await`",
            Self::Go => "`go`",
            Self::Scope => "`scope`",
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
            Self::Use => "`use`",
            Self::Throw => "`throw`",
            Self::Self_ => "`Self`",
            Self::Caret => "`^`",
        }
    }

    /// Get operator precedence for Pratt parsing (higher = binds tighter).
    ///
    /// Returns 0 for non-operator tokens (literals, keywords, delimiters).
    ///
    /// Precedence table (lowest to highest):
    /// ```text
    ///  1: || (logical or)
    ///  2: && (logical and)
    ///  3: == != (equality)
    ///  4: < > <= >= (comparison)
    ///  5: ?? (null coalesce)
    ///  6: | (bitwise or)
    ///  7: ^ (bitwise xor)
    ///  8: & (bitwise and)
    ///  9: << >> (shift)
    /// 10: .. ..= (range)
    /// 11: + - (additive)
    /// 12: * / % (multiplicative)
    /// ```
    pub fn precedence(&self) -> u8 {
        match self {
            // Assignment operators — lowest precedence (handled separately by parser)
            // Return 0 here; the parser handles assignment as a special case
            // because it's right-associative and produces statements, not expressions.
            Self::Eq
            | Self::PlusEq
            | Self::MinusEq
            | Self::StarEq
            | Self::SlashEq
            | Self::PercentEq => 0,

            // Logical OR
            Self::OrOr => 1,

            // Logical AND
            Self::AndAnd => 2,

            // Equality
            Self::EqEq | Self::NotEq => 3,

            // Comparison
            Self::Lt | Self::Gt | Self::LtEq | Self::GtEq => 4,

            // Null coalescing
            Self::QuestionQuestion => 5,

            // Bitwise OR
            Self::Or => 6,

            // Bitwise XOR
            Self::Caret => 7,

            // Bitwise AND
            Self::And => 8,

            // Range
            Self::DotDot | Self::DotDotEq => 10,

            // Additive
            Self::Plus | Self::Minus => 11,

            // Multiplicative
            Self::Star | Self::Slash | Self::Percent => 12,

            // Everything else: not a binary operator
            _ => 0,
        }
    }

    /// Check if this operator is right-associative.
    ///
    /// Right-associative operators bind from right to left:
    /// `a = b = c` parses as `a = (b = c)`
    /// `a ?? b ?? c` parses as `a ?? (b ?? c)`
    ///
    /// Used by the Pratt parser to decide whether to use `min_prec`
    /// or `min_prec - 1` for the right-hand side recursion.
    pub fn is_right_associative(&self) -> bool {
        matches!(
            self,
            Self::Eq
                | Self::PlusEq
                | Self::MinusEq
                | Self::StarEq
                | Self::SlashEq
                | Self::PercentEq
                | Self::QuestionQuestion
        )
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
