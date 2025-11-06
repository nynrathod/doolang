use crate::codegen::core::CodeGen;
use crate::parser::ast::AstNode;
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::{BasicValueEnum, FunctionValue};

impl<'ctx> CodeGen<'ctx> {
    /// Generate LLVM IR for a closure
    /// Creates an actual LLVM function that can be called
    pub fn generate_closure(
        &mut self,
        name: &str,
        params: &[String],
        param_types: &[Option<String>],
        body_expr: &str,
        body_ast: &Option<Box<AstNode>>,
        _return_type: &Option<String>,
        _captures: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        // Generate a unique function name for this closure
        let closure_fn_name = format!("closure_{}", name);

        // Determine parameter types (default to i32 for Int)
        let mut param_llvm_types: Vec<BasicMetadataTypeEnum> = Vec::new();
        for _ in params {
            param_llvm_types.push(self.context.i32_type().into());
        }

        // Create function type (returns i32/bool for now)
        let fn_type = self.context.i32_type().fn_type(&param_llvm_types, false);

        // Create the function
        let function = self.module.add_function(&closure_fn_name, fn_type, None);

        // Save current insert block
        let saved_block = self.builder.get_insert_block();

        // Create entry block for closure function
        let entry_block = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry_block);

        // Store old parameter mappings
        let old_params: Vec<(String, BasicValueEnum)> = params
            .iter()
            .filter_map(|p| self.temp_values.get(p).map(|v| (p.clone(), *v)))
            .collect();

        // Map function parameters to the closure parameter names
        for (i, param_name) in params.iter().enumerate() {
            if let Some(param_value) = function.get_nth_param(i as u32) {
                self.temp_values.insert(param_name.clone(), param_value);
            }
        }

        // Generate the body expression from AST by directly evaluating it
        let result_value = if let Some(ast_body) = body_ast {
            // Directly evaluate the AST body in the current codegen context
            match ast_body.as_ref() {
                AstNode::Block(statements) => {
                    // Process all statements in the block
                    let mut last_result: Option<BasicValueEnum> = None;
                    for stmt in statements {
                        match stmt {
                            AstNode::Return { values } => {
                                if !values.is_empty() {
                                    last_result = self.eval_ast_expr(&values[0]);
                                }
                            }
                            AstNode::LetDecl { pattern, value, .. } => {
                                // Evaluate the value and store it
                                if let Some(val) = self.eval_ast_expr(value) {
                                    if let crate::parser::ast::Pattern::Identifier(name) = pattern {
                                        self.temp_values.insert(name.clone(), val);
                                    }
                                }
                            }
                            _ => {
                                // Try to evaluate other statements
                                self.eval_ast_expr(stmt);
                            }
                        }
                    }
                    last_result
                        .unwrap_or_else(|| self.context.i32_type().const_int(0, false).into())
                }
                _ => {
                    // Single expression - evaluate it directly
                    self.eval_ast_expr(ast_body)
                        .unwrap_or_else(|| self.context.i32_type().const_int(0, false).into())
                }
            }
        } else {
            // Fallback: use pre-evaluated body_expr
            if let Some(val) = self.temp_values.get(body_expr) {
                *val
            } else {
                self.context.i32_type().const_int(0, false).into()
            }
        };

        // Return the result
        if result_value.is_int_value() {
            let int_val = result_value.into_int_value();
            self.builder.build_return(Some(&int_val)).unwrap();
        } else {
            // Default return if not an int
            self.builder
                .build_return(Some(&self.context.i32_type().const_int(0, false)))
                .unwrap();
        }

        // Restore parameter mappings
        for (param_name, _) in old_params {
            self.temp_values.remove(&param_name);
        }

