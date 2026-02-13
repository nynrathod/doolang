use crate::lexer::token::TokenType;
use crate::parser::ast::{AstNode, ImportItem};
use crate::parser::parser::{ParseError, ParseResult, Parser};

impl<'a> Parser<'a> {
    /// Parses an import statement with support for:
    /// 1. Single import: import core::math::Add;
    /// 2. Multiple imports: import core::math::{Add, Subtract};
    /// 3. Aliased imports: import core::math::{Add as mathAdd, Subtract};
    /// 4. Wildcard imports: import core::math::*;
    pub fn parse_import(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::Import)?;

        // Parse module path and import items
        let mut path = Vec::new();

        // Parse first identifier
        let first = self.expect(TokenType::Identifier)?;
        path.push(first.value.to_string());

        // Parse :: separated identifiers until we determine what type of import this is
        while self.peek_is(TokenType::ColonColon) {
            self.advance(); // consume ::

            // Now peek at what comes after ::
            if self.peek_is(TokenType::Star) {
                // Wildcard import: import core::math::*;
                self.advance(); // consume *
                self.expect(TokenType::Semi)?;
                return Ok(AstNode::Import {
                    path,
                    items: vec![ImportItem::Wildcard],
                });
            } else if self.peek_is(TokenType::OpenBrace) {
                // Multiple imports: import core::math::{Add, Subtract};
                self.advance(); // consume {
                let items = self.parse_import_list()?;
                self.expect(TokenType::Semi)?;
                return Ok(AstNode::Import { path, items });
            } else if self.peek_is(TokenType::Identifier) {
                // Could be another path component or the final symbol
                // We need to look ahead to determine this
                let ident = self.expect(TokenType::Identifier)?;
                let ident_str = ident.value.to_string();

                // Peek at what comes after this identifier
                if self.peek_is(TokenType::ColonColon) {
                    // Another :: follows, so this is part of the path
                    path.push(ident_str);
                    // Continue the loop to process the next ::
                } else if self.peek_is(TokenType::Semi) {
                    // End of import after ::
                    // This could be either:
                    // 1. Specific symbol import: import std::Math::Abs; (path has 2+ components)
                    // 2. Namespace import: import std::File; (path has 1 component)
                    // If path already has 2+ components, treat last identifier as a symbol import
                    // Otherwise, treat as namespace import
                    self.advance(); // consume ;
                    if path.len() >= 2 {
                        // Single symbol import without braces: import std::Math::Abs;
                        // path = ["std", "Math"], items = [Symbol("Abs")]
                        return Ok(AstNode::Import {
                            path,
                            items: vec![ImportItem::Symbol(ident_str)],
                        });
                    } else {
                        // Namespace import: import std::File;
                        // path = ["std", "File"], items = []
                        path.push(ident_str);
                        return Ok(AstNode::Import {
                            path,
                            items: vec![],
                        });
                    }
                } else if self.peek_is(TokenType::As) {
                    // Handle 'as' keyword for aliasing
                    self.advance(); // consume 'as'
                    let alias_tok = self.expect(TokenType::Identifier)?;
                    let alias = alias_tok.value.to_string();
                    self.expect(TokenType::Semi)?;
                    
                    if path.len() >= 2 {
                        // Symbol aliased import: import std::Http::Server as S;
                        // path = ["std", "Http"], items = [SymbolWithAlias("Server", "S")]
                        return Ok(AstNode::Import {
                            path,
                            items: vec![ImportItem::SymbolWithAlias(ident_str, alias)],
                        });
                    } else {
                        // Namespace aliased import: import std::Array as Arr;
                        // path = ["std", "Array"], items = [SymbolWithAlias("Array", "Arr")]
                        path.push(ident_str.clone());
                        return Ok(AstNode::Import {
                            path,
                            items: vec![ImportItem::SymbolWithAlias(ident_str, alias)],
                        });
                    }
                } else {
                    return Err(ParseError::UnexpectedTokenAt {
                        msg: "Expected ;, as, or :: after identifier in import".to_string(),
                        line: self.peek().map(|t| t.line).unwrap_or(0),
                        col: self.peek().map(|t| t.col).unwrap_or(0),
                    });
                }
            } else {
                return Err(ParseError::UnexpectedTokenAt {
                    msg: "Expected identifier, wildcard, or brace after ::".to_string(),
                    line: self.peek().map(|t| t.line).unwrap_or(0),
                    col: self.peek().map(|t| t.col).unwrap_or(0),
                });
            }
        }

