use super::{ParseResult, Parser};
use crate::ast::*;
use crate::lexer::TokenKind;
use doo_core::{CompilerError, ErrorCode};
use std::collections::HashSet;

impl Parser {
    /// Parse a top-level item.
    pub fn parse_item(&mut self) -> ParseResult<Item> {
        // Skip decorators and collect them
        let decorators = self.parse_decorators()?;

        match self.current().kind {
            TokenKind::Const => {
                drop(decorators);
                Ok(Item::Const(self.parse_const()?))
            }
            TokenKind::Static => {
                drop(decorators);
                Ok(Item::Static(self.parse_static()?))
            }
            TokenKind::Fn => {
                let mut func = self.parse_function()?;
                func.decorators = decorators;
                Ok(Item::Function(func))
            }
            TokenKind::Async => {
                // async fn — consume `async`, then parse the function
                let mut func = self.parse_function()?;
                func.is_async = true;
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
            TokenKind::Interface => {
                // Decorators not supported on interfaces
                drop(decorators);
                Ok(Item::Interface(self.parse_interface()?))
            }
            TokenKind::Import => {
                // Decorators not supported on imports
                drop(decorators);
                Ok(Item::Import(self.parse_import()?))
            }
            TokenKind::Impl => {
                let mut impl_block = self.parse_impl()?;
                impl_block.decorators = decorators;
                Ok(Item::Impl(impl_block))
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

    pub(crate) fn parse_decorators(&mut self) -> ParseResult<Vec<Decorator>> {
        let mut decorators = Vec::new();

        while self.check(TokenKind::At) {
            decorators.push(self.parse_decorator()?);
        }

        Ok(decorators)
    }

    fn parse_decorator(&mut self) -> ParseResult<Decorator> {
        let start = self.current_span();
        self.expect(TokenKind::At)?;

        let name = self.expect_ident().map_err(|_| {
            CompilerError::new(
                ErrorCode::InvalidDecoratorSyntax,
                "expected decorator name after `@`",
                start,
            )
            .with_suggestion("usage: @decoratorName or @decoratorName(args)")
        })?;
        let mut args = Vec::new();

        if self.check(TokenKind::LParen) {
            self.advance();
            while !self.check(TokenKind::RParen) && !self.is_at_end() {
                args.push(self.parse_expression().map_err(|_| {
                    CompilerError::new(
                        ErrorCode::InvalidDecoratorSyntax,
                        format!("invalid argument in @{} decorator", name),
                        self.current_span(),
                    )
                })?);
                if !self.check(TokenKind::RParen) {
                    self.expect(TokenKind::Comma).map_err(|_| {
                        CompilerError::new(
                            ErrorCode::InvalidDecoratorSyntax,
                            format!("expected `,` or `)` in @{} arguments", name),
                            self.current_span(),
                        )
                    })?;
                }
            }
            self.expect(TokenKind::RParen).map_err(|_| {
                CompilerError::new(
                    ErrorCode::InvalidDecoratorSyntax,
                    format!("missing `)` in @{} decorator", name),
                    self.current_span(),
                )
            })?;
        }

        let end = self.prev_span();
        Ok(Decorator::with_args(name, args, start.merge(end)))
    }

    // === Declarations ===

    /// Parse `const Name = expr` — compile-time constant declaration.
    ///
    /// Syntax:
    ///   const MaxItems = 10
    ///   const FreePlan = "free"
    ///   const Regions = { "us": "us-west1" }
    pub(crate) fn parse_const(&mut self) -> ParseResult<ConstDecl> {
        let start = self.current_span();
        self.expect(TokenKind::Const)?;

        let name = self.expect_ident().map_err(|_| {
            CompilerError::new(
                ErrorCode::ExpectedIdentifier,
                "expected constant name after `const`",
                self.current_span(),
            )
            .with_suggestion("usage: const MaxRetries = 3  or  const AppName = \"doo\"")
        })?;

        self.expect(TokenKind::Eq).map_err(|_| {
            CompilerError::new(
                ErrorCode::UnexpectedToken,
                format!("expected `=` after const name `{}`", name),
                self.current_span(),
            )
            .with_suggestion("usage: const Name = <value>")
        })?;

        let value = self.parse_expression().map_err(|_| {
            CompilerError::new(
                ErrorCode::InvalidConstExpr,
                format!(
                    "expected a compile-time constant expression for `const {}`",
                    name
                ),
                self.current_span(),
            )
            .with_suggestion(
                "const values must be literals, arrays/maps of literals, or const arithmetic",
            )
        })?;

        let end = self.prev_span();
        Ok(ConstDecl::new(name, value, start.merge(end)))
    }

    /// Parse `static Name: Type` — runtime global variable declaration.
    ///
    /// Syntax:
    ///   static DB: Database
    ///   static Cache: Redis
    ///
    /// Rules:
    ///   - Declared at top-level only
    ///   - Type annotation is required
    ///   - Set exactly once in main(), immutable after
    ///   - PascalCase = public, camelCase = private
    pub(crate) fn parse_static(&mut self) -> ParseResult<StaticDecl> {
        let start = self.current_span();
        self.expect(TokenKind::Static)?;

        let name = self.expect_ident().map_err(|_| {
            CompilerError::new(
                ErrorCode::ExpectedIdentifier,
                "expected static variable name after `static`",
                self.current_span(),
            )
            .with_suggestion("usage: static DB: Database  or  static Cache: Redis")
        })?;

        self.expect(TokenKind::Colon).map_err(|_| {
            CompilerError::new(
                ErrorCode::UnexpectedToken,
                format!("expected `:` after static name `{}`", name),
                self.current_span(),
            )
            .with_suggestion("usage: static Name: Type  — type annotation is required")
        })?;

        let type_expr = self.parse_type_expr()?;

        let end = self.prev_span();
        Ok(StaticDecl::new(name, type_expr, start.merge(end)))
    }

    pub(crate) fn parse_function(&mut self) -> ParseResult<FunctionDecl> {
        let start = self.current_span();

        // Handle `async fn` — consume `async` if present
        let is_async = if self.check(TokenKind::Async) {
            self.advance();
            true
        } else {
            false
        };

        self.expect(TokenKind::Fn)?;

        // Check for associated type: fn TypeName.methodName
        let (name, associated_type, receiver) = self.parse_function_name()?;

        // Generic type parameters: fn name<T, U: Constraint>(...)
        let type_params = self.parse_type_params()?;

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
                    Some(TypeExpr::new(TypeExprKind::Tuple(types), start.merge(end)))
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
        self.fn_depth += 1;
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
        } else if self.check(TokenKind::LBrace) {
            (self.parse_block()?, false)
        } else {
            self.fn_depth -= 1;
            return Err(CompilerError::new(
                ErrorCode::MissingFunctionBody,
                format!("function '{}' is missing a body", name),
                self.current_span(),
            )
            .with_suggestion("add a body with `{ ... }` or `=> expr`"));
        };
        self.fn_depth -= 1;

        let end = self.prev_span();
        let is_public = name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false);

        Ok(FunctionDecl {
            name,
            is_public,
            type_params,
            params,
            return_type,
            error_type,
            body,
            decorators: Vec::new(),
            receiver,
            associated_type,
            is_expr_fn,
            is_async,
            span: start.merge(end),
        })
    }

    fn parse_function_name(&mut self) -> ParseResult<(String, Option<String>, Option<String>)> {
        let first = self.expect_ident()?;

        // Support both `.` and `::` for method definitions
        if self.check(TokenKind::Dot) || self.check(TokenKind::ColonColon) {
            self.advance();
            let method_name = self.expect_ident()?;

            // Check if this is an instance method (has 'self' param)
            // We'll determine this later when parsing params
            Ok((method_name, Some(first.clone()), Some(first)))
        } else {
            Ok((first, None, None))
        }
    }

    /// Parse generic type parameters: `<T>`, `<T: Constraint>`, `<A, B>`.
    ///
    /// Called after function/struct name. If no `<` is present, returns empty vec.
    /// Contextually unambiguous: after `fn name` or `struct Name`, `<` can only
    /// mean type parameters, never a comparison operator.
    fn parse_type_params(&mut self) -> ParseResult<Vec<TypeParam>> {
        if !self.check(TokenKind::Lt) {
            return Ok(Vec::new());
        }
        self.advance(); // consume `<`

        let mut params = Vec::new();
        let mut seen_names: HashSet<String> = HashSet::new();

        while !self.check(TokenKind::Gt) && !self.is_at_end() {
            let param_span = self.current_span();
            let name = self.expect_ident().map_err(|_| {
                CompilerError::new(
                    ErrorCode::ExpectedIdentifier,
                    "expected type parameter name (e.g. T, A, B)",
                    self.current_span(),
                )
                .with_suggestion("usage: fn name<T>(...) or struct Name<T> { ... }")
            })?;

            // Check for duplicate type parameter names
            if !seen_names.insert(name.clone()) {
                return Err(CompilerError::new(
                    ErrorCode::DuplicateParameter,
                    format!("duplicate type parameter '{}'", name),
                    param_span,
                )
                .with_suggestion(format!("rename one of the '{}' type parameters", name)));
            }

            // Optional interface constraint: <T: Displayable>
            let constraint = if self.check(TokenKind::Colon) {
                self.advance();
                Some(self.expect_ident().map_err(|_| {
                    CompilerError::new(
                        ErrorCode::ExpectedIdentifier,
                        format!(
                            "expected interface name after ':' in type parameter '{}'",
                            name
                        ),
                        self.current_span(),
                    )
                    .with_suggestion("usage: <T: SomeInterface>")
                })?)
            } else {
                None
            };

            let end = self.prev_span();
            params.push(TypeParam {
                name,
                constraint,
                span: param_span.merge(end),
            });

            if !self.check(TokenKind::Gt) {
                self.expect(TokenKind::Comma)?;
            }
        }

        self.expect(TokenKind::Gt).map_err(|_| {
            CompilerError::new(
                ErrorCode::UnexpectedToken,
                "expected `>` to close type parameter list",
                self.current_span(),
            )
            .with_suggestion("usage: fn name<T>(...) or struct Name<T, U> { ... }")
        })?;

        Ok(params)
    }

    fn parse_param_list(&mut self) -> ParseResult<Vec<(String, Option<TypeExpr>)>> {
        let mut params = Vec::new();
        let mut seen_names: HashSet<String> = HashSet::new();

        while !self.check(TokenKind::RParen) && !self.is_at_end() {
            let param_span = self.current_span();
            let name = self.expect_ident()?;

            // Check for duplicate parameter names
            if !seen_names.insert(name.clone()) {
                return Err(CompilerError::new(
                    ErrorCode::DuplicateParameter,
                    format!("duplicate parameter '{}'", name),
                    param_span,
                )
                .with_suggestion(format!("rename one of the '{}' parameters", name)));
            }

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

    pub(crate) fn parse_struct(&mut self) -> ParseResult<StructDecl> {
        let start = self.current_span();
        self.expect(TokenKind::Struct)?;

        let name = self.expect_ident()?;

        // Generic type parameters: struct Name<T, U>(...)
        let type_params = self.parse_type_params()?;

        self.expect(TokenKind::LBrace)?;

        let mut fields = Vec::new();
        let mut seen_fields: HashSet<String> = HashSet::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            let field = self.parse_field_decl()?;
            // Check for duplicate field names
            if !seen_fields.insert(field.name.clone()) {
                return Err(CompilerError::new(
                    ErrorCode::DuplicateField,
                    format!("duplicate field '{}' in struct '{}'", field.name, name),
                    field.span,
                )
                .with_suggestion(format!("rename one of the '{}' fields", field.name)));
            }
            fields.push(field);
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
            type_params,
            fields,
            decorators: Vec::new(),
            span: start.merge(end),
        })
    }

    fn parse_field_decl(&mut self) -> ParseResult<FieldDecl> {
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

        // Parse decorators AFTER type expression (e.g., `Email: Str @email`)
        let decorators = self.parse_decorators()?;

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
            span: start.merge(end),
        })
    }

