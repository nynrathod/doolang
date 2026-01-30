use super::expr::ParserExpr;
use super::stmt::ParserStmt;
use super::types::ParserTypes;
use super::{ParseResult, Parser};
use crate::ast::*;
use crate::lexer::TokenKind;
use doo_core::{CompilerError, ErrorCode, Span};

/// Trait for parsing top-level items.
pub trait ParserItems {
    fn parse_item(&mut self) -> ParseResult<Item>;
    fn parse_decorators(&mut self) -> ParseResult<Vec<Decorator>>;
    fn parse_decorator(&mut self) -> ParseResult<Decorator>;
    fn parse_function(&mut self) -> ParseResult<FunctionDecl>;
    fn parse_function_name(&mut self) -> ParseResult<(String, Option<String>, Option<String>)>;
    fn parse_param_list(&mut self) -> ParseResult<Vec<(String, Option<TypeExpr>)>>;
    fn parse_struct(&mut self) -> ParseResult<StructDecl>;
    fn parse_field_decl(&mut self) -> ParseResult<FieldDecl>;
    fn parse_enum(&mut self) -> ParseResult<EnumDecl>;
    fn parse_variant_decl(&mut self) -> ParseResult<VariantDecl>;
    fn parse_import(&mut self) -> ParseResult<ImportDecl>;
}

impl ParserItems for Parser {
    /// Parse a top-level item.
    fn parse_item(&mut self) -> ParseResult<Item> {
        // Skip decorators and collect them
        let decorators = self.parse_decorators()?;

        match self.current().kind {
            TokenKind::Fn => {
                let mut func = self.parse_function()?;
                func.decorators = decorators;
                Ok(Item::Function(func))
            }
            TokenKind::Struct => {
                let mut s = self.parse_struct()?;
                s.decorators = decorators;
                Ok(Item::Struct(s))
            }
            TokenKind::Enum => {
                // Decorators not supported on enums yet
                drop(decorators);
                Ok(Item::Enum(self.parse_enum()?))
            }
            TokenKind::Import => {
                // Decorators not supported on imports
                drop(decorators);
                Ok(Item::Import(self.parse_import()?))
            }
            _ => {
                // Treat as statement - decorators not supported
                drop(decorators);
                let stmt = self.parse_statement()?;
                Ok(Item::Statement(stmt))
            }
        }
    }

    // === Decorators ===

    fn parse_decorators(&mut self) -> ParseResult<Vec<Decorator>> {
        let mut decorators = Vec::new();

        while self.check(TokenKind::At) {
            decorators.push(self.parse_decorator()?);
        }

        Ok(decorators)
    }

    fn parse_decorator(&mut self) -> ParseResult<Decorator> {
        let start = self.current_span();
        self.expect(TokenKind::At)?;

        let name = self.expect_ident()?;
        let mut args = Vec::new();

        if self.check(TokenKind::LParen) {
            self.advance();
            while !self.check(TokenKind::RParen) && !self.is_at_end() {
                args.push(self.parse_expression()?);
                if !self.check(TokenKind::RParen) {
                    self.expect(TokenKind::Comma)?;
                }
            }
            self.expect(TokenKind::RParen)?;
        }

        let end = self.prev_span();
        Ok(Decorator::with_args(name, args, start.merge(&end)))
    }

    // === Declarations ===

