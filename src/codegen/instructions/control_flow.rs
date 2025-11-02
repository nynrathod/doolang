use crate::codegen::core::CodeGen;
impl<'ctx> CodeGen<'ctx> {
    pub fn generate_call(
        &mut self,
        dest: &[String],
        func: &str,
        args: &[String],
    ) -> Option<inkwell::values::BasicValueEnum<'ctx>> {
        let callee = self.module.get_function(func).expect(&format!(
            "Function '{}' not found. Make sure it's declared before calling.",
            func
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
                if self.functions_returning_heap.contains(func) {
                    if result.is_pointer_value() {
                        // Mark the result as heap-allocated based on return type
                        if let Some(return_type_str) = self.function_return_types.get(func) {
                            if return_type_str.contains("Str") || return_type_str.contains("String")
                            {
                                self.heap_strings.insert(dest_name.clone());
                            } else if return_type_str.contains("Array") {
                                self.heap_arrays.insert(dest_name.clone());
                            } else if return_type_str.contains("Map") {
                                self.heap_maps.insert(dest_name.clone());
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
            let value_base = value.trim_start_matches('%').trim_end_matches("_array");

            // Check if this value is a loop iteration variable (should NOT be treated as array/map)
            let is_loop_var = self.is_loop_var(value);

            // Check if this value is an array or map by looking at metadata
            // But NEVER treat loop iteration variables as arrays/maps
            let is_array = !is_loop_var
                && (self.array_metadata.contains_key(value) || self.heap_arrays.contains(value));
            let is_map = !is_loop_var
                && (self.map_metadata.contains_key(value) || self.heap_maps.contains(value));

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

        if let Some(metadata) = self.array_metadata.get(array_name) {
            let len_val = self
                .context
                .i32_type()
                .const_int(metadata.length as u64, false);
            self.temp_values.insert(name.to_string(), len_val.into());
            if let Some(sym) = self.symbols.get(name) {
                self.builder.build_store(sym.ptr, len_val).unwrap();
            }
            return Some(len_val.into());
        }

        if let Some(metadata) = self.map_metadata.get(array_name) {
            let len_val = self
                .context
                .i32_type()
                .const_int(metadata.length as u64, false);
            self.temp_values.insert(name.to_string(), len_val.into());
            if let Some(sym) = self.symbols.get(name) {
                self.builder.build_store(sym.ptr, len_val).unwrap();
            }
            return Some(len_val.into());
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
    fn is_boolean_value(&self, var_name: &str) -> bool {
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
                return name_lower.contains("is_")
                    || name_lower.contains("has_")
                    || name_lower.contains("can_")
                    || name_lower.contains("should_")
                    || name_lower.contains("valid")
                    || name_lower.contains("equal")
                    || name_lower.contains("greater")
                    || name_lower.contains("less");
            }
        }

        false
    }
}
