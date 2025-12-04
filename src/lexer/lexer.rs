use crate::lexer::token::{Token, TokenType};
use crate::limits::{
    LEXER_MAX_COMMENT_LENGTH, LEXER_MAX_IDENTIFIER_LENGTH, LEXER_MAX_INPUT_SIZE,
    LEXER_MAX_STRING_LENGTH, LEXER_MAX_TOKEN_COUNT,
};
use bumpalo::Bump;
use std::collections::HashMap;

pub fn lex<'a>(input: &'a str, arena: &'a Bump) -> Vec<Token<'a>> {
    // Validate input size to prevent DoS
    if input.len() > LEXER_MAX_INPUT_SIZE {
        return vec![Token {
            kind: TokenType::Unknown,
            value: arena.alloc_str("Input too large"),
            line: 1,
            col: 1,
        }];
    }

    let chars: Vec<char> = input.chars().collect();
    let mut tokens: Vec<Token> = Vec::new();

    // --- Keyword Maps ---
    let mut keywords: HashMap<&str, TokenType> = HashMap::new();

    // Declarations
    keywords.insert("let", TokenType::Let);
    keywords.insert("mut", TokenType::Mut);
    keywords.insert("fn", TokenType::Function);
    keywords.insert("import", TokenType::Import);
    keywords.insert("as", TokenType::As);
    keywords.insert("struct", TokenType::Struct);
    keywords.insert("enum", TokenType::Enum);
    keywords.insert("match", TokenType::Match);

    // Control flow statements
    keywords.insert("if", TokenType::If);
    keywords.insert("else", TokenType::Else);
    keywords.insert("for", TokenType::For);
    keywords.insert("in", TokenType::In);

    // Statement keywords
    keywords.insert("return", TokenType::Return);
    keywords.insert("break", TokenType::Break);
    keywords.insert("continue", TokenType::Continue);
    keywords.insert("print", TokenType::Print);

    // Error handling keywords
    keywords.insert("Ok", TokenType::Ok);
    keywords.insert("Err", TokenType::Err);
    keywords.insert("nil", TokenType::Nil);

    // Special values and types
    keywords.insert("true", TokenType::Boolean);
    keywords.insert("false", TokenType::Boolean);

    // --- Operator and Punctuation Map ---
    let mut operators: HashMap<&str, TokenType> = HashMap::new();

    // Assignment and arithmetic operators
    operators.insert("=", TokenType::Eq);
    operators.insert("+", TokenType::Plus);
    operators.insert("-", TokenType::Minus);
    operators.insert("*", TokenType::Star);
    operators.insert("/", TokenType::Slash);
    operators.insert("%", TokenType::Percent);

    // Logical and comparison operators
    operators.insert("!", TokenType::Bang);
    operators.insert("<", TokenType::Lt);
    operators.insert(">", TokenType::Gt);
    operators.insert("&", TokenType::And);
    operators.insert("|", TokenType::Or);

    operators.insert("==", TokenType::EqEq);
    operators.insert("!=", TokenType::NotEq);
    operators.insert(">=", TokenType::GtEq);
    operators.insert("<=", TokenType::LtEq);
    operators.insert("&&", TokenType::AndAnd);
    operators.insert("||", TokenType::OrOr);

    // Increment/Decrement operators
    operators.insert("++", TokenType::PlusPlus);
    operators.insert("--", TokenType::MinusMinus);

    // Compound assignment operators
    operators.insert("+=", TokenType::PlusEq);
    operators.insert("-=", TokenType::MinusEq);
    operators.insert("*=", TokenType::StarEq);
    operators.insert("/=", TokenType::SlashEq);
    operators.insert("%=", TokenType::PercentEq);

    // Arrow operators
    operators.insert("->", TokenType::Arrow);
    operators.insert("=>", TokenType::FatArrow);

    // Grouping and delimiter symbols
    operators.insert("(", TokenType::OpenParen);
    operators.insert(")", TokenType::CloseParen);
    operators.insert("{", TokenType::OpenBrace);
    operators.insert("}", TokenType::CloseBrace);
    operators.insert("[", TokenType::OpenBracket);
    operators.insert("]", TokenType::CloseBracket);

    // Punctuation
    operators.insert(",", TokenType::Comma);
    operators.insert(";", TokenType::Semi);
    operators.insert(".", TokenType::Dot);
    operators.insert("..=", TokenType::RangeInc);
    operators.insert("..", TokenType::RangeExc);

    // Miscellaneous symbols
    operators.insert("::", TokenType::ColonColon);
    operators.insert(":", TokenType::Colon);
    operators.insert("#", TokenType::Pound);
    operators.insert("@", TokenType::At);
    operators.insert("~", TokenType::Tilde);
    operators.insert("?", TokenType::Question);
    operators.insert("??", TokenType::DoubleQuestion);
    operators.insert("$", TokenType::Dollar);

    // Special identifier
    operators.insert("_", TokenType::Underscore);

    let mut i = 0;
    let mut line: usize = 1;
    let mut col: usize = 1;
    while i < chars.len() {
        // Check token count before processing each token to prevent memory exhaustion
        if tokens.len() >= LEXER_MAX_TOKEN_COUNT {
            tokens.push(Token {
                kind: TokenType::Unknown,
                value: arena.alloc_str("Too many tokens in input"),
                line,
                col,
            });
            break;
        }

        let c = chars[i];

        // Skip whitespace
        if c.is_whitespace() {
            if c == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
            i += 1;
            continue;
        }

        // Skip comments starting with // until newline
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            i += 2; // skip the `//`
            col += 2;
            let comment_start = i;
            while i < chars.len() && chars[i] != '\n' {
                // Prevent excessively long single-line comments
                if i - comment_start > LEXER_MAX_COMMENT_LENGTH {
                    // Skip to end of line or end of input
                    while i < chars.len() && chars[i] != '\n' {
                        i += 1;
                    }
                    break;
                }
                i += 1;
                col += 1;
            }
            continue;
        }

        // Skip C-style multiline comments /* ... */
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            i += 2;
            col += 2;
            let mut comment_len = 0;
            let mut found_end = false;
            // Find closing */ with bounds checking
            while i + 1 < chars.len() {
                comment_len += 1;
                // Prevent excessively long multiline comments
                if comment_len > LEXER_MAX_COMMENT_LENGTH {
                    // Skip to safe position to resume lexing - find end of line or end of comment
                    while i + 1 < chars.len() {
                        if chars[i] == '*' && chars[i + 1] == '/' {
                            i += 2;
                            col += 2;
                            found_end = true;
                            break;
                        }
                        if chars[i] == '\n' {
                            line += 1;
                            col = 1;
                            i += 1;
                            break;
                        }
                        i += 1;
                        col += 1;
                    }
                    break;
                }

                if chars[i] == '*' && chars[i + 1] == '/' {
                    i += 2;
                    col += 2;
                    found_end = true;
                    break;
                }

                if chars[i] == '\n' {
                    line += 1;
                    col = 1;
                } else {
                    col += 1;
                }
                i += 1;
            }

            // If we didn't find the end and we're at EOF, skip to end
            if !found_end && i < chars.len() {
                i = chars.len();
            }
            continue;
        }

        // Multi-character operators first
        // Always check for ..= and .. before handling numbers/floats
        if i + 3 <= chars.len() {
            let op: String = chars[i..i + 3].iter().collect();
            if op == "..=" {
                let op_str = arena.alloc_str("..=");
                tokens.push(Token {
                    kind: TokenType::RangeInc, // inclusive
                    value: op_str,
                    line,
                    col,
                });
                i += 3;
                col += 3;
                continue;
            }
            // Check for ... (spread operator)
            let op: String = chars[i..i + 3].iter().collect();
            if op == "..." {
                let op_str = arena.alloc_str("...");
                tokens.push(Token {
                    kind: TokenType::Spread,
                    value: op_str,
                    line,
                    col,
                });
                i += 3;
                col += 3;
                continue;
            }
        }
        if i + 2 <= chars.len() {
            let op: String = chars[i..i + 2].iter().collect();
            if op == ".." {
                let op_str = arena.alloc_str("..");
                tokens.push(Token {
                    kind: TokenType::RangeExc, // exclusive
                    value: op_str,
                    line,
                    col,
                });
                i += 2;
                col += 2;
                continue;
            }
        }

        // For value inside string literal
        // Ex: "hello world"
        if c == '"' {
            let token_line = line;
            let token_col = col;
            let start = i + 1; // skip opening "
            i += 1;
            col += 1;
            let mut string_len = 0;
            let mut found_closing_quote = false;

            while i < chars.len() {
                if chars[i] == '"' {
                    found_closing_quote = true;
                    break;
                }

                string_len += 1;

                // Check string length to prevent OOM
                if string_len > LEXER_MAX_STRING_LENGTH {
                    // Emit error token for excessively long string
                    tokens.push(Token {
                        kind: TokenType::Unknown,
                        value: arena.alloc_str("String literal too long"),
                        line: token_line,
                        col: token_col,
                    });

                    // Skip to end of file or next quote, with safety limit
                    let skip_start = i;
                    while i < chars.len() && (i - skip_start) < LEXER_MAX_STRING_LENGTH {
                        if chars[i] == '"' {
                            i += 1; // skip the closing quote
                            col += 1;
                            break;
                        }
                        if chars[i] == '\n' {
                            line += 1;
                            col = 1;
                        } else {
                            col += 1;
                        }
                        i += 1;
                    }

                    // If we still haven't found a quote, skip to end
                    if i >= chars.len() || (i - skip_start) >= LEXER_MAX_STRING_LENGTH {
                        // Skip remaining input or advance past the too-long string
                        if i < chars.len() {
                            i = chars.len().min(i + 100); // Advance at most 100 chars
                        }
                    }
                    // IMPORTANT: Don't use continue here - let the outer loop handle next char
                    found_closing_quote = false;
                    break;
                }

                if chars[i] == '\n' {
                    line += 1;
                    col = 1;
                } else {
                    col += 1;
                }
                i += 1;
            }

            // Only emit String token if we found closing quote and string is within limits
            if found_closing_quote && string_len <= LEXER_MAX_STRING_LENGTH {
                let value: String = chars[start..i].iter().collect();
                let value_str = arena.alloc_str(&value);
                tokens.push(Token {
                    kind: TokenType::String,
                    value: value_str,
                    line: token_line,
                    col: token_col,
                });
                i += 1; // skip closing "
                col += 1;
            } else if !found_closing_quote && string_len <= LEXER_MAX_STRING_LENGTH {
                // Unterminated string (no closing quote)
                tokens.push(Token {
                    kind: TokenType::Unknown,
                    value: arena.alloc_str("Unterminated string literal"),
                    line: token_line,
                    col: token_col,
                });
            }
            // If string was too long, we already emitted error token above
            continue;
        }

        // Numbers and floats
        if c.is_digit(10) {
            let token_line = line;
            let token_col = col;
            let start = i;
            let mut has_dot = false;
            let mut has_exp = false;
            let mut exp_idx = 0;
            let mut num_len = 0;

            // Integer part
            while i < chars.len() && chars[i].is_digit(10) {
                num_len += 1;
                // Sanity check: prevent pathologically long number literals
                if num_len > 1000 {
                    tokens.push(Token {
                        kind: TokenType::Unknown,
                        value: arena.alloc_str("Number literal too long"),
                        line: token_line,
                        col: token_col,
                    });
                    // Skip remaining digits
                    while i < chars.len() && chars[i].is_digit(10) {
                        i += 1;
                        col += 1;
                    }
                    continue;
                }
                i += 1;
                col += 1;
            }

            // Fractional part (float only if . is followed by digit and not .. or ..=)
            if i < chars.len() && chars[i] == '.' {
                // Check if this is a range operator, not a float
                if i + 1 < chars.len() && chars[i + 1] == '.' {
                    // Do not consume . here, let range logic above handle it
                } else if i + 1 < chars.len() && chars[i + 1].is_digit(10) {
                    // Only treat as float if there is at least one digit after the dot
                    has_dot = true;
                    i += 1;
                    col += 1;
                    while i < chars.len() && chars[i].is_digit(10) {
                        num_len += 1;
                        if num_len > 1000 {
                            tokens.push(Token {
                                kind: TokenType::Unknown,
                                value: arena.alloc_str("Number literal too long"),
                                line: token_line,
                                col: token_col,
                            });
                            while i < chars.len() && chars[i].is_digit(10) {
                                i += 1;
                                col += 1;
                            }
                            continue;
                        }
                        i += 1;
                        col += 1;
                    }
                } else {
                    // Dot not followed by digit - this is malformed (like "3.")
                    // Tokenize the number and let the dot be tokenized separately
                }
            }

            // Exponent part
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                has_exp = true;
                exp_idx = i;
                i += 1;
                col += 1;
                if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                    i += 1;
                    col += 1;
                }
                let exp_start = i;
                while i < chars.len() && chars[i].is_digit(10) {
                    num_len += 1;
                    if num_len > 1000 {
                        tokens.push(Token {
                            kind: TokenType::Unknown,
                            value: arena.alloc_str("Number literal too long"),
                            line: token_line,
                            col: token_col,
                        });
                        while i < chars.len() && chars[i].is_digit(10) {
                            i += 1;
                            col += 1;
                        }
                        continue;
                    }
                    i += 1;
                    col += 1;
                }
                // If exponent is not followed by digits, treat as integer/float up to 'e'
                if exp_start == i {
                    i = exp_idx; // rewind to before 'e'
                    col -= i - exp_idx;
                    has_exp = false;
                }
            }

            let value: String = chars[start..i].iter().collect();
            let value_str = arena.alloc_str(&value);
            tokens.push(Token {
                kind: if has_dot || has_exp {
                    TokenType::Float
                } else {
                    TokenType::Number
                },
                value: value_str,
                line: token_line,
                col: token_col,
            });
            continue;
        }

        // Alphabetic: keywords or identifiers
        if c.is_alphabetic() || c == '_' {
            let token_line = line;
            let token_col = col;
            let start = i;
            let mut ident_len = 0;
            let mut too_long = false;

            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                ident_len += 1;
                // Prevent unbounded identifier accumulation (UTF-8 sequences can explode)
                if ident_len > LEXER_MAX_IDENTIFIER_LENGTH {
                    too_long = true;
                    // Emit error token for excessively long identifier
                    tokens.push(Token {
                        kind: TokenType::Unknown,
                        value: arena.alloc_str("Identifier too long"),
                        line: token_line,
                        col: token_col,
                    });
                    // Skip to next whitespace or operator with safety limit
                    let skip_start = i;
                    while i < chars.len()
                        && (chars[i].is_alphanumeric() || chars[i] == '_')
                        && (i - skip_start) < 10000
                    // Safety limit
                    {
                        i += 1;
                        col += 1;
                    }
                    break;
                }
                i += 1;
                col += 1;
            }

            // Only emit identifier/keyword token if not too long
            if !too_long {
                // Use char indices for slicing to support unicode
                let word: String = chars[start..i].iter().collect();

                // Handle standalone wildcard pattern
                if word == "_" {
                    let word_str = arena.alloc_str(&word);
                    tokens.push(Token {
                        kind: TokenType::Underscore,
                        value: word_str,
                        line: token_line,
                        col: token_col,
                    });
                } else {
                    let kind = keywords
                        .get(word.as_str())
                        .unwrap_or(&TokenType::Identifier);

                    // Disallow identifiers starting with underscore (except lone _)
                    if word.starts_with('_') || word.contains('_') {
                        let word_str = arena.alloc_str(&word);
                        tokens.push(Token {
                            kind: TokenType::Unknown,
                            value: word_str,
                            line: token_line,
                            col: token_col,
                        });
                    } else {
                        let word_str = arena.alloc_str(&word);
                        tokens.push(Token {
                            kind: *kind,
                            value: word_str,
                            line: token_line,
                            col: token_col,
                        });
                    }
                }
            }
            continue;
        }

        // Operators (single or multi-character)
        let token_line = line;
        let token_col = col;
        let start = i;
        let mut matched = false;
        for len in (1..=3).rev() {
            // check for operators up to length 3
            if i + len <= chars.len() {
                let op: String = chars[start..start + len].iter().collect();
                if let Some(kind) = operators.get(op.as_str()) {
                    let op_str = arena.alloc_str(&op);
                    tokens.push(Token {
                        kind: *kind,
                        value: op_str,
                        line: token_line,
                        col: token_col,
                    });
                    i += len;
                    col += len;
                    matched = true;
                    break;
                }
            }
        }
        if matched {
            continue;
        }

        // Unknown character: emit Unknown token
        let value: String = chars[i..i + 1].iter().collect();
        let value_str = arena.alloc_str(&value);
        tokens.push(Token {
            kind: TokenType::Unknown,
            value: value_str,
            line,
            col,
        });
        i += 1;
        col += 1;
    }

    // Final check: don't return if we've exceeded max tokens during processing
    if tokens.len() > LEXER_MAX_TOKEN_COUNT {
        return vec![Token {
            kind: TokenType::Unknown,
            value: arena.alloc_str("Too many tokens in input"),
            line: 1,
            col: 1,
        }];
    }

    return tokens;
}
