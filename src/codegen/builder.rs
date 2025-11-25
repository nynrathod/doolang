use crate::codegen::core::{CodeGen, Symbol};
use crate::limits::CODEGEN_MAX_DEPTH;
use crate::mir::MirInstr;
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::BasicValueEnum;

impl<'ctx> CodeGen<'ctx> {
    /// Generates LLVM IR for a single Intermediate Representation (MIR) instruction.
    /// Returns the resulting LLVM value if the instruction produces one (like an expression),
    /// or None if it's purely a control instruction (like a basic block jump).
    pub fn generate_instr(&mut self, instr: &MirInstr) -> Option<BasicValueEnum<'ctx>> {
        // Check recursion depth to prevent stack overflow
        self.recursion_depth += 1;
        if self.recursion_depth > CODEGEN_MAX_DEPTH {
            self.recursion_depth -= 1;
            return None;
        }

        let result = match instr {
            // Constants
            MirInstr::ConstInt { name, value } => {
                self.variable_types.insert(name.clone(), "Int".to_string());
                self.generate_const_int(name, *value)
            }
            MirInstr::ConstFloat { name, value } => {
                self.variable_types
                    .insert(name.clone(), "Float".to_string());
                self.generate_const_float(name, *value)
            }
            MirInstr::ConstBool { name, value } => {
                self.variable_types.insert(name.clone(), "Bool".to_string());
                self.boolean_temps.insert(name.clone());
                self.generate_const_bool(name, *value)
            }
            MirInstr::ConstString { name, value } => {
                self.variable_types.insert(name.clone(), "Str".to_string());
                self.generate_const_string(name, value)
            }

            // Collections
            MirInstr::Array { name, elements } => {
                self.variable_types
                    .insert(name.clone(), "Array".to_string());
                self.generate_array_with_metadata(name, elements)
            }
            MirInstr::Map {
                name,
                entries,
                key_type,
                value_type,
            } => {
                self.variable_types.insert(name.clone(), "Map".to_string());
                self.generate_map_with_metadata(
                    name,
                    entries,
                    key_type.as_deref(),
                    value_type.as_deref(),
                )
            }

            // String operations
            MirInstr::StringConcat { name, left, right } => {
                self.variable_types
                    .insert(name.clone(), "String".to_string());
                self.generate_string_concat(name, left, right)
            }

            // Arithmetic
            MirInstr::BinaryOp(op, dst, lhs, rhs) => self.generate_binary_op(op, dst, lhs, rhs),

            // Collection operations
            MirInstr::LoadArrayElement { dest, array, index } => {
                self.generate_load_array_element(dest, array, index)
            }
            MirInstr::LoadMapPair {
                key_dest,
                val_dest,
                map,
                index,
            } => self.generate_load_map_pair(key_dest, val_dest, map, index),

            MirInstr::MapGetPair { name, map, index } => {
                // MapGetPair: extract both key and value from a map at given index
                // This is used in map iteration with tuple destructuring
                // We use temporary variables to hold the key and value
                let key_tmp = format!("{}_k", name);
                let val_tmp = format!("{}_v", name);
                self.generate_load_map_pair(&key_tmp, &val_tmp, map, index);
                // Return None - the actual extraction happens via TupleGet operations
                None
            }

            // Control flow
            MirInstr::Print { values } => {
                self.generate_print(values);
                None
            }

            MirInstr::Cast {
                name,
                value,
                source_type,
                target_type,
            } => self.generate_cast(name, value, source_type, target_type),

            MirInstr::Call { dest, func, args } => self.generate_call(dest, func, args),
            MirInstr::MethodCall {
                dest,
                object,
                method,
                args,
            } => self.generate_method_call(dest, object, method, args),
            MirInstr::Closure {
                name,
                params,
                param_types,
                body_expr,
                body_ast,
                return_type,
                captures,
            } => self.generate_closure(
                name,
                params,
                param_types,
                body_expr,
                body_ast,
                return_type,
                captures,
            ),
            MirInstr::ArrayLen { name, array } => self.generate_array_len(name, array),
            MirInstr::MapLen { name, map } => self.generate_array_len(name, map),
            MirInstr::MapContains { name, map, key } => self.generate_map_contains(name, map, key),

            // ===== LOOP INSTRUCTIONS =====
            MirInstr::ForRange { .. }
            | MirInstr::ForArray { .. }
            | MirInstr::ForMap { .. }
            | MirInstr::ForInfinite { .. }
            | MirInstr::Break { .. }
            | MirInstr::Continue { .. } => {
                // These need bb_map, so they should be handled in generate_block
                // This is just a placeholder - actual handling in generate_block_with_loops
                None
            }

            MirInstr::LoopBodyMarker { .. } => {
                // Marker instruction - no code generation needed
                // The marker is used by generate_block_with_loops to know how to handle the block
                None
            }

            // ===== EXISTING INSTRUCTIONS =====
            MirInstr::Assign {
                name,
                value,
                mutable: _,
            } => {
                // Propagate type information from source to destination
                if let Some(source_type) = self.variable_types.get(value).cloned() {
                    self.variable_types.insert(name.clone(), source_type);
                }
                // Propagate boolean tracking
                if self.boolean_temps.contains(value) {
                    self.boolean_temps.insert(name.clone());
                }
                // Propagate Result type information from source to destination
                if let Some(result_type) = self.result_types.get(value).cloned() {
                    self.result_types.insert(name.clone(), result_type);
                }
                // Propagate tuple type information from source to destination
                if let Some(tuple_type) = self.tuple_types.get(value).cloned() {
                    self.tuple_types.insert(name.clone(), tuple_type);
                }
                // Propagate array metadata from source to destination
                if let Some(array_meta) = self.array_metadata.get(value).cloned() {
                    self.array_metadata.insert(name.clone(), array_meta);
                }
                // Propagate map metadata from source to destination
                if let Some(map_meta) = self.map_metadata.get(value).cloned() {
                    self.map_metadata.insert(name.clone(), map_meta);
                }
                // Propagate struct instance type from source to destination
                // Check both with and without '%' prefix to handle temp variables
                let struct_type = self
                    .struct_instance_types
                    .get(value)
                    .cloned()
                    .or_else(|| {
                        self.struct_instance_types
                            .get(&value.trim_start_matches('%').to_string())
                            .cloned()
                    })
                    .or_else(|| {
                        // Also check if source has a % prefix but we're looking without it
                        self.struct_instance_types
                            .get(&format!("%{}", value))
                            .cloned()
                    });

                if let Some(struct_type) = struct_type {
                    self.struct_instance_types
                        .insert(name.clone(), struct_type.clone());
                    // Also store with % prefix if name doesn't have it
                    if !name.starts_with('%') {
                        self.struct_instance_types
                            .insert(format!("%{}", name), struct_type);
                    }
                }
                // Propagate heap tracking
                if self.heap_arrays.contains(value) {
                    self.heap_arrays.insert(name.clone());
                }
                if self.heap_maps.contains(value) {
                    self.heap_maps.insert(name.clone());
                }
                let val = self.resolve_value(value);

                // For boolean comparison results, remove any existing symbol and force reallocation
                // This ensures boolean values are always stored as i32, not as their temporary type
                if self.variable_types.get(name).map_or(false, |t| t == "Bool") {
                    self.symbols.remove(name);
                }

                // Check if this value came from ArrayGet - if so, it's a loop iteration variable
                // and should NEVER have array/map metadata propagated to it
                let is_from_arrayget = self.arrayget_sources.contains_key(value);

                // If assigning from ArrayGet, explicitly remove any existing array/map metadata
                // from the destination variable to prevent stale metadata from previous loops
                if is_from_arrayget {
                    self.array_metadata.remove(name);
                    self.map_metadata.remove(name);
                    self.heap_arrays.remove(name);
                    self.heap_maps.remove(name);

                    // If this variable already exists from a previous block/loop,
                    // remove it so we can create a fresh alloca in the current block
                    // This prevents SSA violations when reusing variable names across loops
                    self.symbols.remove(name);
                }

                let value_is_heap_str = self.heap_strings.contains(value);
                let value_is_heap_array = self.heap_arrays.contains(value);
                let value_is_heap_map = self.heap_maps.contains(value);

                if let Some(ptrs) = self.composite_string_ptrs.remove(value) {
                    self.composite_string_ptrs.insert(name.clone(), ptrs);
                }

                if let Some(sym) = self.symbols.get(name) {
                    // Re-assignment: decref old value
                    let name_was_heap_str = self.heap_strings.contains(name);
                    let name_was_heap_array = self.heap_arrays.contains(name);
                    let name_was_heap_map = self.heap_maps.contains(name);

                    if name_was_heap_array || name_was_heap_map {
                        if let Some(old_str_ptrs) = self.composite_string_ptrs.get(name) {
                            for str_ptr in old_str_ptrs {
                                let data_ptr = str_ptr.into_pointer_value();
                                let rc_header = unsafe {
                                    self.builder.build_in_bounds_gep(
                                        self.context.i8_type(),
                                        data_ptr,
                                        &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                        "rc_header",
                                    )
                                }
                                .unwrap();

                                let decref = self.decref_fn.unwrap();
                                self.builder
                                    .build_call(decref, &[rc_header.into()], "")
                                    .unwrap();
                            }
                        }
                    }

                    if name_was_heap_str || name_was_heap_array || name_was_heap_map {
                        self.emit_decref(name);
                    }

                    self.builder.build_store(sym.ptr, val).unwrap();

                    self.heap_strings.remove(name);
                    self.heap_arrays.remove(name);
                    self.heap_maps.remove(name);

                    if value_is_heap_str {
                        self.heap_strings.insert(name.clone());
                        // Only remove temp from tracking if source is NOT a user variable
                        // User variables (in symbols) should stay tracked for cleanup at function exit
                        // Only remove if source is a compiler temporary (starts with %)
                        if !self.symbols.contains_key(value) || value.starts_with('%') {
                            self.heap_strings.remove(value);
                        }
                        // Mark the temp as loop-local too (defensive - should already be marked by ArrayGet)
                        if is_from_arrayget {
                            self.loop_local_vars.insert(value.to_string());
                        }
                        // Only incref when copying from an existing variable (not from a temp)
                        if self.symbols.contains_key(value) {
                            self.emit_incref(name);
                        }
                    } else if value_is_heap_array {
                        self.heap_arrays.insert(name.clone());
                        // Copy array metadata from source to destination
                        if let Some(metadata) = self.array_metadata.get(value).cloned() {
                            self.array_metadata.insert(name.clone(), metadata);
                        }
                        // Only remove temp from tracking if source is NOT a user variable
                        // User variables (in symbols) should stay tracked for cleanup at function exit
                        // Only remove if source is a compiler temporary (starts with %)
                        if !self.symbols.contains_key(value) || value.starts_with('%') {
                            self.heap_arrays.remove(value);
                        }
                        // Only incref when copying from an existing variable (not from a temp)
                        if self.symbols.contains_key(value) {
                            self.emit_incref(name);
                        }

                        // Copy array metadata on re-assignment - ENHANCED
                        // CRITICAL: Try ALL possible ways to find the metadata
                        let mut found_metadata = self.array_metadata.get(value).cloned();

                        // If not found directly, search through ALL array metadata by pointer equality
                        if found_metadata.is_none() {
                            if let Some(val_ptr_value) = self.temp_values.get(value) {
                                if val_ptr_value.is_pointer_value() {
                                    let val_ptr = val_ptr_value.into_pointer_value();
                                    let array_metadata_clone = self.array_metadata.clone();
                                    for (meta_name, metadata) in &array_metadata_clone {
                                        if let Some(meta_val) = self.temp_values.get(meta_name) {
                                            if meta_val.is_pointer_value()
                                                && meta_val.into_pointer_value() == val_ptr
                                            {
                                                found_metadata = Some(metadata.clone());

                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // LAST RESORT: Try to extract array length from LLVM type
                        if found_metadata.is_none() {
                            if let Some(sym) = self.symbols.get(value) {
                                if let Ok(loaded) =
                                    self.builder
                                        .build_load(sym.ty, sym.ptr, "extract_array_len")
                                {
                                    if loaded.is_pointer_value() {
                                        // Try to determine element type and count
                                        // This is a heuristic - we assume string arrays if we can't find metadata
                                        let element_type = if self.heap_strings.contains(value) {
                                            "Str"
                                        } else {
                                            "Int"
                                        };

                                        // For dynamically allocated arrays, try to infer size from usage
                                        // Check if there are any GEP instructions that accessed this array
                                        let mut max_index = 0;
                                        for (check_name, _) in &self.temp_values {
                                            if check_name.contains(value)
                                                && check_name.contains("elem")
                                            {
                                                // Found an element access, try to extract index
                                                if let Some(idx_part) =
                                                    check_name.split("elem_").last()
                                                {
                                                    if let Some(idx) =
                                                        idx_part.chars().next().and_then(|c| {
                                                            c.to_string().parse::<usize>().ok()
                                                        })
                                                    {
                                                        max_index = max_index.max(idx);
                                                    }
                                                }
                                            }
                                        }

                                        if max_index > 0 {
                                            found_metadata = Some(crate::codegen::ArrayMetadata {
                                                length: max_index + 1,
                                                element_type: element_type.to_string(),
                                                contains_strings: element_type == "Str",
                                            });
                                        }
                                    }
                                }
                            }
                        }

                        if let Some(metadata) = found_metadata {
                            // Do not propagate array metadata to loop iteration variables
                            // Loop variables contain scalar elements extracted from arrays, not arrays themselves
                            // Also skip if value came from ArrayGet (definitely a loop iteration variable)
                            if !self.is_loop_var(name) && !is_from_arrayget {
                                // Register metadata only for the exact name, not extensive variations
                                // This prevents accidental metadata leakage to unrelated variables
                                self.array_metadata
                                    .insert(name.to_string(), metadata.clone());
                            }
                        } else {
                            // Try to find metadata by checking if value points to a known array
                            // But skip if assigning to a loop variable or if from ArrayGet
                            if !self.is_loop_var(name) && !is_from_arrayget {
                                self.propagate_metadata(name, value);
                            }
                        }
                    } else if value_is_heap_map {
                        self.heap_maps.insert(name.clone());
                        // Only incref when copying from an existing variable (not from a temp)
                        if self.symbols.contains_key(value) {
                            self.emit_incref(name);
                        }

                        // Copy map metadata on re-assignment
                        // Copy map metadata
                        // But NEVER propagate to loop iteration variables or ArrayGet results
                        if !self.is_loop_var(name) && !is_from_arrayget {
                            if let Some(metadata) = self.map_metadata.get(value).cloned() {
                                self.map_metadata.insert(name.clone(), metadata);
                            } else {
                                // Try to find metadata by checking if value points to a known map
                                self.propagate_metadata(name, value);
                            }
                        }
                    } else {
                        // Even for non-heap reassignments, try to propagate metadata
                        // This handles cases like: inneritem_array = innerarr (both ptrs)
                        self.propagate_metadata(name, value);
                    }
                } else {
                    // Initial assignment
                    // Create alloca in entry block for cross-block variables
                    // Save current position
                    let current_block = self.builder.get_insert_block().unwrap();
                    let func = current_block.get_parent().unwrap();
                    let entry_block = func.get_first_basic_block().unwrap();

                    // Position at end of entry block (before terminator if exists)
                    if let Some(terminator) = entry_block.get_terminator() {
                        self.builder.position_before(&terminator);
                    } else {
                        self.builder.position_at_end(entry_block);
                    }

                    // For boolean values, force i32 allocation
                    // Check both variable_types and if val is i1 (from bool comparison)
                    let is_bool_type = self.variable_types.get(name).map_or(false, |t| t == "Bool");
                    let is_i1_value =
                        val.is_int_value() && val.into_int_value().get_type().get_bit_width() == 1;

                    // For arrays/maps, force pointer type allocation
                    let is_array =
                        self.heap_arrays.contains(name) || self.array_metadata.contains_key(name);
                    let is_map =
                        self.heap_maps.contains(name) || self.map_metadata.contains_key(name);

                    let alloc_type = if is_bool_type || is_i1_value {
                        self.context.i32_type().into()
                    } else if is_array || is_map {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        val.get_type()
                    };
                    let alloca = self.builder.build_alloca(alloc_type, name).unwrap();

                    // Restore position to current block
                    self.builder.position_at_end(current_block);

                    self.builder.build_store(alloca, val).unwrap();

                    self.symbols.insert(
                        name.clone(),
                        Symbol {
                            ptr: alloca,
                            ty: alloc_type,
                        },
                    );

                    // Mark as block-local ONLY if assigning from ArrayGet
                    // ArrayGet is ALWAYS used for loop iteration variables
                    // Regular variables (even in conditionals) should be cleaned up normally
                    if is_from_arrayget {
                        self.loop_local_vars.insert(name.clone());
                    }

                    if value_is_heap_str {
                        self.heap_strings.insert(name.clone());
                        // Remove temp from tracking (ownership transferred to symbol)
                        self.heap_strings.remove(value);
                        // Mark the temp as loop-local too (defensive)
                        if is_from_arrayget {
                            self.loop_local_vars.insert(value.to_string());
                        }
                        if self.symbols.contains_key(value) {
                            self.emit_incref(name);
                        }
                    } else if value_is_heap_array {
                        self.heap_arrays.insert(name.clone());
                        // Remove temp from tracking (ownership transferred to symbol)
                        self.heap_arrays.remove(value);
                        if self.symbols.contains_key(value) {
                            self.emit_incref(name);
                        }

                        // Copy array metadata - ENHANCED for dynamic arrays
                        // CRITICAL: Try ALL possible ways to find the metadata
                        let mut found_metadata = self.array_metadata.get(value).cloned();

                        // If not found directly, search through ALL array metadata by pointer equality
                        if found_metadata.is_none() {
                            if let Some(val_ptr_value) = self.temp_values.get(value) {
                                if val_ptr_value.is_pointer_value() {
                                    let val_ptr = val_ptr_value.into_pointer_value();
                                    let array_metadata_clone = self.array_metadata.clone();
                                    for (meta_name, metadata) in &array_metadata_clone {
                                        if let Some(meta_val) = self.temp_values.get(meta_name) {
                                            if meta_val.is_pointer_value()
                                                && meta_val.into_pointer_value() == val_ptr
                                            {
                                                found_metadata = Some(metadata.clone());
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // LAST RESORT: Try to extract array length from LLVM value directly
                        if found_metadata.is_none() {
                            // Check if the value itself has array information
                            let element_type = if value.contains("str") || value.contains("Str") {
                                "Str"
                            } else {
                                "Int"
                            };

                            // Try to infer from element count in temp_values
                            let mut elem_count = 0;
                            for (temp_name, _) in &self.temp_values {
                                if temp_name.starts_with(&format!("{}_elem_", value))
                                    || temp_name.contains(&format!("{}[", value))
                                {
                                    elem_count += 1;
                                }
                            }

                            if elem_count > 0 {
                                found_metadata = Some(crate::codegen::ArrayMetadata {
                                    length: elem_count,
                                    element_type: element_type.to_string(),
                                    contains_strings: element_type == "Str",
                                });
                            }
                        }

                        if let Some(metadata) = found_metadata {
                            // Do not propagate array metadata to loop iteration variables
                            // Loop variables contain scalar elements extracted from arrays, not arrays themselves
                            // Also skip if value came from ArrayGet (definitely a loop iteration variable)
                            if !self.is_loop_var(name) && !is_from_arrayget {
                                // Register metadata only for the exact name, not extensive variations
                                // This prevents accidental metadata leakage to unrelated variables
                                self.array_metadata
                                    .insert(name.to_string(), metadata.clone());
                            }
                        } else {
                            // Try to find metadata by checking if value points to a known array
                            // But skip if assigning to a loop variable or if from ArrayGet
                            if !self.is_loop_var(name) && !is_from_arrayget {
                                self.propagate_metadata(name, value);
                            }
                        }
                    } else if value_is_heap_map {
                        self.heap_maps.insert(name.clone());
                        // Only remove temp from tracking if source is NOT a user variable
                        // User variables (in symbols) should stay tracked for cleanup at function exit
                        // Only remove if source is a compiler temporary (starts with %)
                        if !self.symbols.contains_key(value) || value.starts_with('%') {
                            self.heap_maps.remove(value);
                        }
                        // Only incref when copying from an existing variable (not from a temp)
                        if self.symbols.contains_key(value) {
                            self.emit_incref(name);
                        }

                        // Copy map metadata
                        // But NEVER propagate to loop iteration variables or ArrayGet results
                        if !self.is_loop_var(name) && !is_from_arrayget {
                            if let Some(metadata) = self.map_metadata.get(value).cloned() {
                                self.map_metadata.insert(name.clone(), metadata);
                            } else {
                                // Try to find metadata by checking if value points to a known map
                                self.propagate_metadata(name, value);
                            }
                        }
                    } else {
                        // Even for initial non-heap assignments, try to propagate metadata
                        // This is critical for variables that store pointers
                        // But skip if assigning to a loop variable or ArrayGet result
                        if !self.is_loop_var(name) && !is_from_arrayget {
                            self.propagate_metadata(name, value);
                        }
                    }
                }

                // Clear arrayget_sources for this name after assignment
                // This prevents stale metadata from persisting across different loops
                // that reuse the same variable name (e.g., multiple loops with variable 'n')
                self.arrayget_sources.remove(name);

                Some(val)
            }

            MirInstr::IncrementDecrement { variable, op } => {
                self.generate_increment_decrement(variable, op);
                None
            }

            MirInstr::IncRef { value } => {
                self.emit_incref(value);
                None
            }

            MirInstr::DecRef { value } => {
                self.emit_decref(value);
                None
            }

            MirInstr::ArraySlice {
                name,
                array,
                start,
                end,
                inclusive,
            } => {
                // Array/string slicing: arr[start..end] or arr[start..=end]
                let array_val = self.resolve_value(array);
                let start_val = self.resolve_value(start).into_int_value();
                let end_val_raw = self.resolve_value(end).into_int_value();

                // Adjust end for inclusive ranges: end = end + 1
                let end_val = if *inclusive {
                    self.builder
                        .build_int_add(
                            end_val_raw,
                            self.context.i32_type().const_int(1, false),
                            "end_inclusive",
                        )
                        .unwrap()
                } else {
                    end_val_raw
                };

                // Calculate slice length: end - start
                let slice_len = self
                    .builder
                    .build_int_sub(end_val, start_val, "slice_len")
                    .unwrap();

                // Get array metadata to determine element type
                let metadata = self.array_metadata.get(array).cloned();

                if let Some(meta) = metadata {
                    // Get element type
                    let elem_type = match meta.element_type.as_str() {
                        "Int" => self.context.i32_type().as_basic_type_enum(),
                        "Float" => self.context.f64_type().as_basic_type_enum(),
                        "Bool" => self.context.bool_type().as_basic_type_enum(),
                        "Str" => self
                            .context
                            .ptr_type(inkwell::AddressSpace::default())
                            .as_basic_type_enum(),
                        _ => self.context.i32_type().as_basic_type_enum(),
                    };

                    // Check if slice length is zero (empty slice)
                    let is_zero = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            slice_len,
                            self.context.i32_type().const_int(0, false),
                            "is_zero_len",
                        )
                        .unwrap();

                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let alloc_bb = self.context.append_basic_block(current_fn, "slice_alloc");
                    let empty_bb = self.context.append_basic_block(current_fn, "slice_empty");
                    let merge_bb = self.context.append_basic_block(current_fn, "slice_merge");

                    // Create a PHI node placeholder for the result
                    self.builder
                        .build_conditional_branch(is_zero, empty_bb, alloc_bb)
                        .unwrap();

                    // Empty slice case
                    self.builder.position_at_end(empty_bb);
                    let null_ptr = self
                        .context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null();
                    self.builder.build_unconditional_branch(merge_bb).unwrap();

                    // Non-empty slice case
                    self.builder.position_at_end(alloc_bb);

                    // Allocate new array for slice using malloc
                    let elem_size = elem_type.size_of().unwrap();
                    let total_size = self
                        .builder
                        .build_int_mul(slice_len, elem_size, "total_size")
                        .unwrap();

                    let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
                        let fn_type = self
                            .context
                            .ptr_type(inkwell::AddressSpace::default())
                            .fn_type(&[self.context.i32_type().into()], false);
                        self.module.add_function("malloc", fn_type, None)
                    });

                    let new_array = self
                        .builder
                        .build_call(malloc_fn, &[total_size.into()], "slice_malloc")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    // Copy elements using memcpy for better performance
                    let array_ptr = array_val.into_pointer_value();

                    // Calculate source pointer with offset
                    let src_ptr = unsafe {
                        self.builder
                            .build_gep(elem_type, array_ptr, &[start_val], "src_start")
                            .unwrap()
                    };

                    // Use memcpy to copy the slice
                    let memcpy_fn = self
                        .module
                        .get_function("llvm.memcpy.p0.p0.i32")
                        .unwrap_or_else(|| {
                            let fn_type = self.context.void_type().fn_type(
                                &[
                                    self.context
                                        .ptr_type(inkwell::AddressSpace::default())
                                        .into(),
                                    self.context
                                        .ptr_type(inkwell::AddressSpace::default())
                                        .into(),
                                    self.context.i32_type().into(),
                                    self.context.bool_type().into(),
                                ],
                                false,
                            );
                            self.module
                                .add_function("llvm.memcpy.p0.p0.i32", fn_type, None)
                        });

                    self.builder
                        .build_call(
                            memcpy_fn,
                            &[
                                new_array.into(),
                                src_ptr.into(),
                                total_size.into(),
                                self.context.bool_type().const_zero().into(),
                            ],
                            "memcpy_slice",
                        )
                        .unwrap();

                    self.builder.build_unconditional_branch(merge_bb).unwrap();

                    // Merge block with PHI node
                    self.builder.position_at_end(merge_bb);
                    let phi = self
                        .builder
                        .build_phi(
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "slice_result",
                        )
                        .unwrap();
                    phi.add_incoming(&[(&null_ptr, empty_bb), (&new_array, alloc_bb)]);
                    let result_ptr = phi.as_basic_value().into_pointer_value();

                    // Store metadata and register the slice
                    self.temp_values.insert(name.clone(), result_ptr.into());

                    // Store metadata for slices so ArrayGet can find the element type
                    self.array_metadata.insert(
                        name.clone(),
                        crate::codegen::ArrayMetadata {
                            element_type: meta.element_type.clone(),
                            length: 0, // Dynamic length - not tracked
                            contains_strings: meta.contains_strings,
                        },
                    );

                    // Mark as a slice (malloc'd but no RC header) so cleanup doesn't try to decref it
                    // Store in slice_arrays instead of heap_arrays to avoid RC deref on cleanup
                    self.slice_arrays.insert(name.clone());
                    self.variable_types
                        .insert(name.clone(), "Array".to_string());

                    Some(result_ptr.into())
                } else {
                    // Fallback for unknown type - return dummy value
                    let dummy = self.context.i32_type().const_int(0, false);
                    self.temp_values.insert(name.clone(), dummy.into());
                    Some(dummy.into())
                }
            }

            MirInstr::ArrayGet { name, array, index } => {
                // eprintln!(
                //     "[DEBUG] ArrayGet: name={}, array={}, index={}",
                //     name, array, index
                // );
                let array_val = self.resolve_value(array);

                // Handle case where array might be loaded from a symbol (e.g., a slice assigned to a variable)
                let array_ptr = if array_val.is_pointer_value() {
                    array_val.into_pointer_value()
                } else {
                    // If it's not a pointer, it might be loaded from a symbol
                    // Try to get it from symbols
                    if let Some(sym) = self.symbols.get(array) {
                        self.builder
                            .build_load(
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                sym.ptr,
                                &format!("{}_load", array),
                            )
                            .unwrap()
                            .into_pointer_value()
                    } else {
                        // Fallback: assume it's a pointer that was incorrectly typed
                        panic!("Found {} but expected PointerValue variant", array_val);
                    }
                };

                let index_val = self.resolve_value(index).into_int_value();

                // Track that this ArrayGet result came from this source array
                self.arrayget_sources.insert(name.clone(), array.clone());

                // Check if this is actually a map iteration (map metadata exists for this array)
                if let Some(_) = self.map_metadata.get(array) {
                    // This is a map being iterated as an array - extract the key-value pair
                    let (key_type, val_type) = self.get_map_types(array);
                    let pair_type = self.context.struct_type(&[key_type, val_type], false);
                    // Use direct pointer arithmetic with single index for runtime maps
                    // This is clearer and more explicit than the two-index array syntax
                    let pair_ptr = unsafe {
                        self.builder.build_in_bounds_gep(
                            pair_type,
                            array_ptr,
                            &[index_val],
                            "pair_ptr",
                        )
                    }
                    .unwrap();

                    // Return the pair pointer so TupleGet can extract key/value
                    // Store the pair pointer in temp_values
                    self.temp_values.insert(name.clone(), pair_ptr.into());

                    // If this temp was pre-allocated as a symbol, store it there too
                    if let Some(sym) = self.symbols.get(name) {
                        self.builder.build_store(sym.ptr, pair_ptr).unwrap();
                    }

                    // Return the pair pointer for subsequent TupleGet operations
                    return Some(pair_ptr.into());
                }

                // Normal array element access
                // Try multiple name variations to find metadata for array iteration
                let elem_type = if let Some(metadata) = self.array_metadata.get(array) {
                    // eprintln!(
                    //     "[DEBUG] Found array metadata for {}: element_type={}",
                    //     array, metadata.element_type
                    // );
                    match metadata.element_type.as_str() {
                        "Int" => self.context.i32_type().into(),
                        "Float" => self.context.f64_type().into(),
                        "Bool" => self.context.bool_type().into(),
                        "Str" => self
                            .context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into(),
                        _ => self.context.i32_type().into(),
                    }
                } else {
                    // eprintln!(
                    //     "[DEBUG] No array metadata found for {}, trying variations",
                    //     array
                    // );
                    // Try array name variations (without _array suffix, with % prefix, etc)
                    let base_name = array.trim_start_matches('%').trim_end_matches("_array");
                    let variations = vec![
                        array.to_string(),
                        base_name.to_string(),
                        format!("{}_array", base_name),
                        format!("{}item_array", base_name),
                    ];

                    let mut found_type = self.context.i32_type().as_basic_type_enum();
                    for var in variations {
                        if let Some(metadata) = self.array_metadata.get(&var) {
                            // eprintln!(
                            //     "[DEBUG] Found metadata for variation {}: element_type={}",
                            //     var, metadata.element_type
                            // );
                            found_type = match metadata.element_type.as_str() {
                                "Int" => self.context.i32_type().into(),
                                "Float" => self.context.f64_type().into(),
                                "Bool" => self.context.bool_type().into(),
                                "Str" => self
                                    .context
                                    .ptr_type(inkwell::AddressSpace::default())
                                    .into(),
                                _ => self.context.i32_type().into(),
                            };
                            break;
                        }
                    }
                    // if found_type.is_int_type() && found_type.into_int_type().get_bit_width() == 32
                    // {
                    //     eprintln!("[DEBUG] Using default Int type for array {}", array);
                    // }
                    found_type
                };

                // Use direct pointer arithmetic with single index for runtime arrays
                // This is clearer and more explicit than the two-index array syntax
                let elem_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(elem_type, array_ptr, &[index_val], "elem_ptr")
                }
                .unwrap();

                // Load the element
                let elem_val = self
                    .builder
                    .build_load(elem_type, elem_ptr, "elem_val")
                    .unwrap();

                // Store in temp_values for immediate use
                self.temp_values.insert(name.clone(), elem_val);

                // Track the type of this result
                if elem_type.is_pointer_type() {
                    self.variable_types.insert(name.clone(), "Str".to_string());
                    // eprintln!("[DEBUG] ArrayGet result {} is Str", name);
                } else if elem_type.is_float_type() {
                    self.variable_types
                        .insert(name.clone(), "Float".to_string());
                    // eprintln!("[DEBUG] ArrayGet result {} is Float", name);
                } else if elem_type.is_int_type() {
                    let int_type = elem_type.into_int_type();
                    if int_type.get_bit_width() == 1 {
                        self.variable_types.insert(name.clone(), "Bool".to_string());
                        // eprintln!("[DEBUG] ArrayGet result {} is Bool", name);
                    } else {
                        self.variable_types.insert(name.clone(), "Int".to_string());
                        // eprintln!("[DEBUG] ArrayGet result {} is Int", name);
                    }
                }

                // If this temp was pre-allocated as a symbol (cross-block usage), store it there too
                if let Some(sym) = self.symbols.get(name) {
                    self.builder.build_store(sym.ptr, elem_val).unwrap();
                }

                // Track if this is a heap-allocated value and increment RC
                if elem_type.is_pointer_type() && self.array_contains_strings(array) {
                    self.heap_strings.insert(name.clone());

                    // Mark ALL ArrayGet results as loop-local
                    // This is safe because ArrayGet is primarily used in loop contexts
                    // Even if not technically in a loop, these are temporary extracted values
                    // that should not be cleaned up at function level (they'll be cleaned at loop exit)
                    self.loop_local_vars.insert(name.clone());

                    // TEMPORARILY DISABLED: Skip incref for debugging
                    // The issue is that string constants don't have RC headers
                    // and we're crashing when trying to incref them
                    // TODO: Fix this properly by detecting constants vs heap strings
                }

                Some(elem_val)
            }

            MirInstr::TupleExtract {
                name,
                source,
                index,
            } => {
                // Extract element from a tuple (multi-value function return)
                // CRITICAL FIX: For Result returns, the source is a pointer to heap-allocated tuple
                // We must use it directly WITHOUT creating intermediate storage

                // First check if source is directly in temp_values (bypassing resolve_value)
                if let Some(source_val) = self.temp_values.get(source).copied() {
                    // Check if this is a Result struct containing a tuple
                    // Result structs have { i32 tag, ptr value } where ptr points to the tuple
                    if source_val.is_struct_value() {
                        let result_struct = source_val.into_struct_value();
                        let struct_type = result_struct.get_type();

                        // Check if this looks like a Result struct (2 fields: i32 tag, ptr value)
                        // Also verify field 1 is actually a pointer (not another int from plain tuple)
                        if struct_type.count_fields() == 2 {
                            if let Some(field0_type) = struct_type.get_field_type_at_index(0) {
                                if let BasicTypeEnum::IntType(int_type) = field0_type {
                                    if int_type.get_bit_width() == 32 {
                                        // Check if field 1 is a pointer (Result) or not (plain tuple)
                                        if let Some(field1_type) =
                                            struct_type.get_field_type_at_index(1)
                                        {
                                            if let BasicTypeEnum::PointerType(_) = field1_type {
                                                // This is a Result struct - extract the Ok value pointer
                                                let ok_value_ptr = self
                                                    .builder
                                                    .build_extract_value(
                                                        result_struct,
                                                        1,
                                                        "ok_tuple_ptr",
                                                    )
                                                    .unwrap()
                                                    .into_pointer_value();

                                                // Now use the tuple pointer to extract the field
                                                if let Some(tuple_type_str) =
                                                    self.tuple_types.get(source).cloned()
                                                {
                                                    if let Some(struct_type) =
                                                        self.tuple_struct_types.get(&tuple_type_str)
                                                    {
                                                        // Use struct_gep to get field pointer from heap tuple
                                                        let field_ptr = self
                                                            .builder
                                                            .build_struct_gep(
                                                                *struct_type,
                                                                ok_value_ptr,
                                                                *index as u32,
                                                                &format!("{}_field", name),
                                                            )
                                                            .unwrap();

                                                        // Load the field value
                                                        let field_type = struct_type
                                                            .get_field_type_at_index(*index as u32)
                                                            .unwrap();
                                                        let field_val = self
                                                            .builder
                                                            .build_load(field_type, field_ptr, name)
                                                            .unwrap();

                                                        // Track metadata for this field
                                                        let inner = tuple_type_str
                                                            .strip_prefix("Tuple(")
                                                            .and_then(|s| s.strip_suffix(")"))
                                                            .unwrap_or("");
                                                        let types = crate::codegen::core::helpers::parse_tuple_types(inner);
                                                        if let Some(type_str) = types.get(*index) {
                                                            let type_str = type_str.as_str();

                                                            if type_str.starts_with("Array") {
                                                                self.heap_arrays
                                                                    .insert(name.clone());
                                                                if let Some(elem_type) = type_str
                                                                    .strip_prefix("Array(")
                                                                    .and_then(|s| {
                                                                        s.strip_suffix(")")
                                                                    })
                                                                {
                                                                    self.array_metadata.insert(
                                                                name.clone(),
                                                                crate::codegen::ArrayMetadata {
                                                                    length: 0,
                                                                    element_type: elem_type
                                                                        .to_string(),
                                                                    contains_strings: elem_type
                                                                        == "Str",
                                                                },
                                                            );
                                                                }
                                                            } else if type_str.starts_with("Map") {
                                                                self.heap_maps.insert(name.clone());
                                                                if let Some(inner) = type_str
                                                                    .strip_prefix("Map(")
                                                                    .and_then(|s| {
                                                                        s.strip_suffix(")")
                                                                    })
                                                                {
                                                                    let parts: Vec<&str> =
                                                                        inner.split(',').collect();
                                                                    if parts.len() == 2 {
                                                                        let key_type = parts[0]
                                                                            .trim()
                                                                            .to_string();
                                                                        let value_type = parts[1]
                                                                            .trim()
                                                                            .to_string();
                                                                        self.map_metadata.insert(
                                                                    name.clone(),
                                                                    crate::codegen::MapMetadata {
                                                                        length: 0,
                                                                        key_type: key_type.clone(),
                                                                        value_type: value_type
                                                                            .clone(),
                                                                        key_is_string: key_type
                                                                            == "Str",
                                                                        value_is_string: value_type
                                                                            == "Str",
                                                                        key_needs_rc: key_type
                                                                            == "Str",
                                                                        value_needs_rc: value_type
                                                                            == "Str",
                                                                    },
                                                                );
                                                                    }
                                                                }
                                                            } else if type_str == "Bool" {
                                                                self.boolean_temps
                                                                    .insert(name.clone());
                                                            } else if type_str
                                                                .starts_with("Struct(")
                                                                || self
                                                                    .struct_metadata
                                                                    .contains_key(type_str)
                                                            {
                                                                // Handle struct types in tuple extraction
                                                                // Normalize to "Struct(Name)" format
                                                                let normalized_type = if type_str
                                                                    .starts_with("Struct(")
                                                                {
                                                                    type_str.to_string()
                                                                } else {
                                                                    format!("Struct({})", type_str)
                                                                };

                                                                self.variable_types.insert(
                                                                    name.clone(),
                                                                    normalized_type,
                                                                );
                                                                self.heap_arrays
                                                                    .insert(name.clone());
                                                                // Track for RC
                                                            } else {
                                                                // For non-struct types, store the type string
                                                                self.variable_types.insert(
                                                                    name.clone(),
                                                                    type_str.to_string(),
                                                                );
                                                            }
                                                        }

                                                        self.temp_values
                                                            .insert(name.clone(), field_val);
                                                        return Some(field_val);
                                                    }
                                                }
                                            } else {
                                                // Field 1 is not a pointer - this is a plain tuple, not Result
                                                // Fall through to plain tuple handling below
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Plain tuple (not Result-wrapped) - extract directly from struct
                        if struct_type.count_fields() > *index as u32 {
                            let field_val = self
                                .builder
                                .build_extract_value(result_struct, *index as u32, name)
                                .unwrap();

                            // Track metadata for this field
                            if let Some(tuple_type_str) = self.tuple_types.get(source).cloned() {
                                let inner = tuple_type_str
                                    .strip_prefix("Tuple(")
                                    .and_then(|s| s.strip_suffix(")"))
                                    .unwrap_or("");
                                let types = crate::codegen::core::helpers::parse_tuple_types(inner);
                                if let Some(type_str) = types.get(*index) {
                                    let type_str = type_str.as_str();

                                    if type_str.starts_with("Array") {
                                        self.heap_arrays.insert(name.clone());
                                        if let Some(elem_type) = type_str
                                            .strip_prefix("Array(")
                                            .and_then(|s| s.strip_suffix(")"))
                                        {
                                            self.array_metadata.insert(
                                                name.clone(),
                                                crate::codegen::ArrayMetadata {
                                                    length: 0,
                                                    element_type: elem_type.to_string(),
                                                    contains_strings: elem_type == "Str",
                                                },
                                            );
                                        }
                                    } else if type_str.starts_with("Map") {
                                        self.heap_maps.insert(name.clone());
                                        if let Some(inner) = type_str
                                            .strip_prefix("Map(")
                                            .and_then(|s| s.strip_suffix(")"))
                                        {
                                            let parts: Vec<&str> = inner.split(',').collect();
                                            if parts.len() == 2 {
                                                let key_type = parts[0].trim().to_string();
                                                let value_type = parts[1].trim().to_string();
                                                self.map_metadata.insert(
                                                    name.clone(),
                                                    crate::codegen::MapMetadata {
                                                        length: 0,
                                                        key_type: key_type.clone(),
                                                        value_type: value_type.clone(),
                                                        key_is_string: key_type == "Str",
                                                        value_is_string: value_type == "Str",
                                                        key_needs_rc: key_type == "Str",
                                                        value_needs_rc: value_type == "Str",
                                                    },
                                                );
                                            }
                                        }
                                    } else if type_str == "Bool" {
                                        self.boolean_temps.insert(name.clone());
                                    } else if type_str == "Str" || type_str.contains("String") {
                                        self.heap_strings.insert(name.clone());
                                        self.variable_types.insert(name.clone(), "Str".to_string());
                                    } else if type_str.starts_with("Struct(")
                                        || self.struct_metadata.contains_key(type_str)
                                    {
                                        let normalized_type = if type_str.starts_with("Struct(") {
                                            type_str.to_string()
                                        } else {
                                            format!("Struct({})", type_str)
                                        };
                                        self.variable_types.insert(name.clone(), normalized_type);
                                        self.heap_arrays.insert(name.clone());
                                    } else {
                                        self.variable_types
                                            .insert(name.clone(), type_str.to_string());
                                    }
                                }
                            }

                            self.temp_values.insert(name.clone(), field_val);
                            return Some(field_val);
                        }
                    } else if source_val.is_pointer_value() {
                        // Direct pointer to tuple (non-Result case)
                        let tuple_ptr = source_val.into_pointer_value();

                        if let Some(tuple_type_str) = self.tuple_types.get(source).cloned() {
                            if let Some(struct_type) = self.tuple_struct_types.get(&tuple_type_str)
                            {
                                // Use struct_gep to get field pointer from heap tuple
                                let field_ptr = self
                                    .builder
                                    .build_struct_gep(
                                        *struct_type,
                                        tuple_ptr,
                                        *index as u32,
                                        &format!("{}_field", name),
                                    )
                                    .unwrap();

                                // Load the field value
                                let field_type =
                                    struct_type.get_field_type_at_index(*index as u32).unwrap();
                                let field_val = self
                                    .builder
                                    .build_load(field_type, field_ptr, name)
                                    .unwrap();

                                // Track metadata
                                let inner = tuple_type_str
                                    .strip_prefix("Tuple(")
                                    .and_then(|s| s.strip_suffix(")"))
                                    .unwrap_or("");
                                let types = crate::codegen::core::helpers::parse_tuple_types(inner);
                                if let Some(type_str) = types.get(*index) {
                                    let type_str = type_str.as_str();

                                    if type_str.starts_with("Array") {
                                        self.heap_arrays.insert(name.clone());
                                        if let Some(elem_type) = type_str
                                            .strip_prefix("Array(")
                                            .and_then(|s| s.strip_suffix(")"))
                                        {
                                            self.array_metadata.insert(
                                                name.clone(),
                                                crate::codegen::ArrayMetadata {
                                                    length: 0,
                                                    element_type: elem_type.to_string(),
                                                    contains_strings: elem_type == "Str",
                                                },
                                            );
                                        }
                                    } else if type_str.starts_with("Map") {
                                        self.heap_maps.insert(name.clone());
                                        if let Some(inner) = type_str
                                            .strip_prefix("Map(")
                                            .and_then(|s| s.strip_suffix(")"))
                                        {
                                            let parts: Vec<&str> = inner.split(',').collect();
                                            if parts.len() == 2 {
                                                let key_type = parts[0].trim().to_string();
                                                let value_type = parts[1].trim().to_string();
                                                self.map_metadata.insert(
                                                    name.clone(),
                                                    crate::codegen::MapMetadata {
                                                        length: 0,
                                                        key_type: key_type.clone(),
                                                        value_type: value_type.clone(),
                                                        key_is_string: key_type == "Str",
                                                        value_is_string: value_type == "Str",
                                                        key_needs_rc: key_type == "Str",
                                                        value_needs_rc: value_type == "Str",
                                                    },
                                                );
                                            }
                                        }
                                    } else if type_str == "Bool" {
                                        self.boolean_temps.insert(name.clone());
                                    } else if type_str == "Str" || type_str.contains("String") {
                                        self.heap_strings.insert(name.clone());
                                        self.variable_types.insert(name.clone(), "Str".to_string());
                                    } else if type_str.starts_with("Struct(")
                                        || self.struct_metadata.contains_key(type_str)
                                    {
                                        // Handle struct types in tuple extraction
                                        let normalized_type = if type_str.starts_with("Struct(") {
                                            type_str.to_string()
                                        } else {
                                            format!("Struct({})", type_str)
                                        };

                                        self.variable_types.insert(name.clone(), normalized_type);
                                        self.heap_arrays.insert(name.clone()); // Track for RC
                                    } else {
                                        // For non-struct types, store the type string
                                        self.variable_types
                                            .insert(name.clone(), type_str.to_string());
                                    }
                                }

                                self.temp_values.insert(name.clone(), field_val);
                                return Some(field_val);
                            }
                        }
                    }
                }

                // Fallback: use resolve_value for non-Result cases
                let source_val = self.resolve_value(source);

                // Check if source has tuple type info
                if let Some(tuple_type_str) = self.tuple_types.get(source).cloned() {
                    // Extract from tuple struct
                    if source_val.is_pointer_value() {
                        let tuple_ptr = source_val.into_pointer_value();

                        // Get the struct type from cache
                        if let Some(struct_type) = self.tuple_struct_types.get(&tuple_type_str) {
                            let field_ptr = self
                                .builder
                                .build_struct_gep(
                                    *struct_type,
                                    tuple_ptr,
                                    *index as u32,
                                    &format!("{}_field", name),
                                )
                                .unwrap();

                            // Determine the type of this field
                            let field_type =
                                struct_type.get_field_type_at_index(*index as u32).unwrap();
                            let field_val = self
                                .builder
                                .build_load(field_type, field_ptr, name)
                                .unwrap();

                            // Track array/map metadata if this field is an array or map
                            let inner = tuple_type_str
                                .strip_prefix("Tuple(")
                                .and_then(|s| s.strip_suffix(")"))
                                .unwrap_or("");
                            let types = crate::codegen::core::helpers::parse_tuple_types(inner);
                            if let Some(type_str) = types.get(*index) {
                                let type_str = type_str.as_str();

                                if type_str.starts_with("Array") {
                                    self.heap_arrays.insert(name.clone());
                                    // Extract element type from Array(Type)
                                    if let Some(elem_type) = type_str
                                        .strip_prefix("Array(")
                                        .and_then(|s| s.strip_suffix(")"))
                                    {
                                        self.array_metadata.insert(
                                            name.clone(),
                                            crate::codegen::ArrayMetadata {
                                                length: 0,
                                                element_type: elem_type.to_string(),
                                                contains_strings: elem_type == "Str",
                                            },
                                        );
                                    }
                                } else if type_str.starts_with("Map") {
                                    self.heap_maps.insert(name.clone());
                                    // Extract key/value types from Map(Key,Value)
                                    if let Some(inner) = type_str
                                        .strip_prefix("Map(")
                                        .and_then(|s| s.strip_suffix(")"))
                                    {
                                        let parts: Vec<&str> = inner.split(',').collect();
                                        if parts.len() == 2 {
                                            let key_type = parts[0].trim().to_string();
                                            let value_type = parts[1].trim().to_string();
                                            self.map_metadata.insert(
                                                name.clone(),
                                                crate::codegen::MapMetadata {
                                                    length: 0,
                                                    key_type: key_type.clone(),
                                                    value_type: value_type.clone(),
                                                    key_is_string: key_type == "Str",
                                                    value_is_string: value_type == "Str",
                                                    key_needs_rc: key_type == "Str",
                                                    value_needs_rc: value_type == "Str",
                                                },
                                            );
                                        }
                                    }
                                } else if type_str == "Bool" {
                                    self.boolean_temps.insert(name.clone());
                                } else if type_str.starts_with("Struct(")
                                    || self.struct_metadata.contains_key(type_str)
                                {
                                    // Handle struct types in tuple extraction
                                    // Normalize to "Struct(Name)" format
                                    let normalized_type = if type_str.starts_with("Struct(") {
                                        type_str.to_string()
                                    } else {
                                        format!("Struct({})", type_str)
                                    };

                                    self.variable_types.insert(name.clone(), normalized_type);
                                    self.heap_arrays.insert(name.clone()); // Track for RC
                                } else {
                                    // For non-struct types, store the type string
                                    self.variable_types
                                        .insert(name.clone(), type_str.to_string());
                                }
                            }

                            self.temp_values.insert(name.clone(), field_val);
                            return Some(field_val);
                        }
                    }
                }

                // Fallback: if no tuple type info, try to extract from struct anyway
                if source_val.is_struct_value() {
                    let struct_val = source_val.into_struct_value();
                    let struct_type = struct_val.get_type();
                    let num_fields = struct_type.count_fields();

                    // Check if this is a Result struct { i32 tag, ptr value }
                    if num_fields == 2 {
                        if let Some(field0_type) = struct_type.get_field_type_at_index(0) {
                            if let BasicTypeEnum::IntType(int_type) = field0_type {
                                if int_type.get_bit_width() == 32 {
                                    // This is a Result struct!
                                    // CRITICAL: Check the tag at runtime - only extract if Ok (tag=0)
                                    let tag = self
                                        .builder
                                        .build_extract_value(struct_val, 0, "result_tag_check")
                                        .unwrap()
                                        .into_int_value();

                                    let is_ok = self
                                        .builder
                                        .build_int_compare(
                                            inkwell::IntPredicate::EQ,
                                            tag,
                                            self.context.i32_type().const_int(0, false),
                                            "is_ok_for_extract",
                                        )
                                        .unwrap();

                                    // Create blocks for Ok and Err cases
                                    let func = self
                                        .builder
                                        .get_insert_block()
                                        .unwrap()
                                        .get_parent()
                                        .unwrap();
                                    let ok_extract_block =
                                        self.context.append_basic_block(func, "extract_ok_value");
                                    let err_extract_block = self
                                        .context
                                        .append_basic_block(func, "extract_err_placeholder");
                                    let continue_block =
                                        self.context.append_basic_block(func, "extract_continue");

                                    self.builder
                                        .build_conditional_branch(
                                            is_ok,
                                            ok_extract_block,
                                            err_extract_block,
                                        )
                                        .unwrap();

                                    // OK block: Extract from tuple
                                    self.builder.position_at_end(ok_extract_block);
                                    let tuple_ptr_value = self
                                        .builder
                                        .build_extract_value(struct_val, 1, "result_tuple_ptr")
                                        .unwrap();

                                    let ok_result: BasicValueEnum = if tuple_ptr_value
                                        .is_pointer_value()
                                    {
                                        let tuple_ptr = tuple_ptr_value.into_pointer_value();

                                        // TYPE-AWARE APPROACH: Use actual result_types to build correct tuple
                                        let tuple_struct_type = if let Some((ok_type_str, _)) =
                                            self.result_types.get(source)
                                        {
                                            // Parse the tuple types from the ok_type_str
                                            // Strip "Tuple(...)" wrapper if present
                                            let inner_types = if ok_type_str.starts_with("Tuple(")
                                                && ok_type_str.ends_with(")")
                                            {
                                                &ok_type_str[6..ok_type_str.len() - 1]
                                            } else {
                                                ok_type_str
                                            };
                                            let types =
                                                crate::codegen::core::helpers::parse_tuple_types(
                                                    inner_types,
                                                );
                                            let tuple_field_types: Vec<
                                                inkwell::types::BasicTypeEnum,
                                            > = types
                                                .iter()
                                                .map(|t| self.map_type_str_to_llvm(t))
                                                .collect();

                                            self.context.struct_type(&tuple_field_types, false)
                                        } else {
                                            // Fallback: use i32 tuple if no type info
                                            let i32_type = self.context.i32_type();
                                            let max_fields = 10;
                                            let tuple_field_types: Vec<
                                                inkwell::types::BasicTypeEnum,
                                            > = vec![i32_type.into(); max_fields];
                                            self.context.struct_type(&tuple_field_types, false)
                                        };

                                        let num_fields = tuple_struct_type.count_fields();

                                        // Use struct GEP to get the field at index
                                        if *index < num_fields as usize {
                                            let field_ptr = self
                                                .builder
                                                .build_struct_gep(
                                                    tuple_struct_type,
                                                    tuple_ptr,
                                                    *index as u32,
                                                    &format!("{}_field_from_result", name),
                                                )
                                                .unwrap();

                                            let field_type = tuple_struct_type
                                                .get_field_type_at_index(*index as u32)
                                                .unwrap();

                                            self.builder
                                                .build_load(field_type, field_ptr, "ok_field")
                                                .unwrap()
                                        } else {
                                            // Fallback to appropriate zero value
                                            self.context.i32_type().const_int(0, false).into()
                                        }
                                    } else {
                                        self.context.i32_type().const_int(0, false).into()
                                    };

                                    self.builder
                                        .build_unconditional_branch(continue_block)
                                        .unwrap();

                                    // ERR block: Return a sentinel value matching the field type
                                    self.builder.position_at_end(err_extract_block);

                                    // Determine the correct type for the error sentinel
                                    let err_result: BasicValueEnum = if let Some((ok_type_str, _)) =
                                        self.result_types.get(source)
                                    {
                                        // Strip "Tuple(...)" wrapper if present
                                        let inner_types = if ok_type_str.starts_with("Tuple(")
                                            && ok_type_str.ends_with(")")
                                        {
                                            &ok_type_str[6..ok_type_str.len() - 1]
                                        } else {
                                            ok_type_str
                                        };
                                        let types =
                                            crate::codegen::core::helpers::parse_tuple_types(
                                                inner_types,
                                            );
                                        if let Some(type_str) = types.get(*index) {
                                            let field_type = self.map_type_str_to_llvm(type_str);
                                            match field_type {
                                                BasicTypeEnum::IntType(int_type) => {
                                                    int_type.const_int(0, false).into()
                                                }
                                                BasicTypeEnum::FloatType(float_type) => {
                                                    float_type.const_float(0.0).into()
                                                }
                                                BasicTypeEnum::PointerType(ptr_type) => {
                                                    ptr_type.const_null().into()
                                                }
                                                _ => self
                                                    .context
                                                    .i32_type()
                                                    .const_int(0, false)
                                                    .into(),
                                            }
                                        } else {
                                            self.context.i32_type().const_int(0, false).into()
                                        }
                                    } else {
                                        self.context.i32_type().const_int(0, false).into()
                                    };

                                    self.builder
                                        .build_unconditional_branch(continue_block)
                                        .unwrap();

                                    // Continue block: Phi node to merge Ok and Err results
                                    self.builder.position_at_end(continue_block);

                                    let phi_type = ok_result.get_type();
                                    let phi = self.builder.build_phi(phi_type, name).unwrap();
                                    phi.add_incoming(&[
                                        (&ok_result, ok_extract_block),
                                        (&err_result, err_extract_block),
                                    ]);

                                    let final_val = phi.as_basic_value();
                                    self.temp_values.insert(name.clone(), final_val);

                                    // Track metadata for the extracted field (after phi merge)
                                    if let Some((ok_type_str, _)) = self.result_types.get(source) {
                                        // Strip "Tuple(...)" wrapper if present
                                        let inner_types = if ok_type_str.starts_with("Tuple(")
                                            && ok_type_str.ends_with(")")
                                        {
                                            &ok_type_str[6..ok_type_str.len() - 1]
                                        } else {
                                            ok_type_str
                                        };
                                        let types =
                                            crate::codegen::core::helpers::parse_tuple_types(
                                                inner_types,
                                            );
                                        if let Some(type_str) = types.get(*index) {
                                            // Normalize struct types to "Struct(Name)" format
                                            if type_str.starts_with("Struct(")
                                                || self.struct_metadata.contains_key(type_str)
                                            {
                                                let normalized_type =
                                                    if type_str.starts_with("Struct(") {
                                                        type_str.to_string()
                                                    } else {
                                                        format!("Struct({})", type_str)
                                                    };
                                                self.variable_types
                                                    .insert(name.clone(), normalized_type);
                                                self.heap_arrays.insert(name.clone());
                                            } else {
                                                self.variable_types
                                                    .insert(name.clone(), type_str.clone());

                                                if type_str == "Bool" {
                                                    self.boolean_temps.insert(name.clone());
                                                }
                                            }
                                        }
                                    }

                                    return Some(final_val);
                                }
                            }
                        }
                    }

                    if *index < num_fields as usize {
                        let field_val = self
                            .builder
                            .build_extract_value(struct_val, *index as u32, name)
                            .unwrap();
                        self.temp_values.insert(name.clone(), field_val);
                        return Some(field_val);
                    } else {
                        eprintln!("ERROR: TupleExtract trying to extract field {} from struct with only {} fields", index, num_fields);
                        eprintln!("  source={}, name={}", source, name);
                        panic!(
                            "ExtractOutOfRange: field {} out of {} fields",
                            index, num_fields
                        );
                    }
                }

                // If source is a pointer to struct, try loading as i32 at the index
                if source_val.is_pointer_value() {
                    // For opaque pointers, we can't determine pointee type
                    // Try to use the source as-is
                    let ptr = source_val.into_pointer_value();

                    // Try to load as generic i32 pointer arithmetic
                    let index_val = self.context.i32_type().const_int(*index as u64, false);
                    let elem_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i32_type(),
                                ptr,
                                &[index_val],
                                &format!("{}_gep", name),
                            )
                            .unwrap()
                    };

                    let field_val = self
                        .builder
                        .build_load(self.context.i32_type(), elem_ptr, name)
                        .unwrap();

                    self.temp_values.insert(name.clone(), field_val);
                    return Some(field_val);
                }

                // Fallback: return zero
                let zero = self.context.i32_type().const_int(0, false);
                self.temp_values.insert(name.clone(), zero.into());
                Some(zero.into())
            }

            MirInstr::TupleGet { name, tuple, index } => {
                // Get the tuple/pair value (should be a pointer to a pair struct from ArrayGet)
                let tuple_val = self.resolve_value(tuple);

                if !tuple_val.is_pointer_value() {
                    // Not a pointer - return a dummy value
                    let dummy = self.context.i32_type().const_int(0, false);
                    self.temp_values.insert(name.clone(), dummy.into());
                    return Some(dummy.into());
                }

                let pair_ptr = tuple_val.into_pointer_value();

                // Find the map metadata by looking up the tuple source variable
                // The tuple variable comes from ArrayGet, which should have map metadata
                let mut found_metadata: Option<&crate::codegen::MapMetadata> = None;
                let mut search_log: Vec<String> = Vec::new();

                // Strategy 1: Look up the source array from ArrayGet tracking
                if let Some(source_array) = self.arrayget_sources.get(tuple) {
                    search_log.push(format!("Strategy 1: ArrayGet source = '{}'", source_array));
                    if let Some(metadata) = self.map_metadata.get(source_array) {
                        found_metadata = Some(metadata);
                        search_log.push(format!(
                            "  ✓ Found metadata for '{}': {}:{}",
                            source_array, metadata.key_type, metadata.value_type
                        ));
                    } else {
                        search_log.push(format!("  ✗ No metadata for '{}'", source_array));
                    }
                }

                // Strategy 2: Try to find metadata directly from the tuple variable name
                if found_metadata.is_none() {
                    search_log.push(format!("Strategy 2: Direct lookup for '{}'", tuple));
                    if let Some(metadata) = self.map_metadata.get(tuple) {
                        found_metadata = Some(metadata);
                        search_log.push(format!(
                            "  ✓ Found metadata: {}:{}",
                            metadata.key_type, metadata.value_type
                        ));
                    } else {
                        search_log.push("  ✗ Not found".to_string());
                    }
                }

                // Strategy 3: Try removing "_array" suffix (e.g., "%45_array" -> "%45")
                if found_metadata.is_none() {
                    let base_name = tuple.trim_end_matches("_array");
                    if base_name != tuple {
                        search_log.push(format!("Strategy 3: Try base name '{}'", base_name));
                        if let Some(metadata) = self.map_metadata.get(base_name) {
                            found_metadata = Some(metadata);
                            search_log.push(format!(
                                "  ✓ Found metadata: {}:{}",
                                metadata.key_type, metadata.value_type
                            ));
                        } else {
                            search_log.push("  ✗ Not found".to_string());
                        }
                    }
                }

                // Strategy 4: Try adding "_array" suffix (e.g., "map1" -> "map1_array")
                if found_metadata.is_none() {
                    let array_name = format!("{}_array", tuple);
                    search_log.push(format!(
                        "Strategy 4: Try with _array suffix '{}'",
                        array_name
                    ));
                    if let Some(metadata) = self.map_metadata.get(&array_name) {
                        found_metadata = Some(metadata);
                        search_log.push(format!(
                            "  ✓ Found metadata: {}:{}",
                            metadata.key_type, metadata.value_type
                        ));
                    } else {
                        search_log.push("  ✗ Not found".to_string());
                    }
                }

                // Strategy 5: Search for any map name that matches or contains this variable
                if found_metadata.is_none() {
                    search_log
                        .push("Strategy 5: Fuzzy search through all map metadata".to_string());
                    for (map_name, metadata) in &self.map_metadata {
                        let tuple_clean = tuple.trim_start_matches('%');
                        let map_clean = map_name.trim_start_matches('%');

                        if map_clean.contains(tuple_clean) || tuple_clean.contains(map_clean) {
                            found_metadata = Some(metadata);
                            search_log.push(format!(
                                "  ✓ Fuzzy match: '{}' contains '{}'",
                                map_name, tuple
                            ));
                            search_log.push(format!(
                                "    Metadata: {}:{}",
                                metadata.key_type, metadata.value_type
                            ));
                            break;
                        }
                    }
                    if found_metadata.is_none() {
                        search_log.push("  ✗ No fuzzy matches found".to_string());
                    }
                }

                let (
                    key_type,
                    val_type,
                    key_is_string,
                    val_is_string,
                    key_needs_rc,
                    value_needs_rc,
                ) = if let Some(metadata) = found_metadata {
                    let k_type = match metadata.key_type.as_str() {
                        "Str" => self
                            .context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into(),
                        "Int" => self.context.i32_type().into(),
                        "Bool" => self.context.bool_type().into(),
                        _ => self.context.i32_type().into(),
                    };
                    let v_type = match metadata.value_type.as_str() {
                        "Str" => self
                            .context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into(),
                        "Int" => self.context.i32_type().into(),
                        "Bool" => self.context.bool_type().into(),
                        _ => self.context.i32_type().into(),
                    };
                    (
                        k_type,
                        v_type,
                        metadata.key_is_string,
                        metadata.value_is_string,
                        metadata.key_needs_rc,
                        metadata.value_needs_rc,
                    )
                } else {
                    // Return dummy values to avoid crash, but this will produce incorrect IR
                    let dummy = self.context.i32_type().const_int(0, false);
                    self.temp_values.insert(name.clone(), dummy.into());
                    return Some(dummy.into());
                };

                // Reconstruct the pair struct type
                let pair_type = self.context.struct_type(&[key_type, val_type], false);

                // Extract the field using struct_gep
                let field_ptr = self
                    .builder
                    .build_struct_gep(pair_type, pair_ptr, *index as u32, &format!("{}_ptr", name))
                    .unwrap();

                // Load the field value
                let field_type = if *index == 0 { key_type } else { val_type };
                let is_string_field = if *index == 0 {
                    key_is_string
                } else {
                    val_is_string
                };
                let needs_rc = if *index == 0 {
                    key_needs_rc
                } else {
                    value_needs_rc
                };

                let field_val = self
                    .builder
                    .build_load(field_type, field_ptr, name)
                    .unwrap();

                // Store in temp_values
                self.temp_values.insert(name.clone(), field_val);

                // Store into existing symbol (allocated by generate_for_map)
                // or create a new one if this is not a loop variable
                if let Some(sym) = self.symbols.get(name) {
                    // Symbol already exists (e.g., loop variable) - reuse it

                    // For map iteration variables, decref old value before storing new one
                    // This prevents memory leaks in loop iterations
                    if is_string_field {
                        // Check if we're in a map loop by checking loop stack
                        let in_map_loop = self.loop_stack.last().map_or(false, |ctx| {
                            matches!(ctx.loop_type, Some(crate::codegen::LoopType::Map { .. }))
                        });

                        if in_map_loop {
                            // Load the old value to check if it needs cleanup
                            let old_val = self
                                .builder
                                .build_load(field_type, sym.ptr, &format!("{}_old", name))
                                .unwrap();

                            if old_val.is_pointer_value() {
                                let old_ptr = old_val.into_pointer_value();

                                // Check if pointer is not null before decref
                                let null_ptr = field_type.into_pointer_type().const_null();
                                let old_int = self
                                    .builder
                                    .build_ptr_to_int(old_ptr, self.context.i64_type(), "old_int")
                                    .unwrap();
                                let null_int = self
                                    .builder
                                    .build_ptr_to_int(null_ptr, self.context.i64_type(), "null_int")
                                    .unwrap();
                                let is_not_null = self
                                    .builder
                                    .build_int_compare(
                                        inkwell::IntPredicate::NE,
                                        old_int,
                                        null_int,
                                        "is_not_null",
                                    )
                                    .unwrap();

                                let current_bb = self.builder.get_insert_block().unwrap();
                                let func = current_bb.get_parent().unwrap();
                                let decref_bb = self.context.append_basic_block(func, "decref_old");
                                let store_bb = self.context.append_basic_block(func, "store_new");

                                self.builder
                                    .build_conditional_branch(is_not_null, decref_bb, store_bb)
                                    .unwrap();

                                // Decref old value
                                self.builder.position_at_end(decref_bb);
                                let rc_header = unsafe {
                                    self.builder.build_in_bounds_gep(
                                        self.context.i8_type(),
                                        old_ptr,
                                        &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                        &format!("{}_old_rc", name),
                                    )
                                }
                                .unwrap();

                                let decref_fn = self.decref_fn.unwrap();
                                self.builder
                                    .build_call(decref_fn, &[rc_header.into()], "")
                                    .unwrap();

                                self.builder.build_unconditional_branch(store_bb).unwrap();

                                // Continue with store
                                self.builder.position_at_end(store_bb);
                            }
                        }
                    }

                    self.builder.build_store(sym.ptr, field_val).unwrap();
                } else {
                    // Symbol doesn't exist - create new alloca in ENTRY BLOCK
                    let current_insert_block = self.builder.get_insert_block().unwrap();
                    let func = current_insert_block.get_parent().unwrap();
                    let entry_block = func.get_first_basic_block().unwrap();

                    // Position at the END of entry block to create alloca
                    if let Some(terminator) = entry_block.get_terminator() {
                        self.builder.position_before(&terminator);
                    } else {
                        self.builder.position_at_end(entry_block);
                    }

                    let alloca = self.builder.build_alloca(field_type, name).unwrap();

                    // Initialize to null/zero if it's a string (pointer type)
                    if is_string_field && field_type.is_pointer_type() {
                        let null_ptr = field_type.into_pointer_type().const_null();
                        self.builder.build_store(alloca, null_ptr).unwrap();
                    }

                    // Restore builder position to where we were
                    self.builder.position_at_end(current_insert_block);

                    self.symbols.insert(
                        name.clone(),
                        crate::codegen::Symbol {
                            ptr: alloca,
                            ty: field_type,
                        },
                    );
                    self.builder.build_store(alloca, field_val).unwrap();

                    // Always mark TupleGet variables as loop-local
                    // TupleGet is used for map iteration (key, value) extraction
                    // These variables are always loop-scoped and should not be cleaned at function level
                    self.loop_local_vars.insert(name.clone());
                }

                // Track if this is a string that needs RC and apply RC increment
                if needs_rc && field_val.is_pointer_value() {
                    self.heap_strings.insert(name.clone());

                    // Apply RC increment for string keys/values
                    let str_ptr = field_val.into_pointer_value();
                    let rc_header = unsafe {
                        self.builder.build_in_bounds_gep(
                            self.context.i8_type(),
                            str_ptr,
                            &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                            &format!("{}_rc_header", name),
                        )
                    }
                    .unwrap();

                    let incref_fn = self.incref_fn.unwrap();
                    self.builder
                        .build_call(incref_fn, &[rc_header.into()], "")
                        .unwrap();
                }

                Some(field_val)
            }

            MirInstr::MapGet { name, map, key } => {
                // eprintln!("[DEBUG] MapGet: name={}, map={}, key={}", name, map, key);
                let map_ptr = self.resolve_value(map).into_pointer_value();
                let key_val = self.resolve_value(key);

                // Get map metadata to determine key and value types
                if let Some(map_metadata_clone) = self.map_metadata.get(map).cloned() {
                    let key_type_str = map_metadata_clone.key_type.clone();
                    let value_type_str = map_metadata_clone.value_type.clone();
                    let key_is_string = map_metadata_clone.key_is_string;
                    let value_is_string = map_metadata_clone.value_is_string;

                    // eprintln!("[DEBUG] Map metadata: key_type={}, value_type={}, key_is_string={}, value_is_string={}",
                    //           key_type_str, value_type_str, key_is_string, value_is_string);

                    let value_type: BasicTypeEnum = match value_type_str.as_str() {
                        "Str" => self
                            .context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into(),
                        "Int" => self.context.i32_type().into(),
                        "Bool" => self.context.bool_type().into(),
                        "Float" => self.context.f64_type().into(),
                        _ => self.context.i32_type().into(),
                    };

                    // Calculate key_type_llvm and pair_type once for consistency
                    let key_type_llvm: BasicTypeEnum = if key_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else if key_type_str == "Float" {
                        self.context.f64_type().into()
                    } else if key_type_str == "Bool" {
                        self.context.bool_type().into()
                    } else {
                        self.context.i32_type().into()
                    };

                    let pair_type = self
                        .context
                        .struct_type(&[key_type_llvm, value_type], false);

                    // Handle different key types
                    let index_val = if key_is_string {
                        // String key: use linear search with strcmp
                        // Maps are stored as arrays of (key, value) pairs
                        // eprintln!("[DEBUG] String-keyed map search starting for map={}, key_val type={:?}", map, key_val.get_type());

                        let key_ptr = key_val.into_pointer_value();
                        // eprintln!("[DEBUG] key_ptr obtained");

                        // Get strcmp function
                        let strcmp_fn = self.module.get_function("strcmp").unwrap_or_else(|| {
                            let i8_ptr_type =
                                self.context.ptr_type(inkwell::AddressSpace::default());
                            let fn_type = self
                                .context
                                .i32_type()
                                .fn_type(&[i8_ptr_type.into(), i8_ptr_type.into()], false);
                            self.module.add_function("strcmp", fn_type, None)
                        });

                        // Get map metadata for length
                        let map_length = map_metadata_clone.length;
                        // eprintln!("[DEBUG] Map length: {}", map_length);

                        // Create blocks for the search loop
                        // eprintln!("[DEBUG] Creating search blocks...");
                        let current_fn = self
                            .builder
                            .get_insert_block()
                            .unwrap()
                            .get_parent()
                            .unwrap();
                        // eprintln!("[DEBUG] Got current function");
                        let loop_block = self
                            .context
                            .append_basic_block(current_fn, "map_search_loop");
                        let found_block = self.context.append_basic_block(current_fn, "map_found");
                        let not_found_block =
                            self.context.append_basic_block(current_fn, "map_not_found");
                        let continue_block =
                            self.context.append_basic_block(current_fn, "map_continue");

                        // Allocate index variable in current block
                        let index_alloca = self
                            .builder
                            .build_alloca(self.context.i32_type(), "search_index")
                            .unwrap();

                        // Initialize index to 0
                        self.builder
                            .build_store(index_alloca, self.context.i32_type().const_int(0, false))
                            .unwrap();
                        self.builder.build_unconditional_branch(loop_block).unwrap();

                        // Loop block: check if index < length
                        self.builder.position_at_end(loop_block);
                        let current_index = self
                            .builder
                            .build_load(self.context.i32_type(), index_alloca, "current_index")
                            .unwrap()
                            .into_int_value();
                        let length_val =
                            self.context.i32_type().const_int(map_length as u64, false);
                        let is_in_bounds = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::ULT,
                                current_index,
                                length_val,
                                "is_in_bounds",
                            )
                            .unwrap();
                        self.builder
                            .build_conditional_branch(is_in_bounds, found_block, not_found_block)
                            .unwrap();

                        // Found block: get the key at current index and compare
                        self.builder.position_at_end(found_block);

                        // Get the key from the pair at current_index
                        // Map is stored as array of {key, value} structs
                        // Use the pair_type calculated earlier

                        let pair_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    pair_type,
                                    map_ptr,
                                    &[current_index],
                                    "pair_ptr",
                                )
                                .unwrap()
                        };

                        // Extract key from pair (index 0 of struct)
                        let stored_key_ptr = self
                            .builder
                            .build_struct_gep(pair_type, pair_ptr, 0, "stored_key_ptr")
                            .unwrap();

                        let stored_key = self
                            .builder
                            .build_load(
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                stored_key_ptr,
                                "stored_key",
                            )
                            .unwrap()
                            .into_pointer_value();

                        // Compare with strcmp
                        let cmp_result = self
                            .builder
                            .build_call(
                                strcmp_fn,
                                &[stored_key.into(), key_ptr.into()],
                                "strcmp_result",
                            )
                            .unwrap()
                            .try_as_basic_value()
                            .left()
                            .unwrap()
                            .into_int_value();

                        let zero = self.context.i32_type().const_int(0, false);
                        let keys_match = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                cmp_result,
                                zero,
                                "keys_match",
                            )
                            .unwrap();

                        // If match, break and use current_index; else increment and continue
                        let increment_block =
                            self.context.append_basic_block(current_fn, "map_increment");
                        self.builder
                            .build_conditional_branch(keys_match, continue_block, increment_block)
                            .unwrap();

                        // Increment block
                        self.builder.position_at_end(increment_block);
                        let next_index = self
                            .builder
                            .build_int_add(
                                current_index,
                                self.context.i32_type().const_int(1, false),
                                "next_index",
                            )
                            .unwrap();
                        self.builder.build_store(index_alloca, next_index).unwrap();
                        self.builder.build_unconditional_branch(loop_block).unwrap();

                        // Not found block: use index 0 as fallback
                        self.builder.position_at_end(not_found_block);
                        self.builder
                            .build_store(index_alloca, self.context.i32_type().const_int(0, false))
                            .unwrap();
                        self.builder
                            .build_unconditional_branch(continue_block)
                            .unwrap();

                        // Continue block: load final index
                        self.builder.position_at_end(continue_block);
                        // eprintln!("[DEBUG] Loading final index from search");
                        self.builder
                            .build_load(self.context.i32_type(), index_alloca, "final_index")
                            .unwrap()
                            .into_int_value()
                    } else if key_type_str == "Float" {
                        // Float key: linear search comparing float values
                        let key_float = key_val.into_float_value();
                        let map_length = map_metadata_clone.length;

                        let current_fn = self
                            .builder
                            .get_insert_block()
                            .unwrap()
                            .get_parent()
                            .unwrap();
                        let loop_block = self
                            .context
                            .append_basic_block(current_fn, "map_search_loop_float");
                        let check_block = self
                            .context
                            .append_basic_block(current_fn, "map_check_float");
                        let not_found_block = self
                            .context
                            .append_basic_block(current_fn, "map_not_found_float");
                        let continue_block = self
                            .context
                            .append_basic_block(current_fn, "map_continue_float");

                        let index_alloca = self
                            .builder
                            .build_alloca(self.context.i32_type(), "search_index_float")
                            .unwrap();

                        self.builder
                            .build_store(index_alloca, self.context.i32_type().const_int(0, false))
                            .unwrap();
                        self.builder.build_unconditional_branch(loop_block).unwrap();

                        self.builder.position_at_end(loop_block);
                        let current_index = self
                            .builder
                            .build_load(self.context.i32_type(), index_alloca, "current_index")
                            .unwrap()
                            .into_int_value();
                        let length_val =
                            self.context.i32_type().const_int(map_length as u64, false);
                        let is_in_bounds = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::ULT,
                                current_index,
                                length_val,
                                "is_in_bounds",
                            )
                            .unwrap();
                        self.builder
                            .build_conditional_branch(is_in_bounds, check_block, not_found_block)
                            .unwrap();

                        self.builder.position_at_end(check_block);
                        let pair_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    pair_type,
                                    map_ptr,
                                    &[current_index],
                                    "pair_ptr_float",
                                )
                                .unwrap()
                        };

                        let stored_key_ptr = self
                            .builder
                            .build_struct_gep(pair_type, pair_ptr, 0, "stored_key_ptr_float")
                            .unwrap();

                        let stored_key = self
                            .builder
                            .build_load(self.context.f64_type(), stored_key_ptr, "stored_key_float")
                            .unwrap()
                            .into_float_value();

                        let keys_match = self
                            .builder
                            .build_float_compare(
                                inkwell::FloatPredicate::OEQ,
                                stored_key,
                                key_float,
                                "keys_match_float",
                            )
                            .unwrap();

                        let increment_block = self
                            .context
                            .append_basic_block(current_fn, "map_increment_float");
                        self.builder
                            .build_conditional_branch(keys_match, continue_block, increment_block)
                            .unwrap();

                        self.builder.position_at_end(increment_block);
                        let next_index = self
                            .builder
                            .build_int_add(
                                current_index,
                                self.context.i32_type().const_int(1, false),
                                "next_index",
                            )
                            .unwrap();
                        self.builder.build_store(index_alloca, next_index).unwrap();
                        self.builder.build_unconditional_branch(loop_block).unwrap();

                        self.builder.position_at_end(not_found_block);
                        self.builder
                            .build_store(index_alloca, self.context.i32_type().const_int(0, false))
                            .unwrap();
                        self.builder
                            .build_unconditional_branch(continue_block)
                            .unwrap();

                        self.builder.position_at_end(continue_block);
                        self.builder
                            .build_load(self.context.i32_type(), index_alloca, "final_index_float")
                            .unwrap()
                            .into_int_value()
                    } else if key_type_str == "Bool" {
                        // Bool key: linear search comparing bool values
                        let key_bool = key_val.into_int_value();
                        let map_length = map_metadata_clone.length;

                        let current_fn = self
                            .builder
                            .get_insert_block()
                            .unwrap()
                            .get_parent()
                            .unwrap();
                        let loop_block = self
                            .context
                            .append_basic_block(current_fn, "map_search_loop_bool");
                        let check_block = self
                            .context
                            .append_basic_block(current_fn, "map_check_bool");
                        let not_found_block = self
                            .context
                            .append_basic_block(current_fn, "map_not_found_bool");
                        let continue_block = self
                            .context
                            .append_basic_block(current_fn, "map_continue_bool");

                        let index_alloca = self
                            .builder
                            .build_alloca(self.context.i32_type(), "search_index_bool")
                            .unwrap();

                        self.builder
                            .build_store(index_alloca, self.context.i32_type().const_int(0, false))
                            .unwrap();
                        self.builder.build_unconditional_branch(loop_block).unwrap();

                        self.builder.position_at_end(loop_block);
                        let current_index = self
                            .builder
                            .build_load(self.context.i32_type(), index_alloca, "current_index")
                            .unwrap()
                            .into_int_value();
                        let length_val =
                            self.context.i32_type().const_int(map_length as u64, false);
                        let is_in_bounds = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::ULT,
                                current_index,
                                length_val,
                                "is_in_bounds",
                            )
                            .unwrap();
                        self.builder
                            .build_conditional_branch(is_in_bounds, check_block, not_found_block)
                            .unwrap();

                        self.builder.position_at_end(check_block);
                        let pair_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    pair_type,
                                    map_ptr,
                                    &[current_index],
                                    "pair_ptr_bool",
                                )
                                .unwrap()
                        };

                        let stored_key_ptr = self
                            .builder
                            .build_struct_gep(pair_type, pair_ptr, 0, "stored_key_ptr_bool")
                            .unwrap();

                        let stored_key = self
                            .builder
                            .build_load(self.context.bool_type(), stored_key_ptr, "stored_key_bool")
                            .unwrap()
                            .into_int_value();

                        let keys_match = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                stored_key,
                                key_bool,
                                "keys_match_bool",
                            )
                            .unwrap();

                        let increment_block = self
                            .context
                            .append_basic_block(current_fn, "map_increment_bool");
                        self.builder
                            .build_conditional_branch(keys_match, continue_block, increment_block)
                            .unwrap();

                        self.builder.position_at_end(increment_block);
                        let next_index = self
                            .builder
                            .build_int_add(
                                current_index,
                                self.context.i32_type().const_int(1, false),
                                "next_index",
                            )
                            .unwrap();
                        self.builder.build_store(index_alloca, next_index).unwrap();
                        self.builder.build_unconditional_branch(loop_block).unwrap();

                        self.builder.position_at_end(not_found_block);
                        self.builder
                            .build_store(index_alloca, self.context.i32_type().const_int(0, false))
                            .unwrap();
                        self.builder
                            .build_unconditional_branch(continue_block)
                            .unwrap();

                        self.builder.position_at_end(continue_block);
                        self.builder
                            .build_load(self.context.i32_type(), index_alloca, "final_index_bool")
                            .unwrap()
                            .into_int_value()
                    } else {
                        // Integer key: linear search comparing int values
                        let key_int = key_val.into_int_value();
                        let map_length = map_metadata_clone.length;

                        let current_fn = self
                            .builder
                            .get_insert_block()
                            .unwrap()
                            .get_parent()
                            .unwrap();
                        let loop_block = self
                            .context
                            .append_basic_block(current_fn, "map_search_loop_int");
                        let check_block =
                            self.context.append_basic_block(current_fn, "map_check_int");
                        let not_found_block = self
                            .context
                            .append_basic_block(current_fn, "map_not_found_int");
                        let continue_block = self
                            .context
                            .append_basic_block(current_fn, "map_continue_int");

                        let index_alloca = self
                            .builder
                            .build_alloca(self.context.i32_type(), "search_index_int")
                            .unwrap();

                        self.builder
                            .build_store(index_alloca, self.context.i32_type().const_int(0, false))
                            .unwrap();
                        self.builder.build_unconditional_branch(loop_block).unwrap();

                        self.builder.position_at_end(loop_block);
                        let current_index = self
                            .builder
                            .build_load(self.context.i32_type(), index_alloca, "current_index")
                            .unwrap()
                            .into_int_value();
                        let length_val =
                            self.context.i32_type().const_int(map_length as u64, false);
                        let is_in_bounds = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::ULT,
                                current_index,
                                length_val,
                                "is_in_bounds",
                            )
                            .unwrap();
                        self.builder
                            .build_conditional_branch(is_in_bounds, check_block, not_found_block)
                            .unwrap();

                        self.builder.position_at_end(check_block);
                        let pair_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    pair_type,
                                    map_ptr,
                                    &[current_index],
                                    "pair_ptr_int",
                                )
                                .unwrap()
                        };

                        let stored_key_ptr = self
                            .builder
                            .build_struct_gep(pair_type, pair_ptr, 0, "stored_key_ptr_int")
                            .unwrap();

                        let stored_key = self
                            .builder
                            .build_load(self.context.i32_type(), stored_key_ptr, "stored_key_int")
                            .unwrap()
                            .into_int_value();

                        let keys_match = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                stored_key,
                                key_int,
                                "keys_match_int",
                            )
                            .unwrap();

                        let increment_block = self
                            .context
                            .append_basic_block(current_fn, "map_increment_int");
                        self.builder
                            .build_conditional_branch(keys_match, continue_block, increment_block)
                            .unwrap();

                        self.builder.position_at_end(increment_block);
                        let next_index = self
                            .builder
                            .build_int_add(
                                current_index,
                                self.context.i32_type().const_int(1, false),
                                "next_index",
                            )
                            .unwrap();
                        self.builder.build_store(index_alloca, next_index).unwrap();
                        self.builder.build_unconditional_branch(loop_block).unwrap();

                        self.builder.position_at_end(not_found_block);
                        self.builder
                            .build_store(index_alloca, self.context.i32_type().const_int(0, false))
                            .unwrap();
                        self.builder
                            .build_unconditional_branch(continue_block)
                            .unwrap();

                        self.builder.position_at_end(continue_block);
                        self.builder
                            .build_load(self.context.i32_type(), index_alloca, "final_index_int")
                            .unwrap()
                            .into_int_value()
                    };

