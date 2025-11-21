use crate::codegen::core::CodeGen;
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

        // Otherwise, handle as primitive
        self.stringify_primitive(dest, arg_name)
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

        Some(data_ptr.into())
    }

    fn get_llvm_type_from_string(&self, type_str: &str) -> inkwell::types::BasicTypeEnum<'ctx> {
        match type_str {
            "Int" => self.context.i32_type().into(),
            "Float" => self.context.f64_type().into(),
            "Bool" => self.context.bool_type().into(),
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
}
