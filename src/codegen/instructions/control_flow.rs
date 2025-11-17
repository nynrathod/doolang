use crate::codegen::core::CodeGen;
impl<'ctx> CodeGen<'ctx> {
    pub fn generate_call(
        &mut self,
        dest: &[String],
        func: &str,
        args: &[String],
    ) -> Option<inkwell::values::BasicValueEnum<'ctx>> {
        match func {
            "println" => {
                self.generate_print(args);
                return None;
            }
            "panic" => {
                self.generate_panic(args);
                return None;
            }
            "typeOf" => {
                if !dest.is_empty() {
                    return self.generate_typeof(&dest[0], args);
                }
                return None;
            }
            _ => {}
        }

        // Resolve alias to actual function name if this is an aliased import
        let actual_func_name = self
            .function_aliases
            .get(func)
            .cloned()
            .unwrap_or_else(|| func.to_string());

        let callee = self.module.get_function(&actual_func_name).expect(&format!(
            "Function '{}' not found. Make sure it's declared before calling.",
            actual_func_name
        ));

        let arg_values: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = args
            .iter()
            .map(|arg| self.resolve_value(arg).into())
            .collect();

        let call_result = self
            .builder
            .build_call(callee, &arg_values, "call_result")
            .unwrap();

        if let Some(result) = call_result.try_as_basic_value().left() {
            if !dest.is_empty() {
                let dest_name = &dest[0];
                self.temp_values.insert(dest_name.clone(), result);

                // Check if this function is known to return heap-allocated values
                // Use actual_func_name in case func was an alias
                if self.functions_returning_heap.contains(&actual_func_name) {
                    if result.is_pointer_value() {
                        // Mark the result as heap-allocated based on return type
                        if let Some(return_type_str) =
                            self.function_return_types.get(&actual_func_name)
                        {
                            if return_type_str.contains("Array") {
                                self.heap_arrays.insert(dest_name.clone());

                                // Create array metadata for the returned array
                                // Extract element type from Array(Type) format
                                let element_type = CodeGen::extract_array_element_type_from_return(
                                    return_type_str,
                                );

                                let contains_strings = element_type == "Str";

                                self.array_metadata.insert(
                                    dest_name.clone(),
                                    crate::codegen::ArrayMetadata {
                                        length: 0, // Will be determined at runtime
                                        element_type: element_type.to_string(),
                                        contains_strings,
                                    },
                                );
                            } else if return_type_str.contains("Map") {
                                self.heap_maps.insert(dest_name.clone());

                                // Create map metadata for the returned map
                                // Extract key and value types from Map(Key,Value) format
                                let (key_type, value_type) =
                                    CodeGen::extract_map_types_from_return(return_type_str);

                                let key_is_string = key_type == "Str";
                                let value_is_string = value_type == "Str";
                                let key_needs_rc = key_is_string;
                                let value_needs_rc = value_is_string;

                                self.map_metadata.insert(
                                    dest_name.clone(),
                                    crate::codegen::MapMetadata {
                                        length: 0, // Will be determined at runtime
                                        key_type: key_type.to_string(),
                                        value_type: value_type.to_string(),
                                        key_is_string,
                                        value_is_string,
                                        key_needs_rc,
                                        value_needs_rc,
                                    },
                                );
                            } else if return_type_str.contains("Str")
                                || return_type_str.contains("String")
                            {
                                self.heap_strings.insert(dest_name.clone());
                            }
                        }
                    }
                }

                return Some(result);
            }
        }