                    // Maps are stored as arrays of (key, value) pairs
                    // We need to access the pair at index_val and extract the value
                    // pair_type was already calculated earlier, so we can use it directly

                    // Get pointer to the pair at index_val
                    let pair_ptr = unsafe {
                        self.builder.build_in_bounds_gep(
                            pair_type,
                            map_ptr,
                            &[index_val],
                            "pair_ptr",
                        )
                    }
                    .unwrap();

                    // Extract the value field (index 1) from the pair
                    let value_ptr = self
                        .builder
                        .build_struct_gep(pair_type, pair_ptr, 1, "value_ptr")
                        .unwrap();

                    let elem_val = self
                        .builder
                        .build_load(value_type, value_ptr, "elem_val")
                        .unwrap();

                    let result_val = elem_val;

                    // Track the type of this result
                    self.variable_types
                        .insert(name.clone(), value_type_str.clone());

                    // Track if this is a string value
                    if value_is_string {
                        self.heap_strings.insert(name.clone());
                    }

                    // Handle RC for string values
                    // NOTE: Map string values might be constants without RC headers, so skip RC operations
                    // The strings are owned by the map itself and will be cleaned up with the map
                    // if value_is_string && value_type.is_pointer_type() {
                    //     let str_ptr = result_val.into_pointer_value();
                    //     let rc_header = unsafe {
                    //         self.builder.build_in_bounds_gep(
                    //             self.context.i8_type(),
                    //             str_ptr,
                    //             &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                    //             "rc_header",
                    //         )
                    //     }
                    //     .unwrap();

