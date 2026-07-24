use super::expr::ParserExpr;
use super::items::ParserItems;
use super::types::ParserTypes;
use super::{ParseResult, Parser};
use crate::ast::*;
use crate::lexer::TokenKind;
use doo_core::{CompilerError, ErrorCode};

/// Trait for parsing statements.
pub trait ParserStmt {
    fn parse_statement(&mut self) -> ParseResult<Stmt>;
    fn parse_let(&mut self) -> ParseResult<Stmt>;
    fn parse_if(&mut self) -> ParseResult<Stmt>;
    fn parse_for(&mut self) -> ParseResult<Stmt>;
    fn parse_return(&mut self) -> ParseResult<Stmt>;
    fn parse_break(&mut self) -> ParseResult<Stmt>;
    fn parse_continue(&mut self) -> ParseResult<Stmt>;
    fn parse_print(&mut self) -> ParseResult<Stmt>;
    fn parse_block(&mut self) -> ParseResult<Vec<Stmt>>;
    fn parse_block_stmt(&mut self) -> ParseResult<Stmt>;
    fn parse_expr_or_assign(&mut self) -> ParseResult<Stmt>;
    fn expr_to_pattern(&self, expr: &Expr) -> ParseResult<Pattern>;
}

impl ParserStmt for Parser {
    // === Statements ===

    fn parse_statement(&mut self) -> ParseResult<Stmt> {
        match self.current().kind {
            TokenKind::Let => self.parse_let(),
            TokenKind::If => self.parse_if(),
            TokenKind::For => self.parse_for(),
            TokenKind::Return => self.parse_return(),
            TokenKind::Break => self.parse_break(),
            TokenKind::Continue => self.parse_continue(),
            TokenKind::Print => self.parse_print(),
            TokenKind::LBrace => self.parse_block_stmt(),
            // Local struct/enum declarations inside function bodies
            TokenKind::Struct => {
                let start = self.current_span();
                let decl = self.parse_struct()?;
                let end = self.prev_span();
                Ok(Stmt::new(StmtKind::StructDecl(decl), start.merge(end)))
            }
            TokenKind::Enum => {
                let start = self.current_span();
                let decl = self.parse_enum()?;
                let end = self.prev_span();
                Ok(Stmt::new(StmtKind::EnumDecl(decl), start.merge(end)))
            }
            _ => self.parse_expr_or_assign(),
        }
    }