    fn parse_function(&mut self) -> ParseResult<FunctionDecl> {
        let start = self.current_span();
        self.expect(TokenKind::Fn)?;

        // Check for associated type: fn TypeName.methodName
        let (name, associated_type, receiver) = self.parse_function_name()?;

        // Parameters
        self.expect(TokenKind::LParen)?;
        let params = self.parse_param_list()?;
        self.expect(TokenKind::RParen)?;

        // Return type and error type
        // Syntax forms:
        //   -> T           -- returns T, no error
        //   -> T ! E       -- returns T, may error with E
        //   -> ! E         -- returns void (no value), may error with E
        //   ! E            -- no arrow, just error (shorthand for -> ! E)
        //   (nothing)      -- returns void, no error
        let (return_type, error_type) = if self.check(TokenKind::Arrow) {
            self.advance();

            // Check for "-> ! E" (void return with error)
            if self.check(TokenKind::Bang) {
                self.advance();
                let err_type = self.parse_type_expr()?;
                (None, Some(err_type))
            } else {
                // Parse return type(s): -> A, B, ...
                let mut types = Vec::new();
                types.push(self.parse_type_expr()?);

                while self.check(TokenKind::Comma) {
                    self.advance();
                    types.push(self.parse_type_expr()?);
                }

                let ret_type = if types.len() == 1 {
                    Some(types.remove(0))
                } else {
                    let end = self.prev_span();
                    Some(TypeExpr::new(TypeExprKind::Tuple(types), start.merge(&end)))
                };

                // Check for error type after return type
                let err_type = if self.check(TokenKind::Bang) {
                    self.advance();
                    Some(self.parse_type_expr()?)
                } else {
                    None
                };

                (ret_type, err_type)
            }
        } else if self.check(TokenKind::Bang) {
            // Shorthand: ! E without arrow (same as -> ! E)
            self.advance();
            let err_type = self.parse_type_expr()?;
            (None, Some(err_type))
        } else {
            (None, None)
        };

        // Body - either block or expression function
        let (body, is_expr_fn) = if self.check(TokenKind::FatArrow) {
            self.advance();
            // Parse comma-separated expressions for tuple returns (same as parse_return)
            let mut values = Vec::new();
            values.push(self.parse_expression()?);
            while self.check(TokenKind::Comma) {
                self.advance();
                values.push(self.parse_expression()?);
            }
            let stmt = Stmt::new(StmtKind::Return(values), self.prev_span());
            (vec![stmt], true)
        } else {
            (self.parse_block()?, false)
        };

        let end = self.prev_span();
        let is_public = name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false);

