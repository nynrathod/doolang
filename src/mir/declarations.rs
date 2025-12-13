use crate::mir::builder::MirBuilder;
use crate::mir::expresssions::build_expression;
use crate::mir::statements::build_statement;
use crate::mir::{MirBlock, MirFunction, MirInstr};
use crate::parser::ast::TypeNode;
use crate::parser::ast::{AstNode, Pattern};
use std::collections::{HashMap, HashSet, VecDeque};

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

        // CRITICAL FIX: If this is an empty array literal and we have a type annotation,
        // update the MirInstr::Array to use the correct element type from the annotation.
        // This ensures `let tasks: [Task] = []` creates an array with element_type="Task", not "Int".
        if let Some(TypeNode::Array(elem_type)) = type_annotation {
            // Find the Array instruction for value_tmp and update its element_type
            for instr in temp_block.instrs.iter_mut() {
                if let MirInstr::Array {
                    name,
                    elements,
                    element_type,
                } = instr
                {
                    if name == &value_tmp && elements.is_empty() {
                        // Update element_type from type annotation
                        let new_elem_type = match elem_type.as_ref() {
                            TypeNode::Int => Some("Int".to_string()),
                            TypeNode::Float => Some("Float".to_string()),
                            TypeNode::Bool => Some("Bool".to_string()),
                            TypeNode::String => Some("Str".to_string()),
                            TypeNode::TypeRef(struct_name) => Some(struct_name.clone()),
                            _ => None,
                        };
                        if new_elem_type.is_some() {
                            *element_type = new_elem_type;
                            // Also update the mir_symbol_table
                            builder
                                .mir_symbol_table
                                .insert(value_tmp.clone(), TypeNode::Array(elem_type.clone()));
                        }
                    }
                }
            }
        }

        // Add the expression evaluation instructions to our result.
        instrs.extend(temp_block.instrs);

        // Determine if reference counting is needed for this variable.
        // Use is_ref_counted from analyzer (handles inferred types) OR check explicit type annotation
        let needs_rc = is_ref_counted.unwrap_or(false)
            || match type_annotation {
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
        error_type,
        body,
        decorators,
        receiver_type,
        associated_type,
        ..
    } = node
    {
        // If this is a method declaration, use mangled name (Type::method)
        // Use associated_type which is set for both static and instance methods
        let func_name = if let Some(type_name) = associated_type {
            format!("{}::{}", type_name, name)
        } else {
            name.clone()
        };
        // Extract FFI information from decorators
        let mut ffi_lib: Option<String> = None;
        let mut ffi_symbol: Option<String> = None;

        for decorator in decorators {
            match decorator.name.as_str() {
                "ffi" => {
                    // @ffi("libname") - extract library name
                    if let Some(AstNode::StringLiteral(lib_name)) = decorator.args.first() {
                        ffi_lib = Some(lib_name.clone());
                    }
                }
                "extern" => {
                    // @extern("symbol_name") - extract symbol name
                    if let Some(AstNode::StringLiteral(symbol_name)) = decorator.args.first() {
                        ffi_symbol = Some(symbol_name.clone());
                    }
                }
                _ => {}
            }
        }

        // For methods, first parameter is the receiver with inferred type
        // For regular functions, just use all parameters as-is
        // Use receiver_type to check if this is an instance method (has self parameter)
        let (all_params, all_param_types) = if let Some(type_name) = associated_type {
            // This is a method - check if it's an instance method or static method
            let mut method_params: Vec<String> = Vec::new();
            let mut method_param_types: Vec<Option<String>> = Vec::new();

            // If receiver_type is Some, this is an instance method with 'self' parameter
            if receiver_type.is_some() {
                // Add first parameter (receiver) with inferred type
                if let Some((receiver_name, _)) = params.first() {
                    method_params.push(receiver_name.clone());

                    // Determine the receiver type
                    let receiver_type_string = match type_name.as_str() {
                        "Int" => Some("Int".to_string()),
                        "Float" => Some("Float".to_string()),
                        "Str" => Some("Str".to_string()),
                        "Bool" => Some("Bool".to_string()),
                        other => Some(other.to_string()),
                    };
                    method_param_types.push(receiver_type_string);
                }

                // Add remaining parameters
                method_params.extend(params.iter().skip(1).map(|(n, _)| n.clone()));
                method_param_types.extend(
                    params
                        .iter()
                        .skip(1)
                        .map(|(_, t)| t.as_ref().map(|ty| ty.format_type_string())),
                );
            } else {
                // Static method: include all parameters as-is
                method_params.extend(params.iter().map(|(n, _)| n.clone()));
                method_param_types.extend(
                    params
                        .iter()
                        .map(|(_, t)| t.as_ref().map(|ty| ty.format_type_string())),
                );
            }

            (method_params, method_param_types)
        } else {
            // Regular function - use all parameters
            let func_params: Vec<String> = params.iter().map(|(n, _)| n.clone()).collect();
            let func_param_types: Vec<Option<String>> = params
                .iter()
                .map(|(_, t)| t.as_ref().map(|ty| ty.format_type_string()))
                .collect();

            (func_params, func_param_types)
        };

        let func = MirFunction {
            name: func_name.clone(),
            params: all_params,
            param_types: all_param_types,
            return_type: return_type.as_ref().map(|t| t.format_type_string()),
            error_type: error_type.as_ref().map(|t| t.format_type_string()),
            blocks: vec![],
            ffi_lib,
            ffi_symbol,
        };

        // Add function to program BEFORE processing body
        // This ensures that when build_statement adds blocks for loops,
        // it adds them to THIS function (via last_mut())
        builder.program.functions.push(func);

        // Track if current function has error type for Ok/Err handling
        let prev_error_type = builder.current_function_error_type.clone();
        builder.current_function_error_type = error_type.clone();

        // Set current_struct_name if this is a method (has receiver_type)
        // This is critical for resolving field types in method bodies
        let prev_struct_name = builder.current_struct_name.clone();
        if let Some(type_name) = receiver_type {
            builder.current_struct_name = Some(type_name.clone());
        }

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

        // Handle receiver parameter if method (first param)
        // Process all parameters from the function signature
        let params_to_check = if receiver_type.is_some() {
            // Method: first param is receiver, doesn't need RC tracking
            if let Some((receiver_name, _)) = params.first() {
                param_rc_types.push((receiver_name.clone(), false));
            }
            params.iter().skip(1)
        } else {
            params.iter().skip(0)
        };

        for (param_name, param_type) in params_to_check {
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
        // Track last expression result for implicit return
        let mut last_expr_result: Option<String> = None;
        let body_len = body.len();

        for (stmt_idx, stmt) in body.iter().enumerate() {
            let is_last_stmt = stmt_idx == body_len - 1;
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
                                    label: next_label.clone(),
                                });
                            }
                        }
                    }
                }
            }

            // For the last statement in a function with return type,
            // if it's a match expression, capture the result for implicit return
            if is_last_stmt && return_type.is_some() {
                if let AstNode::MatchExpr { .. } = stmt {
                    let result_tmp = build_expression(builder, stmt, &mut block);
                    last_expr_result = Some(result_tmp);
                } else {
                    build_statement(builder, stmt, &mut block);
                }
            } else {
                build_statement(builder, stmt, &mut block);
            }

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
                                prev_block.terminator = Some(MirInstr::Jump { label: next_label });
                                break;
                            }
                        }
                    }
                }
            }
        }

        // If we have an implicit return from match expression, add Return terminator
        if let Some(result_tmp) = last_expr_result {
            if block.terminator.is_none() {
                block.terminator = Some(MirInstr::Return {
                    values: vec![result_tmp],
                });
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

        // Reorder blocks to ensure proper control flow
        // This fixes issues with nested if/else where blocks are added in wrong order
        if let Some(func) = builder.program.functions.last_mut() {
            reorder_blocks(func);
        }

        // Restore previous error type (for nested function handling if any)
        builder.current_function_error_type = prev_error_type;

        // Restore previous struct name (for nested function handling if any)
        builder.current_struct_name = prev_struct_name;
    } else {
        debug_assert!(
            false,
            "Expected FunctionDecl node - should be guaranteed by caller"
        );
    }
}