    fn parse_let(&mut self) -> ParseResult<Stmt> {
        let start = self.current_span();
        self.expect(TokenKind::Let)?;

        let mutable = if self.check(TokenKind::Mut) {
            self.advance();
            true
        } else {
            false
        };

        // Parse the first pattern
        let first_pattern = self.parse_pattern()?;

        // Check for comma-separated patterns (tuple unpacking without parentheses)
        // Syntax: `let a, b, c = tuple_expr;`
        if self.check(TokenKind::Comma) {
            let mut patterns = vec![first_pattern];
            while self.check(TokenKind::Comma) {
                self.advance(); // consume ','

                // Check for ?? operator (manual error extraction)
                // Syntax: `let a, b ?? err = result_expr;`
                if self.check(TokenKind::QuestionQuestion) {
                    self.advance(); // consume '??'

                    if mutable {
                        return Err(CompilerError::new(
                            ErrorCode::InvalidAssignTarget,
                            "Manual error extraction cannot be mutable",
                            start,
                        )
                        .with_suggestion("remove `mut`: `let val ?? err = ...`"));
                    }

                    // Parse the error variable name (or _ to ignore)
                    let error_var = if self.check(TokenKind::Underscore) {
                        self.advance();
                        "_".to_string()
                    } else if self.check(TokenKind::Ident) {
                        let name = self.current().text.clone();
                        self.advance();
                        name
                    } else {
                        return Err(CompilerError::new(
                            ErrorCode::ExpectedIdentifier,
                            "Expected identifier or '_' for error binding after '??'",
                            self.current_span(),
                        )
                        .with_suggestion("use: `let val ?? err = expr` or `let val ?? _ = expr`"));
                    };

                    self.expect(TokenKind::Eq)?;
                    let expr = self.parse_expression()?;
                    let end = self.prev_span();

                    // Build ok_pattern as tuple if multiple patterns, or single if just one
                    let ok_pattern = if patterns.len() == 1 {
                        patterns.into_iter().next().unwrap()
                    } else {
                        let ok_start = patterns.first().map(|p| p.span).unwrap_or(start);
                        let ok_end = patterns.last().map(|p| p.span).unwrap_or(start);
                        Pattern::new(PatternKind::Tuple(patterns), ok_start.merge(ok_end))
                    };

                    return Ok(Stmt::new(
                        StmtKind::ManualErrorExtract {
                            expr,
                            ok_pattern,
                            error_var,
                        },
                        start.merge(end),
                    ));
                }

                // Otherwise, continue parsing tuple patterns
                patterns.push(self.parse_pattern()?);
            }

            // Build the tuple pattern
            let pattern_start = patterns.first().map(|p| p.span).unwrap_or(start);
            let pattern_end = patterns.last().map(|p| p.span).unwrap_or(start);
            let pattern = Pattern::new(
                PatternKind::Tuple(patterns),
                pattern_start.merge(pattern_end),
            );

            let type_ann = if self.check(TokenKind::Colon) {
                self.advance();
                Some(self.parse_type_expr()?)
            } else {
                None
            };

            self.expect(TokenKind::Eq)?;
            let value = self.parse_expression()?;

            let end = self.prev_span();
            return Ok(Stmt::new(
                StmtKind::Let {
                    mutable,
                    pattern,
                    type_ann,
                    value,
                },
                start.merge(end),
            ));
        }

        // Check for ?? operator (manual error extraction with single ok pattern)
        // Syntax: `let ok_pattern ?? err_var = expr;`
        if self.check(TokenKind::QuestionQuestion) {
            if mutable {
                return Err(CompilerError::new(
                    ErrorCode::InvalidAssignTarget,
                    "Manual error extraction cannot be mutable",
                    start,
                )
                .with_suggestion("remove `mut`: `let val ?? err = ...`"));
            }

            self.advance(); // consume '??'

            // Parse the error variable name (or _ to ignore)
            let error_var = if self.check(TokenKind::Underscore) {
                self.advance();
                "_".to_string()
            } else if self.check(TokenKind::Ident) {
                let name = self.current().text.clone();
                self.advance();
                name
            } else {
                return Err(CompilerError::new(
                    ErrorCode::ExpectedIdentifier,
                    "Expected identifier or '_' for error binding after '??'",
                    self.current_span(),
                )
                .with_suggestion("use: `let val ?? err = expr` or `let val ?? _ = expr`"));
            };

            self.expect(TokenKind::Eq)?;
            let expr = self.parse_expression()?;
            let end = self.prev_span();

            return Ok(Stmt::new(
                StmtKind::ManualErrorExtract {
                    expr,
                    ok_pattern: first_pattern,
                    error_var,
                },
                start.merge(end),
            ));
        }

        let pattern = first_pattern;

        let type_ann = if self.check(TokenKind::Colon) {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };

        self.expect(TokenKind::Eq)?;
        let value = self.parse_expression()?;

        let end = self.prev_span();
        Ok(Stmt::new(
            StmtKind::Let {
                mutable,
                pattern,
                type_ann,
                value,
            },
            start.merge(end),
        ))
    }

    fn parse_if(&mut self) -> ParseResult<Stmt> {
        let start = self.current_span();
        self.expect(TokenKind::If)?;

        let condition = self.parse_expression()?;
        let then_block = self.parse_block()?;

        let else_branch = if self.check(TokenKind::Else) {
            self.advance();
            if self.check(TokenKind::If) {
                Some(ElseBranch::ElseIf(Box::new(self.parse_if()?)))
            } else {
                Some(ElseBranch::Block(self.parse_block()?))
            }
        } else {
            None
        };

        let end = self.prev_span();
        Ok(Stmt::new(
            StmtKind::If {
                condition,
                then_block,
                else_branch,
            },
            start.merge(end),
        ))
    }