        Ok(FunctionDecl {
            name,
            is_public,
            params,
            return_type,
            error_type,
            body,
            decorators: Vec::new(),
            receiver,
            associated_type,
            is_expr_fn,
            span: start.merge(&end),
        })
    }

    fn parse_function_name(&mut self) -> ParseResult<(String, Option<String>, Option<String>)> {
        let first = self.expect_ident()?;

        if self.check(TokenKind::Dot) {
            self.advance();
            let method_name = self.expect_ident()?;

            // Check if this is an instance method (has 'self' param)
            // We'll determine this later when parsing params
            Ok((method_name, Some(first.clone()), Some(first)))
        } else {
            Ok((first, None, None))
        }
    }

    fn parse_param_list(&mut self) -> ParseResult<Vec<(String, Option<TypeExpr>)>> {
        let mut params = Vec::new();

        while !self.check(TokenKind::RParen) && !self.is_at_end() {
            let name = self.expect_ident()?;

            let type_ann = if self.check(TokenKind::Colon) {
                self.advance();
                Some(self.parse_type_expr()?)
            } else {
                None
            };

            params.push((name, type_ann));

            if !self.check(TokenKind::RParen) {
                self.expect(TokenKind::Comma)?;
            }
        }

        Ok(params)
    }

    fn parse_struct(&mut self) -> ParseResult<StructDecl> {
        let start = self.current_span();
        self.expect(TokenKind::Struct)?;

        let name = self.expect_ident()?;
        self.expect(TokenKind::LBrace)?;

        let mut fields = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            fields.push(self.parse_field_decl()?);
            // Optional comma separator
            if self.check(TokenKind::Comma) {
                self.advance();
            }
        }

        self.expect(TokenKind::RBrace)?;
        let end = self.prev_span();

        let is_public = name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false);

        Ok(StructDecl {
            name,
            is_public,
            fields,
            decorators: Vec::new(),
            span: start.merge(&end),
        })
    }

    fn parse_field_decl(&mut self) -> ParseResult<FieldDecl> {
        let decorators = self.parse_decorators()?;
        let start = self.current_span();

        let name = self.expect_ident()?;

        // Optional marker
        let is_optional = if self.check(TokenKind::Question) {
            self.advance();
            true
        } else {
            false
        };

        self.expect(TokenKind::Colon)?;
        let type_expr = self.parse_type_expr()?;

        // Default value
        let default = if self.check(TokenKind::Eq) {
            self.advance();
            Some(self.parse_expression()?)
        } else {
            None
        };

        let end = self.prev_span();
        let is_public = name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false);

        Ok(FieldDecl {
            name,
            type_expr,
            is_public,
            is_optional,
            default,
            decorators,
            span: start.merge(&end),
        })
    }

    fn parse_enum(&mut self) -> ParseResult<EnumDecl> {
        let start = self.current_span();
        self.expect(TokenKind::Enum)?;

        let name = self.expect_ident()?;

        let variants = if self.check(TokenKind::Colon) {
            self.advance();
            let mut v = Vec::new();
            // At least one variant required for inline syntax? Or empty allowed?
            // Usually "enum Foo:" is rare but valid.
            if !self.check(TokenKind::Semi) && !self.is_at_end() {
                v.push(self.parse_variant_decl()?);
                while self.check(TokenKind::Comma) {
                    self.advance();
                    v.push(self.parse_variant_decl()?);
                }
            }
            // Optional semicolon at end of inline decl
            if self.check(TokenKind::Semi) {
                self.advance();
            }
            v
        } else {
            self.expect(TokenKind::LBrace)?;
            let mut v = Vec::new();
            while !self.check(TokenKind::RBrace) && !self.is_at_end() {
                v.push(self.parse_variant_decl()?);
                // Allow optional commas in block too
                if self.check(TokenKind::Comma) {
                    self.advance();
                }
            }
            self.expect(TokenKind::RBrace)?;
            v
        };

        let end = self.prev_span();
        let is_public = name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false);

        Ok(EnumDecl {
            name,
            is_public,
            variants,
            span: start.merge(&end),
        })
    }

    fn parse_variant_decl(&mut self) -> ParseResult<VariantDecl> {
        let start = self.current_span();
        let name = self.expect_ident()?;

        let payload = if self.check(TokenKind::LParen) {
            self.advance();
            let mut types = Vec::new();
            while !self.check(TokenKind::RParen) && !self.is_at_end() {
                types.push(self.parse_type_expr()?);
                if !self.check(TokenKind::RParen) {
                    self.expect(TokenKind::Comma)?;
                }
            }
            self.expect(TokenKind::RParen)?;

            if types.len() == 1 {
                Some(types.remove(0))
            } else {
                let end = self.prev_span();
                Some(TypeExpr::new(TypeExprKind::Tuple(types), start.merge(&end)))
            }
        } else {
            None
        };

        let end = self.prev_span();
        Ok(VariantDecl {
            name,
            payload,
            span: start.merge(&end),
        })
    }

    fn parse_import(&mut self) -> ParseResult<ImportDecl> {
        let start = self.current_span();
        self.expect(TokenKind::Import)?;

        // Parse path: std::io::File or std::Math::{Abs, Pow}
        let mut path = vec![self.expect_ident()?];
        while self.check(TokenKind::ColonColon) {
            // Check for wildcard `::*`
            if self.peek_next().kind == TokenKind::Star {
                self.advance(); // consume ::
                self.advance(); // consume *
                let end = self.prev_span();
                return Ok(ImportDecl {
                    path,
                    items: Vec::new(),
                    alias: None,
                    wildcard: true,
                    span: start.merge(&end),
                });
            }

            // Check for items `::{Foo, Bar}` - stop path parsing here
            if self.peek_next().kind == TokenKind::LBrace {
                self.advance(); // consume ::
                break;
            }

            self.advance();
            path.push(self.expect_ident()?);
        }

        // Check for module alias: `import std::io as io`
        let mut alias = None;
        if self.check(TokenKind::As) {
            self.advance();
            alias = Some(self.expect_ident()?);
            // Alias means we are done (treating it as module object)
            let end = self.prev_span();
            return Ok(ImportDecl {
                path,
                items: Vec::new(),
                alias,
                wildcard: false,
                span: start.merge(&end),
            });
        }

        // Parse items: { Foo, Bar as Baz }
        let mut items = Vec::new();
        if self.check(TokenKind::LBrace) {
            self.advance();
            while !self.check(TokenKind::RBrace) && !self.is_at_end() {
                if self.check(TokenKind::Star) {
                    self.advance();
                    items.push(ImportItem::Wildcard);
                } else {
                    let name = self.expect_ident()?;
                    if self.check(TokenKind::As) {
                        self.advance();
                        let alias_name = self.expect_ident()?;
                        items.push(ImportItem::Alias {
                            name,
                            alias: alias_name,
                        });
                    } else {
                        items.push(ImportItem::Symbol(name));
                    }
                }
                if !self.check(TokenKind::RBrace) {
                    self.expect(TokenKind::Comma)?;
                }
            }
            self.expect(TokenKind::RBrace)?;
        }

        let end = self.prev_span();
        Ok(ImportDecl {
            path,
            items,
            alias: None,
            wildcard: false,
            span: start.merge(&end),
        })
    }
}
