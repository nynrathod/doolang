//! For loop code generation with automatic reference counting and memory management.
//! This module handles all types of for loops in the language:
//! - Range loops: for i in 0..10, for i in 0..=10
//! - Array loops: for item in arr
//! - Map loops: for (key, value) in map
//! - Infinite loops: for { }
//! - Break and continue statements with proper cleanup

use crate::codegen::core::CodeGen;
use crate::mir::mir::MirInstr;
use inkwell::basic_block::BasicBlock;
use inkwell::IntPredicate;
use std::collections::HashMap;

impl<'ctx> CodeGen<'ctx> {
    /// Main entry point for generating all types of for loops
    /// Handles range, array, map, and infinite loops with proper RC management
    pub fn generate_for_loop(
        &mut self,
        instr: &MirInstr,
        bb_map: &HashMap<String, BasicBlock<'ctx>>,
    ) {
        crate::doo_codegen_debug!("generate_for_loop: {:?}", std::mem::discriminant(instr));
        match instr {
            MirInstr::ForRange {
                var,
                start,
                end,
                inclusive,
                cond_block: _,
                body_block,
                exit_block,
            } => {
                crate::doo_codegen_debug!("ForRange: var={} start={} end={} inclusive={}", var, start, end, inclusive);
                self.generate_for_range(
                    var, start, end, *inclusive, body_block, exit_block, bb_map,
                );
            }
            MirInstr::ForArray {
                var,
                array,
                index_var,
                cond_block: _,
                body_block,
                exit_block,
            } => {
                crate::doo_codegen_debug!("ForArray: var={} array={} index_var={}", var, array, index_var);
                self.generate_for_array(var, array, index_var, body_block, exit_block, bb_map);
            }
            MirInstr::ForMap {
                key_var,
                value_var,
                map,
                index_var,
                cond_block: _,
                body_block,
                exit_block,
            } => {
                crate::doo_codegen_debug!("ForMap: key_var={} value_var={} map={} index_var={}", key_var, value_var, map, index_var);
                self.generate_for_map(
                    key_var, value_var, map, index_var, body_block, exit_block, bb_map,
                );
            }
            MirInstr::ForInfinite { body_block } => {
                crate::doo_codegen_debug!("ForInfinite: body_block={}", body_block);
                self.generate_for_infinite(body_block, bb_map);
            }
            MirInstr::Break { target } => {
                crate::doo_codegen_debug!("Break: target={}", target);
                self.generate_break(target, bb_map);
            }
            MirInstr::Continue { target } => {
                crate::doo_codegen_debug!("Continue: target={}", target);
                self.generate_continue(target, bb_map);
            }
            _ => {}
        }
    }

    /// Generate range-based for loop: for i in 0..10 or for i in 0..=10
    /// Creates proper loop structure with RC-safe variable management
    fn generate_for_range(
        &mut self,
        var: &str,
        start: &str,
        end: &str,
        inclusive: bool,
        body_block: &str,
        exit_block: &str,
        bb_map: &HashMap<String, BasicBlock<'ctx>>,
    ) {
        // Create condition block
        let current_func = self
            .builder
            .get_insert_block()
            .unwrap()
            .get_parent()
            .unwrap();
        let cond_block = self
            .context
            .append_basic_block(current_func, "for.range.cond");

        // Initialize loop variable
        let start_val = self.resolve_value(start).into_int_value();
        let loop_var_alloca = self
            .builder
            .build_alloca(self.context.i32_type(), var)
            .unwrap();
        self.builder
            .build_store(loop_var_alloca, start_val)
            .unwrap();

        // Register loop variable in symbol table
        self.symbols.insert(
            var.to_string(),
            crate::codegen::Symbol {
                ptr: loop_var_alloca,
                ty: self.context.i32_type().into(),
            },
        );

        // Enter loop context for break/continue and cleanup tracking
        self.enter_loop(exit_block.to_string(), "for.range.cond".to_string());

        // Jump to condition check
        self.builder.build_unconditional_branch(cond_block).unwrap();

        // Generate condition block
        self.builder.position_at_end(cond_block);
        let current_val = self
            .builder
            .build_load(self.context.i32_type(), loop_var_alloca, "current")
            .unwrap()
            .into_int_value();

        let end_val = self.resolve_value(end).into_int_value();
        let condition = if inclusive {
            // i <= end
            self.builder
                .build_int_compare(IntPredicate::SLE, current_val, end_val, "cond")
                .unwrap()
        } else {
            // i < end
            self.builder
                .build_int_compare(IntPredicate::SLT, current_val, end_val, "cond")
                .unwrap()
        };

        // Get or create body and exit blocks
        let body_bb = if let Some(bb) = bb_map.get(body_block) {
            *bb
        } else {
            self.context.append_basic_block(current_func, body_block)
        };

        let exit_bb = if let Some(bb) = bb_map.get(exit_block) {
            *bb
        } else {
            self.context.append_basic_block(current_func, exit_block)
        };

        // Create cleanup block for natural loop exit
        let cleanup_bb = self
            .context
            .append_basic_block(current_func, "for.range.cleanup");

        // Build conditional branch to body or cleanup
        self.builder
            .build_conditional_branch(condition, body_bb, cleanup_bb)
            .unwrap();

        // Generate cleanup block that cleans up and jumps to exit
        self.builder.position_at_end(cleanup_bb);
        self.generate_loop_exit_cleanup();
        self.builder.build_unconditional_branch(exit_bb).unwrap();

        // Position builder at body block start for processing
        self.builder.position_at_end(body_bb);
    }