    fn parse_for(&mut self) -> ParseResult<Stmt> {
        let start = self.current_span();
        self.expect(TokenKind::For)?;

        // Check for infinite loop: `for { ... }` (no pattern, no iterable)
        if self.check(TokenKind::LBrace) {
            self.loop_depth += 1;
            let body = self.parse_block()?;
            self.loop_depth -= 1;
            let end = self.prev_span();
            return Ok(Stmt::new(
                StmtKind::For {
                    pattern: Pattern::wildcard(start), // Use wildcard as placeholder
                    iterable: None,
                    body,
                },
                start.merge(end),
            ));
        }

        let first_pattern = self.parse_pattern().map_err(|_| {
            CompilerError::new(
                ErrorCode::InvalidForSyntax,
                "expected loop variable after `for`",
                self.current_span(),
            )
            .with_suggestion("for item in collection { ... }")
        })?;

        let pattern = if self.check(TokenKind::Comma) {
            let mut patterns = vec![first_pattern];
            while self.check(TokenKind::Comma) {
                self.advance();
                patterns.push(self.parse_pattern()?);
            }

            let p_start = patterns.first().map(|p| p.span).unwrap_or(start);
            let p_end = patterns.last().map(|p| p.span).unwrap_or(start);
            Pattern::new(PatternKind::Tuple(patterns), p_start.merge(p_end))
        } else {
            first_pattern
        };

        let iterable = if self.check(TokenKind::In) {
            self.advance();
            Some(self.parse_expression().map_err(|_| {
                CompilerError::new(
                    ErrorCode::InvalidForSyntax,
                    "expected iterable expression after `in`",
                    self.current_span(),
                )
                .with_suggestion("for item in collection { ... }")
            })?)
        } else {
            None
        };

        self.loop_depth += 1;
        let body = self.parse_block().map_err(|_e| {
            self.loop_depth -= 1;
            CompilerError::new(
                ErrorCode::InvalidForSyntax,
                "expected `{` to start for-loop body",
                self.current_span(),
            )
            .with_suggestion("for item in collection { ... }")
        })?;
        self.loop_depth -= 1;

        let end = self.prev_span();
        Ok(Stmt::new(
            StmtKind::For {
                pattern,
                iterable,
                body,
            },
            start.merge(end),
        ))
    }

    fn parse_return(&mut self) -> ParseResult<Stmt> {
        let start = self.current_span();
        self.expect(TokenKind::Return)?;

        if self.fn_depth == 0 {
            return Err(CompilerError::new(
                ErrorCode::ReturnOutsideFunction,
                "`return` can only be used inside a function",
                start,
            )
            .with_suggestion("move this `return` inside a `fn` body"));
        }

        let mut values = Vec::new();
        if !self.is_at_stmt_end() {
            values.push(self.parse_expression()?);
            while self.check(TokenKind::Comma) {
                self.advance();
                values.push(self.parse_expression()?);
            }
        }

        let end = self.prev_span();
        Ok(Stmt::new(StmtKind::Return(values), start.merge(end)))
    }

    fn parse_break(&mut self) -> ParseResult<Stmt> {
        let span = self.current_span();
        self.expect(TokenKind::Break)?;
        if self.loop_depth == 0 {
            return Err(CompilerError::new(
                ErrorCode::BreakOutsideLoop,
                "`break` can only be used inside a loop",
                span,
            )
            .with_suggestion("move this `break` inside a `for` loop"));
        }
        Ok(Stmt::new(StmtKind::Break, span))
    }

    fn parse_continue(&mut self) -> ParseResult<Stmt> {
        let span = self.current_span();
        self.expect(TokenKind::Continue)?;
        if self.loop_depth == 0 {
            return Err(CompilerError::new(
                ErrorCode::ContinueOutsideLoop,
                "`continue` can only be used inside a loop",
                span,
            )
            .with_suggestion("move this `continue` inside a `for` loop"));
        }
        Ok(Stmt::new(StmtKind::Continue, span))
    }

    fn parse_print(&mut self) -> ParseResult<Stmt> {
        let start = self.current_span();
        self.expect(TokenKind::Print)?;
        self.expect(TokenKind::LParen)?;

        let mut exprs = Vec::new();
        while !self.check(TokenKind::RParen) && !self.is_at_end() {
            exprs.push(self.parse_expression()?);
            if !self.check(TokenKind::RParen) {
                self.expect(TokenKind::Comma)?;
            }
        }

        self.expect(TokenKind::RParen)?;
        let end = self.prev_span();
        Ok(Stmt::new(StmtKind::Print(exprs), start.merge(end)))
    }