        None
    }

    pub fn generate_print(&mut self, values: &[String]) {
        let printf_fn = self.get_or_declare_printf();

        for (idx, value) in values.iter().enumerate() {
            let base_name = value.trim_start_matches('%').trim_end_matches("_array");

            // Check if this value is a loop iteration variable (should NOT be treated as array/map)
            // Try multiple name variations to ensure we catch loop variables
            let is_loop_var = self.is_loop_var(value)
                || self.is_loop_var(base_name)
                || self.is_loop_var(&format!("{}_array", base_name))
                || self.is_loop_var(&value.trim_start_matches('%').to_string());

            // Check if this value is an array or map by looking at metadata
            // But NEVER treat loop iteration variables as arrays/maps
            // Also check name variations to handle returned arrays/maps
            let has_array_metadata = self.array_metadata.contains_key(value)
                || self
                    .array_metadata
                    .contains_key(&format!("{}_array", value))
                || self
                    .array_metadata
                    .contains_key(&value.trim_end_matches("_array").to_string())
                || self
                    .array_metadata
                    .contains_key(&value.trim_start_matches('%').to_string());

            let has_map_metadata = self.map_metadata.contains_key(value)
                || self.map_metadata.contains_key(&format!("{}_map", value))
                || self
                    .map_metadata
                    .contains_key(&value.trim_end_matches("_map").to_string())
                || self
                    .map_metadata
                    .contains_key(&value.trim_start_matches('%').to_string());

            // Only treat as array/map if it's actually a pointer value
            // Non-pointer values (like loop iteration variables) should never be treated as collections
            let resolved_val = self.resolve_value(value);
            let is_actually_pointer = resolved_val.is_pointer_value();

            let is_array = !is_loop_var
                && is_actually_pointer
                && (has_array_metadata || self.heap_arrays.contains(value));
            let is_map = !is_loop_var
                && is_actually_pointer
                && (has_map_metadata || self.heap_maps.contains(value));

            if is_array {
                self.print_array(value);
                if idx < values.len() - 1 {
                    let space_fmt = self
                        .builder
                        .build_global_string_ptr(" ", "space_fmt")
                        .unwrap();
                    self.builder
                        .build_call(
                            printf_fn,
                            &[space_fmt.as_pointer_value().into()],
                            "space_call",
                        )
                        .unwrap();
                }
            } else if is_map {
                self.print_map(value);
                if idx < values.len() - 1 {
                    let space_fmt = self
                        .builder
                        .build_global_string_ptr(" ", "space_fmt")
                        .unwrap();
                    self.builder
                        .build_call(
                            printf_fn,
                            &[space_fmt.as_pointer_value().into()],
                            "space_call",
                        )
                        .unwrap();
                }
            } else {
                let val = self.resolve_value(value);

                // Special handling for boolean values
                if self.is_boolean_value(value) {
                    // Use a simple approach to avoid crashes
                    let bool_val = self.resolve_value(value);
                    let int_val = bool_val.into_int_value();

                    // Check if value is 0 (false) or non-zero (true)
                    let zero = self.context.i32_type().const_int(0, false);
                    let is_false = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::EQ, int_val, zero, "is_false")
                        .unwrap();

                    // Use select to choose between "true" and "false" strings
                    let true_str = if idx < values.len() - 1 {
                        "true "
                    } else {
                        "true"
                    };
                    let false_str = if idx < values.len() - 1 {
                        "false "
                    } else {
                        "false"
                    };

                    let true_global = self
                        .builder
                        .build_global_string_ptr(true_str, "bool_true")
                        .unwrap();
                    let false_global = self
                        .builder
                        .build_global_string_ptr(false_str, "bool_false")
                        .unwrap();

                    // Use select instruction to choose the correct string
                    let selected_str = self
                        .builder
                        .build_select(
                            is_false,
                            false_global.as_pointer_value(),
                            true_global.as_pointer_value(),
                            "select_bool_str",
                        )
                        .unwrap()
                        .into_pointer_value();

                    // Print the selected string
                    self.builder
                        .build_call(printf_fn, &[selected_str.into()], "print_bool")
                        .unwrap();
                } else if val.is_int_value() {
                    let format_str = if idx < values.len() - 1 { "%d " } else { "%d" };
                    let format_global = self
                        .builder
                        .build_global_string_ptr(format_str, "print_fmt")
                        .unwrap();

                    self.builder
                        .build_call(
                            printf_fn,
                            &[format_global.as_pointer_value().into(), val.into()],
                            "print_call",
                        )
                        .unwrap();
                } else if val.is_float_value() {
                    let format_str = if idx < values.len() - 1 { "%f " } else { "%f" };
                    let format_global = self
                        .builder
                        .build_global_string_ptr(format_str, "print_fmt_float")
                        .unwrap();

                    self.builder
                        .build_call(
                            printf_fn,
                            &[format_global.as_pointer_value().into(), val.into()],
                            "print_float_call",
                        )
                        .unwrap();
                } else if val.is_pointer_value() {
                    let format_str = if idx < values.len() - 1 { "%s " } else { "%s" };
                    let format_global = self
                        .builder
                        .build_global_string_ptr(format_str, "print_fmt")
                        .unwrap();

                    self.builder
                        .build_call(
                            printf_fn,
                            &[format_global.as_pointer_value().into(), val.into()],
                            "print_call",
                        )
                        .unwrap();
                }
            }
        }

        let newline_fmt = self
            .builder
            .build_global_string_ptr("\n", "newline_fmt")
            .unwrap();
        self.builder
            .build_call(
                printf_fn,
                &[newline_fmt.as_pointer_value().into()],
                "newline_call",
            )
            .unwrap();
    }

    pub fn generate_array_len(
        &mut self,
        name: &str,
        array: &str,
    ) -> Option<inkwell::values::BasicValueEnum<'ctx>> {
        let array_name = array;

        // Try to find metadata with various name variations
        let name_variations = vec![
            array_name.to_string(),
            array_name.trim_start_matches('%').to_string(),
            array_name.trim_end_matches("_array").to_string(),
            format!("{}_array", array_name),
            format!("{}_array", array_name.trim_start_matches('%')),
        ];

        for variation in &name_variations {
            if let Some(_) = self.array_metadata.get(variation) {
                // Read length from heap at runtime (at offset 4 from data pointer)
                // This ensures sliced arrays show correct length
                let array_ptr = self.resolve_value(variation).into_pointer_value();

                // Get the heap pointer (8 bytes before data pointer)
                let heap_ptr = unsafe {
                    self.builder
                        .build_gep(
                            self.context.i8_type(),
                            array_ptr,
                            &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                            "heap_ptr_for_len",
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

                let len_val = self
                    .builder
                    .build_load(self.context.i32_type(), len_ptr_cast, "runtime_len")
                    .unwrap();

                self.temp_values.insert(name.to_string(), len_val);
                if let Some(sym) = self.symbols.get(name) {
                    self.builder.build_store(sym.ptr, len_val).unwrap();
                }
                return Some(len_val);
            }
        }

        if let Some(_metadata) = self.map_metadata.get(array_name) {
            // Read map length from heap at runtime
            let map_ptr = self.resolve_value(array).into_pointer_value();

            // Get the heap pointer (8 bytes before data pointer)
            let heap_ptr = unsafe {
                self.builder
                    .build_gep(
                        self.context.i8_type(),
                        map_ptr,
                        &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                        "map_heap_ptr_for_len",
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
                        "map_len_field_ptr",
                    )
                    .unwrap()
            };

            let len_ptr_cast = self
                .builder
                .build_pointer_cast(
                    len_field_ptr,
                    self.context.ptr_type(inkwell::AddressSpace::default()),
                    "map_len_ptr_cast",
                )
                .unwrap();

            let len_val = self
                .builder
                .build_load(self.context.i32_type(), len_ptr_cast, "map_runtime_len")
                .unwrap();

            self.temp_values.insert(name.to_string(), len_val);
            if let Some(sym) = self.symbols.get(name) {
                self.builder.build_store(sym.ptr, len_val).unwrap();
            }
            return Some(len_val);
        }

        let array_ptr_opt = if let Some(val) = self.temp_values.get(array_name) {
            if val.is_pointer_value() {
                Some(val.into_pointer_value())
            } else {
                None
            }
        } else if let Some(sym) = self.symbols.get(array_name) {
            if let Ok(loaded) =
                self.builder
                    .build_load(sym.ty, sym.ptr, &format!("{}_ptr", array_name))
            {
                if loaded.is_pointer_value() {
                    Some(loaded.into_pointer_value())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        if let Some(array_ptr) = array_ptr_opt {
            let len_ptr_result = unsafe {
                self.builder.build_in_bounds_gep(
                    self.context.i8_type(),
                    array_ptr,
                    &[self.context.i32_type().const_int((-4_i32) as u64, true)],
                    &format!("{}_len_ptr", array_name),
                )
            };

            if let Ok(len_ptr) = len_ptr_result {
                let len_ptr_cast_result = self.builder.build_pointer_cast(
                    len_ptr,
                    self.context.ptr_type(inkwell::AddressSpace::default()),
                    &format!("{}_len_cast", array_name),
                );

                if let Ok(len_ptr_cast) = len_ptr_cast_result {
                    if let Ok(runtime_len) = self.builder.build_load(
                        self.context.i32_type(),
                        len_ptr_cast,
                        &format!("{}_runtime_len", array_name),
                    ) {
                        let len_val = runtime_len.into_int_value();
                        self.temp_values.insert(name.to_string(), len_val.into());
                        if let Some(sym) = self.symbols.get(name) {
                            self.builder.build_store(sym.ptr, len_val).unwrap();
                        }
                        return Some(len_val.into());
                    }
                }
            }
        }

        let len_val = self.context.i32_type().const_int(0, false);
        self.temp_values.insert(name.to_string(), len_val.into());
        if let Some(sym) = self.symbols.get(name) {
            self.builder.build_store(sym.ptr, len_val).unwrap();
        }
        Some(len_val.into())
    }

    /// Check if a variable represents a boolean value (0 or 1)
    /// Helper function to check if a base name suggests a boolean value
    fn is_boolean_base_name(name: &str) -> bool {
        name.contains("is")
            || name.contains("has")
            || name.contains("can")
            || name.contains("should")
            || name.contains("valid")
            || name.contains("contains")
            || name.contains("startswith")
            || name.contains("endswith")
            || name.contains("starts")
            || name.contains("ends")
            || name.contains("empty")
            || name.contains("found")
            || name.contains("match")
            || name.contains("stringcontains")
            || name.contains("stringends")
            || name.contains("startswith")
            || name.contains("endswith")
            || name.contains("stringstartsw")
    }

    fn is_boolean_value(&self, var_name: &str) -> bool {
        // Check if this is in the boolean_temps set (marked from method calls)
        if self.boolean_temps.contains(var_name) {
            return true;
        }

        // Check variable_types map which tracks types from MIR declarations
        if let Some(var_type) = self.variable_types.get(var_name) {
            if var_type == "Bool" {
                return true;
            }
        }

        // Check if this is a comparison operation result (contains comparison keywords)
        if var_name.contains("equal") || var_name.contains("greater") || var_name.contains("less") {
            return true;
        }

        // Check if this is a boolean literal
        if var_name == "true" || var_name == "false" {
            return true;
        }

        // Check if this is a boolean variable in the symbol table with specific naming patterns
        if let Some(sym) = self.symbols.get(var_name) {
            if sym.ty.is_int_type() {
                // Additional check: variable names that suggest boolean usage
                let name_lower = var_name.to_lowercase();

                // Common boolean method/variable patterns
                if name_lower.contains("is_")
                    || name_lower.contains("has")
                    || name_lower.contains("can_")
                    || name_lower.contains("should_")
                    || name_lower.contains("valid")
                    || name_lower.contains("equal")
                    || name_lower.contains("greater")
                    || name_lower.contains("less")
                    || name_lower.contains("contains")
                    || name_lower.contains("startswith")
                    || name_lower.contains("endswith")
                    || name_lower.contains("starts")
                    || name_lower.contains("ends")
                    || name_lower.contains("found")
                    || name_lower.contains("match")
                    || name_lower.contains("empty")
                {
                    return true;
                }

                // Check for numbered boolean results (e.g., contains1, has30, isEmpty1)
                // Only if the base name (without trailing digits) contains boolean keywords
                if name_lower.chars().last().map_or(false, |c| c.is_numeric()) {
                    let base = name_lower.trim_end_matches(char::is_numeric);
                    if Self::is_boolean_base_name(base) {
                        return true;
                    }
                }
            }
        }

        false
    }

    pub fn generate_panic(&mut self, args: &[String]) {
        let printf_fn = self.get_or_declare_printf();

        if !args.is_empty() {
            let msg_val = self.resolve_value(&args[0]);
            let format_str = "panic: %s\n";
            let format_global = self
                .builder
                .build_global_string_ptr(format_str, "panic_fmt")
                .unwrap();

            self.builder
                .build_call(
                    printf_fn,
                    &[format_global.as_pointer_value().into(), msg_val.into()],
                    "panic_call",
                )
                .unwrap();
        } else {
            let format_str = "panic\n";
            let format_global = self
                .builder
                .build_global_string_ptr(format_str, "panic_fmt")
                .unwrap();

            self.builder
                .build_call(
                    printf_fn,
                    &[format_global.as_pointer_value().into()],
                    "panic_call",
                )
                .unwrap();
        }

        let exit_fn = self.module.get_function("exit").unwrap_or_else(|| {
            let fn_type = self
                .context
                .void_type()
                .fn_type(&[self.context.i32_type().into()], false);
            self.module.add_function("exit", fn_type, None)
        });

        self.builder
            .build_call(
                exit_fn,
                &[self.context.i32_type().const_int(1, false).into()],
                "",
            )
            .unwrap();

        self.builder.build_unreachable().unwrap();
    }

    pub fn generate_typeof(
        &mut self,
        dest: &str,
        args: &[String],
    ) -> Option<inkwell::values::BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }

        let value_name = &args[0];

        // First, check the variable_types map if it has been populated
        if let Some(stored_type) = self.variable_types.get(value_name) {
            let type_str_ptr = self
                .builder
                .build_global_string_ptr(stored_type, &format!("typeof_{}", dest))
                .unwrap();

            self.temp_values
                .insert(dest.to_string(), type_str_ptr.as_pointer_value().into());
            return Some(type_str_ptr.as_pointer_value().into());
        }

        // Check if it's a boolean temp
        if self.boolean_temps.contains(value_name) {
            let type_str = "Bool";
            let type_str_ptr = self
                .builder
                .build_global_string_ptr(type_str, &format!("typeof_{}", dest))
                .unwrap();

            self.temp_values
                .insert(dest.to_string(), type_str_ptr.as_pointer_value().into());
            return Some(type_str_ptr.as_pointer_value().into());
        }

        // Check heap allocations
        let type_str = if self.heap_strings.contains(value_name)
            || self.temp_strings.contains_key(value_name)
        {
            "Str"
        } else if self.heap_arrays.contains(value_name)
            || self.array_metadata.contains_key(value_name)
        {
            "Array"
        } else if self.heap_maps.contains(value_name) || self.map_metadata.contains_key(value_name)
        {
            "Map"
        } else if let Some(sym) = self.symbols.get(value_name) {
            // Check if it's a user variable in the symbol table
            let loaded_val = self
                .builder
                .build_load(sym.ty, sym.ptr, &format!("load_for_typeof_{}", value_name))
                .ok();

            if let Some(val) = loaded_val {
                if val.is_int_value() {
                    // Check the bit width to differentiate Bool from Int
                    let int_val = val.into_int_value();
                    if int_val.get_type().get_bit_width() == 1 {
                        "Bool"
                    } else {
                        "Int"
                    }
                } else if val.is_float_value() {
                    "Float"
                } else if val.is_pointer_value() {
                    "Str"
                } else {
                    "Unknown"
                }
            } else {
                "Unknown"
            }
        } else if let Some(val) = self.temp_values.get(value_name) {
            if val.is_int_value() {
                // Check the bit width to differentiate Bool from Int
                let int_val = val.into_int_value();
                if int_val.get_type().get_bit_width() == 1 {
                    "Bool"
                } else {
                    "Int"
                }
            } else if val.is_float_value() {
                "Float"
            } else if val.is_pointer_value() {
                "Str"
            } else {
                "Unknown"
            }
        } else {
            "Unknown"
        };

        let type_str_ptr = self
            .builder
            .build_global_string_ptr(type_str, &format!("typeof_{}", dest))
            .unwrap();

        self.temp_values
            .insert(dest.to_string(), type_str_ptr.as_pointer_value().into());
        Some(type_str_ptr.as_pointer_value().into())
    }
}
