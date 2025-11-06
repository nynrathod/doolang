use crate::codegen::{core::CodeGen, ArrayMetadata};
use inkwell::types::BasicType;
use inkwell::values::BasicValueEnum;

impl<'ctx> CodeGen<'ctx> {
    pub fn generate_map_method(
        &mut self,
        dest: &str,
        object: &str,
        _object_val: BasicValueEnum<'ctx>,
        method: &str,
        args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        match method {
            "get" => {
                // Implement map.get(key) with linear search through key-value pairs
                if let Some(metadata) = self.map_metadata.get(object) {
                    if metadata.length == 0 {
                        // Empty map, return default
                        let default_val = self.context.i32_type().const_int(0, false);
                        self.temp_values
                            .insert(dest.to_string(), default_val.into());
                        return Some(default_val.into());
                    }

                    let key_val = self.resolve_value(&args[0]);
                    let map_ptr = self.resolve_value(object).into_pointer_value();

                    // Determine types
                    let key_type: inkwell::types::BasicTypeEnum = if metadata.key_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        self.context.i32_type().into()
                    };

                    let val_type: inkwell::types::BasicTypeEnum = if metadata.value_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        self.context.i32_type().into()
                    };

                    // Create struct type for key-value pair
                    let pair_type = self.context.struct_type(&[key_type, val_type], false);
                    let map_type = pair_type.array_type(metadata.length as u32);

                    // Search through pairs
                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let loop_block = self.context.append_basic_block(current_fn, "map_get_loop");
                    let check_block = self.context.append_basic_block(current_fn, "map_get_check");
                    let found_block = self.context.append_basic_block(current_fn, "map_get_found");
                    let not_found_block = self
                        .context
                        .append_basic_block(current_fn, "map_get_not_found");
                    let after_block = self.context.append_basic_block(current_fn, "map_get_after");

                    // Counter for loop
                    let counter_ptr = self
                        .builder
                        .build_alloca(self.context.i32_type(), "map_counter")
                        .unwrap();
                    self.builder
                        .build_store(counter_ptr, self.context.i32_type().const_int(0, false))
                        .unwrap();

                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Loop: check if counter < length
                    self.builder.position_at_end(loop_block);
                    let counter = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "counter")
                        .unwrap()
                        .into_int_value();
                    let length = self
                        .context
                        .i32_type()
                        .const_int(metadata.length as u64, false);
                    let cmp = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::ULT, counter, length, "cmp")
                        .unwrap();
                    self.builder
                        .build_conditional_branch(cmp, check_block, not_found_block)
                        .unwrap();

                    // Check block: load key and compare
                    self.builder.position_at_end(check_block);
                    let pair_ptr = unsafe {
                        self.builder
                            .build_gep(
                                map_type,
                                map_ptr,
                                &[self.context.i32_type().const_zero(), counter],
                                "pair_ptr",
                            )
                            .unwrap()
                    };

                    let key_ptr = self
                        .builder
                        .build_struct_gep(pair_type, pair_ptr, 0, "key_ptr")
                        .unwrap();
                    let stored_key = self
                        .builder
                        .build_load(key_type, key_ptr, "stored_key")
                        .unwrap();

