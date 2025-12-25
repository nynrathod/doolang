use crate::codegen::core::CodeGen;
use inkwell::values::BasicValue;
use inkwell::values::BasicValueEnum;

impl<'ctx> CodeGen<'ctx> {
    pub fn generate_json_method(
        &mut self,
        dest: &str,
        method: &str,
        args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        match method {
            "parse" => self.generate_json_parse(dest, args),
            "stringify" => self.generate_json_stringify(dest, args),
            _ => None,
        }
    }

    /// Helper method for parsing JSON with explicit pointer and target type
    /// Used for automatic db.raw() result parsing
    pub fn generate_json_parse_typed(
        &mut self,
        dest: &str,
        json_str_ptr: inkwell::values::PointerValue<'ctx>,
        target_type: &str,
    ) -> Option<BasicValueEnum<'ctx>> {
        // Store the JSON string pointer temporarily
        let temp_json_name = format!("{}_json_temp", dest);
        self.temp_values
            .insert(temp_json_name.clone(), json_str_ptr.into());
        self.heap_strings.insert(temp_json_name.clone());

        // Call the existing generate_json_parse with the temp name
        let args = vec![temp_json_name, target_type.to_string()];
        self.generate_json_parse(dest, &args)
    }

    fn generate_json_parse(&mut self, dest: &str, args: &[String]) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }

        // Get the JSON string argument
        let json_str_val = self.resolve_value(&args[0]);
        let json_str_ptr = json_str_val.into_pointer_value();

        // Declare/get json_parse function from runtime
        let json_parse_fn = self.module.get_function("json_parse").unwrap_or_else(|| {
            let fn_type = self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .fn_type(
                    &[self
                        .context
                        .ptr_type(inkwell::AddressSpace::default())
                        .into()],
                    false,
                );
            self.module.add_function("json_parse", fn_type, None)
        });

        // Call json_parse(json_str)
        let result_ptr = self
            .builder
            .build_call(json_parse_fn, &[json_str_ptr.into()], "json_parse_result")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // The result is a heap-allocated string with RC header
        // Allocate with RC header: [RC: 4 bytes][Length: 4 bytes][data]
        let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
            let fn_type = self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .fn_type(&[self.context.i64_type().into()], false);
            self.module.add_function("malloc", fn_type, None)
        });

        // Get strlen to find the length of returned string
        let strlen_fn = self.module.get_function("strlen").unwrap_or_else(|| {
            let fn_type = self.context.i64_type().fn_type(
                &[self
                    .context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into()],
                false,
            );
            self.module.add_function("strlen", fn_type, None)
        });

        // Get string length
        let str_len = self
            .builder
            .build_call(strlen_fn, &[result_ptr.into()], "parsed_len")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        // Allocate: 8 bytes header + string length + 1 for null terminator
        let header_size = self.context.i64_type().const_int(8, false);
        let data_size = self
            .builder
            .build_int_add(
                str_len,
                self.context.i64_type().const_int(1, false),
                "data_size",
            )
            .unwrap();
        let total_size = self
            .builder
            .build_int_add(header_size, data_size, "total_size")
            .unwrap();

        let heap_ptr = self
            .builder
            .build_call(malloc_fn, &[total_size.into()], "heap_json")
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
                self.context.ptr_type(inkwell::AddressSpace::default()),
                "rc_ptr",
            )
            .unwrap();
        self.builder
            .build_store(rc_ptr, self.context.i32_type().const_int(1, false))
            .unwrap();

        // Store string length at offset 4
        let str_len_i32 = self
            .builder
            .build_int_truncate(str_len, self.context.i32_type(), "len_i32")
            .unwrap();
        let len_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i32_type(),
                    heap_ptr,
                    &[self.context.i32_type().const_int(1, false)],
                    "len_ptr",
                )
                .unwrap()
        };
        self.builder.build_store(len_ptr, str_len_i32).unwrap();

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

        // Copy string content using memcpy
        let memcpy_fn = self
            .module
            .get_function("llvm.memcpy.p0.p0.i64")
            .unwrap_or_else(|| {
                let fn_type = self.context.void_type().fn_type(
                    &[
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into(),
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into(),
                        self.context.i64_type().into(),
                        self.context.bool_type().into(),
                    ],
                    false,
                );
                self.module
                    .add_function("llvm.memcpy.p0.p0.i64", fn_type, None)
            });

        self.builder
            .build_call(
                memcpy_fn,
                &[
                    data_ptr.into(),
                    result_ptr.into(),
                    data_size.into(),
                    self.context.bool_type().const_zero().into(),
                ],
                "",
            )
            .unwrap();

        // Free the original returned pointer from json_parse
        let free_fn = self.module.get_function("free").unwrap_or_else(|| {
            let fn_type = self.context.void_type().fn_type(
                &[self
                    .context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into()],
                false,
            );
            self.module.add_function("free", fn_type, None)
        });

        self.builder
            .build_call(free_fn, &[result_ptr.into()], "")
            .unwrap();

        // Store result and mark as heap string
        self.temp_values.insert(dest.to_string(), data_ptr.into());
        self.heap_strings.insert(dest.to_string());

        // CRITICAL: If there's a pre-allocated symbol for this dest, store the result there too
        // This ensures resolve_value can find the value when loading from the symbol
        if let Some(sym) = self.symbols.get(dest) {
            self.builder.build_store(sym.ptr, data_ptr).unwrap();
        }

        Some(data_ptr.into())
    }

    fn generate_json_stringify(
        &mut self,
        dest: &str,
        args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }

        let arg_name = &args[0];

        // Check if it's an array
        if let Some(array_meta) = self.array_metadata.get(arg_name).cloned() {
            return self.stringify_array(dest, arg_name, &array_meta);
        }

        // Check if it's a map
        if let Some(map_meta) = self.map_metadata.get(arg_name).cloned() {
            return self.stringify_map(dest, arg_name, &map_meta);
        }

        // Check if it's a struct
        if let Some(struct_name) = self.struct_instance_types.get(arg_name).cloned() {
            return self.stringify_struct(dest, arg_name, &struct_name);
        }

        // Also check variable_types for struct types
        if let Some(var_type) = self.variable_types.get(arg_name).cloned() {
            let struct_name = if var_type.starts_with("Struct(") && var_type.ends_with(")") {
                Some(var_type[7..var_type.len() - 1].to_string())
            } else if self.struct_metadata.contains_key(&var_type) {
                Some(var_type.clone())
            } else {
                None
            };

            if let Some(name) = struct_name {
                return self.stringify_struct(dest, arg_name, &name);
            }
        }

        // Otherwise, handle as primitive
        self.stringify_primitive(dest, arg_name)
    }

    fn stringify_struct(
        &mut self,
        dest: &str,
        struct_var: &str,
        struct_name: &str,
    ) -> Option<BasicValueEnum<'ctx>> {
        let metadata = self.struct_metadata.get(struct_name).cloned()?;
        let canonical_type = self.canonical_struct_types.get(struct_name).cloned()?;

        let struct_ptr = self.resolve_value(struct_var).into_pointer_value();

        let malloc_fn = self.get_malloc_fn();
        let sprintf_fn = self.get_sprintf_fn();
        let strcpy_fn = self.get_strcpy_fn();
        let strcat_fn = self.get_strcat_fn();
        let strlen_fn = self.get_strlen_fn();

        // Allocate buffer (1024 bytes for JSON)
        let buffer = self
            .builder
            .build_call(
                malloc_fn,
                &[self.context.i64_type().const_int(1024, false).into()],
                "json_buffer",
            )
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Start with "{"
        let open_brace = self
            .builder
            .build_global_string_ptr("{", "open_brace")
            .unwrap();
        self.builder
            .build_call(
                strcpy_fn,
                &[buffer.into(), open_brace.as_pointer_value().into()],
                "",
            )
            .unwrap();

        // Process each field
        for (field_idx, (field_name, field_type)) in metadata
            .field_names
            .iter()
            .zip(metadata.field_types.iter())
            .enumerate()
        {
            // Add comma if not first field
            if field_idx > 0 {
                let comma = self.builder.build_global_string_ptr(", ", "comma").unwrap();
                self.builder
                    .build_call(
                        strcat_fn,
                        &[buffer.into(), comma.as_pointer_value().into()],
                        "",
                    )
                    .unwrap();
            }

            // Add field name with quotes and colon
            let field_prefix = format!("\"{}\": ", field_name);
            let field_prefix_str = self
                .builder
                .build_global_string_ptr(&field_prefix, &format!("field_prefix_{}", field_idx))
                .unwrap();
            self.builder
                .build_call(
                    strcat_fn,
                    &[buffer.into(), field_prefix_str.as_pointer_value().into()],
                    "",
                )
                .unwrap();

            // Get field pointer
            let field_ptr = self
                .builder
                .build_struct_gep(
                    canonical_type,
                    struct_ptr,
                    field_idx as u32,
                    &format!("field_ptr_{}", field_name),
                )
                .unwrap();

            // Allocate temp buffer for field value
            let temp_buffer = self
                .builder
                .build_call(
                    malloc_fn,
                    &[self.context.i64_type().const_int(256, false).into()],
                    "temp_buffer",
                )
                .unwrap()
                .try_as_basic_value()
                .left()
                .unwrap()
                .into_pointer_value();

            match field_type.as_str() {
                "Int" => {
                    let val = self
                        .builder
                        .build_load(self.context.i32_type(), field_ptr, "field_int")
                        .unwrap()
                        .into_int_value();
                    let fmt = self
                        .builder
                        .build_global_string_ptr("%d", "int_fmt")
                        .unwrap();
                    self.builder
                        .build_call(
                            sprintf_fn,
                            &[
                                temp_buffer.into(),
                                fmt.as_pointer_value().into(),
                                val.into(),
                            ],
                            "",
                        )
                        .unwrap();
                }
                "Float" => {
                    let val = self
                        .builder
                        .build_load(self.context.f64_type(), field_ptr, "field_float")
                        .unwrap()
                        .into_float_value();
                    let fmt = self
                        .builder
                        .build_global_string_ptr("%.15g", "float_fmt")
                        .unwrap();
                    self.builder
                        .build_call(
                            sprintf_fn,
                            &[
                                temp_buffer.into(),
                                fmt.as_pointer_value().into(),
                                val.into(),
                            ],
                            "",
                        )
                        .unwrap();
                }
                "Bool" => {
                    let val = self
                        .builder
                        .build_load(self.context.i32_type(), field_ptr, "field_bool")
                        .unwrap()
                        .into_int_value();
                    let true_str = self
                        .builder
                        .build_global_string_ptr("true", "true_str")
                        .unwrap();
                    let false_str = self
                        .builder
                        .build_global_string_ptr("false", "false_str")
                        .unwrap();
                    let is_true = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::NE,
                            val,
                            self.context.i32_type().const_zero(),
                            "is_true",
                        )
                        .unwrap();
                    let selected = self
                        .builder
                        .build_select(
                            is_true,
                            true_str.as_pointer_value(),
                            false_str.as_pointer_value(),
                            "bool_str",
                        )
                        .unwrap()
                        .into_pointer_value();
                    self.builder
                        .build_call(strcpy_fn, &[temp_buffer.into(), selected.into()], "")
                        .unwrap();
                }
                "Str" => {
                    let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                    let str_ptr = self
                        .builder
                        .build_load(ptr_type, field_ptr, "field_str")
                        .unwrap()
                        .into_pointer_value();
                    // Format as JSON string with quotes
                    let quote = self.context.i8_type().const_int('"' as u64, false);
                    self.builder.build_store(temp_buffer, quote).unwrap();
                    let dest_after = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                temp_buffer,
                                &[self.context.i64_type().const_int(1, false)],
                                "dest_after",
                            )
                            .unwrap()
                    };
                    self.builder
                        .build_call(strcpy_fn, &[dest_after.into(), str_ptr.into()], "")
                        .unwrap();
                    let len = self
                        .builder
                        .build_call(strlen_fn, &[str_ptr.into()], "str_len")
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_int_value();
                    let close_pos = unsafe {
                        self.builder
                            .build_gep(self.context.i8_type(), dest_after, &[len], "close_pos")
                            .unwrap()
                    };
                    self.builder.build_store(close_pos, quote).unwrap();
                    let null_pos = unsafe {
                        self.builder
                            .build_gep(
                                self.context.i8_type(),
                                close_pos,
                                &[self.context.i64_type().const_int(1, false)],
                                "null_pos",
                            )
                            .unwrap()
                    };
                    self.builder
                        .build_store(null_pos, self.context.i8_type().const_zero())
                        .unwrap();
                }
                _ => {
                    // For complex types, output "null"
                    let null_str = self
                        .builder
                        .build_global_string_ptr("null", "null_str")
                        .unwrap();
                    self.builder
                        .build_call(
                            strcpy_fn,
                            &[temp_buffer.into(), null_str.as_pointer_value().into()],
                            "",
                        )
                        .unwrap();
                }
            }

            // Append field value to buffer
            self.builder
                .build_call(strcat_fn, &[buffer.into(), temp_buffer.into()], "")
                .unwrap();

            // Free temp buffer
            let free_fn = self.get_free_fn();
            self.builder
                .build_call(free_fn, &[temp_buffer.into()], "")
                .unwrap();
        }

        // Close with "}"
        let close_brace = self
            .builder
            .build_global_string_ptr("}", "close_brace")
            .unwrap();
        self.builder
            .build_call(
                strcat_fn,
                &[buffer.into(), close_brace.as_pointer_value().into()],
                "",
            )
            .unwrap();

        self.wrap_as_heap_string(dest, buffer)
    }

    fn stringify_primitive(&mut self, dest: &str, arg_name: &str) -> Option<BasicValueEnum<'ctx>> {
        let data_val = self.resolve_value(arg_name);

        let malloc_fn = self.get_malloc_fn();
        let sprintf_fn = self.get_sprintf_fn();

        // Allocate buffer (256 bytes should be enough for any primitive)
        let buffer = self
            .builder
            .build_call(
                malloc_fn,
                &[self.context.i64_type().const_int(256, false).into()],
                "json_buffer",
            )
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Check if this variable is a boolean using metadata
        let is_bool = self
            .variable_types
            .get(arg_name)
            .map(|t| t == "Bool")
            .unwrap_or(false);

        // Check if this variable is an enum
        let var_type = self.variable_types.get(arg_name).cloned();
        let is_enum = var_type
            .as_ref()
            .map(|t| t.starts_with("Enum(") || self.enum_table.contains_key(t))
            .unwrap_or(false);

        // Handle enum values specially
        if is_enum && data_val.is_struct_value() {
            let enum_struct = data_val.into_struct_value();

            // Extract tag (field 0) to determine variant
            let tag = self
                .builder
                .build_extract_value(enum_struct, 0, "enum_tag")
                .unwrap()
                .into_int_value();

            // Extract payload pointer (field 1)
            let payload_ptr = self
                .builder
                .build_extract_value(enum_struct, 1, "enum_payload_ptr")
                .unwrap()
                .into_pointer_value();

            // Get enum name from variable type
            let enum_name = var_type
                .as_ref()
                .map(|t| {
                    if t.starts_with("Enum(") && t.ends_with(")") {
                        &t[5..t.len() - 1]
                    } else {
                        t.as_str()
                    }
                })
                .unwrap_or("Unknown");

            // Get variants for this enum - use enum_variant_order for correct declaration order
            if let Some(variants) = self.enum_variant_order.get(enum_name).cloned() {
                let strcpy_fn = self.get_strcpy_fn();
                let strcat_fn = self.get_strcat_fn();

                // Build switch to select variant name
                let current_fn = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let merge_block = self
                    .context
                    .append_basic_block(current_fn, "enum_stringify_merge");

                let mut cases: Vec<(inkwell::values::IntValue, inkwell::basic_block::BasicBlock)> =
                    Vec::new();

                for (idx, (variant_name, _)) in variants.iter().enumerate() {
                    let case_block = self
                        .context
                        .append_basic_block(current_fn, &format!("enum_case_{}", variant_name));
                    cases.push((
                        self.context.i32_type().const_int(idx as u64, false),
                        case_block,
                    ));
                }

                let default_block = self.context.append_basic_block(current_fn, "enum_default");

                self.builder
                    .build_switch(tag, default_block, &cases)
                    .unwrap();

                // Generate code for each case
                for (idx, (variant_name, payload_type_opt)) in variants.iter().enumerate() {
                    self.builder.position_at_end(cases[idx].1);

                    // Check if this variant has a payload
                    if let Some(payload_type) = payload_type_opt {
                        // Format as {"VariantName":payload}
                        let prefix = format!("{{\"{}\":", variant_name);
                        let prefix_str = self
                            .builder
                            .build_global_string_ptr(&prefix, &format!("variant_prefix_{}", idx))
                            .unwrap();
                        self.builder
                            .build_call(
                                strcpy_fn,
                                &[buffer.into(), prefix_str.as_pointer_value().into()],
                                "",
                            )
                            .unwrap();

                        // Format payload based on type
                        let type_str = format!("{:?}", payload_type);
                        if type_str.contains("String") || type_str.contains("Str") {
                            // String payload - payload_ptr IS the string pointer
                            // Format as "value"
                            let quote_str = self
                                .builder
                                .build_global_string_ptr("\"", "quote_str")
                                .unwrap();
                            self.builder
                                .build_call(
                                    strcat_fn,
                                    &[buffer.into(), quote_str.as_pointer_value().into()],
                                    "",
                                )
                                .unwrap();
                            self.builder
                                .build_call(strcat_fn, &[buffer.into(), payload_ptr.into()], "")
                                .unwrap();
                            self.builder
                                .build_call(
                                    strcat_fn,
                                    &[buffer.into(), quote_str.as_pointer_value().into()],
                                    "",
                                )
                                .unwrap();
                        } else if type_str.contains("Float") {
                            // Float payload - load from pointer and format
                            let payload_val = self
                                .builder
                                .build_load(self.context.f64_type(), payload_ptr, "payload_float")
                                .unwrap();
                            let float_fmt = self
                                .builder
                                .build_global_string_ptr("%.15g", "float_fmt_payload")
                                .unwrap();
                            // Use a temp buffer for sprintf
                            let temp_buf = unsafe {
                                self.builder
                                    .build_gep(
                                        self.context.i8_type(),
                                        buffer,
                                        &[self.context.i64_type().const_int(128, false)],
                                        "temp_buf",
                                    )
                                    .unwrap()
                            };
                            self.builder
                                .build_call(
                                    sprintf_fn,
                                    &[
                                        temp_buf.into(),
                                        float_fmt.as_pointer_value().into(),
                                        payload_val.into(),
                                    ],
                                    "",
                                )
                                .unwrap();
                            self.builder
                                .build_call(strcat_fn, &[buffer.into(), temp_buf.into()], "")
                                .unwrap();
                        } else {
                            // Int or other primitive - load from pointer and format as %d
                            let payload_val = self
                                .builder
                                .build_load(self.context.i32_type(), payload_ptr, "payload_int")
                                .unwrap();
                            let int_fmt = self
                                .builder
                                .build_global_string_ptr("%d", "int_fmt_payload")
                                .unwrap();
                            // Use a temp buffer for sprintf
                            let temp_buf = unsafe {
                                self.builder
                                    .build_gep(
                                        self.context.i8_type(),
                                        buffer,
                                        &[self.context.i64_type().const_int(128, false)],
                                        "temp_buf",
                                    )
                                    .unwrap()
                            };
                            self.builder
                                .build_call(
                                    sprintf_fn,
                                    &[
                                        temp_buf.into(),
                                        int_fmt.as_pointer_value().into(),
                                        payload_val.into(),
                                    ],
                                    "",
                                )
                                .unwrap();
                            self.builder
                                .build_call(strcat_fn, &[buffer.into(), temp_buf.into()], "")
                                .unwrap();
                        }

                        // Close the JSON object
                        let suffix_str = self
                            .builder
                            .build_global_string_ptr("}", "variant_suffix")
                            .unwrap();
                        self.builder
                            .build_call(
                                strcat_fn,
                                &[buffer.into(), suffix_str.as_pointer_value().into()],
                                "",
                            )
                            .unwrap();
                    } else {
                        // Unit variant - format as "\"VariantName\""
                        let variant_json = format!("\"{}\"", variant_name);
                        let variant_str = self
                            .builder
                            .build_global_string_ptr(
                                &variant_json,
                                &format!("variant_json_{}", idx),
                            )
                            .unwrap();
                        self.builder
                            .build_call(
                                strcpy_fn,
                                &[buffer.into(), variant_str.as_pointer_value().into()],
                                "",
                            )
                            .unwrap();
                    }
                    self.builder
                        .build_unconditional_branch(merge_block)
                        .unwrap();
                }

                // Default case: output "null"
                self.builder.position_at_end(default_block);
                let null_str = self
                    .builder
                    .build_global_string_ptr("null", "null_json")
                    .unwrap();
                self.builder
                    .build_call(
                        strcpy_fn,
                        &[buffer.into(), null_str.as_pointer_value().into()],
                        "",
                    )
                    .unwrap();
                self.builder
                    .build_unconditional_branch(merge_block)
                    .unwrap();

                self.builder.position_at_end(merge_block);
                return self.wrap_as_heap_string(dest, buffer);
            }
        }

        if data_val.is_int_value() {
            let int_val = data_val.into_int_value();

            if is_bool {
                // Boolean - output "true" or "false"
                let true_str = self
                    .builder
                    .build_global_string_ptr("true", "true_str")
                    .unwrap();
                let false_str = self
                    .builder
                    .build_global_string_ptr("false", "false_str")
                    .unwrap();

                let is_true = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        int_val,
                        int_val.get_type().const_zero(),
                        "is_true",
                    )
                    .unwrap();

                let selected_str = self
                    .builder
                    .build_select(
                        is_true,
                        true_str.as_pointer_value(),
                        false_str.as_pointer_value(),
                        "bool_str",
                    )
                    .unwrap()
                    .into_pointer_value();

                let strcpy_fn = self.get_strcpy_fn();
                self.builder
                    .build_call(strcpy_fn, &[buffer.into(), selected_str.into()], "")
                    .unwrap();
            } else {
                // Integer - format as number
                let format_str = self
                    .builder
                    .build_global_string_ptr("%d", "int_fmt")
                    .unwrap();
                self.builder
                    .build_call(
                        sprintf_fn,
                        &[
                            buffer.into(),
                            format_str.as_pointer_value().into(),
                            int_val.into(),
                        ],
                        "",
                    )
                    .unwrap();
            }
        } else if data_val.is_float_value() {
            // Float - format as number
            let float_val = data_val.into_float_value();
            let format_str = self
                .builder
                .build_global_string_ptr("%.15g", "float_fmt")
                .unwrap();
            self.builder
                .build_call(
                    sprintf_fn,
                    &[
                        buffer.into(),
                        format_str.as_pointer_value().into(),
                        float_val.into(),
                    ],
                    "",
                )
                .unwrap();
        } else if data_val.is_pointer_value() {
            // String - format as JSON string with quotes and escaping
            let str_ptr = data_val.into_pointer_value();

            // Open quote
            let quote_char = self.context.i8_type().const_int('"' as u64, false);
            self.builder.build_store(buffer, quote_char).unwrap();

            // Get strlen
            let strlen_fn = self.get_strlen_fn();
            let str_len = self
                .builder
                .build_call(strlen_fn, &[str_ptr.into()], "str_len")
                .unwrap()
                .try_as_basic_value()
                .left()
                .unwrap()
                .into_int_value();

            // Copy string content (simplified - no escaping for now)
            let dest_ptr = unsafe {
                self.builder
                    .build_gep(
                        self.context.i8_type(),
                        buffer,
                        &[self.context.i64_type().const_int(1, false)],
                        "dest_after_quote",
                    )
                    .unwrap()
            };

            let strcpy_fn = self.get_strcpy_fn();
            self.builder
                .build_call(strcpy_fn, &[dest_ptr.into(), str_ptr.into()], "")
                .unwrap();

            // Close quote
            let quote_pos = unsafe {
                self.builder
                    .build_gep(
                        self.context.i8_type(),
                        dest_ptr,
                        &[str_len],
                        "close_quote_pos",
                    )
                    .unwrap()
            };
            self.builder.build_store(quote_pos, quote_char).unwrap();

            // Null terminator
            let null_pos = unsafe {
                self.builder
                    .build_gep(
                        self.context.i8_type(),
                        quote_pos,
                        &[self.context.i64_type().const_int(1, false)],
                        "null_pos",
                    )
                    .unwrap()
            };
            self.builder
                .build_store(null_pos, self.context.i8_type().const_zero())
                .unwrap();
        }

        self.wrap_as_heap_string(dest, buffer)
    }

    fn stringify_array(
        &mut self,
        dest: &str,
        arr_name: &str,
        meta: &crate::codegen::core::context::ArrayMetadata,
    ) -> Option<BasicValueEnum<'ctx>> {
        let malloc_fn = self.get_malloc_fn();
        let sprintf_fn = self.get_sprintf_fn();
        let strcat_fn = self.get_strcat_fn();

        // Allocate large buffer for JSON array
        let buffer_size = self.context.i64_type().const_int(65536, false); // 64KB
        let buffer = self
            .builder
            .build_call(malloc_fn, &[buffer_size.into()], "json_arr_buffer")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Start with "["
        let open_bracket = self
            .builder
            .build_global_string_ptr("[", "open_bracket")
            .unwrap();
        let strcpy_fn = self.get_strcpy_fn();
        self.builder
            .build_call(
                strcpy_fn,
                &[buffer.into(), open_bracket.as_pointer_value().into()],
                "",
            )
            .unwrap();

        // Get array pointer
        let arr_val = self.resolve_value(arr_name);
        let arr_ptr = arr_val.into_pointer_value();

        // Iterate through array elements
        let element_type = &meta.element_type;
        let arr_len = meta.length;

        for i in 0..arr_len {
            if i > 0 {
                // Add comma separator
                let comma = self.builder.build_global_string_ptr(",", "comma").unwrap();
                self.builder
                    .build_call(
                        strcat_fn,
                        &[buffer.into(), comma.as_pointer_value().into()],
                        "",
                    )
                    .unwrap();
            }

            // Get element at index i
            let idx = self.context.i64_type().const_int(i as u64, false);
            let elem_ptr = unsafe {
                self.builder
                    .build_gep(
                        self.get_llvm_type_from_string(element_type),
                        arr_ptr,
                        &[idx],
                        &format!("elem_{}", i),
                    )
                    .unwrap()
            };

            let elem_val = self
                .builder
                .build_load(
                    self.get_llvm_type_from_string(element_type),
                    elem_ptr,
                    &format!("elem_val_{}", i),
                )
                .unwrap();

            // Format element based on type
            self.append_value_to_buffer(buffer, elem_val, element_type, sprintf_fn, strcat_fn);
        }

        // Close with "]"
        let close_bracket = self
            .builder
            .build_global_string_ptr("]", "close_bracket")
            .unwrap();
        self.builder
            .build_call(
                strcat_fn,
                &[buffer.into(), close_bracket.as_pointer_value().into()],
                "",
            )
            .unwrap();

        self.wrap_as_heap_string(dest, buffer)
    }

    fn stringify_map(
        &mut self,
        dest: &str,
        map_name: &str,
        meta: &crate::codegen::core::context::MapMetadata,
    ) -> Option<BasicValueEnum<'ctx>> {
        let malloc_fn = self.get_malloc_fn();
        let sprintf_fn = self.get_sprintf_fn();
        let strcat_fn = self.get_strcat_fn();
        let strcpy_fn = self.get_strcpy_fn();

        // Allocate large buffer for JSON object
        let buffer_size = self.context.i64_type().const_int(65536, false); // 64KB
        let buffer = self
            .builder
            .build_call(malloc_fn, &[buffer_size.into()], "json_obj_buffer")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Start with "{"
        let open_brace = self
            .builder
            .build_global_string_ptr("{", "open_brace")
            .unwrap();
        self.builder
            .build_call(
                strcpy_fn,
                &[buffer.into(), open_brace.as_pointer_value().into()],
                "",
            )
            .unwrap();

        // Get map pointer
        let map_val = self.resolve_value(map_name);
        let map_ptr = map_val.into_pointer_value();

        // Determine if this is a heap-allocated map
        let is_heap_allocated = self.heap_maps.contains(map_name);

        // Determine key and value LLVM types
        let key_type: inkwell::types::BasicTypeEnum = if meta.key_is_string {
            self.context
                .ptr_type(inkwell::AddressSpace::default())
                .into()
        } else if meta.key_type == "Float" {
            self.context.f64_type().into()
        } else if meta.key_type == "Bool" {
            self.context.bool_type().into()
        } else {
            self.context.i32_type().into()
        };

        let val_type: inkwell::types::BasicTypeEnum = if meta.value_is_string {
            self.context
                .ptr_type(inkwell::AddressSpace::default())
                .into()
        } else if meta.value_type == "Float" {
            self.context.f64_type().into()
        } else if meta.value_type == "Bool" {
            self.context.bool_type().into()
        } else {
            self.context.i32_type().into()
        };

        // Create struct type for key-value pair
        let pair_type = self.context.struct_type(&[key_type, val_type], false);

        // Get runtime length
        let length_val = if meta.length == 0 || is_heap_allocated {
            // Read length from RC header (8 bytes before data pointer)
            let rc_header_ptr = unsafe {
                self.builder
                    .build_gep(
                        self.context.i8_type(),
                        map_ptr,
                        &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                        "rc_header_ptr",
                    )
                    .unwrap()
            };

            let len_ptr = unsafe {
                self.builder
                    .build_gep(
                        self.context.i8_type(),
                        rc_header_ptr,
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
                .build_load(self.context.i32_type(), len_ptr_cast, "runtime_len")
                .unwrap()
                .into_int_value()
        } else {
            self.context.i32_type().const_int(meta.length as u64, false)
        };

        // Create loop to iterate through pairs
        let current_fn = self
            .builder
            .get_insert_block()
            .unwrap()
            .get_parent()
            .unwrap();
        let loop_block = self
            .context
            .append_basic_block(current_fn, "map_stringify_loop");
        let body_block = self
            .context
            .append_basic_block(current_fn, "map_stringify_body");
        let after_block = self
            .context
            .append_basic_block(current_fn, "after_map_stringify");

        // Allocate counter
        let counter_ptr = self
            .builder
            .build_alloca(self.context.i32_type(), "map_counter")
            .unwrap();
        self.builder
            .build_store(counter_ptr, self.context.i32_type().const_zero())
            .unwrap();

        self.builder.build_unconditional_branch(loop_block).unwrap();
        self.builder.position_at_end(loop_block);

        // Load counter and check if done
        let counter = self
            .builder
            .build_load(self.context.i32_type(), counter_ptr, "counter_val")
            .unwrap()
            .into_int_value();

        let is_done = self
            .builder
            .build_int_compare(inkwell::IntPredicate::UGE, counter, length_val, "is_done")
            .unwrap();

        self.builder
            .build_conditional_branch(is_done, after_block, body_block)
            .unwrap();

        self.builder.position_at_end(body_block);

        // Add comma separator if not first element
        let is_first = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                counter,
                self.context.i32_type().const_zero(),
                "is_first",
            )
            .unwrap();

        let comma_block = self.context.append_basic_block(current_fn, "add_comma");
        let continue_block = self
            .context
            .append_basic_block(current_fn, "continue_stringify");

        self.builder
            .build_conditional_branch(is_first, continue_block, comma_block)
            .unwrap();

        self.builder.position_at_end(comma_block);
        let comma = self.builder.build_global_string_ptr(",", "comma").unwrap();
        self.builder
            .build_call(
                strcat_fn,
                &[buffer.into(), comma.as_pointer_value().into()],
                "",
            )
            .unwrap();
        self.builder
            .build_unconditional_branch(continue_block)
            .unwrap();

        self.builder.position_at_end(continue_block);

        // Get pair pointer (maps are stored as arrays of pair structs)
        let pair_ptr = if !is_heap_allocated {
            // Global constant - use typed GEP
            let map_array_type = pair_type.array_type(meta.length as u32);
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
                    .build_gep(self.context.i8_type(), map_ptr, &[offset], "pair_ptr_i8")
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

        // Load key from pair struct (field 0)
        let key_ptr = self
            .builder
            .build_struct_gep(pair_type, pair_ptr, 0, "key_ptr")
            .unwrap();
        let key_val = self
            .builder
            .build_load(key_type, key_ptr, "key_val")
            .unwrap();

        // Load value from pair struct (field 1)
        let value_ptr = self
            .builder
            .build_struct_gep(pair_type, pair_ptr, 1, "value_ptr")
            .unwrap();
        let value_val = self
            .builder
            .build_load(val_type, value_ptr, "value_val")
            .unwrap();

        // Append key (as JSON key with quotes)
        self.append_value_to_buffer_as_key(buffer, key_val, &meta.key_type, sprintf_fn, strcat_fn);

        // Append ":"
        let colon = self.builder.build_global_string_ptr(":", "colon").unwrap();
        self.builder
            .build_call(
                strcat_fn,
                &[buffer.into(), colon.as_pointer_value().into()],
                "",
            )
            .unwrap();

        // Append value
        self.append_value_to_buffer(buffer, value_val, &meta.value_type, sprintf_fn, strcat_fn);

        // Increment counter
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

        // After loop
        self.builder.position_at_end(after_block);

        // Close with "}"
        let close_brace = self
            .builder
            .build_global_string_ptr("}", "close_brace")
            .unwrap();
        self.builder
            .build_call(
                strcat_fn,
                &[buffer.into(), close_brace.as_pointer_value().into()],
                "",
            )
            .unwrap();

        self.wrap_as_heap_string(dest, buffer)
    }

    fn append_value_to_buffer(
        &mut self,
        buffer: inkwell::values::PointerValue<'ctx>,
        value: BasicValueEnum<'ctx>,
        type_str: &str,
        sprintf_fn: inkwell::values::FunctionValue<'ctx>,
        strcat_fn: inkwell::values::FunctionValue<'ctx>,
    ) {
        let malloc_fn = self.get_malloc_fn();
        let temp_buf = self
            .builder
            .build_call(
                malloc_fn,
                &[self.context.i64_type().const_int(256, false).into()],
                "temp_buf",
            )
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        match type_str {
            "Int" => {
                let format = self
                    .builder
                    .build_global_string_ptr("%d", "int_fmt")
                    .unwrap();
                self.builder
                    .build_call(
                        sprintf_fn,
                        &[
                            temp_buf.into(),
                            format.as_pointer_value().into(),
                            value.into(),
                        ],
                        "",
                    )
                    .unwrap();
            }
            "Float" => {
                let format = self
                    .builder
                    .build_global_string_ptr("%.15g", "float_fmt")
                    .unwrap();
                self.builder
                    .build_call(
                        sprintf_fn,
                        &[
                            temp_buf.into(),
                            format.as_pointer_value().into(),
                            value.into(),
                        ],
                        "",
                    )
                    .unwrap();
            }
            "Bool" => {
                let int_val = value.into_int_value();
                let true_str = self
                    .builder
                    .build_global_string_ptr("true", "true_str")
                    .unwrap();
                let false_str = self
                    .builder
                    .build_global_string_ptr("false", "false_str")
                    .unwrap();

                let is_true = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        int_val,
                        int_val.get_type().const_zero(),
                        "is_true",
                    )
                    .unwrap();

                let selected = self
                    .builder
                    .build_select(
                        is_true,
                        true_str.as_pointer_value(),
                        false_str.as_pointer_value(),
                        "bool_str",
                    )
                    .unwrap();

                let strcpy_fn = self.get_strcpy_fn();
                self.builder
                    .build_call(strcpy_fn, &[temp_buf.into(), selected.into()], "")
                    .unwrap();
            }
            "Str" => {
                // String with quotes
                let quote = self.builder.build_global_string_ptr("\"", "quote").unwrap();
                let strcpy_fn = self.get_strcpy_fn();
                self.builder
                    .build_call(
                        strcpy_fn,
                        &[temp_buf.into(), quote.as_pointer_value().into()],
                        "",
                    )
                    .unwrap();

                self.builder
                    .build_call(strcat_fn, &[temp_buf.into(), value.into()], "")
                    .unwrap();

                self.builder
                    .build_call(
                        strcat_fn,
                        &[temp_buf.into(), quote.as_pointer_value().into()],
                        "",
                    )
                    .unwrap();
            }
            _ => {}
        }

        self.builder
            .build_call(strcat_fn, &[buffer.into(), temp_buf.into()], "")
            .unwrap();

        // Free temp buffer
        let free_fn = self.get_free_fn();
        self.builder
            .build_call(free_fn, &[temp_buf.into()], "")
            .unwrap();
    }

    fn append_value_to_buffer_as_key(
        &mut self,
        buffer: inkwell::values::PointerValue<'ctx>,
        value: BasicValueEnum<'ctx>,
        type_str: &str,
        sprintf_fn: inkwell::values::FunctionValue<'ctx>,
        strcat_fn: inkwell::values::FunctionValue<'ctx>,
    ) {
        // Keys in JSON objects must be strings
        let malloc_fn = self.get_malloc_fn();
        let temp_buf = self
            .builder
            .build_call(
                malloc_fn,
                &[self.context.i64_type().const_int(256, false).into()],
                "key_buf",
            )
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Always quote keys
        let quote = self.builder.build_global_string_ptr("\"", "quote").unwrap();
        let strcpy_fn = self.get_strcpy_fn();
        self.builder
            .build_call(
                strcpy_fn,
                &[temp_buf.into(), quote.as_pointer_value().into()],
                "",
            )
            .unwrap();

        match type_str {
            "Int" => {
                let inner_buf = self
                    .builder
                    .build_call(
                        malloc_fn,
                        &[self.context.i64_type().const_int(64, false).into()],
                        "inner_buf",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                let format = self
                    .builder
                    .build_global_string_ptr("%d", "int_fmt")
                    .unwrap();
                self.builder
                    .build_call(
                        sprintf_fn,
                        &[
                            inner_buf.into(),
                            format.as_pointer_value().into(),
                            value.into(),
                        ],
                        "",
                    )
                    .unwrap();

                self.builder
                    .build_call(strcat_fn, &[temp_buf.into(), inner_buf.into()], "")
                    .unwrap();

                let free_fn = self.get_free_fn();
                self.builder
                    .build_call(free_fn, &[inner_buf.into()], "")
                    .unwrap();
            }
            "Float" => {
                let inner_buf = self
                    .builder
                    .build_call(
                        malloc_fn,
                        &[self.context.i64_type().const_int(64, false).into()],
                        "inner_buf",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                let format = self
                    .builder
                    .build_global_string_ptr("%.15g", "float_fmt")
                    .unwrap();
                self.builder
                    .build_call(
                        sprintf_fn,
                        &[
                            inner_buf.into(),
                            format.as_pointer_value().into(),
                            value.into(),
                        ],
                        "",
                    )
                    .unwrap();

                self.builder
                    .build_call(strcat_fn, &[temp_buf.into(), inner_buf.into()], "")
                    .unwrap();

                let free_fn = self.get_free_fn();
                self.builder
                    .build_call(free_fn, &[inner_buf.into()], "")
                    .unwrap();
            }
            "Bool" => {
                let int_val = value.into_int_value();
                let true_str = self
                    .builder
                    .build_global_string_ptr("true", "true_str")
                    .unwrap();
                let false_str = self
                    .builder
                    .build_global_string_ptr("false", "false_str")
                    .unwrap();

                let is_true = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        int_val,
                        int_val.get_type().const_zero(),
                        "is_true",
                    )
                    .unwrap();

                let selected = self
                    .builder
                    .build_select(
                        is_true,
                        true_str.as_pointer_value(),
                        false_str.as_pointer_value(),
                        "bool_str",
                    )
                    .unwrap();

                self.builder
                    .build_call(strcat_fn, &[temp_buf.into(), selected.into()], "")
                    .unwrap();
            }
            "Str" => {
                self.builder
                    .build_call(strcat_fn, &[temp_buf.into(), value.into()], "")
                    .unwrap();
            }
            _ => {}
        }

        // Close quote
        self.builder
            .build_call(
                strcat_fn,
                &[temp_buf.into(), quote.as_pointer_value().into()],
                "",
            )
            .unwrap();

        self.builder
            .build_call(strcat_fn, &[buffer.into(), temp_buf.into()], "")
            .unwrap();

        let free_fn = self.get_free_fn();
        self.builder
            .build_call(free_fn, &[temp_buf.into()], "")
            .unwrap();
    }

    fn wrap_as_heap_string(
        &mut self,
        dest: &str,
        buffer: inkwell::values::PointerValue<'ctx>,
    ) -> Option<BasicValueEnum<'ctx>> {
        let malloc_fn = self.get_malloc_fn();
        let strlen_fn = self.get_strlen_fn();

        let str_len = self
            .builder
            .build_call(strlen_fn, &[buffer.into()], "result_len")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        let header_size = self.context.i64_type().const_int(8, false);
        let data_size = self
            .builder
            .build_int_add(
                str_len,
                self.context.i64_type().const_int(1, false),
                "data_size",
            )
            .unwrap();
        let total_size = self
            .builder
            .build_int_add(header_size, data_size, "total_size")
            .unwrap();

        let heap_ptr = self
            .builder
            .build_call(malloc_fn, &[total_size.into()], "heap_json")
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

        // Store length
        let str_len_i32 = self
            .builder
            .build_int_truncate(str_len, self.context.i32_type(), "len_i32")
            .unwrap();
        let len_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i32_type(),
                    heap_ptr,
                    &[self.context.i32_type().const_int(1, false)],
                    "len_ptr",
                )
                .unwrap()
        };
        self.builder.build_store(len_ptr, str_len_i32).unwrap();

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

        // Copy content
        let strcpy_fn = self.get_strcpy_fn();
        self.builder
            .build_call(strcpy_fn, &[data_ptr.into(), buffer.into()], "")
            .unwrap();

        // Free temp buffer
        let free_fn = self.get_free_fn();
        self.builder
            .build_call(free_fn, &[buffer.into()], "")
            .unwrap();

        self.temp_values.insert(dest.to_string(), data_ptr.into());
        self.heap_strings.insert(dest.to_string());

        // CRITICAL: If there's a pre-allocated symbol for this dest, store the result there too
        // This ensures resolve_value can find the value when loading from the symbol
        if let Some(sym) = self.symbols.get(dest) {
            self.builder.build_store(sym.ptr, data_ptr).unwrap();
        }

        Some(data_ptr.into())
    }

    fn get_llvm_type_from_string(&self, type_str: &str) -> inkwell::types::BasicTypeEnum<'ctx> {
        match type_str {
            "Int" => self.context.i32_type().into(),
            "Float" => self.context.f64_type().into(),
            // Bool is stored as i32 in Doo, not i1
            "Bool" => self.context.i32_type().into(),
            "Str" => self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .into(),
            _ => self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .into(),
        }
    }

    fn get_type_size(&self, type_str: &str) -> usize {
        match type_str {
            "Int" => 4,
            "Float" => 8,
            "Bool" => 1,
            "Str" => 8, // pointer size
            _ => 8,
        }
    }

    // Helper methods for C standard library functions
    fn get_malloc_fn(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        self.module.get_function("malloc").unwrap_or_else(|| {
            let fn_type = self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .fn_type(&[self.context.i64_type().into()], false);
            self.module.add_function("malloc", fn_type, None)
        })
    }

    fn get_free_fn(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        self.module.get_function("free").unwrap_or_else(|| {
            let fn_type = self.context.void_type().fn_type(
                &[self
                    .context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into()],
                false,
            );
            self.module.add_function("free", fn_type, None)
        })
    }

    fn get_sprintf_fn(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        self.module.get_function("sprintf").unwrap_or_else(|| {
            let fn_type = self.context.i32_type().fn_type(
                &[
                    self.context
                        .ptr_type(inkwell::AddressSpace::default())
                        .into(),
                    self.context
                        .ptr_type(inkwell::AddressSpace::default())
                        .into(),
                ],
                true,
            );
            self.module.add_function("sprintf", fn_type, None)
        })
    }

    fn get_strlen_fn(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        self.module.get_function("strlen").unwrap_or_else(|| {
            let fn_type = self.context.i64_type().fn_type(
                &[self
                    .context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into()],
                false,
            );
            self.module.add_function("strlen", fn_type, None)
        })
    }

    fn get_strcpy_fn(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        self.module.get_function("strcpy").unwrap_or_else(|| {
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
        })
    }

    fn get_strcat_fn(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        self.module.get_function("strcat").unwrap_or_else(|| {
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
        })
    }

    /// Convert a JSON string (from JSON.parse) to a typed LLVM value
    /// This is used when JSON.parse(...) is passed as a function parameter
    pub fn convert_json_string_to_type(
        &mut self,
        json_str_ptr: inkwell::values::PointerValue<'ctx>,
        expected_type: &str,
    ) -> Option<BasicValueEnum<'ctx>> {
        // For primitives from db.rawWithParams, the JSON is a result set like [{"count": 5}]
        // We need to use our json_extract_first_* functions to extract the scalar value

        // Handle Struct() wrapper (e.g. Struct(Int)) by unwrapping it
        let clean_type = if expected_type.starts_with("Struct(") && expected_type.ends_with(")") {
            &expected_type[7..expected_type.len() - 1]
        } else {
            expected_type
        };



        if clean_type == "Int" {
            // Call json_extract_scalar_v2 to extract integer from JSON result set
            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
            let extract_fn = self.module.get_function("json_extract_scalar_v2").unwrap_or_else(|| {
                let fn_type = self.context.i32_type().fn_type(&[ptr_type.into()], false);
                self.module.add_function("json_extract_scalar_v2", fn_type, None)
            });

            // Cast json_str_ptr to i8* if needed (it usually is)
            let i8_ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
            let json_ptr_casted = self.builder.build_bit_cast(json_str_ptr, i8_ptr_type, "json_ptr_casted").unwrap();

            // Call FFI -> returns i32 (Doo Int is i32)
            let int_val = self
                .builder
                .build_call(extract_fn, &[json_ptr_casted.into()], "parsed_int")
                .unwrap()
                .try_as_basic_value()
                .left()
                .unwrap()
                .into_int_value();

            return Some(int_val.as_basic_value_enum());
        } else if clean_type == "Float" {
            // Call json_extract_first_float to extract float from JSON result set
            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
            let extract_fn = self
                .module
                .get_function("json_extract_first_float")
                .unwrap_or_else(|| {
                    let fn_type = self.context.f64_type().fn_type(&[ptr_type.into()], false);
                    self.module
                        .add_function("json_extract_first_float", fn_type, None)
                });

            let i8_ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
            let json_ptr_casted = self
                .builder
                .build_bit_cast(json_str_ptr, i8_ptr_type, "json_ptr_casted")
                .unwrap();

            let float_val = self
                .builder
                .build_call(extract_fn, &[json_ptr_casted.into()], "parsed_float")
                .unwrap()
                .try_as_basic_value()
                .left()
                .unwrap();

            return Some(float_val);
        } else if clean_type == "Bool" {
            // Call json_extract_first_bool to extract bool from JSON result set
            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
            let extract_fn = self
                .module
                .get_function("json_extract_first_bool")
                .unwrap_or_else(|| {
                    let fn_type = self.context.i32_type().fn_type(&[ptr_type.into()], false);
                    self.module
                        .add_function("json_extract_first_bool", fn_type, None)
                });

            let i8_ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
            let json_ptr_casted = self
                .builder
                .build_bit_cast(json_str_ptr, i8_ptr_type, "json_ptr_casted")
                .unwrap();

            let bool_val = self
                .builder
                .build_call(extract_fn, &[json_ptr_casted.into()], "parsed_bool")
                .unwrap()
                .try_as_basic_value()
                .left()
                .unwrap();

            return Some(bool_val);
        } else if clean_type == "Str" {
            // For string, remove surrounding quotes if present
            // JSON.stringify adds quotes around strings: "abc" -> "\"abc\""
            // We need to strip those outer quotes

            // Check if first char is quote
            let first_char = self
                .builder
                .build_load(self.context.i8_type(), json_str_ptr, "first_char")
                .unwrap()
                .into_int_value();

            let is_quote = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::EQ,
                    first_char,
                    self.context.i8_type().const_int(b'"' as u64, false),
                    "is_quote",
                )
                .unwrap();

            // If it starts with quote, skip first char and calculate new length
            let strlen_fn = self.get_strlen_fn();
            let str_len = self
                .builder
                .build_call(strlen_fn, &[json_str_ptr.into()], "json_str_len")
                .unwrap()
                .try_as_basic_value()
                .left()
                .unwrap()
                .into_int_value();

            // New pointer: json_str_ptr + 1 if quote, else json_str_ptr
            let offset = self
                .builder
                .build_select(
                    is_quote,
                    self.context.i64_type().const_int(1, false),
                    self.context.i64_type().const_int(0, false),
                    "offset",
                )
                .unwrap()
                .into_int_value();

            let new_ptr = unsafe {
                self.builder
                    .build_gep(self.context.i8_type(), json_str_ptr, &[offset], "str_ptr")
                    .unwrap()
            };

            // New length: str_len - 2 if quote (remove leading and trailing), else str_len
            let len_offset = self
                .builder
                .build_select(
                    is_quote,
                    self.context.i64_type().const_int(2, false),
                    self.context.i64_type().const_int(0, false),
                    "len_offset",
                )
                .unwrap()
                .into_int_value();

            let new_len = self
                .builder
                .build_int_sub(str_len, len_offset, "new_len")
                .unwrap();

            // Allocate new string with RC header
            let malloc_fn = self.get_malloc_fn();
            let header_size = self.context.i64_type().const_int(8, false);
            let data_size = self
                .builder
                .build_int_add(
                    new_len,
                    self.context.i64_type().const_int(1, false),
                    "data_size",
                )
                .unwrap();
            let total_size = self
                .builder
                .build_int_add(header_size, data_size, "total_size")
                .unwrap();

            let heap_ptr = self
                .builder
                .build_call(malloc_fn, &[total_size.into()], "heap_str")
                .unwrap()
                .try_as_basic_value()
                .left()
                .unwrap()
                .into_pointer_value();

            // Store RC = 1 at offset 0
            let rc_ptr = heap_ptr;
            self.builder
                .build_store(rc_ptr, self.context.i32_type().const_int(1, false))
                .unwrap();

            // Store length at offset 4
            let new_len_i32 = self
                .builder
                .build_int_truncate(new_len, self.context.i32_type(), "len_i32")
                .unwrap();
            let len_ptr = unsafe {
                self.builder
                    .build_gep(
                        self.context.i32_type(),
                        heap_ptr,
                        &[self.context.i32_type().const_int(1, false)],
                        "len_ptr",
                    )
                    .unwrap()
            };
            self.builder.build_store(len_ptr, new_len_i32).unwrap();

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

            // Copy string content using memcpy
            let memcpy_fn = self
                .module
                .get_function("llvm.memcpy.p0.p0.i64")
                .unwrap_or_else(|| {
                    let fn_type = self.context.void_type().fn_type(
                        &[
                            self.context
                                .ptr_type(inkwell::AddressSpace::default())
                                .into(),
                            self.context
                                .ptr_type(inkwell::AddressSpace::default())
                                .into(),
                            self.context.i64_type().into(),
                            self.context.bool_type().into(),
                        ],
                        false,
                    );
                    self.module
                        .add_function("llvm.memcpy.p0.p0.i64", fn_type, None)
                });

            self.builder
                .build_call(
                    memcpy_fn,
                    &[
                        data_ptr.into(),
                        new_ptr.into(),
                        new_len.into(),
                        self.context.bool_type().const_zero().into(),
                    ],
                    "",
                )
                .unwrap();

            // Null terminate
            let null_ptr = unsafe {
                self.builder
                    .build_gep(self.context.i8_type(), data_ptr, &[new_len], "null_ptr")
                    .unwrap()
            };
            self.builder
                .build_store(null_ptr, self.context.i8_type().const_zero())
                .unwrap();

            return Some(data_ptr.into());
        } else if expected_type.starts_with("Array(") {
            // Extract element type from Array(ElementType)
            let element_type = &expected_type[6..expected_type.len() - 1];

            // Choose the right runtime parser based on element type
            let fn_name = match element_type {
                "Int" => "json_parse_array_int",
                "Float" => "json_parse_array_float",
                "Bool" => "json_parse_array_bool",
                "Str" => "json_parse_array_str",
                _ => {
                    // Check if element type is a known struct
                    if self.struct_metadata.contains_key(element_type) {
                        // For struct arrays, return JSON string as-is
                        // The HTTP handler will serialize it directly
                        return Some(json_str_ptr.into());
                    } else {
                        // Fallback - return pointer as-is for unsupported types
                        return Some(json_str_ptr.into());
                    }
                }
            };

            // Declare or get the runtime function
            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
            let parse_fn = self.module.get_function(fn_name).unwrap_or_else(|| {
                let fn_type = ptr_type.fn_type(&[ptr_type.into()], false);
                self.module.add_function(fn_name, fn_type, None)
            });

            // Call the runtime parser
            let result_ptr = self
                .builder
                .build_call(parse_fn, &[json_str_ptr.into()], "parsed_array")
                .unwrap()
                .try_as_basic_value()
                .left()
                .unwrap()
                .into_pointer_value();

            return Some(result_ptr.into());
        } else if expected_type.starts_with("Map(") {
            // Extract key and value types from Map(KeyType,ValueType)
            let inner = &expected_type[4..expected_type.len() - 1];
            let parts: Vec<&str> = inner.split(',').collect();
            if parts.len() != 2 {
                return Some(json_str_ptr.into());
            }
            let key_type = parts[0].trim();
            let value_type = parts[1].trim();

            // Choose the right runtime parser
            let fn_name = match (key_type, value_type) {
                ("Str", "Int") => "json_parse_map_str_int",
                ("Str", "Float") => "json_parse_map_str_float",
                ("Str", "Bool") => "json_parse_map_str_bool",
                ("Str", "Str") => "json_parse_map_str_str",
                ("Int", "Int") => "json_parse_map_int_int",
                ("Int", "Float") => "json_parse_map_int_float",
                ("Int", "Bool") => "json_parse_map_int_bool",
                ("Int", "Str") => "json_parse_map_int_str",
                ("Float", "Int") => "json_parse_map_float_int",
                ("Float", "Float") => "json_parse_map_float_float",
                ("Float", "Bool") => "json_parse_map_float_bool",
                ("Float", "Str") => "json_parse_map_float_str",
                ("Bool", "Int") => "json_parse_map_bool_int",
                ("Bool", "Float") => "json_parse_map_bool_float",
                ("Bool", "Bool") => "json_parse_map_bool_bool",
                ("Bool", "Str") => "json_parse_map_bool_str",
                _ => {
                    // Fallback for unsupported types
                    return Some(json_str_ptr.into());
                }
            };

            // Declare or get the runtime function
            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
            let parse_fn = self.module.get_function(fn_name).unwrap_or_else(|| {
                let fn_type = ptr_type.fn_type(&[ptr_type.into()], false);
                self.module.add_function(fn_name, fn_type, None)
            });

            // Call the runtime parser
            let result_ptr = self
                .builder
                .build_call(parse_fn, &[json_str_ptr.into()], "parsed_map")
                .unwrap()
                .try_as_basic_value()
                .left()
                .unwrap()
                .into_pointer_value();

            return Some(result_ptr.into());
        } else if expected_type.starts_with("Enum(")
            || self.enum_variant_order.contains_key(expected_type)
        {
            // Enum types - parse the JSON string to extract tag and payload
            // JSON.stringify(Status::Active) produces "\"Active\"" (unit variant)
            // JSON.stringify(Result::Success(123)) produces "{\"Success\":123}" (payload variant)

            // Get the enum name
            let enum_name = if expected_type.starts_with("Enum(") && expected_type.ends_with(")") {
                &expected_type[5..expected_type.len() - 1]
            } else {
                expected_type
            };

            // Clone variants to avoid borrow issues - use enum_variant_order for correct declaration order
            let variants_opt = self.enum_variant_order.get(enum_name).cloned();

            // Get enum variants to determine the tag
            if let Some(variants) = variants_opt {
                // Allocate the enum struct { i32 tag, ptr payload }
                let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                let enum_struct_type = self
                    .context
                    .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false);

                let malloc_fn = self.get_malloc_fn();
                let struct_size = self.context.i64_type().const_int(16, false); // i32 + ptr = 16 bytes on 64-bit
                let enum_ptr = self
                    .builder
                    .build_call(malloc_fn, &[struct_size.into()], "enum_alloc")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                // Check first char to determine format: '{' = payload variant, '"' = unit variant
                let first_char = self
                    .builder
                    .build_load(self.context.i8_type(), json_str_ptr, "first_char")
                    .unwrap()
                    .into_int_value();

                let is_object = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        first_char,
                        self.context.i8_type().const_int(b'{' as u64, false),
                        "is_object",
                    )
                    .unwrap();

                // Calculate content pointer: skip '{\"' for objects or '\"' for unit variants
                let object_offset = self.context.i64_type().const_int(2, false); // Skip {"
                let quote_offset = self.context.i64_type().const_int(1, false); // Skip "
                let offset = self
                    .builder
                    .build_select(is_object, object_offset, quote_offset, "skip_offset")
                    .unwrap()
                    .into_int_value();

                let content_ptr = unsafe {
                    self.builder
                        .build_gep(
                            self.context.i8_type(),
                            json_str_ptr,
                            &[offset],
                            "content_ptr",
                        )
                        .unwrap()
                };

                // Get strncmp function for comparing variant names
                let strncmp_fn = self.module.get_function("strncmp").unwrap_or_else(|| {
                    let fn_type = self.context.i32_type().fn_type(
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
                    self.module.add_function("strncmp", fn_type, None)
                });

                // Default tag to 0 (first variant)
                let tag_alloca = self
                    .builder
                    .build_alloca(self.context.i32_type(), "tag_var")
                    .unwrap();
                self.builder
                    .build_store(tag_alloca, self.context.i32_type().const_int(0, false))
                    .unwrap();

                // Alloca for payload pointer
                let payload_alloca = self
                    .builder
                    .build_alloca(ptr_type, "payload_alloca")
                    .unwrap();
                self.builder
                    .build_store(payload_alloca, ptr_type.const_null())
                    .unwrap();

                let current_fn = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();

                // Compare against each variant
                for (idx, (variant_name, payload_type_opt)) in variants.iter().enumerate() {
                    let variant_str = self
                        .builder
                        .build_global_string_ptr(variant_name, &format!("variant_{}", variant_name))
                        .unwrap();
                    let variant_len = self
                        .context
                        .i64_type()
                        .const_int(variant_name.len() as u64, false);

                    let cmp_result = self
                        .builder
                        .build_call(
                            strncmp_fn,
                            &[
                                content_ptr.into(),
                                variant_str.as_pointer_value().into(),
                                variant_len.into(),
                            ],
                            "cmp_result",
                        )
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_int_value();

                    let is_match = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            cmp_result,
                            self.context.i32_type().const_int(0, false),
                            "is_match",
                        )
                        .unwrap();

                    let then_bb = self
                        .context
                        .append_basic_block(current_fn, &format!("match_{}", variant_name));
                    let cont_bb = self
                        .context
                        .append_basic_block(current_fn, &format!("cont_{}", variant_name));

                    self.builder
                        .build_conditional_branch(is_match, then_bb, cont_bb)
                        .unwrap();

                    self.builder.position_at_end(then_bb);
                    self.builder
                        .build_store(
                            tag_alloca,
                            self.context.i32_type().const_int(idx as u64, false),
                        )
                        .unwrap();

                    // Parse payload if this variant has one and JSON is object format
                    if let Some(payload_type) = payload_type_opt {
                        // Only parse payload if we have object format (starts with '{')
                        let parse_payload_bb = self
                            .context
                            .append_basic_block(current_fn, &format!("parse_payload_{}", idx));
                        let skip_payload_bb = self
                            .context
                            .append_basic_block(current_fn, &format!("skip_payload_{}", idx));

                        self.builder
                            .build_conditional_branch(is_object, parse_payload_bb, skip_payload_bb)
                            .unwrap();

                        self.builder.position_at_end(parse_payload_bb);

                        // Find colon position: content_ptr points after {"VariantName
                        // We need to skip past variant_name and ":
                        // Payload starts at content_ptr + variant_len + 2 (for ":)
                        let payload_offset = self
                            .builder
                            .build_int_add(
                                variant_len,
                                self.context.i64_type().const_int(2, false),
                                "payload_offset",
                            )
                            .unwrap();
                        let payload_start = unsafe {
                            self.builder
                                .build_gep(
                                    self.context.i8_type(),
                                    content_ptr,
                                    &[payload_offset],
                                    "payload_start",
                                )
                                .unwrap()
                        };

                        // Parse payload based on type
                        let type_str = format!("{:?}", payload_type);
                        let parsed_payload = if type_str.contains("String")
                            || type_str.contains("Str")
                        {
                            // String payload: skip opening quote, find closing quote
                            // payload_start points to "value"}
                            let str_content = unsafe {
                                self.builder
                                    .build_gep(
                                        self.context.i8_type(),
                                        payload_start,
                                        &[self.context.i64_type().const_int(1, false)],
                                        "str_content",
                                    )
                                    .unwrap()
                            };

                            // Get strlen to find the string length (minus closing "})
                            let strlen_fn = self.get_strlen_fn();
                            let full_len = self
                                .builder
                                .build_call(strlen_fn, &[str_content.into()], "str_full_len")
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap()
                                .into_int_value();

                            // Subtract 2 for closing "}
                            let str_len = self
                                .builder
                                .build_int_sub(
                                    full_len,
                                    self.context.i64_type().const_int(2, false),
                                    "str_len",
                                )
                                .unwrap();

                            // Allocate and copy string
                            let alloc_size = self
                                .builder
                                .build_int_add(
                                    str_len,
                                    self.context.i64_type().const_int(1, false),
                                    "alloc_size",
                                )
                                .unwrap();
                            let str_alloc = self
                                .builder
                                .build_call(malloc_fn, &[alloc_size.into()], "str_alloc")
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap()
                                .into_pointer_value();

                            // Copy string content
                            let memcpy_fn = self
                                .module
                                .get_function("llvm.memcpy.p0.p0.i64")
                                .unwrap_or_else(|| {
                                    let fn_type = self.context.void_type().fn_type(
                                        &[
                                            ptr_type.into(),
                                            ptr_type.into(),
                                            self.context.i64_type().into(),
                                            self.context.bool_type().into(),
                                        ],
                                        false,
                                    );
                                    self.module
                                        .add_function("llvm.memcpy.p0.p0.i64", fn_type, None)
                                });

                            self.builder
                                .build_call(
                                    memcpy_fn,
                                    &[
                                        str_alloc.into(),
                                        str_content.into(),
                                        str_len.into(),
                                        self.context.bool_type().const_zero().into(),
                                    ],
                                    "",
                                )
                                .unwrap();

                            // Null terminate
                            let null_ptr = unsafe {
                                self.builder
                                    .build_gep(
                                        self.context.i8_type(),
                                        str_alloc,
                                        &[str_len],
                                        "null_ptr",
                                    )
                                    .unwrap()
                            };
                            self.builder
                                .build_store(null_ptr, self.context.i8_type().const_zero())
                                .unwrap();

                            str_alloc
                        } else if type_str.contains("Float") {
                            // Float payload - use atof
                            let atof_fn = self.module.get_function("atof").unwrap_or_else(|| {
                                let fn_type =
                                    self.context.f64_type().fn_type(&[ptr_type.into()], false);
                                self.module.add_function("atof", fn_type, None)
                            });

                            let float_val = self
                                .builder
                                .build_call(atof_fn, &[payload_start.into()], "parsed_float")
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap();

                            // Box the float
                            let float_box = self
                                .builder
                                .build_call(
                                    malloc_fn,
                                    &[self.context.i64_type().const_int(8, false).into()],
                                    "float_box",
                                )
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap()
                                .into_pointer_value();
                            self.builder.build_store(float_box, float_val).unwrap();
                            float_box
                        } else {
                            // Int payload - use atoi
                            let atoi_fn = self.module.get_function("atoi").unwrap_or_else(|| {
                                let fn_type =
                                    self.context.i32_type().fn_type(&[ptr_type.into()], false);
                                self.module.add_function("atoi", fn_type, None)
                            });

                            let int_val = self
                                .builder
                                .build_call(atoi_fn, &[payload_start.into()], "parsed_int")
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap();

                            // Box the int
                            let int_box = self
                                .builder
                                .build_call(
                                    malloc_fn,
                                    &[self.context.i64_type().const_int(4, false).into()],
                                    "int_box",
                                )
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap()
                                .into_pointer_value();
                            self.builder.build_store(int_box, int_val).unwrap();
                            int_box
                        };

                        self.builder
                            .build_store(payload_alloca, parsed_payload)
                            .unwrap();
                        self.builder.build_unconditional_branch(cont_bb).unwrap();

                        self.builder.position_at_end(skip_payload_bb);
                    }

                    self.builder.build_unconditional_branch(cont_bb).unwrap();
                    self.builder.position_at_end(cont_bb);
                }

                // Load the determined tag
                let final_tag = self
                    .builder
                    .build_load(self.context.i32_type(), tag_alloca, "final_tag")
                    .unwrap()
                    .into_int_value();

                // Store tag in enum struct
                let tag_ptr = self
                    .builder
                    .build_struct_gep(enum_struct_type, enum_ptr, 0, "tag_ptr")
                    .unwrap();
                self.builder.build_store(tag_ptr, final_tag).unwrap();

                // Store payload pointer
                let final_payload = self
                    .builder
                    .build_load(ptr_type, payload_alloca, "final_payload")
                    .unwrap();
                let payload_ptr = self
                    .builder
                    .build_struct_gep(enum_struct_type, enum_ptr, 1, "payload_ptr")
                    .unwrap();
                self.builder
                    .build_store(payload_ptr, final_payload)
                    .unwrap();

                // Load the enum struct to return by value
                let enum_val = self
                    .builder
                    .build_load(enum_struct_type, enum_ptr, "enum_val")
                    .unwrap();

                return Some(enum_val);
            }

            return None;
        } else if matches!(
            expected_type,
            "Int"
                | "Float"
                | "Bool"
                | "Str"
                | "Struct(Int)"
                | "Struct(Float)"
                | "Struct(Bool)"
                | "Struct(Str)"
        ) {
            // Primitive types - extract scalar value from JSON result set
            // COUNT queries return: [{"count": 5}] or [{"COUNT(*)": 5}]
            // We need to extract the first value from the first row

            // Handle Struct() wrapper (e.g. Struct(Int)) by unwrapping it
            let clean_type = if expected_type.starts_with("Struct(") && expected_type.ends_with(")")
            {
                &expected_type[7..expected_type.len() - 1]
            } else {
                expected_type
            };

            // Call runtime helper to extract first scalar value from result set JSON
            let fn_name = match clean_type {
                "Int" => "json_extract_first_int",
                "Float" => "json_extract_first_float",
                "Bool" => "json_extract_first_bool",
                "Str" => "json_extract_first_str",
                _ => return Some(json_str_ptr.into()),
            };

            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
            let extract_fn = self.module.get_function(fn_name).unwrap_or_else(|| {
                let fn_type = match clean_type {
                    "Int" | "Bool" => self.context.i32_type().fn_type(&[ptr_type.into()], false),
                    "Float" => self.context.f64_type().fn_type(&[ptr_type.into()], false),
                    "Str" => ptr_type.fn_type(&[ptr_type.into()], false),
                    _ => self.context.i32_type().fn_type(&[ptr_type.into()], false),
                };
                self.module.add_function(fn_name, fn_type, None)
            });

            // Call the extraction function
            let scalar_value = self
                .builder
                .build_call(extract_fn, &[json_str_ptr.into()], "scalar_value")
                .unwrap()
                .try_as_basic_value()
                .left()
                .unwrap();

            return Some(scalar_value);
        } else if expected_type.starts_with("Struct(")
            || self.struct_metadata.contains_key(expected_type)
        {
            // Struct types - parse JSON object into struct
            // Extract struct name
            let struct_name =
                if expected_type.starts_with("Struct(") && expected_type.ends_with(")") {
                    &expected_type[7..expected_type.len() - 1]
                } else {
                    expected_type
                };

            // Get struct metadata
            let metadata_opt = self.struct_metadata.get(struct_name).cloned();
            let canonical_type_opt = self.canonical_struct_types.get(struct_name).cloned();

            if let (Some(metadata), Some(canonical_type)) = (metadata_opt, canonical_type_opt) {
                // Allocate struct on heap
                let malloc_fn = self.get_malloc_fn();
                let struct_size = canonical_type.size_of().unwrap();
                let struct_ptr = self
                    .builder
                    .build_call(malloc_fn, &[struct_size.into()], "struct_alloc")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                // For each field, find it in the JSON and parse it
                // We'll use runtime helpers to extract field values
                let json_get_int_fn = self.get_or_declare_json_get_int();
                let json_get_float_fn = self.get_or_declare_json_get_float();
                let json_get_bool_fn = self.get_or_declare_json_get_bool();
                let json_get_str_fn = self.get_or_declare_json_get_str();
                let json_validate_field_fn = self.get_or_declare_json_validate_field();

                for (field_idx, (field_name, field_type)) in metadata
                    .field_names
                    .iter()
                    .zip(metadata.field_types.iter())
                    .enumerate()
                {
                    let field_name_str = self
                        .builder
                        .build_global_string_ptr(field_name, &format!("field_{}", field_name))
                        .unwrap();

                    let field_ptr = self
                        .builder
                        .build_struct_gep(
                            canonical_type,
                            struct_ptr,
                            field_idx as u32,
                            &format!("field_ptr_{}", field_name),
                        )
                        .unwrap();

                    match field_type.as_str() {
                        "Int" => {
                            // Validate field type before extraction
                            let type_str = self
                                .builder
                                .build_global_string_ptr("Int", "type_int")
                                .unwrap();
                            let is_valid = self
                                .builder
                                .build_call(
                                    json_validate_field_fn,
                                    &[
                                        json_str_ptr.into(),
                                        field_name_str.as_pointer_value().into(),
                                        type_str.as_pointer_value().into(),
                                    ],
                                    "validate_field_int",
                                )
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap()
                                .into_int_value();

                            // If validation fails (returns 0), store 0 and continue
                            // The HTTP layer will detect this as a validation error
                            let val = self
                                .builder
                                .build_call(
                                    json_get_int_fn,
                                    &[
                                        json_str_ptr.into(),
                                        field_name_str.as_pointer_value().into(),
                                    ],
                                    "field_int",
                                )
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap();
                            self.builder.build_store(field_ptr, val).unwrap();
                        }
                        "Float" => {
                            let val = self
                                .builder
                                .build_call(
                                    json_get_float_fn,
                                    &[
                                        json_str_ptr.into(),
                                        field_name_str.as_pointer_value().into(),
                                    ],
                                    "field_float",
                                )
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap();
                            self.builder.build_store(field_ptr, val).unwrap();
                        }
                        "Bool" => {
                            let val = self
                                .builder
                                .build_call(
                                    json_get_bool_fn,
                                    &[
                                        json_str_ptr.into(),
                                        field_name_str.as_pointer_value().into(),
                                    ],
                                    "field_bool",
                                )
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap();
                            self.builder.build_store(field_ptr, val).unwrap();
                        }
                        "Str" => {
                            let val = self
                                .builder
                                .build_call(
                                    json_get_str_fn,
                                    &[
                                        json_str_ptr.into(),
                                        field_name_str.as_pointer_value().into(),
                                    ],
                                    "field_str",
                                )
                                .unwrap()
                                .try_as_basic_value()
                                .left()
                                .unwrap();
                            self.builder.build_store(field_ptr, val).unwrap();
                        }
                        _ => {
                            // For complex types (arrays, maps, enums), store null for now
                            let null_ptr = self
                                .context
                                .ptr_type(inkwell::AddressSpace::default())
                                .const_null();
                            self.builder.build_store(field_ptr, null_ptr).unwrap();
                        }
                    }
                }

                // Note: Type mismatch validation removed here because the return type check
                // was generating incorrect LLVM IR (returning ptr null instead of { i32, ptr }).
                // Type mismatches will still be caught by the runtime's JSON parsing logic.
                // If needed, proper error handling should be done at the caller level
                // by checking dooruntime_get_json_type_mismatch() after the call returns.

                return Some(struct_ptr.into());
            }

            // Fallback: return pointer as-is
            return Some(json_str_ptr.into());
        }

        // For unsupported types, return None
        None
    }

    fn get_or_declare_json_get_int(&self) -> inkwell::values::FunctionValue<'ctx> {
        let fn_name = "json_get_int";
        self.module.get_function(fn_name).unwrap_or_else(|| {
            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
            let fn_type = self
                .context
                .i32_type()
                .fn_type(&[ptr_type.into(), ptr_type.into()], false);
            self.module.add_function(fn_name, fn_type, None)
        })
    }

    fn get_or_declare_json_get_float(&self) -> inkwell::values::FunctionValue<'ctx> {
        let fn_name = "json_get_float";
        self.module.get_function(fn_name).unwrap_or_else(|| {
            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
            let fn_type = self
                .context
                .f64_type()
                .fn_type(&[ptr_type.into(), ptr_type.into()], false);
            self.module.add_function(fn_name, fn_type, None)
        })
    }

    fn get_or_declare_json_get_bool(&self) -> inkwell::values::FunctionValue<'ctx> {
        let fn_name = "json_get_bool";
        self.module.get_function(fn_name).unwrap_or_else(|| {
            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
            let fn_type = self
                .context
                .i32_type()
                .fn_type(&[ptr_type.into(), ptr_type.into()], false);
            self.module.add_function(fn_name, fn_type, None)
        })
    }

    fn get_or_declare_json_get_str(&self) -> inkwell::values::FunctionValue<'ctx> {
        let fn_name = "json_get_str";
        self.module.get_function(fn_name).unwrap_or_else(|| {
            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
            let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
            self.module.add_function(fn_name, fn_type, None)
        })
    }

    fn get_or_declare_json_validate_field(&self) -> inkwell::values::FunctionValue<'ctx> {
        let fn_name = "json_validate_field_type";
        self.module.get_function(fn_name).unwrap_or_else(|| {
            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
            let fn_type = self
                .context
                .i32_type()
                .fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false);
            self.module.add_function(fn_name, fn_type, None)
        })
    }
}
