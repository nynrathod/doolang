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
        // Check for JSON builtin FIRST before trying to resolve the object
        // This prevents panic when "JSON" is passed as the object (it's not a real variable)
        if object == "JSON" {
            return self.generate_json_method(dest, method, args);
        }

        // Check for Server auth() and crud() methods - route to database codegen
        if method == "auth" && args.len() == 4 {
            // app.auth(signupPath, loginPath, UserStruct, db)
            return self.generate_auth_routes(dest, object, args);
        } else if method == "crud" && args.len() == 3 {
            // app.crud(basePath, ResourceStruct, db)
            return self.generate_crud_routes(dest, object, args);
        }

        // First, check if this is a custom user-defined method
        // Try to determine the type name from the object
        let object_val = self.resolve_value(object);
        // Determine type name for method lookup
        // IMPORTANT: Check struct_instance_types FIRST before heap_arrays
        // because heap_arrays is reused to track both arrays AND structs for RC
        let type_name = if let Some(struct_type) =
            self.struct_instance_types.get(object).cloned().or_else(|| {
                // Try without % prefix
                if object.starts_with('%') {
                    self.struct_instance_types
                        .get(&object.trim_start_matches('%').to_string())
                        .cloned()
                } else {
                    // Try with % prefix
                    self.struct_instance_types
                        .get(&format!("%{}", object))
                        .cloned()
                }
            }) {
            // Found in struct_instance_types - this is a struct
            Some(struct_type)
        } else if let Some(var_type) = self.variable_types.get(object).cloned().or_else(|| {
            // Try without % prefix
            if object.starts_with('%') {
                self.variable_types
                    .get(&object.trim_start_matches('%').to_string())
                    .cloned()
            } else {
                // Try with % prefix
                self.variable_types.get(&format!("%{}", object)).cloned()
            }
        }) {
            // Check if variable_types has a struct type (not a primitive)
            // This handles cases where StructGet result is tracked in variable_types but not struct_instance_types
            if !var_type.starts_with("Array")
                && !var_type.starts_with("Map")
                && var_type != "Int"
                && var_type != "Float"
                && var_type != "Bool"
                && var_type != "Str"
                && var_type != "String"
                && !var_type.is_empty()
            {
                // This is likely a struct type name
                Some(var_type)
            } else {
                None
            }
        } else if self.array_metadata.contains_key(object) {
            // Has array metadata - definitely an array
            if let Some(metadata) = self.array_metadata.get(object) {
                Some(format!("Array({})", metadata.element_type))
            } else {
                Some("Array".to_string())
            }
        } else if self.map_metadata.contains_key(object) {
            // Has map metadata - definitely a map
            if let Some(metadata) = self.map_metadata.get(object) {
                Some(format!(
                    "Map({},{})",
                    metadata.key_type, metadata.value_type
                ))
            } else {
                Some("Map".to_string())
            }
        } else if self.heap_strings.contains(object) || self.temp_strings.contains_key(object) {
            Some("Str".to_string())
        } else if object_val.is_int_value() {
            Some("Int".to_string())
        } else if object_val.is_float_value() {
            Some("Float".to_string())
        } else if object_val.is_pointer_value() {
            // Generic pointer - could be string or unknown struct
            Some("Str".to_string())
        } else {
            None
        };

        // Check if there's a custom method defined (format: Type::method)
        if let Some(ref type_str) = type_name {
            // Extract struct name from "Struct(Name)" format if present
            let clean_type = if type_str.starts_with("Struct(") && type_str.ends_with(")") {
                &type_str[7..type_str.len() - 1]
            } else {
                type_str.as_str()
            };

            let mangled_method_name = format!("{}::{}", clean_type, method);

            // Check function aliases first (for FFI functions)
            let actual_func_name = self
                .function_aliases
                .get(&mangled_method_name)
                .cloned()
                .unwrap_or_else(|| mangled_method_name.clone());

            if let Some(_func) = self.module.get_function(&actual_func_name) {
                // This is a custom user-defined method
                // Call it as a regular function with object as first argument
                let mut all_args = vec![object.to_string()];
                all_args.extend_from_slice(args);
                let dest_vec = vec![dest.to_string()];
                return self.generate_call(&dest_vec, &mangled_method_name, all_args.as_slice());
            } else {
                // Method was expected but not found - this is likely a struct method
                // Check if it's a struct type and provide helpful error
                if !clean_type.starts_with("Array")
                    && !clean_type.starts_with("Map")
                    && clean_type != "Int"
                    && clean_type != "Float"
                    && clean_type != "Bool"
                    && clean_type != "Str"
                {
                    for func in self.module.get_functions() {
                        eprintln!("  - {}", func.get_name().to_str().unwrap());
                    }
                    panic!(
                        "Method '{}::{}' was not generated - check MIR generation",
                        clean_type, method
                    );
                }
                // For primitive types, fall through to built-in methods below
            }
        }

        // Note: JSON builtin check is now at the top of this function

        // Fall back to built-in methods
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