    pub(crate) fn parse_impl(&mut self) -> ParseResult<ImplDecl> {
        let start = self.current_span();
        self.expect(TokenKind::Impl)?;

        let mut struct_name = self.expect_ident().map_err(|_| {
            CompilerError::new(
                ErrorCode::ExpectedIdentifier,
                "expected struct name after `impl`",
                self.current_span(),
            )
            .with_suggestion("usage: impl StructName { fn method(self) -> ... }")
        })?;

        // Support module paths: impl Module::Struct { ... }
        while self.check(TokenKind::ColonColon) {
            self.advance();
            let next = self.expect_ident()?;
            struct_name = format!("{}::{}", struct_name, next);
        }

        // Support generic impl: impl Array<T> { ... }
        // Parse and discard the type parameters for now
        if self.check(TokenKind::Lt) {
            let _ = self.parse_type_params()?;
        }

        self.expect(TokenKind::LBrace).map_err(|_| {
            CompilerError::new(
                ErrorCode::UnexpectedToken,
                format!("expected `{{` after `impl {}`", struct_name),
                self.current_span(),
            )
        })?;

        let mut methods = Vec::new();
        let mut seen_methods: HashSet<String> = HashSet::new();

        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            // Each method must start with `fn`
            let mut func = self.parse_function().map_err(|e| {
                CompilerError::new(
                    ErrorCode::UnexpectedToken,
                    format!("invalid method in impl {}: {}", struct_name, e.message),
                    e.span,
                )
            })?;

            // Set the associated type for the method
            func.associated_type = Some(struct_name.clone());

            // Verify method has 'self' as first parameter
            if func.params.is_empty() || func.params[0].0 != "self" {
                return Err(CompilerError::new(
                    ErrorCode::MissingFunctionBody,
                    format!(
                        "method '{}' in impl {} must have 'self' as its first parameter",
                        func.name, struct_name
                    ),
                    func.span,
                )
                .with_suggestion(format!("add `self` parameter: fn {}(self, ...)", func.name)));
            }

            // Check for duplicate method names
            if !seen_methods.insert(func.name.clone()) {
                return Err(CompilerError::new(
                    ErrorCode::DuplicateMethod,
                    format!("duplicate method '{}' in impl {}", func.name, struct_name),
                    func.span,
                )
                .with_suggestion(format!("rename one of the '{}' methods", func.name)));
            }

            methods.push(func);
        }