        // Restore insert position
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }

        // Store the function reference so filter/map/reduce can find it
        let closure_key = format!("closure_{}", name);
        self.temp_strings
            .insert(format!("{}_fn_name", closure_key), closure_fn_name.clone());
        self.temp_strings
            .insert(format!("{}_params", closure_key), params.join(","));

        // Store function value for later retrieval
        let fn_ptr = function.as_global_value().as_pointer_value();
        self.temp_values.insert(name.to_string(), fn_ptr.into());

        Some(fn_ptr.into())
    }

    /// Check if a value is a closure
    pub fn is_closure(&self, value_name: &str) -> bool {
        let closure_key = format!("closure_{}_fn_name", value_name);
        self.temp_strings.contains_key(&closure_key)
    }

    /// Get the LLVM function for a closure
    pub fn get_closure_function(&self, name: &str) -> Option<FunctionValue<'ctx>> {
        let closure_key = format!("closure_{}_fn_name", name);
        if let Some(fn_name) = self.temp_strings.get(&closure_key) {
            self.module.get_function(fn_name)
        } else {
            None
        }
    }

    /// Get closure parameter count
    pub fn get_closure_param_count(&self, name: &str) -> usize {
        let closure_key = format!("closure_{}_params", name);
        self.temp_strings
            .get(&closure_key)
            .map(|s| {
                if s.is_empty() {
                    0
                } else {
                    s.split(',').count()
                }
            })
            .unwrap_or(0)
    }

    /// Execute a closure with one argument and return the result
    pub fn call_closure_with_one_arg(
        &mut self,
        closure_name: &str,
        arg: BasicValueEnum<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        if let Some(closure_fn) = self.get_closure_function(closure_name) {
            let arg_int = if arg.is_int_value() {
                arg.into_int_value()
            } else {
                return None;
            };

            let call_result = self
                .builder
                .build_call(closure_fn, &[arg_int.into()], "closure_call")
                .unwrap();

            call_result.try_as_basic_value().left()
        } else {
            None
        }
    }

    /// Execute a closure with two arguments and return the result
    pub fn call_closure_with_two_args(
        &mut self,
        closure_name: &str,
        arg1: BasicValueEnum<'ctx>,
        arg2: BasicValueEnum<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        if let Some(closure_fn) = self.get_closure_function(closure_name) {
            let arg1_int = if arg1.is_int_value() {
                arg1.into_int_value()
            } else {
                return None;
            };

            let arg2_int = if arg2.is_int_value() {
                arg2.into_int_value()
            } else {
                return None;
            };

            let call_result = self
                .builder
                .build_call(
                    closure_fn,
                    &[arg1_int.into(), arg2_int.into()],
                    "closure_call_2",
                )
                .unwrap();

            call_result.try_as_basic_value().left()
        } else {
            None
        }
    }

    /// Evaluate an AST expression node directly in the current codegen context
    /// Used for closure body evaluation
    fn eval_ast_expr(&mut self, node: &AstNode) -> Option<BasicValueEnum<'ctx>> {
        match node {
            AstNode::NumberLiteral(n) => {
                Some(self.context.i32_type().const_int(*n as u64, false).into())
            }
            AstNode::Identifier(name) => self.temp_values.get(name).copied(),
            AstNode::BinaryExpr { left, op, right } => {
                let left_val = self.eval_ast_expr(left)?;
                let right_val = self.eval_ast_expr(right)?;

                if !left_val.is_int_value() || !right_val.is_int_value() {
                    return None;
                }

                let left_int = left_val.into_int_value();
                let right_int = right_val.into_int_value();

                use crate::lexar::token::TokenType;
                match op {
                    TokenType::Plus => Some(
                        self.builder
                            .build_int_add(left_int, right_int, "add")
                            .unwrap()
                            .into(),
                    ),
                    TokenType::Minus => Some(
                        self.builder
                            .build_int_sub(left_int, right_int, "sub")
                            .unwrap()
                            .into(),
                    ),
                    TokenType::Star => Some(
                        self.builder
                            .build_int_mul(left_int, right_int, "mul")
                            .unwrap()
                            .into(),
                    ),
                    TokenType::Slash => Some(
                        self.builder
                            .build_int_signed_div(left_int, right_int, "div")
                            .unwrap()
                            .into(),
                    ),
                    TokenType::Percent => Some(
                        self.builder
                            .build_int_signed_rem(left_int, right_int, "rem")
                            .unwrap()
                            .into(),
                    ),
                    TokenType::Gt => {
                        let cmp = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::SGT,
                                left_int,
                                right_int,
                                "gt",
                            )
                            .unwrap();
                        Some(
                            self.builder
                                .build_int_cast(cmp, self.context.i32_type(), "gt_cast")
                                .unwrap()
                                .into(),
                        )
                    }
                    TokenType::Lt => {
                        let cmp = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::SLT,
                                left_int,
                                right_int,
                                "lt",
                            )
                            .unwrap();
                        Some(
                            self.builder
                                .build_int_cast(cmp, self.context.i32_type(), "lt_cast")
                                .unwrap()
                                .into(),
                        )
                    }
                    TokenType::GtEq => {
                        let cmp = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::SGE,
                                left_int,
                                right_int,
                                "gte",
                            )
                            .unwrap();
                        Some(
                            self.builder
                                .build_int_cast(cmp, self.context.i32_type(), "gte_cast")
                                .unwrap()
                                .into(),
                        )
                    }
                    TokenType::LtEq => {
                        let cmp = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::SLE,
                                left_int,
                                right_int,
                                "lte",
                            )
                            .unwrap();
                        Some(
                            self.builder
                                .build_int_cast(cmp, self.context.i32_type(), "lte_cast")
                                .unwrap()
                                .into(),
                        )
                    }
                    TokenType::EqEq => {
                        let cmp = self
                            .builder
                            .build_int_compare(inkwell::IntPredicate::EQ, left_int, right_int, "eq")
                            .unwrap();
                        Some(
                            self.builder
                                .build_int_cast(cmp, self.context.i32_type(), "eq_cast")
                                .unwrap()
                                .into(),
                        )
                    }
                    TokenType::NotEq => {
                        let cmp = self
                            .builder
                            .build_int_compare(inkwell::IntPredicate::NE, left_int, right_int, "ne")
                            .unwrap();
                        Some(
                            self.builder
                                .build_int_cast(cmp, self.context.i32_type(), "ne_cast")
                                .unwrap()
                                .into(),
                        )
                    }
                    TokenType::AndAnd => {
                        // Logical AND: both must be non-zero
                        let left_bool = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::NE,
                                left_int,
                                self.context.i32_type().const_int(0, false),
                                "left_bool",
                            )
                            .unwrap();
                        let right_bool = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::NE,
                                right_int,
                                self.context.i32_type().const_int(0, false),
                                "right_bool",
                            )
                            .unwrap();
                        let and_result = self
                            .builder
                            .build_and(left_bool, right_bool, "and")
                            .unwrap();
                        Some(
                            self.builder
                                .build_int_cast(and_result, self.context.i32_type(), "and_cast")
                                .unwrap()
                                .into(),
                        )
                    }
                    TokenType::OrOr => {
                        // Logical OR: at least one must be non-zero
                        let left_bool = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::NE,
                                left_int,
                                self.context.i32_type().const_int(0, false),
                                "left_bool",
                            )
                            .unwrap();
                        let right_bool = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::NE,
                                right_int,
                                self.context.i32_type().const_int(0, false),
                                "right_bool",
                            )
                            .unwrap();
                        let or_result = self.builder.build_or(left_bool, right_bool, "or").unwrap();
                        Some(
                            self.builder
                                .build_int_cast(or_result, self.context.i32_type(), "or_cast")
                                .unwrap()
                                .into(),
                        )
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
}
