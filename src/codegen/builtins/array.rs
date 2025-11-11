use crate::codegen::core::{ArrayMetadata, CodeGen};
use inkwell::types::BasicType;
use inkwell::values::BasicValueEnum;

impl<'ctx> CodeGen<'ctx> {
    pub fn generate_array_method(
        &mut self,
        dest: &str,
        object: &str,
        _object_val: BasicValueEnum<'ctx>,
        method: &str,
        args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        match method {
            "len" => self.generate_array_len(dest, object),
            "push" => {
                // Implement proper push with reallocation
                if let Some(mut metadata) = self.array_metadata.get(object).cloned() {
                    let value_to_push = self.resolve_value(&args[0]);
                    let old_array_ptr = self.resolve_value(object).into_pointer_value();

                    // Read actual runtime length from heap structure instead of using metadata.length
                    // This is crucial for arrays passed to functions where metadata.length may be 0
                    let heap_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                old_array_ptr,
                                &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                "heap_ptr_for_push",
                            )
                            .unwrap()
                    };

                    let len_field_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                heap_ptr,
                                &[self.context.i32_type().const_int(4, false)],
                                "len_field_ptr",
                            )
                            .unwrap()
                    };

                    let len_ptr_cast = self
                        .builder
                        .build_pointer_cast(
                            len_field_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "len_ptr_cast",
                        )
                        .unwrap();

                    let old_length_val = self
                        .builder
                        .build_load(self.context.i32_type(), len_ptr_cast, "runtime_old_len")
                        .unwrap()
                        .into_int_value();

                    // Calculate new length at runtime
                    let new_length_val = self
                        .builder
                        .build_int_add(
                            old_length_val,
                            self.context.i32_type().const_int(1, false),
                            "new_len",
                        )
                        .unwrap();

                    // For array type sizing, we need a constant or estimate
                    // Use a large buffer size to accommodate growth
                    let estimated_capacity = old_length_val
                        .get_zero_extended_constant()
                        .unwrap_or(0)
                        .max(4) as u32
                        + 10; // Allocate extra space
                    let new_length =
                        new_length_val.get_zero_extended_constant().unwrap_or(1) as usize;

                    // Determine element type
                    let elem_type = if metadata.contains_strings {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .as_basic_type_enum()
                    } else {
                        self.context.i32_type().as_basic_type_enum()
                    };

                    // Allocate new array with size for new_length elements
                    let realloc_fn = self.module.get_function("realloc").unwrap_or_else(|| {
                        let fn_type = self
                            .context
                            .ptr_type(inkwell::AddressSpace::default())
                            .fn_type(
                                &[
                                    self.context
                                        .ptr_type(inkwell::AddressSpace::default())
                                        .into(),
                                    self.context.i64_type().into(),
                                ],
                                false,
                            );
                        self.module.add_function("realloc", fn_type, None)
                    });

                    let array_type = elem_type.array_type(estimated_capacity);
                    let array_size = array_type.size_of().unwrap();
                    let header_size = self.context.i64_type().const_int(8, false);
                    let total_size = self
                        .builder
                        .build_int_add(header_size, array_size, "total_size")
                        .unwrap();

