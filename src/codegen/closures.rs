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
        _param_types: &[Option<String>],
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

        // Store closure body for on-demand string closure generation
        if let Some(ast_body) = body_ast {
            self.closure_bodies
                .insert(name.to_string(), (params.to_vec(), ast_body.clone()));
        }

        // Store function value for later retrieval
        let fn_ptr = function.as_global_value().as_pointer_value();
        self.temp_values.insert(name.to_string(), fn_ptr.into());

        Some(fn_ptr.into())
    }

    /// Generate LLVM IR for a string closure (takes and returns pointers)
    /// Used for string array map/filter operations
    pub fn generate_string_closure(
        &mut self,
        name: &str,
        params: &[String],
        body_ast: &Option<Box<AstNode>>,
    ) -> Option<BasicValueEnum<'ctx>> {
        // Generate a unique function name for this string closure
        let closure_fn_name = format!("str_closure_{}", name);

        // String closures take pointer parameters and return pointer
        let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
        let mut param_llvm_types: Vec<BasicMetadataTypeEnum> = Vec::new();
        for _ in params {
            param_llvm_types.push(ptr_type.into());
        }

        // Create function type (returns ptr for strings)
        let fn_type = ptr_type.fn_type(&param_llvm_types, false);

        // Create the function
        let function = self.module.add_function(&closure_fn_name, fn_type, None);

        // Save current insert block
        let saved_block = self.builder.get_insert_block();

        // Create entry block for closure function
        let entry_block = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry_block);

        // Store old parameter mappings and heap_strings state
        let old_params: Vec<(String, BasicValueEnum)> = params
            .iter()
            .filter_map(|p| self.temp_values.get(p).map(|v| (p.clone(), *v)))
            .collect();

        // Map function parameters to the closure parameter names
        // Mark them as heap strings so string operations work
        for (i, param_name) in params.iter().enumerate() {
            if let Some(param_value) = function.get_nth_param(i as u32) {
                self.temp_values.insert(param_name.clone(), param_value);
                self.heap_strings.insert(param_name.clone());
            }
        }

        // Generate the body expression from AST
        let result_value = if let Some(ast_body) = body_ast {
            match ast_body.as_ref() {
                AstNode::Block(statements) => {
                    let mut last_result: Option<BasicValueEnum> = None;
                    for stmt in statements {
                        match stmt {
                            AstNode::Return { values } => {
                                if !values.is_empty() {
                                    last_result = self.eval_string_ast_expr(&values[0]);
                                }
                            }
                            AstNode::LetDecl { pattern, value, .. } => {
                                if let Some(val) = self.eval_string_ast_expr(value) {
                                    if let crate::parser::ast::Pattern::Identifier(name) = pattern {
                                        self.temp_values.insert(name.clone(), val);
                                        if val.is_pointer_value() {
                                            self.heap_strings.insert(name.clone());
                                        }
                                    }
                                }
                            }
                            _ => {
                                self.eval_string_ast_expr(stmt);
                            }
                        }
                    }
                    last_result
                }
                _ => self.eval_string_ast_expr(ast_body),
            }
        } else {
            None
        };

        // Return the result (pointer for strings)
        if let Some(result) = result_value {
            if result.is_pointer_value() {
                self.builder
                    .build_return(Some(&result.into_pointer_value()))
                    .unwrap();
            } else {
                // Return null pointer if result is not a pointer
                self.builder
                    .build_return(Some(&ptr_type.const_null()))
                    .unwrap();
            }
        } else {
            self.builder
                .build_return(Some(&ptr_type.const_null()))
                .unwrap();
        }

        // Restore parameter mappings
        for (param_name, _) in &old_params {
            self.temp_values.remove(param_name);
            self.heap_strings.remove(param_name);
        }

        // Restore insert position
        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }

        // Store the function reference
        let closure_key = format!("str_closure_{}", name);
        self.temp_strings
            .insert(format!("{}_fn_name", closure_key), closure_fn_name.clone());
        self.temp_strings
            .insert(format!("{}_params", closure_key), params.join(","));
        self.temp_strings
            .insert(format!("{}_is_string", closure_key), "true".to_string());

        // Store function value for later retrieval
        let fn_ptr = function.as_global_value().as_pointer_value();
        self.temp_values.insert(name.to_string(), fn_ptr.into());

        Some(fn_ptr.into())
    }

    /// Generate LLVM IR for a string filter closure (takes pointer, returns bool/i32)
    pub fn generate_string_filter_closure(
        &mut self,
        name: &str,
        params: &[String],
        body_ast: &Option<Box<AstNode>>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let closure_fn_name = format!("str_filter_closure_{}", name);

        let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
        let mut param_llvm_types: Vec<BasicMetadataTypeEnum> = Vec::new();
        for _ in params {
            param_llvm_types.push(ptr_type.into());
        }

        // Filter closures return i32 (bool)
        let fn_type = self.context.i32_type().fn_type(&param_llvm_types, false);
        let function = self.module.add_function(&closure_fn_name, fn_type, None);

        let saved_block = self.builder.get_insert_block();
        let entry_block = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry_block);

        let old_params: Vec<(String, BasicValueEnum)> = params
            .iter()
            .filter_map(|p| self.temp_values.get(p).map(|v| (p.clone(), *v)))
            .collect();

        for (i, param_name) in params.iter().enumerate() {
            if let Some(param_value) = function.get_nth_param(i as u32) {
                self.temp_values.insert(param_name.clone(), param_value);
                self.heap_strings.insert(param_name.clone());
            }
        }

        let result_value = if let Some(ast_body) = body_ast {
            match ast_body.as_ref() {
                AstNode::Block(statements) => {
                    let mut last_result: Option<BasicValueEnum> = None;
                    for stmt in statements {
                        match stmt {
                            AstNode::Return { values } => {
                                if !values.is_empty() {
                                    last_result = self.eval_string_filter_ast_expr(&values[0]);
                                }
                            }
                            AstNode::LetDecl { pattern, value, .. } => {
                                if let Some(val) = self.eval_string_filter_ast_expr(value) {
                                    if let crate::parser::ast::Pattern::Identifier(name) = pattern {
                                        self.temp_values.insert(name.clone(), val);
                                    }
                                }
                            }
                            _ => {
                                self.eval_string_filter_ast_expr(stmt);
                            }
                        }
                    }
                    last_result
                }
                _ => self.eval_string_filter_ast_expr(ast_body),
            }
        } else {
            None
        };

        if let Some(result) = result_value {
            if result.is_int_value() {
                self.builder
                    .build_return(Some(&result.into_int_value()))
                    .unwrap();
            } else {
                self.builder
                    .build_return(Some(&self.context.i32_type().const_int(0, false)))
                    .unwrap();
            }
        } else {
            self.builder
                .build_return(Some(&self.context.i32_type().const_int(0, false)))
                .unwrap();
        }

        for (param_name, _) in &old_params {
            self.temp_values.remove(param_name);
            self.heap_strings.remove(param_name);
        }

        if let Some(block) = saved_block {
            self.builder.position_at_end(block);
        }

        let closure_key = format!("str_filter_closure_{}", name);
        self.temp_strings
            .insert(format!("{}_fn_name", closure_key), closure_fn_name.clone());
        self.temp_strings
            .insert(format!("{}_params", closure_key), params.join(","));
        self.temp_strings
            .insert(format!("{}_is_filter", closure_key), "true".to_string());

        let fn_ptr = function.as_global_value().as_pointer_value();
        self.temp_values
            .insert(format!("{}_filter", name), fn_ptr.into());

        Some(fn_ptr.into())
    }

    /// Check if a value is a closure
    pub fn is_closure(&self, value_name: &str) -> bool {
        let closure_key = format!("closure_{}_fn_name", value_name);
        let str_closure_key = format!("str_closure_{}_fn_name", value_name);
        let str_filter_key = format!("str_filter_closure_{}_fn_name", value_name);
        self.temp_strings.contains_key(&closure_key)
            || self.temp_strings.contains_key(&str_closure_key)
            || self.temp_strings.contains_key(&str_filter_key)
    }

    /// Check if a closure is a string closure
    pub fn is_string_closure(&self, value_name: &str) -> bool {
        let str_closure_key = format!("str_closure_{}_is_string", value_name);
        self.temp_strings.contains_key(&str_closure_key)
    }

    /// Check if a closure is a string filter closure
    pub fn is_string_filter_closure(&self, value_name: &str) -> bool {
        let str_filter_key = format!("str_filter_closure_{}_is_filter", value_name);
        self.temp_strings.contains_key(&str_filter_key)
    }

    /// Get the LLVM function for a closure
    pub fn get_closure_function(&self, name: &str) -> Option<FunctionValue<'ctx>> {
        let closure_key = format!("closure_{}_fn_name", name);
        if let Some(fn_name) = self.temp_strings.get(&closure_key) {
            return self.module.get_function(fn_name);
        }
        None
    }

    /// Get the LLVM function for a string closure
    pub fn get_string_closure_function(&self, name: &str) -> Option<FunctionValue<'ctx>> {
        let str_closure_key = format!("str_closure_{}_fn_name", name);
        if let Some(fn_name) = self.temp_strings.get(&str_closure_key) {
            return self.module.get_function(fn_name);
        }
        None
    }

    /// Get the LLVM function for a string filter closure
    pub fn get_string_filter_closure_function(&self, name: &str) -> Option<FunctionValue<'ctx>> {
        let str_filter_key = format!("str_filter_closure_{}_fn_name", name);
        if let Some(fn_name) = self.temp_strings.get(&str_filter_key) {
            return self.module.get_function(fn_name);
        }
        None
    }

    /// Get closure parameter count
    pub fn get_closure_param_count(&self, name: &str) -> usize {
        let closure_key = format!("closure_{}_params", name);
        if let Some(s) = self.temp_strings.get(&closure_key) {
            if s.is_empty() {
                return 0;
            }
            return s.split(',').count();
        }
        let str_closure_key = format!("str_closure_{}_params", name);
        if let Some(s) = self.temp_strings.get(&str_closure_key) {
            if s.is_empty() {
                return 0;
            }
            return s.split(',').count();
        }
        0
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

    /// Execute a string closure with one pointer argument and return pointer result
    pub fn call_string_closure_with_one_arg(
        &mut self,
        closure_name: &str,
        arg: BasicValueEnum<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        if let Some(closure_fn) = self.get_string_closure_function(closure_name) {
            let arg_ptr = if arg.is_pointer_value() {
                arg.into_pointer_value()
            } else {
                return None;
            };

            let call_result = self
                .builder
                .build_call(closure_fn, &[arg_ptr.into()], "str_closure_call")
                .unwrap();

            call_result.try_as_basic_value().left()
        } else {
            None
        }
    }

    /// Execute a string filter closure with one pointer argument and return i32 result
    pub fn call_string_filter_closure_with_one_arg(
        &mut self,
        closure_name: &str,
        arg: BasicValueEnum<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        if let Some(closure_fn) = self.get_string_filter_closure_function(closure_name) {
            let arg_ptr = if arg.is_pointer_value() {
                arg.into_pointer_value()
            } else {
                return None;
            };

            let call_result = self
                .builder
                .build_call(closure_fn, &[arg_ptr.into()], "str_filter_closure_call")
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

                use crate::lexer::token::TokenType;
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
            AstNode::UnaryExpr { op, expr } => {
                use crate::lexer::token::TokenType;
                let val = self.eval_ast_expr(expr)?;

                match op {
                    TokenType::Bang => {
                        // Logical NOT: !value
                        if val.is_int_value() {
                            let int_val = val.into_int_value();
                            // Compare to 0: if value == 0 then result is 1, else 0
                            let is_zero = self
                                .builder
                                .build_int_compare(
                                    inkwell::IntPredicate::EQ,
                                    int_val,
                                    int_val.get_type().const_int(0, false),
                                    "is_zero",
                                )
                                .unwrap();
                            Some(
                                self.builder
                                    .build_int_z_extend(
                                        is_zero,
                                        self.context.i32_type(),
                                        "not_result",
                                    )
                                    .unwrap()
                                    .into(),
                            )
                        } else {
                            None
                        }
                    }
                    TokenType::Minus => {
                        // Unary minus: -value
                        if val.is_int_value() {
                            let int_val = val.into_int_value();
                            Some(self.builder.build_int_neg(int_val, "neg").unwrap().into())
                        } else if val.is_float_value() {
                            let float_val = val.into_float_value();
                            Some(
                                self.builder
                                    .build_float_neg(float_val, "fneg")
                                    .unwrap()
                                    .into(),
                            )
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
            AstNode::BoolLiteral(b) => {
                let val = if *b { 1 } else { 0 };
                Some(self.context.i32_type().const_int(val, false).into())
            }
            AstNode::ConditionalExpr {
                condition,
                then_expr,
                else_expr,
            } => {
                // Evaluate the condition
                let cond_val = self.eval_ast_expr(condition)?;
                if !cond_val.is_int_value() {
                    return None;
                }
                let cond_int = cond_val.into_int_value();

                // Build conditional branches
                let current_fn = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();

                let then_bb = self
                    .context
                    .append_basic_block(current_fn, "if_then_closure");
                let else_bb = self
                    .context
                    .append_basic_block(current_fn, "if_else_closure");
                let merge_bb = self
                    .context
                    .append_basic_block(current_fn, "if_merge_closure");

                // Convert condition to i1 for branch
                let cond_bool = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        cond_int,
                        cond_int.get_type().const_int(0, false),
                        "cond_bool",
                    )
                    .unwrap();

                self.builder
                    .build_conditional_branch(cond_bool, then_bb, else_bb)
                    .unwrap();

                // Generate then block
                self.builder.position_at_end(then_bb);
                let then_val = self
                    .eval_ast_expr(then_expr)
                    .unwrap_or_else(|| self.context.i32_type().const_int(0, false).into());
                let then_val_int = if then_val.is_int_value() {
                    then_val.into_int_value()
                } else {
                    self.context.i32_type().const_int(0, false)
                };
                let then_end_bb = self.builder.get_insert_block().unwrap();
                self.builder.build_unconditional_branch(merge_bb).unwrap();

                // Generate else block
                self.builder.position_at_end(else_bb);
                let else_val = self
                    .eval_ast_expr(else_expr)
                    .unwrap_or_else(|| self.context.i32_type().const_int(0, false).into());
                let else_val_int = if else_val.is_int_value() {
                    else_val.into_int_value()
                } else {
                    self.context.i32_type().const_int(0, false)
                };
                let else_end_bb = self.builder.get_insert_block().unwrap();
                self.builder.build_unconditional_branch(merge_bb).unwrap();

                // Generate merge block with phi
                self.builder.position_at_end(merge_bb);
                let phi = self
                    .builder
                    .build_phi(self.context.i32_type(), "if_result")
                    .unwrap();
                phi.add_incoming(&[(&then_val_int, then_end_bb), (&else_val_int, else_end_bb)]);

                Some(phi.as_basic_value())
            }
            AstNode::BlockExpr { statements, result } => {
                // Process block statements first
                for stmt in statements {
                    self.eval_ast_expr(stmt);
                }
                // Then evaluate and return the result expression
                self.eval_ast_expr(result)
            }
            AstNode::Block(statements) => {
                // Process block statements and return last expression value
                let mut last_val: Option<BasicValueEnum<'ctx>> = None;
                for stmt in statements {
                    match stmt {
                        AstNode::Return { values } => {
                            if !values.is_empty() {
                                return self.eval_ast_expr(&values[0]);
                            }
                        }
                        _ => {
                            last_val = self.eval_ast_expr(stmt);
                        }
                    }
                }
                last_val
            }
            _ => None,
        }
    }

    /// Evaluate an AST expression for string closures (returns string/pointer)
    fn eval_string_ast_expr(&mut self, node: &AstNode) -> Option<BasicValueEnum<'ctx>> {
        match node {
            AstNode::StringLiteral(s) => {
                // Create a heap-allocated string from literal
                Some(self.create_heap_string(s).into())
            }
            AstNode::Identifier(name) => self.temp_values.get(name).copied(),
            AstNode::BinaryExpr { left, op, right } => {
                use crate::lexer::token::TokenType;
                if *op == TokenType::Plus {
                    // String concatenation
                    let left_val = self.eval_string_ast_expr(left)?;
                    let right_val = self.eval_string_ast_expr(right)?;

                    if left_val.is_pointer_value() && right_val.is_pointer_value() {
                        let left_ptr = left_val.into_pointer_value();
                        let right_ptr = right_val.into_pointer_value();
                        Some(self.concat_strings(left_ptr, right_ptr).into())
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            AstNode::Block(statements) => {
                let mut last_val: Option<BasicValueEnum<'ctx>> = None;
                for stmt in statements {
                    match stmt {
                        AstNode::Return { values } => {
                            if !values.is_empty() {
                                return self.eval_string_ast_expr(&values[0]);
                            }
                        }
                        AstNode::LetDecl { pattern, value, .. } => {
                            if let Some(val) = self.eval_string_ast_expr(value) {
                                if let crate::parser::ast::Pattern::Identifier(name) = pattern {
                                    self.temp_values.insert(name.clone(), val);
                                    if val.is_pointer_value() {
                                        self.heap_strings.insert(name.clone());
                                    }
                                }
                            }
                        }
                        _ => {
                            last_val = self.eval_string_ast_expr(stmt);
                        }
                    }
                }
                last_val
            }
            _ => None,
        }
    }

    /// Evaluate an AST expression for string filter closures (returns bool/i32)
    fn eval_string_filter_ast_expr(&mut self, node: &AstNode) -> Option<BasicValueEnum<'ctx>> {
        match node {
            AstNode::NumberLiteral(n) => {
                Some(self.context.i32_type().const_int(*n as u64, false).into())
            }
            AstNode::BoolLiteral(b) => {
                let val = if *b { 1 } else { 0 };
                Some(self.context.i32_type().const_int(val, false).into())
            }
            AstNode::Identifier(name) => self.temp_values.get(name).copied(),
            AstNode::BinaryExpr { left, op, right } => {
                use crate::lexer::token::TokenType;
                match op {
                    TokenType::Gt | TokenType::Lt | TokenType::GtEq | TokenType::LtEq => {
                        // Handle comparisons like s.len() > 5
                        let left_val = self.eval_string_filter_ast_expr(left)?;
                        let right_val = self.eval_string_filter_ast_expr(right)?;

                        if left_val.is_int_value() && right_val.is_int_value() {
                            let left_int = left_val.into_int_value();
                            let right_int = right_val.into_int_value();

                            let pred = match op {
                                TokenType::Gt => inkwell::IntPredicate::SGT,
                                TokenType::Lt => inkwell::IntPredicate::SLT,
                                TokenType::GtEq => inkwell::IntPredicate::SGE,
                                TokenType::LtEq => inkwell::IntPredicate::SLE,
                                _ => return None,
                            };

                            let cmp = self
                                .builder
                                .build_int_compare(pred, left_int, right_int, "cmp")
                                .unwrap();
                            Some(
                                self.builder
                                    .build_int_z_extend(cmp, self.context.i32_type(), "cmp_ext")
                                    .unwrap()
                                    .into(),
                            )
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
            AstNode::MethodCall {
                object,
                method,
                args,
            } => {
                // Handle string methods like s.len(), s.startsWith("x")
                let obj_val = self
                    .temp_values
                    .get(Self::extract_identifier(object)?)?
                    .clone();

                if obj_val.is_pointer_value() {
                    let str_ptr = obj_val.into_pointer_value();

                    match method.as_str() {
                        "len" => {
                            // Get string length
                            let len = self.get_string_length(str_ptr);
                            Some(len.into())
                        }
                        "startsWith" => {
                            if !args.is_empty() {
                                if let AstNode::StringLiteral(prefix) = &args[0] {
                                    let result = self.string_starts_with(str_ptr, prefix);
                                    Some(result.into())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                        "endsWith" => {
                            if !args.is_empty() {
                                if let AstNode::StringLiteral(suffix) = &args[0] {
                                    let result = self.string_ends_with(str_ptr, suffix);
                                    Some(result.into())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                        "contains" => {
                            if !args.is_empty() {
                                if let AstNode::StringLiteral(needle) = &args[0] {
                                    let result = self.string_contains(str_ptr, needle);
                                    Some(result.into())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            }
            AstNode::Block(statements) => {
                let mut last_val: Option<BasicValueEnum<'ctx>> = None;
                for stmt in statements {
                    match stmt {
                        AstNode::Return { values } => {
                            if !values.is_empty() {
                                return self.eval_string_filter_ast_expr(&values[0]);
                            }
                        }
                        AstNode::LetDecl { pattern, value, .. } => {
                            if let Some(val) = self.eval_string_filter_ast_expr(value) {
                                if let crate::parser::ast::Pattern::Identifier(name) = pattern {
                                    self.temp_values.insert(name.clone(), val);
                                }
                            }
                        }
                        _ => {
                            last_val = self.eval_string_filter_ast_expr(stmt);
                        }
                    }
                }
                last_val
            }
            _ => None,
        }
    }

    /// Extract identifier name from an AST node
    fn extract_identifier(node: &AstNode) -> Option<&str> {
        match node {
            AstNode::Identifier(name) => Some(name),
            _ => None,
        }
    }

    /// Get string length from a string pointer
    /// Uses strlen to handle both global string constants and heap-allocated strings
    fn get_string_length(
        &mut self,
        str_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> inkwell::values::IntValue<'ctx> {
        // Use strlen to get length - works for both global constants and heap strings
        let strlen_fn = self.get_or_declare_strlen();
        let len_i64 = self
            .builder
            .build_call(strlen_fn, &[str_ptr.into()], "str_len_i64")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        // Convert i64 to i32 for comparison
        self.builder
            .build_int_cast(len_i64, self.context.i32_type(), "str_len")
            .unwrap()
    }

    /// Check if string starts with a prefix
    fn string_starts_with(
        &mut self,
        str_ptr: inkwell::values::PointerValue<'ctx>,
        prefix: &str,
    ) -> inkwell::values::IntValue<'ctx> {
        let prefix_len = prefix.len() as u64;
        let str_len = self.get_string_length(str_ptr);

        // If string is shorter than prefix, return false
        let len_check = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::UGE,
                str_len,
                self.context.i32_type().const_int(prefix_len, false),
                "len_check",
            )
            .unwrap();

        let current_fn = self
            .builder
            .get_insert_block()
            .unwrap()
            .get_parent()
            .unwrap();

        let check_bb = self.context.append_basic_block(current_fn, "starts_check");
        let false_bb = self.context.append_basic_block(current_fn, "starts_false");
        let merge_bb = self.context.append_basic_block(current_fn, "starts_merge");

        self.builder
            .build_conditional_branch(len_check, check_bb, false_bb)
            .unwrap();

        // Check block: compare prefix bytes
        self.builder.position_at_end(check_bb);
        let prefix_global = self.create_global_string(prefix, "prefix");
        let memcmp_fn = self.get_or_declare_memcmp();
        let cmp_result = self
            .builder
            .build_call(
                memcmp_fn,
                &[
                    str_ptr.into(),
                    prefix_global.into(),
                    self.context.i64_type().const_int(prefix_len, false).into(),
                ],
                "memcmp_result",
            )
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        let is_match = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                cmp_result,
                self.context.i32_type().const_int(0, false),
                "is_match",
            )
            .unwrap();
        let check_result = self
            .builder
            .build_int_z_extend(is_match, self.context.i32_type(), "check_result")
            .unwrap();
        let check_end_bb = self.builder.get_insert_block().unwrap();
        self.builder.build_unconditional_branch(merge_bb).unwrap();

        // False block
        self.builder.position_at_end(false_bb);
        let false_result = self.context.i32_type().const_int(0, false);
        self.builder.build_unconditional_branch(merge_bb).unwrap();

        // Merge block
        self.builder.position_at_end(merge_bb);
        let phi = self
            .builder
            .build_phi(self.context.i32_type(), "starts_result")
            .unwrap();
        phi.add_incoming(&[(&check_result, check_end_bb), (&false_result, false_bb)]);

        phi.as_basic_value().into_int_value()
    }

    /// Check if string ends with a suffix
    fn string_ends_with(
        &mut self,
        str_ptr: inkwell::values::PointerValue<'ctx>,
        suffix: &str,
    ) -> inkwell::values::IntValue<'ctx> {
        let suffix_len = suffix.len() as u64;
        let str_len = self.get_string_length(str_ptr);

        let len_check = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::UGE,
                str_len,
                self.context.i32_type().const_int(suffix_len, false),
                "len_check",
            )
            .unwrap();

        let current_fn = self
            .builder
            .get_insert_block()
            .unwrap()
            .get_parent()
            .unwrap();

        let check_bb = self.context.append_basic_block(current_fn, "ends_check");
        let false_bb = self.context.append_basic_block(current_fn, "ends_false");
        let merge_bb = self.context.append_basic_block(current_fn, "ends_merge");

        self.builder
            .build_conditional_branch(len_check, check_bb, false_bb)
            .unwrap();

        self.builder.position_at_end(check_bb);
        // Calculate offset: str_len - suffix_len
        let offset = self
            .builder
            .build_int_sub(
                str_len,
                self.context.i32_type().const_int(suffix_len, false),
                "offset",
            )
            .unwrap();
        let end_ptr = unsafe {
            self.builder
                .build_gep(self.context.i8_type(), str_ptr, &[offset], "end_ptr")
                .unwrap()
        };

        let suffix_global = self.create_global_string(suffix, "suffix");
        let memcmp_fn = self.get_or_declare_memcmp();
        let cmp_result = self
            .builder
            .build_call(
                memcmp_fn,
                &[
                    end_ptr.into(),
                    suffix_global.into(),
                    self.context.i64_type().const_int(suffix_len, false).into(),
                ],
                "memcmp_result",
            )
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        let is_match = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                cmp_result,
                self.context.i32_type().const_int(0, false),
                "is_match",
            )
            .unwrap();
        let check_result = self
            .builder
            .build_int_z_extend(is_match, self.context.i32_type(), "check_result")
            .unwrap();
        let check_end_bb = self.builder.get_insert_block().unwrap();
        self.builder.build_unconditional_branch(merge_bb).unwrap();

        self.builder.position_at_end(false_bb);
        let false_result = self.context.i32_type().const_int(0, false);
        self.builder.build_unconditional_branch(merge_bb).unwrap();

        self.builder.position_at_end(merge_bb);
        let phi = self
            .builder
            .build_phi(self.context.i32_type(), "ends_result")
            .unwrap();
        phi.add_incoming(&[(&check_result, check_end_bb), (&false_result, false_bb)]);

        phi.as_basic_value().into_int_value()
    }

    /// Check if string contains a substring
    fn string_contains(
        &mut self,
        str_ptr: inkwell::values::PointerValue<'ctx>,
        needle: &str,
    ) -> inkwell::values::IntValue<'ctx> {
        let strstr_fn = self.get_or_declare_strstr();
        let needle_global = self.create_global_string(needle, "needle");

        let result = self
            .builder
            .build_call(
                strstr_fn,
                &[str_ptr.into(), needle_global.into()],
                "strstr_result",
            )
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        let is_found = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                self.builder
                    .build_ptr_to_int(result, self.context.i64_type(), "ptr_int")
                    .unwrap(),
                self.context.i64_type().const_int(0, false),
                "is_found",
            )
            .unwrap();

        self.builder
            .build_int_z_extend(is_found, self.context.i32_type(), "contains_result")
            .unwrap()
    }

    /// Get or declare memcmp function
    fn get_or_declare_memcmp(&self) -> FunctionValue<'ctx> {
        if let Some(fn_val) = self.module.get_function("memcmp") {
            return fn_val;
        }

        let fn_type = self.context.i32_type().fn_type(
            &[
                self.context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into(),
                self.context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into(),
                self.context.i64_type().into(),
            ],
            false,
        );

        self.module.add_function("memcmp", fn_type, None)
    }

    /// Get or declare strstr function
    fn get_or_declare_strstr(&self) -> FunctionValue<'ctx> {
        if let Some(fn_val) = self.module.get_function("strstr") {
            return fn_val;
        }

        let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
        let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);

        self.module.add_function("strstr", fn_type, None)
    }

    /// Create a global string constant and return pointer to it
    fn create_global_string(&self, s: &str, name: &str) -> inkwell::values::PointerValue<'ctx> {
        let global_name = format!("str_const_{}", name);

        // Check if already exists
        if let Some(global) = self.module.get_global(&global_name) {
            return global.as_pointer_value();
        }

        let bytes = s.as_bytes();
        let array_type = self.context.i8_type().array_type((bytes.len() + 1) as u32);
        let global = self.module.add_global(array_type, None, &global_name);

        let mut values: Vec<inkwell::values::IntValue> = bytes
            .iter()
            .map(|b| self.context.i8_type().const_int(*b as u64, false))
            .collect();
        values.push(self.context.i8_type().const_int(0, false)); // null terminator

        let const_array = self.context.i8_type().const_array(&values);
        global.set_initializer(&const_array);
        global.set_constant(true);

        global.as_pointer_value()
    }

    /// Create a heap-allocated string from a literal (for closures)
    /// Returns pointer to the data portion of the heap string
    pub fn create_heap_string(&mut self, s: &str) -> inkwell::values::PointerValue<'ctx> {
        let strlen = s.len() as u64;
        let total_size = strlen + 1 + 8; // data + null + header (RC + len)

        let malloc_fn = self.get_or_declare_malloc();
        let heap_ptr = self
            .builder
            .build_call(
                malloc_fn,
                &[self.context.i64_type().const_int(total_size, false).into()],
                "heap_str",
            )
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Store RC = 1 at offset 0
        let rc_ptr = self
            .builder
            .build_pointer_cast(
                heap_ptr,
                self.context.ptr_type(inkwell::AddressSpace::default()),
                "rc_ptr",
            )
            .unwrap();
        self.builder
            .build_store(rc_ptr, self.context.i32_type().const_int(1, false))
            .unwrap();

        // Store length at offset 4
        let len_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    heap_ptr,
                    &[self.context.i32_type().const_int(4, false)],
                    "len_ptr",
                )
                .unwrap()
        };
        let len_ptr_cast = self
            .builder
            .build_pointer_cast(
                len_ptr,
                self.context.ptr_type(inkwell::AddressSpace::default()),
                "len_ptr_cast",
            )
            .unwrap();
        self.builder
            .build_store(
                len_ptr_cast,
                self.context.i32_type().const_int(strlen as u64, false),
            )
            .unwrap();

        // Get data pointer at offset 8
        let data_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    heap_ptr,
                    &[self.context.i32_type().const_int(8, false)],
                    "data_ptr",
                )
                .unwrap()
        };

        // Copy string data
        let global_str = self.create_global_string(s, &format!("lit_{}", s.len()));
        let memcpy_fn = self.get_or_declare_memcpy();
        self.builder
            .build_call(
                memcpy_fn,
                &[
                    data_ptr.into(),
                    global_str.into(),
                    self.context.i64_type().const_int(strlen + 1, false).into(),
                    self.context.bool_type().const_zero().into(),
                ],
                "",
            )
            .unwrap();

        data_ptr
    }

    /// Concatenate two heap-allocated strings and return new heap string
    /// Both left_ptr and right_ptr point to data portion of heap strings
    pub fn concat_strings(
        &mut self,
        left_ptr: inkwell::values::PointerValue<'ctx>,
        right_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> inkwell::values::PointerValue<'ctx> {
        let strlen_fn = self.get_or_declare_strlen();

        // Get lengths using strlen
        let left_len = self
            .builder
            .build_call(strlen_fn, &[left_ptr.into()], "left_len")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        let right_len = self
            .builder
            .build_call(strlen_fn, &[right_ptr.into()], "right_len")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        // Calculate total size: left_len + right_len + 1 (null) + 8 (header)
        let total_len = self
            .builder
            .build_int_add(left_len, right_len, "total_len")
            .unwrap();
        let total_size = self
            .builder
            .build_int_add(
                total_len,
                self.context.i64_type().const_int(9, false), // 1 for null + 8 for header
                "total_size",
            )
            .unwrap();

        // Allocate new heap string
        let malloc_fn = self.get_or_declare_malloc();
        let heap_ptr = self
            .builder
            .build_call(malloc_fn, &[total_size.into()], "concat_heap")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Store RC = 1
        let rc_ptr = self
            .builder
            .build_pointer_cast(
                heap_ptr,
                self.context.ptr_type(inkwell::AddressSpace::default()),
                "rc_ptr",
            )
            .unwrap();
        self.builder
            .build_store(rc_ptr, self.context.i32_type().const_int(1, false))
            .unwrap();

        // Store length
        let len_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    heap_ptr,
                    &[self.context.i32_type().const_int(4, false)],
                    "len_ptr",
                )
                .unwrap()
        };
        let len_ptr_cast = self
            .builder
            .build_pointer_cast(
                len_ptr,
                self.context.ptr_type(inkwell::AddressSpace::default()),
                "len_ptr_cast",
            )
            .unwrap();
        let total_len_i32 = self
            .builder
            .build_int_cast(total_len, self.context.i32_type(), "total_len_i32")
            .unwrap();
        self.builder
            .build_store(len_ptr_cast, total_len_i32)
            .unwrap();

        // Get data pointer
        let data_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    heap_ptr,
                    &[self.context.i32_type().const_int(8, false)],
                    "data_ptr",
                )
                .unwrap()
        };

        // Copy left string
        let memcpy_fn = self.get_or_declare_memcpy();
        self.builder
            .build_call(
                memcpy_fn,
                &[
                    data_ptr.into(),
                    left_ptr.into(),
                    left_len.into(),
                    self.context.bool_type().const_zero().into(),
                ],
                "",
            )
            .unwrap();

        // Copy right string after left
        let left_len_i32 = self
            .builder
            .build_int_cast(left_len, self.context.i32_type(), "left_len_i32")
            .unwrap();
        let right_dest = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    data_ptr,
                    &[left_len_i32],
                    "right_dest",
                )
                .unwrap()
        };
        self.builder
            .build_call(
                memcpy_fn,
                &[
                    right_dest.into(),
                    right_ptr.into(),
                    right_len.into(),
                    self.context.bool_type().const_zero().into(),
                ],
                "",
            )
            .unwrap();

        // Add null terminator
        let null_pos = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    data_ptr,
                    &[self
                        .builder
                        .build_int_cast(total_len, self.context.i32_type(), "total_i32")
                        .unwrap()],
                    "null_pos",
                )
                .unwrap()
        };
        self.builder
            .build_store(null_pos, self.context.i8_type().const_zero())
            .unwrap();

        data_ptr
    }
}
