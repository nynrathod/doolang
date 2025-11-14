use crate::mir::builder::MirBuilder;
use crate::mir::expresssions::build_expression;
use crate::mir::statements::build_statement;
use crate::mir::{MirBlock, MirFunction, MirInstr};
use crate::parser::ast::TypeNode;
use crate::parser::ast::{AstNode, Pattern};

/// Build MIR instructions for a variable declaration (`let` statement).
/// - Handles single variable and tuple destructuring patterns.
/// - Evaluates the right-hand side expression and assigns it to the variable(s).
/// - Inserts reference counting instructions for heap-allocated types (strings, arrays, maps).
pub fn build_let_decl(builder: &mut MirBuilder, node: &AstNode) -> Vec<MirInstr> {
    if let AstNode::LetDecl {
        pattern,
        value,
        mutable,
        type_annotation,
        is_ref_counted,
        ..
    } = node
    {
        let mut instrs = vec![];
        // Create a temporary block to evaluate the right-hand side expression.
        let mut temp_block = MirBlock {
            label: "temp".to_string(),
            instrs: vec![],
            terminator: None,
        };

        // Build MIR for the value expression.
        let value_tmp = build_expression(builder, value, &mut temp_block);

        // Add the expression evaluation instructions to our result.
        instrs.extend(temp_block.instrs);

        // Determine if reference counting is needed for this variable.
        let needs_rc = match type_annotation {
            Some(TypeNode::String) => true,
            Some(TypeNode::Array(_)) => true,
            Some(TypeNode::Map(_, _)) => true,
            _ => false,
        };

        // Check if value_tmp is a simple variable identifier (not a temp or literal).
        // We only need to incref when COPYING from an existing variable.
        // Temps starting with '%' are newly created values (from ConstString, Array, Map, etc.)
        // that already have RC=1, so we shouldn't incref them.
        let is_copying_variable = !value_tmp.starts_with('%')
            && !value_tmp.parse::<i32>().is_ok()
            && value_tmp != "true"
            && value_tmp != "false";

        // Handle different binding patterns for the left-hand side.
        match pattern {
            Pattern::Identifier(name) => {
                instrs.push(MirInstr::Assign {
                    name: name.clone(),
                    value: value_tmp.clone(),
                    mutable: *mutable,
                });

                // Track variable type in mir_symbol_table
                // Copy type from value_tmp if available, or use type_annotation
                if let Some(value_type) = builder.mir_symbol_table.get(&value_tmp).cloned() {
                    builder.mir_symbol_table.insert(name.clone(), value_type);
                } else if let Some(type_ann) = type_annotation {
                    builder
                        .mir_symbol_table
                        .insert(name.clone(), type_ann.clone());
                }

                // Insert IncRef ONLY when copying from an existing variable.
                // Don't incref for newly created temps (they already have RC=1).
                if needs_rc && is_copying_variable {
                    instrs.push(MirInstr::IncRef {
                        value: name.clone(),
                    });
                }

                // Always track RC variables for cleanup at scope end
                if needs_rc {
                    builder.track_rc_var(name.clone());
                }
            }
            Pattern::Tuple(patterns) => {
                for (i, pattern) in patterns.iter().enumerate() {
                    if let Pattern::Identifier(name) = pattern {
                        // Extract each tuple element into a temporary.
                        let extract_tmp = builder.next_tmp();
                        instrs.push(MirInstr::TupleExtract {
                            name: extract_tmp.clone(),
                            source: value_tmp.clone(),
                            index: i,
                        });
                        instrs.push(MirInstr::Assign {
                            name: name.clone(),
                            value: extract_tmp,
                            mutable: *mutable,
                        });

                        // Reference counting for tuple elements if needed.
                        if is_ref_counted.unwrap_or(false) {
                            instrs.push(MirInstr::IncRef {
                                value: name.clone(),
                            });
                        }
                    }
                }
            }
            _ => {
                // Handle other patterns (e.g., struct destructuring) in the future.
            }
        }

        instrs
    } else {
        vec![]
    }
}

