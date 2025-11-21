use crate::codegen::core::CodeGen;
use inkwell::values::BasicValueEnum;

impl<'ctx> CodeGen<'ctx> {
    pub fn generate_file_method(
        &mut self,
        dest: &str,
        method: &str,
        args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        match method {
            "read" => self.generate_file_read(dest, args),
            "write" => self.generate_file_write(dest, args),
            "append" => self.generate_file_append(dest, args),
            "exists" => self.generate_file_exists(dest, args),
            "delete" => self.generate_file_delete(dest, args),
            "list" => self.generate_file_list(dest, args),
            "mkdir" => self.generate_file_mkdir(dest, args),
            "readLines" => self.generate_file_read_lines(dest, args),
            _ => None,
        }
    }

    fn generate_file_read(&mut self, dest: &str, args: &[String]) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }

        let path_val = self.resolve_value(&args[0]);
        let path_ptr = path_val.into_pointer_value();

        // Declare/get file_read function
        let file_read_fn = self.module.get_function("file_read").unwrap_or_else(|| {
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
            self.module.add_function("file_read", fn_type, None)
        });

        // Call file_read(path)
        let result_ptr = self
            .builder
            .build_call(file_read_fn, &[path_ptr.into()], "file_read_result")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Wrap result in RC-counted heap string
        let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
            let fn_type = self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .fn_type(&[self.context.i64_type().into()], false);
            self.module.add_function("malloc", fn_type, None)
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

        let str_len = self
            .builder
            .build_call(strlen_fn, &[result_ptr.into()], "content_len")
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
            .build_call(malloc_fn, &[total_size.into()], "heap_file_content")
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

        // Free original result
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

        self.temp_values.insert(dest.to_string(), data_ptr.into());
        self.heap_strings.insert(dest.to_string());

        Some(data_ptr.into())
    }

    fn generate_file_write(&mut self, dest: &str, args: &[String]) -> Option<BasicValueEnum<'ctx>> {
        if args.len() < 2 {
            return None;
        }

        let path_val = self.resolve_value(&args[0]);
        let content_val = self.resolve_value(&args[1]);
        let path_ptr = path_val.into_pointer_value();
        let content_ptr = content_val.into_pointer_value();

        // Declare/get file_write function
        let file_write_fn = self.module.get_function("file_write").unwrap_or_else(|| {
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
            self.module.add_function("file_write", fn_type, None)
        });

        // Call file_write(path, content)
        let result = self
            .builder
            .build_call(
                file_write_fn,
                &[path_ptr.into(), content_ptr.into()],
                "file_write_result",
            )
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        self.temp_values.insert(dest.to_string(), result.into());

        Some(result.into())
    }

    fn generate_file_append(
        &mut self,
        dest: &str,
        args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.len() < 2 {
            return None;
        }

        let path_val = self.resolve_value(&args[0]);
        let content_val = self.resolve_value(&args[1]);
        let path_ptr = path_val.into_pointer_value();
        let content_ptr = content_val.into_pointer_value();

        // Declare/get file_append function
        let file_append_fn = self.module.get_function("file_append").unwrap_or_else(|| {
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
            self.module.add_function("file_append", fn_type, None)
        });

        // Call file_append(path, content)
        let result = self
            .builder
            .build_call(
                file_append_fn,
                &[path_ptr.into(), content_ptr.into()],
                "file_append_result",
            )
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        self.temp_values.insert(dest.to_string(), result.into());

        Some(result.into())
    }

    fn generate_file_exists(
        &mut self,
        dest: &str,
        args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }

        let path_val = self.resolve_value(&args[0]);
        let path_ptr = path_val.into_pointer_value();

        // Declare/get file_exists function
        let file_exists_fn = self.module.get_function("file_exists").unwrap_or_else(|| {
            let fn_type = self.context.i32_type().fn_type(
                &[self
                    .context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into()],
                false,
            );
            self.module.add_function("file_exists", fn_type, None)
        });

        // Call file_exists(path)
        let result = self
            .builder
            .build_call(file_exists_fn, &[path_ptr.into()], "file_exists_result")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        self.temp_values.insert(dest.to_string(), result.into());
        self.boolean_temps.insert(dest.to_string());

        Some(result.into())
    }

    fn generate_file_delete(
        &mut self,
        dest: &str,
        args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }

        let path_val = self.resolve_value(&args[0]);
        let path_ptr = path_val.into_pointer_value();

        // Declare/get file_delete function
        let file_delete_fn = self.module.get_function("file_delete").unwrap_or_else(|| {
            let fn_type = self.context.i32_type().fn_type(
                &[self
                    .context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into()],
                false,
            );
            self.module.add_function("file_delete", fn_type, None)
        });

        // Call file_delete(path)
        let result = self
            .builder
            .build_call(file_delete_fn, &[path_ptr.into()], "file_delete_result")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        self.temp_values.insert(dest.to_string(), result.into());

        Some(result.into())
    }

    fn generate_file_mkdir(&mut self, dest: &str, args: &[String]) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }

        let path_val = self.resolve_value(&args[0]);
        let path_ptr = path_val.into_pointer_value();

        // Declare/get file_mkdir function
        let file_mkdir_fn = self.module.get_function("file_mkdir").unwrap_or_else(|| {
            let fn_type = self.context.i32_type().fn_type(
                &[self
                    .context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into()],
                false,
            );
            self.module.add_function("file_mkdir", fn_type, None)
        });

        // Call file_mkdir(path)
        let result = self
            .builder
            .build_call(file_mkdir_fn, &[path_ptr.into()], "file_mkdir_result")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();

        self.temp_values.insert(dest.to_string(), result.into());

        Some(result.into())
    }

    fn generate_file_list(&mut self, dest: &str, args: &[String]) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }

        let path_val = self.resolve_value(&args[0]);
        let path_ptr = path_val.into_pointer_value();

        // Declare/get file_list function
        let file_list_fn = self.module.get_function("file_list").unwrap_or_else(|| {
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
            self.module.add_function("file_list", fn_type, None)
        });

        // Call file_list(path) - returns newline-separated string
        let result_ptr = self
            .builder
            .build_call(file_list_fn, &[path_ptr.into()], "file_list_result")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Convert newline-separated string to array
        // For now, we'll wrap it as a string and let the runtime handle it
        // In a full implementation, we'd parse the string and create an array

        // Wrap in RC-counted string
        self.temp_values.insert(dest.to_string(), result_ptr.into());
        self.heap_strings.insert(dest.to_string());

        Some(result_ptr.into())
    }

    fn generate_file_read_lines(
        &mut self,
        dest: &str,
        args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        if args.is_empty() {
            return None;
        }

        let path_val = self.resolve_value(&args[0]);
        let path_ptr = path_val.into_pointer_value();

        // Declare/get file_read_lines function
        let file_read_lines_fn = self
            .module
            .get_function("file_read_lines")
            .unwrap_or_else(|| {
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
                self.module.add_function("file_read_lines", fn_type, None)
            });

        // Call file_read_lines(path) - returns delimiter-separated string
        let result_ptr = self
            .builder
            .build_call(
                file_read_lines_fn,
                &[path_ptr.into()],
                "file_read_lines_result",
            )
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Similar to file_list, wrap as string for now
        self.temp_values.insert(dest.to_string(), result_ptr.into());
        self.heap_strings.insert(dest.to_string());

        Some(result_ptr.into())
    }
}
