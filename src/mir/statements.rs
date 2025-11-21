use crate::lexer::token::TokenType;
use crate::limits::MIR_MAX_DEPTH;
use crate::mir::builder::MirBuilder;
use crate::mir::expresssions::build_expression;
use crate::mir::{MirBlock, MirInstr};
use crate::parser::ast::{AstNode, Pattern};

pub fn build_statement(builder: &mut MirBuilder, stmt: &AstNode, block: &mut MirBlock) {
    // Check recursion depth to prevent stack overflow
    builder.recursion_depth += 1;
    if builder.recursion_depth > MIR_MAX_DEPTH {
        builder.recursion_depth -= 1;
        return;
    }

    build_statement_inner(builder, stmt, block);
    builder.recursion_depth -= 1;
}

fn build_statement_inner(builder: &mut MirBuilder, stmt: &AstNode, block: &mut MirBlock) {
    match stmt {
        // Handle variable declaration (`let` statement).
        // Supports both single variable and tuple destructuring patterns.
        AstNode::LetDecl {
            pattern,
            value,
            mutable,
            ..
        } => {
            // Build MIR for the right-hand side expression.
            let value_tmp = build_expression(builder, value, block);

            match pattern {
                // Simple variable assignment.
                Pattern::Identifier(name) => {
                    block.instrs.push(MirInstr::Assign {
                        name: name.clone(),
                        value: value_tmp.clone(),
                        mutable: *mutable,
                    });

                    // Track variable type in mir_symbol_table
                    // Copy type from value_tmp if available
                    if let Some(value_type) = builder.mir_symbol_table.get(&value_tmp).cloned() {
                        builder.mir_symbol_table.insert(name.clone(), value_type);
                    }
                }
                // Tuple destructuring: let (a, b) = expr;
                Pattern::Tuple(patterns) => {
                    for (i, pattern) in patterns.iter().enumerate() {
                        if let Pattern::Identifier(name) = pattern {
                            // Extract each tuple element into a temporary variable.
                            let extract_tmp = builder.next_tmp();
                            block.instrs.push(MirInstr::TupleExtract {
                                name: extract_tmp.clone(),
                                source: value_tmp.clone(),
                                index: i,
                            });
                            block.instrs.push(MirInstr::Assign {
                                name: name.clone(),
                                value: extract_tmp,
                                mutable: *mutable,
                            });
                        }
                    }
                }
                // Other patterns (wildcards, structs) can be added here in the future.
                _ => {}
            }
        }

        // Handle manual error extraction with ?? operator (e.g., let a, b ?? err = expr;)
        AstNode::ManualErrorExtract {
            expr,
            ok_pattern,
            error_var,
        } => {
            // Build MIR for the expression that returns Result
            let result_tmp = build_expression(builder, expr, block);

            // Collect Ok value names from the pattern
            let ok_names = match ok_pattern {
                Pattern::Identifier(name) => vec![name.clone()],
                Pattern::Tuple(patterns) => patterns
                    .iter()
                    .filter_map(|p| {
                        if let Pattern::Identifier(name) = p {
                            Some(name.clone())
                        } else {
                            None
                        }
                    })
                    .collect(),
                Pattern::Wildcard => vec![],
            };

            // Generate ManualErrorExtract instruction
            block.instrs.push(MirInstr::ManualErrorExtract {
                ok_names,
                error_name: error_var.clone(),
                result: result_tmp,
            });
        }

        // Handle assignment statements (e.g., x = expr, (a, b) = func()).
        AstNode::Assignment { pattern, value } => {
            let value_tmp = build_expression(builder, value, block);

            match pattern {
                // Simple variable assignment.
                Pattern::Identifier(name) => {
                    block.instrs.push(MirInstr::Assign {
                        name: name.clone(),
                        value: value_tmp.clone(),
                        mutable: true,
                    });

                    // Track variable type in mir_symbol_table for re-assignments
                    // Copy type from value_tmp if available
                    if let Some(value_type) = builder.mir_symbol_table.get(&value_tmp).cloned() {
                        builder.mir_symbol_table.insert(name.clone(), value_type);
                    }
                }
                // Tuple destructuring assignment.
                Pattern::Tuple(patterns) => {
                    for (i, pattern) in patterns.iter().enumerate() {
                        if let Pattern::Identifier(name) = pattern {
                            // Extract each tuple element into a temporary variable.
                            let extract_tmp = builder.next_tmp();
                            block.instrs.push(MirInstr::TupleExtract {
                                name: extract_tmp.clone(),
                                source: value_tmp.clone(),
                                index: i,
                            });
                            block.instrs.push(MirInstr::Assign {
                                name: name.clone(),
                                value: extract_tmp,
                                mutable: true,
                            });
                        }
                    }
                }
                // Other patterns can be added here in the future.
                _ => {}
            }
        }

        // Handle compound assignment statements (e.g., x += 1, y *= 2).
        // Converts `x += expr` to `x = x + expr` at the MIR level.
        AstNode::CompoundAssignment { pattern, op, value } => {
            if let Pattern::Identifier(name) = pattern {
                // Build MIR for the RHS expression
                let rhs_tmp = build_expression(builder, value, block);

                // Map compound operator to binary operator string
                let op_str = match op {
                    TokenType::PlusEq => "add",
                    TokenType::MinusEq => "sub",
                    TokenType::StarEq => "mul",
                    TokenType::SlashEq => "div",
                    TokenType::PercentEq => "mod",
                    _ => return, // Should not happen due to parser validation
                };

                // Determine the operation type (int or float) based on the variable and RHS types
                use crate::mir::expresssions::determine_op_type;
                let op_type = match determine_op_type(builder, name, &rhs_tmp) {
                    Ok(t) => t,
                    Err(_) => "int".to_string(), // Default to int if type cannot be determined
                };

                // Create a temporary for the binary operation result
                let result_tmp = builder.next_tmp();

                // Generate: result_tmp = name <op> rhs_tmp
                block.instrs.push(MirInstr::BinaryOp(
                    format!("{}:{}", op_str, op_type),
                    result_tmp.clone(),
                    name.clone(),
                    rhs_tmp,
                ));

                // Generate: name = result_tmp
                block.instrs.push(MirInstr::Assign {
                    name: name.clone(),
                    value: result_tmp.clone(),
                    mutable: true,
                });

                // Track variable type in mir_symbol_table
                if let Some(value_type) = builder.mir_symbol_table.get(&result_tmp).cloned() {
                    builder.mir_symbol_table.insert(name.clone(), value_type);
                }
            }
        }

        // Handle increment/decrement statements (e.g., i++, i--)
        AstNode::IncrementDecrement { variable, op } => {
            let op_str = match op {
                crate::lexer::token::TokenType::PlusPlus => "++",
                crate::lexer::token::TokenType::MinusMinus => "--",
                _ => return,
            };

            block.instrs.push(MirInstr::IncrementDecrement {
                variable: variable.clone(),
                op: op_str.to_string(),
            });
        }

        // Handle element assignment statements (e.g., arr[0] = 5, map["key"] = value).
        AstNode::ElementAssignment {
            array,
            index,
            value,
        } => {
            // Build MIR for array/map expression
            let _array_tmp = build_expression(builder, array, block);

            // Build MIR for index expression
            let index_tmp = build_expression(builder, index, block);

            // Build MIR for value expression
            let value_tmp = build_expression(builder, value, block);

            // Get the array/map variable name
            if let AstNode::Identifier(array_name) = &**array {
                // Check if it's an array or map based on the MIR symbol table type
                // For now, we'll emit both ArraySet and MapSet and let codegen handle it
                // We can determine the type from builder.mir_symbol_table if available

                // Emit ArraySet instruction (works for both arrays and maps at MIR level)
                block.instrs.push(MirInstr::ArraySet {
                    array: array_name.clone(),
                    index: index_tmp,
                    value: value_tmp,
                });
            }
        }

        // Handle struct declarations (type definitions, not instances).
        AstNode::StructDecl {
            name,
            fields,
            is_public,
        } => {
            // Create a placeholder instance showing the structure.
            let tmp = builder.next_tmp();
            let field_vals: Vec<(String, String)> = fields
                .iter()
                .map(|field| {
                    let val_tmp = builder.next_tmp();
                    (field.name.clone(), val_tmp)
                })
                .collect();

            block.instrs.push(MirInstr::StructInit {
                name: tmp,
                struct_name: name.clone(),
                fields: field_vals,
            });
        }

        // Handle enum declarations (type definitions, not instances).
        AstNode::EnumDecl {
            name,
            variants,
            is_public,
        } => {
            for variant in variants {
                let variant_name = &variant.name;
                let opt_type = &variant.payload;
                let tmp = builder.next_tmp();
                let value_tmp = opt_type.as_ref().map(|_| builder.next_tmp());
                block.instrs.push(MirInstr::EnumInit {
                    name: tmp,
                    enum_name: name.clone(),
                    variant: variant_name.clone(),
                    value: value_tmp,
                });
            }
        }

        // Handle conditional statements (if/else).
        AstNode::ConditionalStmt {
            condition,
            then_block,
            else_branch,
        } => {
            // Build MIR for the condition expression.
            let cond_tmp = build_expression(builder, condition, block);

            // Generate labels for then, else, and exit blocks.
            let then_label = builder.next_block();
            let else_label = builder.next_block();
            let end_label = builder.next_block();

            block.terminator = Some(MirInstr::CondJump {
                cond: cond_tmp,
                then_block: then_label.clone(),
                else_block: if else_branch.is_some() {
                    else_label.clone()
                } else {
                    end_label.clone()
                },
            });

            // Then block with scope tracking for reference counting.
            builder.enter_scope();
            let mut then_mir_block = MirBlock {
                label: then_label,
                instrs: vec![],
                terminator: None,
            };

            for stmt in then_block {
                build_statement(builder, stmt, &mut then_mir_block);
            }

            builder.exit_scope(&mut then_mir_block); // DecRefs inserted here

            // Add jump to end if then block doesn't have a terminator
            if then_mir_block.terminator.is_none() {
                then_mir_block.terminator = Some(MirInstr::Jump {
                    target: end_label.clone(),
                });
            }

            if let Some(else_stmt) = else_branch {
                builder.enter_scope();
                let mut else_mir_block = MirBlock {
                    label: else_label,
                    instrs: vec![],
                    terminator: None, // Don't preset terminator - let statements set it
                };

                // Handle else branch - it might be a Block or a single statement
                match else_stmt.as_ref() {
                    AstNode::Block(statements) => {
                        // If it's a block, iterate through all statements
                        for stmt in statements {
                            build_statement(builder, stmt, &mut else_mir_block);
                        }
                    }
                    _ => {
                        // Single statement (like another if)
                        build_statement(builder, else_stmt, &mut else_mir_block);
                    }
                }

                builder.exit_scope(&mut else_mir_block);

                // Only add jump to end if block doesn't already have a terminator (like Return)
                if else_mir_block.terminator.is_none() {
                    else_mir_block.terminator = Some(MirInstr::Jump {
                        target: end_label.clone(),
                    });
                }

                if let Some(current_func) = builder.program.functions.last_mut() {
                    // Save the original block (with CondJump) before modifying it
                    let original_block = MirBlock {
                        label: block.label.clone(),
                        instrs: block.instrs.clone(),
                        terminator: block.terminator.clone(),
                    };
                    current_func.blocks.push(original_block);
                    current_func.blocks.push(then_mir_block);
                    current_func.blocks.push(else_mir_block);
                }
            } else {
                if let Some(current_func) = builder.program.functions.last_mut() {
                    // Save the original block (with CondJump) before modifying it
                    let original_block = MirBlock {
                        label: block.label.clone(),
                        instrs: block.instrs.clone(),
                        terminator: block.terminator.clone(),
                    };
                    current_func.blocks.push(original_block);
                    current_func.blocks.push(then_mir_block);
                }
            }

            // Replace current block with the end_label continuation
            // This ensures subsequent statements in the same scope go into the continuation block
            block.label = end_label.clone();
            block.instrs.clear();
            block.terminator = None;
        }

        // Handle return statements.
        AstNode::Return { values } => {
            let mut ret_vals = vec![];
            for val in values {
                // Build MIR for each return value expression.
                let ret_tmp = build_expression(builder, val, block);
                ret_vals.push(ret_tmp);
            }
            block.terminator = Some(MirInstr::Return { values: ret_vals });
        }

        // Handle standalone expressions (like function calls for their side effects).
        AstNode::BinaryExpr { .. } | AstNode::FunctionCall { .. } | AstNode::MethodCall { .. } => {
            // Evaluate the expression but don't necessarily store the result.
            build_expression(builder, stmt, block);
        }

        // Handle print statements.
        AstNode::Print { exprs } => {
            let mut vals = vec![];
            for expr in exprs {
                // Build MIR for each print argument.
                let val_tmp = build_expression(builder, expr, block);
                vals.push(val_tmp);
            }
            block.instrs.push(MirInstr::Print { values: vals });
        }

        // Handle break statement in loops.
        AstNode::Break => {
            if let Some(loop_ctx) = builder.current_loop() {
                block.terminator = Some(MirInstr::Jump {
                    target: loop_ctx.break_target.clone(),
                });
            } else {
                debug_assert!(
                    false,
                    "Break statement outside of loop - should be caught by analyzer"
                );
            }
        }

        // Handle continue statement in loops.
        AstNode::Continue => {
            if let Some(loop_ctx) = builder.current_loop() {
                block.terminator = Some(MirInstr::Jump {
                    target: loop_ctx.continue_target.clone(),
                });
            } else {
                debug_assert!(
                    false,
                    "Continue statement outside of loop - should be caught by analyzer"
                );
            }
        }

        // Handle for loop statements, including infinite loops and loops with iterable.
        AstNode::ForLoopStmt {
            pattern,
            iterable,
            body,
        } => {
            // Infinite loop: for { ... }
            if iterable.is_none() {
                let loop_header = builder.next_block();
                let loop_body = builder.next_block();
                let loop_end = builder.next_block();

                // Enter loop context for break/continue handling.
                builder.enter_loop(loop_end.clone(), loop_header.clone());

                // Only set terminator if block doesn't already have one
                if block.terminator.is_none() {
                    block.terminator = Some(MirInstr::Jump {
                        target: loop_header.clone(),
                    });
                } else {
                    // Sequential loops: connect previous loop's exit to this loop's header
                    if let Some(current_func) = builder.program.functions.last_mut() {
                        for prev_block in current_func.blocks.iter_mut().rev() {
                            if prev_block.terminator.is_none() {
                                prev_block.terminator = Some(MirInstr::Jump {
                                    target: loop_header.clone(),
                                });
                                break;
                            }
                        }
                    }
                }

                // Header block jumps directly to body.
                let header_block = MirBlock {
                    label: loop_header.clone(),
                    instrs: vec![],
                    terminator: Some(MirInstr::Jump {
                        target: loop_body.clone(),
                    }),
                };

                // Body block executes statements, then jumps back to header.
                let mut body_block = MirBlock {
                    label: loop_body.clone(),
                    instrs: vec![],
                    terminator: None,
                };
                for stmt in body {
                    build_statement(builder, stmt, &mut body_block);
                }
                if body_block.terminator.is_none() {
                    body_block.terminator = Some(MirInstr::Jump {
                        target: loop_header,
                    });
                }

                if let Some(func) = builder.program.functions.last_mut() {
                    func.blocks.push(header_block);
                    func.blocks.push(body_block);
                    func.blocks.push(MirBlock {
                        label: loop_end,
                        instrs: vec![],
                        terminator: None,
                    });
                }

                builder.exit_loop();
                return; // stop further processing
            }

            // Check if this is a tuple pattern for map iteration
            let is_tuple_pattern = matches!(pattern, Pattern::Tuple(_));
            let (key_var, value_var) = if let Pattern::Tuple(ref patterns) = pattern {
                if patterns.len() == 2 {
                    let key = match &patterns[0] {
                        Pattern::Identifier(name) => name.clone(),
                        _ => builder.next_tmp(),
                    };
                    let val = match &patterns[1] {
                        Pattern::Identifier(name) => name.clone(),
                        _ => builder.next_tmp(),
                    };
                    (Some(key), Some(val))
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };

            let loop_var = match pattern {
                Pattern::Identifier(name) => Some(name.clone()),
                Pattern::Wildcard => Some("_".to_string()), // <- handle wildcard here
                Pattern::Tuple(_) => {
                    // For tuple patterns, use a temp variable for the pair
                    if key_var.is_some() && value_var.is_some() {
                        Some(builder.next_tmp())
                    } else {
                        Some(builder.next_tmp())
                    }
                }
            };

            let loop_header = builder.next_block();
            let loop_body = builder.next_block();
            let loop_increment = builder.next_block();
            let loop_end = builder.next_block();

            // Enter loop context (continue goes to increment, break goes to end)
            builder.enter_loop(loop_end.clone(), loop_increment.clone());

            let mut blocks_to_add = Vec::new();

            if let Some(iter_expr) = iterable {
                match iter_expr.as_ref() {
                    // Range-based loops: for i in 0..10
                    AstNode::BinaryExpr { left, op, right }
                        if matches!(op, TokenType::RangeExc | TokenType::RangeInc) =>
                    {
                        let loop_var = loop_var.expect("Loop variable required");

                        // Initialize loop variable
                        let start_tmp = build_expression(builder, left, block);
                        block.instrs.push(MirInstr::Assign {
                            name: loop_var.clone(),
                            value: start_tmp,
                            mutable: true,
                        });

                        // Store end value in a variable so it's accessible in header block
                        let end_tmp = build_expression(builder, right, block);
                        let end_var = format!("{}_end", loop_var);
                        block.instrs.push(MirInstr::Assign {
                            name: end_var.clone(),
                            value: end_tmp,
                            mutable: false,
                        });

                        // Set terminator to jump to this loop's header
                        // If block already has a terminator, we're in a sequential loop situation
                        // The previous loop's exit block should already be handled below
                        if block.terminator.is_none() {
                            block.terminator = Some(MirInstr::Jump {
                                target: loop_header.clone(),
                            });
                        } else {
                            // Sequential loops: connect previous loop's exit to this loop's header
                            if let Some(current_func) = builder.program.functions.last_mut() {
                                // Find the most recently added exit block that has no terminator
                                for prev_block in current_func.blocks.iter_mut().rev() {
                                    if prev_block.terminator.is_none() {
                                        prev_block.terminator = Some(MirInstr::Jump {
                                            target: loop_header.clone(),
                                        });
                                        break;
                                    }
                                }
                            }
                        }

                        // Header block: condition check
                        let mut header_block = MirBlock {
                            label: loop_header.clone(),
                            instrs: vec![],
                            terminator: None,
                        };

                        let cmp_tmp = builder.next_tmp();
                        let op_str = match op {
                            TokenType::RangeInc => "le",
                            TokenType::RangeExc => "lt",
                            _ => unreachable!(),
                        };

                        header_block.instrs.push(MirInstr::BinaryOp(
                            op_str.to_string(),
                            cmp_tmp.clone(),
                            loop_var.clone(),
                            end_var,
                        ));

                        header_block.terminator = Some(MirInstr::CondJump {
                            cond: cmp_tmp,
                            then_block: loop_body.clone(),
                            else_block: loop_end.clone(),
                        });

                        blocks_to_add.push(header_block);

                        // Body block: execute loop statements
                        let mut body_block = MirBlock {
                            label: loop_body.clone(),
                            instrs: vec![],
                            terminator: None,
                        };

                        // Build body statements (may contain break/continue)
                        for stmt in body {
                            build_statement(builder, stmt, &mut body_block);
                        }

                        // If no break/continue, jump to increment
                        if body_block.terminator.is_none() {
                            body_block.terminator = Some(MirInstr::Jump {
                                target: loop_increment.clone(),
                            });
                        }

                        blocks_to_add.push(body_block);

                        // Increment block: i = i + 1, then jump to header
                        let mut increment_block = MirBlock {
                            label: loop_increment,
                            instrs: vec![],
                            terminator: None,
                        };

                        let one_tmp = builder.next_tmp();
                        increment_block.instrs.push(MirInstr::ConstInt {
                            name: one_tmp.clone(),
                            value: 1,
                        });

                        let new_val_tmp = builder.next_tmp();
                        increment_block.instrs.push(MirInstr::BinaryOp(
                            "add".to_string(),
                            new_val_tmp.clone(),
                            loop_var.clone(),
                            one_tmp,
                        ));

                        increment_block.instrs.push(MirInstr::Assign {
                            name: loop_var,
                            value: new_val_tmp,
                            mutable: true,
                        });

                        increment_block.terminator = Some(MirInstr::Jump {
                            target: loop_header.clone(),
                        });

                        blocks_to_add.push(increment_block);

                        // End block
                        let end_block = MirBlock {
                            label: loop_end,
                            instrs: vec![],
                            terminator: None,
                        };

                        blocks_to_add.push(end_block);
                    }

                    // Map iteration: for (key, value) in map
                    AstNode::MapLiteral(_) => {
                        // Check if this is a tuple pattern for map iteration
                        if let Pattern::Tuple(ref patterns) = pattern {
                            if patterns.len() == 2 {
                                // Extract key and value variable names
                                let key_var = match &patterns[0] {
                                    Pattern::Identifier(name) => name.clone(),
                                    _ => builder.next_tmp(),
                                };
                                let value_var = match &patterns[1] {
                                    Pattern::Identifier(name) => name.clone(),
                                    _ => builder.next_tmp(),
                                };

                                let iter_tmp = build_expression(builder, iter_expr, block);

                                // Store map directly without creating an array wrapper
                                let map_var = format!("{}_{}_map", key_var, value_var);
                                block.instrs.push(MirInstr::Assign {
                                    name: map_var.clone(),
                                    value: iter_tmp,
                                    mutable: false,
                                });

                                let index_var = format!("{}_{}__index", key_var, value_var);

                                // Initialize index
                                let zero_tmp = builder.next_tmp();
                                block.instrs.push(MirInstr::ConstInt {
                                    name: zero_tmp.clone(),
                                    value: 0,
                                });
                                block.instrs.push(MirInstr::Assign {
                                    name: index_var.clone(),
                                    value: zero_tmp,
                                    mutable: true,
                                });

                                if block.terminator.is_none() {
                                    block.terminator = Some(MirInstr::Jump {
                                        target: loop_header.clone(),
                                    });
                                } else {
                                    // Sequential loops: connect previous loop's exit to this loop's header
                                    if let Some(current_func) = builder.program.functions.last_mut()
                                    {
                                        for prev_block in current_func.blocks.iter_mut().rev() {
                                            if prev_block.terminator.is_none() {
                                                prev_block.terminator = Some(MirInstr::Jump {
                                                    target: loop_header.clone(),
                                                });
                                                break;
                                            }
                                        }
                                    }
                                }

                                // Header: check map bounds
                                let mut header_block = MirBlock {
                                    label: loop_header.clone(),
                                    instrs: vec![],
                                    terminator: None,
                                };

                                // Use MapLen instruction for maps
                                let len_tmp = builder.next_tmp();
                                header_block.instrs.push(MirInstr::MapLen {
                                    name: len_tmp.clone(),
                                    map: map_var.clone(),
                                });

                                let cmp_tmp = builder.next_tmp();
                                header_block.instrs.push(MirInstr::BinaryOp(
                                    "lt".to_string(),
                                    cmp_tmp.clone(),
                                    index_var.clone(),
                                    len_tmp,
                                ));

                                header_block.terminator = Some(MirInstr::CondJump {
                                    cond: cmp_tmp,
                                    then_block: loop_body.clone(),
                                    else_block: loop_end.clone(),
                                });

                                blocks_to_add.push(header_block);

                                // Body: extract key-value pair
                                let mut body_block = MirBlock {
                                    label: loop_body.clone(),
                                    instrs: vec![],
                                    terminator: None,
                                };

                                // Use MapGet to extract key-value pair
                                let pair_tmp = builder.next_tmp();
                                body_block.instrs.push(MirInstr::MapGetPair {
                                    name: pair_tmp.clone(),
                                    map: map_var,
                                    index: index_var.clone(),
                                });

                                // Extract key and value from pair
                                body_block.instrs.push(MirInstr::TupleGet {
                                    name: key_var.clone(),
                                    tuple: pair_tmp.clone(),
                                    index: 0,
                                });

                                body_block.instrs.push(MirInstr::TupleGet {
                                    name: value_var.clone(),
                                    tuple: pair_tmp,
                                    index: 1,
                                });

                                // Build body statements
                                for stmt in body {
                                    build_statement(builder, stmt, &mut body_block);
                                }

                                if body_block.terminator.is_none() {
                                    body_block.terminator = Some(MirInstr::Jump {
                                        target: loop_increment.clone(),
                                    });
                                }

                                blocks_to_add.push(body_block);

                                // Increment block
                                let mut increment_block = MirBlock {
                                    label: loop_increment,
                                    instrs: vec![],
                                    terminator: None,
                                };

                                let one_tmp = builder.next_tmp();
                                increment_block.instrs.push(MirInstr::ConstInt {
                                    name: one_tmp.clone(),
                                    value: 1,
                                });

                                let new_index_tmp = builder.next_tmp();
                                increment_block.instrs.push(MirInstr::BinaryOp(
                                    "add".to_string(),
                                    new_index_tmp.clone(),
                                    index_var.clone(),
                                    one_tmp,
                                ));

                                increment_block.instrs.push(MirInstr::Assign {
                                    name: index_var,
                                    value: new_index_tmp,
                                    mutable: true,
                                });

                                increment_block.terminator = Some(MirInstr::Jump {
                                    target: loop_header.clone(),
                                });

                                blocks_to_add.push(increment_block);

                                // End block
                                let end_block = MirBlock {
                                    label: loop_end,
                                    instrs: vec![],
                                    terminator: None,
                                };

                                blocks_to_add.push(end_block);

                                if block.terminator.is_some()
                                    && !blocks_to_add.is_empty()
                                    && builder.loop_stack.len() == 1
                                {
                                    if let Some(current_func) = builder.program.functions.last_mut()
                                    {
                                        for prev_block in current_func.blocks.iter_mut().rev() {
                                            if prev_block.terminator.is_none() {
                                                prev_block.terminator = Some(MirInstr::Jump {
                                                    target: loop_header.clone(),
                                                });
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Array literal iteration: for i in [1, 2, 3]
                    AstNode::ArrayLiteral(_) => {
                        if let Some(loop_var) = &loop_var {
                            let iter_tmp = build_expression(builder, iter_expr, block);

                            // Store array in a variable so it's accessible in header block
                            let array_var = format!("{}_array", loop_var);
                            block.instrs.push(MirInstr::Assign {
                                name: array_var.clone(),
                                value: iter_tmp,
                                mutable: false,
                            });

                            let index_var = format!("{}__index", loop_var);

                            // Initialize index
                            let zero_tmp = builder.next_tmp();
                            block.instrs.push(MirInstr::ConstInt {
                                name: zero_tmp.clone(),
                                value: 0,
                            });
                            block.instrs.push(MirInstr::Assign {
                                name: index_var.clone(),
                                value: zero_tmp,
                                mutable: true,
                            });

                            // Only set terminator if block doesn't already have one
                            if block.terminator.is_none() {
                                block.terminator = Some(MirInstr::Jump {
                                    target: loop_header.clone(),
                                });
                            } else {
                                // Sequential loops: connect previous loop's exit to this loop's header
                                if let Some(current_func) = builder.program.functions.last_mut() {
                                    for prev_block in current_func.blocks.iter_mut().rev() {
                                        if prev_block.terminator.is_none() {
                                            prev_block.terminator = Some(MirInstr::Jump {
                                                target: loop_header.clone(),
                                            });
                                            break;
                                        }
                                    }
                                }
                            }

                            // Header: bounds check
                            let mut header_block = MirBlock {
                                label: loop_header.clone(),
                                instrs: vec![],
                                terminator: None,
                            };

                            let len_tmp = builder.next_tmp();
                            header_block.instrs.push(MirInstr::ArrayLen {
                                name: len_tmp.clone(),
                                array: array_var.clone(),
                            });

                            let cmp_tmp = builder.next_tmp();
                            header_block.instrs.push(MirInstr::BinaryOp(
                                "lt".to_string(),
                                cmp_tmp.clone(),
                                index_var.clone(),
                                len_tmp,
                            ));

                            header_block.terminator = Some(MirInstr::CondJump {
                                cond: cmp_tmp,
                                then_block: loop_body.clone(),
                                else_block: loop_end.clone(),
                            });

                            blocks_to_add.push(header_block);

                            // Body: extract element and execute statements
                            let mut body_block = MirBlock {
                                label: loop_body.clone(),
                                instrs: vec![],
                                terminator: None,
                            };

                            let elem_tmp = builder.next_tmp();
                            body_block.instrs.push(MirInstr::ArrayGet {
                                name: elem_tmp.clone(),
                                array: array_var.clone(),
                                index: index_var.clone(),
                            });

                            // Assign element to loop variable
                            body_block.instrs.push(MirInstr::Assign {
                                name: loop_var.clone(),
                                value: elem_tmp,
                                mutable: false,
                            });

                            // Build body statements
                            for stmt in body {
                                build_statement(builder, stmt, &mut body_block);
                            }

                            if body_block.terminator.is_none() {
                                body_block.terminator = Some(MirInstr::Jump {
                                    target: loop_increment.clone(),
                                });
                            }

                            blocks_to_add.push(body_block);

                            // Increment: index++
                            let mut increment_block = MirBlock {
                                label: loop_increment,
                                instrs: vec![],
                                terminator: None,
                            };

                            let one_tmp = builder.next_tmp();
                            increment_block.instrs.push(MirInstr::ConstInt {
                                name: one_tmp.clone(),
                                value: 1,
                            });

                            let new_index_tmp = builder.next_tmp();
                            increment_block.instrs.push(MirInstr::BinaryOp(
                                "add".to_string(),
                                new_index_tmp.clone(),
                                index_var.clone(),
                                one_tmp,
                            ));

                            increment_block.instrs.push(MirInstr::Assign {
                                name: index_var,
                                value: new_index_tmp,
                                mutable: true,
                            });

                            increment_block.terminator = Some(MirInstr::Jump {
                                target: loop_header.clone(),
                            });

                            blocks_to_add.push(increment_block);

                            // End block
                            let end_block = MirBlock {
                                label: loop_end,
                                instrs: vec![],
                                terminator: None,
                            };

                            blocks_to_add.push(end_block);
                        }
                    }

                    // Array iteration with break/continue support
                    AstNode::Identifier(_) => {
                        if let Some(loop_var) = &loop_var {
                            let iter_tmp = build_expression(builder, iter_expr, block);

                            // Store array in a variable so it's accessible in header block
                            let array_var = format!("{}_array", loop_var);
                            block.instrs.push(MirInstr::Assign {
                                name: array_var.clone(),
                                value: iter_tmp,
                                mutable: false,
                            });

                            let index_var = format!("{}__index", loop_var);

                            // Initialize index
                            let zero_tmp = builder.next_tmp();
                            block.instrs.push(MirInstr::ConstInt {
                                name: zero_tmp.clone(),
                                value: 0,
                            });
                            block.instrs.push(MirInstr::Assign {
                                name: index_var.clone(),
                                value: zero_tmp,
                                mutable: true,
                            });

                            // Only set terminator if block doesn't already have one
                            if block.terminator.is_none() {
                                block.terminator = Some(MirInstr::Jump {
                                    target: loop_header.clone(),
                                });
                            } else {
                                // Sequential loops: connect previous loop's exit to this loop's header
                                if let Some(current_func) = builder.program.functions.last_mut() {
                                    for prev_block in current_func.blocks.iter_mut().rev() {
                                        if prev_block.terminator.is_none() {
                                            prev_block.terminator = Some(MirInstr::Jump {
                                                target: loop_header.clone(),
                                            });
                                            break;
                                        }
                                    }
                                }
                            }

                            // Header: bounds check
                            let mut header_block = MirBlock {
                                label: loop_header.clone(),
                                instrs: vec![],
                                terminator: None,
                            };

                            let len_tmp = builder.next_tmp();
                            header_block.instrs.push(MirInstr::ArrayLen {
                                name: len_tmp.clone(),
                                array: array_var.clone(),
                            });

                            let cmp_tmp = builder.next_tmp();
                            header_block.instrs.push(MirInstr::BinaryOp(
                                "lt".to_string(),
                                cmp_tmp.clone(),
                                index_var.clone(),
                                len_tmp,
                            ));

                            header_block.terminator = Some(MirInstr::CondJump {
                                cond: cmp_tmp,
                                then_block: loop_body.clone(),
                                else_block: loop_end.clone(),
                            });

                            blocks_to_add.push(header_block);

                            // Body: extract element and execute statements
                            let mut body_block = MirBlock {
                                label: loop_body.clone(),
                                instrs: vec![],
                                terminator: None,
                            };

                            let elem_tmp = builder.next_tmp();
                            body_block.instrs.push(MirInstr::ArrayGet {
                                name: elem_tmp.clone(),
                                array: array_var.clone(),
                                index: index_var.clone(),
                            });

                            // If this is a tuple pattern (for map iteration), extract key and value
                            if is_tuple_pattern && key_var.is_some() && value_var.is_some() {
                                let key = key_var.as_ref().unwrap();
                                let val = value_var.as_ref().unwrap();

                                // Extract key (field 0) from the pair
                                body_block.instrs.push(MirInstr::TupleGet {
                                    name: key.clone(),
                                    tuple: elem_tmp.clone(),
                                    index: 0,
                                });

                                // Extract value (field 1) from the pair
                                body_block.instrs.push(MirInstr::TupleGet {
                                    name: val.clone(),
                                    tuple: elem_tmp,
                                    index: 1,
                                });
                            } else {
                                // Regular array iteration - assign element to loop variable
                                body_block.instrs.push(MirInstr::Assign {
                                    name: loop_var.clone(),
                                    value: elem_tmp,
                                    mutable: false,
                                });
                            }

                            // Build body statements
                            for stmt in body {
                                build_statement(builder, stmt, &mut body_block);
                            }

                            if body_block.terminator.is_none() {
                                body_block.terminator = Some(MirInstr::Jump {
                                    target: loop_increment.clone(),
                                });
                            }

                            blocks_to_add.push(body_block);

                            // Increment: index++
                            let mut increment_block = MirBlock {
                                label: loop_increment,
                                instrs: vec![],
                                terminator: None,
                            };

                            let one_tmp = builder.next_tmp();
                            increment_block.instrs.push(MirInstr::ConstInt {
                                name: one_tmp.clone(),
                                value: 1,
                            });

                            let new_index_tmp = builder.next_tmp();
                            increment_block.instrs.push(MirInstr::BinaryOp(
                                "add".to_string(),
                                new_index_tmp.clone(),
                                index_var.clone(),
                                one_tmp,
                            ));

                            increment_block.instrs.push(MirInstr::Assign {
                                name: index_var,
                                value: new_index_tmp,
                                mutable: true,
                            });

                            increment_block.terminator = Some(MirInstr::Jump {
                                target: loop_header.clone(),
                            });

                            blocks_to_add.push(increment_block);

                            // End block
                            let end_block = MirBlock {
                                label: loop_end,
                                instrs: vec![],
                                terminator: None,
                            };

                            blocks_to_add.push(end_block);
                        }
                    }

                    _ => {
                        // Handle other cases
                    }
                }
            }

            // Add the initialization block FIRST, then the loop blocks
            if let Some(current_func) = builder.program.functions.last_mut() {
                // ALWAYS push the current block if it has instructions OR will have a terminator
                // This ensures statements before the loop are not lost
                if !block.instrs.is_empty() {
                    current_func.blocks.push(block.clone());
                } else if block.terminator.is_some() {
                    // Block has terminator but no instructions - still add it for flow control
                    current_func.blocks.push(block.clone());
                }

                // Then add the loop blocks (header, body, increment, exit)
                current_func.blocks.extend(blocks_to_add);

                // If we're in a nested loop context (parent loop exists),
                // make this loop's end block jump to the parent loop's continue target
                if builder.loop_stack.len() > 1 {
                    // Get parent loop's continue target (before we exit current loop)
                    if let Some(parent_loop) = builder.loop_stack.get(builder.loop_stack.len() - 2)
                    {
                        let parent_continue = parent_loop.continue_target.clone();

                        // Find this loop's end block (should be the last block added with no terminator)
                        if let Some(end_block) = current_func
                            .blocks
                            .iter_mut()
                            .rev()
                            .find(|b| b.terminator.is_none())
                        {
                            end_block.terminator = Some(MirInstr::Jump {
                                target: parent_continue,
                            });
                        }
                    }
                }
            }

            builder.exit_loop(); // Important: exit loop context

            // Create a fresh block for subsequent statements (don't reuse the pushed block)
            let continuation_label = builder.next_block();

            // Connect the loop exit block to the continuation block
            if let Some(current_func) = builder.program.functions.last_mut() {
                // Find the loop exit block (should be the last block with no terminator)
                for exit_block in current_func.blocks.iter_mut().rev() {
                    if exit_block.terminator.is_none() {
                        exit_block.terminator = Some(MirInstr::Jump {
                            target: continuation_label.clone(),
                        });
                        break;
                    }
                }
            }

            *block = MirBlock {
                label: continuation_label,
                instrs: vec![],
                terminator: None,
            };
        }

        // Handle Ok expression (implicit return with success value)
        AstNode::OkExpr { values } => {
            // Build MIR for each value expression
            let value_tmps: Vec<String> = values
                .iter()
                .map(|v| build_expression(builder, v, block))
                .collect();

            // Create a Result Ok instruction
            let result_tmp = builder.next_tmp();
            block.instrs.push(MirInstr::ResultOk {
                name: result_tmp.clone(),
                values: value_tmps,
            });

            // Set terminator to return the Ok result
            block.terminator = Some(MirInstr::Return {
                values: vec![result_tmp],
            });
        }

        // Handle Err expression (implicit return with error value)
        AstNode::ErrExpr { value } => {
            // Build MIR for the error value
            let error_tmp = build_expression(builder, value, block);

            // Create a Result Err instruction
            let result_tmp = builder.next_tmp();
            block.instrs.push(MirInstr::ResultErr {
                name: result_tmp.clone(),
                error: error_tmp,
            });

            // Set terminator to return the Err result
            block.terminator = Some(MirInstr::Return {
                values: vec![result_tmp],
            });
        }

        // Handle expression statements (expressions used as statements with ;)
        // This includes TryPropagate (?), function calls, etc.
        _ => {
            // If this is an expression node, build it to generate its side effects
            // This is critical for ? operator used as a statement: CheckPositive(x)?;
            // The expression will be built and the TryPropagate instruction will be added
            build_expression(builder, stmt, block);
        }
    }
}