/// Build MIR instructions for a function declaration.
/// - Sets up a new MIR function with parameters and return type.
/// - Tracks reference-counted variables in function scope (but NOT parameters).
/// - Maps function arguments to temporaries and assigns them to parameter names.
/// - Builds MIR for each statement in the function body.
/// - Properly connects blocks when loops are present.
/// - Adds DecRef cleanup to the final reachable block only (no duplicates).
/// - Parameters are NOT tracked for RC cleanup since caller owns them.
/// - Adds an implicit return if none is present and the function has no return type.
pub fn build_function_decl(builder: &mut MirBuilder, node: &AstNode) {
    if let AstNode::FunctionDecl {
        name,
        params,
        return_type,
        body,
        ..
    } = node
    {
        let func = MirFunction {
            name: name.clone(),
            params: params.iter().map(|(n, _)| n.clone()).collect(),
            param_types: params
                .iter()
                .map(|(_, t)| t.as_ref().map(|ty| ty.format_type_string()))
                .collect(),
            return_type: return_type.as_ref().map(|t| t.format_type_string()),
            blocks: vec![],
        };

        // Add function to program BEFORE processing body
        // This ensures that when build_statement adds blocks for loops,
        // it adds them to THIS function (via last_mut())
        builder.program.functions.push(func);

        // Create the first block for the function body
        let first_block_label = builder.next_block();
        let mut block = MirBlock {
            label: first_block_label.clone(),
            instrs: vec![],
            terminator: None,
        };

        // Enter function scope for reference counting.
        builder.enter_scope();

        // Track parameter names and types to check if they need RC
        let mut param_rc_types: Vec<(String, bool)> = Vec::new();

        // Parameters are handled directly by codegen (allocated and stored from function args)
        // No need for Arg instructions or intermediate temps
        // Just track which parameters need RC for potential future use
        for (param_name, param_type) in params {
            // Check if parameter is RC type (String, Array, Map)
            let is_rc = match param_type {
                Some(TypeNode::String) => true,
                Some(TypeNode::Array(_)) => true,
                Some(TypeNode::Map(_, _)) => true,
                _ => false,
            };

            param_rc_types.push((param_name.clone(), is_rc));

            // Track parameter types in mir_symbol_table
            if let Some(ptype) = param_type {
                builder
                    .mir_symbol_table
                    .insert(param_name.clone(), ptype.clone());
            }

            // DO NOT track parameters as RC variables for cleanup
            // Parameters are owned by the caller, not by this function
            // The function borrows them, and caller handles cleanup
        }

        // Build MIR for each statement in the function body.
        for stmt in body {
            let old_label = block.label.clone();

            // Add the current block BEFORE processing the statement
            // This ensures proper block ordering when statements (like loops with if inside)
            // add their own blocks to the function
            let should_add_block_before = !block.instrs.is_empty() || block.terminator.is_some();
            if should_add_block_before {
                if let Some(current_func) = builder.program.functions.last_mut() {
                    current_func.blocks.push(block.clone());
                }

                // Create a new block for the next statement
                let next_label = builder.next_block();
                let old_block = block.clone();
                block = MirBlock {
                    label: next_label.clone(),
                    instrs: vec![],
                    terminator: None,
                };

                // Connect previous block to this new block if it doesn't have a terminator
                if old_block.terminator.is_none() {
                    if let Some(current_func) = builder.program.functions.last_mut() {
                        if let Some(prev_block) = current_func.blocks.last_mut() {
                            if prev_block.label == old_block.label {
                                prev_block.terminator = Some(MirInstr::Jump {
                                    target: next_label.clone(),
                                });
                            }
                        }
                    }
                }
            }

            build_statement(builder, stmt, &mut block);

            // If the statement set a terminator (like a for-loop), subsequent statements
            // need a new block to avoid adding instructions after the terminator
            if block.terminator.is_some() {
                let block_was_updated = block.label != old_label;

                if !block_was_updated {
                    // Add the current block to the function
                    if let Some(current_func) = builder.program.functions.last_mut() {
                        current_func.blocks.push(block.clone());
                    }

                    // Create a new block for the next statement
                    let next_label = builder.next_block();
                    block = MirBlock {
                        label: next_label.clone(),
                        instrs: vec![],
                        terminator: None,
                    };

                    // Connect the previous loop's exit block to this new continuation block
                    if let Some(current_func) = builder.program.functions.last_mut() {
                        for prev_block in current_func.blocks.iter_mut().rev() {
                            if prev_block.terminator.is_none() && prev_block.label != next_label {
                                prev_block.terminator = Some(MirInstr::Jump { target: next_label });
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Add the final block if it has content or a terminator
        if !block.instrs.is_empty() || block.terminator.is_some() {
            if let Some(current_func) = builder.program.functions.last_mut() {
                current_func.blocks.push(block.clone());
            }
        }

        // Get cleanup instructions from exit_scope
        let mut temp_block = MirBlock {
            label: "temp_cleanup".to_string(),
            instrs: vec![],
            terminator: None,
        };
        builder.exit_scope(&mut temp_block);
        let decref_instrs = temp_block.instrs;

        // Check if function has multiple blocks (loops exist)
        let has_multiple_blocks = if let Some(func) = builder.program.functions.last() {
            func.blocks.len() > 1
        } else {
            false
        };

        // Add cleanup to the appropriate blocks
        if let Some(func) = builder.program.functions.last_mut() {
            if has_multiple_blocks {
                // Multiple blocks exist (loops are present)
                // Add cleanup and return to ALL blocks without terminators
                let blocks_needing_cleanup: Vec<String> = func
                    .blocks
                    .iter()
                    .filter(|b| b.terminator.is_none())
                    .map(|b| b.label.clone())
                    .collect();

                for block_label in blocks_needing_cleanup {
                    if let Some(final_block) =
                        func.blocks.iter_mut().find(|b| b.label == block_label)
                    {
                        // Add decrefs to this block
                        for decref_instr in &decref_instrs {
                            final_block.instrs.push(decref_instr.clone());
                        }

                        // Only add return if function is void
                        if return_type.is_none() {
                            final_block.terminator = Some(MirInstr::Return { values: vec![] });
                        }
                    }
                }
            } else {
                // Single block (no loops) - add cleanup to the only block
                if let Some(entry_block) = func.blocks.first_mut() {
                    // Add decrefs to entry block
                    for decref_instr in decref_instrs {
                        entry_block.instrs.push(decref_instr);
                    }

                    // Add return if needed
                    if return_type.is_none() && entry_block.terminator.is_none() {
                        entry_block.terminator = Some(MirInstr::Return { values: vec![] });
                    }
                }
            }
        }
    } else {
        debug_assert!(
            false,
            "Expected FunctionDecl node - should be guaranteed by caller"
        );
    }
}

/// Helper function to build MIR instructions for nested collections.
/// NOTE: Nested collections are NOT supported for production.
/// This function exists for future extension but should not be used.
/// Regular arrays and maps work fine, but nested structures (array of arrays, etc.) are not implemented.
#[allow(dead_code)]
pub fn build_nested_collection(
    builder: &mut MirBuilder,
    expr: &AstNode,
    block: &mut MirBlock,
) -> String {
    // For now, just fall back to regular expression building
    // Nested collections are not supported
    build_expression(builder, expr, block)
}