    fn parse_block(&mut self) -> ParseResult<Vec<Stmt>> {
        self.expect(TokenKind::LBrace)?;

        let mut stmts = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            match self.parse_statement() {
                Ok(stmt) => {
                    // Enforce mandatory semicolons (Rust-like rules)
                    if stmt.kind.needs_semicolon() {
                        if self.check(TokenKind::Semi) {
                            self.advance();
                        } else {
                            self.errors.push(
                                CompilerError::new(
                                    ErrorCode::MissingSemicolon,
                                    "expected `;` after statement",
                                    self.prev_span(),
                                )
                                .with_suggestion("add `;` at the end of this statement"),
                            );
                        }
                    } else {
                        // Block-ending statements: consume optional semicolon
                        if self.check(TokenKind::Semi) {
                            self.advance();
                        }
                    }
                    stmts.push(stmt);
                }
                Err(e) => {
                    self.errors.push(e);
                    self.synchronize();
                }
            }
        }

        self.expect(TokenKind::RBrace)?;
        Ok(stmts)
    }

    fn parse_block_stmt(&mut self) -> ParseResult<Stmt> {
        let start = self.current_span();
        let stmts = self.parse_block()?;
        let end = self.prev_span();
        Ok(Stmt::new(StmtKind::Block(stmts), start.merge(end)))
    }

    fn parse_expr_or_assign(&mut self) -> ParseResult<Stmt> {
        let start = self.current_span();
        let expr = self.parse_expression()?;

        // Check for assignment
        if self.check(TokenKind::Eq) {
            self.advance();
            let value = self.parse_expression()?;
            let end = self.prev_span();

            // Convert expr to pattern
            let pattern = self.expr_to_pattern(&expr)?;
            return Ok(Stmt::new(
                StmtKind::Assign {
                    target: pattern,
                    value,
                },
                start.merge(end),
            ));
        }

        // Check for compound assignment
        if let Some(op) = CompoundOp::from_token(self.current().kind) {
            self.advance();
            let value = self.parse_expression()?;
            let end = self.prev_span();
            let pattern = self.expr_to_pattern(&expr)?;
            return Ok(Stmt::new(
                StmtKind::CompoundAssign {
                    target: pattern,
                    op,
                    value,
                },
                start.merge(end),
            ));
        }

        // Check for increment/decrement
        if let Some(op) = IncDecOp::from_token(self.current().kind) {
            self.advance();
            let end = self.prev_span();
            if let ExprKind::Ident(name) = &expr.kind {
                return Ok(Stmt::new(
                    StmtKind::IncDec {
                        variable: name.clone(),
                        op,
                    },
                    start.merge(end),
                ));
            }
        }

        let end = self.prev_span();
        Ok(Stmt::new(StmtKind::Expr(expr), start.merge(end)))
    }

    fn expr_to_pattern(&self, expr: &Expr) -> ParseResult<Pattern> {
        match &expr.kind {
            ExprKind::Ident(name) => Ok(Pattern::ident(name.clone(), expr.span)),
            ExprKind::TupleLit(items) => {
                let patterns: Result<Vec<_>, _> =
                    items.iter().map(|e| self.expr_to_pattern(e)).collect();
                Ok(Pattern::new(PatternKind::Tuple(patterns?), expr.span))
            }
            ExprKind::Index { object, index } => {
                let object_pattern = self.expr_to_pattern(object)?;
                Ok(Pattern::new(
                    PatternKind::Index {
                        object: Box::new(object_pattern),
                        index: index.clone(),
                    },
                    expr.span,
                ))
            }
            ExprKind::Field { object, field } => {
                let object_pattern = self.expr_to_pattern(object)?;
                Ok(Pattern::new(
                    PatternKind::Field {
                        object: Box::new(object_pattern),
                        field: field.clone(),
                    },
                    expr.span,
                ))
            }
            _ => Err(CompilerError::new(
                ErrorCode::InvalidPattern,
                "Cannot use this expression as assignment target",
                expr.span,
            )
            .with_suggestion("assign to a variable, field, or index")),
        }
    }
}
