use crate::lexer::token::TokenType;
use crate::parser::ast::{AstNode, Pattern, TypeNode};
use crate::parser::{ParseError, ParseResult, Parser};

impl<'a> Parser<'a> {
    /// Let decl handles optional 'mut', pattern, optional type annotation, assignment, and semicolon.
    /// Example: `let mut x: Int = 42;`
    pub fn parse_let_decl(&mut self) -> ParseResult<AstNode> {
        // Consume the 'let' keyword
        let first_tok = self.advance().ok_or(ParseError::EndOfInput)?;
        if first_tok.kind != TokenType::Let {
            return Err(ParseError::UnexpectedTokenAt {
                msg: "Expected 'let'".into(),
                line: first_tok.line,
                col: first_tok.col,
            });
        }

        // Check for optional 'mut' keyword (mutable variable)
        let mut mutable = false;
        if let Some(tok) = self.peek() {
            if tok.kind == TokenType::Mut {
                self.advance(); // consume 'mut'
                mutable = true;
            }
        }

        // Parse the pattern (single or tuple of variables)
        let pattern = self.parse_let_pattern()?;

        // Parse optional type annotation (e.g., ': Int')
        let mut type_annotation = None;
        if let Some(tok) = self.peek() {
            if tok.kind == TokenType::Colon {
                self.advance(); // consume ':'
                let parsed_type = self.parse_type_annotation()?;
                type_annotation = Some(parsed_type);
            }
        }

        // Parse assignment operator '=' and the expression to assign
        self.expect(TokenType::Eq)?;
        let value = self.parse_expression()?;

        // Expect a semicolon at the end of the statement
        self.expect(TokenType::Semi)?;

        Ok(AstNode::LetDecl {
            mutable,
            type_annotation,
            pattern,
            value: Box::new(value),
            is_ref_counted: None,
        })
    }

    /// Function decl handles function name, parameters (with mandatory types),
    /// optional return type, optional error type (with !), and body block.
    /// Example: `fn foo(a: Int, b: Str) -> Str ! Str { ... }`
    pub fn parse_functional_decl(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::Function)?; // consume 'fn'

        // Parse function name (identifier)
        let func_name = self.expect_ident()?;

        // Determine visibility based on naming convention (uppercase = public)
        let visibility = if func_name.chars().next().unwrap_or('a').is_uppercase() {
            "Public".to_string()
        } else {
            "Private".to_string()
        };

        self.expect(TokenType::OpenParen)?; // consume '('

        // Parse function parameters until ')' is found
        let params = self.parse_comma_separated(
            |p| {
                let param_name = p.expect_ident()?;
                // Enforce mandatory type annotation for each parameter
                let tok = p.peek().ok_or(ParseError::EndOfInput)?;
                if tok.kind != TokenType::Colon {
                    return Err(ParseError::UnexpectedTokenAt {
                        msg: "Function parameter type annotation is required".to_string(),
                        line: tok.line,
                        col: tok.col,
                    });
                }
                p.advance(); // consume ':'
                let param_type = Some(p.parse_type_annotation()?);
                Ok((param_name, param_type))
            },
            TokenType::CloseParen,
        )?;

        self.expect(TokenType::CloseParen)?; // consume ')'

        // Parse optional return type (e.g., '-> Type')
        let mut return_type = None;
        let mut error_type = None;

        if let Some(tok) = self.peek() {
            if tok.kind == TokenType::Arrow {
                // e.g., '->'
                self.advance();

                // Check if next token is '!' (error-only function)
                if let Some(next_tok) = self.peek() {
                    if next_tok.kind == TokenType::Bang {
                        // This is -> ! ErrorType (no success return)
                        self.advance(); // consume '!'
                        error_type = Some(self.parse_type_annotation()?);
                    } else {
                        // This is -> ReturnType [! ErrorType]
                        return_type = Some(self.parse_return_type()?);

                        // Now check for optional error type
                        if let Some(err_tok) = self.peek() {
                            if err_tok.kind == TokenType::Bang {
                                self.advance(); // consume '!'
                                error_type = Some(self.parse_type_annotation()?);
                            }
                        }
                    }
                }
            }
        }

        // Parse function body block
        let body_block = self.parse_braced_block()?; // parse function body

