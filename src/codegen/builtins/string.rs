use crate::codegen::{core::CodeGen, ArrayMetadata};
use inkwell::values::BasicValueEnum;

impl<'ctx> CodeGen<'ctx> {
    pub fn generate_string_method(
        &mut self,
        dest: &str,
        _object: &str,
        object_val: BasicValueEnum<'ctx>,
        method: &str,
        args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        match method {
            "len" => {
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

                let str_ptr = object_val.into_pointer_value();
                let len_i64 = self
                    .builder
                    .build_call(strlen_fn, &[str_ptr.into()], "strlen")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                let len = self
                    .builder
                    .build_int_truncate(len_i64, self.context.i32_type(), "len_i32")
                    .unwrap();
                self.temp_values.insert(dest.to_string(), len.into());
                Some(len.into())
            }
            "charAt" => {
                let index_val = self.resolve_value(&args[0]);
                let str_ptr = object_val.into_pointer_value();

                let char_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            str_ptr,
                            &[index_val.into_int_value()],
                            "char_ptr",
                        )
                        .unwrap()
                };

                let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
                    let fn_type = self
                        .context
                        .ptr_type(inkwell::AddressSpace::default())
                        .fn_type(&[self.context.i64_type().into()], false);
                    self.module.add_function("malloc", fn_type, None)
                });

                // Allocate with RC header: [RC: 4 bytes][Length: 4 bytes][char + null]
                let header_size = self.context.i64_type().const_int(8, false);
                let data_size = self.context.i64_type().const_int(2, false); // char + null
                let total_size = self
                    .builder
                    .build_int_add(header_size, data_size, "total_size")
                    .unwrap();

                let heap_ptr = self
                    .builder
                    .build_call(malloc_fn, &[total_size.into()], "heap_char_str")
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

                // Store string length at offset 4 (length = 1 for single char)
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
                self.builder
                    .build_store(len_ptr_cast, self.context.i32_type().const_int(1, false))
                    .unwrap();

                // Get data pointer at offset 8
                let result_ptr = unsafe {
                    self.builder
                        .build_gep(
                            self.context.i8_type(),
                            heap_ptr,
                            &[self.context.i32_type().const_int(8, false)],
                            "data_ptr",
                        )
                        .unwrap()
                };

                let char_val = self
                    .builder
                    .build_load(self.context.i8_type(), char_ptr, "char")
                    .unwrap();
                let _ = self.builder.build_store(result_ptr, char_val);

                let null_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            result_ptr,
                            &[self.context.i32_type().const_int(1, false)],
                            "null_ptr",
                        )
                        .unwrap()
                };
                let _ = self
                    .builder
                    .build_store(null_ptr, self.context.i8_type().const_int(0, false));

                self.temp_values.insert(dest.to_string(), result_ptr.into());
                self.heap_strings.insert(dest.to_string());
                Some(result_ptr.into())
            }
            "substring" => {
                let start_val = self.resolve_value(&args[0]);
                let end_val = self.resolve_value(&args[1]);
                let str_ptr = object_val.into_pointer_value();

                let len = self
                    .builder
                    .build_int_sub(
                        end_val.into_int_value(),
                        start_val.into_int_value(),
                        "substr_len",
                    )
                    .unwrap();

                let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
                    let fn_type = self
                        .context
                        .ptr_type(inkwell::AddressSpace::default())
                        .fn_type(&[self.context.i64_type().into()], false);
                    self.module.add_function("malloc", fn_type, None)
                });

                // Allocate with RC header: [RC: 4 bytes][Length: 4 bytes][string data + null]
                let len_plus_one = self
                    .builder
                    .build_int_add(
                        len,
                        self.context.i32_type().const_int(1, false),
                        "len_plus_one",
                    )
                    .unwrap();
                let header_size = self.context.i64_type().const_int(8, false);
                let data_size = self
                    .builder
                    .build_int_z_extend(len_plus_one, self.context.i64_type(), "data_size")
                    .unwrap();
                let total_size = self
                    .builder
                    .build_int_add(header_size, data_size, "total_size")
                    .unwrap();

                let heap_ptr = self
                    .builder
                    .build_call(malloc_fn, &[total_size.into()], "heap_reversed_str")
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
                self.builder.build_store(len_ptr_cast, len).unwrap();

                // Get data pointer at offset 8
                let result_ptr = unsafe {
                    self.builder
                        .build_gep(
                            self.context.i8_type(),
                            heap_ptr,
                            &[self.context.i32_type().const_int(8, false)],
                            "data_ptr",
                        )
                        .unwrap()
                };

                let src_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            str_ptr,
                            &[start_val.into_int_value()],
                            "src_ptr",
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

                let len_i64 = self
                    .builder
                    .build_int_z_extend(len, self.context.i64_type(), "len_i64")
                    .unwrap();
                let _ = self.builder.build_call(
                    memcpy_fn,
                    &[result_ptr.into(), src_ptr.into(), len_i64.into()],
                    "",
                );

                let null_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(self.context.i8_type(), result_ptr, &[len], "null_ptr")
                        .unwrap()
                };
                let _ = self
                    .builder
                    .build_store(null_ptr, self.context.i8_type().const_int(0, false));

                self.temp_values.insert(dest.to_string(), result_ptr.into());
                self.heap_strings.insert(dest.to_string());
                Some(result_ptr.into())
            }
            "toUpper" | "toLower" => {
                let str_ptr = object_val.into_pointer_value();
                let is_upper = method == "toUpper";

                // Get string length
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

                let len_i64 = self
                    .builder
                    .build_call(strlen_fn, &[str_ptr.into()], "strlen")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                // Allocate new string
                let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
                    let fn_type = self
                        .context
                        .ptr_type(inkwell::AddressSpace::default())
                        .fn_type(&[self.context.i64_type().into()], false);
                    self.module.add_function("malloc", fn_type, None)
                });

                // Get lengths first
                let len_i32 = self
                    .builder
                    .build_int_truncate(len_i64, self.context.i32_type(), "len_i32")
                    .unwrap();

                // Allocate with RC header: [RC: 4 bytes][Length: 4 bytes][string data + null]
                let len_plus_one = self
                    .builder
                    .build_int_add(
                        len_i64,
                        self.context.i64_type().const_int(1, false),
                        "len_plus_one",
                    )
                    .unwrap();
                let header_size = self.context.i64_type().const_int(8, false);
                let total_size = self
                    .builder
                    .build_int_add(header_size, len_plus_one, "total_size")
                    .unwrap();

                let heap_ptr = self
                    .builder
                    .build_call(malloc_fn, &[total_size.into()], "heap_concat_str")
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
                self.builder.build_store(len_ptr_cast, len_i32).unwrap();

                // Get data pointer at offset 8
                let result_ptr = unsafe {
                    self.builder
                        .build_gep(
                            self.context.i8_type(),
                            heap_ptr,
                            &[self.context.i32_type().const_int(8, false)],
                            "data_ptr",
                        )
                        .unwrap()
                };

                let _unused_val = self
                    .builder
                    .build_call(
                        malloc_fn,
                        &[self.context.i64_type().const_int(1, false).into()],
                        "unused_result_str",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                // Convert case character by character
                let current_fn = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let loop_block = self.context.append_basic_block(current_fn, "case_loop");
                let body_block = self.context.append_basic_block(current_fn, "case_body");
                let after_block = self.context.append_basic_block(current_fn, "case_after");

                let counter_ptr = self
                    .builder
                    .build_alloca(self.context.i64_type(), "counter")
                    .unwrap();
                self.builder
                    .build_store(counter_ptr, self.context.i64_type().const_int(0, false))
                    .unwrap();

                self.builder.build_unconditional_branch(loop_block).unwrap();

                // Loop condition
                self.builder.position_at_end(loop_block);
                let counter = self
                    .builder
                    .build_load(self.context.i64_type(), counter_ptr, "counter")
                    .unwrap()
                    .into_int_value();
                let cmp = self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::ULT, counter, len_i64, "cmp")
                    .unwrap();
                self.builder
                    .build_conditional_branch(cmp, body_block, after_block)
                    .unwrap();

                // Loop body
                self.builder.position_at_end(body_block);
                let src_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(self.context.i8_type(), str_ptr, &[counter], "src_ptr")
                        .unwrap()
                };
                let dst_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            result_ptr,
                            &[counter],
                            "dst_ptr",
                        )
                        .unwrap()
                };

                let char_val = self
                    .builder
                    .build_load(self.context.i8_type(), src_ptr, "char")
                    .unwrap()
                    .into_int_value();

                // Convert case: if (char >= 'a' && char <= 'z') char -= 32 for upper
                // if (char >= 'A' && char <= 'Z') char += 32 for lower
                let converted = if is_upper {
                    // Check if lowercase letter (97-122)
                    let is_lower_start = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::UGE,
                            char_val,
                            self.context.i8_type().const_int(97, false),
                            "is_lower_start",
                        )
                        .unwrap();
                    let is_lower_end = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::ULE,
                            char_val,
                            self.context.i8_type().const_int(122, false),
                            "is_lower_end",
                        )
                        .unwrap();
                    let is_lower = self
                        .builder
                        .build_and(is_lower_start, is_lower_end, "is_lower")
                        .unwrap();

                    let upper_char = self
                        .builder
                        .build_int_sub(
                            char_val,
                            self.context.i8_type().const_int(32, false),
                            "upper_char",
                        )
                        .unwrap();

                    self.builder
                        .build_select(is_lower, upper_char, char_val, "converted")
                        .unwrap()
                } else {
                    // Check if uppercase letter (65-90)
                    let is_upper_start = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::UGE,
                            char_val,
                            self.context.i8_type().const_int(65, false),
                            "is_upper_start",
                        )
                        .unwrap();
                    let is_upper_end = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::ULE,
                            char_val,
                            self.context.i8_type().const_int(90, false),
                            "is_upper_end",
                        )
                        .unwrap();
                    let is_upper = self
                        .builder
                        .build_and(is_upper_start, is_upper_end, "is_upper")
                        .unwrap();

                    let lower_char = self
                        .builder
                        .build_int_add(
                            char_val,
                            self.context.i8_type().const_int(32, false),
                            "lower_char",
                        )
                        .unwrap();

                    self.builder
                        .build_select(is_upper, lower_char, char_val, "converted")
                        .unwrap()
                };

                self.builder.build_store(dst_ptr, converted).unwrap();

                let next_counter = self
                    .builder
                    .build_int_add(
                        counter,
                        self.context.i64_type().const_int(1, false),
                        "next_counter",
                    )
                    .unwrap();
                self.builder.build_store(counter_ptr, next_counter).unwrap();
                self.builder.build_unconditional_branch(loop_block).unwrap();

                // After loop - null terminate
                self.builder.position_at_end(after_block);
                let null_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            result_ptr,
                            &[len_i64],
                            "null_ptr",
                        )
                        .unwrap()
                };
                self.builder
                    .build_store(null_ptr, self.context.i8_type().const_int(0, false))
                    .unwrap();

                self.temp_values.insert(dest.to_string(), result_ptr.into());
                self.heap_strings.insert(dest.to_string());
                Some(result_ptr.into())
            }
            "contains" => {
                let str_ptr = object_val.into_pointer_value();
                let needle_ptr = self.resolve_value(&args[0]).into_pointer_value();

                let strstr_fn = self.module.get_function("strstr").unwrap_or_else(|| {
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
                    self.module.add_function("strstr", fn_type, None)
                });

                let found_ptr = self
                    .builder
                    .build_call(strstr_fn, &[str_ptr.into(), needle_ptr.into()], "found")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                let is_null = self.builder.build_is_null(found_ptr, "is_null").unwrap();
                let result = self
                    .builder
                    .build_select(
                        is_null,
                        self.context.i32_type().const_int(0, false),
                        self.context.i32_type().const_int(1, false),
                        "result",
                    )
                    .unwrap();

                self.temp_values.insert(dest.to_string(), result);
                self.boolean_temps.insert(dest.to_string());
                Some(result)
            }
            "startsWith" => {
                let str_ptr = object_val.into_pointer_value();
                let prefix_ptr = self.resolve_value(&args[0]).into_pointer_value();

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

                let prefix_len = self
                    .builder
                    .build_call(strlen_fn, &[prefix_ptr.into()], "prefix_len")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                let cmp_result = self
                    .builder
                    .build_call(
                        strncmp_fn,
                        &[str_ptr.into(), prefix_ptr.into(), prefix_len.into()],
                        "cmp_result",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                let is_equal = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        cmp_result,
                        self.context.i32_type().const_int(0, false),
                        "is_equal",
                    )
                    .unwrap();

                let result = self
                    .builder
                    .build_select(
                        is_equal,
                        self.context.i32_type().const_int(1, false),
                        self.context.i32_type().const_int(0, false),
                        "result",
                    )
                    .unwrap();

                self.temp_values.insert(dest.to_string(), result);
                self.boolean_temps.insert(dest.to_string());
                Some(result)
            }
            "endsWith" => {
                let str_ptr = object_val.into_pointer_value();
                let suffix_ptr = self.resolve_value(&args[0]).into_pointer_value();

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

                let str_len = self
                    .builder
                    .build_call(strlen_fn, &[str_ptr.into()], "str_len")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                let suffix_len = self
                    .builder
                    .build_call(strlen_fn, &[suffix_ptr.into()], "suffix_len")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                let cmp_offset = self
                    .builder
                    .build_int_sub(str_len, suffix_len, "offset")
                    .unwrap();

                let start_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            str_ptr,
                            &[cmp_offset],
                            "start_ptr",
                        )
                        .unwrap()
                };

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

                let cmp_result = self
                    .builder
                    .build_call(
                        strncmp_fn,
                        &[start_ptr.into(), suffix_ptr.into(), suffix_len.into()],
                        "cmp_result",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                let is_equal = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        cmp_result,
                        self.context.i32_type().const_int(0, false),
                        "is_equal",
                    )
                    .unwrap();

                let result = self
                    .builder
                    .build_select(
                        is_equal,
                        self.context.i32_type().const_int(1, false),
                        self.context.i32_type().const_int(0, false),
                        "result",
                    )
                    .unwrap();

                self.temp_values.insert(dest.to_string(), result);
                self.boolean_temps.insert(dest.to_string());
                Some(result)
            }
            "trim" => {
                let str_ptr = object_val.into_pointer_value();

                // Get string length
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

                let len_i64 = self
                    .builder
                    .build_call(strlen_fn, &[str_ptr.into()], "strlen")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                // Find start index (skip leading spaces)
                let start_ptr = self
                    .builder
                    .build_alloca(self.context.i64_type(), "start_idx")
                    .unwrap();
                self.builder
                    .build_store(start_ptr, self.context.i64_type().const_int(0, false))
                    .unwrap();

                let current_fn = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let start_loop = self
                    .context
                    .append_basic_block(current_fn, "trim_start_loop");
                let start_body = self
                    .context
                    .append_basic_block(current_fn, "trim_start_body");
                let start_after = self
                    .context
                    .append_basic_block(current_fn, "trim_start_after");

                self.builder.build_unconditional_branch(start_loop).unwrap();

                // Skip leading whitespace
                self.builder.position_at_end(start_loop);
                let start_idx = self
                    .builder
                    .build_load(self.context.i64_type(), start_ptr, "start_idx")
                    .unwrap()
                    .into_int_value();
                let cmp = self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::ULT, start_idx, len_i64, "cmp")
                    .unwrap();
                self.builder
                    .build_conditional_branch(cmp, start_body, start_after)
                    .unwrap();

                self.builder.position_at_end(start_body);
                let char_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            str_ptr,
                            &[start_idx],
                            "char_ptr",
                        )
                        .unwrap()
                };
                let char_val = self
                    .builder
                    .build_load(self.context.i8_type(), char_ptr, "char")
                    .unwrap()
                    .into_int_value();

                // Check if whitespace (space=32, tab=9, newline=10, carriage return=13)
                let is_space = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        char_val,
                        self.context.i8_type().const_int(32, false),
                        "is_space",
                    )
                    .unwrap();
                let is_tab = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        char_val,
                        self.context.i8_type().const_int(9, false),
                        "is_tab",
                    )
                    .unwrap();
                let is_newline = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        char_val,
                        self.context.i8_type().const_int(10, false),
                        "is_newline",
                    )
                    .unwrap();
                let is_cr = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        char_val,
                        self.context.i8_type().const_int(13, false),
                        "is_cr",
                    )
                    .unwrap();

                let is_ws1 = self.builder.build_or(is_space, is_tab, "is_ws1").unwrap();
                let is_ws2 = self.builder.build_or(is_newline, is_cr, "is_ws2").unwrap();
                let is_whitespace = self
                    .builder
                    .build_or(is_ws1, is_ws2, "is_whitespace")
                    .unwrap();

                let continue_trim = self
                    .context
                    .append_basic_block(current_fn, "continue_trim_start");
                self.builder
                    .build_conditional_branch(is_whitespace, continue_trim, start_after)
                    .unwrap();

                self.builder.position_at_end(continue_trim);
                let next_start = self
                    .builder
                    .build_int_add(
                        start_idx,
                        self.context.i64_type().const_int(1, false),
                        "next_start",
                    )
                    .unwrap();
                self.builder.build_store(start_ptr, next_start).unwrap();
                self.builder.build_unconditional_branch(start_loop).unwrap();

                // Find end index (skip trailing spaces)
                self.builder.position_at_end(start_after);
                let final_start = self
                    .builder
                    .build_load(self.context.i64_type(), start_ptr, "final_start")
                    .unwrap()
                    .into_int_value();

                let end_ptr = self
                    .builder
                    .build_alloca(self.context.i64_type(), "end_idx")
                    .unwrap();
                let initial_end = self
                    .builder
                    .build_int_sub(
                        len_i64,
                        self.context.i64_type().const_int(1, false),
                        "initial_end",
                    )
                    .unwrap();
                self.builder.build_store(end_ptr, initial_end).unwrap();

                let end_loop = self.context.append_basic_block(current_fn, "trim_end_loop");
                let end_body = self.context.append_basic_block(current_fn, "trim_end_body");
                let end_after = self
                    .context
                    .append_basic_block(current_fn, "trim_end_after");

                self.builder.build_unconditional_branch(end_loop).unwrap();

                // Skip trailing whitespace
                self.builder.position_at_end(end_loop);
                let end_idx = self
                    .builder
                    .build_load(self.context.i64_type(), end_ptr, "end_idx")
                    .unwrap()
                    .into_int_value();
                let cmp_end = self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::SGE, end_idx, final_start, "cmp_end")
                    .unwrap();
                self.builder
                    .build_conditional_branch(cmp_end, end_body, end_after)
                    .unwrap();

                self.builder.position_at_end(end_body);
                let end_char_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            str_ptr,
                            &[end_idx],
                            "end_char_ptr",
                        )
                        .unwrap()
                };
                let end_char_val = self
                    .builder
                    .build_load(self.context.i8_type(), end_char_ptr, "end_char")
                    .unwrap()
                    .into_int_value();

                let is_space_end = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        end_char_val,
                        self.context.i8_type().const_int(32, false),
                        "is_space_end",
                    )
                    .unwrap();
                let is_tab_end = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        end_char_val,
                        self.context.i8_type().const_int(9, false),
                        "is_tab_end",
                    )
                    .unwrap();
                let is_newline_end = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        end_char_val,
                        self.context.i8_type().const_int(10, false),
                        "is_newline_end",
                    )
                    .unwrap();
                let is_cr_end = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        end_char_val,
                        self.context.i8_type().const_int(13, false),
                        "is_cr_end",
                    )
                    .unwrap();

                let is_ws1_end = self
                    .builder
                    .build_or(is_space_end, is_tab_end, "is_ws1_end")
                    .unwrap();
                let is_ws2_end = self
                    .builder
                    .build_or(is_newline_end, is_cr_end, "is_ws2_end")
                    .unwrap();
                let is_whitespace_end = self
                    .builder
                    .build_or(is_ws1_end, is_ws2_end, "is_whitespace_end")
                    .unwrap();

                let continue_trim_end = self
                    .context
                    .append_basic_block(current_fn, "continue_trim_end");
                self.builder
                    .build_conditional_branch(is_whitespace_end, continue_trim_end, end_after)
                    .unwrap();

                self.builder.position_at_end(continue_trim_end);
                let next_end = self
                    .builder
                    .build_int_sub(
                        end_idx,
                        self.context.i64_type().const_int(1, false),
                        "next_end",
                    )
                    .unwrap();
                self.builder.build_store(end_ptr, next_end).unwrap();
                self.builder.build_unconditional_branch(end_loop).unwrap();

                // Create trimmed string
                self.builder.position_at_end(end_after);
                let final_end = self
                    .builder
                    .build_load(self.context.i64_type(), end_ptr, "final_end")
                    .unwrap()
                    .into_int_value();

                let trimmed_len = self
                    .builder
                    .build_int_sub(final_end, final_start, "trimmed_len_temp")
                    .unwrap();
                let trimmed_len_plus = self
                    .builder
                    .build_int_add(
                        trimmed_len,
                        self.context.i64_type().const_int(1, false),
                        "trimmed_len",
                    )
                    .unwrap();

                let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
                    let fn_type = self
                        .context
                        .ptr_type(inkwell::AddressSpace::default())
                        .fn_type(&[self.context.i64_type().into()], false);
                    self.module.add_function("malloc", fn_type, None)
                });

                // Return empty string - allocate with RC header
                let header_size = self.context.i64_type().const_int(8, false);
                let data_size = self.context.i64_type().const_int(1, false); // just null byte
                let total_size = self
                    .builder
                    .build_int_add(header_size, data_size, "total_size")
                    .unwrap();

                let heap_ptr = self
                    .builder
                    .build_call(malloc_fn, &[total_size.into()], "heap_empty_str")
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

                // Store string length at offset 4 (length = 0)
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
                self.builder
                    .build_store(len_ptr_cast, self.context.i32_type().const_int(0, false))
                    .unwrap();

                // Get data pointer at offset 8
                let result_ptr = unsafe {
                    self.builder
                        .build_gep(
                            self.context.i8_type(),
                            heap_ptr,
                            &[self.context.i32_type().const_int(8, false)],
                            "data_ptr",
                        )
                        .unwrap()
                };

                let _unused = self
                    .builder
                    .build_call(
                        malloc_fn,
                        &[self.context.i64_type().const_int(1, false).into()],
                        "unused_trimmed_str",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                // Copy trimmed portion
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

                let src_start = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            str_ptr,
                            &[final_start],
                            "src_start",
                        )
                        .unwrap()
                };

                self.builder
                    .build_call(
                        memcpy_fn,
                        &[result_ptr.into(), src_start.into(), trimmed_len_plus.into()],
                        "",
                    )
                    .unwrap();

                // Null terminate
                let null_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            result_ptr,
                            &[trimmed_len_plus],
                            "null_ptr",
                        )
                        .unwrap()
                };
                self.builder
                    .build_store(null_ptr, self.context.i8_type().const_int(0, false))
                    .unwrap();

                self.temp_values.insert(dest.to_string(), result_ptr.into());
                self.heap_strings.insert(dest.to_string());
                Some(result_ptr.into())
            }
            "reverse" => {
                let str_ptr = object_val.into_pointer_value();

                // Get string length
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

                let len_i64 = self
                    .builder
                    .build_call(strlen_fn, &[str_ptr.into()], "strlen")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                // Allocate new string
                let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
                    let fn_type = self
                        .context
                        .ptr_type(inkwell::AddressSpace::default())
                        .fn_type(&[self.context.i64_type().into()], false);
                    self.module.add_function("malloc", fn_type, None)
                });

                let len_plus_one = self
                    .builder
                    .build_int_add(
                        len_i64,
                        self.context.i64_type().const_int(1, false),
                        "len_plus_one",
                    )
                    .unwrap();

                let result_ptr = self
                    .builder
                    .build_call(malloc_fn, &[len_plus_one.into()], "reversed_str")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                // Copy in reverse order
                let current_fn = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let loop_block = self.context.append_basic_block(current_fn, "reverse_loop");
                let body_block = self.context.append_basic_block(current_fn, "reverse_body");
                let after_block = self.context.append_basic_block(current_fn, "reverse_after");

                let counter_ptr = self
                    .builder
                    .build_alloca(self.context.i64_type(), "counter")
                    .unwrap();
                self.builder
                    .build_store(counter_ptr, self.context.i64_type().const_int(0, false))
                    .unwrap();

                self.builder.build_unconditional_branch(loop_block).unwrap();

                // Loop condition
                self.builder.position_at_end(loop_block);
                let counter = self
                    .builder
                    .build_load(self.context.i64_type(), counter_ptr, "counter")
                    .unwrap()
                    .into_int_value();
                let cmp = self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::ULT, counter, len_i64, "cmp")
                    .unwrap();
                self.builder
                    .build_conditional_branch(cmp, body_block, after_block)
                    .unwrap();

                // Loop body
                self.builder.position_at_end(body_block);

                // Source index = len - 1 - counter
                let src_idx = self
                    .builder
                    .build_int_sub(
                        self.builder
                            .build_int_sub(
                                len_i64,
                                self.context.i64_type().const_int(1, false),
                                "len_minus_1",
                            )
                            .unwrap(),
                        counter,
                        "src_idx",
                    )
                    .unwrap();

                let src_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(self.context.i8_type(), str_ptr, &[src_idx], "src_ptr")
                        .unwrap()
                };
                let dst_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            result_ptr,
                            &[counter],
                            "dst_ptr",
                        )
                        .unwrap()
                };

                let char_val = self
                    .builder
                    .build_load(self.context.i8_type(), src_ptr, "char")
                    .unwrap();

                self.builder.build_store(dst_ptr, char_val).unwrap();

                let next_counter = self
                    .builder
                    .build_int_add(
                        counter,
                        self.context.i64_type().const_int(1, false),
                        "next_counter",
                    )
                    .unwrap();
                self.builder.build_store(counter_ptr, next_counter).unwrap();
                self.builder.build_unconditional_branch(loop_block).unwrap();

                // After loop - null terminate
                self.builder.position_at_end(after_block);
                let null_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            result_ptr,
                            &[len_i64],
                            "null_ptr",
                        )
                        .unwrap()
                };
                self.builder
                    .build_store(null_ptr, self.context.i8_type().const_int(0, false))
                    .unwrap();

                self.temp_values.insert(dest.to_string(), result_ptr.into());
                self.heap_strings.insert(dest.to_string());
                Some(result_ptr.into())
            }
            "indexOf" => {
                let str_ptr = object_val.into_pointer_value();
                let needle_ptr = self.resolve_value(&args[0]).into_pointer_value();

                let strstr_fn = self.module.get_function("strstr").unwrap_or_else(|| {
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
                    self.module.add_function("strstr", fn_type, None)
                });

                let found_ptr = self
                    .builder
                    .build_call(strstr_fn, &[str_ptr.into(), needle_ptr.into()], "found")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                let is_null = self.builder.build_is_null(found_ptr, "is_null").unwrap();
                let index = self
                    .builder
                    .build_ptr_diff(self.context.i8_type(), found_ptr, str_ptr, "index")
                    .unwrap();
                let index_i32 = self
                    .builder
                    .build_int_truncate(index, self.context.i32_type(), "index_i32")
                    .unwrap();
                let result = self
                    .builder
                    .build_select(
                        is_null,
                        self.context.i32_type().const_int((-1_i32) as u64, true),
                        index_i32,
                        "result",
                    )
                    .unwrap();

                self.temp_values.insert(dest.to_string(), result);
                Some(result)
            }
            "replace" => {
                let str_ptr = object_val.into_pointer_value();
                let old_str_ptr = self.resolve_value(&args[0]).into_pointer_value();
                let new_str_ptr = self.resolve_value(&args[1]).into_pointer_value();

                // Get lengths
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

                let str_len = self
                    .builder
                    .build_call(strlen_fn, &[str_ptr.into()], "str_len")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                let old_len = self
                    .builder
                    .build_call(strlen_fn, &[old_str_ptr.into()], "old_len")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                let new_len = self
                    .builder
                    .build_call(strlen_fn, &[new_str_ptr.into()], "new_len")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                // Allocate result string (worst case: replace makes string longer)
                let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
                    let fn_type = self
                        .context
                        .ptr_type(inkwell::AddressSpace::default())
                        .fn_type(&[self.context.i64_type().into()], false);
                    self.module.add_function("malloc", fn_type, None)
                });

                // Allocate generous size for result
                let result_size = self
                    .builder
                    .build_int_mul(
                        str_len,
                        self.context.i64_type().const_int(2, false),
                        "result_size_temp",
                    )
                    .unwrap();
                let result_size_plus = self
                    .builder
                    .build_int_add(
                        result_size,
                        self.context.i64_type().const_int(100, false),
                        "result_size",
                    )
                    .unwrap();

                let result_ptr = self
                    .builder
                    .build_call(malloc_fn, &[result_size_plus.into()], "replace_result")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                // Replace all occurrences by looping through the string
                let current_fn = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();

                // Loop to handle all replacements
                let loop_start = self
                    .context
                    .append_basic_block(current_fn, "replace_loop_start");
                let loop_body = self
                    .context
                    .append_basic_block(current_fn, "replace_loop_body");
                let _ = self
                    .context
                    .append_basic_block(current_fn, "replace_loop_copy");
                let _ = self
                    .context
                    .append_basic_block(current_fn, "replace_loop_next");
                let after_block = self.context.append_basic_block(current_fn, "replace_after");

                // Allocate counters: src_pos, dest_pos
                let src_pos_ptr = self
                    .builder
                    .build_alloca(self.context.i64_type(), "src_pos")
                    .unwrap();
                let dest_pos_ptr = self
                    .builder
                    .build_alloca(self.context.i64_type(), "dest_pos")
                    .unwrap();

                self.builder
                    .build_store(src_pos_ptr, self.context.i64_type().const_zero())
                    .unwrap();
                self.builder
                    .build_store(dest_pos_ptr, self.context.i64_type().const_zero())
                    .unwrap();

                self.builder.build_unconditional_branch(loop_start).unwrap();

                // Loop start: check if src_pos < str_len
                self.builder.position_at_end(loop_start);
                let src_pos = self
                    .builder
                    .build_load(self.context.i64_type(), src_pos_ptr, "src_pos")
                    .unwrap()
                    .into_int_value();
                let cmp = self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::ULT, src_pos, str_len, "cmp_src_pos")
                    .unwrap();
                self.builder
                    .build_conditional_branch(cmp, loop_body, after_block)
                    .unwrap();

                // Loop body: check if substring matches at current position
                self.builder.position_at_end(loop_body);

                // First check if there are enough characters left to match
                let remaining_len = self
                    .builder
                    .build_int_sub(str_len, src_pos, "remaining_len")
                    .unwrap();
                let has_enough = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::UGE,
                        remaining_len,
                        old_len,
                        "has_enough",
                    )
                    .unwrap();

                let can_match_block = self
                    .context
                    .append_basic_block(current_fn, "replace_can_match");
                let no_match_block = self
                    .context
                    .append_basic_block(current_fn, "replace_no_match");

                self.builder
                    .build_conditional_branch(has_enough, can_match_block, no_match_block)
                    .unwrap();

                // Can match block: perform strncmp
                self.builder.position_at_end(can_match_block);

                // Get strncmp for substring comparison
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

                let current_src = unsafe {
                    self.builder
                        .build_gep(self.context.i8_type(), str_ptr, &[src_pos], "current_src")
                        .unwrap()
                };

                let cmp_result = self
                    .builder
                    .build_call(
                        strncmp_fn,
                        &[current_src.into(), old_str_ptr.into(), old_len.into()],
                        "cmp_result",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                let zero = self.context.i32_type().const_zero();
                let is_match = self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::EQ, cmp_result, zero, "is_match")
                    .unwrap();

                let match_block = self.context.append_basic_block(current_fn, "replace_match");

                self.builder
                    .build_conditional_branch(is_match, match_block, no_match_block)
                    .unwrap();

                // Match: copy replacement and advance src by old_len
                self.builder.position_at_end(match_block);
                let dest_pos_m = self
                    .builder
                    .build_load(self.context.i64_type(), dest_pos_ptr, "dest_pos_m")
                    .unwrap()
                    .into_int_value();
                let dest_ptr_m = unsafe {
                    self.builder
                        .build_gep(
                            self.context.i8_type(),
                            result_ptr,
                            &[dest_pos_m],
                            "dest_ptr_m",
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

                self.builder
                    .build_call(
                        memcpy_fn,
                        &[dest_ptr_m.into(), new_str_ptr.into(), new_len.into()],
                        "",
                    )
                    .unwrap();

                let new_dest_pos = self
                    .builder
                    .build_int_add(dest_pos_m, new_len, "new_dest_pos")
                    .unwrap();
                self.builder
                    .build_store(dest_pos_ptr, new_dest_pos)
                    .unwrap();

                let new_src_pos = self
                    .builder
                    .build_int_add(src_pos, old_len, "new_src_pos")
                    .unwrap();
                self.builder.build_store(src_pos_ptr, new_src_pos).unwrap();
                self.builder.build_unconditional_branch(loop_start).unwrap();

                // No match: copy single character
                self.builder.position_at_end(no_match_block);
                let src_pos_nm = self
                    .builder
                    .build_load(self.context.i64_type(), src_pos_ptr, "src_pos_nm")
                    .unwrap()
                    .into_int_value();
                let dest_pos_nm = self
                    .builder
                    .build_load(self.context.i64_type(), dest_pos_ptr, "dest_pos_nm")
                    .unwrap()
                    .into_int_value();
                let src_char_ptr = unsafe {
                    self.builder
                        .build_gep(
                            self.context.i8_type(),
                            str_ptr,
                            &[src_pos_nm],
                            "src_char_ptr",
                        )
                        .unwrap()
                };
                let dest_char_ptr = unsafe {
                    self.builder
                        .build_gep(
                            self.context.i8_type(),
                            result_ptr,
                            &[dest_pos_nm],
                            "dest_char_ptr",
                        )
                        .unwrap()
                };

                let src_char = self
                    .builder
                    .build_load(self.context.i8_type(), src_char_ptr, "src_char")
                    .unwrap();
                self.builder.build_store(dest_char_ptr, src_char).unwrap();

                let new_dest_pos_nm = self
                    .builder
                    .build_int_add(
                        dest_pos_nm,
                        self.context.i64_type().const_int(1, false),
                        "new_dest_pos_nm",
                    )
                    .unwrap();
                self.builder
                    .build_store(dest_pos_ptr, new_dest_pos_nm)
                    .unwrap();

                let new_src_pos_nm = self
                    .builder
                    .build_int_add(
                        src_pos_nm,
                        self.context.i64_type().const_int(1, false),
                        "new_src_pos_nm",
                    )
                    .unwrap();
                self.builder
                    .build_store(src_pos_ptr, new_src_pos_nm)
                    .unwrap();
                self.builder.build_unconditional_branch(loop_start).unwrap();

                // After loop: null terminate
                self.builder.position_at_end(after_block);
                let final_dest_pos = self
                    .builder
                    .build_load(self.context.i64_type(), dest_pos_ptr, "final_dest_pos")
                    .unwrap()
                    .into_int_value();
                let null_pos = unsafe {
                    self.builder
                        .build_gep(
                            self.context.i8_type(),
                            result_ptr,
                            &[final_dest_pos],
                            "null_pos",
                        )
                        .unwrap()
                };
                self.builder
                    .build_store(null_pos, self.context.i8_type().const_int(0, false))
                    .unwrap();

                self.temp_values.insert(dest.to_string(), result_ptr.into());
                self.heap_strings.insert(dest.to_string());
                Some(result_ptr.into())
            }
            "repeat" => {
                let str_ptr = object_val.into_pointer_value();
                let count_val = self.resolve_value(&args[0]).into_int_value();

                // Get string length
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

                let str_len_i64 = self
                    .builder
                    .build_call(strlen_fn, &[str_ptr.into()], "str_len")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                // Convert count to i64
                let count_i64 = self
                    .builder
                    .build_int_z_extend(count_val, self.context.i64_type(), "count_i64")
                    .unwrap();

                // Calculate total length: str_len * count
                let total_len = self
                    .builder
                    .build_int_mul(str_len_i64, count_i64, "total_len")
                    .unwrap();

                // Allocate memory: total_len + 1 (for null terminator)
                let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
                    let fn_type = self
                        .context
                        .ptr_type(inkwell::AddressSpace::default())
                        .fn_type(&[self.context.i64_type().into()], false);
                    self.module.add_function("malloc", fn_type, None)
                });

                let alloc_size = self
                    .builder
                    .build_int_add(
                        total_len,
                        self.context.i64_type().const_int(1, false),
                        "alloc_size",
                    )
                    .unwrap();

                let result_ptr = self
                    .builder
                    .build_call(malloc_fn, &[alloc_size.into()], "repeat_result")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_pointer_value();

                // Loop to copy string count times
                let current_fn = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();

                let loop_start = self
                    .context
                    .append_basic_block(current_fn, "repeat_loop_start");
                let loop_body = self
                    .context
                    .append_basic_block(current_fn, "repeat_loop_body");
                let after_block = self.context.append_basic_block(current_fn, "repeat_after");

                // Allocate counter
                let counter_ptr = self
                    .builder
                    .build_alloca(self.context.i64_type(), "counter")
                    .unwrap();
                self.builder
                    .build_store(counter_ptr, self.context.i64_type().const_zero())
                    .unwrap();

                self.builder.build_unconditional_branch(loop_start).unwrap();

                // Loop start: check counter < count
                self.builder.position_at_end(loop_start);
                let counter = self
                    .builder
                    .build_load(self.context.i64_type(), counter_ptr, "counter")
                    .unwrap()
                    .into_int_value();
                let cmp = self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::ULT, counter, count_i64, "cmp")
                    .unwrap();
                self.builder
                    .build_conditional_branch(cmp, loop_body, after_block)
                    .unwrap();

                // Loop body: copy string to result at offset counter * str_len
                self.builder.position_at_end(loop_body);
                let offset = self
                    .builder
                    .build_int_mul(counter, str_len_i64, "offset")
                    .unwrap();
                let dest_ptr = unsafe {
                    self.builder
                        .build_gep(self.context.i8_type(), result_ptr, &[offset], "dest_ptr")
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

                self.builder
                    .build_call(
                        memcpy_fn,
                        &[dest_ptr.into(), str_ptr.into(), str_len_i64.into()],
                        "",
                    )
                    .unwrap();

                // Increment counter
                let new_counter = self
                    .builder
                    .build_int_add(
                        counter,
                        self.context.i64_type().const_int(1, false),
                        "new_counter",
                    )
                    .unwrap();
                self.builder.build_store(counter_ptr, new_counter).unwrap();
                self.builder.build_unconditional_branch(loop_start).unwrap();

                // After loop: null terminate
                self.builder.position_at_end(after_block);
                let null_pos = unsafe {
                    self.builder
                        .build_gep(self.context.i8_type(), result_ptr, &[total_len], "null_pos")
                        .unwrap()
                };
                self.builder
                    .build_store(null_pos, self.context.i8_type().const_int(0, false))
                    .unwrap();

                self.temp_values.insert(dest.to_string(), result_ptr.into());
                self.heap_strings.insert(dest.to_string());
                Some(result_ptr.into())
            }
            "concat" => self.generate_string_concat(dest, _object, &args[0]),
            "charCode" => {
                // Get the ASCII/Unicode code of the first character in the string
                let str_ptr = object_val.into_pointer_value();

                // Load first byte (character)
                let first_char = self
                    .builder
                    .build_load(self.context.i8_type(), str_ptr, "first_char")
                    .unwrap()
                    .into_int_value();

                // Convert i8 to i32 (sign extend to preserve ASCII values)
                let char_code = self
                    .builder
                    .build_int_cast(first_char, self.context.i32_type(), "char_code")
                    .unwrap();

                self.temp_values.insert(dest.to_string(), char_code.into());
                Some(char_code.into())
            }
            "countSubstr" => {
                let str_ptr = object_val.into_pointer_value();
                let substr_ptr = self.resolve_value(&args[0]).into_pointer_value();

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

                let str_len = self
                    .builder
                    .build_call(strlen_fn, &[str_ptr.into()], "str_len")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                let substr_len = self
                    .builder
                    .build_call(strlen_fn, &[substr_ptr.into()], "substr_len")
                    .unwrap()
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                // If substr is empty, return 0
                let current_fn = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();

                let empty_block = self
                    .context
                    .append_basic_block(current_fn, "countsubstr_empty");
                let count_block = self
                    .context
                    .append_basic_block(current_fn, "countsubstr_count");
                let after_block = self
                    .context
                    .append_basic_block(current_fn, "countsubstr_after");

                let is_empty = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        substr_len,
                        self.context.i64_type().const_zero(),
                        "is_empty",
                    )
                    .unwrap();

                self.builder
                    .build_conditional_branch(is_empty, empty_block, count_block)
                    .unwrap();

                // Empty substr
                self.builder.position_at_end(empty_block);
                let zero_count = self.context.i32_type().const_zero();
                self.builder
                    .build_unconditional_branch(after_block)
                    .unwrap();

                // Count occurrences
                self.builder.position_at_end(count_block);

                let count_ptr = self
                    .builder
                    .build_alloca(self.context.i32_type(), "count")
                    .unwrap();
                self.builder
                    .build_store(count_ptr, self.context.i32_type().const_zero())
                    .unwrap();

                let i_ptr = self
                    .builder
                    .build_alloca(self.context.i64_type(), "i")
                    .unwrap();
                self.builder
                    .build_store(i_ptr, self.context.i64_type().const_zero())
                    .unwrap();

                let loop_start = self
                    .context
                    .append_basic_block(current_fn, "countsubstr_loop_start");
                let loop_body = self
                    .context
                    .append_basic_block(current_fn, "countsubstr_loop_body");
                let check_match = self
                    .context
                    .append_basic_block(current_fn, "countsubstr_check_match");
                let match_found = self
                    .context
                    .append_basic_block(current_fn, "countsubstr_match_found");
                let no_match = self
                    .context
                    .append_basic_block(current_fn, "countsubstr_no_match");
                let loop_end = self
                    .context
                    .append_basic_block(current_fn, "countsubstr_loop_end");

                self.builder.build_unconditional_branch(loop_start).unwrap();

                self.builder.position_at_end(loop_start);
                let i = self
                    .builder
                    .build_load(self.context.i64_type(), i_ptr, "i")
                    .unwrap()
                    .into_int_value();
                let cmp = self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::ULT, i, str_len, "cmp")
                    .unwrap();
                self.builder
                    .build_conditional_branch(cmp, loop_body, loop_end)
                    .unwrap();

                self.builder.position_at_end(loop_body);
                let remaining = self.builder.build_int_sub(str_len, i, "remaining").unwrap();
                let has_enough = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::UGE,
                        remaining,
                        substr_len,
                        "has_enough",
                    )
                    .unwrap();

                self.builder
                    .build_conditional_branch(has_enough, check_match, no_match)
                    .unwrap();

                self.builder.position_at_end(check_match);
                let current_pos = unsafe {
                    self.builder
                        .build_gep(self.context.i8_type(), str_ptr, &[i], "current_pos")
                        .unwrap()
                };

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

                let cmp_result = self
                    .builder
                    .build_call(
                        strncmp_fn,
                        &[current_pos.into(), substr_ptr.into(), substr_len.into()],
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
                        self.context.i32_type().const_zero(),
                        "is_match",
                    )
                    .unwrap();

                self.builder
                    .build_conditional_branch(is_match, match_found, no_match)
                    .unwrap();

                self.builder.position_at_end(match_found);
                let count = self
                    .builder
                    .build_load(self.context.i32_type(), count_ptr, "count")
                    .unwrap()
                    .into_int_value();
                let new_count = self
                    .builder
                    .build_int_add(
                        count,
                        self.context.i32_type().const_int(1, false),
                        "new_count",
                    )
                    .unwrap();
                self.builder.build_store(count_ptr, new_count).unwrap();

                let substr_len_i64 = substr_len;
                let new_i_match = self
                    .builder
                    .build_int_add(i, substr_len_i64, "new_i_match")
                    .unwrap();
                self.builder.build_store(i_ptr, new_i_match).unwrap();
                self.builder.build_unconditional_branch(loop_start).unwrap();

                self.builder.position_at_end(no_match);
                let new_i_no = self
                    .builder
                    .build_int_add(i, self.context.i64_type().const_int(1, false), "new_i_no")
                    .unwrap();
                self.builder.build_store(i_ptr, new_i_no).unwrap();
                self.builder.build_unconditional_branch(loop_start).unwrap();

                self.builder.position_at_end(loop_end);
                let final_count = self
                    .builder
                    .build_load(self.context.i32_type(), count_ptr, "final_count")
                    .unwrap()
                    .into_int_value();
                self.builder
                    .build_unconditional_branch(after_block)
                    .unwrap();

                // After block
                self.builder.position_at_end(after_block);
                let result_phi = self
                    .builder
                    .build_phi(self.context.i32_type(), "result_phi")
                    .unwrap();

                result_phi.add_incoming(&[(&zero_count, empty_block), (&final_count, loop_end)]);

                let result = result_phi.as_basic_value().into_int_value();
                self.temp_values.insert(dest.to_string(), result.into());
                Some(result.into())
            }
            _ => None,
        }
    }
}
