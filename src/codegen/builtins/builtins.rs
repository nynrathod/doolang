use crate::codegen::core::CodeGen;

use inkwell::values::BasicValueEnum;

impl<'ctx> CodeGen<'ctx> {
    pub fn generate_method_call(
        &mut self,
        dest: &str,
        object: &str,
        method: &str,
        args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        let object_val = self.resolve_value(object);

        // Check arrays and maps BEFORE strings, since they are also pointer types
        if self.heap_arrays.contains(object) || self.array_metadata.contains_key(object) {
            self.generate_array_method(dest, object, object_val, method, args)
        } else if self.heap_maps.contains(object) || self.map_metadata.contains_key(object) {
            self.generate_map_method(dest, object, object_val, method, args)
        } else if self.heap_strings.contains(object)
            || self.temp_strings.contains_key(object)
            || object_val.is_pointer_value()
        {
            self.generate_string_method(dest, object, object_val, method, args)
        } else if object_val.is_int_value() {
            self.generate_int_method(dest, object, object_val, method, args)
        } else {
            None
        }
    }

    pub fn generate_int_method(
        &mut self,
        dest: &str,
        _object: &str,
        object_val: BasicValueEnum<'ctx>,
        method: &str,
        _args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        match method {
            "toChar" => {
                // Convert Int (ASCII code) to String (single character)
                let char_code = object_val.into_int_value();

                let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
                    let fn_type = self
                        .context
                        .i8_type()
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
                            self.context.i32_type(),
                            heap_ptr,
                            &[self.context.i32_type().const_int(1, false)],
                            "len_ptr",
                        )
                        .unwrap()
                };
                self.builder
                    .build_store(len_ptr, self.context.i32_type().const_int(1, false))
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

                // Store the character (truncate i32 to i8)
                let char_i8 = self
                    .builder
                    .build_int_truncate(char_code, self.context.i8_type(), "char_i8")
                    .unwrap();
                self.builder.build_store(result_ptr, char_i8).unwrap();

                // Store null terminator
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
                self.builder
                    .build_store(null_ptr, self.context.i8_type().const_int(0, false))
                    .unwrap();

                self.temp_values.insert(dest.to_string(), heap_ptr.into());
                self.heap_strings.insert(dest.to_string());
                Some(heap_ptr.into())
            }
            _ => None,
        }
    }
}
