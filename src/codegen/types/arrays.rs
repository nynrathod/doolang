use crate::codegen::core::CodeGen;
use inkwell::types::BasicType;
use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

impl<'ctx> CodeGen<'ctx> {
    pub fn generate_array_with_metadata(
        &mut self,
        name: &str,
        elements: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        let element_values: Vec<BasicValueEnum<'ctx>> =
            elements.iter().map(|el| self.resolve_value(el)).collect();

        // Allow empty arrays: default element type to Int if elements is empty
        let elem_type = if element_values.is_empty() {
            self.context.i32_type().as_basic_type_enum()
        } else {
            element_values[0].get_type()
        };

        let array_type = elem_type.array_type(elements.len() as u32);

        // Track string pointers
        let str_ptrs: Vec<BasicValueEnum<'ctx>> = element_values
            .iter()
            .enumerate()
            .filter(|(i, _)| self.heap_strings.contains(&elements[*i]))
            .map(|(_, val)| *val)
            .collect();

        let contains_strings = !str_ptrs.is_empty();

        if contains_strings {
            self.composite_string_ptrs
                .insert(name.to_string(), str_ptrs);
        }

        // Store metadata
        // Check if this is a bool array by seeing if all elements are 0 or 1
        let element_type_name = if elem_type.is_float_type() {
            "Float"
        } else if elem_type.is_int_type() {
            // Heuristic: if all values are 0 or 1, treat as Bool array
            let all_bool_like = element_values.iter().all(|v| {
                if let Some(int_val) = v.into_int_value().get_zero_extended_constant() {
                    int_val == 0 || int_val == 1
                } else {
                    false
                }
            });

            if !element_values.is_empty() && all_bool_like {
                "Bool"
            } else {
                "Int"
            }
        } else if elem_type.is_pointer_type() {
            "Str"
        } else {
            "Unknown"
        };

        let metadata = crate::codegen::ArrayMetadata {
            length: elements.len(),
            element_type: element_type_name.to_string(),
            contains_strings,
        };

        // Register metadata under EXTENSIVE name variations for better lookup
        // This is CRITICAL for arrays created inside loops
        let base_name = name.trim_start_matches('%').trim_end_matches("_array");
        let name_variations = vec![
            name.to_string(),
            name.trim_end_matches("_array").to_string(),
            name.trim_start_matches('%').to_string(),
            format!("{}_array", name),
            format!("{}_array", name.trim_start_matches('%')),
            format!("{}_array", base_name),
            base_name.to_string(),
            format!("{}item_array", base_name),
            format!("{}item", base_name),
        ];

        for variation in name_variations {
            self.array_metadata.insert(variation, metadata.clone());
        }

        // HEAP ALLOCATE with RC header and length field
        // Layout: [RC: 4 bytes][Length: 4 bytes][data...]
        let malloc_fn = self.get_or_declare_malloc();
        let array_size = array_type.size_of().unwrap();
        let header_size = self.context.i64_type().const_int(8, false); // RC + Length = 8 bytes (use i64)
        let total_size = self
            .builder
            .build_int_add(header_size, array_size, "total_size")
            .unwrap();

        let heap_ptr = self
            .builder
            .build_call(malloc_fn, &[total_size.into()], "heap_array")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Store RC = 1 at offset 0
        let rc_ptr = self
            .builder
            .build_pointer_cast(
                heap_ptr,
                self.context.ptr_type(AddressSpace::default()),
                "rc_ptr",
            )
            .unwrap();
        self.builder
            .build_store(rc_ptr, self.context.i32_type().const_int(1, false))
            .unwrap();

        // Store array length at offset 4
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
                self.context.ptr_type(AddressSpace::default()),
                "len_ptr_cast",
            )
            .unwrap();
        self.builder
            .build_store(
                len_ptr_cast,
                self.context
                    .i32_type()
                    .const_int(elements.len() as u64, false),
            )
            .unwrap();

        // Get data pointer at offset 8
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

        // Cast to array type pointer
        let array_ptr = self
            .builder
            .build_pointer_cast(
                data_ptr,
                self.context.ptr_type(AddressSpace::default()),
                "array_ptr",
            )
            .unwrap();

        // Store elements
        for (i, val) in element_values.iter().enumerate() {
            let idx = self.context.i32_type().const_int(i as u64, false);
            let elem_ptr = unsafe {
                self.builder
                    .build_gep(
                        array_type,
                        array_ptr,
                        &[self.context.i32_type().const_zero(), idx],
                        &format!("elem_{}", i),
                    )
                    .unwrap()
            };
            self.builder.build_store(elem_ptr, *val).unwrap();
        }