    /// Generate array iteration: for item in arr
    /// Handles RC for array elements, especially strings
    fn generate_for_array(
        &mut self,
        var: &str,
        array: &str,
        index_var: &str,
        body_block: &str,
        exit_block: &str,
        bb_map: &HashMap<String, BasicBlock<'ctx>>,
    ) {
        // Create condition block
        let current_func = self
            .builder
            .get_insert_block()
            .unwrap()
            .get_parent()
            .unwrap();
        let cond_block = self
            .context
            .append_basic_block(current_func, "for.array.cond");

        // Initialize index variable
        let index_alloca = self
            .builder
            .build_alloca(self.context.i32_type(), index_var)
            .unwrap();
        self.builder
            .build_store(index_alloca, self.context.i32_type().const_zero())
            .unwrap();

        // Register index variable
        self.symbols.insert(
            index_var.to_string(),
            crate::codegen::Symbol {
                ptr: index_alloca,
                ty: self.context.i32_type().into(),
            },
        );

        // Create item variable allocation
        let elem_type = self.get_array_element_type(array);
        let item_alloca = self.builder.build_alloca(elem_type, var).unwrap();

        // Initialize to null for pointer types to prevent decref of uninitialized memory
        if elem_type.is_pointer_type() {
            let null_ptr = self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .const_null();
            self.builder.build_store(item_alloca, null_ptr).unwrap();
        }

        // Register item variable
        self.symbols.insert(
            var.to_string(),
            crate::codegen::Symbol {
                ptr: item_alloca,
                ty: elem_type,
            },
        );

        // Mark as loop-local variable (allocated inside loop body, cannot be cleaned up at function level)
        self.loop_local_vars.insert(var.to_string());

        // Enter loop context for tracking and cleanup
        self.enter_loop(exit_block.to_string(), "for.array.cond".to_string());
        self.add_loop_var(var.to_string());

        // Jump to condition check
        self.builder.build_unconditional_branch(cond_block).unwrap();

        // Generate condition block
        self.builder.position_at_end(cond_block);
        let current_index = self
            .builder
            .build_load(self.context.i32_type(), index_alloca, "current_idx")
            .unwrap()
            .into_int_value();

        let array_len = self.get_array_length(array);
        let condition = self
            .builder
            .build_int_compare(IntPredicate::SLT, current_index, array_len, "arr_cond")
            .unwrap();

        // Get or create body and exit blocks
        let body_bb = if let Some(bb) = bb_map.get(body_block) {
            *bb
        } else {
            self.context.append_basic_block(current_func, body_block)
        };

        let exit_bb = if let Some(bb) = bb_map.get(exit_block) {
            *bb
        } else {
            self.context.append_basic_block(current_func, exit_block)
        };

        // Create cleanup block for natural loop exit
        let cleanup_bb = self
            .context
            .append_basic_block(current_func, "for.array.cleanup");

        // Build conditional branch to body or cleanup
        self.builder
            .build_conditional_branch(condition, body_bb, cleanup_bb)
            .unwrap();

        // Generate cleanup block that cleans up and jumps to exit
        self.builder.position_at_end(cleanup_bb);
        self.generate_loop_exit_cleanup();
        self.builder.build_unconditional_branch(exit_bb).unwrap();

        // Position at body block and load array element
        self.builder.position_at_end(body_bb);

        // Load array element with proper RC handling
        let array_ptr = self.resolve_value(array).into_pointer_value();
        let current_index = self
            .builder
            .build_load(self.context.i32_type(), index_alloca, "load_idx")
            .unwrap()
            .into_int_value();

        let is_string = self.array_contains_strings(array);
        if is_string {
            // CRITICAL: Decref the previous iteration's value before loading new one
            // This prevents memory leaks in loops
            let old_val = self
                .builder
                .build_load(
                    self.context.ptr_type(inkwell::AddressSpace::default()),
                    item_alloca,
                    "old_item",
                )
                .unwrap();

            // Check if old value is not null before decreffing
            if old_val.is_pointer_value() {
                let old_ptr = old_val.into_pointer_value();
                let _ = self
                    .context
                    .ptr_type(inkwell::AddressSpace::default())
                    .const_null();
                let old_ptr_int = self
                    .builder
                    .build_ptr_to_int(old_ptr, self.context.i64_type(), "old_ptr_int")
                    .unwrap();
                let null_ptr_int = self.context.i64_type().const_zero();
                let is_not_null = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        old_ptr_int,
                        null_ptr_int,
                        "is_not_null",
                    )
                    .unwrap();

                let current_func = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let decref_block = self
                    .context
                    .append_basic_block(current_func, "do_decref_prev");
                let skip_decref_block = self
                    .context
                    .append_basic_block(current_func, "skip_decref_prev");

                self.builder
                    .build_conditional_branch(is_not_null, decref_block, skip_decref_block)
                    .unwrap();

                // Decref block
                self.builder.position_at_end(decref_block);
                let old_rc_header = unsafe {
                    self.builder.build_in_bounds_gep(
                        self.context.i8_type(),
                        old_ptr,
                        &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                        "old_rc_header",
                    )
                }
                .unwrap();

                let decref_fn = self.decref_fn.unwrap();
                self.builder
                    .build_call(decref_fn, &[old_rc_header.into()], "")
                    .unwrap();

                self.builder
                    .build_unconditional_branch(skip_decref_block)
                    .unwrap();

                // Continue in skip block
                self.builder.position_at_end(skip_decref_block);
            }

            // Now load the new element - use pointer type directly for runtime indexing
            let elem_ptr = unsafe {
                self.builder.build_gep(
                    self.context.ptr_type(inkwell::AddressSpace::default()),
                    array_ptr,
                    &[current_index],
                    "elem_ptr",
                )
            }
            .unwrap();

            let str_val = self
                .builder
                .build_load(
                    self.context.ptr_type(inkwell::AddressSpace::default()),
                    elem_ptr,
                    "str_val",
                )
                .unwrap();

            // Store in item variable
            self.builder.build_store(item_alloca, str_val).unwrap();

            // Increment RC for loaded string
            let str_ptr = str_val.into_pointer_value();
            let rc_header = unsafe {
                self.builder.build_in_bounds_gep(
                    self.context.i8_type(),
                    str_ptr,
                    &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                    "rc_header",
                )
            }
            .unwrap();

            let incref_fn = self.incref_fn.unwrap();
            self.builder
                .build_call(incref_fn, &[rc_header.into()], "")
                .unwrap();

            // Track for cleanup
            self.heap_strings.insert(var.to_string());
        } else {
            // For non-string arrays, just load the element (no RC needed)
            // Use proper runtime indexing with element type
            let elem_ptr = unsafe {
                self.builder
                    .build_gep(elem_type, array_ptr, &[current_index], "elem_ptr")
            }
            .unwrap();

            let elem_val = self
                .builder
                .build_load(elem_type, elem_ptr, "elem_val")
                .unwrap();

            // CRITICAL: For arrays of structs (pointers), check if element is null
            // This handles sparse arrays from map.values() where some indices may be uninitialized
            // Maps using integer keys as direct indices may have gaps (e.g., {1: user} leaves index 0 empty)
            let is_struct_array = if let Some(metadata) = self.array_metadata.get(array) {
                self.struct_metadata.contains_key(&metadata.element_type)
            } else {
                false
            };

            if is_struct_array && elem_type.is_pointer_type() {
                // Check if the loaded pointer is null - skip this iteration if so
                let elem_ptr_val = elem_val.into_pointer_value();
                let is_null = self
                    .builder
                    .build_is_null(elem_ptr_val, "elem_is_null")
                    .unwrap();

                // Create blocks for null check
                let current_func = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let skip_null_block = self
                    .context
                    .append_basic_block(current_func, "skip_null_elem");
                let process_elem_block = self
                    .context
                    .append_basic_block(current_func, "process_elem");

                self.builder
                    .build_conditional_branch(is_null, skip_null_block, process_elem_block)
                    .unwrap();

                // Skip null block: increment index and jump back to condition
                self.builder.position_at_end(skip_null_block);
                let next_index = self
                    .builder
                    .build_int_add(
                        current_index,
                        self.context.i32_type().const_int(1, false),
                        "skip_next_idx",
                    )
                    .unwrap();
                self.builder.build_store(index_alloca, next_index).unwrap();
                self.builder.build_unconditional_branch(cond_block).unwrap();

                // Process element block: continue with normal loop body
                self.builder.position_at_end(process_elem_block);

                // Re-load elem_val since we're in a new block
                let elem_val = self
                    .builder
                    .build_load(elem_type, elem_ptr, "elem_val_reloaded")
                    .unwrap();

                self.builder.build_store(item_alloca, elem_val).unwrap();

                // Track struct element types for the loop variable
                if let Some(metadata) = self.array_metadata.get(array) {
                    let elem_type_name = &metadata.element_type;
                    if self.struct_metadata.contains_key(elem_type_name) {
                        self.struct_instance_types
                            .insert(var.to_string(), elem_type_name.clone());
                        self.variable_types
                            .insert(var.to_string(), elem_type_name.clone());
                        self.temp_values.insert(var.to_string(), elem_val);
                    }
                }
            } else {
                self.builder.build_store(item_alloca, elem_val).unwrap();

                // CRITICAL: Track struct element types for the loop variable
                // This enables field access (like u.Name) on elements during iteration
                if let Some(metadata) = self.array_metadata.get(array) {
                    let elem_type_name = &metadata.element_type;
                    // Check if element type is a struct
                    if self.struct_metadata.contains_key(elem_type_name) {
                        // Track loop variable as a struct instance
                        self.struct_instance_types
                            .insert(var.to_string(), elem_type_name.clone());
                        // Also track in variable_types for completeness
                        self.variable_types
                            .insert(var.to_string(), elem_type_name.clone());
                        // Store the loaded value in temp_values so resolve_value can find it
                        self.temp_values.insert(var.to_string(), elem_val);
                    }
                }
            }
        }
    }

    /// Generate map iteration: for (key, value) in map
    /// Handles RC for string keys and values
    fn generate_for_map(
        &mut self,
        key_var: &str,
        value_var: &str,
        map: &str,
        index_var: &str,
        body_block: &str,
        exit_block: &str,
        bb_map: &HashMap<String, BasicBlock<'ctx>>,
    ) {
        // Create condition block
        let current_func = self
            .builder
            .get_insert_block()
            .unwrap()
            .get_parent()
            .unwrap();
        let cond_block = self
            .context
            .append_basic_block(current_func, "for.map.cond");

        // Get the entry block to ensure allocas are created there
        let entry_block = current_func.get_first_basic_block().unwrap();
        let current_insert_block = self.builder.get_insert_block().unwrap();

        // Get map types and check if they contain strings
        let (key_type, val_type) = self.get_map_types(map);
        let (key_is_string, val_is_string) = self.map_contains_strings(map);

        // Get the type names from metadata for tracking
        let (key_type_name, val_type_name) = if let Some(metadata) = self.map_metadata.get(map) {
            (metadata.key_type.clone(), metadata.value_type.clone())
        } else {
            ("Int".to_string(), "Int".to_string())
        };

        // CRITICAL FIX: Never reuse allocas across different types to prevent corruption
        // Always create unique allocas in entry block with type-specific and counter-based names
        // This prevents stack corruption when iterating maps with different key/value types

        // Generate unique counter for this loop to prevent any name collision
        static LOOP_COUNTER: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(0);
        let loop_id = LOOP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        // Position builder in entry block for alloca creation
        if let Some(terminator) = entry_block.get_terminator() {
            self.builder.position_before(&terminator);
        } else {
            self.builder.position_at_end(entry_block);
        }

        // Always create fresh allocas with unique names based on type and loop counter
        // Remove old symbols to avoid stale pointers
        self.symbols.remove(index_var);
        self.symbols.remove(key_var);
        self.symbols.remove(value_var);

        let index_alloca = self
            .builder
            .build_alloca(
                self.context.i32_type(),
                &format!("{}_idx_L{}", index_var, loop_id),
            )
            .unwrap();

        let key_alloca = self
            .builder
            .build_alloca(
                key_type,
                &format!("{}_{}_L{}", key_var, key_type_name, loop_id),
            )
            .unwrap();

        let val_alloca = self
            .builder
            .build_alloca(
                val_type,
                &format!("{}_{}_L{}", value_var, val_type_name, loop_id),
            )
            .unwrap();

        // Restore builder position
        self.builder.position_at_end(current_insert_block);

        // Initialize key/value to null/zero to enable cleanup detection
        if key_is_string {
            let null_ptr = key_type.into_pointer_type().const_null();
            self.builder.build_store(key_alloca, null_ptr).unwrap();
        }
        if val_is_string {
            let null_ptr = val_type.into_pointer_type().const_null();
            self.builder.build_store(val_alloca, null_ptr).unwrap();
        }

        // Now initialize index to 0 at current position
        self.builder
            .build_store(index_alloca, self.context.i32_type().const_zero())
            .unwrap();

        // Register index variable
        self.symbols.insert(
            index_var.to_string(),
            crate::codegen::Symbol {
                ptr: index_alloca,
                ty: self.context.i32_type().into(),
            },
        );

        // Register key and value variables
        self.symbols.insert(
            key_var.to_string(),
            crate::codegen::Symbol {
                ptr: key_alloca,
                ty: key_type,
            },
        );
        self.symbols.insert(
            value_var.to_string(),
            crate::codegen::Symbol {
                ptr: val_alloca,
                ty: val_type,
            },
        );

        // Mark key and value as loop-local variables (allocated inside loop body, cannot be cleaned up at function level)
        self.loop_local_vars.insert(key_var.to_string());
        self.loop_local_vars.insert(value_var.to_string());

        // Enter loop context with map type information
        self.enter_loop_with_type(
            exit_block.to_string(),
            "for.map.cond".to_string(),
            Some(crate::codegen::LoopType::Map {
                key_var: key_var.to_string(),
                value_var: value_var.to_string(),
                map_var: map.to_string(),
            }),
        );
        self.add_loop_var(key_var.to_string());
        self.add_loop_var(value_var.to_string());

        // Jump to condition check
        self.builder.build_unconditional_branch(cond_block).unwrap();

        // Generate condition block
        self.builder.position_at_end(cond_block);
        let current_index = self
            .builder
            .build_load(self.context.i32_type(), index_alloca, "current_idx")
            .unwrap()
            .into_int_value();

        let map_len = self.get_map_length(map);
        let condition = self
            .builder
            .build_int_compare(IntPredicate::SLT, current_index, map_len, "map_cond")
            .unwrap();

        // Get or create body and exit blocks
        let body_bb = if let Some(bb) = bb_map.get(body_block) {
            *bb
        } else {
            self.context.append_basic_block(current_func, body_block)
        };

        let exit_bb = if let Some(bb) = bb_map.get(exit_block) {
            *bb
        } else {
            self.context.append_basic_block(current_func, exit_block)
        };

        // Create cleanup block for natural loop exit
        let cleanup_bb = self
            .context
            .append_basic_block(current_func, "for.map.cleanup");

        // Build conditional branch to body or cleanup
        self.builder
            .build_conditional_branch(condition, body_bb, cleanup_bb)
            .unwrap();

        // Generate cleanup block that cleans up and jumps to exit
        self.builder.position_at_end(cleanup_bb);
        self.generate_loop_exit_cleanup();
        self.builder.build_unconditional_branch(exit_bb).unwrap();

        // Position at body block and load map pair
        self.builder.position_at_end(body_bb);

        // Load map key-value pair with RC handling
        let map_ptr = self.resolve_value(map).into_pointer_value();
        let current_index = self
            .builder
            .build_load(self.context.i32_type(), index_alloca, "load_idx")
            .unwrap()
            .into_int_value();

        // key_is_string and val_is_string already retrieved above
        let pair_type = self.get_map_pair_type(map);

        // Get pointer to the pair at current index
        let pair_ptr = unsafe {
            self.builder
                .build_gep(pair_type, map_ptr, &[current_index], "pair_ptr")
        }
        .unwrap();

        // Load key
        let key_ptr = self
            .builder
            .build_struct_gep(pair_type, pair_ptr, 0, "key_ptr")
            .unwrap();
        let key_val = self
            .builder
            .build_load(key_type, key_ptr, "key_val")
            .unwrap();
        self.builder.build_store(key_alloca, key_val).unwrap();

        // Load value
        let val_ptr = self
            .builder
            .build_struct_gep(pair_type, pair_ptr, 1, "val_ptr")
            .unwrap();
        let val_val = self
            .builder
            .build_load(val_type, val_ptr, "val_val")
            .unwrap();
        self.builder.build_store(val_alloca, val_val).unwrap();

        // CRITICAL FIX: Set variable_types for loop variables
        // This ensures print() and other operations know the correct type
        // Without this, stale types from previous loops cause type confusion
        self.variable_types
            .insert(key_var.to_string(), key_type_name.clone());
        self.variable_types
            .insert(value_var.to_string(), val_type_name.clone());

        // Handle RC for strings
        if key_is_string {
            let key_str_ptr = key_val.into_pointer_value();
            let rc_header = unsafe {
                self.builder.build_in_bounds_gep(
                    self.context.i8_type(),
                    key_str_ptr,
                    &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                    "key_rc_header",
                )
            }
            .unwrap();

            let incref_fn = self.incref_fn.unwrap();
            self.builder
                .build_call(incref_fn, &[rc_header.into()], "")
                .unwrap();

            self.heap_strings.insert(key_var.to_string());
        }

        if val_is_string {
            let val_str_ptr = val_val.into_pointer_value();
            let rc_header = unsafe {
                self.builder.build_in_bounds_gep(
                    self.context.i8_type(),
                    val_str_ptr,
                    &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                    "val_rc_header",
                )
            }
            .unwrap();

            let incref_fn = self.incref_fn.unwrap();
            self.builder
                .build_call(incref_fn, &[rc_header.into()], "")
                .unwrap();

            self.heap_strings.insert(value_var.to_string());
        }

        // Note: The body block will be filled by subsequent MIR instructions
        // The MIR already generates an increment block - we'll inject cleanup there
    }

    /// Generate infinite loop: for { }
    fn generate_for_infinite(
        &mut self,
        body_block: &str,
        bb_map: &HashMap<String, BasicBlock<'ctx>>,
    ) {
        // Get current function
        let current_func = self
            .builder
            .get_insert_block()
            .unwrap()
            .get_parent()
            .unwrap();

        // Get or create body block
        let body_bb = if let Some(bb) = bb_map.get(body_block) {
            *bb
        } else {
            self.context.append_basic_block(current_func, body_block)
        };

        // Create an unreachable exit block for break statements
        let exit_bb = self
            .context
            .append_basic_block(current_func, "for.inf.exit");

        // Enter loop context
        self.enter_loop(
            exit_bb.get_name().to_str().unwrap().to_string(),
            body_bb.get_name().to_str().unwrap().to_string(),
        );

        // Jump directly to body
        self.builder.build_unconditional_branch(body_bb).unwrap();

        // Position at body block
        self.builder.position_at_end(body_bb);
    }

    /// Handle break statement with proper cleanup
    fn generate_break(&mut self, target: &str, bb_map: &HashMap<String, BasicBlock<'ctx>>) {
        // Clean up loop variables before breaking
        self.generate_loop_exit_cleanup();

        let target_bb = if let Some(bb) = bb_map.get(target) {
            *bb
        } else {
            let current_func = self
                .builder
                .get_insert_block()
                .unwrap()
                .get_parent()
                .unwrap();
            self.context.append_basic_block(current_func, target)
        };
        self.builder.build_unconditional_branch(target_bb).unwrap();
    }

    /// Handle continue statement with proper cleanup
    fn generate_continue(&mut self, target: &str, bb_map: &HashMap<String, BasicBlock<'ctx>>) {
        // Clean up iteration variables before continuing
        if let Some(loop_ctx) = self.loop_stack.last() {
            for var in &loop_ctx.loop_vars.clone() {
                if self.heap_strings.contains(var) {
                    self.emit_decref(var);
                }
            }
        }

        let target_bb = if let Some(bb) = bb_map.get(target) {
            *bb
        } else {
            let current_func = self
                .builder
                .get_insert_block()
                .unwrap()
                .get_parent()
                .unwrap();
            self.context.append_basic_block(current_func, target)
        };
        self.builder.build_unconditional_branch(target_bb).unwrap();
    }

    /// Generate loop increment and branch for range loops
    /// Called at the end of range loop bodies
    pub fn generate_loop_increment_and_branch(&mut self, var: &str, cond_block: BasicBlock<'ctx>) {
        // Handle RC cleanup for string variables before increment
        if let Some(loop_ctx) = self.loop_stack.last() {
            for loop_var in &loop_ctx.loop_vars {
                if self.heap_strings.contains(loop_var) {
                    self.emit_decref(loop_var);
                }
            }
        }

        if let Some(symbol) = self.symbols.get(var) {
            // Load current value
            let current = self
                .builder
                .build_load(self.context.i32_type(), symbol.ptr, "current")
                .unwrap()
                .into_int_value();

            // Increment by 1
            let one = self.context.i32_type().const_int(1, false);
            let incremented = self
                .builder
                .build_int_add(current, one, "incremented")
                .unwrap();

            // Store back
            self.builder.build_store(symbol.ptr, incremented).unwrap();
        }

        // Jump back to condition
        self.builder.build_unconditional_branch(cond_block).unwrap();
    }

    /// Generate array loop increment and branch
    /// Called at the end of array loop bodies
    pub fn generate_array_loop_increment_and_branch(
        &mut self,
        index_var: &str,
        item_var: &str,
        cond_block: BasicBlock<'ctx>,
    ) {
        // Clean up string item before next iteration
        if self.heap_strings.contains(item_var) {
            self.emit_decref(item_var);
        }

        // Increment index
        if let Some(symbol) = self.symbols.get(index_var) {
            let current = self
                .builder
                .build_load(self.context.i32_type(), symbol.ptr, "current_idx")
                .unwrap()
                .into_int_value();

            let one = self.context.i32_type().const_int(1, false);
            let incremented = self
                .builder
                .build_int_add(current, one, "incremented")
                .unwrap();

            self.builder.build_store(symbol.ptr, incremented).unwrap();
        }

        // Jump back to condition
        self.builder.build_unconditional_branch(cond_block).unwrap();
    }

    /// Generate map loop increment and branch
    /// Called at the end of map loop bodies
    pub fn generate_map_loop_increment_and_branch(
        &mut self,
        index_var: &str,
        key_var: &str,
        value_var: &str,
        cond_block: BasicBlock<'ctx>,
    ) {
        // Clean up string variables before next iteration
        if self.heap_strings.contains(key_var) {
            self.emit_decref(key_var);
        }
        if self.heap_strings.contains(value_var) {
            self.emit_decref(value_var);
        }

        // Increment index
        if let Some(symbol) = self.symbols.get(index_var) {
            let current = self
                .builder
                .build_load(self.context.i32_type(), symbol.ptr, "current_idx")
                .unwrap()
                .into_int_value();

            let one = self.context.i32_type().const_int(1, false);
            let incremented = self
                .builder
                .build_int_add(current, one, "incremented")
                .unwrap();

            self.builder.build_store(symbol.ptr, incremented).unwrap();
        }

        // Jump back to condition
        self.builder.build_unconditional_branch(cond_block).unwrap();
    }
}
