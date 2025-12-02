use crate::lexer::token::TokenType;
use crate::parser::ast::{AstNode, Pattern, TypeNode};
use crate::parser::{ParseError, ParseResult, Parser};

impl<'a> Parser<'a> {
    /// Let decl handles optional 'mut', pattern, optional type annotation, assignment, and semicolon.
    /// Example: `let mut x: Int = 42;`
    /// Also supports manual error extraction: `let a, b , err = expr;`
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

        // Check for , operator (manual error extraction)
        if let Some(tok) = self.peek() {
            if tok.kind == TokenType::DoubleQuestion {
                self.advance(); // consume '??'

                // Parse the error variable name (or _ to ignore)
                let error_var = if let Some(tok) = self.peek() {
                    if tok.kind == TokenType::Underscore {
                        self.advance(); // consume '_'
                        "_".to_string()
                    } else {
                        self.expect_ident()?
                    }
                } else {
                    return Err(ParseError::EndOfInput);
                };

                // Parse assignment operator '='
                self.expect(TokenType::Eq)?;

                // Parse the expression that returns a Result
                let expr = self.parse_expression()?;

                // Expect a semicolon
                self.expect(TokenType::Semi)?;

                // Return ManualErrorExtract node wrapped in LetDecl
                return Ok(AstNode::ManualErrorExtract {
                    expr: Box::new(expr),
                    ok_pattern: pattern,
                    error_var,
                });
            }
        }

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
        self.parse_functional_decl_with_decorators(Vec::new())
    }

    /// Function decl with decorators (for FFI support) and method syntax support
    /// Example: `@ffi("libname") @extern("c_func") fn foo(a: Int) -> Int { ... }`
    /// Example: `fn User.isAdult(self) -> Bool { ... }`
    /// Example: `fn add(a: Int, b: Int) -> Int => a + b` (expression function)
    /// Example: `fn greet(name: Str, prefix: Str = "Hello") { ... }` (default params)
    pub fn parse_functional_decl_with_decorators(
        &mut self,
        decorators: Vec<crate::parser::ast::Decorator>,
    ) -> ParseResult<AstNode> {
        self.expect(TokenType::Function)?; // consume 'fn'

        // Parse function name (identifier)
        let func_name = self.expect_ident()?;

        // Check if this is a method declaration (Type.method syntax)
        let (receiver_type, actual_func_name) = if self.peek_is(TokenType::Dot) {
            self.advance(); // consume '.'
            let method_name = self.expect_ident()?;
            (Some(func_name), method_name)
        } else {
            (None, func_name)
        };

        // Determine visibility based on naming convention (uppercase = public)
        let visibility = if actual_func_name
            .chars()
            .next()
            .unwrap_or('a')
            .is_uppercase()
        {
            "Public".to_string()
        } else {
            "Private".to_string()
        };

        self.expect(TokenType::OpenParen)?; // consume '('

        // Parse function parameters until ')' is found
        // Track if we've seen the first parameter (for methods)
        let mut is_first_param = true;
        let params = self.parse_comma_separated(
            |p| {
                let param_name = p.expect_ident()?;

                // Special case: first parameter in methods doesn't need type annotation (receiver)
                if is_first_param && receiver_type.is_some() {
                    is_first_param = false;
                    // Receiver parameter - no type annotation needed, type is inferred from receiver
                    return Ok((param_name, None));
                }
                is_first_param = false;

                // Enforce mandatory type annotation for each parameter (except receiver)
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

                // Check for default value: = expr
                if p.peek_is(TokenType::Eq) {
                    p.advance(); // consume '='
                    let default_expr = p.parse_expression()?;
                    // For now, store default value in a modified param structure
                    // We'll wrap this in a special marker later if needed
                    // Since params is Vec<(String, Option<TypeNode>)>, we need to extend AST
                    // For now, ignore default values - they'll need AST changes
                }

                Ok((param_name, param_type))
            },
            TokenType::CloseParen,
        )?;

        self.expect(TokenType::CloseParen)?; // consume ')'

        // Parse optional return type (e.g., '-> Type') and/or error type (e.g., '! Type')
        let mut return_type = None;
        let mut error_type = None;

        if let Some(tok) = self.peek() {
            if tok.kind == TokenType::Arrow {
                // e.g., '->'
                self.advance();

                // Check if next token is '!' (error-only function with ->)
                if let Some(next_tok) = self.peek() {
                    if next_tok.kind == TokenType::Bang {
                        // This is -> ! ErrorType (no success return, kept for backward compatibility)
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
            } else if tok.kind == TokenType::Bang {
                // Direct error type without -> (e.g., fn foo() ! Str)
                self.advance(); // consume '!'
                error_type = Some(self.parse_type_annotation()?);
            }
        }

        // Check for expression function: => expr or => expr1, expr2, ...
        if self.peek_is(TokenType::FatArrow) {
            self.advance(); // consume '=>'

            // Parse comma-separated expressions for return values
            let mut return_values = Vec::new();
            loop {
                let expr = self.parse_expression()?;
                return_values.push(expr);

                // Check for comma to continue parsing more return values
                if self.peek_is(TokenType::Comma) {
                    self.advance(); // consume ','
                } else {
                    break;
                }
            }

            // Infer return type if not specified
            if return_type.is_none() {
                // Return type will be inferred by analyzer
            }

            // Expect semicolon at the end of inline function
            self.expect(TokenType::Semi)?;

            // Wrap expressions in a return statement
            let body_block = vec![AstNode::Return {
                values: return_values,
            }];

            return Ok(AstNode::FunctionDecl {
                name: actual_func_name,
                visibility,
                params,
                return_type,
                error_type,
                body: body_block,
                decorators,
                receiver_type,
                is_expression: true,
            });
        }

        // Parse function body block
        let body_block = self.parse_braced_block()?; // parse function body

        Ok(AstNode::FunctionDecl {
            name: actual_func_name,
            visibility,
            params,
            return_type,
            error_type,
            body: body_block,
            decorators,
            receiver_type,
            is_expression: false,
        })
    }

    /// Struct decl with full support for decorators, optional fields, default values
    /// Example: `struct User { email: Str @email @unique, age: Int?, Role: Str = "user" }`
    pub fn parse_struct_decl(&mut self) -> ParseResult<AstNode> {
        self.expect(TokenType::Struct)?; // consume 'struct'

        let struct_name = self.expect_ident()?; // Parse struct name
        let is_public = crate::parser::ast::TypeNode::is_public_name(&struct_name);

        self.expect(TokenType::OpenBrace)?; // `{`

        // Parse fields until closing brace
        let mut fields = Vec::new();

        while !self.peek_is(TokenType::CloseBrace) {
            // Parse field with all metadata
            let field = self.parse_struct_field()?;
            fields.push(field);

            // Expect comma or closing brace
            if !self.peek_is(TokenType::CloseBrace) {
                self.expect(TokenType::Comma)?;
            }
        }

        self.expect(TokenType::CloseBrace)?;

        Ok(AstNode::StructDecl {
            name: struct_name,
            fields,
            is_public,
        })
    }

    /// Parse a single struct field with decorators, optional marker, and default value
    /// Example: `email: Str @email @unique` or `age: Int?` or `Role: Str = "user"`
    fn parse_struct_field(&mut self) -> ParseResult<crate::parser::ast::StructField> {
        use crate::parser::ast::{Decorator, StructField, TypeNode};

        let field_name = self.expect_ident()?;
        self.expect(TokenType::Colon)?;

        // Parse the base type
        let mut field_type = self.parse_type_annotation()?;

        // Check for optional marker (?)
        let is_optional = if self.peek_is(TokenType::Question) {
            self.advance();
            true
        } else {
            false
        };

        // Wrap in Optional type if marked optional
        if is_optional {
            field_type = TypeNode::Optional(Box::new(field_type));
        }

        // Parse decorators (@email, @unique, @min(8), etc.)
        let mut decorators = Vec::new();
        while self.peek_is(TokenType::At) {
            self.advance(); // consume '@'
            let decorator = self.parse_decorator()?;
            decorators.push(decorator);
        }

        // Parse default value (= expr)
        let default_value = if self.peek_is(TokenType::Eq) {
            self.advance(); // consume '='
            Some(Box::new(self.parse_expression()?))
        } else {
            None
        };

        // Create field with metadata
        let is_public = TypeNode::is_public_name(&field_name);
        Ok(StructField {
            name: field_name,
            field_type,
            is_public,
            is_optional,
            default_value,
            decorators,
        })
    }

    /// Parse a decorator like @email or @min(8) or @hash
    pub fn parse_decorator(&mut self) -> ParseResult<crate::parser::ast::Decorator> {
        use crate::parser::ast::Decorator;

        let decorator_name = self.expect_ident()?;

        // Check for arguments
        let args = if self.peek_is(TokenType::OpenParen) {
            self.advance(); // consume '('
            let args =
                self.parse_comma_separated(|p| p.parse_expression(), TokenType::CloseParen)?;
            self.expect(TokenType::CloseParen)?;
            args
        } else {
            Vec::new()
        };

        Ok(Decorator::new(decorator_name, args))
    }

    /// Enum decl handles enum name, variants (with optional associated types), and braces.
    /// Supports two syntaxes:
    /// 1. Block syntax: `enum UserRole { Admin(AdminRole), Guest }`
    /// 2. Inline syntax: `enum UserRole: Admin | Guest | Moderator(Str)`
    pub fn parse_enum_decl(&mut self) -> ParseResult<AstNode> {
        use crate::parser::ast::EnumVariant;

        self.expect(TokenType::Enum)?; // consume 'enum'

        // Parse enum name
        let enum_name = self.expect_ident()?;
        let is_public = crate::parser::ast::TypeNode::is_public_name(&enum_name);

        // Check for inline syntax (colon) or block syntax (brace)
        let variants = if self.peek_is(TokenType::Colon) {
            // Inline syntax: enum Name: Variant1 | Variant2 | Variant3(Type)
            self.advance(); // consume ':'

            let mut variants = Vec::new();

            // Parse first variant
            let variant_name = self.expect_ident_or_keyword()?;
            let mut variant_data = None;
            if self.peek_is(TokenType::OpenParen) {
                self.advance();
                // Parse first type
                let first_type = self.parse_type_annotation()?;

                // Check for comma (multiple types = tuple)
                if self.peek_is(TokenType::Comma) {
                    let mut types = vec![first_type];
                    while self.peek_is(TokenType::Comma) {
                        self.advance(); // consume ','
                        types.push(self.parse_type_annotation()?);
                    }
                    variant_data = Some(TypeNode::Tuple(types));
                } else {
                    variant_data = Some(first_type);
                }
                self.expect(TokenType::CloseParen)?;
            }
            variants.push(EnumVariant::new(variant_name, variant_data));

            // Parse remaining variants separated by |
            while self.peek_is(TokenType::Or) {
                self.advance(); // consume '|'
                let variant_name = self.expect_ident_or_keyword()?;
                let mut variant_data = None;
                if self.peek_is(TokenType::OpenParen) {
                    self.advance();
                    // Parse first type
                    let first_type = self.parse_type_annotation()?;

                    // Check for comma (multiple types = tuple)
                    if self.peek_is(TokenType::Comma) {
                        let mut types = vec![first_type];
                        while self.peek_is(TokenType::Comma) {
                            self.advance(); // consume ','
                            types.push(self.parse_type_annotation()?);
                        }
                        variant_data = Some(TypeNode::Tuple(types));
                    } else {
                        variant_data = Some(first_type);
                    }
                    self.expect(TokenType::CloseParen)?;
                }
                variants.push(EnumVariant::new(variant_name, variant_data));
            }

            // Expect semicolon for inline syntax
            self.expect(TokenType::Semi)?;

            variants
        } else {
            // Block syntax: enum Name { Variant1, Variant2, Variant3(Type) }
            self.expect(TokenType::OpenBrace)?;

            // Parse variants until closing brace
            let variants = self.parse_comma_separated(
                |p| {
                    let variant_name = p.expect_ident_or_keyword()?;
                    let mut variant_data = None;
                    if let Some(tok) = p.peek() {
                        if tok.kind == TokenType::OpenParen {
                            p.advance();
                            // Parse first type
                            let first_type = p.parse_type_annotation()?;

                            // Check for comma (multiple types = tuple)
                            if p.peek_is(TokenType::Comma) {
                                let mut types = vec![first_type];
                                while p.peek_is(TokenType::Comma) {
                                    p.advance(); // consume ','
                                    types.push(p.parse_type_annotation()?);
                                }
                                variant_data = Some(TypeNode::Tuple(types));
                            } else {
                                variant_data = Some(first_type);
                            }
                            p.expect(TokenType::CloseParen)?;
                        }
                    }
                    Ok(EnumVariant::new(variant_name, variant_data))
                },
                TokenType::CloseBrace,
            )?;

            self.expect(TokenType::CloseBrace)?;

            variants
        };

        Ok(AstNode::EnumDecl {
            name: enum_name,
            variants,
            is_public,
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
                "Error" => Ok(TypeNode::Error),
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