        // CRITICAL: Remove element strings from heap_strings - they're now owned by the array
        // The array's composite_string_ptrs tracking will handle their cleanup
        for elem_name in elements {
            if self.heap_strings.contains(elem_name) {
                self.heap_strings.remove(elem_name);
            }
        }

        self.temp_values.insert(name.to_string(), data_ptr.into());
        self.heap_arrays.insert(name.to_string());

        // Store full heap pointer for tuple returns
        self.heap_pointers.insert(name.to_string(), heap_ptr);

        Some(data_ptr.into())
    }

    /// Helper implementations for array and map operations with RC
    pub fn get_array_length(&self, array_name: &str) -> inkwell::values::IntValue<'ctx> {
        // STEP 1: Direct metadata lookup
        if let Some(metadata) = self.array_metadata.get(array_name) {
            return self
                .context
                .i32_type()
                .const_int(metadata.length as u64, false);
        }

        // STEP 2: Try name variations
        let search_names = vec![
            array_name.trim_end_matches("_array").to_string(),
            format!("{}_array", array_name),
            array_name.trim_start_matches('%').to_string(),
        ];

        for search_name in &search_names {
            if let Some(metadata) = self.array_metadata.get(search_name) {
                return self
                    .context
                    .i32_type()
                    .const_int(metadata.length as u64, false);
            }
        }

        // STEP 3: Try pointer equality matching
        if let Some(sym) = self.symbols.get(array_name) {
            if let Ok(loaded) = self.builder.build_load(sym.ty, sym.ptr, "check_ptr") {
                if loaded.is_pointer_value() {
                    let ptr_val = loaded.into_pointer_value();

                    for (other_name, metadata) in &self.array_metadata {
                        if let Some(other_val) = self.temp_values.get(other_name) {
                            if other_val.is_pointer_value()
                                && other_val.into_pointer_value() == ptr_val
                            {
                                return self
                                    .context
                                    .i32_type()
                                    .const_int(metadata.length as u64, false);
                            }
                        }
                    }
                }
            }
        }

        // STEP 4: CRITICAL - Runtime length extraction from heap header
        // For dynamically created arrays (like innerarr), extract length at runtime

        // Try symbols first
        if let Some(sym) = self.symbols.get(array_name) {
            if let Ok(loaded) = self.builder.build_load(sym.ty, sym.ptr, "runtime_load") {
                if loaded.is_pointer_value() {
                    let arr_ptr = loaded.into_pointer_value();

                    // Array layout: [RC: 4 bytes][Length: 4 bytes][data at offset 8]
                    // arr_ptr points to data, so length is at offset -4
                    let len_ptr_result = unsafe {
                        self.builder.build_in_bounds_gep(
                            self.context.i8_type(),
                            arr_ptr,
                            &[self.context.i32_type().const_int((-4_i32) as u64, true)],
                            &format!("{}_runtime_len_ptr", array_name),
                        )
                    };

                    if let Ok(len_ptr) = len_ptr_result {
                        let len_ptr_cast_result = self.builder.build_pointer_cast(
                            len_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            &format!("{}_len_ptr_cast", array_name),
                        );

                        if let Ok(len_ptr_cast) = len_ptr_cast_result {
                            if let Ok(runtime_len) = self.builder.build_load(
                                self.context.i32_type(),
                                len_ptr_cast,
                                &format!("{}_runtime_len", array_name),
                            ) {
                                eprintln!(
                                    "[SUCCESS] Extracted runtime length for '{}'",
                                    array_name
                                );
                                return runtime_len.into_int_value();
                            }
                        }
                    }
                }
            }
        }

        // STEP 5: Try temp_values (for tuple-extracted arrays)
        if let Some(temp_val) = self.temp_values.get(array_name) {
            if temp_val.is_pointer_value() {
                let arr_ptr = temp_val.into_pointer_value();

                // Array layout: [RC: 4 bytes][Length: 4 bytes][data at offset 8]
                // arr_ptr points to data, so length is at offset -4
                let len_ptr_result = unsafe {
                    self.builder.build_in_bounds_gep(
                        self.context.i8_type(),
                        arr_ptr,
                        &[self.context.i32_type().const_int((-4_i32) as u64, true)],
                        &format!("{}_runtime_len_ptr", array_name),
                    )
                };

                if let Ok(len_ptr) = len_ptr_result {
                    let len_ptr_cast_result = self.builder.build_pointer_cast(
                        len_ptr,
                        self.context.ptr_type(inkwell::AddressSpace::default()),
                        &format!("{}_len_ptr_cast", array_name),
                    );

                    if let Ok(len_ptr_cast) = len_ptr_cast_result {
                        if let Ok(runtime_len) = self.builder.build_load(
                            self.context.i32_type(),
                            len_ptr_cast,
                            &format!("{}_runtime_len", array_name),
                        ) {
                            eprintln!(
                                "[SUCCESS] Extracted runtime length for '{}' from temp_values",
                                array_name
                            );
                            return runtime_len.into_int_value();
                        }
                    }
                }
            }
        }

        // FINAL FALLBACK: Return 0 to skip loop safely
        self.context.i32_type().const_int(0, false)
    }

    pub fn get_array_element_type(&self, array_name: &str) -> inkwell::types::BasicTypeEnum<'ctx> {
        if let Some(metadata) = self.array_metadata.get(array_name) {
            match metadata.element_type.as_str() {
                "Int" => self.context.i32_type().into(), // Only i32 for integers
                "Float" => self.context.f64_type().into(), // f64 for floating point
                "Bool" => self.context.bool_type().into(),
                "Str" => self.context.ptr_type(AddressSpace::default()).into(),
                _ => self.context.i32_type().into(),
            }
        } else {
            self.context.i32_type().into()
        }
    }

    /// Returns true if the array contains string elements.
    pub fn array_contains_strings(&self, array_name: &str) -> bool {
        if let Some(metadata) = self.array_metadata.get(array_name) {
            metadata.contains_strings
        } else {
            false
        }
    }

    /// Load array element with proper RC management for strings
    pub fn load_array_element_with_rc(
        &mut self,
        array_ptr: inkwell::values::PointerValue<'ctx>,
        index: inkwell::values::IntValue<'ctx>,
        elem_type: inkwell::types::BasicTypeEnum<'ctx>,
        is_string: bool,
    ) -> inkwell::values::BasicValueEnum<'ctx> {
        // GEP to get element pointer
        let elem_ptr = unsafe {
            self.builder.build_gep(
                elem_type.array_type(0), // We need actual array type here
                array_ptr,
                &[self.context.i32_type().const_zero(), index],
                "elem_ptr",
            )
        }
        .unwrap();

        // Load element
        let elem_val = self
            .builder
            .build_load(elem_type, elem_ptr, "elem")
            .unwrap();

        // If it's a heap-allocated string, increment RC
        if is_string {
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
        }

        elem_val
    }

    /// Generate cleanup when exiting a loop (called from loops.rs)
    pub fn generate_loop_exit_cleanup(&mut self) {
        // Get current loop context
        if let Some(loop_ctx) = self.exit_loop() {
            // Clean up any heap-allocated loop variables
            for var in &loop_ctx.loop_vars {
                if self.heap_strings.contains(var) {
                    self.emit_decref(var);
                    self.heap_strings.remove(var);
                }
                if self.heap_arrays.contains(var) {
                    // Free the array - __decref will handle element cleanup recursively
                    self.emit_decref(var);
                } else if self.heap_maps.contains(var) {
                    // Clean up strings in map if needed
                    if let Some(str_names) = self.composite_strings.get(var) {
                        for str_name in str_names.clone() {
                            if let Some(val) = self.temp_values.get(&str_name) {
                                if val.is_pointer_value() {
                                    let data_ptr = val.into_pointer_value();
                                    let rc_header = unsafe {
                                        self.builder.build_in_bounds_gep(
                                            self.context.i8_type(),
                                            data_ptr,
                                            &[self
                                                .context
                                                .i32_type()
                                                .const_int((-8_i32) as u64, true)],
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
                    }
                    self.emit_decref(var);
                    self.heap_maps.remove(var);
                }
            }
        }
    }

    /// Helper method to print an array
    pub fn print_array(&mut self, array_name: &str) {
        let printf_fn = self.get_or_declare_printf();

        // Print opening bracket
        let open_bracket = self
            .builder
            .build_global_string_ptr("[", "open_bracket")
            .unwrap();
        self.builder
            .build_call(printf_fn, &[open_bracket.as_pointer_value().into()], "")
            .unwrap();

        // Get array metadata - try multiple name variations
        let mut metadata = self.array_metadata.get(array_name).cloned();

        // If not found, try variations
        if metadata.is_none() {
            let variations = vec![
                array_name.trim_start_matches('%').to_string(),
                array_name.trim_end_matches("_array").to_string(),
                format!("{}_array", array_name),
                format!("{}_array", array_name.trim_start_matches('%')),
            ];

            for var in variations {
                if let Some(meta) = self.array_metadata.get(&var).cloned() {
                    metadata = Some(meta);
                    break;
                }
            }
        }

        if let Some(metadata) = metadata {
            // Get pointer to the array data
            let array_ptr = if self.symbols.contains_key(array_name) {
                // Variable case: resolve_pointer gives us the alloca,
                // we need to load the actual array pointer from it
                let var_alloca = self.resolve_pointer(array_name);
                self.builder
                    .build_load(
                        self.context.ptr_type(AddressSpace::default()),
                        var_alloca,
                        "array_data_ptr",
                    )
                    .unwrap()
                    .into_pointer_value()
            } else {
                // For temporary arrays, resolve_value should work
                self.resolve_value(array_name).into_pointer_value()
            };
            let elem_type = if metadata.element_type == "Str" {
                self.context
                    .ptr_type(AddressSpace::default())
                    .as_basic_type_enum()
            } else if metadata.element_type == "Float" {
                self.context.f64_type().as_basic_type_enum()
            } else {
                self.context.i32_type().as_basic_type_enum()
            };

            let array_type = elem_type.array_type(metadata.length as u32);
            let typed_array_ptr = self
                .builder
                .build_pointer_cast(
                    array_ptr,
                    self.context.ptr_type(AddressSpace::default()),
                    "typed_array_ptr",
                )
                .unwrap();

            // Read heap length from header
            // Array layout: [RC: 4 bytes][Length: 4 bytes][data...]
            // Data pointer is at offset +8, so RC header is at -8
            // Length field is at -8 + 4 = -4
            let rc_header_ptr = unsafe {
                self.builder
                    .build_gep(
                        self.context.i8_type(),
                        array_ptr,
                        &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                        "rc_header_ptr_print",
                    )
                    .unwrap()
            };

            let len_field_ptr = unsafe {
                self.builder
                    .build_gep(
                        self.context.i8_type(),
                        rc_header_ptr,
                        &[self.context.i32_type().const_int(4, false)],
                        "len_field_ptr_print",
                    )
                    .unwrap()
            };

            let len_ptr_cast = self
                .builder
                .build_pointer_cast(
                    len_field_ptr,
                    self.context.ptr_type(AddressSpace::default()),
                    "len_ptr_cast_print",
                )
                .unwrap();
            let heap_len = self
                .builder
                .build_load(self.context.i32_type(), len_ptr_cast, "heap_len_print")
                .unwrap()
                .into_int_value();

            // Create a dynamic loop to print elements based on runtime heap_len
            let print_fn = self
                .builder
                .get_insert_block()
                .unwrap()
                .get_parent()
                .unwrap();

            let loop_header = self
                .context
                .append_basic_block(print_fn, "print_loop_header");
            let loop_body = self.context.append_basic_block(print_fn, "print_loop_body");
            let loop_end = self.context.append_basic_block(print_fn, "print_loop_end");

            // Create a loop counter
            let counter_alloca = self
                .builder
                .build_alloca(self.context.i32_type(), "print_counter")
                .unwrap();
            self.builder
                .build_store(counter_alloca, self.context.i32_type().const_zero())
                .unwrap();

            // Jump to loop header
            self.builder
                .build_unconditional_branch(loop_header)
                .unwrap();

            // Loop header: check if counter < heap_len
            self.builder.position_at_end(loop_header);
            let counter_val = self
                .builder
                .build_load(self.context.i32_type(), counter_alloca, "counter")
                .unwrap()
                .into_int_value();
            let should_continue = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::ULT,
                    counter_val,
                    heap_len,
                    "should_continue",
                )
                .unwrap();
            self.builder
                .build_conditional_branch(should_continue, loop_body, loop_end)
                .unwrap();

            // Loop body: print element at counter_val index
            self.builder.position_at_end(loop_body);

            let elem_ptr = unsafe {
                self.builder.build_gep(
                    array_type,
                    typed_array_ptr,
                    &[self.context.i32_type().const_zero(), counter_val],
                    "elem_ptr",
                )
            }
            .unwrap();

            let elem_val = self
                .builder
                .build_load(elem_type, elem_ptr, "elem")
                .unwrap();

            // Determine if this is the last element
            let next_counter = self
                .builder
                .build_int_add(
                    counter_val,
                    self.context.i32_type().const_int(1, false),
                    "next_counter",
                )
                .unwrap();
            let is_last_element = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::EQ,
                    next_counter,
                    heap_len,
                    "is_last_element",
                )
                .unwrap();

            // Print the element based on its type
            if metadata.element_type == "Str" {
                let with_comma = self
                    .builder
                    .build_global_string_ptr("\"%s\", ", "array_elem_fmt_comma")
                    .unwrap();
                let without_comma = self
                    .builder
                    .build_global_string_ptr("\"%s\"", "array_elem_fmt_no_comma")
                    .unwrap();

                let format_global = self
                    .builder
                    .build_select(
                        is_last_element,
                        without_comma.as_pointer_value(),
                        with_comma.as_pointer_value(),
                        "select_format_str",
                    )
                    .unwrap()
                    .into_pointer_value();

                self.builder
                    .build_call(printf_fn, &[format_global.into(), elem_val.into()], "")
                    .unwrap();
            } else if metadata.element_type == "Bool" {
                // Check if bool value is true (non-zero) or false (zero)
                let int_val = elem_val.into_int_value();
                let zero = self.context.i32_type().const_int(0, false);
                let is_true = self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::NE, int_val, zero, "bool_is_true")
                    .unwrap();

                // Build format strings for true/false with and without commas
                let true_with_comma = self
                    .builder
                    .build_global_string_ptr("true, ", "array_bool_true_comma")
                    .unwrap();
                let true_without_comma = self
                    .builder
                    .build_global_string_ptr("true", "array_bool_true_no_comma")
                    .unwrap();
                let false_with_comma = self
                    .builder
                    .build_global_string_ptr("false, ", "array_bool_false_comma")
                    .unwrap();
                let false_without_comma = self
                    .builder
                    .build_global_string_ptr("false", "array_bool_false_no_comma")
                    .unwrap();

                // Select based on is_true
                let true_format = self
                    .builder
                    .build_select(
                        is_last_element,
                        true_without_comma.as_pointer_value(),
                        true_with_comma.as_pointer_value(),
                        "select_true_format",
                    )
                    .unwrap()
                    .into_pointer_value();

                let false_format = self
                    .builder
                    .build_select(
                        is_last_element,
                        false_without_comma.as_pointer_value(),
                        false_with_comma.as_pointer_value(),
                        "select_false_format",
                    )
                    .unwrap()
                    .into_pointer_value();

                let final_format = self
                    .builder
                    .build_select(is_true, true_format, false_format, "select_bool_format")
                    .unwrap()
                    .into_pointer_value();

                self.builder
                    .build_call(printf_fn, &[final_format.into()], "")
                    .unwrap();
            } else if metadata.element_type == "Float" {
                let with_comma = self
                    .builder
                    .build_global_string_ptr("%f, ", "array_elem_fmt_comma_float")
                    .unwrap();
                let without_comma = self
                    .builder
                    .build_global_string_ptr("%f", "array_elem_fmt_no_comma_float")
                    .unwrap();

                let format_global = self
                    .builder
                    .build_select(
                        is_last_element,
                        without_comma.as_pointer_value(),
                        with_comma.as_pointer_value(),
                        "select_format_str_float",
                    )
                    .unwrap()
                    .into_pointer_value();

                self.builder
                    .build_call(printf_fn, &[format_global.into(), elem_val.into()], "")
                    .unwrap();
            } else {
                let with_comma = self
                    .builder
                    .build_global_string_ptr("%d, ", "array_elem_fmt_comma_int")
                    .unwrap();
                let without_comma = self
                    .builder
                    .build_global_string_ptr("%d", "array_elem_fmt_no_comma_int")
                    .unwrap();

                let format_global = self
                    .builder
                    .build_select(
                        is_last_element,
                        without_comma.as_pointer_value(),
                        with_comma.as_pointer_value(),
                        "select_format_str_int",
                    )
                    .unwrap()
                    .into_pointer_value();

                self.builder
                    .build_call(printf_fn, &[format_global.into(), elem_val.into()], "")
                    .unwrap();
            }

            // Increment counter and loop back
            self.builder
                .build_store(counter_alloca, next_counter)
                .unwrap();
            self.builder
                .build_unconditional_branch(loop_header)
                .unwrap();

            // Loop end - exit the loop
            self.builder.position_at_end(loop_end);
        }

        // Print closing bracket
        let close_bracket = self
            .builder
            .build_global_string_ptr("]", "close_bracket")
            .unwrap();
        self.builder
            .build_call(printf_fn, &[close_bracket.as_pointer_value().into()], "")
            .unwrap();
    }
}
