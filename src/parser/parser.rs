use crate::lexer::token::{Token, TokenType};
use crate::limits::PARSER_MAX_DEPTH;
use crate::parser::ast::AstNode;
use std::fmt;

/// Re-export for backwards compatibility
pub const MAX_DEPTH: usize = PARSER_MAX_DEPTH;

/// Error type for parser.
/// Used to signal parsing failures, such as unexpected tokens or premature end of input.
#[allow(dead_code)]
#[derive(Debug)]
pub enum ParseError {
    UnexpectedToken(String),
    UnexpectedTokenAt {
        msg: String,
        line: usize,
        col: usize,
    },
    EndOfInput,
}

/// Standard result type for parsing.
/// Wraps either a successful parse result or a ParseError.
pub type ParseResult<T> = Result<T, ParseError>;

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnexpectedToken(msg) => write!(f, "Parse error: {}", msg),
            ParseError::UnexpectedTokenAt { msg, line, col } => {
                write!(f, "Parse error at {}:{}: {}", line, col, msg)
            }
            ParseError::EndOfInput => write!(f, "Parse error: unexpected end of input"),
        }
    }
}

/// The Parser struct is the stateful engine. It consumes tokens (from lexer)
/// and builds AST nodes (for analyzer, codegen, etc).
#[derive(Debug)]
pub struct Parser<'a> {
    pub tokens: &'a [Token<'a>], // Reference to a slice of tokens from lexer.
    pub current: usize,          // Current index; tracks progress through tokens.
    pub depth: usize,            // Current recursion depth to prevent stack overflow.
}