        Ok(AstNode::FunctionDecl {
            name: func_name,
            visibility,
            params,
            return_type,
            error_type,
            body: body_block,
        })
    }

    /// Struct decl Handles struct name, fields (name and type), and braces.
    /// Example: `struct Foo { x: Int, y: Str }`
    pub fn parse_struct_decl(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::Struct)?; // consume 'struct'

        let struct_name = self.expect_ident()?; // Parse struct name

        self.expect(TokenType::OpenBrace)?; // `{`

        // Parse fields until closing brace
        let fields = self.parse_comma_separated(
            |p| {
                let field_name = p.expect_ident()?;
                p.expect(TokenType::Colon)?;
                let field_type = p.parse_type_annotation()?;
                Ok((field_name, field_type))
            },
            TokenType::CloseBrace,
        )?;

        self.expect(TokenType::CloseBrace)?;

        Ok(AstNode::StructDecl {
            name: struct_name,
            fields,
        })
    }

    /// Enum decl handles enum name, variants (with optional associated types), and braces.
    /// Example: `enum Bar { A, B(Int), C(Str) }`
    pub fn parse_enum_decl(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::Enum)?; // consume 'enum'

        // Parse enum name
        let struct_name = self.expect_ident()?;

        self.expect(TokenType::OpenBrace)?;

        // Parse variants until closing brace
        // Enum doesn't support multiple parameter in type
        let variants = self.parse_comma_separated(
            |p| {
                let variant_name = p.expect_ident()?;
                let mut variant_data = None;
                if let Some(tok) = p.peek() {
                    if tok.kind == TokenType::OpenParen {
                        p.advance();
                        let types = p.parse_type_annotation()?;
                        variant_data = Some(types);
                        p.expect(TokenType::CloseParen)?;
                    }
                }
                Ok((variant_name, variant_data))
            },
            TokenType::CloseBrace,
        )?;

        self.expect(TokenType::CloseBrace)?;

        Ok(AstNode::EnumDecl {
            name: struct_name,
            variants,
        })
    }

    /// Parses a pattern for a 'let' declaration.
    /// Supports single identifiers and tuple patterns
    /// 🟡 TODO: Doo does not supporting tuple patter yet in var decl yet
    /// (e.g., `let x, y = ...` or with parentheses `let (x, y) = ...`).
    fn parse_let_pattern(&mut self) -> ParseResult<Pattern> {
        // - `x` → single identifier
        // - `x, y, z` → tuple pattern
        let patterns = self.parse_comma_separated(|p| p.parse_pattern(), TokenType::Eq)?;

        // Error if no variable name is provided (e.g., `let = 42;`)
        if patterns.is_empty() {
            if let Some(tok) = self.peek() {
                return Err(ParseError::UnexpectedTokenAt {
                    msg: "Missing variable name in let declaration".into(),
                    line: tok.line,
                    col: tok.col,
                });
            } else {
                return Err(ParseError::UnexpectedToken(
                    "Missing variable name in let declaration".into(),
                ));
            }
        }

        // If only one pattern, return it directly; otherwise, return a tuple pattern
        if patterns.len() == 1 {
            Ok(patterns.into_iter().next().unwrap())
        } else {
            Ok(Pattern::Tuple(patterns))
        }
    }

    /// Parses a function return type.
    /// Supports single types and tuple types (e.g., `-> Int` or `-> Str, Int`).
    /// Multiple return types are separated by commas without parentheses
    fn parse_return_type(&mut self) -> ParseResult<TypeNode> {
        if self.peek().is_none() {
            return Err(ParseError::EndOfInput);
        }

        // Parse first type
        let first_type = self.parse_type_annotation()?;

        // Check if there are more types (comma-separated tuple)
        let mut types = vec![first_type];

        while let Some(tok) = self.peek() {
            if tok.kind == TokenType::Comma {
                self.advance(); // consume ','
                let next_type = self.parse_type_annotation()?;
                types.push(next_type);
            } else {
                // Stop at any non-comma token (e.g., '{' for function body)
                break;
            }
        }

        // Return tuple if multiple types, otherwise single type
        if types.len() > 1 {
            Ok(TypeNode::Tuple(types))
        } else {
            Ok(types.into_iter().next().unwrap())
        }
    }

    /// Supports arrays, maps, primitive types
    /// Examples: `Int`, `[Int]`, `{Str: Int}`, `Bool`
    /// Note: User defined types are not supported yet.
    pub fn parse_type_annotation(&mut self) -> ParseResult<TypeNode> {
        self.depth += 1;
        if self.depth > super::parser::MAX_DEPTH {
            self.depth -= 1;
            return Err(ParseError::UnexpectedTokenAt {
                msg: "Type annotation nesting too deep (recursion limit exceeded)".to_string(),
                line: self.peek().map(|t| t.line).unwrap_or(0),
                col: self.peek().map(|t| t.col).unwrap_or(0),
            });
        }

        let result = if self.peek_is(TokenType::OpenBracket) {
            // Array type: [Type]
            self.advance(); // consume '['
            let inner = self.parse_type_annotation()?;
            self.expect(TokenType::CloseBracket)?;
            Ok(TypeNode::Array(Box::new(inner)))
        } else if self.peek_is(TokenType::OpenBrace) {
            // Map type: {KeyType: ValueType}
            self.advance(); // consume '{'
            let key = self.parse_type_annotation()?;
            self.expect(TokenType::Colon)?;
            let value = self.parse_type_annotation()?;
            self.expect(TokenType::CloseBrace)?;
            Ok(TypeNode::Map(Box::new(key), Box::new(value)))
        } else if self.peek_is(TokenType::Identifier) {
            // Primitive type
            let tok = self.advance().unwrap();
            match tok.value {
                "Int" => Ok(TypeNode::Int),
                "Float" => Ok(TypeNode::Float),
                "Str" => Ok(TypeNode::String),
                "Bool" => Ok(TypeNode::Bool),
                other => {
                    // Accept any previously declared struct as type
                    Ok(TypeNode::TypeRef(other.to_string()))
                }
            }
        } else {
            Err(ParseError::UnexpectedToken(
                "Expected type annotation".into(),
            ))
        };

        self.depth -= 1;
        result
    }

    /// Expects and parses an identifier token, returning its string value.
    fn expect_ident(&mut self) -> ParseResult<String> {
        let tok = self.expect(TokenType::Identifier)?;
        Ok(tok.value.to_string())
    }
}
