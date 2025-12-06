use crate::codegen::core::{CodeGen, Symbol};
use inkwell::types::BasicType;
impl<'ctx> CodeGen<'ctx> {
    pub fn generate_load_array_element(
        &mut self,
        dest: &str,
        array: &str,
        index: &str,
    ) -> Option<inkwell::values::BasicValueEnum<'ctx>> {
        let array_ptr = self.resolve_value(array).into_pointer_value();
        let index_val = self.resolve_value(index).into_int_value();

        let is_string = self.array_contains_strings(array);
        let elem_type = self.get_array_element_type(array);

        // Use a large array type for GEP - we use 1000000 as a safe upper bound
        // The actual runtime bounds checking is done by the runtime, not LLVM type system
        // Array layout: [RC (i32)] [Length (i32)] [Elements...] at offset +8
        let array_type = elem_type.array_type(1000000);

        // GEP to get element pointer using the large array type
        // This allows GEP to work with any runtime index value
        let elem_ptr = unsafe {
            self.builder
                .build_gep(array_type, array_ptr, &[index_val], "elem_ptr")
        }
        .unwrap();

        // Load element
        let elem_val = self
            .builder
            .build_load(elem_type, elem_ptr, "elem")
            .unwrap();

        // If it's a heap-allocated string, increment RC
        // Note: arrays with contains_strings=false will skip this block
        if is_string && elem_val.is_pointer_value() {
            let str_ptr = elem_val.into_pointer_value();
            let rc_header = unsafe {
                self.builder.build_in_bounds_gep(
                    self.context.i8_type(),
                    str_ptr,
                    &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                    "rc_header",
                )
            }
            .unwrap();

            let incref = self.incref_fn.unwrap();
            self.builder
                .build_call(incref, &[rc_header.into()], "")
                .unwrap();

            // Mark this variable as heap string for cleanup
            self.heap_strings.insert(dest.to_string());
        }

        // Store in destination variable
        if let Some(symbol) = self.symbols.get(dest) {
            self.builder.build_store(symbol.ptr, elem_val).unwrap();
        } else {
            // Create new variable
            let alloca = self.builder.build_alloca(elem_type, dest).unwrap();
            self.builder.build_store(alloca, elem_val).unwrap();

            self.symbols.insert(
                dest.to_string(),
                Symbol {
                    ptr: alloca,
                    ty: elem_type,
                },
            );
        }