                    // Get the original heap pointer (before the data pointer)
                    let old_heap_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                old_array_ptr,
                                &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                "old_heap_ptr",
                            )
                            .unwrap()
                    };

                    // Reallocate
                    let new_heap_ptr = self
                        .builder
                        .build_call(
                            realloc_fn,
                            &[old_heap_ptr.into(), total_size.into()],
                            "new_heap",
                        )
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    // Update RC to 1 (realloc might move memory)
                    let rc_ptr = self
                        .builder
                        .build_pointer_cast(
                            new_heap_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "rc_ptr",
                        )
                        .unwrap();
                    self.builder
                        .build_store(rc_ptr, self.context.i32_type().const_int(1, false))
                        .unwrap();

                    // Update length field
                    let len_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                new_heap_ptr,
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
                        .build_store(len_ptr_cast, new_length_val)
                        .unwrap();

                    // Get new data pointer
                    let new_data_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                new_heap_ptr,
                                &[self.context.i32_type().const_int(8, false)],
                                "new_data_ptr",
                            )
                            .unwrap()
                    };

                    let new_array_ptr = self
                        .builder
                        .build_pointer_cast(
                            new_data_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "new_array_ptr",
                        )
                        .unwrap();

                    // Store the new element at the end (use runtime old_length_val as index)
                    let new_index = old_length_val;

                    if metadata.contains_strings {
                        let element_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    self.context.ptr_type(inkwell::AddressSpace::default()),
                                    new_array_ptr,
                                    &[new_index],
                                    "push_ptr",
                                )
                                .unwrap()
                        };
                        self.builder
                            .build_store(element_ptr, value_to_push)
                            .unwrap();
                    } else {
                        let element_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    self.context.i32_type(),
                                    new_array_ptr,
                                    &[new_index],
                                    "push_ptr",
                                )
                                .unwrap()
                        };
                        self.builder
                            .build_store(element_ptr, value_to_push)
                            .unwrap();
                    }

                    // Update metadata for all name variations
                    metadata.length = new_length;

                    let base_name = object.trim_start_matches('%').trim_end_matches("_array");
                    let name_variations = vec![
                        object.to_string(),
                        object.trim_end_matches("_array").to_string(),
                        object.trim_start_matches('%').to_string(),
                        format!("{}_array", object),
                        format!("{}_array", object.trim_start_matches('%')),
                        format!("{}_array", base_name),
                        base_name.to_string(),
                    ];

                    for variation in &name_variations {
                        self.array_metadata
                            .insert(variation.clone(), metadata.clone());
                    }

                    // Update the temp_values to point to the new array
                    self.temp_values
                        .insert(object.to_string(), new_data_ptr.into());

                    // Update symbol table if object is a variable
                    if let Some(sym) = self.symbols.get(object) {
                        self.builder.build_store(sym.ptr, new_data_ptr).unwrap();
                    }

                    None
                } else {
                    None
                }
            }
            "pop" => {
                if let Some(mut metadata) = self.array_metadata.get(object).cloned() {
                    let array_ptr = self.resolve_value(object).into_pointer_value();

                    // Get the heap pointer (8 bytes before data pointer)
                    let heap_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                array_ptr,
                                &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                "heap_ptr_for_pop",
                            )
                            .unwrap()
                    };

                    // Read length field at offset 4 from heap start
                    let len_field_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                heap_ptr,
                                &[self.context.i32_type().const_int(4, false)],
                                "len_field_ptr_pop",
                            )
                            .unwrap()
                    };

                    let len_ptr_cast = self
                        .builder
                        .build_pointer_cast(
                            len_field_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "len_ptr_cast_pop",
                        )
                        .unwrap();

                    let runtime_len = self
                        .builder
                        .build_load(self.context.i32_type(), len_ptr_cast, "runtime_len_pop")
                        .unwrap()
                        .into_int_value();

                    // Calculate last index (length - 1)
                    let last_index = self
                        .builder
                        .build_int_sub(
                            runtime_len,
                            self.context.i32_type().const_int(1, false),
                            "last_index",
                        )
                        .unwrap();

                    // Get the last element
                    let result = if metadata.contains_strings {
                        let elem_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    self.context.ptr_type(inkwell::AddressSpace::default()),
                                    array_ptr,
                                    &[last_index],
                                    "elem_ptr_pop",
                                )
                                .unwrap()
                        };
                        self.builder
                            .build_load(
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                elem_ptr,
                                "popped_string",
                            )
                            .unwrap()
                    } else {
                        let elem_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    self.context.i32_type(),
                                    array_ptr,
                                    &[last_index],
                                    "elem_ptr_pop",
                                )
                                .unwrap()
                        };
                        self.builder
                            .build_load(self.context.i32_type(), elem_ptr, "popped_int")
                            .unwrap()
                    };

                    // Decrement the length field
                    let new_len = self
                        .builder
                        .build_int_sub(
                            runtime_len,
                            self.context.i32_type().const_int(1, false),
                            "new_len",
                        )
                        .unwrap();
                    self.builder.build_store(len_ptr_cast, new_len).unwrap();

                    // Update metadata compile-time length if known
                    if metadata.length > 0 {
                        metadata.length -= 1;
                        self.array_metadata.insert(object.to_string(), metadata);
                    }

                    self.temp_values.insert(dest.to_string(), result);
                    return Some(result);
                }
                None
            }
            "get" => self.generate_array_get_internal(dest, object, &args[0]),
            "set" => {
                // Implement array.set(index, value)
                if let Some(metadata) = self.array_metadata.get(object) {
                    let index_val = self.resolve_value(&args[0]).into_int_value();
                    let value_val = self.resolve_value(&args[1]);
                    let array_ptr = self.resolve_value(object).into_pointer_value();

                    if metadata.contains_strings {
                        let element_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    self.context.ptr_type(inkwell::AddressSpace::default()),
                                    array_ptr,
                                    &[index_val],
                                    "set_ptr",
                                )
                                .unwrap()
                        };
                        self.builder.build_store(element_ptr, value_val).unwrap();
                    } else {
                        let element_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    self.context.i32_type(),
                                    array_ptr,
                                    &[index_val],
                                    "set_ptr",
                                )
                                .unwrap()
                        };
                        self.builder.build_store(element_ptr, value_val).unwrap();
                    }
                    None
                } else {
                    None
                }
            }
            "contains" => {
                // Implement array contains by iterating through elements
                if let Some(metadata) = self.array_metadata.get(object) {
                    let target_val = self.resolve_value(&args[0]);
                    let array_ptr = self.resolve_value(object).into_pointer_value();
                    let is_string_array = metadata.contains_strings;

                    // Create basic blocks for the loop
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
                    let after_block = self
                        .context
                        .append_basic_block(current_fn, "contains_after");

                    // Initialize counter
                    let counter_ptr = self
                        .builder
                        .build_alloca(self.context.i32_type(), "contains_counter")
                        .unwrap();
                    self.builder
                        .build_store(counter_ptr, self.context.i32_type().const_int(0, false))
                        .unwrap();

                    // Read runtime length from heap structure instead of using static metadata.length
                    let heap_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                array_ptr,
                                &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                "heap_ptr_for_contains",
                            )
                            .unwrap()
                    };

                    let len_field_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                heap_ptr,
                                &[self.context.i32_type().const_int(4, false)],
                                "len_field_ptr",
                            )
                            .unwrap()
                    };

                    let len_ptr_cast = self
                        .builder
                        .build_pointer_cast(
                            len_field_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "len_ptr_cast",
                        )
                        .unwrap();

                    let length = self
                        .builder
                        .build_load(self.context.i32_type(), len_ptr_cast, "runtime_length")
                        .unwrap()
                        .into_int_value();

                    // Jump to loop
                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Loop block: check if counter < length
                    self.builder.position_at_end(loop_block);
                    let counter = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "counter")
                        .unwrap()
                        .into_int_value();
                    let cmp = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::ULT, counter, length, "cmp")
                        .unwrap();
                    self.builder
                        .build_conditional_branch(cmp, check_block, not_found_block)
                        .unwrap();

                    // Check block: load element and compare
                    self.builder.position_at_end(check_block);

                    let equals = if is_string_array {
                        // For string arrays, use strcmp
                        let elem_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    self.context.ptr_type(inkwell::AddressSpace::default()),
                                    array_ptr,
                                    &[counter],
                                    "elem_ptr",
                                )
                                .unwrap()
                        };
                        let elem = self
                            .builder
                            .build_load(
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                elem_ptr,
                                "elem",
                            )
                            .unwrap()
                            .into_pointer_value();

                        // Use strcmp for string comparison
                        let strcmp_fn = self.module.get_function("strcmp").unwrap_or_else(|| {
                            let fn_type = self.context.i32_type().fn_type(
                                &[
                                    self.context
                                        .ptr_type(inkwell::AddressSpace::default())
                                        .into(),
                                    self.context
                                        .ptr_type(inkwell::AddressSpace::default())
                                        .into(),
                                ],
                                false,
                            );
                            self.module.add_function("strcmp", fn_type, None)
                        });

                        let cmp_result = self
                            .builder
                            .build_call(
                                strcmp_fn,
                                &[elem.into(), target_val.into_pointer_value().into()],
                                "strcmp_result",
                            )
                            .unwrap()
                            .try_as_basic_value()
                            .left()
                            .unwrap()
                            .into_int_value();

                        // strcmp returns 0 if equal
                        self.builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                cmp_result,
                                self.context.i32_type().const_int(0, false),
                                "equals",
                            )
                            .unwrap()
                    } else {
                        // For integer arrays
                        let elem_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    self.context.i32_type(),
                                    array_ptr,
                                    &[counter],
                                    "elem_ptr",
                                )
                                .unwrap()
                        };
                        let elem = self
                            .builder
                            .build_load(self.context.i32_type(), elem_ptr, "elem")
                            .unwrap()
                            .into_int_value();
                        self.builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                elem,
                                target_val.into_int_value(),
                                "equals",
                            )
                            .unwrap()
                    };

                    // If equal, jump to found, else increment and continue
                    let continue_block = self
                        .context
                        .append_basic_block(current_fn, "contains_continue");
                    self.builder
                        .build_conditional_branch(equals, found_block, continue_block)
                        .unwrap();

                    // Continue block: increment counter
                    self.builder.position_at_end(continue_block);
                    let next_counter = self
                        .builder
                        .build_int_add(
                            counter,
                            self.context.i32_type().const_int(1, false),
                            "next_counter",
                        )
                        .unwrap();
                    self.builder.build_store(counter_ptr, next_counter).unwrap();
                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Found block
                    self.builder.position_at_end(found_block);
                    let true_val = self.context.i32_type().const_int(1, false);
                    self.builder
                        .build_unconditional_branch(after_block)
                        .unwrap();

                    // Not found block
                    self.builder.position_at_end(not_found_block);
                    let false_val = self.context.i32_type().const_int(0, false);
                    self.builder
                        .build_unconditional_branch(after_block)
                        .unwrap();

                    // After block: phi node to get result
                    self.builder.position_at_end(after_block);
                    let phi = self
                        .builder
                        .build_phi(self.context.i32_type(), "contains_result")
                        .unwrap();
                    phi.add_incoming(&[(&true_val, found_block), (&false_val, not_found_block)]);
                    let result = phi.as_basic_value();

                    self.temp_values.insert(dest.to_string(), result);
                    Some(result)
                } else {
                    let result = self.context.i32_type().const_int(0, false);
                    self.temp_values.insert(dest.to_string(), result.into());
                    Some(result.into())
                }
            }
            "first" => {
                if let Some(metadata) = self.array_metadata.get(object) {
                    if metadata.length > 0 {
                        let first_idx = self.context.i32_type().const_int(0, false);
                        let first_idx_str = format!("{}_first_idx", dest);
                        self.temp_values
                            .insert(first_idx_str.clone(), first_idx.into());
                        return self.generate_array_get_internal(dest, object, &first_idx_str);
                    }
                }
                None
            }
            "last" => {
                if let Some(metadata) = self.array_metadata.get(object) {
                    if metadata.length > 0 {
                        let last_idx = self
                            .context
                            .i32_type()
                            .const_int((metadata.length - 1) as u64, false);
                        let last_idx_str = format!("{}_last_idx", dest);
                        self.temp_values
                            .insert(last_idx_str.clone(), last_idx.into());
                        return self.generate_array_get_internal(dest, object, &last_idx_str);
                    }
                }
                None
            }
            "reverse" => {
                // Implement array.reverse() - reverse elements in-place (like sort)
                if let Some(metadata) = self.array_metadata.get(object).cloned() {
                    let array_ptr = self.resolve_value(object).into_pointer_value();
                    let length = metadata.length;

                    if length <= 1 {
                        return None;
                    }

                    // Reverse elements in place using two-pointer technique
                    let half_length = length / 2;
                    for i in 0..half_length {
                        let left_idx = self.context.i32_type().const_int(i as u64, false);
                        let right_idx = self
                            .context
                            .i32_type()
                            .const_int((length - 1 - i) as u64, false);

                        let left_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    self.context.i32_type(),
                                    array_ptr,
                                    &[left_idx],
                                    &format!("rev_left_ptr_{}", i),
                                )
                                .unwrap()
                        };
                        let right_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    self.context.i32_type(),
                                    array_ptr,
                                    &[right_idx],
                                    &format!("rev_right_ptr_{}", i),
                                )
                                .unwrap()
                        };

                        let left_val = self
                            .builder
                            .build_load(
                                self.context.i32_type(),
                                left_ptr,
                                &format!("rev_left_val_{}", i),
                            )
                            .unwrap()
                            .into_int_value();
                        let right_val = self
                            .builder
                            .build_load(
                                self.context.i32_type(),
                                right_ptr,
                                &format!("rev_right_val_{}", i),
                            )
                            .unwrap()
                            .into_int_value();

                        // Swap: left = right_val, right = left_val
                        self.builder.build_store(right_ptr, left_val).unwrap();
                        self.builder.build_store(left_ptr, right_val).unwrap();
                    }
                    None
                } else {
                    None
                }
            }
            "isEmpty" => {
                if let Some(metadata) = self.array_metadata.get(object) {
                    let is_empty = if metadata.length == 0 { 1 } else { 0 };
                    let result = self.context.i32_type().const_int(is_empty, false);
                    self.temp_values.insert(dest.to_string(), result.into());
                    self.boolean_temps.insert(dest.to_string());
                    Some(result.into())
                } else {
                    None
                }
            }

            "clear" => {
                // Implement array.clear() - set length to 0
                if let Some(mut metadata) = self.array_metadata.get(object).cloned() {
                    let array_ptr = self.resolve_value(object).into_pointer_value();

                    // Update runtime length in heap header
                    // Get the heap pointer (8 bytes before data pointer)
                    let heap_ptr = unsafe {
                        self.builder.build_gep(
                            self.context.i8_type(),
                            array_ptr,
                            &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                            "heap_ptr_for_clear",
                        )
                    }
                    .unwrap();

                    // Read length field at offset 4 from heap start (same as generate_array_len)
                    let len_field_ptr = unsafe {
                        self.builder.build_gep(
                            self.context.i8_type(),
                            heap_ptr,
                            &[self.context.i32_type().const_int(4, false)],
                            "len_field_ptr_clear",
                        )
                    }
                    .unwrap();

                    let len_ptr_cast = self
                        .builder
                        .build_pointer_cast(
                            len_field_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "len_ptr_cast_clear",
                        )
                        .unwrap();

                    // Set runtime length to 0
                    self.builder
                        .build_store(len_ptr_cast, self.context.i32_type().const_int(0, false))
                        .unwrap();

                    // Update metadata
                    metadata.length = 0;
                    self.array_metadata.insert(object.to_string(), metadata);
                    None
                } else {
                    None
                }
            }
            "sort" => {
                // Implement array.sort() - simple bubble sort for now
                if let Some(metadata) = self.array_metadata.get(object).cloned() {
                    let array_ptr = self.resolve_value(object).into_pointer_value();
                    let length = metadata.length;

                    if length <= 1 {
                        return None;
                    }

                    // Simple bubble sort implementation
                    for i in 0..length {
                        for j in 0..(length - 1 - i) {
                            let idx_j = self.context.i32_type().const_int(j as u64, false);
                            let idx_j_plus_1 =
                                self.context.i32_type().const_int((j + 1) as u64, false);

                            let ptr_j = unsafe {
                                self.builder
                                    .build_in_bounds_gep(
                                        self.context.i32_type(),
                                        array_ptr,
                                        &[idx_j],
                                        &format!("sort_ptr_{}", j),
                                    )
                                    .unwrap()
                            };
                            let ptr_j_plus_1 = unsafe {
                                self.builder
                                    .build_in_bounds_gep(
                                        self.context.i32_type(),
                                        array_ptr,
                                        &[idx_j_plus_1],
                                        &format!("sort_ptr_{}", j + 1),
                                    )
                                    .unwrap()
                            };

                            let val_j = self
                                .builder
                                .build_load(self.context.i32_type(), ptr_j, "val_j")
                                .unwrap()
                                .into_int_value();
                            let val_j_plus_1 = self
                                .builder
                                .build_load(self.context.i32_type(), ptr_j_plus_1, "val_j_plus_1")
                                .unwrap()
                                .into_int_value();

                            let cmp = self
                                .builder
                                .build_int_compare(
                                    inkwell::IntPredicate::SGT,
                                    val_j,
                                    val_j_plus_1,
                                    "cmp",
                                )
                                .unwrap();

                            // Create conditional swap
                            let current_fn = self
                                .builder
                                .get_insert_block()
                                .unwrap()
                                .get_parent()
                                .unwrap();
                            let swap_block = self.context.append_basic_block(current_fn, "swap");
                            let continue_block =
                                self.context.append_basic_block(current_fn, "continue");

                            self.builder
                                .build_conditional_branch(cmp, swap_block, continue_block)
                                .unwrap();

                            self.builder.position_at_end(swap_block);
                            self.builder.build_store(ptr_j, val_j_plus_1).unwrap();
                            self.builder.build_store(ptr_j_plus_1, val_j).unwrap();
                            self.builder
                                .build_unconditional_branch(continue_block)
                                .unwrap();

                            self.builder.position_at_end(continue_block);
                        }
                    }
                    None
                } else {
                    None
                }
            }
            "slice" => {
                // Implement array.slice(start, end)
                if let Some(metadata) = self.array_metadata.get(object).cloned() {
                    let start_val = self.resolve_value(&args[0]).into_int_value();
                    let end_val = self.resolve_value(&args[1]).into_int_value();
                    let array_ptr = self.resolve_value(object).into_pointer_value();

                    let slice_len = self
                        .builder
                        .build_int_sub(end_val, start_val, "slice_len")
                        .unwrap();

                    // Allocate new array for slice - use small size, relies on heap length field
                    let malloc_fn = self.get_or_declare_malloc();
                    let array_type = self.context.i32_type().array_type(1);
                    let array_size = array_type.size_of().unwrap();
                    let header_size = self.context.i64_type().const_int(8, false);
                    let total_size = self
                        .builder
                        .build_int_add(header_size, array_size, "total_size")
                        .unwrap();

                    let heap_ptr = self
                        .builder
                        .build_call(malloc_fn, &[total_size.into()], "heap_array_slice")
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

                    // Store length (slice_len - actual runtime computed value)
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
                    self.builder.build_store(len_ptr_cast, slice_len).unwrap();

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

                    let new_array_ptr = self
                        .builder
                        .build_pointer_cast(
                            data_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "new_array_ptr",
                        )
                        .unwrap();

                    // Copy elements from start to end
                    let src_ptr = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.i32_type(),
                                array_ptr,
                                &[start_val],
                                "src_start",
                            )
                            .unwrap()
                    };

                    let memcpy_fn = self.module.get_function("memcpy").unwrap_or_else(|| {
                        let fn_type = self
                            .context
                            .ptr_type(inkwell::AddressSpace::default())
                            .fn_type(
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
                        self.module.add_function("memcpy", fn_type, None)
                    });

                    let byte_size = self
                        .builder
                        .build_int_mul(
                            slice_len,
                            self.context.i32_type().const_int(4, false),
                            "byte_size",
                        )
                        .unwrap();
                    let byte_size_64 = self
                        .builder
                        .build_int_z_extend(byte_size, self.context.i64_type(), "byte_size_64")
                        .unwrap();

                    self.builder
                        .build_call(
                            memcpy_fn,
                            &[new_array_ptr.into(), src_ptr.into(), byte_size_64.into()],
                            "",
                        )
                        .unwrap();

                    // Store result - use original metadata.length as safe upper bound
                    // Actual runtime length is stored in heap at offset 4
                    let mut new_metadata = metadata.clone();
                    new_metadata.length = metadata.length;
                    self.temp_values.insert(dest.to_string(), data_ptr.into());
                    self.heap_arrays.insert(dest.to_string());
                    self.array_metadata.insert(dest.to_string(), new_metadata);
                    Some(data_ptr.into())
                } else {
                    None
                }
            }
            "indexOf" => {
                // Implement array.indexOf(value) - returns index or -1
                if let Some(metadata) = self.array_metadata.get(object) {
                    let target_val = self.resolve_value(&args[0]);
                    let array_ptr = self.resolve_value(object).into_pointer_value();
                    let is_string_array = metadata.contains_strings;

                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let loop_block = self.context.append_basic_block(current_fn, "indexOf_loop");
                    let check_block = self.context.append_basic_block(current_fn, "indexOf_check");
                    let found_block = self.context.append_basic_block(current_fn, "indexOf_found");
                    let not_found_block = self
                        .context
                        .append_basic_block(current_fn, "indexOf_not_found");
                    let after_block = self.context.append_basic_block(current_fn, "indexOf_after");

                    let counter_ptr = self
                        .builder
                        .build_alloca(self.context.i32_type(), "indexOf_counter")
                        .unwrap();
                    self.builder
                        .build_store(counter_ptr, self.context.i32_type().const_int(0, false))
                        .unwrap();

                    // Read runtime length from heap structure
                    let heap_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                array_ptr,
                                &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                "heap_ptr_for_indexOf",
                            )
                            .unwrap()
                    };

                    let len_field_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                heap_ptr,
                                &[self.context.i32_type().const_int(4, false)],
                                "len_field_ptr",
                            )
                            .unwrap()
                    };

                    let len_ptr_cast = self
                        .builder
                        .build_pointer_cast(
                            len_field_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "len_ptr_cast",
                        )
                        .unwrap();

                    let length = self
                        .builder
                        .build_load(self.context.i32_type(), len_ptr_cast, "runtime_length")
                        .unwrap()
                        .into_int_value();

                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Loop: check if counter < length
                    self.builder.position_at_end(loop_block);
                    let counter = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "counter")
                        .unwrap()
                        .into_int_value();
                    let cmp = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::ULT, counter, length, "cmp")
                        .unwrap();
                    self.builder
                        .build_conditional_branch(cmp, check_block, not_found_block)
                        .unwrap();

                    // Check: compare element
                    self.builder.position_at_end(check_block);
                    let equals = if is_string_array {
                        let elem_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    self.context.ptr_type(inkwell::AddressSpace::default()),
                                    array_ptr,
                                    &[counter],
                                    "elem_ptr",
                                )
                                .unwrap()
                        };
                        let elem = self
                            .builder
                            .build_load(
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                elem_ptr,
                                "elem",
                            )
                            .unwrap()
                            .into_pointer_value();

                        let strcmp_fn = self.module.get_function("strcmp").unwrap_or_else(|| {
                            let fn_type = self.context.i32_type().fn_type(
                                &[
                                    self.context
                                        .ptr_type(inkwell::AddressSpace::default())
                                        .into(),
                                    self.context
                                        .ptr_type(inkwell::AddressSpace::default())
                                        .into(),
                                ],
                                false,
                            );
                            self.module.add_function("strcmp", fn_type, None)
                        });

                        let cmp_result = self
                            .builder
                            .build_call(
                                strcmp_fn,
                                &[elem.into(), target_val.into_pointer_value().into()],
                                "strcmp_result",
                            )
                            .unwrap()
                            .try_as_basic_value()
                            .left()
                            .unwrap()
                            .into_int_value();

                        self.builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                cmp_result,
                                self.context.i32_type().const_int(0, false),
                                "equals",
                            )
                            .unwrap()
                    } else {
                        let elem_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    self.context.i32_type(),
                                    array_ptr,
                                    &[counter],
                                    "elem_ptr",
                                )
                                .unwrap()
                        };
                        let elem = self
                            .builder
                            .build_load(self.context.i32_type(), elem_ptr, "elem")
                            .unwrap()
                            .into_int_value();

                        self.builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                elem,
                                target_val.into_int_value(),
                                "equals",
                            )
                            .unwrap()
                    };

                    let continue_block = self
                        .context
                        .append_basic_block(current_fn, "indexOf_continue");
                    self.builder
                        .build_conditional_branch(equals, found_block, continue_block)
                        .unwrap();

                    // Continue: increment counter
                    self.builder.position_at_end(continue_block);
                    let next_counter = self
                        .builder
                        .build_int_add(
                            counter,
                            self.context.i32_type().const_int(1, false),
                            "next_counter",
                        )
                        .unwrap();
                    self.builder.build_store(counter_ptr, next_counter).unwrap();
                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Found: return current index
                    self.builder.position_at_end(found_block);
                    let found_counter = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "found_idx")
                        .unwrap()
                        .into_int_value();
                    self.builder
                        .build_unconditional_branch(after_block)
                        .unwrap();

                    // Not found: return -1
                    self.builder.position_at_end(not_found_block);
                    let not_found_val = self.context.i32_type().const_int((-1_i32) as u64, true);
                    self.builder
                        .build_unconditional_branch(after_block)
                        .unwrap();

                    // After: phi node
                    self.builder.position_at_end(after_block);
                    let phi = self
                        .builder
                        .build_phi(self.context.i32_type(), "indexOf_result")
                        .unwrap();
                    phi.add_incoming(&[
                        (&found_counter, found_block),
                        (&not_found_val, not_found_block),
                    ]);
                    let result = phi.as_basic_value();

                    self.temp_values.insert(dest.to_string(), result);
                    Some(result)
                } else {
                    let result = self.context.i32_type().const_int((-1_i32) as u64, true);
                    self.temp_values.insert(dest.to_string(), result.into());
                    Some(result.into())
                }
            }
            "filter" => {
                // Implement array.filter(closure) with proper closure execution
                if let Some(metadata) = self.array_metadata.get(object).cloned() {
                    let array_ptr = self.resolve_value(object).into_pointer_value();

                    // Check if the argument is a closure
                    if !self.is_closure(&args[0]) {
                        return None;
                    }

                    // Read runtime length
                    let heap_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                array_ptr,
                                &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                "heap_ptr_for_filter",
                            )
                            .unwrap()
                    };

                    let len_field_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                heap_ptr,
                                &[self.context.i32_type().const_int(4, false)],
                                "len_field_ptr_filter",
                            )
                            .unwrap()
                    };

                    let len_ptr_cast = self
                        .builder
                        .build_pointer_cast(
                            len_field_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "len_ptr_cast_filter",
                        )
                        .unwrap();

                    let length = self
                        .builder
                        .build_load(
                            self.context.i32_type(),
                            len_ptr_cast,
                            "runtime_length_filter",
                        )
                        .unwrap()
                        .into_int_value();

                    // Allocate new result array
                    let malloc_fn = self.get_or_declare_malloc();
                    let array_type = self.context.i32_type().array_type(100);
                    let array_size = array_type.size_of().unwrap();
                    let header_size = self.context.i64_type().const_int(8, false);
                    let total_size = self
                        .builder
                        .build_int_add(header_size, array_size, "total_size_filter")
                        .unwrap();

                    let new_heap_ptr = self
                        .builder
                        .build_call(malloc_fn, &[total_size.into()], "heap_array_filter")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    // Store RC = 1
                    let rc_ptr = self
                        .builder
                        .build_pointer_cast(
                            new_heap_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "rc_ptr_filter",
                        )
                        .unwrap();
                    self.builder
                        .build_store(rc_ptr, self.context.i32_type().const_int(1, false))
                        .unwrap();

                    // Initialize result length to 0
                    let result_len_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                new_heap_ptr,
                                &[self.context.i32_type().const_int(4, false)],
                                "result_len_ptr",
                            )
                            .unwrap()
                    };
                    let result_len_ptr_cast = self
                        .builder
                        .build_pointer_cast(
                            result_len_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "result_len_ptr_cast",
                        )
                        .unwrap();
                    self.builder
                        .build_store(
                            result_len_ptr_cast,
                            self.context.i32_type().const_int(0, false),
                        )
                        .unwrap();

                    // Get result data pointer
                    let result_data_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                new_heap_ptr,
                                &[self.context.i32_type().const_int(8, false)],
                                "result_data_ptr",
                            )
                            .unwrap()
                    };

                    let new_array_ptr = self
                        .builder
                        .build_pointer_cast(
                            result_data_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "new_array_ptr_filter",
                        )
                        .unwrap();

                    // Loop through array and filter using closure
                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let loop_block = self.context.append_basic_block(current_fn, "filter_loop");
                    let check_block = self.context.append_basic_block(current_fn, "filter_check");
                    let include_block = self
                        .context
                        .append_basic_block(current_fn, "filter_include");
                    let after_block = self.context.append_basic_block(current_fn, "filter_after");

                    let counter_ptr = self
                        .builder
                        .build_alloca(self.context.i32_type(), "filter_counter")
                        .unwrap();
                    let result_idx_ptr = self
                        .builder
                        .build_alloca(self.context.i32_type(), "filter_result_idx")
                        .unwrap();
                    self.builder
                        .build_store(counter_ptr, self.context.i32_type().const_int(0, false))
                        .unwrap();
                    self.builder
                        .build_store(result_idx_ptr, self.context.i32_type().const_int(0, false))
                        .unwrap();

                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Loop: check if counter < length
                    self.builder.position_at_end(loop_block);
                    let counter = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "counter_filter")
                        .unwrap()
                        .into_int_value();
                    let cmp = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::ULT,
                            counter,
                            length,
                            "cmp_filter",
                        )
                        .unwrap();
                    self.builder
                        .build_conditional_branch(cmp, check_block, after_block)
                        .unwrap();

                    // Check element value by calling closure
                    self.builder.position_at_end(check_block);
                    let elem_ptr = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.i32_type(),
                                array_ptr,
                                &[counter],
                                "elem_ptr_filter",
                            )
                            .unwrap()
                    };
                    let elem = self
                        .builder
                        .build_load(self.context.i32_type(), elem_ptr, "elem_filter")
                        .unwrap()
                        .into_int_value();

                    // Call the closure with the element
                    let closure_result = self.call_closure_with_one_arg(&args[0], elem.into());

                    let should_include = if let Some(result) = closure_result {
                        if result.is_int_value() {
                            let result_int = result.into_int_value();
                            // Treat non-zero as true
                            self.builder
                                .build_int_compare(
                                    inkwell::IntPredicate::NE,
                                    result_int,
                                    self.context.i32_type().const_int(0, false),
                                    "should_include",
                                )
                                .unwrap()
                        } else {
                            self.context.bool_type().const_int(0, false)
                        }
                    } else {
                        self.context.bool_type().const_int(0, false)
                    };

                    let skip_block = self.context.append_basic_block(current_fn, "filter_skip");
                    self.builder
                        .build_conditional_branch(should_include, include_block, skip_block)
                        .unwrap();

                    // Include: add to result array
                    self.builder.position_at_end(include_block);
                    let result_idx = self
                        .builder
                        .build_load(self.context.i32_type(), result_idx_ptr, "result_idx")
                        .unwrap()
                        .into_int_value();

                    let result_elem_ptr = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.i32_type(),
                                new_array_ptr,
                                &[result_idx],
                                "result_elem_ptr",
                            )
                            .unwrap()
                    };
                    self.builder.build_store(result_elem_ptr, elem).unwrap();

                    let new_result_idx = self
                        .builder
                        .build_int_add(
                            result_idx,
                            self.context.i32_type().const_int(1, false),
                            "new_result_idx",
                        )
                        .unwrap();
                    self.builder
                        .build_store(result_idx_ptr, new_result_idx)
                        .unwrap();
                    self.builder.build_unconditional_branch(skip_block).unwrap();

                    // Skip block
                    self.builder.position_at_end(skip_block);
                    let next_counter = self
                        .builder
                        .build_int_add(
                            counter,
                            self.context.i32_type().const_int(1, false),
                            "next_counter_filter",
                        )
                        .unwrap();
                    self.builder.build_store(counter_ptr, next_counter).unwrap();
                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // After loop: update result length
                    self.builder.position_at_end(after_block);
                    let final_result_idx = self
                        .builder
                        .build_load(self.context.i32_type(), result_idx_ptr, "final_result_idx")
                        .unwrap()
                        .into_int_value();
                    self.builder
                        .build_store(result_len_ptr_cast, final_result_idx)
                        .unwrap();

                    let mut new_metadata = metadata.clone();
                    new_metadata.length = 100;
                    self.temp_values
                        .insert(dest.to_string(), result_data_ptr.into());
                    self.heap_arrays.insert(dest.to_string());
                    self.array_metadata.insert(dest.to_string(), new_metadata);
                    Some(result_data_ptr.into())
                } else {
                    None
                }
            }
            "map" => {
                // Implement array.map(closure) with proper closure execution
                if let Some(metadata) = self.array_metadata.get(object).cloned() {
                    let array_ptr = self.resolve_value(object).into_pointer_value();

                    // Check if the argument is a closure
                    if !self.is_closure(&args[0]) {
                        return None;
                    }

                    // Read runtime length
                    let heap_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                array_ptr,
                                &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                "heap_ptr_for_map",
                            )
                            .unwrap()
                    };

                    let len_field_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                heap_ptr,
                                &[self.context.i32_type().const_int(4, false)],
                                "len_field_ptr_map",
                            )
                            .unwrap()
                    };

                    let len_ptr_cast = self
                        .builder
                        .build_pointer_cast(
                            len_field_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "len_ptr_cast_map",
                        )
                        .unwrap();

                    let length = self
                        .builder
                        .build_load(self.context.i32_type(), len_ptr_cast, "runtime_length_map")
                        .unwrap()
                        .into_int_value();

                    // Allocate new array
                    let malloc_fn = self.get_or_declare_malloc();
                    let array_type = self.context.i32_type().array_type(100);
                    let array_size = array_type.size_of().unwrap();
                    let header_size = self.context.i64_type().const_int(8, false);
                    let total_size = self
                        .builder
                        .build_int_add(header_size, array_size, "total_size_map")
                        .unwrap();

                    let new_heap_ptr = self
                        .builder
                        .build_call(malloc_fn, &[total_size.into()], "heap_array_map")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    // Store RC = 1
                    let rc_ptr = self
                        .builder
                        .build_pointer_cast(
                            new_heap_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "rc_ptr_map",
                        )
                        .unwrap();
                    self.builder
                        .build_store(rc_ptr, self.context.i32_type().const_int(1, false))
                        .unwrap();

                    // Set result length = input length
                    let result_len_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                new_heap_ptr,
                                &[self.context.i32_type().const_int(4, false)],
                                "result_len_ptr_map",
                            )
                            .unwrap()
                    };
                    let result_len_ptr_cast = self
                        .builder
                        .build_pointer_cast(
                            result_len_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "result_len_ptr_cast_map",
                        )
                        .unwrap();
                    self.builder
                        .build_store(result_len_ptr_cast, length)
                        .unwrap();

                    // Get result data pointer
                    let result_data_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                new_heap_ptr,
                                &[self.context.i32_type().const_int(8, false)],
                                "result_data_ptr_map",
                            )
                            .unwrap()
                    };

                    let new_array_ptr = self
                        .builder
                        .build_pointer_cast(
                            result_data_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "new_array_ptr_map",
                        )
                        .unwrap();

                    // Create loop to iterate and map
                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let loop_block = self.context.append_basic_block(current_fn, "map_loop");
                    let check_block = self.context.append_basic_block(current_fn, "map_check");
                    let after_block = self.context.append_basic_block(current_fn, "map_after");

                    let counter_ptr = self
                        .builder
                        .build_alloca(self.context.i32_type(), "map_counter")
                        .unwrap();
                    self.builder
                        .build_store(counter_ptr, self.context.i32_type().const_int(0, false))
                        .unwrap();

                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Loop body
                    self.builder.position_at_end(loop_block);
                    let counter = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "counter_map")
                        .unwrap()
                        .into_int_value();
                    let cmp = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::ULT, counter, length, "cmp_map")
                        .unwrap();
                    self.builder
                        .build_conditional_branch(cmp, check_block, after_block)
                        .unwrap();

                    // Load and transform element using closure
                    self.builder.position_at_end(check_block);
                    let elem_ptr = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.i32_type(),
                                array_ptr,
                                &[counter],
                                "elem_ptr_map",
                            )
                            .unwrap()
                    };
                    let elem = self
                        .builder
                        .build_load(self.context.i32_type(), elem_ptr, "elem_map")
                        .unwrap()
                        .into_int_value();

                    // Call the closure with the element
                    let closure_result = self.call_closure_with_one_arg(&args[0], elem.into());

                    let transformed = if let Some(result) = closure_result {
                        if result.is_int_value() {
                            result.into_int_value()
                        } else {
                            self.context.i32_type().const_int(0, false)
                        }
                    } else {
                        self.context.i32_type().const_int(0, false)
                    };

                    // Store transformed element
                    let result_elem_ptr = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.i32_type(),
                                new_array_ptr,
                                &[counter],
                                "result_elem_ptr_map",
                            )
                            .unwrap()
                    };
                    self.builder
                        .build_store(result_elem_ptr, transformed)
                        .unwrap();

                    let next_counter = self
                        .builder
                        .build_int_add(
                            counter,
                            self.context.i32_type().const_int(1, false),
                            "next_counter_map",
                        )
                        .unwrap();
                    self.builder.build_store(counter_ptr, next_counter).unwrap();
                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    self.builder.position_at_end(after_block);

                    let mut new_metadata = metadata.clone();
                    new_metadata.length = 100;
                    self.temp_values
                        .insert(dest.to_string(), result_data_ptr.into());
                    self.heap_arrays.insert(dest.to_string());
                    self.array_metadata.insert(dest.to_string(), new_metadata);
                    Some(result_data_ptr.into())
                } else {
                    None
                }
            }
            "reduce" => {
                // Implement array.reduce(init, closure) with proper closure execution
                if let Some(metadata) = self.array_metadata.get(object).cloned() {
                    let array_ptr = self.resolve_value(object).into_pointer_value();
                    let initial_val = self.resolve_value(&args[0]).into_int_value();

                    // Check if the second argument is a closure
                    if args.len() < 2 || !self.is_closure(&args[1]) {
                        return None;
                    }

                    // Read runtime length
                    let heap_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                array_ptr,
                                &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                "heap_ptr_for_reduce",
                            )
                            .unwrap()
                    };

                    let len_field_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                heap_ptr,
                                &[self.context.i32_type().const_int(4, false)],
                                "len_field_ptr_reduce",
                            )
                            .unwrap()
                    };

                    let len_ptr_cast = self
                        .builder
                        .build_pointer_cast(
                            len_field_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "len_ptr_cast_reduce",
                        )
                        .unwrap();

                    let length = self
                        .builder
                        .build_load(
                            self.context.i32_type(),
                            len_ptr_cast,
                            "runtime_length_reduce",
                        )
                        .unwrap()
                        .into_int_value();

                    // Create loop to accumulate
                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let loop_block = self.context.append_basic_block(current_fn, "reduce_loop");
                    let check_block = self.context.append_basic_block(current_fn, "reduce_check");
                    let after_block = self.context.append_basic_block(current_fn, "reduce_after");

                    let counter_ptr = self
                        .builder
                        .build_alloca(self.context.i32_type(), "reduce_counter")
                        .unwrap();
                    let accumulator_ptr = self
                        .builder
                        .build_alloca(self.context.i32_type(), "reduce_accumulator")
                        .unwrap();

                    self.builder
                        .build_store(counter_ptr, self.context.i32_type().const_int(0, false))
                        .unwrap();
                    self.builder
                        .build_store(accumulator_ptr, initial_val)
                        .unwrap();

                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Loop body
                    self.builder.position_at_end(loop_block);
                    let counter = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "counter_reduce")
                        .unwrap()
                        .into_int_value();
                    let cmp = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::ULT,
                            counter,
                            length,
                            "cmp_reduce",
                        )
                        .unwrap();
                    self.builder
                        .build_conditional_branch(cmp, check_block, after_block)
                        .unwrap();

                    // Accumulate using closure
                    self.builder.position_at_end(check_block);
                    let accumulator = self
                        .builder
                        .build_load(self.context.i32_type(), accumulator_ptr, "accumulator")
                        .unwrap()
                        .into_int_value();

                    let elem_ptr = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.i32_type(),
                                array_ptr,
                                &[counter],
                                "elem_ptr_reduce",
                            )
                            .unwrap()
                    };
                    let elem = self
                        .builder
                        .build_load(self.context.i32_type(), elem_ptr, "elem_reduce")
                        .unwrap()
                        .into_int_value();

                    // Call the closure with accumulator and element
                    let closure_result =
                        self.call_closure_with_two_args(&args[1], accumulator.into(), elem.into());

                    let new_accumulator = if let Some(result) = closure_result {
                        if result.is_int_value() {
                            result.into_int_value()
                        } else {
                            accumulator
                        }
                    } else {
                        accumulator
                    };

                    self.builder
                        .build_store(accumulator_ptr, new_accumulator)
                        .unwrap();

                    let next_counter = self
                        .builder
                        .build_int_add(
                            counter,
                            self.context.i32_type().const_int(1, false),
                            "next_counter_reduce",
                        )
                        .unwrap();
                    self.builder.build_store(counter_ptr, next_counter).unwrap();
                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    self.builder.position_at_end(after_block);
                    let result = self
                        .builder
                        .build_load(self.context.i32_type(), accumulator_ptr, "reduce_result")
                        .unwrap();

                    self.temp_values.insert(dest.to_string(), result);
                    Some(result)
                } else {
                    None
                }
            }
            "join" => {
                // Implement array.join(separator) - concatenates string array with separator
                if let Some(metadata) = self.array_metadata.get(object).cloned() {
                    let array_ptr = self.resolve_value(object).into_pointer_value();
                    let separator = self.resolve_value(&args[0]).into_pointer_value();

                    // Read runtime length
                    let heap_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                array_ptr,
                                &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                "heap_ptr_for_join",
                            )
                            .unwrap()
                    };

                    let len_field_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                heap_ptr,
                                &[self.context.i32_type().const_int(4, false)],
                                "len_field_ptr_join",
                            )
                            .unwrap()
                    };

                    let len_ptr_cast = self
                        .builder
                        .build_pointer_cast(
                            len_field_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "len_ptr_cast_join",
                        )
                        .unwrap();

                    let length = self
                        .builder
                        .build_load(self.context.i32_type(), len_ptr_cast, "runtime_length_join")
                        .unwrap()
                        .into_int_value();

                    // Allocate result string buffer (2048 bytes)
                    let malloc_fn = self.get_or_declare_malloc();
                    let buffer_size = self.context.i64_type().const_int(2048, false);
                    let result_heap = self
                        .builder
                        .build_call(malloc_fn, &[buffer_size.into()], "join_buffer")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    // Set RC to 1
                    let rc_ptr = self
                        .builder
                        .build_pointer_cast(
                            result_heap,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "rc_ptr_join",
                        )
                        .unwrap();
                    self.builder
                        .build_store(rc_ptr, self.context.i32_type().const_int(1, false))
                        .unwrap();

                    // Get data pointer
                    let result_data_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                result_heap,
                                &[self.context.i32_type().const_int(8, false)],
                                "result_data_ptr",
                            )
                            .unwrap()
                    };

                    let result_ptr_typed = self
                        .builder
                        .build_pointer_cast(
                            result_data_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "result_ptr_typed",
                        )
                        .unwrap();

                    // Get strcpy function
                    let strcpy_fn = self.module.get_function("strcpy").unwrap_or_else(|| {
                        let fn_type = self
                            .context
                            .ptr_type(inkwell::AddressSpace::default())
                            .fn_type(
                                &[
                                    self.context
                                        .ptr_type(inkwell::AddressSpace::default())
                                        .into(),
                                    self.context
                                        .ptr_type(inkwell::AddressSpace::default())
                                        .into(),
                                ],
                                false,
                            );
                        self.module.add_function("strcpy", fn_type, None)
                    });

                    // Get strcat function
                    let strcat_fn = self.module.get_function("strcat").unwrap_or_else(|| {
                        let fn_type = self
                            .context
                            .ptr_type(inkwell::AddressSpace::default())
                            .fn_type(
                                &[
                                    self.context
                                        .ptr_type(inkwell::AddressSpace::default())
                                        .into(),
                                    self.context
                                        .ptr_type(inkwell::AddressSpace::default())
                                        .into(),
                                ],
                                false,
                            );
                        self.module.add_function("strcat", fn_type, None)
                    });

                    // Loop: copy first element, then append separator + element
                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let loop_block = self.context.append_basic_block(current_fn, "join_loop");
                    let check_block = self.context.append_basic_block(current_fn, "join_check");
                    let first_elem_block =
                        self.context.append_basic_block(current_fn, "join_first");
                    let other_elem_block =
                        self.context.append_basic_block(current_fn, "join_other");
                    let after_block = self.context.append_basic_block(current_fn, "join_after");

                    let counter_ptr = self
                        .builder
                        .build_alloca(self.context.i32_type(), "join_counter")
                        .unwrap();
                    self.builder
                        .build_store(counter_ptr, self.context.i32_type().const_int(0, false))
                        .unwrap();

                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Loop body
                    self.builder.position_at_end(loop_block);
                    let counter = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "counter_join")
                        .unwrap()
                        .into_int_value();
                    let cmp = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::ULT, counter, length, "cmp_join")
                        .unwrap();
                    self.builder
                        .build_conditional_branch(cmp, check_block, after_block)
                        .unwrap();

                    // Check if first element
                    self.builder.position_at_end(check_block);
                    let is_first = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            counter,
                            self.context.i32_type().const_int(0, false),
                            "is_first",
                        )
                        .unwrap();
                    self.builder
                        .build_conditional_branch(is_first, first_elem_block, other_elem_block)
                        .unwrap();

                    // First element: just copy it
                    self.builder.position_at_end(first_elem_block);
                    let elem_ptr = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                array_ptr,
                                &[counter],
                                "elem_ptr_first",
                            )
                            .unwrap()
                    };
                    let elem = self
                        .builder
                        .build_load(
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            elem_ptr,
                            "elem_first",
                        )
                        .unwrap()
                        .into_pointer_value();

                    self.builder
                        .build_call(strcpy_fn, &[result_ptr_typed.into(), elem.into()], "")
                        .unwrap();

                    let inc_block = self.context.append_basic_block(current_fn, "join_inc");
                    self.builder.build_unconditional_branch(inc_block).unwrap();

                    // Other elements: append separator + element
                    self.builder.position_at_end(other_elem_block);
                    let elem_ptr2 = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                array_ptr,
                                &[counter],
                                "elem_ptr_other",
                            )
                            .unwrap()
                    };
                    let elem2 = self
                        .builder
                        .build_load(
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            elem_ptr2,
                            "elem_other",
                        )
                        .unwrap()
                        .into_pointer_value();

                    // Append separator
                    self.builder
                        .build_call(strcat_fn, &[result_ptr_typed.into(), separator.into()], "")
                        .unwrap();

                    // Append element
                    self.builder
                        .build_call(strcat_fn, &[result_ptr_typed.into(), elem2.into()], "")
                        .unwrap();

                    self.builder.build_unconditional_branch(inc_block).unwrap();

                    // Increment counter
                    self.builder.position_at_end(inc_block);
                    let next_counter = self
                        .builder
                        .build_int_add(
                            counter,
                            self.context.i32_type().const_int(1, false),
                            "next_counter_join",
                        )
                        .unwrap();
                    self.builder.build_store(counter_ptr, next_counter).unwrap();
                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    self.builder.position_at_end(after_block);

                    self.temp_values
                        .insert(dest.to_string(), result_ptr_typed.into());
                    self.heap_strings.insert(dest.to_string());
                    Some(result_ptr_typed.into())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn generate_array_get_internal(
        &mut self,
        dest: &str,
        array: &str,
        index: &str,
    ) -> Option<BasicValueEnum<'ctx>> {
        let array_val = self.resolve_value(array);
        let index_val = self.resolve_value(index);

        let array_ptr = array_val.into_pointer_value();

        // Check if array contains strings
        let metadata_opt = self.array_metadata.get(array);
        let is_string_array = metadata_opt.map(|m| m.contains_strings).unwrap_or(false);

        if is_string_array {
            // For string arrays, elements are pointers (i8*)
            // We need to use GEP with the pointee type, which is i8* for strings
            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());

            // Cast array_ptr to i8** (pointer to pointer)
            let array_ptr_typed = self
                .builder
                .build_pointer_cast(array_ptr, ptr_type, "array_ptr_typed")
                .unwrap();

            let element_ptr = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        ptr_type,
                        array_ptr_typed,
                        &[index_val.into_int_value()],
                        "element_ptr",
                    )
                    .unwrap()
            };

            let element = self
                .builder
                .build_load(ptr_type, element_ptr, "element")
                .unwrap();
            self.temp_values.insert(dest.to_string(), element);
            // Mark as heap string so it gets properly printed
            self.heap_strings.insert(dest.to_string());
            Some(element)
        } else {
            // For integer arrays
            let element_ptr = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        self.context.i32_type(),
                        array_ptr,
                        &[index_val.into_int_value()],
                        "element_ptr",
                    )
                    .unwrap()
            };

            let element = self
                .builder
                .build_load(self.context.i32_type(), element_ptr, "element")
                .unwrap();
            self.temp_values.insert(dest.to_string(), element);
            Some(element)
        }
    }
}