/// Reorder MIR blocks to ensure proper control flow.
/// Uses BFS to order blocks in the order they are reachable from Block0 (the true entry point).
fn reorder_blocks(func: &mut MirFunction) {
    if func.blocks.is_empty() {
        return;
    }

    // Build adjacency list of block transitions
    let mut successors: HashMap<String, Vec<String>> = HashMap::new();

    for block in &func.blocks {
        let mut succs = Vec::new();

        // Check terminator for successors
        if let Some(term) = &block.terminator {
            match term {
                MirInstr::Jump { label } => {
                    succs.push(label.clone());
                }
                MirInstr::CondJump {
                    then_block,
                    else_block,
                    ..
                } => {
                    succs.push(then_block.clone());
                    succs.push(else_block.clone());
                }
                MirInstr::Return { .. } => {
                    // No successors
                }
                _ => {}
            }
        }

        successors.insert(block.label.clone(), succs);
    }

    // Find the actual first block (Block0) - not the first in the list
    // The first block is the one created by build_function_decl, which should be Block0
    let mut first_label = None;

    // Look for Block0 specifically
    for block in &func.blocks {
        if block.label == "Block0" {
            first_label = Some(block.label.clone());
            break;
        }
    }

    // If Block0 doesn't exist, try to find the lowest numbered block
    if first_label.is_none() {
        let mut min_num = usize::MAX;
        for block in &func.blocks {
            if let Some(num_str) = block.label.strip_prefix("Block") {
                if let Ok(num) = num_str.parse::<usize>() {
                    if num < min_num {
                        min_num = num;
                        first_label = Some(block.label.clone());
                    }
                }
            }
        }
    }

    // If still no first label found, use the first block in the list
    if first_label.is_none() {
        first_label = Some(func.blocks[0].label.clone());
    }

    let first_label = first_label.unwrap();

    // BFS from the actual first block to determine reachability order
    let mut visited = HashSet::new();
    let mut ordered_labels = Vec::new();
    let mut queue = VecDeque::new();

    queue.push_back(first_label.clone());
    visited.insert(first_label.clone());

    while let Some(label) = queue.pop_front() {
        ordered_labels.push(label.clone());

        if let Some(succs) = successors.get(&label) {
            for succ in succs {
                if !visited.contains(succ) {
                    visited.insert(succ.clone());
                    queue.push_back(succ.clone());
                }
            }
        }
    }

    // Add any unreachable blocks at the end (shouldn't happen, but be safe)
    for block in &func.blocks {
        if !visited.contains(&block.label) {
            ordered_labels.push(block.label.clone());
        }
    }

    // Create a map of label -> block for quick lookup
    let block_map: HashMap<String, MirBlock> = func
        .blocks
        .iter()
        .map(|b| (b.label.clone(), b.clone()))
        .collect();

    // Rebuild blocks vector in the correct order
    func.blocks = ordered_labels
        .into_iter()
        .filter_map(|label| block_map.get(&label).cloned())
        .collect();
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
