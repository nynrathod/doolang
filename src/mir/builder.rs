use crate::limits::MIR_MAX_DEPTH;
use crate::mir::declarations::{build_function_decl, build_let_decl, build_nested_collection};
use crate::mir::{
    expresssions::build_expression, statements::build_statement, MirBlock, MirFunction, MirInstr,
    MirProgram,
};
use crate::parser::ast::{AstNode, Pattern};
use std::collections::HashSet;
use std::mem::discriminant;

/// This struct is responsible for translating parsed AST nodes into MIR instructions.
/// It manages temporary variable generation, block labeling, loop context for break/continue,
/// and reference counting for memory management.
pub struct MirBuilder {
    pub program: MirProgram,  // Holds all MIR functions and global instructions
    pub tmp_counter: usize,   // For generating unique temporary variable names
    pub block_counter: usize, // For generating unique block labels
    pub loop_stack: Vec<LoopContext>, // Stack for nested loop break/continue targets
    pub rc_tracked_vars: Vec<Vec<String>>, // Stack of scopes with reference-counted variables
    pub mir_symbol_table: std::collections::HashMap<String, crate::parser::ast::TypeNode>, // Track variable types for MIR
    pub recursion_depth: usize, // Track recursion depth to prevent stack overflow
}

/// Context for tracking loop break/continue targets
/// Used to resolve break/continue statements inside nested loops.
#[derive(Debug, Clone)]
pub struct LoopContext {
    pub break_target: String,    // Where break jumps to
    pub continue_target: String, // Where continue jumps to
}

impl MirBuilder {
    /// Create a new MIR builder with empty program and counters initialized.
    pub fn new() -> Self {
        Self {
            program: MirProgram {
                functions: vec![],
                globals: vec![],
                is_main_entry: true, // Default to true; can be set to false for imported modules
            },
            tmp_counter: 1,
            block_counter: 0,
            loop_stack: vec![],
            rc_tracked_vars: vec![vec![]],
            mir_symbol_table: std::collections::HashMap::new(),
            recursion_depth: 0,
        }
    }

    /// Check if we've exceeded maximum recursion depth
    pub fn check_depth(&self) -> Result<(), String> {
        if self.recursion_depth >= MIR_MAX_DEPTH {
            return Err("Expression too deeply nested (recursion limit exceeded)".to_string());
        }
        Ok(())
    }

    /// Generate a unique temporary variable name for MIR instructions.
    /// Used for intermediate results and SSA-style variables.
    pub fn set_is_main_entry(&mut self, is_main: bool) {
        self.program.is_main_entry = is_main;
    }

    pub fn next_tmp(&mut self) -> String {
        let tmp = format!("%{}", self.tmp_counter);
        self.tmp_counter += 1;
        tmp
    }

    /// Generate a unique basic block label for MIR control flow.
    pub fn next_block(&mut self) -> String {
        let label = format!("Block{}", self.block_counter);
        self.block_counter += 1;
        label
    }

    /// Enter a new loop context, pushing break/continue targets onto the stack.
    /// Used to resolve break/continue statements inside nested loops.
    pub fn enter_loop(&mut self, break_target: String, continue_target: String) {
        self.loop_stack.push(LoopContext {
            break_target,
            continue_target,
        });
    }

    /// Exit the current loop context, popping it from the stack.
    pub fn exit_loop(&mut self) {
        self.loop_stack.pop();
    }

    /// Get the current loop context, if any.
    /// Returns the top of the loop stack.
    pub fn current_loop(&self) -> Option<&LoopContext> {
        self.loop_stack.last()
    }

    /// Enter a new reference-counted variable scope.
    /// Used to track which variables need DecRef when leaving scope.
    pub fn enter_scope(&mut self) {
        self.rc_tracked_vars.push(vec![]);
    }