impl<'a> Parser<'a> {
    /// Create a new parser for a given token stream.
    pub fn new(tokens: &'a [Token<'a>]) -> Self {
        Parser {
            tokens,
            current: 0,
            depth: 0,
        }
    }

    /// Peek at the current token without advancing.
    /// Used in almost every parse function to check what's next.
    pub fn peek(&self) -> Option<&Token<'a>> {
        self.tokens.get(self.current)
    }

    /// Peek ahead N tokens without advancing.
    /// Used to look ahead for patterns like :: in enum matching.
    pub fn peek_ahead(&self, n: usize) -> Option<&Token<'a>> {
        self.tokens.get(self.current + n)
    }

    /// Checks if the current token matches a given kind.
    pub(crate) fn peek_is(&self, kind: TokenType) -> bool {
        self.peek().map(|tok| tok.kind == kind).unwrap_or(false)
    }

    /// Advance to the next token and return the previous one.
    pub fn advance(&mut self) -> Option<&Token<'a>> {
        let tok = self.tokens.get(self.current);
        if tok.is_some() {
            self.current += 1;
        }
        tok
    }

    /// If the current token matches the given kind, consume it and return true.
    /// Otherwise, do nothing and return false.
    pub(crate) fn consume_if(&mut self, kind: TokenType) -> bool {
        if self.peek_is(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Expect the next token to be of a specific kind.
    /// If it matches, consume and return it.
    /// If not, return a ParseError.
    pub(crate) fn expect(&mut self, kind: TokenType) -> ParseResult<&Token<'a>> {
        match self.advance() {
            Some(tok) if tok.kind == kind => Ok(tok),
            Some(tok) => Err(ParseError::UnexpectedTokenAt {
                msg: format!("Expected {:?}, got {:?} ({:?})", kind, tok.kind, tok.value),
                line: tok.line,
                col: tok.col,
            }),
            None => Err(ParseError::EndOfInput),
        }
    }

    /// Expect an identifier or allow keywords like Ok/Err for enum variants
    pub(crate) fn expect_ident_or_keyword(&mut self) -> ParseResult<String> {
        match self.advance() {
            Some(tok) => match tok.kind {
                TokenType::Identifier => Ok(tok.value.to_string()),
                TokenType::Ok => Ok("Ok".to_string()),
                TokenType::Err => Ok("Err".to_string()),
                _ => Err(ParseError::UnexpectedTokenAt {
                    msg: format!("Expected Identifier, got {:?} ({:?})", tok.kind, tok.value),
                    line: tok.line,
                    col: tok.col,
                }),
            },
            None => Err(ParseError::EndOfInput),
        }
    }

    /// Parses a single statement.
    /// Dispatches to the correct parse function based on the current token.
    /// Handles declarations, control flow, assignments, and expression statements.
    pub fn parse_statement(&mut self) -> ParseResult<AstNode> {
        match self.peek() {
            Some(tok) => match tok.kind {
                // Decorators (for FFI functions, etc.)
                TokenType::At => {
                    // Parse decorators and then the following function
                    let mut decorators = Vec::new();
                    while self.peek_is(TokenType::At) {
                        self.advance(); // consume '@'
                        let decorator = self.parse_decorator()?;
                        decorators.push(decorator);
                    }

                    // After decorators, we expect a function declaration
                    if self.peek_is(TokenType::Function) {
                        return self.parse_functional_decl_with_decorators(decorators);
                    } else {
                        return Err(ParseError::UnexpectedToken(
                            "Decorators are currently only supported on functions".into(),
                        ));
                    }
                }

                // Declarations
                TokenType::Let => self.parse_let_decl(),
                TokenType::Function => self.parse_functional_decl(),
                TokenType::Struct => self.parse_struct_decl(),
                TokenType::Enum => self.parse_enum_decl(),

                // Import statement
                TokenType::Import => self.parse_import(),

                // Statements
                TokenType::If => self.parse_conditional_stmt(),
                TokenType::For => self.parse_for_stmt(),
                TokenType::Match => {
                    // Parse match expression - no semicolon required after }
                    self.parse_match_expr()
                }
                TokenType::Return => self.parse_return(),
                TokenType::Break => self.parse_break(),
                TokenType::Continue => self.parse_continue(),
                TokenType::Print => self.parse_print(),

                // Ok and Err expressions as statements (implicit returns in Result functions)
                TokenType::Ok | TokenType::Err => {
                    let expr = self.parse_expression()?;
                    self.expect(TokenType::Semi)?;
                    Ok(expr)
                }

                TokenType::OpenBrace => {
                    // Handle empty block or block statement: {}
                    self.advance(); // consume '{'
                    self.expect(TokenType::CloseBrace)?; // expect '}'
                    Ok(AstNode::Block(vec![]))
                }

                // Handles statements that start with an identifier.
                // Could be assignment (x = 5;) or compound assignment (x += 1;) or expression statement (abc();)
                TokenType::Identifier => {
                    // Try to parse as expression first (handles function calls)
                    let expr = self.parse_expression()?;

                    // Check if it's followed by '=' or compound assignment operator
                    if let Some(tok) = self.peek() {
                        match tok.kind {
                            TokenType::Eq => {
                                self.advance(); // consume '='
                                let value = self.parse_expression()?;
                                self.expect(TokenType::Semi)?;

                                // Handle array/map element assignment: arr[index] = value
                                match expr {
                                    AstNode::Identifier(name) => {
                                        return Ok(AstNode::Assignment {
                                            pattern: crate::parser::ast::Pattern::Identifier(name),
                                            value: Box::new(value),
                                        });
                                    }
                                    AstNode::ElementAccess { array, index } => {
                                        return Ok(AstNode::ElementAssignment {
                                            array,
                                            index,
                                            value: Box::new(value),
                                        });
                                    }
                                    _ => {
                                        return Err(ParseError::UnexpectedToken(
                                            "Only single-variable or element assignment is allowed without 'let'"
                                                .into(),
                                        ));
                                    }
                                }
                            }
                            TokenType::PlusEq
                            | TokenType::MinusEq
                            | TokenType::StarEq
                            | TokenType::SlashEq
                            | TokenType::PercentEq => {
                                let op = tok.kind;
                                self.advance(); // consume compound operator
                                let value = self.parse_expression()?;
                                self.expect(TokenType::Semi)?;

                                // Extract identifier from expr for compound assignment
                                if let AstNode::Identifier(name) = expr {
                                    return Ok(AstNode::CompoundAssignment {
                                        pattern: crate::parser::ast::Pattern::Identifier(name),
                                        op,
                                        value: Box::new(value),
                                    });
                                } else {
                                    return Err(ParseError::UnexpectedToken(
                                        "Only single-variable compound assignment is allowed"
                                            .into(),
                                    ));
                                }
                            }
                            TokenType::PlusPlus | TokenType::MinusMinus => {
                                let op = tok.kind;
                                self.advance(); // consume ++ or --
                                self.expect(TokenType::Semi)?;

                                // Extract identifier from expr for increment/decrement
                                if let AstNode::Identifier(name) = expr {
                                    return Ok(AstNode::IncrementDecrement { variable: name, op });
                                } else {
                                    return Err(ParseError::UnexpectedToken(
                                        "Only single-variable increment/decrement is allowed"
                                            .into(),
                                    ));
                                }
                            }
                            _ => {
                                // It's an expression statement (like function call)
                                self.expect(TokenType::Semi)?;
                                return Ok(expr);
                            }
                        }
                    } else {
                        // It's an expression statement (like function call)
                        self.expect(TokenType::Semi)?;
                        return Ok(expr);
                    }
                }

                TokenType::Number | TokenType::Float => {
                    // Disallow number/float literals as statements.
                    // Example: `42;` or `3.14;` is not allowed as a statement.
                    let tok = self.peek().unwrap();
                    return Err(ParseError::UnexpectedTokenAt {
                        msg: "Invalid expression as statement (e.g. `42;` is not allowed)"
                            .to_string(),
                        line: tok.line,
                        col: tok.col,
                    });
                }

                // If the token doesn't match any known statement start, check for Unknown token and handle error.
                _ => Err(ParseError::UnexpectedTokenAt {
                    msg: format!("Unexpected token: {:?}", tok.kind),
                    line: tok.line,
                    col: tok.col,
                }),
            },
            None => Err(ParseError::EndOfInput),
        }
    }

    /// Parses an entire program (sequence of statements).
    /// Keeps parsing statements until all tokens are consumed.
    pub fn parse_program(&mut self) -> ParseResult<AstNode> {
        let mut statements = Vec::new();
        while self.current < self.tokens.len() {
            let stmt = self.parse_statement()?;
            statements.push(stmt);
        }
        Ok(AstNode::Program(statements))
    }

    /// Parses a comma-separated list of items until an end token is reached.
    ///
    /// This is a generic helper for parsing lists such as function parameters,
    /// struct fields, enum variants, function parameters, return types
    ///
    /// - `parse_item`: a closure that parses a single item from the stream.
    /// - `end_token`: the token that marks the end of the list (e.g., `)` or `}`).
    ///
    /// Example usage:
    ///     parse_comma_separated(|p| p.parse_type_annotation(), TokenType::CloseParen)
    ///
    /// Parsing stops when the end token is encountered or when there are no more commas.
    /// Returns a vector of parsed items.
    pub fn parse_comma_separated<T, F>(
        &mut self,
        mut parse_item: F,
        end_token: TokenType,
    ) -> ParseResult<Vec<T>>
    where
        F: FnMut(&mut Self) -> ParseResult<T>,
    {
        let mut items = Vec::new();
        // Continue parsing items until the end token is found
        while !self.peek_is(end_token) {
            // Parse a single item using the provided closure
            items.push(parse_item(self)?);
            // If there's a comma, consume it and continue parsing the next item
            // If not, break the loop (list is finished)
            if !self.consume_if(TokenType::Comma) {
                break;
            }
        }
        Ok(items)
    }
}