                    let keys_equal = if stored_key.is_pointer_value() && key_val.is_pointer_value()
                    {
                        // For pointer keys (strings), use strcmp
                        let stored_ptr = stored_key.into_pointer_value();
                        let key_ptr_val = key_val.into_pointer_value();

                        // Get or declare strcmp
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
                                &[stored_ptr.into(), key_ptr_val.into()],
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
                                "keys_equal",
                            )
                            .unwrap()
                    } else if stored_key.is_int_value() && key_val.is_int_value() {
                        // For int keys
                        self.builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                stored_key.into_int_value(),
                                key_val.into_int_value(),
                                "keys_equal",
                            )
                            .unwrap()
                    } else {
                        // Type mismatch, always false
                        self.context.bool_type().const_zero()
                    };

                    let continue_block = self
                        .context
                        .append_basic_block(current_fn, "map_get_continue");
                    self.builder
                        .build_conditional_branch(keys_equal, found_block, continue_block)
                        .unwrap();

                    // Continue: increment counter
                    self.builder.position_at_end(continue_block);
                    let next_counter = self
                        .builder
                        .build_int_add(counter, self.context.i32_type().const_int(1, false), "next")
                        .unwrap();
                    self.builder.build_store(counter_ptr, next_counter).unwrap();
                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Found: extract value
                    self.builder.position_at_end(found_block);
                    let found_pair_ptr = unsafe {
                        self.builder
                            .build_gep(
                                map_type,
                                map_ptr,
                                &[self.context.i32_type().const_zero(), counter],
                                "found_pair_ptr",
                            )
                            .unwrap()
                    };
                    let val_ptr = self
                        .builder
                        .build_struct_gep(pair_type, found_pair_ptr, 1, "val_ptr")
                        .unwrap();
                    let found_val = self
                        .builder
                        .build_load(val_type, val_ptr, "found_val")
                        .unwrap();
                    self.builder
                        .build_unconditional_branch(after_block)
                        .unwrap();

                    // Not found: return default
                    self.builder.position_at_end(not_found_block);
                    let default_val: BasicValueEnum = if metadata.value_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .const_null()
                            .into()
                    } else {
                        self.context.i32_type().const_int(0, false).into()
                    };
                    self.builder
                        .build_unconditional_branch(after_block)
                        .unwrap();

                    // After: phi node
                    self.builder.position_at_end(after_block);
                    let phi = self.builder.build_phi(val_type, "map_get_result").unwrap();
                    phi.add_incoming(&[(&found_val, found_block), (&default_val, not_found_block)]);
                    let result = phi.as_basic_value();

                    self.temp_values.insert(dest.to_string(), result);
                    if metadata.value_is_string {
                        self.heap_strings.insert(dest.to_string());
                    }
                    Some(result)
                } else {
                    None
                }
            }
            "set" => {
                // Implement map.set(key, value) with runtime length support
                if let Some(metadata) = self.map_metadata.get(object).cloned() {
                    let key_val = self.resolve_value(&args[0]);
                    let value_val = self.resolve_value(&args[1]);

                    let key_type: inkwell::types::BasicTypeEnum = if metadata.key_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        self.context.i32_type().into()
                    };

                    let val_type: inkwell::types::BasicTypeEnum = if metadata.value_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        self.context.i32_type().into()
                    };

                    let pair_type = self.context.struct_type(&[key_type, val_type], false);
                    let old_data_ptr = self.resolve_value(object).into_pointer_value();

                    // Check if map is empty (null pointer)
                    let is_null = self.builder.build_is_null(old_data_ptr, "is_null").unwrap();

                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();

                    let empty_map_block =
                        self.context.append_basic_block(current_fn, "map_set_empty");
                    let non_empty_block = self
                        .context
                        .append_basic_block(current_fn, "map_set_non_empty");

                    self.builder
                        .build_conditional_branch(is_null, empty_map_block, non_empty_block)
                        .unwrap();

                    // Handle empty map: create new map with one entry
                    self.builder.position_at_end(empty_map_block);
                    let map_type = pair_type.array_type(1);
                    let malloc_fn = self.get_or_declare_malloc();
                    let map_size = map_type.size_of().unwrap();
                    let header_size = self.context.i64_type().const_int(8, false);
                    let total_size = self
                        .builder
                        .build_int_add(header_size, map_size, "total_size")
                        .unwrap();

                    let heap_ptr = self
                        .builder
                        .build_call(malloc_fn, &[total_size.into()], "heap_ptr")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

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

                    let len_field_ptr_empty = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                heap_ptr,
                                &[self.context.i32_type().const_int(4, false)],
                                "len_field_ptr_empty",
                            )
                            .unwrap()
                    };
                    let len_ptr_empty = self
                        .builder
                        .build_pointer_cast(
                            len_field_ptr_empty,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "len_ptr_empty",
                        )
                        .unwrap();
                    self.builder
                        .build_store(len_ptr_empty, self.context.i32_type().const_int(1, false))
                        .unwrap();

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

                    let pair_ptr = unsafe {
                        self.builder
                            .build_gep(
                                map_type,
                                data_ptr,
                                &[
                                    self.context.i32_type().const_zero(),
                                    self.context.i32_type().const_zero(),
                                ],
                                "pair_ptr",
                            )
                            .unwrap()
                    };
                    let key_ptr = self
                        .builder
                        .build_struct_gep(pair_type, pair_ptr, 0, "key_ptr")
                        .unwrap();
                    let val_ptr = self
                        .builder
                        .build_struct_gep(pair_type, pair_ptr, 1, "val_ptr")
                        .unwrap();
                    self.builder.build_store(key_ptr, key_val).unwrap();
                    self.builder.build_store(val_ptr, value_val).unwrap();

                    self.temp_values.insert(object.to_string(), data_ptr.into());
                    if let Some(sym) = self.symbols.get(object) {
                        self.builder.build_store(sym.ptr, data_ptr).unwrap();
                    }
                    return None;

                    // Handle non-empty map
                    self.builder.position_at_end(non_empty_block);

                    // Read current runtime length from map header
                    let heap_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                old_data_ptr,
                                &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                "heap_ptr_set",
                            )
                            .unwrap()
                    };

                    let len_field_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                heap_ptr,
                                &[self.context.i32_type().const_int(4, false)],
                                "len_field_ptr_set",
                            )
                            .unwrap()
                    };

                    let len_ptr_cast = self
                        .builder
                        .build_pointer_cast(
                            len_field_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "len_ptr_cast_set",
                        )
                        .unwrap();

                    let old_runtime_len = self
                        .builder
                        .build_load(self.context.i32_type(), len_ptr_cast, "old_runtime_len")
                        .unwrap()
                        .into_int_value();

                    // First search for the key in existing entries
                    let search_loop = self
                        .context
                        .append_basic_block(current_fn, "map_set_search");
                    let search_check = self.context.append_basic_block(current_fn, "map_set_check");
                    let search_next = self.context.append_basic_block(current_fn, "map_set_next");
                    let key_found_block =
                        self.context.append_basic_block(current_fn, "map_set_found");
                    let key_not_found = self
                        .context
                        .append_basic_block(current_fn, "map_set_not_found");

                    let search_idx_ptr = self
                        .builder
                        .build_alloca(self.context.i32_type(), "search_idx")
                        .unwrap();
                    self.builder
                        .build_store(search_idx_ptr, self.context.i32_type().const_zero())
                        .unwrap();

                    self.builder
                        .build_unconditional_branch(search_loop)
                        .unwrap();

                    // Search loop: while idx < old_runtime_len
                    self.builder.position_at_end(search_loop);
                    let search_idx = self
                        .builder
                        .build_load(self.context.i32_type(), search_idx_ptr, "search_idx")
                        .unwrap()
                        .into_int_value();
                    let in_range = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::ULT,
                            search_idx,
                            old_runtime_len,
                            "in_range",
                        )
                        .unwrap();
                    self.builder
                        .build_conditional_branch(in_range, search_check, key_not_found)
                        .unwrap();

                    // Check if key at current index matches
                    self.builder.position_at_end(search_check);

                    // We need to allocate enough space for max possible size
                    // Use metadata.length as a safe upper bound for array type
                    let max_len = if metadata.length > 0 {
                        metadata.length
                    } else {
                        100
                    };
                    let search_map_type = pair_type.array_type(max_len as u32);

                    let check_pair = unsafe {
                        self.builder
                            .build_gep(
                                search_map_type,
                                old_data_ptr,
                                &[self.context.i32_type().const_zero(), search_idx],
                                "check_pair",
                            )
                            .unwrap()
                    };
                    let check_k_ptr = self
                        .builder
                        .build_struct_gep(pair_type, check_pair, 0, "check_k")
                        .unwrap();
                    let check_k = self
                        .builder
                        .build_load(key_type, check_k_ptr, "check_k")
                        .unwrap();

                    let match_result = if check_k.is_pointer_value() && key_val.is_pointer_value() {
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
                        let cmp_res = self
                            .builder
                            .build_call(strcmp_fn, &[check_k.into(), key_val.into()], "strcmp")
                            .unwrap()
                            .try_as_basic_value()
                            .left()
                            .unwrap()
                            .into_int_value();
                        self.builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                cmp_res,
                                self.context.i32_type().const_zero(),
                                "match",
                            )
                            .unwrap()
                    } else {
                        self.builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                check_k.into_int_value(),
                                key_val.into_int_value(),
                                "match",
                            )
                            .unwrap()
                    };

                    self.builder
                        .build_conditional_branch(match_result, key_found_block, search_next)
                        .unwrap();

                    // Key found: update value in place
                    self.builder.position_at_end(key_found_block);
                    let update_v_ptr = self
                        .builder
                        .build_struct_gep(pair_type, check_pair, 1, "update_v")
                        .unwrap();
                    self.builder.build_store(update_v_ptr, value_val).unwrap();
                    // Length stays the same, so we're done
                    return None;

                    // Search next
                    self.builder.position_at_end(search_next);
                    let next_idx = self
                        .builder
                        .build_int_add(
                            search_idx,
                            self.context.i32_type().const_int(1, false),
                            "next",
                        )
                        .unwrap();
                    self.builder.build_store(search_idx_ptr, next_idx).unwrap();
                    self.builder
                        .build_unconditional_branch(search_loop)
                        .unwrap();

                    // Key not found: need to append
                    self.builder.position_at_end(key_not_found);
                    let new_runtime_len = self
                        .builder
                        .build_int_add(
                            old_runtime_len,
                            self.context.i32_type().const_int(1, false),
                            "new_len",
                        )
                        .unwrap();

                    // Reallocate with space for one more entry
                    let new_runtime_len_64 = self
                        .builder
                        .build_int_z_extend(new_runtime_len, self.context.i64_type(), "new_len_64")
                        .unwrap();
                    let pair_size = pair_type.size_of().unwrap();
                    let new_data_size = self
                        .builder
                        .build_int_mul(new_runtime_len_64, pair_size, "new_data_size")
                        .unwrap();
                    let header_size = self.context.i64_type().const_int(8, false);
                    let new_total_size = self
                        .builder
                        .build_int_add(header_size, new_data_size, "new_total")
                        .unwrap();

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

                    let new_heap = self
                        .builder
                        .build_call(
                            realloc_fn,
                            &[heap_ptr.into(), new_total_size.into()],
                            "new_heap",
                        )
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    // Update RC to 1
                    let new_rc_ptr = self
                        .builder
                        .build_pointer_cast(
                            new_heap,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "new_rc",
                        )
                        .unwrap();
                    self.builder
                        .build_store(new_rc_ptr, self.context.i32_type().const_int(1, false))
                        .unwrap();

                    // Update length field at offset 4
                    let new_len_field = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                new_heap,
                                &[self.context.i32_type().const_int(4, false)],
                                "new_len_field",
                            )
                            .unwrap()
                    };
                    let new_len_ptr = self
                        .builder
                        .build_pointer_cast(
                            new_len_field,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "new_len_ptr",
                        )
                        .unwrap();
                    self.builder
                        .build_store(new_len_ptr, new_runtime_len)
                        .unwrap();

                    // Get new data pointer
                    let new_data = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                new_heap,
                                &[self.context.i32_type().const_int(8, false)],
                                "new_data",
                            )
                            .unwrap()
                    };

                    // Append new key-value pair at index old_runtime_len
                    // Use byte-level GEP to avoid array type size issues
                    let old_runtime_len_64 = self
                        .builder
                        .build_int_z_extend(old_runtime_len, self.context.i64_type(), "old_len_64")
                        .unwrap();
                    let pair_size_for_append = pair_type.size_of().unwrap();
                    let byte_offset = self
                        .builder
                        .build_int_mul(old_runtime_len_64, pair_size_for_append, "byte_offset")
                        .unwrap();

                    let append_pair = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                new_data,
                                &[byte_offset],
                                "append_pair_bytes",
                            )
                            .unwrap()
                    };

                    let append_pair = self
                        .builder
                        .build_pointer_cast(
                            append_pair,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "append_pair",
                        )
                        .unwrap();
                    let append_k = self
                        .builder
                        .build_struct_gep(pair_type, append_pair, 0, "append_k")
                        .unwrap();
                    self.builder.build_store(append_k, key_val).unwrap();
                    let append_v = self
                        .builder
                        .build_struct_gep(pair_type, append_pair, 1, "append_v")
                        .unwrap();
                    self.builder.build_store(append_v, value_val).unwrap();

                    // Store new data pointer back to variable
                    self.temp_values.insert(object.to_string(), new_data.into());
                    if let Some(sym) = self.symbols.get(object) {
                        self.builder.build_store(sym.ptr, new_data).unwrap();
                    }

                    None
                } else {
                    None
                }
            }
            "has" => {
                // Implement map.has(key) with linear search through key-value pairs
                if let Some(metadata) = self.map_metadata.get(object) {
                    if metadata.length == 0 {
                        // Empty map, return false
                        let false_val = self.context.i32_type().const_int(0, false);
                        self.temp_values.insert(dest.to_string(), false_val.into());
                        return Some(false_val.into());
                    }

                    let key_val = self.resolve_value(&args[0]);
                    let map_ptr = self.resolve_value(object).into_pointer_value();

                    // Determine types
                    let key_type: inkwell::types::BasicTypeEnum = if metadata.key_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        self.context.i32_type().into()
                    };

                    let val_type: inkwell::types::BasicTypeEnum = if metadata.value_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        self.context.i32_type().into()
                    };

                    // Create struct type for key-value pair
                    let pair_type = self.context.struct_type(&[key_type, val_type], false);
                    let map_type = pair_type.array_type(metadata.length as u32);

                    // Search through pairs
                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let loop_block = self.context.append_basic_block(current_fn, "map_has_loop");
                    let check_block = self.context.append_basic_block(current_fn, "map_has_check");
                    let found_block = self.context.append_basic_block(current_fn, "map_has_found");
                    let not_found_block = self
                        .context
                        .append_basic_block(current_fn, "map_has_not_found");
                    let after_block = self.context.append_basic_block(current_fn, "map_has_after");

                    // Counter for loop
                    let counter_ptr = self
                        .builder
                        .build_alloca(self.context.i32_type(), "map_counter")
                        .unwrap();
                    self.builder
                        .build_store(counter_ptr, self.context.i32_type().const_int(0, false))
                        .unwrap();

                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Loop: check if counter < length
                    self.builder.position_at_end(loop_block);
                    let counter = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "counter")
                        .unwrap()
                        .into_int_value();
                    let length = self
                        .context
                        .i32_type()
                        .const_int(metadata.length as u64, false);
                    let cmp = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::ULT, counter, length, "cmp")
                        .unwrap();
                    self.builder
                        .build_conditional_branch(cmp, check_block, not_found_block)
                        .unwrap();

                    // Check block: load key and compare
                    self.builder.position_at_end(check_block);
                    let pair_ptr = unsafe {
                        self.builder
                            .build_gep(
                                map_type,
                                map_ptr,
                                &[self.context.i32_type().const_zero(), counter],
                                "pair_ptr",
                            )
                            .unwrap()
                    };

                    let key_ptr = self
                        .builder
                        .build_struct_gep(pair_type, pair_ptr, 0, "key_ptr")
                        .unwrap();
                    let stored_key = self
                        .builder
                        .build_load(key_type, key_ptr, "stored_key")
                        .unwrap();

                    let keys_equal = if stored_key.is_pointer_value() && key_val.is_pointer_value()
                    {
                        // For pointer keys (strings), use strcmp
                        let stored_ptr = stored_key.into_pointer_value();
                        let key_ptr_val = key_val.into_pointer_value();

                        // Get or declare strcmp
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
                                &[stored_ptr.into(), key_ptr_val.into()],
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
                                "keys_equal",
                            )
                            .unwrap()
                    } else if stored_key.is_int_value() && key_val.is_int_value() {
                        // For int keys
                        self.builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                stored_key.into_int_value(),
                                key_val.into_int_value(),
                                "keys_equal",
                            )
                            .unwrap()
                    } else {
                        // Type mismatch, always false
                        self.context.bool_type().const_zero()
                    };

                    let continue_block = self
                        .context
                        .append_basic_block(current_fn, "map_has_continue");
                    self.builder
                        .build_conditional_branch(keys_equal, found_block, continue_block)
                        .unwrap();

                    // Continue: increment counter
                    self.builder.position_at_end(continue_block);
                    let next_counter = self
                        .builder
                        .build_int_add(counter, self.context.i32_type().const_int(1, false), "next")
                        .unwrap();
                    self.builder.build_store(counter_ptr, next_counter).unwrap();
                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Found: return true
                    self.builder.position_at_end(found_block);
                    let true_val = self.context.i32_type().const_int(1, false);
                    self.builder
                        .build_unconditional_branch(after_block)
                        .unwrap();

                    // Not found: return false
                    self.builder.position_at_end(not_found_block);
                    let false_val = self.context.i32_type().const_int(0, false);
                    self.builder
                        .build_unconditional_branch(after_block)
                        .unwrap();

                    // After: phi node
                    self.builder.position_at_end(after_block);
                    let phi = self
                        .builder
                        .build_phi(self.context.i32_type(), "map_has_result")
                        .unwrap();
                    phi.add_incoming(&[(&true_val, found_block), (&false_val, not_found_block)]);
                    let result = phi.as_basic_value();

                    self.temp_values.insert(dest.to_string(), result);
                    self.boolean_temps.insert(dest.to_string());
                    Some(result)
                } else {
                    let false_val = self.context.i32_type().const_int(0, false);
                    self.temp_values.insert(dest.to_string(), false_val.into());
                    self.boolean_temps.insert(dest.to_string());
                    Some(false_val.into())
                }
            }
            "remove" => {
                // Implement map.remove(key) - find key and shift remaining elements
                if let Some(mut metadata) = self.map_metadata.get(object).cloned() {
                    if metadata.length == 0 {
                        // Empty map, nothing to remove
                        let false_val = self.context.i32_type().const_int(0, false);
                        self.temp_values.insert(dest.to_string(), false_val.into());
                        return Some(false_val.into());
                    }

                    let key_val = self.resolve_value(&args[0]);
                    let map_ptr = self.resolve_value(object).into_pointer_value();

                    // Determine types
                    let key_type: inkwell::types::BasicTypeEnum = if metadata.key_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        self.context.i32_type().into()
                    };

                    let val_type: inkwell::types::BasicTypeEnum = if metadata.value_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        self.context.i32_type().into()
                    };

                    let pair_type = self.context.struct_type(&[key_type, val_type], false);
                    let map_type = pair_type.array_type(metadata.length as u32);

                    // Search for key to remove
                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let loop_block = self
                        .context
                        .append_basic_block(current_fn, "map_remove_loop");
                    let check_block = self
                        .context
                        .append_basic_block(current_fn, "map_remove_check");
                    let found_block = self
                        .context
                        .append_basic_block(current_fn, "map_remove_found");
                    let not_found_block = self
                        .context
                        .append_basic_block(current_fn, "map_remove_not_found");
                    let after_block = self
                        .context
                        .append_basic_block(current_fn, "map_remove_after");

                    // Counter for loop
                    let counter_ptr = self
                        .builder
                        .build_alloca(self.context.i32_type(), "map_counter")
                        .unwrap();
                    self.builder
                        .build_store(counter_ptr, self.context.i32_type().const_int(0, false))
                        .unwrap();

                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Loop: check if counter < length
                    self.builder.position_at_end(loop_block);
                    let counter = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "counter")
                        .unwrap()
                        .into_int_value();
                    let length = self
                        .context
                        .i32_type()
                        .const_int(metadata.length as u64, false);
                    let cmp = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::ULT, counter, length, "cmp")
                        .unwrap();
                    self.builder
                        .build_conditional_branch(cmp, check_block, not_found_block)
                        .unwrap();

                    // Check block: load key and compare
                    self.builder.position_at_end(check_block);
                    let pair_ptr = unsafe {
                        self.builder
                            .build_gep(
                                map_type,
                                map_ptr,
                                &[self.context.i32_type().const_zero(), counter],
                                "pair_ptr",
                            )
                            .unwrap()
                    };

                    let key_ptr = self
                        .builder
                        .build_struct_gep(pair_type, pair_ptr, 0, "key_ptr")
                        .unwrap();
                    let stored_key = self
                        .builder
                        .build_load(key_type, key_ptr, "stored_key")
                        .unwrap();

                    let keys_equal = if stored_key.is_pointer_value() && key_val.is_pointer_value()
                    {
                        // For pointer keys (strings), use strcmp
                        let stored_ptr = stored_key.into_pointer_value();
                        let key_ptr_val = key_val.into_pointer_value();

                        // Get or declare strcmp
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
                                &[stored_ptr.into(), key_ptr_val.into()],
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
                                "keys_equal",
                            )
                            .unwrap()
                    } else if stored_key.is_int_value() && key_val.is_int_value() {
                        // For int keys
                        self.builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                stored_key.into_int_value(),
                                key_val.into_int_value(),
                                "keys_equal",
                            )
                            .unwrap()
                    } else {
                        // Type mismatch, always false
                        self.context.bool_type().const_zero()
                    };

                    let continue_block = self
                        .context
                        .append_basic_block(current_fn, "map_remove_continue");
                    self.builder
                        .build_conditional_branch(keys_equal, found_block, continue_block)
                        .unwrap();

                    // Continue: increment counter
                    self.builder.position_at_end(continue_block);
                    let next_counter = self
                        .builder
                        .build_int_add(counter, self.context.i32_type().const_int(1, false), "next")
                        .unwrap();
                    self.builder.build_store(counter_ptr, next_counter).unwrap();
                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Found: shift remaining elements to fill the gap
                    self.builder.position_at_end(found_block);

                    // Get the index where the key was found
                    let remove_idx = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "remove_idx")
                        .unwrap()
                        .into_int_value();

                    // If this isn't the last element, shift remaining elements down
                    let is_last = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            remove_idx,
                            self.context
                                .i32_type()
                                .const_int((metadata.length - 1) as u64, false),
                            "is_last",
                        )
                        .unwrap();

                    let shift_block = self
                        .context
                        .append_basic_block(current_fn, "shift_elements");
                    let shift_loop = self.context.append_basic_block(current_fn, "shift_loop");
                    let shift_check = self.context.append_basic_block(current_fn, "shift_check");
                    let shift_done = self.context.append_basic_block(current_fn, "shift_done");

                    self.builder
                        .build_conditional_branch(is_last, shift_done, shift_block)
                        .unwrap();

                    // Shift block: prepare to shift elements
                    self.builder.position_at_end(shift_block);
                    let shift_counter_ptr = self
                        .builder
                        .build_alloca(self.context.i32_type(), "shift_counter")
                        .unwrap();
                    // Start from the element after the removed one
                    let start_shift = self
                        .builder
                        .build_int_add(
                            remove_idx,
                            self.context.i32_type().const_int(1, false),
                            "start_shift",
                        )
                        .unwrap();
                    self.builder
                        .build_store(shift_counter_ptr, start_shift)
                        .unwrap();
                    self.builder.build_unconditional_branch(shift_loop).unwrap();

                    // Shift loop: copy element at position i to position i-1
                    self.builder.position_at_end(shift_loop);
                    let shift_i = self
                        .builder
                        .build_load(self.context.i32_type(), shift_counter_ptr, "shift_i")
                        .unwrap()
                        .into_int_value();
                    let shift_cmp = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::ULT,
                            shift_i,
                            self.context
                                .i32_type()
                                .const_int(metadata.length as u64, false),
                            "shift_cmp",
                        )
                        .unwrap();
                    self.builder
                        .build_conditional_branch(shift_cmp, shift_check, shift_done)
                        .unwrap();

                    // Shift check: do the actual copy
                    self.builder.position_at_end(shift_check);

                    // Load from current position
                    let src_pair_ptr = unsafe {
                        self.builder
                            .build_gep(
                                map_type,
                                map_ptr,
                                &[self.context.i32_type().const_zero(), shift_i],
                                "src_pair_ptr",
                            )
                            .unwrap()
                    };
                    let src_key_ptr = self
                        .builder
                        .build_struct_gep(pair_type, src_pair_ptr, 0, "src_key_ptr")
                        .unwrap();
                    let src_val_ptr = self
                        .builder
                        .build_struct_gep(pair_type, src_pair_ptr, 1, "src_val_ptr")
                        .unwrap();
                    let src_key = self
                        .builder
                        .build_load(key_type, src_key_ptr, "src_key")
                        .unwrap();
                    let src_val = self
                        .builder
                        .build_load(val_type, src_val_ptr, "src_val")
                        .unwrap();

                    // Store to previous position
                    let prev_i = self
                        .builder
                        .build_int_sub(
                            shift_i,
                            self.context.i32_type().const_int(1, false),
                            "prev_i",
                        )
                        .unwrap();
                    let dst_pair_ptr = unsafe {
                        self.builder
                            .build_gep(
                                map_type,
                                map_ptr,
                                &[self.context.i32_type().const_zero(), prev_i],
                                "dst_pair_ptr",
                            )
                            .unwrap()
                    };
                    let dst_key_ptr = self
                        .builder
                        .build_struct_gep(pair_type, dst_pair_ptr, 0, "dst_key_ptr")
                        .unwrap();
                    let dst_val_ptr = self
                        .builder
                        .build_struct_gep(pair_type, dst_pair_ptr, 1, "dst_val_ptr")
                        .unwrap();
                    self.builder.build_store(dst_key_ptr, src_key).unwrap();
                    self.builder.build_store(dst_val_ptr, src_val).unwrap();

                    // Increment shift counter
                    let next_shift_i = self
                        .builder
                        .build_int_add(
                            shift_i,
                            self.context.i32_type().const_int(1, false),
                            "next_shift_i",
                        )
                        .unwrap();
                    self.builder
                        .build_store(shift_counter_ptr, next_shift_i)
                        .unwrap();
                    self.builder.build_unconditional_branch(shift_loop).unwrap();

                    // Shift done: update metadata
                    self.builder.position_at_end(shift_done);
                    metadata.length -= 1;
                    self.map_metadata.insert(object.to_string(), metadata);
                    let true_val = self.context.i32_type().const_int(1, false);
                    self.builder
                        .build_unconditional_branch(after_block)
                        .unwrap();

                    // Not found: return false
                    self.builder.position_at_end(not_found_block);
                    let false_val = self.context.i32_type().const_int(0, false);
                    self.builder
                        .build_unconditional_branch(after_block)
                        .unwrap();

                    // After: phi node
                    self.builder.position_at_end(after_block);
                    let phi = self
                        .builder
                        .build_phi(self.context.i32_type(), "map_remove_result")
                        .unwrap();
                    phi.add_incoming(&[(&true_val, found_block), (&false_val, not_found_block)]);
                    let result = phi.as_basic_value();

                    self.temp_values.insert(dest.to_string(), result);
                    Some(result)
                } else {
                    None
                }
            }
            "isEmpty" => {
                if let Some(metadata) = self.map_metadata.get(object) {
                    let is_empty = if metadata.length == 0 { 1 } else { 0 };
                    let result = self.context.i32_type().const_int(is_empty, false);
                    self.temp_values.insert(dest.to_string(), result.into());
                    Some(result.into())
                } else {
                    let result = self.context.i32_type().const_int(1, false);
                    self.temp_values.insert(dest.to_string(), result.into());
                    Some(result.into())
                }
            }
            "size" => {
                // Read runtime length from map header (like arrays do)
                let map_ptr = self.resolve_value(object).into_pointer_value();

                // Get heap pointer (8 bytes before data pointer)
                let heap_ptr = unsafe {
                    self.builder
                        .build_gep(
                            self.context.i8_type(),
                            map_ptr,
                            &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                            "heap_ptr_for_size",
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
                            "len_field_ptr_size",
                        )
                        .unwrap()
                };

                let len_ptr_cast = self
                    .builder
                    .build_pointer_cast(
                        len_field_ptr,
                        self.context.ptr_type(inkwell::AddressSpace::default()),
                        "len_ptr_cast_size",
                    )
                    .unwrap();

                let runtime_len = self
                    .builder
                    .build_load(self.context.i32_type(), len_ptr_cast, "runtime_len_size")
                    .unwrap()
                    .into_int_value();

                self.temp_values
                    .insert(dest.to_string(), runtime_len.into());
                Some(runtime_len.into())
            }
            "clear" => {
                if let Some(mut metadata) = self.map_metadata.get(object).cloned() {
                    metadata.length = 0;
                    self.map_metadata.insert(object.to_string(), metadata);
                    let true_val = self.context.i32_type().const_int(1, false);
                    self.temp_values.insert(dest.to_string(), true_val.into());
                    Some(true_val.into())
                } else {
                    None
                }
            }
            "keys" => {
                // Extract all keys from the map into a new array
                if let Some(metadata) = self.map_metadata.get(object) {
                    if metadata.length == 0 {
                        // Empty map, return empty array
                        let empty_ptr = self
                            .context
                            .ptr_type(inkwell::AddressSpace::default())
                            .const_null();
                        self.temp_values.insert(dest.to_string(), empty_ptr.into());
                        let empty_meta = ArrayMetadata {
                            length: 0,
                            element_type: if metadata.key_is_string {
                                "Str".to_string()
                            } else {
                                "Int".to_string()
                            },
                            contains_strings: metadata.key_is_string,
                        };
                        self.array_metadata.insert(dest.to_string(), empty_meta);
                        return Some(empty_ptr.into());
                    }

                    let map_ptr = self.resolve_value(object).into_pointer_value();

                    // Determine key type
                    let key_type: inkwell::types::BasicTypeEnum = if metadata.key_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        self.context.i32_type().into()
                    };

                    let val_type: inkwell::types::BasicTypeEnum = if metadata.value_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        self.context.i32_type().into()
                    };

                    let pair_type = self.context.struct_type(&[key_type, val_type], false);
                    let map_type = pair_type.array_type(metadata.length as u32);

                    // Allocate array for keys
                    let malloc_fn = self.get_or_declare_malloc();
                    let key_array_type = key_type.array_type(metadata.length as u32);
                    let key_array_size = key_array_type.size_of().unwrap();
                    let header_size = self.context.i64_type().const_int(8, false);
                    let total_size = self
                        .builder
                        .build_int_add(header_size, key_array_size, "total_size")
                        .unwrap();

                    let heap_ptr = self
                        .builder
                        .build_call(malloc_fn, &[total_size.into()], "heap_ptr")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    // Initialize RC
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

                    // Loop through map and copy keys
                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let loop_block = self.context.append_basic_block(current_fn, "keys_loop");
                    let loop_body = self.context.append_basic_block(current_fn, "keys_body");
                    let loop_done = self.context.append_basic_block(current_fn, "keys_done");

                    let counter_ptr = self
                        .builder
                        .build_alloca(self.context.i32_type(), "keys_counter")
                        .unwrap();
                    self.builder
                        .build_store(counter_ptr, self.context.i32_type().const_zero())
                        .unwrap();
                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Loop check
                    self.builder.position_at_end(loop_block);
                    let counter = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "counter")
                        .unwrap()
                        .into_int_value();
                    let length = self
                        .context
                        .i32_type()
                        .const_int(metadata.length as u64, false);
                    let cmp = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::ULT, counter, length, "cmp")
                        .unwrap();
                    self.builder
                        .build_conditional_branch(cmp, loop_body, loop_done)
                        .unwrap();

                    // Loop body: copy key
                    self.builder.position_at_end(loop_body);
                    let pair_ptr = unsafe {
                        self.builder
                            .build_gep(
                                map_type,
                                map_ptr,
                                &[self.context.i32_type().const_zero(), counter],
                                "pair_ptr",
                            )
                            .unwrap()
                    };
                    let key_ptr = self
                        .builder
                        .build_struct_gep(pair_type, pair_ptr, 0, "key_ptr")
                        .unwrap();
                    let key_val = self
                        .builder
                        .build_load(key_type, key_ptr, "key_val")
                        .unwrap();

                    // Store in output array
                    let out_ptr = self
                        .builder
                        .build_pointer_cast(
                            data_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "out_ptr",
                        )
                        .unwrap();
                    let elem_ptr = unsafe {
                        self.builder
                            .build_gep(key_type, out_ptr, &[counter], "elem_ptr")
                            .unwrap()
                    };
                    self.builder.build_store(elem_ptr, key_val).unwrap();

                    // Increment counter
                    let next = self
                        .builder
                        .build_int_add(counter, self.context.i32_type().const_int(1, false), "next")
                        .unwrap();
                    self.builder.build_store(counter_ptr, next).unwrap();
                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Done
                    self.builder.position_at_end(loop_done);
                    self.temp_values.insert(dest.to_string(), data_ptr.into());
                    self.heap_arrays.insert(dest.to_string());
                    let arr_meta = ArrayMetadata {
                        length: metadata.length,
                        element_type: if metadata.key_is_string {
                            "Str".to_string()
                        } else {
                            "Int".to_string()
                        },
                        contains_strings: metadata.key_is_string,
                    };
                    self.array_metadata.insert(dest.to_string(), arr_meta);
                    Some(data_ptr.into())
                } else {
                    None
                }
            }
            "values" => {
                // Extract all values from the map into a new array
                if let Some(metadata) = self.map_metadata.get(object) {
                    if metadata.length == 0 {
                        // Empty map, return empty array
                        let empty_ptr = self
                            .context
                            .ptr_type(inkwell::AddressSpace::default())
                            .const_null();
                        self.temp_values.insert(dest.to_string(), empty_ptr.into());
                        let empty_meta = ArrayMetadata {
                            length: 0,
                            element_type: if metadata.value_is_string {
                                "Str".to_string()
                            } else {
                                "Int".to_string()
                            },
                            contains_strings: metadata.value_is_string,
                        };
                        self.array_metadata.insert(dest.to_string(), empty_meta);
                        return Some(empty_ptr.into());
                    }

                    let map_ptr = self.resolve_value(object).into_pointer_value();

                    // Determine types
                    let key_type: inkwell::types::BasicTypeEnum = if metadata.key_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        self.context.i32_type().into()
                    };

                    let val_type: inkwell::types::BasicTypeEnum = if metadata.value_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        self.context.i32_type().into()
                    };

                    let pair_type = self.context.struct_type(&[key_type, val_type], false);
                    let map_type = pair_type.array_type(metadata.length as u32);

                    // Allocate array for values
                    let malloc_fn = self.get_or_declare_malloc();
                    let val_array_type = val_type.array_type(metadata.length as u32);
                    let val_array_size = val_array_type.size_of().unwrap();
                    let header_size = self.context.i64_type().const_int(8, false);
                    let total_size = self
                        .builder
                        .build_int_add(header_size, val_array_size, "total_size")
                        .unwrap();

                    let heap_ptr = self
                        .builder
                        .build_call(malloc_fn, &[total_size.into()], "heap_ptr")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    // Initialize RC
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

                    // Loop through map and copy values
                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let loop_block = self.context.append_basic_block(current_fn, "values_loop");
                    let loop_body = self.context.append_basic_block(current_fn, "values_body");
                    let loop_done = self.context.append_basic_block(current_fn, "values_done");

                    let counter_ptr = self
                        .builder
                        .build_alloca(self.context.i32_type(), "values_counter")
                        .unwrap();
                    self.builder
                        .build_store(counter_ptr, self.context.i32_type().const_zero())
                        .unwrap();
                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Loop check
                    self.builder.position_at_end(loop_block);
                    let counter = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "counter")
                        .unwrap()
                        .into_int_value();
                    let length = self
                        .context
                        .i32_type()
                        .const_int(metadata.length as u64, false);
                    let cmp = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::ULT, counter, length, "cmp")
                        .unwrap();
                    self.builder
                        .build_conditional_branch(cmp, loop_body, loop_done)
                        .unwrap();

                    // Loop body: copy value
                    self.builder.position_at_end(loop_body);
                    let pair_ptr = unsafe {
                        self.builder
                            .build_gep(
                                map_type,
                                map_ptr,
                                &[self.context.i32_type().const_zero(), counter],
                                "pair_ptr",
                            )
                            .unwrap()
                    };
                    let val_ptr = self
                        .builder
                        .build_struct_gep(pair_type, pair_ptr, 1, "val_ptr")
                        .unwrap();
                    let val_val = self
                        .builder
                        .build_load(val_type, val_ptr, "val_val")
                        .unwrap();

                    // Store in output array
                    let out_ptr = self
                        .builder
                        .build_pointer_cast(
                            data_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "out_ptr",
                        )
                        .unwrap();
                    let elem_ptr = unsafe {
                        self.builder
                            .build_gep(val_type, out_ptr, &[counter], "elem_ptr")
                            .unwrap()
                    };
                    self.builder.build_store(elem_ptr, val_val).unwrap();

                    // Increment counter
                    let next = self
                        .builder
                        .build_int_add(counter, self.context.i32_type().const_int(1, false), "next")
                        .unwrap();
                    self.builder.build_store(counter_ptr, next).unwrap();
                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Done
                    self.builder.position_at_end(loop_done);
                    self.temp_values.insert(dest.to_string(), data_ptr.into());
                    self.heap_arrays.insert(dest.to_string());
                    let arr_meta = ArrayMetadata {
                        length: metadata.length,
                        element_type: if metadata.value_is_string {
                            "Str".to_string()
                        } else {
                            "Int".to_string()
                        },
                        contains_strings: metadata.value_is_string,
                    };
                    self.array_metadata.insert(dest.to_string(), arr_meta);
                    Some(data_ptr.into())
                } else {
                    None
                }
            }
            "containsKey" => {
                // Same as has()
                if let Some(metadata) = self.map_metadata.get(object) {
                    if metadata.length == 0 {
                        let result = self.context.i32_type().const_int(0, false);
                        self.temp_values.insert(dest.to_string(), result.into());
                        return Some(result.into());
                    }

                    let map_ptr = self.resolve_value(object).into_pointer_value();
                    let key_val = self.resolve_value(&args[0]);

                    let key_type: inkwell::types::BasicTypeEnum = if metadata.key_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        self.context.i32_type().into()
                    };

                    let val_type: inkwell::types::BasicTypeEnum = if metadata.value_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        self.context.i32_type().into()
                    };

                    let map_type = self.context.struct_type(&[key_type, val_type], false);

                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let loop_block = self
                        .context
                        .append_basic_block(current_fn, "containsKey_loop");
                    let body_block = self
                        .context
                        .append_basic_block(current_fn, "containsKey_body");
                    let found_block = self
                        .context
                        .append_basic_block(current_fn, "containsKey_found");
                    let not_found_block = self
                        .context
                        .append_basic_block(current_fn, "containsKey_not_found");
                    let after_block = self
                        .context
                        .append_basic_block(current_fn, "containsKey_after");

                    let counter_ptr = self
                        .builder
                        .build_alloca(self.context.i32_type(), "counter")
                        .unwrap();
                    self.builder
                        .build_store(counter_ptr, self.context.i32_type().const_int(0, false))
                        .unwrap();

                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    self.builder.position_at_end(loop_block);
                    let counter = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "counter")
                        .unwrap()
                        .into_int_value();
                    let length = self
                        .context
                        .i32_type()
                        .const_int(metadata.length as u64, false);
                    let cmp = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::ULT, counter, length, "cmp")
                        .unwrap();
                    self.builder
                        .build_conditional_branch(cmp, body_block, not_found_block)
                        .unwrap();

                    self.builder.position_at_end(body_block);
                    let pair_ptr = unsafe {
                        self.builder
                            .build_gep(
                                map_type,
                                map_ptr,
                                &[self.context.i32_type().const_zero(), counter],
                                "pair_ptr",
                            )
                            .unwrap()
                    };
                    let key_ptr = self
                        .builder
                        .build_struct_gep(map_type, pair_ptr, 0, "key_ptr")
                        .unwrap();
                    let stored_key = self
                        .builder
                        .build_load(key_type, key_ptr, "stored_key")
                        .unwrap();

                    let keys_equal = if stored_key.is_pointer_value() && key_val.is_pointer_value()
                    {
                        // For pointer keys (strings), use strcmp
                        let stored_ptr = stored_key.into_pointer_value();
                        let key_ptr_val = key_val.into_pointer_value();

                        // Get or declare strcmp
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
                                &[stored_ptr.into(), key_ptr_val.into()],
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
                                "keys_equal",
                            )
                            .unwrap()
                    } else {
                        // For integer keys, direct comparison
                        let stored_int = stored_key.into_int_value();
                        let key_int = key_val.into_int_value();
                        self.builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                stored_int,
                                key_int,
                                "keys_equal",
                            )
                            .unwrap()
                    };

                    let continue_block = self
                        .context
                        .append_basic_block(current_fn, "containsKey_continue");
                    self.builder
                        .build_conditional_branch(keys_equal, found_block, continue_block)
                        .unwrap();

                    self.builder.position_at_end(continue_block);
                    let next_counter = self
                        .builder
                        .build_int_add(counter, self.context.i32_type().const_int(1, false), "next")
                        .unwrap();
                    self.builder.build_store(counter_ptr, next_counter).unwrap();
                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    self.builder.position_at_end(found_block);
                    let true_val = self.context.i32_type().const_int(1, false);
                    self.builder
                        .build_unconditional_branch(after_block)
                        .unwrap();

                    self.builder.position_at_end(not_found_block);
                    let false_val = self.context.i32_type().const_int(0, false);
                    self.builder
                        .build_unconditional_branch(after_block)
                        .unwrap();

                    self.builder.position_at_end(after_block);
                    let phi = self
                        .builder
                        .build_phi(self.context.i32_type(), "containsKey_result")
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
            "containsValue" => {
                // For now, return false - placeholder
                let result = self.context.i32_type().const_int(0, false);
                self.temp_values.insert(dest.to_string(), result.into());
                Some(result.into())
            }
            _ => None,
        }
    }
}
