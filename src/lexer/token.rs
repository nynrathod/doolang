#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TokenType {
    Unknown, // For invalid or unrecognized characters
    Eof,
    // --- Keywords ---
    Let,       // let
    Mut,       // mutable keyword for let
    Function,  // function
    Import,    // import
    As,        // as (for aliasing imports)
    Struct,    // struct
    Enum,      // enum
    If,        // if
    Else,      // else
    For,       // for
    In,        // in
    Return,    // return
    Break,     // break
    Continue,  // continue
    Print,     // print
    Ok,        // Ok (for Result type)
    Err,       // Err (for Result type)
    Nil,       // nil (null value)

    // --- Literals ---
    Number,
    Float,
    String,
    Boolean,

    // --- Identifier ---
    Identifier,

    // --- Operators ---
    // Arithmetic
    Plus,    // +
    Minus,   // -
    Star,    // *
    Slash,   // /
    Percent, // %

    // Assignment
    Eq,        // =
    PlusEq,    // +=
    MinusEq,   // -=
    StarEq,    // *=
    SlashEq,   // /=
    PercentEq, // %=

    // Increment/Decrement
    PlusPlus,   // ++
    MinusMinus, // --

    // Comparison
    EqEq,  // ==
    NotEq, // !=
    Gt,    // >
    Lt,    // <
    GtEq,  // >=
    LtEq,  // <=

    // Logical
    Bang,   // !
    And,    // &
    Or,     // |
    AndAnd, // &&
    OrOr,   // ||

    // Arrow
    Arrow,    // ->
    FatArrow, // =>

    // --- Delimiters & Punctuation ---
    OpenParen,      // (
    CloseParen,     // )
    OpenBrace,      // {
    CloseBrace,     // }
    OpenBracket,    // [
    CloseBracket,   // ]
    Comma,          // ,
    Semi,           // ;
    Dot,            // .
    RangeInc,       // ..=
    RangeExc,       // ..
    Colon,          // :
    ColonColon,     // ::
    Pound,          // #
    At,             // @
    Tilde,          // ~
    Question,       // ?
    DoubleQuestion, // ??
    Dollar,         // $
    Underscore,     // _
}

#[derive(Debug, Clone)]
pub struct Token<'a> {
    pub kind: TokenType,
    pub value: &'a str,
    pub line: usize,
    pub col: usize,
}