    /// Exit the current scope, emitting DecRef instructions for all tracked variables.
    /// Ensures proper memory management for reference-counted objects.
    pub fn exit_scope(&mut self, block: &mut MirBlock) {
        if let Some(scope_vars) = self.rc_tracked_vars.pop() {
            // Insert DecRef for all RC'd variables in this scope (in reverse order)
            for var in scope_vars.iter().rev() {
                block.instrs.push(MirInstr::DecRef { value: var.clone() });
            }
        }
    }

    /// Track a variable as reference-counted in the current scope.
    /// Used for arrays, maps, strings, etc.
    pub fn track_rc_var(&mut self, var: String) {
        if let Some(current_scope) = self.rc_tracked_vars.last_mut() {
            current_scope.push(var);
        }
    }

    /// Check if a variable is currently tracked for reference counting.
    pub fn is_rc_tracked(&self, var: &str) -> bool {
        self.rc_tracked_vars
            .iter()
            .any(|scope| scope.contains(&var.to_string()))
    }

    /// Build the MIR program from a list of AST nodes.
    /// This is the main entry point for converting parsed code into MIR.
    /// Handles functions, globals, structs, enums, assignments, prints, loops, conditionals, and expressions.
    pub fn build_program(&mut self, nodes: &[AstNode]) {
        for node in nodes {
            match node {
                // Declarations
                AstNode::LetDecl { .. } => {
                    let instrs = build_let_decl(self, node);
                    self.program.globals.extend(instrs);
                }
                AstNode::FunctionDecl { .. } => {
                    build_function_decl(self, node);
                }

                // Import statement - skip in MIR (already handled by analyzer)
                // The analyzer has already loaded imported functions into the function table
                // and will process them when analyzing the importing module
                AstNode::Import { .. } => {
                    // No MIR generation needed - imports are resolved at analysis time
                    continue;
                }

                // Handle struct declarations (type definitions, not instances).
                AstNode::StructDecl {
                    name,
                    fields,
                    is_public,
                } => {
                    // Extract field names and types from the StructField metadata
                    let field_names: Vec<String> =
                        fields.iter().map(|field| field.name.clone()).collect();

                    let field_types: Vec<String> = fields
                        .iter()
                        .map(|field| field.field_type.format_type_string())
                        .collect();

                    // Emit StructDecl with complete type information
                    self.program.globals.push(MirInstr::StructDecl {
                        struct_name: name.clone(),
                        field_names,
                        field_types,
                    });
                }

                // Statements
                AstNode::StringLiteral(value) => {
                    let tmp = self.next_tmp();
                    self.program.globals.push(MirInstr::ConstString {
                        name: tmp.clone(),
                        value: value.clone(),
                    });

                    // Optionally assign it to a global variable.
                    // This is useful if you want to reference or reuse the string elsewhere in your program.
                    // If you don't need a named global reference, you can skip this assignment.
                    self.program.globals.push(MirInstr::Assign {
                        name: format!("global_str_{}", self.tmp_counter - 1),
                        value: tmp,
                        mutable: false,
                    });
                }

                AstNode::EnumDecl {
                    name,
                    variants,
                    is_public,
                } => {
                    for variant in variants {
                        let variant_name = &variant.name;
                        let opt_type = &variant.payload;
                        let tmp = self.next_tmp();
                        let value_tmp = if opt_type.is_some() {
                            Some(self.next_tmp())
                        } else {
                            None
                        };

                        self.program.globals.push(MirInstr::EnumInit {
                            name: tmp.clone(),
                            enum_name: name.clone(),
                            variant: variant_name.clone(),
                            value: value_tmp,
                        });

                        self.program.globals.push(MirInstr::Assign {
                            name: format!("global_enum_{}_{}", name, variant_name),
                            value: tmp,
                            mutable: false,
                        });
                    }
                }

                // Handle global assignments (outside functions).
                AstNode::Assignment { pattern, value } => {
                    let mut temp_block = MirBlock {
                        label: "temp".to_string(),
                        instrs: vec![],
                        terminator: None,
                    };

                    let value_tmp = build_expression(self, value, &mut temp_block);
                    self.program.globals.extend(temp_block.instrs);
                    // Only handle simple identifier patterns for globals.
                    if let Pattern::Identifier(name) = pattern {
                        self.program.globals.push(MirInstr::Assign {
                            name: name.clone(),
                            value: value_tmp,
                            mutable: true,
                        });
                    }
                }

                // Handle global element assignments (e.g., arr[0] = 5).
                AstNode::ElementAssignment {
                    array,
                    index,
                    value,
                } => {
                    let mut temp_block = MirBlock {
                        label: "temp".to_string(),
                        instrs: vec![],
                        terminator: None,
                    };

                    let _array_tmp = build_expression(self, array, &mut temp_block);
                    let index_tmp = build_expression(self, index, &mut temp_block);
                    let value_tmp = build_expression(self, value, &mut temp_block);
                    self.program.globals.extend(temp_block.instrs);

                    if let AstNode::Identifier(array_name) = &**array {
                        self.program.globals.push(MirInstr::ArraySet {
                            array: array_name.clone(),
                            index: index_tmp,
                            value: value_tmp,
                        });
                    }
                }

                AstNode::Print { exprs } => {
                    let mut temp_block = MirBlock {
                        label: "temp".to_string(),
                        instrs: vec![],
                        terminator: None,
                    };

                    let mut print_vals = vec![];
                    for expr in exprs {
                        let val_tmp = build_expression(self, expr, &mut temp_block);
                        print_vals.push(val_tmp);
                    }

                    self.program.globals.extend(temp_block.instrs);
                    self.program
                        .globals
                        .push(MirInstr::Print { values: print_vals });
                }

                AstNode::ConditionalStmt { .. } => {
                    // Wrap the if statement in a temporary function for isolation.
                    let if_func_name = self.create_temp_function("if");
                    let mut temp_func = MirFunction {
                        name: if_func_name.clone(),
                        params: vec![],
                        param_types: vec![],
                        return_type: None,
                        error_type: None,
                        blocks: vec![],
                    };

                    let block_label = self.next_block();
                    let mut block = MirBlock {
                        label: block_label,
                        instrs: vec![],
                        terminator: None,
                    };
                    build_statement(self, node, &mut block);
                    temp_func.blocks.push(block);
                    self.program.functions.push(temp_func);

                    let call_tmp = self.next_tmp();
                    self.program.globals.push(MirInstr::Call {
                        dest: vec![call_tmp],
                        func: if_func_name,
                        args: vec![],
                    });
                }

                // Handle for loops at global level (rare but possible).
                AstNode::ForLoopStmt { .. } => {
                    // Wrap the loop in a temporary function for isolation.
                    let loop_func_name = self.create_temp_function("loop");
                    let mut temp_func = MirFunction {
                        name: loop_func_name.clone(),
                        params: vec![],
                        param_types: vec![],
                        return_type: None,
                        error_type: None,
                        blocks: vec![],
                    };

                    let block_label = self.next_block();

                    let mut block = MirBlock {
                        label: block_label,
                        instrs: vec![],
                        terminator: None,
                    };

                    // Build the for loop in the temporary function.
                    build_statement(self, node, &mut block);
                    temp_func.blocks.push(block);
                    self.program.functions.push(temp_func);
                    let call_tmp = self.next_tmp();
                    self.program.globals.push(MirInstr::Call {
                        dest: vec![call_tmp],
                        func: loop_func_name,
                        args: vec![],
                    });
                }

                AstNode::BinaryExpr { .. } | AstNode::FunctionCall { .. } => {
                    let mut temp_block = MirBlock {
                        label: "temp".to_string(),
                        instrs: vec![],
                        terminator: None,
                    };

                    let expr_tmp = build_expression(self, node, &mut temp_block);
                    self.program.globals.extend(temp_block.instrs);
                    let global_name = format!("__global_expr_{}", self.tmp_counter - 1);
                    self.program.globals.push(MirInstr::Assign {
                        name: global_name,
                        value: expr_tmp,
                        mutable: false,
                    });
                }

                AstNode::ArrayLiteral { .. } | AstNode::MapLiteral { .. } => {
                    let mut temp_block = MirBlock {
                        label: "temp".to_string(),
                        instrs: vec![],
                        terminator: None,
                    };

                    // Nested is not supported in codegen yet
                    // TODO: Implement nested collection support in codegen
                    let collection_tmp = build_nested_collection(self, node, &mut temp_block);
                    self.program.globals.extend(temp_block.instrs);
                    let global_name = format!("__global_collection_{}", self.tmp_counter - 1);
                    self.program.globals.push(MirInstr::Assign {
                        name: global_name,
                        value: collection_tmp,
                        mutable: false,
                    });
                }

                _ => {
                    // For unhandled node types, create a placeholder and print a warning.
                    println!(
                        "Warning: Unhandled AST node type in global scope: {:?}",
                        discriminant(node)
                    );
                }
            }
        }

        // Emit DecRef instructions for all reference-counted variables in the global scope.
        let global_scope_vars = self.rc_tracked_vars.last().cloned().unwrap_or_default();
        for var in global_scope_vars.iter().rev() {
            self.program
                .globals
                .push(MirInstr::DecRef { value: var.clone() });
        }
    }