        self.expect(TokenKind::RBrace).map_err(|_| {
            CompilerError::new(
                ErrorCode::UnexpectedToken,
                format!("expected `}}` to close impl {}", struct_name),
                self.current_span(),
            )
        })?;

        let end = self.prev_span();
        Ok(ImplDecl {
            struct_name,
            methods,
            decorators: Vec::new(),
            span: start.merge(end),
        })
    }

    pub(crate) fn parse_enum(&mut self) -> ParseResult<EnumDecl> {
        let start = self.current_span();
        self.expect(TokenKind::Enum)?;

        let name = self.expect_ident()?;

        let mut seen_variants: HashSet<String> = HashSet::new();

        let variants = if self.check(TokenKind::Colon) {
            self.advance();
            let mut v = Vec::new();
            if !self.check(TokenKind::Semi) && !self.is_at_end() {
                let variant = self.parse_variant_decl()?;
                if !seen_variants.insert(variant.name.clone()) {
                    return Err(CompilerError::new(
                        ErrorCode::DuplicateVariant,
                        format!("duplicate variant '{}' in enum '{}'", variant.name, name),
                        variant.span,
                    )
                    .with_suggestion(format!("rename one of the '{}' variants", variant.name)));
                }
                v.push(variant);
                // Accept both comma `,` and pipe `|` as variant separators for inline enums
                while self.check(TokenKind::Comma) || self.check(TokenKind::Or) {
                    self.advance();
                    let variant = self.parse_variant_decl()?;
                    if !seen_variants.insert(variant.name.clone()) {
                        return Err(CompilerError::new(
                            ErrorCode::DuplicateVariant,
                            format!("duplicate variant '{}' in enum '{}'", variant.name, name),
                            variant.span,
                        )
                        .with_suggestion(format!(
                            "rename one of the '{}' variants",
                            variant.name
                        )));
                    }
                    v.push(variant);
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
                let variant = self.parse_variant_decl()?;
                if !seen_variants.insert(variant.name.clone()) {
                    return Err(CompilerError::new(
                        ErrorCode::DuplicateVariant,
                        format!("duplicate variant '{}' in enum '{}'", variant.name, name),
                        variant.span,
                    )
                    .with_suggestion(format!("rename one of the '{}' variants", variant.name)));
                }
                v.push(variant);
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
            span: start.merge(end),
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
                Some(TypeExpr::new(TypeExprKind::Tuple(types), start.merge(end)))
            }
        } else {
            None
        };

        // Parse decorators on variants (e.g. @inherits(User))
        let decorators = self.parse_decorators()?;

        let end = self.prev_span();
        Ok(VariantDecl {
            name,
            payload,
            decorators,
            span: start.merge(end),
        })
    }

    pub(crate) fn parse_interface(&mut self) -> ParseResult<InterfaceDecl> {
        let start = self.current_span();
        self.expect(TokenKind::Interface)?;

        let name = self.expect_ident()?;
        self.expect(TokenKind::LBrace)?;

        let mut methods = Vec::new();
        let mut seen_methods: HashSet<String> = HashSet::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            let method = self.parse_interface_method()?;
            if !seen_methods.insert(method.name.clone()) {
                return Err(CompilerError::new(
                    ErrorCode::DuplicateMethod,
                    format!("duplicate method '{}' in interface '{}'", method.name, name),
                    method.span,
                )
                .with_suggestion(format!("rename one of the '{}' methods", method.name)));
            }
            methods.push(method);
        }

        self.expect(TokenKind::RBrace)?;
        let end = self.prev_span();

        let is_public = name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false);

        Ok(InterfaceDecl {
            name,
            is_public,
            methods,
            span: start.merge(end),
        })
    }

    fn parse_interface_method(&mut self) -> ParseResult<InterfaceMethodDecl> {
        let start = self.current_span();
        self.expect(TokenKind::Fn)?;

        let name = self.expect_ident()?;

        // Parameters
        self.expect(TokenKind::LParen)?;
        let params = self.parse_param_list()?;
        self.expect(TokenKind::RParen)?;

        // Return type and error type (same as function parsing)
        let (return_type, error_type) = if self.check(TokenKind::Arrow) {
            self.advance();
            if self.check(TokenKind::Bang) {
                self.advance();
                let err_type = self.parse_type_expr()?;
                (None, Some(err_type))
            } else {
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
                    Some(TypeExpr::new(TypeExprKind::Tuple(types), start.merge(end)))
                };
                let err_type = if self.check(TokenKind::Bang) {
                    self.advance();
                    Some(self.parse_type_expr()?)
                } else {
                    None
                };
                (ret_type, err_type)
            }
        } else if self.check(TokenKind::Bang) {
            self.advance();
            let err_type = self.parse_type_expr()?;
            (None, Some(err_type))
        } else {
            (None, None)
        };

        // Optional semicolon terminator for interface methods
        if self.check(TokenKind::Semi) {
            self.advance();
        }

        let end = self.prev_span();
        Ok(InterfaceMethodDecl {
            name,
            params,
            return_type,
            error_type,
            span: start.merge(end),
        })
    }

    pub(crate) fn parse_import(&mut self) -> ParseResult<ImportDecl> {
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
                    span: start.merge(end),
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
        if self.check(TokenKind::As) {
            self.advance();
            let alias = Some(self.expect_ident()?);
            // Alias means we are done (treating it as module object)
            let end = self.prev_span();
            return Ok(ImportDecl {
                path,
                items: Vec::new(),
                alias,
                wildcard: false,
                span: start.merge(end),
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
            span: start.merge(end),
        })
    }
}