                    //     if let Some(incref_fn) = self.incref_fn {
                    //         self.builder
                    //             .build_call(incref_fn, &[rc_header.into()], "")
                    //             .unwrap();
                    //     }
                    // }

                    // Store in temp_values
                    self.temp_values.insert(name.clone(), result_val);

                    if let Some(sym) = self.symbols.get(name) {
                        self.builder.build_store(sym.ptr, result_val).unwrap();
                    }

                    // eprintln!("[DEBUG] MapGet result stored for {}", name);
                    Some(result_val)
                } else {
                    // eprintln!("[DEBUG] MapGet: No metadata found for map {}", map);
                    // Fallback: return 0
                    let default = self.context.i32_type().const_int(0, false);
                    self.temp_values.insert(name.clone(), default.into());
                    Some(default.into())
                }
            }

            // Array element assignment: arr[index] = value
            MirInstr::ArraySet {
                array,
                index,
                value,
            } => {
                let array_ptr = self.resolve_value(array).into_pointer_value();
                let index_val = self.resolve_value(index).into_int_value();
                let value_val = self.resolve_value(value);

                // Get array metadata
                if let Some(metadata) = self.array_metadata.get(array).cloned() {
                    let elem_type = self.get_array_element_type(array);

                    let array_len = metadata.length as u32;
                    let array_type = elem_type.array_type(array_len);

                    // Cast data pointer to array pointer
                    let typed_array_ptr = self
                        .builder
                        .build_pointer_cast(
                            array_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "array_ptr_typed",
                        )
                        .unwrap();

                    // GEP to get element pointer
                    let elem_ptr = unsafe {
                        self.builder.build_gep(
                            array_type,
                            typed_array_ptr,
                            &[self.context.i32_type().const_zero(), index_val],
                            "elem_ptr",
                        )
                    }
                    .unwrap();

                    // Store value at element pointer
                    self.builder.build_store(elem_ptr, value_val).unwrap();

                    None
                } else {
                    None
                }
            }

            // Map element assignment: map[key] = value
            MirInstr::MapSet { map, key, value } => {
                let map_ptr = self.resolve_value(map).into_pointer_value();
                let key_val = self.resolve_value(key);
                let value_val = self.resolve_value(value);

                // Get map metadata
                if let Some(map_metadata) = self.map_metadata.get(map).cloned() {
                    let value_type: BasicTypeEnum = match map_metadata.value_type.as_str() {
                        "Str" => self
                            .context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into(),
                        "Int" => self.context.i32_type().into(),
                        "Bool" => self.context.bool_type().into(),
                        "Float" => self.context.f64_type().into(),
                        _ => self.context.i32_type().into(),
                    };

                    // For now, simplified implementation: use the key_val as an index
                    let index_val = key_val.into_int_value();

                    // GEP to get element pointer in map values array
                    let elem_ptr = unsafe {
                        self.builder.build_in_bounds_gep(
                            value_type,
                            map_ptr,
                            &[index_val],
                            "elem_ptr",
                        )
                    }
                    .unwrap();

                    // Store value at element pointer
                    self.builder.build_store(elem_ptr, value_val).unwrap();

                    None
                } else {
                    None
                }
            }

            // Result/Error handling: Ok expression creates a success result
            MirInstr::ResultOk { name, values } => {
                // Create a Result struct with tag=0 (Ok) and the value(s)
                // NEW APPROACH: Don't force through i64, keep actual types
                let ok_types: Vec<String> = values
                    .iter()
                    .map(|v| {
                        self.variable_types
                            .get(v)
                            .cloned()
                            .unwrap_or_else(|| "Unknown".to_string())
                    })
                    .collect();

                // Use the actual error type from the current function, not hardcoded "Str"
                let err_type = self
                    .current_error_type
                    .clone()
                    .unwrap_or_else(|| "Str".to_string());
                let ok_type = if ok_types.len() == 1 {
                    ok_types[0].clone()
                } else {
                    format!("Tuple({})", ok_types.join(","))
                };

                self.result_types
                    .insert(name.clone(), (ok_type.clone(), err_type));
                self.result_values
                    .insert(name.clone(), (true, values.join(",")));
                self.variable_types
                    .insert(name.clone(), "Result".to_string());

                if values.is_empty() {
                    // No value (void Ok) - create Result struct with tag=0 and null pointer
                    let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                    let struct_type = self
                        .context
                        .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false);

                    let struct_alloca = self
                        .builder
                        .build_alloca(struct_type, "result_void_ok")
                        .unwrap();

                    // Set tag = 0 (Ok)
                    let tag_ptr = self
                        .builder
                        .build_struct_gep(struct_type, struct_alloca, 0, "tag_ptr")
                        .unwrap();
                    self.builder
                        .build_store(tag_ptr, self.context.i32_type().const_int(0, false))
                        .unwrap();

                    // Set value to null pointer (void)
                    let value_ptr_field = self
                        .builder
                        .build_struct_gep(struct_type, struct_alloca, 1, "value_ptr")
                        .unwrap();
                    self.builder
                        .build_store(value_ptr_field, ptr_type.const_null())
                        .unwrap();

                    // Load and return the struct
                    let result_struct = self
                        .builder
                        .build_load(struct_type, struct_alloca, "result_void_struct")
                        .unwrap();

                    self.temp_values.insert(name.clone(), result_struct);
                    Some(result_struct)
                } else if values.len() == 1 {
                    // CRITICAL FIX: Use ptrtoint for primitives, keep pointers as-is
                    // This avoids boxing and makes Ok/Err symmetric
                    let value = self.resolve_value(&values[0]);

                    // Convert value to pointer representation
                    let value_ptr = if value.is_pointer_value() {
                        // Already a pointer (string, array, map)
                        value.into_pointer_value()
                    } else if value.is_int_value() {
                        // Cast integer to pointer using inttoptr
                        let int_val = value.into_int_value();
                        let int_64 = if int_val.get_type().get_bit_width() == 64 {
                            int_val
                        } else {
                            self.builder
                                .build_int_z_extend(int_val, self.context.i64_type(), "ext")
                                .unwrap()
                        };
                        self.builder
                            .build_int_to_ptr(
                                int_64,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                "int_as_ptr",
                            )
                            .unwrap()
                    } else if value.is_float_value() {
                        // Bitcast float to i64 then to pointer
                        let float_val = value.into_float_value();
                        let alloca = self
                            .builder
                            .build_alloca(self.context.f64_type(), "f_tmp")
                            .unwrap();
                        self.builder.build_store(alloca, float_val).unwrap();
                        let i64_ptr = self
                            .builder
                            .build_pointer_cast(
                                alloca,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                "i64_ptr",
                            )
                            .unwrap();
                        let i64_val = self
                            .builder
                            .build_load(self.context.i64_type(), i64_ptr, "f_as_i64")
                            .unwrap()
                            .into_int_value();
                        self.builder
                            .build_int_to_ptr(
                                i64_val,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                "float_as_ptr",
                            )
                            .unwrap()
                    } else {
                        // Fallback: use null pointer
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .const_null()
                    };

                    // Create Result struct: { i32 tag, ptr value }
                    let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                    let struct_type = self
                        .context
                        .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false);

                    let struct_alloca =
                        self.builder.build_alloca(struct_type, "result_ok").unwrap();

                    // Set tag = 0 (Ok)
                    let tag_ptr = self
                        .builder
                        .build_struct_gep(struct_type, struct_alloca, 0, "tag_ptr")
                        .unwrap();
                    self.builder
                        .build_store(tag_ptr, self.context.i32_type().const_int(0, false))
                        .unwrap();

                    // Set value (as pointer)
                    let value_ptr_field = self
                        .builder
                        .build_struct_gep(struct_type, struct_alloca, 1, "value_ptr")
                        .unwrap();
                    self.builder
                        .build_store(value_ptr_field, value_ptr)
                        .unwrap();

                    // Load and return the struct
                    let result_struct = self
                        .builder
                        .build_load(struct_type, struct_alloca, "result_struct")
                        .unwrap();

                    // Store in temp_values so it can be retrieved by Return
                    self.temp_values.insert(name.clone(), result_struct);
                    Some(result_struct)
                } else {
                    // Multiple values - create tuple on heap and return { i32, ptr }
                    let value_vec: Vec<BasicValueEnum> =
                        values.iter().map(|v| self.resolve_value(v)).collect();

                    let value_types: Vec<BasicTypeEnum> =
                        value_vec.iter().map(|v| v.get_type()).collect();

                    let tuple_type = self.context.struct_type(&value_types, false);

                    // NOTE: Don't store tuple_types here - the function call handler
                    // (control_flow.rs) already sets up tuple metadata from function_return_types
                    // ResultOk just creates the actual tuple data structure

                    // Allocate tuple on heap using malloc
                    let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
                        let malloc_type = self
                            .context
                            .ptr_type(inkwell::AddressSpace::default())
                            .fn_type(&[self.context.i64_type().into()], false);
                        self.module.add_function("malloc", malloc_type, None)
                    });

                    let tuple_size = tuple_type.size_of().unwrap();
                    let heap_ptr = self
                        .builder
                        .build_call(malloc_fn, &[tuple_size.into()], "heap_tuple")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    // Store tuple fields into heap memory
                    for (i, val) in value_vec.iter().enumerate() {
                        let field_ptr = self
                            .builder
                            .build_struct_gep(
                                tuple_type,
                                heap_ptr,
                                i as u32,
                                &format!("field_{}", i),
                            )
                            .unwrap();
                        self.builder.build_store(field_ptr, *val).unwrap();
                    }

                    // Create Result struct: { i32 tag, ptr value }
                    let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                    let struct_type = self
                        .context
                        .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false);

                    let struct_alloca =
                        self.builder.build_alloca(struct_type, "result_ok").unwrap();

                    // Set tag = 0 (Ok)
                    let tag_ptr = self
                        .builder
                        .build_struct_gep(struct_type, struct_alloca, 0, "tag_ptr")
                        .unwrap();
                    self.builder
                        .build_store(tag_ptr, self.context.i32_type().const_int(0, false))
                        .unwrap();

                    // Set value (tuple pointer, keep as pointer type)
                    let value_ptr = self
                        .builder
                        .build_struct_gep(struct_type, struct_alloca, 1, "value_ptr")
                        .unwrap();
                    self.builder.build_store(value_ptr, heap_ptr).unwrap();

                    // Load and return the struct
                    let result_struct = self
                        .builder
                        .build_load(struct_type, struct_alloca, "result_struct")
                        .unwrap();

                    // Store in temp_values so it can be retrieved by Return
                    self.temp_values.insert(name.clone(), result_struct);
                    Some(result_struct)
                }
            }

            // Result/Error handling: Err expression creates an error result
            MirInstr::ResultErr { name, error } => {
                // Create a Result struct with tag=1 (Err) and the error value
                // NEW APPROACH: Keep error as pointer (usually string pointer)
                let error_val = self.resolve_value(error);
                self.variable_types
                    .insert(name.clone(), "Result".to_string());

                let error_type = self
                    .variable_types
                    .get(error)
                    .cloned()
                    .unwrap_or_else(|| "Str".to_string());

                self.result_types
                    .insert(name.clone(), ("Unknown".to_string(), error_type.clone()));
                self.result_values
                    .insert(name.clone(), (false, error.clone()));

                // Create Result struct: { i32 tag, ptr error }
                // For Err: tag = 1, keep error as pointer
                let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                let struct_type = self
                    .context
                    .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false);

                let struct_alloca = self
                    .builder
                    .build_alloca(struct_type, "result_err")
                    .unwrap();

                // Set tag = 1 (Err)
                let tag_ptr = self
                    .builder
                    .build_struct_gep(struct_type, struct_alloca, 0, "tag_ptr")
                    .unwrap();
                self.builder
                    .build_store(tag_ptr, self.context.i32_type().const_int(1, false))
                    .unwrap();

                // Set error value (convert to pointer representation)
                let error_ptr_val = if error_val.is_pointer_value() {
                    // Already a pointer (string, array, map, struct)
                    error_val.into_pointer_value()
                } else if error_val.is_int_value() {
                    // Cast integer to pointer using inttoptr
                    let int_val = error_val.into_int_value();
                    let int_64 = if int_val.get_type().get_bit_width() == 64 {
                        int_val
                    } else {
                        self.builder
                            .build_int_z_extend(int_val, self.context.i64_type(), "ext")
                            .unwrap()
                    };
                    self.builder
                        .build_int_to_ptr(
                            int_64,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "int_as_ptr",
                        )
                        .unwrap()
                } else if error_val.is_float_value() {
                    // Bitcast float to i64 then to pointer
                    let float_val = error_val.into_float_value();
                    let alloca = self
                        .builder
                        .build_alloca(self.context.f64_type(), "f_tmp")
                        .unwrap();
                    self.builder.build_store(alloca, float_val).unwrap();
                    let i64_ptr = self
                        .builder
                        .build_pointer_cast(
                            alloca,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "i64_ptr",
                        )
                        .unwrap();
                    let i64_val = self
                        .builder
                        .build_load(self.context.i64_type(), i64_ptr, "f_as_i64")
                        .unwrap()
                        .into_int_value();
                    self.builder
                        .build_int_to_ptr(
                            i64_val,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "float_as_ptr",
                        )
                        .unwrap()
                } else {
                    // Fallback: use null pointer
                    self.context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                };

                let error_ptr = self
                    .builder
                    .build_struct_gep(struct_type, struct_alloca, 1, "error_ptr")
                    .unwrap();
                self.builder.build_store(error_ptr, error_ptr_val).unwrap();

                // Load and return the struct
                let result_struct = self
                    .builder
                    .build_load(struct_type, struct_alloca, "result_struct")
                    .unwrap();

                // Store in temp_values so it can be retrieved by Return
                self.temp_values.insert(name.clone(), result_struct);
                Some(result_struct)
            }

            // Try propagate (?): check result and propagate error if needed
            MirInstr::TryPropagate {
                name,
                result: result_tmp,
                error_block: _error_block,
            } => {
                // Extract the Result struct and check the tag
                let mut result_val = self.resolve_value(result_tmp);

                // CRITICAL FIX: If result_val is a pointer, we need to load the Result struct from it
                // This happens when FFI functions return pointer to Result struct
                if result_val.is_pointer_value() && !result_val.is_struct_value() {
                    let result_ptr = result_val.into_pointer_value();
                    let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                    let result_struct_type = self
                        .context
                        .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false);
                    result_val = self
                        .builder
                        .build_load(result_struct_type, result_ptr, "result_struct_load_try")
                        .expect("Failed to load Result struct from pointer in TryPropagate");
                }

                // Try to load Result struct if not already a struct value (fallback for symbols)
                if !result_val.is_struct_value() {
                    // Check if this is supposed to be a Result type
                    if let Some((ok_type, _err_type)) = self.result_types.get(result_tmp) {
                        // This should be a Result struct but resolve_value didn't return it as such
                        // Try loading it directly from symbols with the correct struct type
                        if let Some(sym) = self.symbols.get(result_tmp) {
                            // Create the Result struct type: { i32, ptr }
                            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                            let result_struct_type = self.context.struct_type(
                                &[self.context.i32_type().into(), ptr_type.into()],
                                false,
                            );

                            // Load as struct
                            result_val = self
                                .builder
                                .build_load(result_struct_type, sym.ptr, "result_struct_reload")
                                .expect("Failed to reload Result struct");
                        }
                    }
                }

                // If result is a struct (Result type), extract tag and value
                if result_val.is_struct_value() {
                    let result_struct = result_val.into_struct_value();

                    // Extract tag (field 0)
                    let tag = self
                        .builder
                        .build_extract_value(result_struct, 0, "result_tag")
                        .unwrap()
                        .into_int_value();

                    // Check if tag == 1 (Err)
                    let is_err = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            tag,
                            self.context.i32_type().const_int(1, false),
                            "is_err",
                        )
                        .unwrap();

                    // Create blocks for error and ok paths
                    let func = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let err_block = self.context.append_basic_block(func, "propagate_err");
                    let ok_block = self.context.append_basic_block(func, "propagate_ok");

                    self.builder
                        .build_conditional_branch(is_err, err_block, ok_block)
                        .unwrap();

                    // Error path: return the error struct as-is
                    self.builder.position_at_end(err_block);
                    self.builder.build_return(Some(&result_struct)).unwrap();

                    // Ok path: extract value (field 1) which is a pointer
                    self.builder.position_at_end(ok_block);
                    let ok_value_ptr = self
                        .builder
                        .build_extract_value(result_struct, 1, "ok_value_ptr")
                        .unwrap()
                        .into_pointer_value();

                    // Get the Ok type from result_types to know how to convert the pointer back
                    // Use same fallback logic as ManualErrorExtract
                    let ok_type = self
                        .result_types
                        .get(result_tmp)
                        .map(|(t, _)| t.clone())
                        .or_else(|| {
                            // Fallback: try to find by name
                            if !name.is_empty() {
                                self.result_types.get(name).map(|(t, _)| t.clone())
                            } else {
                                None
                            }
                        })
                        .or_else(|| {
                            // Last resort: search all result_types for any entry
                            self.result_types.values().next().map(|(t, _)| t.clone())
                        })
                        .unwrap_or_else(|| "Int".to_string());

                    // For void Ok types, don't try to extract a value
                    if ok_type == "Void" || ok_type.is_empty() {
                        // Void result - no value to extract, just continue
                        // Store a dummy i32(0) as a placeholder
                        let void_placeholder = self.context.i32_type().const_int(0, false);
                        self.temp_values
                            .insert(name.clone(), void_placeholder.into());
                        self.variable_types.insert(name.clone(), "Void".to_string());
                        Some(void_placeholder.into())
                    } else {
                        // Convert pointer back to actual value based on type
                        // Check if it's a struct type (either "Struct(Name)" or just a struct name)
                        let is_struct_type = ok_type.contains("Struct(")
                            || self.struct_metadata.contains_key(&ok_type);

                        // Check if it's a tuple type
                        let is_tuple_type = ok_type.starts_with("Tuple(") || ok_type.contains(',');

                        let actual_value = if ok_type.contains("Str")
                            || ok_type.contains("String")
                            || ok_type.contains("Array")
                            || ok_type.contains("Map")
                            || is_struct_type
                            || is_tuple_type
                        {
                            // Already a pointer - use as-is (for strings, arrays, maps, structs, and tuples)
                            ok_value_ptr.into()
                        } else if ok_type.contains("Float") {
                            // Convert pointer to i64 then to f64
                            let i64_val = self
                                .builder
                                .build_ptr_to_int(
                                    ok_value_ptr,
                                    self.context.i64_type(),
                                    "ptr_to_i64",
                                )
                                .unwrap();
                            let alloca = self
                                .builder
                                .build_alloca(self.context.i64_type(), "i64_tmp")
                                .unwrap();
                            self.builder.build_store(alloca, i64_val).unwrap();
                            let f64_ptr = self
                                .builder
                                .build_pointer_cast(
                                    alloca,
                                    self.context.ptr_type(inkwell::AddressSpace::default()),
                                    "f64_ptr",
                                )
                                .unwrap();
                            self.builder
                                .build_load(self.context.f64_type(), f64_ptr, "f64_val")
                                .unwrap()
                        } else {
                            // Int, Bool, or default - convert pointer to i32
                            let i64_val = self
                                .builder
                                .build_ptr_to_int(
                                    ok_value_ptr,
                                    self.context.i64_type(),
                                    "ptr_to_i64",
                                )
                                .unwrap();
                            self.builder
                                .build_int_truncate(i64_val, self.context.i32_type(), "ptr_to_i32")
                                .unwrap()
                                .into()
                        };

                        // Store the unwrapped value
                        self.temp_values.insert(name.clone(), actual_value);

                        // Set the variable type to the Ok type (not Result anymore - it's been unwrapped)
                        // Normalize struct types to "Struct(Name)" format
                        let normalized_type = if is_struct_type && !ok_type.contains("Struct(") {
                            format!("Struct({})", ok_type)
                        } else {
                            ok_type.clone()
                        };
                        self.variable_types.insert(name.clone(), normalized_type);

                        // If this is a struct type, also track it for heap management
                        if is_struct_type {
                            self.heap_arrays.insert(name.clone());
                        }

                        // If this is a tuple type, propagate tuple metadata
                        if is_tuple_type {
                            // Propagate tuple_types and result_types from the source Result
                            if let Some((tuple_ok_type, err_type)) =
                                self.result_types.get(result_tmp).cloned()
                            {
                                // Store the unwrapped value as having the tuple type
                                self.result_types
                                    .insert(name.clone(), (tuple_ok_type.clone(), err_type));
                            }

                            // Propagate tuple_types mapping
                            if let Some(tuple_type_str) = self.tuple_types.get(result_tmp).cloned()
                            {
                                self.tuple_types.insert(name.clone(), tuple_type_str);
                            }
                        }

                        // DO NOT propagate result_types - the unwrapped value is NOT a Result
                        // It's the inner Ok type (Int, Str, etc.)

                        Some(actual_value)
                    }
                } else {
                    // Not a Result struct - this should not happen for properly typed Result functions
                    // Pass through without error checking
                    self.temp_values.insert(name.clone(), result_val);
                    self.variable_types
                        .insert(name.clone(), "Unknown".to_string());

                    Some(result_val)
                }
            }

            // Manual error extraction with ?? operator
            // let a, b ?? err = expr;
            MirInstr::ManualErrorExtract {
                ok_names,
                error_name,
                result: result_tmp,
            } => {
                // Extract the Result struct
                let mut result_val = self.resolve_value(result_tmp);

                // CRITICAL FIX: If result_val is a pointer, we need to load the Result struct from it
                // This happens when FFI functions return pointer to Result struct
                if result_val.is_pointer_value() && !result_val.is_struct_value() {
                    let result_ptr = result_val.into_pointer_value();
                    let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                    let result_struct_type = self
                        .context
                        .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false);
                    result_val = self
                        .builder
                        .build_load(result_struct_type, result_ptr, "result_struct_load")
                        .expect("Failed to load Result struct from pointer");
                }

                // Try to load Result struct if not already a struct value (fallback for symbols)
                if !result_val.is_struct_value() {
                    if let Some((_ok_type, _err_type)) = self.result_types.get(result_tmp) {
                        if let Some(sym) = self.symbols.get(result_tmp) {
                            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                            let result_struct_type = self.context.struct_type(
                                &[self.context.i32_type().into(), ptr_type.into()],
                                false,
                            );
                            result_val = self
                                .builder
                                .build_load(result_struct_type, sym.ptr, "result_struct_reload")
                                .expect("Failed to reload Result struct");
                        }
                    }
                }

                if result_val.is_struct_value() {
                    let result_struct = result_val.into_struct_value();

                    // Extract tag (field 0)
                    let tag = self
                        .builder
                        .build_extract_value(result_struct, 0, "result_tag")
                        .unwrap()
                        .into_int_value();

                    // Check if tag == 0 (Ok) or 1 (Err)
                    let is_ok = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            tag,
                            self.context.i32_type().const_int(0, false),
                            "is_ok",
                        )
                        .unwrap();

                    // Create blocks for ok and err paths
                    let func = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let ok_block = self.context.append_basic_block(func, "manual_ok");
                    let err_block = self.context.append_basic_block(func, "manual_err");
                    let cont_block = self.context.append_basic_block(func, "manual_cont");

                    self.builder
                        .build_conditional_branch(is_ok, ok_block, err_block)
                        .unwrap();

                    // Ok path: extract value(s) and set error to nil (null pointer)
                    self.builder.position_at_end(ok_block);
                    let ok_value_ptr = self
                        .builder
                        .build_extract_value(result_struct, 1, "ok_value_ptr")
                        .unwrap()
                        .into_pointer_value();

                    // Get the Ok type to know how to extract values
                    // Try to find by result_tmp first, then try by ok_names[0] if available
                    let ok_type = self
                        .result_types
                        .get(result_tmp)
                        .map(|(t, _)| t.clone())
                        .or_else(|| {
                            // Fallback: try to find by ok_name
                            if !ok_names.is_empty() {
                                self.result_types.get(&ok_names[0]).map(|(t, _)| t.clone())
                            } else {
                                None
                            }
                        })
                        .or_else(|| {
                            // Last resort: search all result_types for any entry
                            // This handles cases where the temp name doesn't match
                            self.result_types.values().next().map(|(t, _)| t.clone())
                        })
                        .unwrap_or_else(|| "Void".to_string());

                    // Check if Ok type is a tuple
                    let is_tuple = ok_type.starts_with("Tuple(") || ok_names.len() > 1;

                    // Store ok values from Ok path
                    let mut ok_values_from_ok_path: Vec<inkwell::values::BasicValueEnum> =
                        Vec::new();

                    if is_tuple && ok_names.len() > 1 {
                        // Extract tuple fields
                        if let Some(tuple_type_str) = self.tuple_types.get(result_tmp).cloned() {
                            if let Some(struct_type) = self.tuple_struct_types.get(&tuple_type_str)
                            {
                                for (idx, _ok_name) in ok_names.iter().enumerate() {
                                    let field_ptr = self
                                        .builder
                                        .build_struct_gep(
                                            *struct_type,
                                            ok_value_ptr,
                                            idx as u32,
                                            &format!("ok_field_{}", idx),
                                        )
                                        .unwrap();

                                    let field_type =
                                        struct_type.get_field_type_at_index(idx as u32).unwrap();
                                    let field_val = self
                                        .builder
                                        .build_load(
                                            field_type,
                                            field_ptr,
                                            &format!("ok_val_{}", idx),
                                        )
                                        .unwrap();

                                    ok_values_from_ok_path.push(field_val);
                                }
                            }
                        }
                    } else if !ok_names.is_empty() {
                        // Single value
                        let is_struct_type = ok_type.contains("Struct(")
                            || self.struct_metadata.contains_key(&ok_type);

                        // Handle Void type specially - no value to extract
                        if ok_type.contains("Void") {
                            // For Void, push a dummy i32 value (0) since we need something
                            // but it will never be used
                            ok_values_from_ok_path
                                .push(self.context.i32_type().const_int(0, false).into());
                        } else {
                            let actual_value =
                                if ok_type.contains("Str") || ok_type.contains("String") {
                                    // For strings from FFI: ok_value_ptr IS the C string pointer (void* cast)
                                    // The FFI stores the string pointer directly as void*, not pointer-to-pointer
                                    ok_value_ptr.into()
                                } else if ok_type.contains("Array")
                                    || ok_type.contains("Map")
                                    || is_struct_type
                                {
                                    // For arrays, maps, and structs, the pointer is directly usable
                                    ok_value_ptr.into()
                                } else if ok_type.contains("Float") {
                                    let i64_val = self
                                        .builder
                                        .build_ptr_to_int(
                                            ok_value_ptr,
                                            self.context.i64_type(),
                                            "ptr_to_i64",
                                        )
                                        .unwrap();
                                    let alloca = self
                                        .builder
                                        .build_alloca(self.context.i64_type(), "i64_tmp")
                                        .unwrap();
                                    self.builder.build_store(alloca, i64_val).unwrap();
                                    let f64_ptr = self
                                        .builder
                                        .build_pointer_cast(
                                            alloca,
                                            self.context.ptr_type(inkwell::AddressSpace::default()),
                                            "f64_ptr",
                                        )
                                        .unwrap();
                                    self.builder
                                        .build_load(self.context.f64_type(), f64_ptr, "f64_val")
                                        .unwrap()
                                } else if ok_type.contains("Int") {
                                    // For Int, the pointer stores the actual int value
                                    let i64_val = self
                                        .builder
                                        .build_ptr_to_int(
                                            ok_value_ptr,
                                            self.context.i64_type(),
                                            "ptr_to_i64",
                                        )
                                        .unwrap();
                                    self.builder
                                        .build_int_truncate(
                                            i64_val,
                                            self.context.i32_type(),
                                            "ptr_to_i32",
                                        )
                                        .unwrap()
                                        .into()
                                } else {
                                    // Default: convert pointer to int
                                    let i64_val = self
                                        .builder
                                        .build_ptr_to_int(
                                            ok_value_ptr,
                                            self.context.i64_type(),
                                            "ptr_to_i64",
                                        )
                                        .unwrap();
                                    self.builder
                                        .build_int_truncate(
                                            i64_val,
                                            self.context.i32_type(),
                                            "ptr_to_i32",
                                        )
                                        .unwrap()
                                        .into()
                                };

                            ok_values_from_ok_path.push(actual_value);
                        }
                    }

                    // Error variable from Ok path (nil/default value based on error type)
                    let err_from_ok_path: inkwell::values::BasicValueEnum = if error_name != "_" {
                        // Get the error type to determine what "nil" means
                        let err_type = self
                            .result_types
                            .get(result_tmp)
                            .map(|(_, e)| e.clone())
                            .unwrap_or_else(|| "Str".to_string());

                        // Create appropriate nil/default value for error type
                        if err_type.contains("Int") {
                            self.context.i32_type().const_int(0, false).into()
                        } else if err_type.contains("Float") {
                            self.context.f64_type().const_float(0.0).into()
                        } else if err_type.contains("Bool") {
                            self.context.bool_type().const_int(0, false).into()
                        } else {
                            // Str, Array, Map, Struct - use null pointer
                            self.context
                                .ptr_type(inkwell::AddressSpace::default())
                                .const_null()
                                .into()
                        }
                    } else {
                        self.context.i32_type().const_int(0, false).into()
                    };

                    self.builder.build_unconditional_branch(cont_block).unwrap();

                    // Err path: set error variable and set ok values to defaults
                    self.builder.position_at_end(err_block);
                    let err_value_ptr = self
                        .builder
                        .build_extract_value(result_struct, 1, "err_value_ptr")
                        .unwrap()
                        .into_pointer_value();

                    // Error value from Err path - convert pointer back to actual error type
                    let err_from_err_path: inkwell::values::BasicValueEnum = if error_name != "_" {
                        // Get the error type from result_types
                        let err_type = self
                            .result_types
                            .get(result_tmp)
                            .map(|(_, e)| e.clone())
                            .unwrap_or_else(|| "Str".to_string());

                        // Convert pointer back to the actual error type
                        if err_type.contains("Str") || err_type.contains("String") {
                            // String errors: err_value_ptr IS the C string pointer (void* cast)
                            // The FFI stores the string pointer directly as void*, not pointer-to-pointer
                            err_value_ptr.into()
                        } else if err_type.contains("Array") || err_type.contains("Map") {
                            // Array and Map errors are pointers
                            err_value_ptr.into()
                        } else if err_type.contains("Struct(")
                            || self.struct_metadata.contains_key(&err_type)
                        {
                            // Struct errors are pointers
                            err_value_ptr.into()
                        } else if err_type.contains("Int") {
                            // Int errors: convert pointer to int
                            let i64_val = self
                                .builder
                                .build_ptr_to_int(
                                    err_value_ptr,
                                    self.context.i64_type(),
                                    "ptr_to_i64",
                                )
                                .unwrap();
                            self.builder
                                .build_int_truncate(i64_val, self.context.i32_type(), "ptr_to_i32")
                                .unwrap()
                                .into()
                        } else if err_type.contains("Float") {
                            // Float errors: convert pointer to float
                            let i64_val = self
                                .builder
                                .build_ptr_to_int(
                                    err_value_ptr,
                                    self.context.i64_type(),
                                    "ptr_to_i64",
                                )
                                .unwrap();
                            let alloca = self
                                .builder
                                .build_alloca(self.context.i64_type(), "i64_tmp")
                                .unwrap();
                            self.builder.build_store(alloca, i64_val).unwrap();
                            let f64_ptr = self
                                .builder
                                .build_pointer_cast(
                                    alloca,
                                    self.context.ptr_type(inkwell::AddressSpace::default()),
                                    "f64_ptr",
                                )
                                .unwrap();
                            self.builder
                                .build_load(self.context.f64_type(), f64_ptr, "f64_val")
                                .unwrap()
                        } else if err_type.contains("Bool") {
                            // Bool errors: convert pointer to i32, then compare to 0 to get bool
                            let i64_val = self
                                .builder
                                .build_ptr_to_int(
                                    err_value_ptr,
                                    self.context.i64_type(),
                                    "ptr_to_i64",
                                )
                                .unwrap();
                            let i32_val = self
                                .builder
                                .build_int_truncate(i64_val, self.context.i32_type(), "ptr_to_i32")
                                .unwrap();
                            // Convert i32 to bool by comparing to 0
                            self.builder
                                .build_int_compare(
                                    inkwell::IntPredicate::NE,
                                    i32_val,
                                    self.context.i32_type().const_int(0, false),
                                    "i32_to_bool",
                                )
                                .unwrap()
                                .into()
                        } else {
                            // Default: keep as pointer
                            err_value_ptr.into()
                        }
                    } else {
                        self.context.i32_type().const_int(0, false).into()
                    };

                    // Set ok variables to default values (matching types from Ok path)
                    let mut ok_values_from_err_path: Vec<inkwell::values::BasicValueEnum> =
                        Vec::new();
                    for ok_val in ok_values_from_ok_path.iter() {
                        // Create a default value of the same type as the Ok path value
                        let default_val = if ok_val.is_int_value() {
                            self.context.i32_type().const_int(0, false).into()
                        } else if ok_val.is_float_value() {
                            self.context.f64_type().const_float(0.0).into()
                        } else if ok_val.is_pointer_value() {
                            self.context
                                .ptr_type(inkwell::AddressSpace::default())
                                .const_null()
                                .into()
                        } else {
                            self.context.i32_type().const_int(0, false).into()
                        };
                        ok_values_from_err_path.push(default_val);
                    }

                    self.builder.build_unconditional_branch(cont_block).unwrap();

                    // Continue block - merge both paths with phi nodes
                    self.builder.position_at_end(cont_block);

                    // Create phi nodes for ok values
                    for (idx, ok_name) in ok_names.iter().enumerate() {
                        if let (Some(ok_val), Some(err_val)) = (
                            ok_values_from_ok_path.get(idx),
                            ok_values_from_err_path.get(idx),
                        ) {
                            let phi = self.builder.build_phi(ok_val.get_type(), ok_name).unwrap();
                            phi.add_incoming(&[(ok_val, ok_block), (err_val, err_block)]);
                            let phi_val = phi.as_basic_value();
                            self.temp_values.insert(ok_name.clone(), phi_val);
                            self.variable_types
                                .insert(ok_name.clone(), "Int".to_string());
                        }
                    }

                    // Create phi node for error variable
                    if error_name != "_" {
                        // Get the actual error type to determine phi node type
                        let err_type = self
                            .result_types
                            .get(result_tmp)
                            .map(|(_, e)| e.clone())
                            .unwrap_or_else(|| "Str".to_string());

                        // Determine the LLVM type for the phi node based on error type
                        let phi_type: inkwell::types::BasicTypeEnum = if err_type.contains("Int") {
                            self.context.i32_type().into()
                        } else if err_type.contains("Float") {
                            self.context.f64_type().into()
                        } else if err_type.contains("Bool") {
                            self.context.bool_type().into()
                        } else {
                            // Str, Array, Map, Struct - all are pointers
                            self.context
                                .ptr_type(inkwell::AddressSpace::default())
                                .into()
                        };

                        let phi = self.builder.build_phi(phi_type, error_name).unwrap();
                        phi.add_incoming(&[
                            (&err_from_ok_path, ok_block),
                            (&err_from_err_path, err_block),
                        ]);
                        let phi_val = phi.as_basic_value();
                        self.temp_values.insert(error_name.clone(), phi_val);
                        self.variable_types.insert(error_name.clone(), err_type);
                    }
                }

                None
            }

            // Struct initialization: Point { x: 10, y: 20 }
            MirInstr::StructInit {
                name,
                struct_name,
                fields,
            } => {
                // Use the canonical struct type from metadata if available
                let struct_type =
                    if let Some(canonical_type) = self.canonical_struct_types.get(struct_name) {
                        *canonical_type
                    } else {
                        // Fallback: infer from values (for backward compatibility)
                        let field_types: Vec<inkwell::types::BasicTypeEnum> = fields
                            .iter()
                            .map(|(_, value_tmp)| {
                                let val = self.resolve_value(value_tmp);
                                val.get_type()
                            })
                            .collect();

                        let inferred_type = self.context.struct_type(&field_types, false);

                        // Store struct metadata for field lookups
                        let field_names: Vec<String> =
                            fields.iter().map(|(name, _)| name.clone()).collect();
                        let field_type_names: Vec<String> = fields
                            .iter()
                            .enumerate()
                            .map(|(idx, (_, value_tmp))| {
                                // Try to get type from variable_types first
                                if let Some(type_str) = self.variable_types.get(value_tmp) {
                                    return type_str.clone();
                                }
                                // Fall back to inferring from LLVM type
                                let llvm_type = &field_types[idx];
                                match llvm_type {
                                    BasicTypeEnum::IntType(_) => "Int".to_string(),
                                    BasicTypeEnum::FloatType(_) => "Float".to_string(),
                                    BasicTypeEnum::PointerType(_) => "Str".to_string(),
                                    _ => "Unknown".to_string(),
                                }
                            })
                            .collect();

                        // Store the LLVM struct type along with metadata
                        let metadata = crate::codegen::core::context::StructMetadata {
                            field_names: field_names.clone(),
                            field_types: field_type_names,
                        };
                        self.struct_metadata.insert(struct_name.clone(), metadata);

                        inferred_type
                    };

                // Track this struct instance type for printing
                // Store with multiple name variations for robust lookups
                self.struct_instance_types
                    .insert(name.clone(), struct_name.clone());
                // Also store without % prefix if name has it
                if name.starts_with('%') {
                    self.struct_instance_types.insert(
                        name.trim_start_matches('%').to_string(),
                        struct_name.clone(),
                    );
                }
                // Also store with % prefix if name doesn't have it
                if !name.starts_with('%') {
                    self.struct_instance_types
                        .insert(format!("%{}", name), struct_name.clone());
                }

                // Allocate space for the struct on the heap (RC managed)
                let struct_size = struct_type.size_of().unwrap();

                let malloc_fn = self.get_or_declare_malloc();
                let struct_ptr = self
                    .builder
                    .build_call(malloc_fn, &[struct_size.into()], &format!("{}_alloc", name))
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                // Cast to correct struct type
                let typed_ptr = self
                    .builder
                    .build_pointer_cast(
                        struct_ptr,
                        struct_type.ptr_type(inkwell::AddressSpace::default()),
                        &format!("{}_typed", name),
                    )
                    .unwrap();

                // Store each field value in the correct order according to struct declaration
                // We need to reorder fields from the literal to match the canonical field order
                if let Some(metadata) = self.struct_metadata.get(struct_name) {
                    // Use metadata to store fields in the correct order
                    for (canonical_idx, canonical_field_name) in
                        metadata.field_names.iter().enumerate()
                    {
                        // Find this field in the provided fields
                        if let Some((_, value_tmp)) = fields
                            .iter()
                            .find(|(field_name, _)| field_name == canonical_field_name)
                        {
                            let value = self.resolve_value(value_tmp);

                            // Increment reference count for heap-allocated values stored in struct fields
                            // Only incref if the value is tracked as heap-allocated (has RC header)
                            let field_type = metadata.field_types.get(canonical_idx);
                            if let Some(type_str) = field_type {
                                if (type_str.contains("Str")
                                    || type_str.contains("String")
                                    || type_str.contains("Array")
                                    || type_str.contains("Map"))
                                    && value.is_pointer_value()
                                {
                                    // Only incref if this is a heap-allocated value (not a global constant)
                                    let is_heap_value = self.heap_strings.contains(value_tmp)
                                        || self.heap_arrays.contains(value_tmp)
                                        || self.heap_maps.contains(value_tmp);

                                    if is_heap_value {
                                        let ptr = value.into_pointer_value();
                                        let rc_header = unsafe {
                                            self.builder.build_in_bounds_gep(
                                                self.context.i8_type(),
                                                ptr,
                                                &[self
                                                    .context
                                                    .i32_type()
                                                    .const_int((-8_i32) as u64, true)],
                                                "struct_field_rc_header",
                                            )
                                        }
                                        .unwrap();

                                        let incref_fn = self.incref_fn.unwrap();
                                        self.builder
                                            .build_call(incref_fn, &[rc_header.into()], "")
                                            .unwrap();
                                    }
                                }
                            }

                            let field_ptr = self
                                .builder
                                .build_struct_gep(
                                    struct_type,
                                    typed_ptr,
                                    canonical_idx as u32,
                                    &format!("{}_field", canonical_field_name),
                                )
                                .unwrap();
                            self.builder.build_store(field_ptr, value).unwrap();
                        }
                    }
                } else {
                    // Fallback: store in the order provided (for backward compatibility)
                    for (idx, (field_name, value_tmp)) in fields.iter().enumerate() {
                        let value = self.resolve_value(value_tmp);

                        // Increment reference count for heap-allocated values stored in struct fields
                        // Try to get type from metadata if available
                        // Only incref if the value is tracked as heap-allocated (has RC header)
                        if let Some(metadata) = self.struct_metadata.get(struct_name) {
                            let field_type = metadata.field_types.get(idx);
                            if let Some(type_str) = field_type {
                                if (type_str.contains("Str")
                                    || type_str.contains("String")
                                    || type_str.contains("Array")
                                    || type_str.contains("Map"))
                                    && value.is_pointer_value()
                                {
                                    // Only incref if this is a heap-allocated value (not a global constant)
                                    let is_heap_value = self.heap_strings.contains(value_tmp)
                                        || self.heap_arrays.contains(value_tmp)
                                        || self.heap_maps.contains(value_tmp);

                                    if is_heap_value {
                                        let ptr = value.into_pointer_value();
                                        let rc_header = unsafe {
                                            self.builder.build_in_bounds_gep(
                                                self.context.i8_type(),
                                                ptr,
                                                &[self
                                                    .context
                                                    .i32_type()
                                                    .const_int((-8_i32) as u64, true)],
                                                "struct_field_rc_header",
                                            )
                                        }
                                        .unwrap();

                                        let incref_fn = self.incref_fn.unwrap();
                                        self.builder
                                            .build_call(incref_fn, &[rc_header.into()], "")
                                            .unwrap();
                                    }
                                }
                            }
                        }

                        let field_ptr = self
                            .builder
                            .build_struct_gep(
                                struct_type,
                                typed_ptr,
                                idx as u32,
                                &format!("{}_field", field_name),
                            )
                            .unwrap();
                        self.builder.build_store(field_ptr, value).unwrap();
                    }
                }

                // Store the struct pointer
                self.temp_values.insert(name.clone(), typed_ptr.into());
                self.variable_types
                    .insert(name.clone(), format!("Struct({})", struct_name));

                // Store instance metadata mapping this variable to its struct type
                self.variable_types
                    .insert(format!("{}_struct_type", name), struct_name.clone());

                // Track for RC memory management
                self.heap_arrays.insert(name.clone()); // Reuse heap tracking for structs

                Some(typed_ptr.into())
            }

            // Field access: obj.field
            MirInstr::StructGet {
                name,
                struct_instance,
                field,
            } => {
                let struct_ptr = self.resolve_value(struct_instance);

                if !struct_ptr.is_pointer_value() {
                    return None;
                }

                let ptr = struct_ptr.into_pointer_value();

                // Get the struct type from variable_types
                let struct_type_str = self
                    .variable_types
                    .get(struct_instance)
                    .cloned()
                    .unwrap_or_else(|| "Unknown".to_string());

                // Extract struct name from type string "Struct(StructName)" or just "StructName"
                let struct_name =
                    if struct_type_str.starts_with("Struct(") && struct_type_str.ends_with(")") {
                        &struct_type_str[7..struct_type_str.len() - 1]
                    } else if !struct_type_str.is_empty()
                        && struct_type_str != "Unknown"
                        && !struct_type_str.starts_with("Array")
                        && !struct_type_str.starts_with("Map")
                        && !struct_type_str.starts_with("Int")
                        && !struct_type_str.starts_with("Float")
                        && !struct_type_str.starts_with("Bool")
                        && !struct_type_str.starts_with("Str")
                    {
                        // This is likely a struct name without the "Struct(...)" wrapper
                        // This happens when error types are TypeRef instead of Struct
                        &struct_type_str
                    } else {
                        ""
                    };

                // Look up field index from metadata
                let (field_index, field_type_name) =
                    if let Some(metadata) = self.struct_metadata.get(struct_name) {
                        let index = metadata
                            .field_names
                            .iter()
                            .position(|f| f == field)
                            .unwrap_or(0);
                        let type_name = metadata
                            .field_types
                            .get(index)
                            .cloned()
                            .unwrap_or_else(|| "Int".to_string());
                        (index, type_name)
                    } else {
                        (0, "Int".to_string())
                    };
                // Use the canonical struct type if available
                let struct_type =
                    if let Some(canonical_type) = self.canonical_struct_types.get(struct_name) {
                        *canonical_type
                    } else if let Some(metadata) = self.struct_metadata.get(struct_name) {
                        // Fallback: reconstruct from metadata
                        let field_llvm_types: Vec<inkwell::types::BasicTypeEnum> = metadata
                            .field_types
                            .iter()
                            .map(|type_name| {
                                match type_name.as_str() {
                                    "Int" => self.context.i32_type().into(),
                                    "Float" => self.context.f64_type().into(),
                                    "Bool" => self.context.bool_type().into(),
                                    "Str" | "String" => self
                                        .context
                                        .ptr_type(inkwell::AddressSpace::default())
                                        .into(),
                                    _ if self.struct_metadata.contains_key(type_name) => {
                                        // Nested struct - use pointer to the struct
                                        self.context
                                            .ptr_type(inkwell::AddressSpace::default())
                                            .into()
                                    }
                                    _ => self.context.i32_type().into(),
                                }
                            })
                            .collect();
                        self.context.struct_type(&field_llvm_types, false)
                    } else {
                        // Last resort fallback: create a simple struct type
                        self.context
                            .struct_type(&[self.context.i32_type().into()], false)
                    };
                // Get the field LLVM type from the struct type
                let field_llvm_type = struct_type
                    .get_field_type_at_index(field_index as u32)
                    .unwrap_or_else(|| self.context.i32_type().into());

                // Access the field at the correct index
                let field_ptr = self
                    .builder
                    .build_struct_gep(
                        struct_type,
                        ptr,
                        field_index as u32,
                        &format!("{}_field_ptr", field),
                    )
                    .unwrap();

                // Check if the field type is a nested struct
                // If so, load the pointer value (since nested structs are stored as pointers)
                let is_nested_struct = self.struct_metadata.contains_key(&field_type_name);

                let field_val = if is_nested_struct {
                    // For nested structs, the field is a pointer to the struct
                    // We need to load that pointer value
                    self.builder
                        .build_load(field_llvm_type, field_ptr, &format!("{}_load", name))
                        .unwrap()
                } else {
                    // For non-struct types, load the value
                    self.builder
                        .build_load(field_llvm_type, field_ptr, &format!("{}_load", name))
                        .unwrap()
                };

                self.temp_values.insert(name.clone(), field_val);
                self.variable_types
                    .insert(name.clone(), field_type_name.clone());

                Some(field_val)
            }

            // Enum initialization: Direction::North or Status::Active(value)
            MirInstr::EnumInit {
                name,
                enum_name,
                variant,
                value,
            } => {
                // Enum is represented as a tagged union: { i32 tag, ptr payload }
                let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                let enum_type = self
                    .context
                    .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false);

                let enum_alloca = self
                    .builder
                    .build_alloca(enum_type, &format!("{}_enum", name))
                    .unwrap();

                // Set tag (variant index - for now use hash of variant name)
                let tag_value = variant.len() as u32; // Simple hash - TODO: Use proper variant indexing
                let tag_ptr = self
                    .builder
                    .build_struct_gep(enum_type, enum_alloca, 0, "tag_ptr")
                    .unwrap();
                self.builder
                    .build_store(
                        tag_ptr,
                        self.context.i32_type().const_int(tag_value as u64, false),
                    )
                    .unwrap();

                // Set payload
                let payload_ptr_field = self
                    .builder
                    .build_struct_gep(enum_type, enum_alloca, 1, "payload_ptr")
                    .unwrap();

                if let Some(payload_tmp) = value {
                    let payload_val = self.resolve_value(payload_tmp);
                    // Box the payload value
                    let payload_ptr = if payload_val.is_pointer_value() {
                        payload_val.into_pointer_value()
                    } else {
                        // Allocate and store the value
                        let alloca = self
                            .builder
                            .build_alloca(payload_val.get_type(), "payload_alloca")
                            .unwrap();
                        self.builder.build_store(alloca, payload_val).unwrap();
                        alloca
                    };
                    self.builder
                        .build_store(payload_ptr_field, payload_ptr)
                        .unwrap();
                } else {
                    // No payload - store null pointer
                    self.builder
                        .build_store(payload_ptr_field, ptr_type.const_null())
                        .unwrap();
                }

                let enum_val = self
                    .builder
                    .build_load(enum_type, enum_alloca, &format!("{}_load", name))
                    .unwrap();

                self.temp_values.insert(name.clone(), enum_val);
                self.variable_types
                    .insert(name.clone(), format!("Enum({})", enum_name));

                Some(enum_val)
            }

            _ => None,
        };

        self.recursion_depth -= 1;
        result
    }

    /// Propagate array/map metadata from source to destination by checking all possible sources
    pub fn propagate_metadata(&mut self, dest_name: &str, source_name: &str) {
        // Never propagate metadata to loop iteration variables
        // Loop variables are scalar values extracted from arrays/maps, not collections themselves
        if self.is_loop_var(dest_name) {
            return;
        }

        // Try to propagate array metadata directly
        if let Some(metadata) = self.array_metadata.get(source_name).cloned() {
            // Only propagate to the exact destination name, not wild variations
            // This prevents accidental metadata leakage to unrelated variables
            self.array_metadata.insert(dest_name.to_string(), metadata);
            return;
        }

        // Try to propagate map metadata directly
        if let Some(metadata) = self.map_metadata.get(source_name).cloned() {
            self.map_metadata.insert(dest_name.to_string(), metadata);
            return;
        }

        // Try common variations of the source name
        let source_variations = vec![
            source_name.to_string(),
            source_name.trim_end_matches("_array").to_string(),
            format!("{}_array", source_name),
            source_name.trim_start_matches('%').to_string(),
            format!("%{}", source_name),
        ];

        for variation in &source_variations {
            if let Some(metadata) = self.array_metadata.get(variation).cloned() {
                // Only propagate to exact destination name
                self.array_metadata.insert(dest_name.to_string(), metadata);
                return;
            }

            if let Some(metadata) = self.map_metadata.get(variation).cloned() {
                self.map_metadata.insert(dest_name.to_string(), metadata);
                return;
            }
        }

        // Try dest_name variations against all metadata
        let dest_variations = vec![
            dest_name.to_string(),
            dest_name.trim_end_matches("_array").to_string(),
            dest_name.trim_start_matches('%').to_string(),
        ];

        for _ in &dest_variations {
            for source_var in &source_variations {
                if let Some(metadata) = self.array_metadata.get(source_var).cloned() {
                    // Register under ALL dest variations
                    for final_dest in &dest_variations {
                        self.array_metadata
                            .insert(final_dest.to_string(), metadata.clone());
                    }
                    return;
                }
            }
        }

        // Try by pointer equality
        if let Some(source_val) = self.temp_values.get(source_name) {
            if source_val.is_pointer_value() {
                let source_ptr = source_val.into_pointer_value();

                // Search through all array metadata for a matching pointer
                let array_metadata_clone = self.array_metadata.clone();
                for (other_name, metadata) in &array_metadata_clone {
                    if let Some(other_val) = self.temp_values.get(other_name) {
                        if other_val.is_pointer_value()
                            && other_val.into_pointer_value() == source_ptr
                        {
                            // Register under EXTENSIVE variations
                            let dest_base =
                                dest_name.trim_start_matches('%').trim_end_matches("_array");
                            let dest_variations = vec![
                                dest_name.to_string(),
                                dest_name.trim_end_matches("_array").to_string(),
                                dest_name.trim_start_matches('%').to_string(),
                                format!("{}_array", dest_name),
                                format!("{}_array", dest_base),
                                dest_base.to_string(),
                                format!("{}item_array", dest_base),
                                format!("{}item", dest_base),
                            ];

                            for variation in dest_variations {
                                self.array_metadata.insert(variation, metadata.clone());
                            }
                            return;
                        }
                    }
                }

                // Search through map metadata
                let map_metadata_clone = self.map_metadata.clone();
                for (other_name, metadata) in &map_metadata_clone {
                    if let Some(other_val) = self.temp_values.get(other_name) {
                        if other_val.is_pointer_value()
                            && other_val.into_pointer_value() == source_ptr
                        {
                            self.map_metadata
                                .insert(dest_name.to_string(), metadata.clone());
                            return;
                        }
                    }
                }
            }
        }

        // Enhanced fuzzy matching - check both directions and partial matches
        let array_metadata_clone = self.array_metadata.clone();
        for (meta_name, metadata) in &array_metadata_clone {
            let meta_base = meta_name.trim_end_matches("_array").trim_start_matches('%');
            let source_base = source_name
                .trim_end_matches("_array")
                .trim_start_matches('%');
            let dest_base = dest_name.trim_end_matches("_array").trim_start_matches('%');

            // Calculate dest_base_name first
            let dest_base_name = dest_name.trim_start_matches('%').trim_end_matches("_array");

            // STRICT FILTERING: Never propagate to loop item variables
            // Check if the destination is actually a loop iteration variable
            let is_loop_iteration_var = self.is_loop_var(dest_name);

            // Only allow exact base name matches, no substring matching
            let is_exact_match = meta_base == source_base || meta_base == dest_base;

            if !is_loop_iteration_var && is_exact_match {
                // Register under EXTENSIVE variations
                let dest_variations = vec![
                    dest_name.to_string(),
                    dest_name.trim_end_matches("_array").to_string(),
                    dest_name.trim_start_matches('%').to_string(),
                    format!("{}_array", dest_name),
                    format!("{}_array", dest_base_name),
                    dest_base_name.to_string(),
                    format!("{}item_array", dest_base_name),
                    format!("{}item", dest_base_name),
                ];

                for variation in dest_variations {
                    self.array_metadata.insert(variation, metadata.clone());
                }
                return;
            }
        }

        let map_metadata_clone = self.map_metadata.clone();
        for (meta_name, metadata) in &map_metadata_clone {
            let meta_base = meta_name.trim_start_matches('%');
            let source_base = source_name.trim_start_matches('%');

            if meta_base == source_base
                || meta_name.contains(source_name)
                || source_name.contains(meta_name.as_str())
            {
                self.map_metadata
                    .insert(dest_name.to_string(), metadata.clone());
                return;
            }
        }

        // Try loading from symbols and comparing pointers
        if let Some(source_sym) = self.symbols.get(source_name) {
            if let Ok(loaded) =
                self.builder
                    .build_load(source_sym.ty, source_sym.ptr, "propagate_check")
            {
                if loaded.is_pointer_value() {
                    let source_ptr = loaded.into_pointer_value();

                    // Search through all array metadata for a matching pointer
                    let mut found_array_meta: Option<crate::codegen::ArrayMetadata> = None;
                    let array_metadata_clone = self.array_metadata.clone();
                    for (other_name, metadata) in &array_metadata_clone {
                        if let Some(other_val) = self.temp_values.get(other_name) {
                            if other_val.is_pointer_value()
                                && other_val.into_pointer_value() == source_ptr
                            {
                                found_array_meta = Some(metadata.clone());
                                break;
                            }
                        }

                        // Also check symbols
                        if let Some(other_sym) = self.symbols.get(other_name) {
                            if let Ok(other_loaded) = self.builder.build_load(
                                other_sym.ty,
                                other_sym.ptr,
                                "other_propagate",
                            ) {
                                if other_loaded.is_pointer_value()
                                    && other_loaded.into_pointer_value() == source_ptr
                                {
                                    found_array_meta = Some(metadata.clone());
                                    break;
                                }
                            }
                        }
                    }

                    if let Some(metadata) = found_array_meta {
                        // Register under EXTENSIVE variations
                        let dest_base =
                            dest_name.trim_start_matches('%').trim_end_matches("_array");
                        let dest_variations = vec![
                            dest_name.to_string(),
                            dest_name.trim_end_matches("_array").to_string(),
                            dest_name.trim_start_matches('%').to_string(),
                            format!("{}_array", dest_name),
                            format!("{}_array", dest_base),
                            dest_base.to_string(),
                            format!("{}item_array", dest_base),
                            format!("{}item", dest_base),
                        ];

                        for variation in dest_variations {
                            self.array_metadata.insert(variation, metadata.clone());
                        }
                        return;
                    }

                    // Search through map metadata
                    let mut found_map_meta: Option<crate::codegen::MapMetadata> = None;
                    let map_metadata_clone = self.map_metadata.clone();
                    for (other_name, metadata) in &map_metadata_clone {
                        if let Some(other_val) = self.temp_values.get(other_name) {
                            if other_val.is_pointer_value()
                                && other_val.into_pointer_value() == source_ptr
                            {
                                found_map_meta = Some(metadata.clone());
                                break;
                            }
                        }
                    }

                    if let Some(metadata) = found_map_meta {
                        self.map_metadata.insert(dest_name.to_string(), metadata);
                        return;
                    }
                }
            }
        }
    }

    /// Map a type string to LLVM BasicTypeEnum
    pub fn map_type_str_to_llvm(&self, type_str: &str) -> BasicTypeEnum<'ctx> {
        let trimmed = type_str.trim();
        if trimmed.contains("Str") || trimmed.contains("String") {
            self.context
                .ptr_type(inkwell::AddressSpace::default())
                .into()
        } else if trimmed.contains("Array") {
            self.context
                .ptr_type(inkwell::AddressSpace::default())
                .into()
        } else if trimmed.contains("Map") {
            self.context
                .ptr_type(inkwell::AddressSpace::default())
                .into()
        } else if trimmed.contains("Struct(") || self.struct_metadata.contains_key(trimmed) {
            // Structs are passed as pointers
            self.context
                .ptr_type(inkwell::AddressSpace::default())
                .into()
        } else if trimmed.contains("Float") {
            self.context.f64_type().into()
        } else if trimmed.contains("Bool") {
            self.context.bool_type().into()
        } else {
            self.context.i32_type().into()
        }
    }
}