        Some(elem_val)
    }

    pub fn generate_load_map_pair(
        &mut self,
        key_dest: &str,
        val_dest: &str,
        map: &str,
        index: &str,
    ) -> Option<inkwell::values::BasicValueEnum<'ctx>> {
        let map_resolved = self.resolve_value(map);

        // Check if we got a valid pointer
        if !map_resolved.is_pointer_value() {
            // Return None to avoid crash
            return None;
        }

        let map_ptr = map_resolved.into_pointer_value();
        let index_val = self.resolve_value(index).into_int_value();

        let (key_is_string, val_is_string) = self.map_contains_strings(map);
        let (key_needs_rc, val_needs_rc) = self.map_strings_need_rc(map);
        let (key_type, val_type) = self.get_map_types(map);

        // Get the type names from metadata for tracking
        let (key_type_name, val_type_name) = if let Some(metadata) = self.map_metadata.get(map) {
            (metadata.key_type.clone(), metadata.value_type.clone())
        } else {
            ("Int".to_string(), "Int".to_string())
        };
        let pair_type = self.context.struct_type(&[key_type, val_type], false);

        // Get map length for array type
        let map_len = if let Some(metadata) = self.map_metadata.get(map) {
            metadata.length as u32
        } else {
            0
        };

        let map_array_type = pair_type.array_type(map_len);

        // Cast to typed map pointer
        let typed_map_ptr = self
            .builder
            .build_pointer_cast(
                map_ptr,
                self.context.ptr_type(inkwell::AddressSpace::default()),
                "map_ptr_typed",
            )
            .unwrap();

        // GEP to get pair pointer
        let pair_ptr = unsafe {
            self.builder.build_gep(
                map_array_type,
                typed_map_ptr,
                &[self.context.i32_type().const_zero(), index_val],
                "pair_ptr",
            )
        }
        .unwrap();

        // Extract key (field 0)
        let key_ptr = self
            .builder
            .build_struct_gep(pair_type, pair_ptr, 0, "key_ptr")
            .unwrap();
        let mut key_val = self.builder.build_load(key_type, key_ptr, "key").unwrap();

        // Extend bool key from i1 to i32 if needed
        if key_type_name == "Bool" && key_val.is_int_value() {
            let bool_val = key_val.into_int_value();
            if bool_val.get_type().get_bit_width() == 1 {
                key_val = self
                    .builder
                    .build_int_z_extend(bool_val, self.context.i32_type(), "key_bool_ext")
                    .unwrap()
                    .into();
            }
        }

        // Extract value (field 1)
        let val_ptr = self
            .builder
            .build_struct_gep(pair_type, pair_ptr, 1, "val_ptr")
            .unwrap();
        let mut val_val = self.builder.build_load(val_type, val_ptr, "val").unwrap();

        // Extend bool value from i1 to i32 if needed
        if val_type_name == "Bool" && val_val.is_int_value() {
            let bool_val = val_val.into_int_value();
            if bool_val.get_type().get_bit_width() == 1 {
                val_val = self
                    .builder
                    .build_int_z_extend(bool_val, self.context.i32_type(), "val_bool_ext")
                    .unwrap()
                    .into();
            }
        }

        // Handle RC for key if string and needs RC (heap-allocated, not constant)
        if key_needs_rc && key_val.is_pointer_value() {
            let str_ptr = key_val.into_pointer_value();
            let rc_header = unsafe {
                self.builder.build_in_bounds_gep(
                    self.context.i8_type(),
                    str_ptr,
                    &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                    "rc_header",
                )
            }
            .unwrap();
            let incref = self.incref_fn.unwrap();
            self.builder
                .build_call(incref, &[rc_header.into()], "")
                .unwrap();
            self.heap_strings.insert(key_dest.to_string());
        }

        // Handle RC for value if string and needs RC (heap-allocated, not constant)
        if val_needs_rc && val_val.is_pointer_value() {
            let str_ptr = val_val.into_pointer_value();
            let rc_header = unsafe {
                self.builder.build_in_bounds_gep(
                    self.context.i8_type(),
                    str_ptr,
                    &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                    "rc_header",
                )
            }
            .unwrap();
            let incref = self.incref_fn.unwrap();
            self.builder
                .build_call(incref, &[rc_header.into()], "")
                .unwrap();
            self.heap_strings.insert(val_dest.to_string());
        }

        // Store key - recreate symbol if type has changed (e.g., variable reused across loops)
        let needs_new_alloca = if let Some(symbol) = self.symbols.get(key_dest) {
            // Check if the existing symbol has a different type
            symbol.ty != key_type
        } else {
            true
        };

        if needs_new_alloca {
            // Remove old symbol and create new alloca with correct type
            self.symbols.remove(key_dest);
            let alloca = self.builder.build_alloca(key_type, key_dest).unwrap();
            self.builder.build_store(alloca, key_val).unwrap();
            self.symbols.insert(
                key_dest.to_string(),
                Symbol {
                    ptr: alloca,
                    ty: key_type,
                },
            );
        } else {
            // Reuse existing symbol
            let symbol = self.symbols.get(key_dest).unwrap();
            self.builder.build_store(symbol.ptr, key_val).unwrap();
        }

        // Track the key type for proper printing - store in temp_values for immediate use
        // Also store type information so Assign can propagate it correctly
        self.variable_types
            .insert(key_dest.to_string(), key_type_name.clone());

        // Mark as loop-local variable so is_loop_var returns true during print codegen
        self.loop_local_vars.insert(key_dest.to_string());

        // Clean up stale metadata to avoid cross-loop pollution
        if key_is_string {
            // For strings, store the actual pointer value in temp_values
            // This takes precedence in resolve_value
            self.temp_values.insert(key_dest.to_string(), key_val);
            // Remove from boolean_temps if it was there from a previous loop
            self.boolean_temps.remove(key_dest);
        } else if key_type_name == "Bool" {
            // For bools, we need to mark them specially for printing
            // Store the extended i32 value in temp_values
            self.temp_values.insert(key_dest.to_string(), key_val);
            self.boolean_temps.insert(key_dest.to_string());
        } else {
            // For other types (Int, Float), remove from boolean_temps if present
            self.boolean_temps.remove(key_dest);
        }

        // Store value - recreate symbol if type has changed (e.g., variable reused across loops)
        let needs_new_alloca = if let Some(symbol) = self.symbols.get(val_dest) {
            // Check if the existing symbol has a different type
            symbol.ty != val_type
        } else {
            true
        };

        if needs_new_alloca {
            // Remove old symbol and create new alloca with correct type
            self.symbols.remove(val_dest);
            let alloca = self.builder.build_alloca(val_type, val_dest).unwrap();
            self.builder.build_store(alloca, val_val).unwrap();
            self.symbols.insert(
                val_dest.to_string(),
                Symbol {
                    ptr: alloca,
                    ty: val_type,
                },
            );
        } else {
            // Reuse existing symbol
            let symbol = self.symbols.get(val_dest).unwrap();
            self.builder.build_store(symbol.ptr, val_val).unwrap();
        }

        // Track the value type for proper printing - store in temp_values for immediate use
        // Also store type information so Assign can propagate it correctly
        self.variable_types
            .insert(val_dest.to_string(), val_type_name.clone());

        // Mark as loop-local variable so is_loop_var returns true during print codegen
        self.loop_local_vars.insert(val_dest.to_string());

        // Clean up stale metadata to avoid cross-loop pollution
        if val_is_string {
            // For strings, store the actual pointer value in temp_values
            self.temp_values.insert(val_dest.to_string(), val_val);
            // Remove from boolean_temps if it was there from a previous loop
            self.boolean_temps.remove(val_dest);
        } else if val_type_name == "Bool" {
            // For bools, we need to mark them specially for printing
            // Store the extended i32 value in temp_values
            self.temp_values.insert(val_dest.to_string(), val_val);
            self.boolean_temps.insert(val_dest.to_string());
        } else {
            // For other types (Int, Float), remove from boolean_temps if present
            self.boolean_temps.remove(val_dest);
        }

        None
    }

    pub fn generate_map_contains(
        &mut self,
        name: &str,
        map: &str,
        key: &str,
    ) -> Option<inkwell::values::BasicValueEnum<'ctx>> {
        let key_val = self.resolve_value(key);

        // Get map metadata to determine key type
        if let Some(map_metadata_clone) = self.map_metadata.get(map).cloned() {
            let key_type_str = map_metadata_clone.key_type.clone();
            let key_is_string = map_metadata_clone.key_is_string;
            let map_length = map_metadata_clone.length;

            let key_type_llvm: inkwell::types::BasicTypeEnum = if key_is_string {
                self.context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into()
            } else if key_type_str == "Float" {
                self.context.f64_type().into()
            } else if key_type_str == "Bool" {
                // Use i32 for bool keys to simplify comparisons
                self.context.i32_type().into()
            } else {
                self.context.i32_type().into()
            };

            let value_type_str = map_metadata_clone.value_type.clone();
            let value_is_string = map_metadata_clone.value_is_string;
            // IMPORTANT: Use i32 for Bool to match how bools are stored in maps
            // (bools are resolved as i32 in resolve_value, so map struct uses i32)
            let value_type: inkwell::types::BasicTypeEnum = match value_type_str.as_str() {
                "Str" => self
                    .context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into(),
                "Int" => self.context.i32_type().into(),
                "Bool" => self.context.i32_type().into(),
                "Float" => self.context.f64_type().into(),
                _ => self.context.i32_type().into(),
            };

            let pair_type = self
                .context
                .struct_type(&[key_type_llvm, value_type], false);

            // Handle different key types
            if key_is_string {
                // String key: use linear search with strcmp
                let key_ptr = key_val.into_pointer_value();

                // Get strcmp function
                let strcmp_fn = self.module.get_function("strcmp").unwrap_or_else(|| {
                    let i8_ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                    let fn_type = self
                        .context
                        .i32_type()
                        .fn_type(&[i8_ptr_type.into(), i8_ptr_type.into()], false);
                    self.module.add_function("strcmp", fn_type, None)
                });

                // Create blocks for the search loop
                let current_fn = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let loop_block = self.context.append_basic_block(current_fn, "contains_loop");
                let found_block = self
                    .context
                    .append_basic_block(current_fn, "contains_found");
                let not_found_block = self
                    .context
                    .append_basic_block(current_fn, "contains_not_found");
                let continue_block = self
                    .context
                    .append_basic_block(current_fn, "contains_continue");

                // Create index variable in entry block (LLVM requires allocas in entry block)
                let current_block = self.builder.get_insert_block().unwrap();
                let entry_block = current_fn.get_first_basic_block().unwrap();
                if let Some(terminator) = entry_block.get_terminator() {
                    self.builder.position_before(&terminator);
                } else {
                    self.builder.position_at_end(entry_block);
                }
                let index_alloca = self
                    .builder
                    .build_alloca(self.context.i32_type(), "contains_index_str")
                    .unwrap();
                self.builder.position_at_end(current_block);

                self.builder
                    .build_store(index_alloca, self.context.i32_type().const_zero())
                    .unwrap();

                // Jump to loop
                self.builder.build_unconditional_branch(loop_block).unwrap();

                // Loop block: check if index < length
                self.builder.position_at_end(loop_block);
                let current_index = self
                    .builder
                    .build_load(self.context.i32_type(), index_alloca, "idx")
                    .unwrap()
                    .into_int_value();
                let length_val = self.context.i32_type().const_int(map_length as u64, false);
                let cmp = self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::ULT, current_index, length_val, "cmp")
                    .unwrap();

                let check_key_block = self.context.append_basic_block(current_fn, "check_key");
                self.builder
                    .build_conditional_branch(cmp, check_key_block, not_found_block)
                    .unwrap();

                // Check key block: compare current key with search key
                self.builder.position_at_end(check_key_block);

                // CRITICAL: Load map_ptr fresh from symbol in this block to ensure SSA dominance
                // and prevent corruption from stale pointers after many iterations
                let map_ptr = if let Some(symbol) = self.symbols.get(map) {
                    self.builder
                        .build_load(
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            symbol.ptr,
                            "map_ptr_fresh",
                        )
                        .unwrap()
                        .into_pointer_value()
                } else {
                    self.resolve_value(map).into_pointer_value()
                };

                // Build map array type for proper GEP (same as non-string case)
                let map_array_type = pair_type.array_type(map_length as u32);

                let pair_ptr = unsafe {
                    self.builder.build_gep(
                        map_array_type,
                        map_ptr,
                        &[self.context.i32_type().const_zero(), current_index],
                        "pair_ptr",
                    )
                }
                .unwrap();

                let key_ptr_in_map = self
                    .builder
                    .build_struct_gep(pair_type, pair_ptr, 0, "key_ptr")
                    .unwrap();

                let key_in_map = self
                    .builder
                    .build_load(key_type_llvm, key_ptr_in_map, "key_in_map")
                    .unwrap()
                    .into_pointer_value();

                let cmp_result = self
                    .builder
                    .build_call(strcmp_fn, &[key_in_map.into(), key_ptr.into()], "strcmp")
                    .unwrap();
                let cmp_val = cmp_result
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();
                let is_equal = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        cmp_val,
                        self.context.i32_type().const_zero(),
                        "is_equal",
                    )
                    .unwrap();

                let inc_block = self.context.append_basic_block(current_fn, "inc_idx");
                // If equal, jump to found; otherwise increment and loop
                self.builder
                    .build_conditional_branch(is_equal, found_block, inc_block)
                    .unwrap();

                // Increment block: increment index and loop back
                self.builder.position_at_end(inc_block);
                let next_index = self
                    .builder
                    .build_int_add(
                        current_index,
                        self.context.i32_type().const_int(1, false),
                        "next_idx",
                    )
                    .unwrap();
                self.builder.build_store(index_alloca, next_index).unwrap();
                self.builder.build_unconditional_branch(loop_block).unwrap();

                // Found block: jump to continue
                self.builder.position_at_end(found_block);
                self.builder
                    .build_unconditional_branch(continue_block)
                    .unwrap();

                // Not found block: jump to continue
                self.builder.position_at_end(not_found_block);
                self.builder
                    .build_unconditional_branch(continue_block)
                    .unwrap();

                // Continue block: merge results with phi node
                self.builder.position_at_end(continue_block);
                let phi = self
                    .builder
                    .build_phi(self.context.bool_type(), name)
                    .unwrap();
                phi.add_incoming(&[
                    (&self.context.bool_type().const_int(1, false), found_block),
                    (&self.context.bool_type().const_zero(), not_found_block),
                ]);

                let result_val = phi.as_basic_value();
                self.temp_values.insert(name.to_string(), result_val);
                return Some(result_val);
            } else {
                // Non-string key: direct comparison
                let current_fn = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let loop_block = self.context.append_basic_block(current_fn, "contains_loop");
                let check_block = self
                    .context
                    .append_basic_block(current_fn, "contains_check");
                let found_block = self
                    .context
                    .append_basic_block(current_fn, "contains_found");
                let not_found_block = self
                    .context
                    .append_basic_block(current_fn, "contains_not_found");
                let continue_block = self
                    .context
                    .append_basic_block(current_fn, "contains_continue");

                // Create index variable in entry block (LLVM requires allocas in entry block)
                let current_block = self.builder.get_insert_block().unwrap();
                let entry_block = current_fn.get_first_basic_block().unwrap();
                if let Some(terminator) = entry_block.get_terminator() {
                    self.builder.position_before(&terminator);
                } else {
                    self.builder.position_at_end(entry_block);
                }
                let index_alloca = self
                    .builder
                    .build_alloca(self.context.i32_type(), "contains_index_nonstr")
                    .unwrap();
                self.builder.position_at_end(current_block);

                self.builder
                    .build_store(index_alloca, self.context.i32_type().const_zero())
                    .unwrap();

                // Jump to loop
                self.builder.build_unconditional_branch(loop_block).unwrap();

                // Loop block: check if index < length
                self.builder.position_at_end(loop_block);
                let current_index = self
                    .builder
                    .build_load(self.context.i32_type(), index_alloca, "idx")
                    .unwrap()
                    .into_int_value();
                let length_val = self.context.i32_type().const_int(map_length as u64, false);
                let cmp = self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::ULT, current_index, length_val, "cmp")
                    .unwrap();
                self.builder
                    .build_conditional_branch(cmp, check_block, not_found_block)
                    .unwrap();

                // Check block: compare key
                self.builder.position_at_end(check_block);

                // CRITICAL: Load map_ptr fresh from symbol in this block to ensure SSA dominance
                // and prevent corruption from stale pointers after many iterations
                let map_ptr = if let Some(symbol) = self.symbols.get(map) {
                    self.builder
                        .build_load(
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            symbol.ptr,
                            "map_ptr_fresh",
                        )
                        .unwrap()
                        .into_pointer_value()
                } else {
                    self.resolve_value(map).into_pointer_value()
                };

                // Build map array type for proper GEP
                let map_array_type = pair_type.array_type(map_length as u32);

                let pair_ptr = unsafe {
                    self.builder.build_gep(
                        map_array_type,
                        map_ptr,
                        &[self.context.i32_type().const_zero(), current_index],
                        "pair_ptr",
                    )
                }
                .unwrap();

                let key_ptr_in_map = self
                    .builder
                    .build_struct_gep(pair_type, pair_ptr, 0, "key_ptr")
                    .unwrap();
                let key_in_map = self
                    .builder
                    .build_load(key_type_llvm, key_ptr_in_map, "key_in_map")
                    .unwrap();

                let is_equal = if key_type_str == "Float" {
                    self.builder
                        .build_float_compare(
                            inkwell::FloatPredicate::OEQ,
                            key_in_map.into_float_value(),
                            key_val.into_float_value(),
                            "is_equal",
                        )
                        .unwrap()
                } else if key_type_str == "Bool" {
                    // Handle bool keys: extend i1 to i32 if necessary
                    let key_in_map_int = if key_in_map.is_int_value() {
                        let val = key_in_map.into_int_value();
                        if val.get_type().get_bit_width() == 1 {
                            self.builder
                                .build_int_z_extend(val, self.context.i32_type(), "bool_ext_map")
                                .unwrap()
                        } else {
                            val
                        }
                    } else {
                        key_in_map.into_int_value()
                    };

                    let key_val_int = if key_val.is_int_value() {
                        let val = key_val.into_int_value();
                        if val.get_type().get_bit_width() == 1 {
                            self.builder
                                .build_int_z_extend(val, self.context.i32_type(), "bool_ext_key")
                                .unwrap()
                        } else {
                            val
                        }
                    } else {
                        key_val.into_int_value()
                    };

                    self.builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            key_in_map_int,
                            key_val_int,
                            "is_equal",
                        )
                        .unwrap()
                } else {
                    self.builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            key_in_map.into_int_value(),
                            key_val.into_int_value(),
                            "is_equal",
                        )
                        .unwrap()
                };

                // If equal, jump to found; otherwise increment and continue loop
                let inc_block = self.context.append_basic_block(current_fn, "contains_inc");
                self.builder
                    .build_conditional_branch(is_equal, found_block, inc_block)
                    .unwrap();

                // Increment block
                self.builder.position_at_end(inc_block);
                let next_index = self
                    .builder
                    .build_int_add(
                        current_index,
                        self.context.i32_type().const_int(1, false),
                        "next_idx",
                    )
                    .unwrap();
                self.builder.build_store(index_alloca, next_index).unwrap();
                self.builder.build_unconditional_branch(loop_block).unwrap();

                // Found block: return true
                self.builder.position_at_end(found_block);
                self.builder
                    .build_unconditional_branch(continue_block)
                    .unwrap();

                // Not found block: return false
                self.builder.position_at_end(not_found_block);
                self.builder
                    .build_unconditional_branch(continue_block)
                    .unwrap();

                // Continue block: merge results with phi node
                self.builder.position_at_end(continue_block);
                let phi = self
                    .builder
                    .build_phi(self.context.bool_type(), name)
                    .unwrap();
                phi.add_incoming(&[
                    (&self.context.bool_type().const_int(1, false), found_block),
                    (&self.context.bool_type().const_zero(), not_found_block),
                ]);

                let result_val = phi.as_basic_value();
                self.temp_values.insert(name.to_string(), result_val);
                return Some(result_val);
            }
        }

        // Fallback: return false if metadata not found
        let false_val = self.context.bool_type().const_zero();
        self.temp_values.insert(name.to_string(), false_val.into());
        Some(false_val.into())
    }

    pub fn generate_array_contains(
        &mut self,
        name: &str,
        array: &str,
        element: &str,
    ) -> Option<inkwell::values::BasicValueEnum<'ctx>> {
        let element_val = self.resolve_value(element);

        // Get array metadata to determine element type
        if let Some(array_metadata) = self.array_metadata.get(array).cloned() {
            let element_type_str = array_metadata.element_type.clone();

            let element_type_llvm: inkwell::types::BasicTypeEnum = match element_type_str.as_str() {
                "Str" => self
                    .context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into(),
                "Float" => self.context.f64_type().into(),
                "Bool" => self.context.i32_type().into(),
                _ => self.context.i32_type().into(),
            };

            // Create blocks for the search loop
            let current_fn = self
                .builder
                .get_insert_block()
                .unwrap()
                .get_parent()
                .unwrap();
            let loop_block = self
                .context
                .append_basic_block(current_fn, "arr_contains_loop");
            let check_block = self
                .context
                .append_basic_block(current_fn, "arr_contains_check");
            let found_block = self
                .context
                .append_basic_block(current_fn, "arr_contains_found");
            let not_found_block = self
                .context
                .append_basic_block(current_fn, "arr_contains_not_found");
            let continue_block = self
                .context
                .append_basic_block(current_fn, "arr_contains_continue");

            // Create index variable in entry block
            let current_block = self.builder.get_insert_block().unwrap();
            let entry_block = current_fn.get_first_basic_block().unwrap();
            if let Some(terminator) = entry_block.get_terminator() {
                self.builder.position_before(&terminator);
            } else {
                self.builder.position_at_end(entry_block);
            }
            let index_alloca = self
                .builder
                .build_alloca(self.context.i32_type(), "arr_contains_index")
                .unwrap();
            self.builder.position_at_end(current_block);

            self.builder
                .build_store(index_alloca, self.context.i32_type().const_zero())
                .unwrap();

            // Get array length from runtime header
            let array_ptr = if let Some(symbol) = self.symbols.get(array) {
                self.builder
                    .build_load(
                        self.context.ptr_type(inkwell::AddressSpace::default()),
                        symbol.ptr,
                        "array_ptr",
                    )
                    .unwrap()
                    .into_pointer_value()
            } else {
                self.resolve_value(array).into_pointer_value()
            };

            // Read length from header (at offset -4 from data pointer)
            let len_ptr = unsafe {
                self.builder
                    .build_gep(
                        self.context.i8_type(),
                        array_ptr,
                        &[self.context.i32_type().const_int((-4_i32) as u64, true)],
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
            let array_length = self
                .builder
                .build_load(self.context.i32_type(), len_ptr_cast, "array_len")
                .unwrap()
                .into_int_value();

            // Jump to loop
            self.builder.build_unconditional_branch(loop_block).unwrap();

            // Loop block: check if index < length
            self.builder.position_at_end(loop_block);
            let current_index = self
                .builder
                .build_load(self.context.i32_type(), index_alloca, "idx")
                .unwrap()
                .into_int_value();
            let cmp = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::ULT,
                    current_index,
                    array_length,
                    "cmp",
                )
                .unwrap();
            self.builder
                .build_conditional_branch(cmp, check_block, not_found_block)
                .unwrap();

            // Check block: compare current element with search element
            self.builder.position_at_end(check_block);

            // Reload array pointer for SSA dominance
            let array_ptr_fresh = if let Some(symbol) = self.symbols.get(array) {
                self.builder
                    .build_load(
                        self.context.ptr_type(inkwell::AddressSpace::default()),
                        symbol.ptr,
                        "array_ptr_fresh",
                    )
                    .unwrap()
                    .into_pointer_value()
            } else {
                self.resolve_value(array).into_pointer_value()
            };

            let elem_ptr = unsafe {
                self.builder
                    .build_gep(
                        element_type_llvm,
                        array_ptr_fresh,
                        &[current_index],
                        "elem_ptr",
                    )
                    .unwrap()
            };

            let elem_in_array = self
                .builder
                .build_load(element_type_llvm, elem_ptr, "elem_in_array")
                .unwrap();

            let is_equal = if element_type_str == "Str" {
                // String comparison with strcmp
                let strcmp_fn = self.module.get_function("strcmp").unwrap_or_else(|| {
                    let i8_ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                    let fn_type = self
                        .context
                        .i32_type()
                        .fn_type(&[i8_ptr_type.into(), i8_ptr_type.into()], false);
                    self.module.add_function("strcmp", fn_type, None)
                });

                let cmp_result = self
                    .builder
                    .build_call(
                        strcmp_fn,
                        &[
                            elem_in_array.into_pointer_value().into(),
                            element_val.into_pointer_value().into(),
                        ],
                        "strcmp",
                    )
                    .unwrap();
                let cmp_val = cmp_result
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();
                self.builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        cmp_val,
                        self.context.i32_type().const_zero(),
                        "is_equal",
                    )
                    .unwrap()
            } else if element_type_str == "Float" {
                self.builder
                    .build_float_compare(
                        inkwell::FloatPredicate::OEQ,
                        elem_in_array.into_float_value(),
                        element_val.into_float_value(),
                        "is_equal",
                    )
                    .unwrap()
            } else {
                // Int or Bool comparison
                self.builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        elem_in_array.into_int_value(),
                        element_val.into_int_value(),
                        "is_equal",
                    )
                    .unwrap()
            };

            // If equal, jump to found; otherwise increment and continue loop
            let inc_block = self
                .context
                .append_basic_block(current_fn, "arr_contains_inc");
            self.builder
                .build_conditional_branch(is_equal, found_block, inc_block)
                .unwrap();

            // Increment block
            self.builder.position_at_end(inc_block);
            let next_index = self
                .builder
                .build_int_add(
                    current_index,
                    self.context.i32_type().const_int(1, false),
                    "next_idx",
                )
                .unwrap();
            self.builder.build_store(index_alloca, next_index).unwrap();
            self.builder.build_unconditional_branch(loop_block).unwrap();

            // Found block: return true
            self.builder.position_at_end(found_block);
            self.builder
                .build_unconditional_branch(continue_block)
                .unwrap();

            // Not found block: return false
            self.builder.position_at_end(not_found_block);
            self.builder
                .build_unconditional_branch(continue_block)
                .unwrap();

            // Continue block: merge results with phi node
            // Use i32 for boolean result (consistent with rest of compiler)
            self.builder.position_at_end(continue_block);
            let phi = self
                .builder
                .build_phi(self.context.i32_type(), name)
                .unwrap();
            phi.add_incoming(&[
                (&self.context.i32_type().const_int(1, false), found_block),
                (&self.context.i32_type().const_zero(), not_found_block),
            ]);

            let result_val = phi.as_basic_value();

            // CRITICAL: Store result to symbol alloca if one exists
            // This ensures the value is available when the conditional branch reads it
            if let Some(sym) = self.symbols.get(name) {
                self.builder.build_store(sym.ptr, result_val).unwrap();
            }

            self.temp_values.insert(name.to_string(), result_val);
            return Some(result_val);
        }

        // Fallback: return false if metadata not found
        let false_val = self.context.i32_type().const_zero();
        self.temp_values.insert(name.to_string(), false_val.into());
        Some(false_val.into())
    }
}