        // If we get here, we have just one identifier with no ::
        // This could be:
        // 1. Namespace import: import File;
        // 2. Namespace import with alias: import File as F;
        if self.peek_is(TokenType::Semi) {
            self.advance(); // consume ;
            return Ok(AstNode::Import {
                path,
                items: vec![],
            });
        } else if self.peek_is(TokenType::As) {
            // Namespace import with alias: import module as Alias;
            self.advance(); // consume 'as'
            let alias_tok = self.expect(TokenType::Identifier)?;
            let alias = alias_tok.value.to_string();
            self.expect(TokenType::Semi)?;

            // Store the original module name and create an alias
            // We'll use a special item to indicate this is a namespace alias
            let module_name = path.last().unwrap().clone();
            return Ok(AstNode::Import {
                path,
                items: vec![ImportItem::SymbolWithAlias(module_name, alias)],
            });
        }

        // Otherwise, invalid syntax
        Err(ParseError::UnexpectedTokenAt {
            msg: "Expected ; or :: after identifier in import".to_string(),
            line: self.peek().map(|t| t.line).unwrap_or(0),
            col: self.peek().map(|t| t.col).unwrap_or(0),
        })
    }

    /// Parses a comma-separated list of import items within braces
    /// Handles: {Add, Subtract} or {Add as mathAdd, Subtract}
    fn parse_import_list(&mut self) -> ParseResult<Vec<ImportItem>> {
        let mut items = Vec::new();

        // Parse items until we hit closing brace
        while !self.peek_is(TokenType::CloseBrace) {
            // Wildcard import within braces: {*}
            if self.peek_is(TokenType::Star) {
                self.advance();
                items.push(ImportItem::Wildcard);
            } else if self.peek_is(TokenType::Identifier) {
                let name_tok = self.expect(TokenType::Identifier)?;
                let name = name_tok.value.to_string();

                // Check for 'as' keyword for aliasing
                if self.peek_is(TokenType::As) {
                    self.advance(); // consume 'as'
                    let alias_tok = self.expect(TokenType::Identifier)?;
                    let alias = alias_tok.value.to_string();
                    items.push(ImportItem::SymbolWithAlias(name, alias));
                } else {
                    items.push(ImportItem::Symbol(name));
                }
            } else {
                return Err(ParseError::UnexpectedTokenAt {
                    msg: "Expected identifier or wildcard in import list".to_string(),
                    line: self.peek().map(|t| t.line).unwrap_or(0),
                    col: self.peek().map(|t| t.col).unwrap_or(0),
                });
            }

            // Check for comma to continue, or closing brace to end
            if self.peek_is(TokenType::Comma) {
                self.advance(); // consume comma
                                // Allow trailing comma before closing brace
                if self.peek_is(TokenType::CloseBrace) {
                    break;
                }
            } else if !self.peek_is(TokenType::CloseBrace) {
                return Err(ParseError::UnexpectedTokenAt {
                    msg: "Expected comma or closing brace in import list".to_string(),
                    line: self.peek().map(|t| t.line).unwrap_or(0),
                    col: self.peek().map(|t| t.col).unwrap_or(0),
                });
            }
        }

        self.expect(TokenType::CloseBrace)?;

        if items.is_empty() {
            return Err(ParseError::UnexpectedTokenAt {
                msg: "Import list cannot be empty".to_string(),
                line: self.peek().map(|t| t.line).unwrap_or(0),
                col: self.peek().map(|t| t.col).unwrap_or(0),
            });
        }

        Ok(items)
    }
}