    /// Helper method to create a temporary function for complex global constructs.
    /// Used for wrapping loops and conditionals at global scope.
    fn create_temp_function(&mut self, name_prefix: &str) -> String {
        let func_name = format!(
            "__{}_{}_{}",
            name_prefix, self.block_counter, self.tmp_counter
        );
        func_name
    }

    /// Finalize the MIR program: clean up and lightweight optimizations.
    /// - Removes empty blocks (but keeps referenced ones).
    /// - Deduplicates global constants/assignments.
    /// - Optionally merges consecutive assignments to the same target.
    pub fn finalize(&mut self) {
        // 1. Remove empty blocks (blocks without instructions and no terminator)
        //    BUT: keep blocks that are referenced by other blocks
        for func in &mut self.program.functions {
            // collect all referenced block labels
            let mut referenced_blocks = HashSet::new();
            for block in &func.blocks {
                if let Some(term) = &block.terminator {
                    match term {
                        MirInstr::Jump { target } => {
                            referenced_blocks.insert(target.clone());
                        }
                        MirInstr::CondJump {
                            then_block,
                            else_block,
                            ..
                        } => {
                            referenced_blocks.insert(then_block.clone());
                            referenced_blocks.insert(else_block.clone());
                        }
                        _ => {}
                    }
                }
            }

            // Now remove only empty blocks that are NOT referenced
            func.blocks.retain(|block| {
                !block.instrs.is_empty()
                    || block.terminator.is_some()
                    || referenced_blocks.contains(&block.label)
            });
        }

        // 2. Deduplicate global constants / assignments by name
        //    Ensures multiple identical constants are not emitted repeatedly
        let mut seen_assigns: HashSet<String> = HashSet::new();
        self.program.globals.retain(|instr| {
            match instr {
                MirInstr::Assign { name, .. }
                | MirInstr::ConstString { name, .. }
                | MirInstr::StructInit { name, .. }
                | MirInstr::EnumInit { name, .. } => {
                    if seen_assigns.contains(name) {
                        false // already seen, remove duplicate
                    } else {
                        seen_assigns.insert(name.clone());
                        true
                    }
                }
                _ => true,
            }
        });

        // 3. Optional: merge consecutive constant assignments to same target
        //    (if MIR has %tmp1 = const, %tmp1 = const again, keep last)
        let mut last_assigns: HashSet<String> = HashSet::new();
        self.program.globals.retain(|instr| {
            if let MirInstr::Assign { name, .. } = instr {
                if last_assigns.contains(name) {
                    false
                } else {
                    last_assigns.insert(name.clone());

                    true
                }
            } else {
                true
            }
        });
    }
}
