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
                // map.get() is removed - use map[key] syntax instead
                panic!("map.get() is removed. Use map[key] syntax instead for map access.");
            }
            "_get_internal" => {
                // Internal implementation kept for [] syntax
                if let Some(metadata) = self.map_metadata.get(object) {
                    let key_val = self.resolve_value(&args[0]);
                    let map_ptr = self.resolve_value(object).into_pointer_value();

                    // Determine types
                    let key_type: inkwell::types::BasicTypeEnum = if metadata.key_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else if metadata.key_type == "Float" {
                        self.context.f64_type().into()
                    } else {
                        self.context.i32_type().into()
                    };

                    let val_type: inkwell::types::BasicTypeEnum = if metadata.value_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else if metadata.value_type.contains("Float") {
                        self.context.f64_type().into()
                    } else {
                        self.context.i32_type().into()
                    };

                    // Create struct type for key-value pair
                    let pair_type = self.context.struct_type(&[key_type, val_type], false);

                    // Get the runtime length from map header if metadata.length is 0 (parameter case)
                    let length_val = if metadata.length == 0 {
                        // Read length from map header at offset 4 bytes from RC header
                        // RC header is 8 bytes before the data pointer
                        let rc_header_ptr = unsafe {
                            self.builder
                                .build_gep(
                                    self.context.i8_type(),
                                    map_ptr,
                                    &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                    "rc_header_ptr_get",
                                )
                                .unwrap()
                        };

                        let len_ptr = unsafe {
                            self.builder
                                .build_gep(
                                    self.context.i8_type(),
                                    rc_header_ptr,
                                    &[self.context.i32_type().const_int(4, false)],
                                    "len_ptr_get",
                                )
                                .unwrap()
                        };

                        let len_ptr_cast = self
                            .builder
                            .build_pointer_cast(
                                len_ptr,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                "len_ptr_cast_get",
                            )
                            .unwrap();

                        self.builder
                            .build_load(self.context.i32_type(), len_ptr_cast, "runtime_len")
                            .unwrap()
                            .into_int_value()
                    } else {
                        // Use static metadata length
                        self.context
                            .i32_type()
                            .const_int(metadata.length as u64, false)
                    };

                    // Check if map is empty
                    let zero = self.context.i32_type().const_int(0, false);
                    let is_empty = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::EQ, length_val, zero, "is_empty")
                        .unwrap();

                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();

                    // Declare all blocks upfront
                    let empty_block = self.context.append_basic_block(current_fn, "map_get_empty");
                    let search_block = self
                        .context
                        .append_basic_block(current_fn, "map_get_search");
                    let loop_block = self.context.append_basic_block(current_fn, "map_get_loop");
                    let check_block = self.context.append_basic_block(current_fn, "map_get_check");
                    let found_block = self.context.append_basic_block(current_fn, "map_get_found");
                    let not_found_block = self
                        .context
                        .append_basic_block(current_fn, "map_get_not_found");
                    let after_block = self.context.append_basic_block(current_fn, "map_get_after");

                    self.builder
                        .build_conditional_branch(is_empty, empty_block, search_block)
                        .unwrap();

                    // Empty map case
                    self.builder.position_at_end(empty_block);
                    let empty_default_val: BasicValueEnum = if metadata.value_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .const_null()
                            .into()
                    } else if metadata.value_type.contains("Float") {
                        self.context.f64_type().const_float(0.0).into()
                    } else {
                        self.context.i32_type().const_int(0, false).into()
                    };
                    self.builder
                        .build_unconditional_branch(after_block)
                        .unwrap();
                    let empty_bb = self.builder.get_insert_block().unwrap();

                    // Search case
                    self.builder.position_at_end(search_block);

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
                    let cmp = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::ULT, counter, length_val, "cmp")
                        .unwrap();
                    self.builder
                        .build_conditional_branch(cmp, check_block, not_found_block)
                        .unwrap();

                    // Make a copy of map_type for use in search block
                    let map_type_search = if metadata.length > 0 {
                        pair_type.array_type(metadata.length as u32)
                    } else {
                        // For parameters, use a very large array type
                        pair_type.array_type(1000)
                    };

                    // Check block: load key and compare
                    self.builder.position_at_end(check_block);
                    let pair_ptr = unsafe {
                        self.builder
                            .build_gep(
                                map_type_search,
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
                    } else if stored_key.is_float_value() && key_val.is_float_value() {
                        // For float keys, use floating point comparison
                        self.builder
                            .build_float_compare(
                                inkwell::FloatPredicate::OEQ,
                                stored_key.into_float_value(),
                                key_val.into_float_value(),
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
                                map_type_search,
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
                    } else if metadata.value_type.contains("Float") {
                        self.context.f64_type().const_float(0.0).into()
                    } else {
                        self.context.i32_type().const_int(0, false).into()
                    };
                    self.builder
                        .build_unconditional_branch(after_block)
                        .unwrap();

                    // After: phi node
                    self.builder.position_at_end(after_block);
                    let phi = self.builder.build_phi(val_type, "map_get_result").unwrap();
                    phi.add_incoming(&[
                        (&found_val, found_block),
                        (&default_val, not_found_block),
                        (&empty_default_val, empty_bb),
                    ]);
                    let result = phi.as_basic_value();

                    self.temp_values.insert(dest.to_string(), result);
                    // NOTE: Don't mark map value results as heap strings!
                    // String values from maps are embedded in the map structure,
                    // not owned by this function. They should NOT be reference counted or freed.
                    Some(result)
                } else {
                    None
                }
            }
            "set" => {
                // map.set() is removed - use map[key] = value syntax instead
                panic!(
                    "map.set() is removed. Use map[key] = value syntax instead for map assignment."
                );
            }
            "_set_internal" => {
                // Internal implementation kept for [] syntax
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

                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();

                    // Always use runtime null check to determine if we need initial allocation
                    // This handles both global constants and runtime-created maps correctly
                    let needs_initial_allocation =
                        self.builder.build_is_null(old_data_ptr, "is_null").unwrap();

                    let initial_alloc_block = self
                        .context
                        .append_basic_block(current_fn, "map_set_initial_alloc");
                    let modify_existing_block = self
                        .context
                        .append_basic_block(current_fn, "map_set_modify_existing");
                    let after_all_block = self
                        .context
                        .append_basic_block(current_fn, "map_set_after_all");

                    self.builder
                        .build_conditional_branch(
                            needs_initial_allocation,
                            initial_alloc_block,
                            modify_existing_block,
                        )
                        .unwrap();

                    // Handle initial allocation (empty map or global constant on first modification)
                    self.builder.position_at_end(initial_alloc_block);

                    // Determine the initial length from metadata
                    // If the map is null, initial_len will be 0 (no data to copy)
                    // Otherwise, use the metadata length
                    let initial_len = metadata.length;

                    // Allocate space for existing data + 1 new entry
                    let new_len = initial_len + 1;
                    let map_type = pair_type.array_type(new_len as u32);
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

                    // Write initial length (will be overwritten with correct value after search)
                    let initial_len_field = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                heap_ptr,
                                &[self.context.i32_type().const_int(4, false)],
                                "initial_len_field",
                            )
                            .unwrap()
                    };
                    let initial_len_ptr_i32 = self
                        .builder
                        .build_pointer_cast(
                            initial_len_field,
                            self.context
                                .i32_type()
                                .ptr_type(inkwell::AddressSpace::default()),
                            "initial_len_ptr",
                        )
                        .unwrap();
                    self.builder
                        .build_store(
                            initial_len_ptr_i32,
                            self.context.i32_type().const_int(new_len as u64, false),
                        )
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

                    // Copy existing data from global constant if any
                    if initial_len > 0 {
                        let memcpy_fn = self.module.get_function("memcpy").unwrap_or_else(|| {
                            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                            let fn_type = ptr_type.fn_type(
                                &[
                                    ptr_type.into(),
                                    ptr_type.into(),
                                    self.context.i64_type().into(),
                                ],
                                false,
                            );
                            self.module.add_function("memcpy", fn_type, None)
                        });

                        let old_size = pair_type.size_of().unwrap();
                        let old_total = self
                            .builder
                            .build_int_mul(
                                self.context.i64_type().const_int(initial_len as u64, false),
                                old_size,
                                "old_total",
                            )
                            .unwrap();

                        self.builder
                            .build_call(
                                memcpy_fn,
                                &[data_ptr.into(), old_data_ptr.into(), old_total.into()],
                                "",
                            )
                            .unwrap();
                    }

                    // Now search through the copied data to see if key exists
                    // If it exists, update in place. If not, append at the end.
                    // TEMPORARY FIX: Skip update-in-place optimization - always append
                    // This avoids the heap header corruption issue
                    let append_new_block = self
                        .context
                        .append_basic_block(current_fn, "init_append_new");
                    let after_init_search = self
                        .context
                        .append_basic_block(current_fn, "after_init_search");

                    self.builder
                        .build_unconditional_branch(append_new_block)
                        .unwrap();

                    // Append new key at the end
                    self.builder.position_at_end(append_new_block);
                    let new_pair_offset =
                        self.context.i64_type().const_int(initial_len as u64, false);
                    let pair_size_64 = pair_type.size_of().unwrap();
                    let byte_offset = self
                        .builder
                        .build_int_mul(new_pair_offset, pair_size_64, "byte_offset")
                        .unwrap();
                    let new_pair_ptr_i8 = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                data_ptr,
                                &[byte_offset],
                                "new_pair_ptr_i8",
                            )
                            .unwrap()
                    };
                    let new_pair_ptr = self
                        .builder
                        .build_pointer_cast(
                            new_pair_ptr_i8,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "new_pair_ptr",
                        )
                        .unwrap();
                    let key_ptr = self
                        .builder
                        .build_struct_gep(pair_type, new_pair_ptr, 0, "key_ptr")
                        .unwrap();
                    let val_ptr = self
                        .builder
                        .build_struct_gep(pair_type, new_pair_ptr, 1, "val_ptr")
                        .unwrap();
                    self.builder.build_store(key_ptr, key_val).unwrap();
                    self.builder.build_store(val_ptr, value_val).unwrap();

                    // When appending, length is new_len (added new entry) - already defined above
                    self.builder
                        .build_unconditional_branch(after_init_search)
                        .unwrap();

                    // After init search - finalize
                    self.builder.position_at_end(after_init_search);

                    // TEMPORARY FIX: Since we always append, length is always new_len
                    // No need for PHI node or complex length calculation

                    self.temp_values.insert(object.to_string(), data_ptr.into());
                    if let Some(sym) = self.symbols.get(object) {
                        self.builder.build_store(sym.ptr, data_ptr).unwrap();
                    }

                    // Mark this map as heap-allocated
                    self.heap_maps.insert(object.to_string());

                    // Update metadata with the final length (always new_len since we always append)
                    let mut new_metadata = metadata.clone();
                    new_metadata.length = new_len;
                    self.map_metadata.insert(object.to_string(), new_metadata);

                    // Branch to after block
                    self.builder
                        .build_unconditional_branch(after_all_block)
                        .unwrap();

                    // Handle modifying existing heap-allocated map
                    self.builder.position_at_end(modify_existing_block);

                    // Compute heap_ptr from data pointer by subtracting 8 bytes (header size)
                    // Use ptrtoint/sub/inttoptr instead of negative GEP
                    let data_ptr_int = self
                        .builder
                        .build_ptr_to_int(old_data_ptr, self.context.i64_type(), "data_ptr_int")
                        .unwrap();
                    let heap_ptr_int = self
                        .builder
                        .build_int_sub(
                            data_ptr_int,
                            self.context.i64_type().const_int(8, false),
                            "heap_ptr_int",
                        )
                        .unwrap();
                    let heap_ptr = self
                        .builder
                        .build_int_to_ptr(
                            heap_ptr_int,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "heap_ptr_from_data",
                        )
                        .unwrap();

                    // Read runtime length from heap header
                    let len_field = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                heap_ptr,
                                &[self.context.i32_type().const_int(4, false)],
                                "len_field_set",
                            )
                            .unwrap()
                    };
                    let len_ptr = self
                        .builder
                        .build_pointer_cast(
                            len_field,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "len_ptr_set",
                        )
                        .unwrap();
                    let old_runtime_len = self
                        .builder
                        .build_load(self.context.i32_type(), len_ptr, "old_runtime_len")
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

                    // Use byte-level GEP to avoid array size issues with runtime lengths
                    let search_idx_64 = self
                        .builder
                        .build_int_z_extend(search_idx, self.context.i64_type(), "search_idx_64")
                        .unwrap();
                    let pair_size = pair_type.size_of().unwrap();
                    let byte_offset = self
                        .builder
                        .build_int_mul(search_idx_64, pair_size, "byte_offset_search")
                        .unwrap();
                    let check_pair_i8 = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                old_data_ptr,
                                &[byte_offset],
                                "check_pair_i8",
                            )
                            .unwrap()
                    };
                    let check_pair = self
                        .builder
                        .build_pointer_cast(
                            check_pair_i8,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "check_pair",
                        )
                        .unwrap();
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
                    // Length stays the same, branch to after block
                    self.builder
                        .build_unconditional_branch(after_all_block)
                        .unwrap();

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

                    // Branch to after block
                    self.builder
                        .build_unconditional_branch(after_all_block)
                        .unwrap();

                    // After block - merge point for all paths
                    self.builder.position_at_end(after_all_block);

                    None
                } else {
                    None
                }
            }
            "has" => {
                // Implement map.has(key) with linear search through key-value pairs
                if let Some(metadata) = self.map_metadata.get(object) {
                    let key_val = self.resolve_value(&args[0]);
                    // Load from symbol if it exists, otherwise use resolve_value
                    let map_ptr = if let Some(sym) = self.symbols.get(object) {
                        self.builder
                            .build_load(
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                sym.ptr,
                                "map_ptr_has",
                            )
                            .unwrap()
                            .into_pointer_value()
                    } else {
                        self.resolve_value(object).into_pointer_value()
                    };

                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();

                    // Check if this map has been heap-allocated
                    let is_heap_allocated = self.heap_maps.contains(object);

                    // Read runtime length from heap header if heap-allocated, otherwise use metadata
                    let runtime_len = if is_heap_allocated {
                        // Compute heap_ptr from data pointer by subtracting 8 bytes
                        // Use ptrtoint/sub/inttoptr instead of negative GEP
                        let data_ptr_int = self
                            .builder
                            .build_ptr_to_int(map_ptr, self.context.i64_type(), "data_ptr_int_has")
                            .unwrap();
                        let heap_ptr_int = self
                            .builder
                            .build_int_sub(
                                data_ptr_int,
                                self.context.i64_type().const_int(8, false),
                                "heap_ptr_int_has",
                            )
                            .unwrap();
                        let heap_ptr = self
                            .builder
                            .build_int_to_ptr(
                                heap_ptr_int,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                "heap_ptr_has",
                            )
                            .unwrap();

                        // Read length from offset 4
                        let len_field = unsafe {
                            self.builder
                                .build_gep(
                                    self.context.i8_type(),
                                    heap_ptr,
                                    &[self.context.i32_type().const_int(4, false)],
                                    "len_field_has",
                                )
                                .unwrap()
                        };
                        let len_ptr = self
                            .builder
                            .build_pointer_cast(
                                len_field,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                "len_ptr_has",
                            )
                            .unwrap();
                        self.builder
                            .build_load(self.context.i32_type(), len_ptr, "runtime_len_has")
                            .unwrap()
                            .into_int_value()
                    } else {
                        self.context
                            .i32_type()
                            .const_int(metadata.length as u64, false)
                    };

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

                    // Search through pairs
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

                    // Loop: check if counter < runtime length
                    self.builder.position_at_end(loop_block);
                    let counter = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "counter")
                        .unwrap()
                        .into_int_value();
                    let cmp = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::ULT, counter, runtime_len, "cmp")
                        .unwrap();
                    self.builder
                        .build_conditional_branch(cmp, check_block, not_found_block)
                        .unwrap();

                    // Check block: load key and compare
                    self.builder.position_at_end(check_block);

                    // Calculate pair pointer
                    // For global constants: cast map_ptr to pair array type and use GEP with [0, counter]
                    // For heap maps: use byte-level GEP with counter * sizeof(pair)
                    let pair_ptr = if !is_heap_allocated {
                        // Global constant - use typed GEP
                        let map_array_type = pair_type.array_type(metadata.length as u64 as u32);
                        unsafe {
                            self.builder
                                .build_gep(
                                    map_array_type,
                                    map_ptr,
                                    &[self.context.i32_type().const_zero(), counter],
                                    "pair_ptr",
                                )
                                .unwrap()
                        }
                    } else {
                        // Heap-allocated - use byte-level GEP
                        let pair_size = pair_type.size_of().unwrap();
                        let counter_64 = self
                            .builder
                            .build_int_z_extend(counter, self.context.i64_type(), "counter_64")
                            .unwrap();
                        let offset = self
                            .builder
                            .build_int_mul(counter_64, pair_size, "offset")
                            .unwrap();
                        let pair_ptr_i8 = unsafe {
                            self.builder
                                .build_gep(
                                    self.context.i8_type(),
                                    map_ptr,
                                    &[offset],
                                    "pair_ptr_i8",
                                )
                                .unwrap()
                        };
                        self.builder
                            .build_pointer_cast(
                                pair_ptr_i8,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
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
                    // Check if this map has been heap-allocated
                    let is_heap_allocated = self.heap_maps.contains(object);

                    // Load from symbol if it exists, otherwise use resolve_value
                    let map_ptr = if let Some(sym) = self.symbols.get(object) {
                        self.builder
                            .build_load(
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                sym.ptr,
                                "map_ptr_remove",
                            )
                            .unwrap()
                            .into_pointer_value()
                    } else {
                        self.resolve_value(object).into_pointer_value()
                    };

                    // For heap-allocated maps, read runtime length from heap header
                    let runtime_len = if is_heap_allocated {
                        let data_ptr_int = self
                            .builder
                            .build_ptr_to_int(
                                map_ptr,
                                self.context.i64_type(),
                                "data_ptr_int_remove",
                            )
                            .unwrap();
                        let heap_ptr_int = self
                            .builder
                            .build_int_sub(
                                data_ptr_int,
                                self.context.i64_type().const_int(8, false),
                                "heap_ptr_int_remove",
                            )
                            .unwrap();
                        let heap_ptr = self
                            .builder
                            .build_int_to_ptr(
                                heap_ptr_int,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                "heap_ptr_remove",
                            )
                            .unwrap();
                        let len_field = unsafe {
                            self.builder
                                .build_gep(
                                    self.context.i8_type(),
                                    heap_ptr,
                                    &[self.context.i32_type().const_int(4, false)],
                                    "len_field_remove",
                                )
                                .unwrap()
                        };
                        let len_ptr = self
                            .builder
                            .build_pointer_cast(
                                len_field,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                "len_ptr_remove",
                            )
                            .unwrap();
                        self.builder
                            .build_load(self.context.i32_type(), len_ptr, "runtime_len_remove")
                            .unwrap()
                            .into_int_value()
                    } else {
                        self.context
                            .i32_type()
                            .const_int(metadata.length as u64, false)
                    };

                    // Check if map is empty
                    let is_empty = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            runtime_len,
                            self.context.i32_type().const_int(0, false),
                            "is_empty",
                        )
                        .unwrap();

                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let search_block = self.context.append_basic_block(current_fn, "remove_search");
                    let empty_block = self.context.append_basic_block(current_fn, "remove_empty");

                    self.builder
                        .build_conditional_branch(is_empty, empty_block, search_block)
                        .unwrap();

                    // Empty map path
                    self.builder.position_at_end(empty_block);
                    let false_val = self.context.i32_type().const_int(0, false);
                    self.builder
                        .build_unconditional_branch(search_block)
                        .unwrap();

                    // Search path
                    self.builder.position_at_end(search_block);

                    let key_val = self.resolve_value(&args[0]);

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

                    // Search for key to remove
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

                    // Loop: check if counter < runtime_len
                    self.builder.position_at_end(loop_block);
                    let counter = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "counter")
                        .unwrap()
                        .into_int_value();
                    let cmp = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::ULT, counter, runtime_len, "cmp")
                        .unwrap();
                    self.builder
                        .build_conditional_branch(cmp, check_block, not_found_block)
                        .unwrap();

                    // Check block: load key and compare
                    self.builder.position_at_end(check_block);

                    // Use byte-level GEP to avoid array size issues
                    let counter_64 = self
                        .builder
                        .build_int_z_extend(counter, self.context.i64_type(), "counter_64_remove")
                        .unwrap();
                    let pair_size = pair_type.size_of().unwrap();
                    let byte_offset = self
                        .builder
                        .build_int_mul(counter_64, pair_size, "byte_offset_remove")
                        .unwrap();
                    let pair_ptr_i8 = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                map_ptr,
                                &[byte_offset],
                                "pair_ptr_i8_remove",
                            )
                            .unwrap()
                    };
                    let pair_ptr = self
                        .builder
                        .build_pointer_cast(
                            pair_ptr_i8,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "pair_ptr_remove",
                        )
                        .unwrap();

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

                    // Check if this is the last element
                    let last_idx = self
                        .builder
                        .build_int_sub(
                            runtime_len,
                            self.context.i32_type().const_int(1, false),
                            "last_idx",
                        )
                        .unwrap();
                    let is_last = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            remove_idx,
                            last_idx,
                            "is_last",
                        )
                        .unwrap();

                    let shift_block = self
                        .context
                        .append_basic_block(current_fn, "shift_elements");
                    let shift_loop = self.context.append_basic_block(current_fn, "shift_loop");
                    let shift_check = self.context.append_basic_block(current_fn, "shift_check");
                    let shift_done = self.context.append_basic_block(current_fn, "shift_done");
                    let update_length_block =
                        self.context.append_basic_block(current_fn, "update_length");

                    self.builder
                        .build_conditional_branch(is_last, update_length_block, shift_block)
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
                            runtime_len,
                            "shift_cmp",
                        )
                        .unwrap();
                    self.builder
                        .build_conditional_branch(shift_cmp, shift_check, shift_done)
                        .unwrap();

                    // Shift check: do the actual copy
                    self.builder.position_at_end(shift_check);

                    // Load from current position using byte-level GEP
                    let shift_i_64 = self
                        .builder
                        .build_int_z_extend(shift_i, self.context.i64_type(), "shift_i_64")
                        .unwrap();
                    let shift_offset = self
                        .builder
                        .build_int_mul(shift_i_64, pair_size, "shift_offset")
                        .unwrap();
                    let src_pair_ptr_i8 = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                map_ptr,
                                &[shift_offset],
                                "src_pair_ptr_i8",
                            )
                            .unwrap()
                    };
                    let src_pair_ptr = self
                        .builder
                        .build_pointer_cast(
                            src_pair_ptr_i8,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "src_pair_ptr",
                        )
                        .unwrap();
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

                    // Store to previous position using byte-level GEP
                    let prev_i = self
                        .builder
                        .build_int_sub(
                            shift_i,
                            self.context.i32_type().const_int(1, false),
                            "prev_i",
                        )
                        .unwrap();
                    let prev_i_64 = self
                        .builder
                        .build_int_z_extend(prev_i, self.context.i64_type(), "prev_i_64")
                        .unwrap();
                    let dst_offset = self
                        .builder
                        .build_int_mul(prev_i_64, pair_size, "dst_offset")
                        .unwrap();
                    let dst_pair_ptr_i8 = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                map_ptr,
                                &[dst_offset],
                                "dst_pair_ptr_i8",
                            )
                            .unwrap()
                    };
                    let dst_pair_ptr = self
                        .builder
                        .build_pointer_cast(
                            dst_pair_ptr_i8,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "dst_pair_ptr",
                        )
                        .unwrap();
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

                    // Shift done: branch to update length
                    self.builder.position_at_end(shift_done);
                    self.builder
                        .build_unconditional_branch(update_length_block)
                        .unwrap();

                    // Update length block: decrement the map length in both metadata and heap header
                    self.builder.position_at_end(update_length_block);

                    let new_runtime_len = self
                        .builder
                        .build_int_sub(
                            runtime_len,
                            self.context.i32_type().const_int(1, false),
                            "new_runtime_len_remove",
                        )
                        .unwrap();

                    // Update heap header if heap-allocated
                    if is_heap_allocated {
                        let data_ptr_int = self
                            .builder
                            .build_ptr_to_int(
                                map_ptr,
                                self.context.i64_type(),
                                "data_ptr_int_update_len",
                            )
                            .unwrap();
                        let heap_ptr_int = self
                            .builder
                            .build_int_sub(
                                data_ptr_int,
                                self.context.i64_type().const_int(8, false),
                                "heap_ptr_int_update_len",
                            )
                            .unwrap();
                        let heap_ptr = self
                            .builder
                            .build_int_to_ptr(
                                heap_ptr_int,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                "heap_ptr_update_len",
                            )
                            .unwrap();
                        let len_field = unsafe {
                            self.builder
                                .build_gep(
                                    self.context.i8_type(),
                                    heap_ptr,
                                    &[self.context.i32_type().const_int(4, false)],
                                    "len_field_update",
                                )
                                .unwrap()
                        };
                        let len_ptr = self
                            .builder
                            .build_pointer_cast(
                                len_field,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                "len_ptr_update",
                            )
                            .unwrap();
                        self.builder.build_store(len_ptr, new_runtime_len).unwrap();
                    }

                    // Update metadata
                    metadata.length = metadata.length.saturating_sub(1);
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
                    phi.add_incoming(&[
                        (&true_val, update_length_block),
                        (&false_val, not_found_block),
                    ]);
                    let result = phi.as_basic_value();

                    self.temp_values.insert(dest.to_string(), result);
                    Some(result)
                } else {
                    None
                }
            }
            "isEmpty" => {
                if let Some(metadata) = self.map_metadata.get(object) {
                    // Check if this map has been heap-allocated
                    let is_heap_allocated = self.heap_maps.contains(object);

                    // For heap-allocated maps, read runtime length from heap header
                    let runtime_len = if is_heap_allocated {
                        let map_ptr = if let Some(sym) = self.symbols.get(object) {
                            self.builder
                                .build_load(
                                    self.context.ptr_type(inkwell::AddressSpace::default()),
                                    sym.ptr,
                                    "map_ptr_isempty",
                                )
                                .unwrap()
                                .into_pointer_value()
                        } else {
                            self.resolve_value(object).into_pointer_value()
                        };

                        let data_ptr_int = self
                            .builder
                            .build_ptr_to_int(
                                map_ptr,
                                self.context.i64_type(),
                                "data_ptr_int_isempty",
                            )
                            .unwrap();
                        let heap_ptr_int = self
                            .builder
                            .build_int_sub(
                                data_ptr_int,
                                self.context.i64_type().const_int(8, false),
                                "heap_ptr_int_isempty",
                            )
                            .unwrap();
                        let heap_ptr = self
                            .builder
                            .build_int_to_ptr(
                                heap_ptr_int,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                "heap_ptr_isempty",
                            )
                            .unwrap();
                        let len_field = unsafe {
                            self.builder
                                .build_gep(
                                    self.context.i8_type(),
                                    heap_ptr,
                                    &[self.context.i32_type().const_int(4, false)],
                                    "len_field_isempty",
                                )
                                .unwrap()
                        };
                        let len_ptr = self
                            .builder
                            .build_pointer_cast(
                                len_field,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                "len_ptr_isempty",
                            )
                            .unwrap();
                        self.builder
                            .build_load(self.context.i32_type(), len_ptr, "runtime_len_isempty")
                            .unwrap()
                            .into_int_value()
                    } else {
                        self.context
                            .i32_type()
                            .const_int(metadata.length as u64, false)
                    };

                    let is_zero = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            runtime_len,
                            self.context.i32_type().const_int(0, false),
                            "is_zero_isempty",
                        )
                        .unwrap();

                    let result = self
                        .builder
                        .build_select(
                            is_zero,
                            self.context.i32_type().const_int(1, false),
                            self.context.i32_type().const_int(0, false),
                            "isEmpty_result",
                        )
                        .unwrap();
                    self.temp_values.insert(dest.to_string(), result.into());
                    self.boolean_temps.insert(dest.to_string());
                    Some(result.into())
                } else {
                    let result = self.context.i32_type().const_int(1, false);
                    self.temp_values.insert(dest.to_string(), result.into());
                    self.boolean_temps.insert(dest.to_string());
                    Some(result.into())
                }
            }
            "size" => {
                // For global constant maps, use metadata length
                // For heap-allocated maps (after set/remove), read runtime length from header
                if let Some(metadata) = self.map_metadata.get(object) {
                    // Load from symbol if it exists, otherwise use resolve_value
                    let map_ptr = if let Some(sym) = self.symbols.get(object) {
                        self.builder
                            .build_load(
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                sym.ptr,
                                "map_ptr_size",
                            )
                            .unwrap()
                            .into_pointer_value()
                    } else {
                        self.resolve_value(object).into_pointer_value()
                    };

                    // Check if this map has been heap-allocated
                    let is_heap_allocated = self.heap_maps.contains(object);

                    // Read runtime length from heap header if heap-allocated, otherwise use metadata
                    let size_val = if is_heap_allocated {
                        // Compute heap_ptr from data pointer by subtracting 8 bytes
                        // Use ptrtoint/sub/inttoptr instead of negative GEP
                        let data_ptr_int = self
                            .builder
                            .build_ptr_to_int(map_ptr, self.context.i64_type(), "data_ptr_int_size")
                            .unwrap();
                        let heap_ptr_int = self
                            .builder
                            .build_int_sub(
                                data_ptr_int,
                                self.context.i64_type().const_int(8, false),
                                "heap_ptr_int_size",
                            )
                            .unwrap();
                        let heap_ptr = self
                            .builder
                            .build_int_to_ptr(
                                heap_ptr_int,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                "heap_ptr_size",
                            )
                            .unwrap();

                        // Read length from offset 4
                        let len_field = unsafe {
                            self.builder
                                .build_gep(
                                    self.context.i8_type(),
                                    heap_ptr,
                                    &[self.context.i32_type().const_int(4, false)],
                                    "len_field_size",
                                )
                                .unwrap()
                        };
                        let len_ptr = self
                            .builder
                            .build_pointer_cast(
                                len_field,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                "len_ptr_size",
                            )
                            .unwrap();
                        self.builder
                            .build_load(self.context.i32_type(), len_ptr, "runtime_len_size")
                            .unwrap()
                            .into_int_value()
                    } else {
                        self.context
                            .i32_type()
                            .const_int(metadata.length as u64, false)
                    };

                    self.temp_values.insert(dest.to_string(), size_val.into());
                    Some(size_val.into())
                } else {
                    None
                }
            }
            "clear" => {
                if let Some(mut metadata) = self.map_metadata.get(object).cloned() {
                    // Check if this map has been heap-allocated
                    let is_heap_allocated = self.heap_maps.contains(object);

                    // If heap-allocated, update the length field in the heap header
                    if is_heap_allocated {
                        // Load from symbol if it exists, otherwise use resolve_value
                        let map_ptr = if let Some(sym) = self.symbols.get(object) {
                            self.builder
                                .build_load(
                                    self.context.ptr_type(inkwell::AddressSpace::default()),
                                    sym.ptr,
                                    "map_ptr_clear",
                                )
                                .unwrap()
                                .into_pointer_value()
                        } else {
                            self.resolve_value(object).into_pointer_value()
                        };

                        let data_ptr_int = self
                            .builder
                            .build_ptr_to_int(
                                map_ptr,
                                self.context.i64_type(),
                                "data_ptr_int_clear",
                            )
                            .unwrap();
                        let heap_ptr_int = self
                            .builder
                            .build_int_sub(
                                data_ptr_int,
                                self.context.i64_type().const_int(8, false),
                                "heap_ptr_int_clear",
                            )
                            .unwrap();
                        let heap_ptr = self
                            .builder
                            .build_int_to_ptr(
                                heap_ptr_int,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                "heap_ptr_clear",
                            )
                            .unwrap();
                        let len_field = unsafe {
                            self.builder
                                .build_gep(
                                    self.context.i8_type(),
                                    heap_ptr,
                                    &[self.context.i32_type().const_int(4, false)],
                                    "len_field_clear",
                                )
                                .unwrap()
                        };
                        let len_ptr = self
                            .builder
                            .build_pointer_cast(
                                len_field,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                "len_ptr_clear",
                            )
                            .unwrap();
                        self.builder
                            .build_store(len_ptr, self.context.i32_type().const_int(0, false))
                            .unwrap();
                    }

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
                if let Some(metadata) = self.map_metadata.get(object).cloned() {
                    // Load from symbol if it exists (to get current value after set() calls)
                    let map_ptr = if let Some(sym) = self.symbols.get(object) {
                        self.builder
                            .build_load(
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                sym.ptr,
                                "map_ptr_keys",
                            )
                            .unwrap()
                            .into_pointer_value()
                    } else {
                        self.resolve_value(object).into_pointer_value()
                    };
                    let is_heap_allocated = self.heap_maps.contains(object);

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

                    // Read runtime length from heap header if heap-allocated
                    let runtime_length = if is_heap_allocated {
                        // Compute heap_ptr from data pointer by subtracting 8 bytes
                        // Use GEP with negative offset instead of ptrtoint/inttoptr
                        let heap_ptr_for_len = unsafe {
                            self.builder
                                .build_gep(
                                    self.context.i8_type(),
                                    map_ptr,
                                    &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                    "heap_ptr_for_len_keys",
                                )
                                .unwrap()
                        };

                        // Read length from offset 4
                        let len_field = unsafe {
                            self.builder
                                .build_gep(
                                    self.context.i8_type(),
                                    heap_ptr_for_len,
                                    &[self.context.i32_type().const_int(4, false)],
                                    "len_field_keys",
                                )
                                .unwrap()
                        };
                        self.builder
                            .build_load(self.context.i32_type(), len_field, "runtime_len_keys")
                            .unwrap()
                            .into_int_value()
                    } else {
                        // Use static metadata length for non-heap maps
                        self.context
                            .i32_type()
                            .const_int(metadata.length as u64, false)
                    };

                    // Check if empty at runtime
                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let empty_block = self.context.append_basic_block(current_fn, "keys_empty");
                    let non_empty_block = self
                        .context
                        .append_basic_block(current_fn, "keys_non_empty");
                    let merge_block = self.context.append_basic_block(current_fn, "keys_merge");

                    let is_empty = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            runtime_length,
                            self.context.i32_type().const_zero(),
                            "is_empty_keys",
                        )
                        .unwrap();
                    self.builder
                        .build_conditional_branch(is_empty, empty_block, non_empty_block)
                        .unwrap();

                    // Empty case: return null pointer
                    self.builder.position_at_end(empty_block);
                    let empty_ptr = self
                        .context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null();
                    self.builder
                        .build_unconditional_branch(merge_block)
                        .unwrap();
                    let empty_block_end = self.builder.get_insert_block().unwrap();

                    // Non-empty case: allocate and fill keys array
                    self.builder.position_at_end(non_empty_block);

                    // Calculate size for keys array based on runtime length
                    let key_elem_size = key_type.size_of().unwrap();
                    let runtime_len_64 = self
                        .builder
                        .build_int_z_extend(
                            runtime_length,
                            self.context.i64_type(),
                            "runtime_len_64_keys",
                        )
                        .unwrap();
                    let key_array_size = self
                        .builder
                        .build_int_mul(runtime_len_64, key_elem_size, "key_array_size")
                        .unwrap();
                    let header_size = self.context.i64_type().const_int(8, false);
                    let total_size = self
                        .builder
                        .build_int_add(header_size, key_array_size, "total_size_keys")
                        .unwrap();

                    let malloc_fn = self.get_or_declare_malloc();
                    let heap_ptr = self
                        .builder
                        .build_call(malloc_fn, &[total_size.into()], "heap_ptr_keys")
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
                            "rc_ptr_keys",
                        )
                        .unwrap();
                    self.builder
                        .build_store(rc_ptr, self.context.i32_type().const_int(1, false))
                        .unwrap();

                    // Store length at offset 4
                    let len_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i32_type(),
                                heap_ptr,
                                &[self.context.i32_type().const_int(1, false)],
                                "len_ptr_store_keys",
                            )
                            .unwrap()
                    };
                    self.builder.build_store(len_ptr, runtime_length).unwrap();

                    // Get data pointer
                    let data_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                heap_ptr,
                                &[self.context.i32_type().const_int(8, false)],
                                "data_ptr_keys",
                            )
                            .unwrap()
                    };

                    // Loop through map and copy keys using byte-level GEP
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

                    // Loop check - use runtime_length for comparison
                    self.builder.position_at_end(loop_block);
                    let counter = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "counter_keys")
                        .unwrap()
                        .into_int_value();
                    let cmp = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::ULT,
                            counter,
                            runtime_length,
                            "cmp_keys",
                        )
                        .unwrap();
                    self.builder
                        .build_conditional_branch(cmp, loop_body, loop_done)
                        .unwrap();

                    // Loop body: copy key using byte-level GEP
                    self.builder.position_at_end(loop_body);

                    // Calculate byte offset for pair access
                    let pair_size = pair_type.size_of().unwrap();
                    let counter_64 = self
                        .builder
                        .build_int_z_extend(counter, self.context.i64_type(), "counter_64_keys")
                        .unwrap();
                    let byte_offset = self
                        .builder
                        .build_int_mul(counter_64, pair_size, "byte_offset_keys")
                        .unwrap();

                    let pair_ptr_i8 = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                map_ptr,
                                &[byte_offset],
                                "pair_ptr_i8_keys",
                            )
                            .unwrap()
                    };
                    let pair_ptr = self
                        .builder
                        .build_pointer_cast(
                            pair_ptr_i8,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "pair_ptr_keys",
                        )
                        .unwrap();

                    let key_ptr = self
                        .builder
                        .build_struct_gep(pair_type, pair_ptr, 0, "key_ptr_keys")
                        .unwrap();
                    let key_val = self
                        .builder
                        .build_load(key_type, key_ptr, "key_val_keys")
                        .unwrap();

                    // Store in output array using byte-level GEP
                    let out_byte_offset = self
                        .builder
                        .build_int_mul(counter_64, key_elem_size, "out_byte_offset_keys")
                        .unwrap();
                    let elem_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                data_ptr,
                                &[out_byte_offset],
                                "elem_ptr_keys",
                            )
                            .unwrap()
                    };
                    let elem_ptr_typed = self
                        .builder
                        .build_pointer_cast(
                            elem_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "elem_ptr_typed_keys",
                        )
                        .unwrap();
                    self.builder.build_store(elem_ptr_typed, key_val).unwrap();

                    // Increment counter
                    let next = self
                        .builder
                        .build_int_add(
                            counter,
                            self.context.i32_type().const_int(1, false),
                            "next_keys",
                        )
                        .unwrap();
                    self.builder.build_store(counter_ptr, next).unwrap();
                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Done with loop
                    self.builder.position_at_end(loop_done);
                    self.builder
                        .build_unconditional_branch(merge_block)
                        .unwrap();
                    let non_empty_block_end = self.builder.get_insert_block().unwrap();

                    // Merge block: PHI for result pointer and heap pointer
                    self.builder.position_at_end(merge_block);
                    let result_phi = self
                        .builder
                        .build_phi(
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "keys_result",
                        )
                        .unwrap();
                    result_phi.add_incoming(&[
                        (&empty_ptr, empty_block_end),
                        (&data_ptr, non_empty_block_end),
                    ]);
                    let result_ptr = result_phi.as_basic_value().into_pointer_value();

                    // PHI for heap pointer (null for empty case, heap_ptr for non-empty)
                    let heap_phi = self
                        .builder
                        .build_phi(
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "keys_heap",
                        )
                        .unwrap();
                    let null_heap_ptr = self
                        .context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null();
                    heap_phi.add_incoming(&[
                        (&null_heap_ptr, empty_block_end),
                        (&heap_ptr, non_empty_block_end),
                    ]);
                    let final_heap_ptr = heap_phi.as_basic_value().into_pointer_value();

                    // Store result
                    self.temp_values.insert(dest.to_string(), result_ptr.into());
                    self.heap_arrays.insert(dest.to_string());
                    self.heap_pointers.insert(dest.to_string(), final_heap_ptr);

                    // CRITICAL: Store into symbol alloca if it exists, so print_array can load it
                    if let Some(sym) = self.symbols.get(dest) {
                        self.builder.build_store(sym.ptr, result_ptr).unwrap();
                    }

                    // For array metadata, we use a placeholder length since actual length is runtime
                    // The print_array function will read from heap header
                    let arr_meta = ArrayMetadata {
                        length: if is_heap_allocated {
                            0
                        } else {
                            metadata.length
                        },
                        element_type: if metadata.key_is_string {
                            "Str".to_string()
                        } else {
                            "Int".to_string()
                        },
                        contains_strings: metadata.key_is_string,
                    };
                    self.array_metadata.insert(dest.to_string(), arr_meta);
                    Some(result_ptr.into())
                } else {
                    None
                }
            }
            "values" => {
                // Extract all values from the map into a new array
                if let Some(metadata) = self.map_metadata.get(object).cloned() {
                    // Load from symbol if it exists (to get current value after set() calls)
                    let map_ptr = if let Some(sym) = self.symbols.get(object) {
                        self.builder
                            .build_load(
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                sym.ptr,
                                "map_ptr_values",
                            )
                            .unwrap()
                            .into_pointer_value()
                    } else {
                        self.resolve_value(object).into_pointer_value()
                    };
                    let is_heap_allocated = self.heap_maps.contains(object);

                    // Determine types
                    let key_type: inkwell::types::BasicTypeEnum = if metadata.key_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        self.context.i32_type().into()
                    };

                    // Determine value type - strings and structs are pointers, primitives are i32
                    let val_type: inkwell::types::BasicTypeEnum = if metadata.value_is_string {
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else if metadata.value_needs_rc
                        || self.struct_metadata.contains_key(&metadata.value_type)
                    {
                        // Struct values are stored as pointers
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        self.context.i32_type().into()
                    };

                    let pair_type = self.context.struct_type(&[key_type, val_type], false);

                    // Read runtime length from heap header if heap-allocated
                    let runtime_length = if is_heap_allocated {
                        // Use GEP with negative offset instead of ptrtoint/inttoptr
                        let heap_ptr_for_len = unsafe {
                            self.builder
                                .build_gep(
                                    self.context.i8_type(),
                                    map_ptr,
                                    &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                                    "heap_ptr_for_len_values",
                                )
                                .unwrap()
                        };

                        // Read length from offset 4
                        let len_field = unsafe {
                            self.builder
                                .build_gep(
                                    self.context.i8_type(),
                                    heap_ptr_for_len,
                                    &[self.context.i32_type().const_int(4, false)],
                                    "len_field_values",
                                )
                                .unwrap()
                        };
                        self.builder
                            .build_load(self.context.i32_type(), len_field, "runtime_len_values")
                            .unwrap()
                            .into_int_value()
                    } else {
                        // Use static metadata length for non-heap maps
                        self.context
                            .i32_type()
                            .const_int(metadata.length as u64, false)
                    };

                    // Check if empty at runtime
                    let current_fn = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let empty_block = self.context.append_basic_block(current_fn, "values_empty");
                    let non_empty_block = self
                        .context
                        .append_basic_block(current_fn, "values_non_empty");
                    let merge_block = self.context.append_basic_block(current_fn, "values_merge");

                    let is_empty = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            runtime_length,
                            self.context.i32_type().const_zero(),
                            "is_empty_values",
                        )
                        .unwrap();
                    self.builder
                        .build_conditional_branch(is_empty, empty_block, non_empty_block)
                        .unwrap();

                    // Empty case: return null pointer
                    self.builder.position_at_end(empty_block);
                    let empty_ptr = self
                        .context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null();
                    self.builder
                        .build_unconditional_branch(merge_block)
                        .unwrap();
                    let empty_block_end = self.builder.get_insert_block().unwrap();

                    // Non-empty case: allocate and fill values array
                    self.builder.position_at_end(non_empty_block);

                    // Calculate size for values array based on runtime length
                    let val_elem_size = val_type.size_of().unwrap();
                    let runtime_len_64 = self
                        .builder
                        .build_int_z_extend(
                            runtime_length,
                            self.context.i64_type(),
                            "runtime_len_64_values",
                        )
                        .unwrap();
                    let val_array_size = self
                        .builder
                        .build_int_mul(runtime_len_64, val_elem_size, "val_array_size")
                        .unwrap();
                    let header_size = self.context.i64_type().const_int(8, false);
                    let total_size = self
                        .builder
                        .build_int_add(header_size, val_array_size, "total_size_values")
                        .unwrap();

                    let malloc_fn = self.get_or_declare_malloc();
                    let heap_ptr = self
                        .builder
                        .build_call(malloc_fn, &[total_size.into()], "heap_ptr_values")
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
                            "rc_ptr_values",
                        )
                        .unwrap();
                    self.builder
                        .build_store(rc_ptr, self.context.i32_type().const_int(1, false))
                        .unwrap();

                    // Store length at offset 4
                    let len_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i32_type(),
                                heap_ptr,
                                &[self.context.i32_type().const_int(1, false)],
                                "len_ptr_store_values",
                            )
                            .unwrap()
                    };
                    self.builder.build_store(len_ptr, runtime_length).unwrap();

                    // Get data pointer
                    let data_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                heap_ptr,
                                &[self.context.i32_type().const_int(8, false)],
                                "data_ptr_values",
                            )
                            .unwrap()
                    };

                    // Loop through map and copy values using byte-level GEP
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

                    // Loop check - use runtime_length for comparison
                    self.builder.position_at_end(loop_block);
                    let counter = self
                        .builder
                        .build_load(self.context.i32_type(), counter_ptr, "counter_values")
                        .unwrap()
                        .into_int_value();
                    let cmp = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::ULT,
                            counter,
                            runtime_length,
                            "cmp_values",
                        )
                        .unwrap();
                    self.builder
                        .build_conditional_branch(cmp, loop_body, loop_done)
                        .unwrap();

                    // Loop body: copy value using byte-level GEP
                    self.builder.position_at_end(loop_body);

                    // Calculate byte offset for pair access
                    let pair_size = pair_type.size_of().unwrap();
                    let counter_64 = self
                        .builder
                        .build_int_z_extend(counter, self.context.i64_type(), "counter_64_values")
                        .unwrap();
                    let byte_offset = self
                        .builder
                        .build_int_mul(counter_64, pair_size, "byte_offset_values")
                        .unwrap();

                    let pair_ptr_i8 = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                map_ptr,
                                &[byte_offset],
                                "pair_ptr_i8_values",
                            )
                            .unwrap()
                    };
                    let pair_ptr = self
                        .builder
                        .build_pointer_cast(
                            pair_ptr_i8,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "pair_ptr_values",
                        )
                        .unwrap();

                    let val_ptr = self
                        .builder
                        .build_struct_gep(pair_type, pair_ptr, 1, "val_ptr_values")
                        .unwrap();
                    let val_val = self
                        .builder
                        .build_load(val_type, val_ptr, "val_val_values")
                        .unwrap();

                    // Store in output array using byte-level GEP
                    let out_byte_offset = self
                        .builder
                        .build_int_mul(counter_64, val_elem_size, "out_byte_offset_values")
                        .unwrap();
                    let elem_ptr = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                data_ptr,
                                &[out_byte_offset],
                                "elem_ptr_values",
                            )
                            .unwrap()
                    };
                    let elem_ptr_typed = self
                        .builder
                        .build_pointer_cast(
                            elem_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "elem_ptr_typed_values",
                        )
                        .unwrap();
                    self.builder.build_store(elem_ptr_typed, val_val).unwrap();

                    // Increment counter
                    let next = self
                        .builder
                        .build_int_add(
                            counter,
                            self.context.i32_type().const_int(1, false),
                            "next_values",
                        )
                        .unwrap();
                    self.builder.build_store(counter_ptr, next).unwrap();
                    self.builder.build_unconditional_branch(loop_block).unwrap();

                    // Done with loop
                    self.builder.position_at_end(loop_done);
                    self.builder
                        .build_unconditional_branch(merge_block)
                        .unwrap();
                    let non_empty_block_end = self.builder.get_insert_block().unwrap();

                    // Merge block: PHI for result pointer and heap pointer
                    self.builder.position_at_end(merge_block);
                    let result_phi = self
                        .builder
                        .build_phi(
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "values_result",
                        )
                        .unwrap();
                    result_phi.add_incoming(&[
                        (&empty_ptr, empty_block_end),
                        (&data_ptr, non_empty_block_end),
                    ]);
                    let result_ptr = result_phi.as_basic_value().into_pointer_value();

                    // PHI for heap pointer (null for empty case, heap_ptr for non-empty)
                    let heap_phi = self
                        .builder
                        .build_phi(
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            "values_heap",
                        )
                        .unwrap();
                    let null_heap_ptr = self
                        .context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null();
                    heap_phi.add_incoming(&[
                        (&null_heap_ptr, empty_block_end),
                        (&heap_ptr, non_empty_block_end),
                    ]);
                    let final_heap_ptr = heap_phi.as_basic_value().into_pointer_value();

                    // Store result
                    self.temp_values.insert(dest.to_string(), result_ptr.into());
                    self.heap_arrays.insert(dest.to_string());
                    self.heap_pointers.insert(dest.to_string(), final_heap_ptr);

                    // CRITICAL: Store into symbol alloca if it exists, so print_array can load it
                    if let Some(sym) = self.symbols.get(dest) {
                        self.builder.build_store(sym.ptr, result_ptr).unwrap();
                    }

                    // For array metadata, we use a placeholder length since actual length is runtime
                    // The print_array function will read from heap header
                    let arr_meta = ArrayMetadata {
                        length: if is_heap_allocated {
                            0
                        } else {
                            metadata.length
                        },
                        element_type: if metadata.value_is_string {
                            "Str".to_string()
                        } else if metadata.value_needs_rc
                            || self.struct_metadata.contains_key(&metadata.value_type)
                        {
                            // Preserve struct type name for array element type
                            metadata.value_type.clone()
                        } else {
                            "Int".to_string()
                        },
                        contains_strings: metadata.value_is_string,
                    };
                    self.array_metadata.insert(dest.to_string(), arr_meta);
                    Some(result_ptr.into())
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}
