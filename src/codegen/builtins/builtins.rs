use crate::codegen::core::CodeGen;

use inkwell::values::{BasicValue, BasicValueEnum};

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

        // Check for Database static methods
        if object == "Database" {
            if method == "get" {
                // Database::get() - retrieve global DB instance
                return self.generate_database_get(dest);
            }
            // For other Database methods like postgres(), fall through to normal call handling
        }

        // Check for Database instance methods that need type-aware handling
        if method == "raw" || method == "rawWithParams" {
            // db.raw() / db.rawWithParams() - auto-deserialize JSON into typed arrays based on return type
            return self.generate_database_raw_typed(dest, object, method, args);
        }

        // Check for Server methods: auth(), crud(), cors(), ratelimit()
        if method == "auth" && args.len() == 4 {
            // app.auth(signupPath, loginPath, UserStruct, db)
            return self.generate_auth_routes(dest, object, args);
        } else if method == "crud" && args.len() == 3 {
            // app.crud(basePath, ResourceStruct, db)
            return self.generate_crud_routes(dest, object, args);
        } else if method == "cors" {
            // app.cors() or app.cors(origins: "...", methods: "...", credentials: true)
            return self.generate_cors_config(dest, object, args);
        } else if method == "ratelimit" {
            // app.ratelimit() or app.ratelimit(max: 500, window: 3600, per: "user")
            return self.generate_ratelimit_config(dest, object, args);
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

    /// Generate call to doo_db_get_global FFI function
    /// Returns the global database instance that was initialized via Database::postgres()
    fn generate_database_get(&mut self, dest: &str) -> Option<BasicValueEnum<'ctx>> {
        use inkwell::AddressSpace;

        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Declare doo_db_get_global FFI function if not already declared
        let get_global_fn = self
            .module
            .get_function("doo_db_get_global")
            .unwrap_or_else(|| {
                let fn_type = ptr_type.fn_type(&[], false);
                self.module.add_function("doo_db_get_global", fn_type, None)
            });

        // Call FFI function
        let result = self
            .builder
            .build_call(get_global_fn, &[], "db_get_global")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap();

        // Store result in temp_values
        self.temp_values.insert(dest.to_string(), result);

        Some(result)
    }

    /// Generate Database.raw() or Database.rawWithParams() call
    /// Automatically converts JSON result to typed data based on expected return type:
    /// - If return type is Str: returns raw JSON string
    /// - If return type is struct or [struct]: automatically parses JSON
    /// Returns DooResult* which will be unwrapped by MIR TryPropagate
    fn generate_database_raw_typed(
        &mut self,
        dest: &str,
        object: &str,
        method: &str,
        args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        use inkwell::AddressSpace;

        if args.is_empty() {
            eprintln!("Error: db.{}() requires SQL query argument", method);
            return None;
        }

        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Get Database instance and SQL query
        let db_val = self.resolve_value(object);
        let sql_arg = self.resolve_value(&args[0]);

        // Determine which FFI function to call based on method
        let (raw_fn_name, call_args) = if method == "rawWithParams" {
            // db.rawWithParams(sql, params) -> doo_db_raw_param(db, sql, params)
            if args.len() < 2 {
                eprintln!(
                    "Error: db.rawWithParams() requires 2 arguments: SQL query and parameters"
                );
                return None;
            }
            let params_arg_raw = self.resolve_value(&args[1]);

            // Check for Array type - must be serialized to JSON string for FFI
            let params_var = &args[1];
            let mut is_array = false;
            let mut elem_type = "Int".to_string();

            // Strategy 1: Direct lookup by variable name
            if self.array_metadata.contains_key(params_var) || self.heap_arrays.contains(params_var)
            {
                is_array = true;
                if let Some(meta) = self.array_metadata.get(params_var) {
                    elem_type = meta.element_type.clone();
                } else if let Some(type_str) = self.variable_types.get(params_var) {
                    if type_str.starts_with("Array(") && type_str.ends_with(")") {
                        elem_type = type_str[6..type_str.len() - 1].to_string();
                    } else if type_str.starts_with("[") && type_str.ends_with("]") {
                        elem_type = type_str[1..type_str.len() - 1].to_string();
                    }
                }
            } else if let Some(type_str) = self.variable_types.get(params_var) {
                if type_str.starts_with("Array(")
                    || type_str.starts_with("[")
                    || type_str == "Array"
                {
                    is_array = true;
                    if type_str.starts_with("Array(") && type_str.ends_with(")") {
                        elem_type = type_str[6..type_str.len() - 1].to_string();
                    } else if type_str.starts_with("[") && type_str.ends_with("]") {
                        elem_type = type_str[1..type_str.len() - 1].to_string();
                    }
                }
            }

            // Strategy 2: Try with % prefix stripped
            if !is_array {
                let stripped = params_var.trim_start_matches('%');
                if self.array_metadata.contains_key(stripped) || self.heap_arrays.contains(stripped)
                {
                    is_array = true;
                    if let Some(meta) = self.array_metadata.get(stripped) {
                        elem_type = meta.element_type.clone();
                    }
                }
            }

            // Strategy 3: Check _array suffix variations
            if !is_array {
                let with_suffix = format!("{}_array", params_var);
                let stripped_with_suffix = format!("{}_array", params_var.trim_start_matches('%'));
                if self.array_metadata.contains_key(&with_suffix) {
                    is_array = true;
                    if let Some(meta) = self.array_metadata.get(&with_suffix) {
                        elem_type = meta.element_type.clone();
                    }
                } else if self.array_metadata.contains_key(&stripped_with_suffix) {
                    is_array = true;
                    if let Some(meta) = self.array_metadata.get(&stripped_with_suffix) {
                        elem_type = meta.element_type.clone();
                    }
                }
            }

            // Strategy 4: Check by resolved pointer - if it's in temp_values and is a pointer, likely an array
            if !is_array && params_arg_raw.is_pointer_value() {
                // Look through all array metadata to find any matching pointer
                // This is a fallback for inline array literals
                for (key, meta) in &self.array_metadata.clone() {
                    if key.contains(params_var.trim_start_matches('%'))
                        || params_var.contains(key.trim_start_matches('%'))
                    {
                        is_array = true;
                        elem_type = meta.element_type.clone();
                        break;
                    }
                }
            }

            if is_array
                && elem_type != "Int"
                && elem_type != "Float"
                && elem_type != "Bool"
                && elem_type != "Str"
            {
                // Check if it's a known Enum to handle variants
                if !self.enum_variants.contains_key(&elem_type) {
                    // Try stripping namespace
                    let mut found = false;

                    // Handle Enum(Type) wrapper
                    if elem_type.starts_with("Enum(") && elem_type.ends_with(")") {
                        elem_type = elem_type[5..elem_type.len() - 1].to_string();
                        if self.enum_variants.contains_key(&elem_type) {
                            found = true;
                        }
                    }

                    if !found && elem_type.contains("::") {
                        let parts: Vec<&str> = elem_type.split("::").collect();
                        if let Some(last) = parts.last() {
                            if self.enum_variants.contains_key(*last) {
                                elem_type = last.to_string();
                                found = true;
                            }
                        }
                    }

                    if !found {
                        elem_type = "Enum".to_string();
                    }
                }
            }

            // Convert non-pointer types to string for FFI
            // The FFI function expects a string parameter (ptr type)
            let params_arg = if is_array {
                // Ensure we have a pointer to the array struct
                let array_ptr = if params_arg_raw.is_pointer_value() {
                    params_arg_raw.into_pointer_value()
                } else {
                    let alloca = self
                        .builder
                        .build_alloca(params_arg_raw.get_type(), "array_ptr_tmp")
                        .unwrap();
                    self.builder.build_store(alloca, params_arg_raw).unwrap();
                    alloca
                };

                let mut is_enum = false;
                let mut variants_str = String::new();

                if let Some(variants) = self.enum_variants.get(&elem_type) {
                    is_enum = true;
                    // Trim variants to avoid issues with parser including spaces
                    variants_str = variants
                        .iter()
                        .map(|(v, _)| v.as_str().trim())
                        .collect::<Vec<&str>>()
                        .join(",");
                }

                if is_enum {
                    let serialize_fn = self
                        .module
                        .get_function("doo_db_serialize_enum_array")
                        .unwrap_or_else(|| {
                            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                            let i32_type = self.context.i32_type();
                            // (ptr, str_ptr, stride)
                            let fn_type = ptr_type.fn_type(
                                &[ptr_type.into(), ptr_type.into(), i32_type.into()],
                                false,
                            );
                            self.module
                                .add_function("doo_db_serialize_enum_array", fn_type, None)
                        });

                    let variants_arg = self
                        .builder
                        .build_global_string_ptr(&variants_str, &format!("{}_vars", elem_type))
                        .unwrap();
                    // Stride is 16 for Enum { i32, ptr }
                    let stride_arg = self.context.i32_type().const_int(16, false);

                    self.builder
                        .build_call(
                            serialize_fn,
                            &[
                                array_ptr.into(),
                                variants_arg.as_pointer_value().into(),
                                stride_arg.into(),
                            ],
                            "json_params",
                        )
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                } else {
                    let serialize_fn = self
                        .module
                        .get_function("doo_db_serialize_array")
                        .unwrap_or_else(|| {
                            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                            let fn_type =
                                ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                            self.module
                                .add_function("doo_db_serialize_array", fn_type, None)
                        });

                    let type_arg = self
                        .builder
                        .build_global_string_ptr(&elem_type, "arr_type")
                        .unwrap();
                    self.builder
                        .build_call(
                            serialize_fn,
                            &[array_ptr.into(), type_arg.as_pointer_value().into()],
                            "serialized_params",
                        )
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                }
            } else if params_arg_raw.is_struct_value() {
                let struct_val = params_arg_raw.into_struct_value();

                // Infer type name: variable_type or from syntax "Type::Val"
                let mut type_name = self
                    .variable_types
                    .get(params_var)
                    .map(|s| s.clone())
                    .unwrap_or_default();

                // Handle Enum(Type) wrapper
                if type_name.starts_with("Enum(") && type_name.ends_with(")") {
                    type_name = type_name[5..type_name.len() - 1].to_string();
                }

                // Try to resolve namespace if not found
                if !self.enum_variants.contains_key(&type_name) && type_name.contains("::") {
                    let parts: Vec<&str> = type_name.split("::").collect();
                    if let Some(last) = parts.last() {
                        if self.enum_variants.contains_key(*last) {
                            type_name = last.to_string();
                        }
                    }
                }

                if type_name.is_empty() && params_var.contains("::") {
                    // e.g. Status::Done -> Status
                    let parts: Vec<&str> = params_var.split("::").collect();
                    if parts.len() > 0 {
                        type_name = parts[0].to_string();
                    }
                }

                if let Some(variants) = self.enum_variants.get(&type_name) {
                    // Generate Switch for Enum -> String Literal
                    if struct_val.get_type().count_fields() >= 1 {
                        let tag = self
                            .builder
                            .build_extract_value(struct_val, 0, "tag")
                            .unwrap()
                            .into_int_value();

                        let current_block = self.builder.get_insert_block().unwrap();
                        let target_fn = current_block.get_parent().unwrap();
                        let merge_block = self.context.append_basic_block(target_fn, "enum_merge");
                        let default_block =
                            self.context.append_basic_block(target_fn, "enum_default");

                        self.builder.position_at_end(default_block);
                        // Wrap Unknown in JSON array just in case
                        let unk_str = self
                            .builder
                            .build_global_string_ptr("[\"Unknown\"]", "str_unk")
                            .unwrap();
                        self.builder.build_unconditional_branch(merge_block);

                        let phi_type = self.context.ptr_type(inkwell::AddressSpace::default());

                        // We need to keep values alive to reference them
                        let mut incoming_vals: Vec<inkwell::values::BasicValueEnum> = Vec::new();
                        let mut incoming_blocks: Vec<inkwell::basic_block::BasicBlock> = Vec::new();
                        let mut cases: Vec<(
                            inkwell::values::IntValue,
                            inkwell::basic_block::BasicBlock,
                        )> = Vec::new();

                        incoming_vals.push(unk_str.as_pointer_value().as_basic_value_enum());
                        incoming_blocks.push(default_block);

                        for (i, variant_tuple) in variants.iter().enumerate() {
                            let variant = &variant_tuple.0;
                            let clean_variant = variant.trim();
                            let case_block = self
                                .context
                                .append_basic_block(target_fn, &format!("case_{}", clean_variant));
                            self.builder.position_at_end(case_block);
                            // Wrap in JSON array [] to ensure FFI treats it as parameter list matching the user expectation "work without []"
                            let json_val = format!("[\"{}\"]", clean_variant);
                            let s_ptr = self
                                .builder
                                .build_global_string_ptr(
                                    &json_val,
                                    &format!("str_{}", clean_variant),
                                )
                                .unwrap();
                            self.builder.build_unconditional_branch(merge_block);

                            cases.push((
                                self.context.i32_type().const_int(i as u64, false),
                                case_block,
                            ));
                            incoming_vals.push(s_ptr.as_pointer_value().as_basic_value_enum());
                            incoming_blocks.push(case_block);
                        }

                        self.builder.position_at_end(current_block);
                        self.builder.build_switch(tag, default_block, &cases);

                        self.builder.position_at_end(merge_block);
                        let phi = self.builder.build_phi(phi_type, "enum_str").unwrap();

                        // Construct slice of references
                        let incoming_refs: Vec<(
                            &dyn inkwell::values::BasicValue,
                            inkwell::basic_block::BasicBlock,
                        )> = incoming_vals
                            .iter()
                            .zip(incoming_blocks.iter())
                            .map(|(v, b)| (v as &dyn inkwell::values::BasicValue, *b))
                            .collect();

                        phi.add_incoming(&incoming_refs);
                        phi.as_basic_value()
                    } else {
                        params_arg_raw.into()
                    }
                } else {
                    // Fallback: struct but not known enum (or tag extraction for unknown enum)
                    if struct_val.get_type().count_fields() >= 1 {
                        let tag_extract = self
                            .builder
                            .build_extract_value(struct_val, 0, "enum_tag")
                            .unwrap();
                        if tag_extract.is_int_value() {
                            let int_val = tag_extract.into_int_value();
                            self.convert_int_to_string_via_sprintf(int_val)
                        } else {
                            params_arg_raw.into()
                        }
                    } else {
                        params_arg_raw.into()
                    }
                }
            } else if params_arg_raw.is_int_value() {
                let int_val = params_arg_raw.into_int_value();

                // Check for boolean type
                let mut is_bool = int_val.get_type().get_bit_width() == 1; // i1 is definitly bool
                if !is_bool {
                    // Check explicit type name
                    if let Some(type_str) = self.variable_types.get(params_var) {
                        if type_str == "Bool" {
                            is_bool = true;
                        }
                    } else if params_var == "true" || params_var == "false" {
                        is_bool = true;
                    }
                }

                if is_bool {
                    // Convert to "true" or "false" string with JSON array wrap if needed?
                    // Libdoo_db should handle un-wrapped bools via fallback.
                    // But for consistency we can stick to raw strings for bools/ints as they worked.
                    let true_str = self
                        .builder
                        .build_global_string_ptr("true", "str_true")
                        .unwrap();
                    let false_str = self
                        .builder
                        .build_global_string_ptr("false", "str_false")
                        .unwrap();

                    let is_true = if int_val.get_type().get_bit_width() == 1 {
                        int_val
                    } else {
                        self.builder
                            .build_int_compare(
                                inkwell::IntPredicate::NE,
                                int_val,
                                self.context.i32_type().const_zero(),
                                "is_true",
                            )
                            .unwrap()
                    };

                    self.builder
                        .build_select(
                            is_true,
                            true_str.as_pointer_value(),
                            false_str.as_pointer_value(),
                            "bool_str",
                        )
                        .unwrap()
                        .into()
                } else {
                    // Convert Int to string using sprintf
                    self.convert_int_to_string_via_sprintf(int_val)
                }
            } else if params_arg_raw.is_float_value() {
                // Convert Float to string using sprintf
                let float_val = params_arg_raw.into_float_value();
                self.convert_float_to_string_via_sprintf(float_val)
            } else {
                // Already a pointer (string) or other compatible type
                params_arg_raw
            };

            (
                "doo_db_raw_param",
                vec![db_val.into(), sql_arg.into(), params_arg.into()],
            )
        } else {
            // db.raw(sql) -> doo_db_raw(db, sql)
            ("doo_db_raw", vec![db_val.into(), sql_arg.into()])
        };

        // Declare the appropriate FFI function
        let raw_fn = if let Some(f) = self.module.get_function(raw_fn_name) {
            f
        } else {
            let fn_type = if method == "rawWithParams" {
                ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false)
            } else {
                ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false)
            };
            self.module.add_function(raw_fn_name, fn_type, None)
        };

        // Call the FFI function - returns DooResult*
        let raw_result = self
            .builder
            .build_call(raw_fn, &call_args, "db_raw_result")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap();

        // RUNTIME DEBUG: Print IMMEDIATELY after FFI call returns
        let printf_fn = self.get_or_declare_printf();
        let debug_msg_ffi_returned = self
            .builder
            .build_global_string_ptr(
                "[COMPILER_RUNTIME] @@@@@ doo_db_raw FFI call RETURNED @@@@@\n",
                "debug_ffi_returned",
            )
            .unwrap();
        self.builder
            .build_call(
                printf_fn,
                &[debug_msg_ffi_returned.as_pointer_value().into()],
                "",
            )
            .unwrap();

        // DIRECT EXTRACTION: No FFI call - extract DooResult fields directly
        // DooResult struct layout: { i32 tag, ptr value, u8 owner }
        let debug_msg_direct_extract = self
            .builder
            .build_global_string_ptr(
                "[COMPILER_RUNTIME] ##### DIRECT extraction from DooResult (no FFI call) #####\n",
                "debug_direct_extract",
            )
            .unwrap();
        self.builder
            .build_call(
                printf_fn,
                &[debug_msg_direct_extract.as_pointer_value().into()],
                "",
            )
            .unwrap();

        // Cast raw_result (ptr) to DooResult* for struct access
        let result_ptr = raw_result.into_pointer_value();

        // RUNTIME DEBUG: Print DooResult pointer
        let debug_msg_result_ptr = self
            .builder
            .build_global_string_ptr(
                "[COMPILER_RUNTIME] DooResult pointer: %p\n",
                "debug_result_ptr_fmt",
            )
            .unwrap();
        self.builder
            .build_call(
                printf_fn,
                &[
                    debug_msg_result_ptr.as_pointer_value().into(),
                    result_ptr.into(),
                ],
                "",
            )
            .unwrap();

        // Define DooResult struct type: { i32, ptr, i8 }
        let i32_type = self.context.i32_type();
        let i8_type = self.context.i8_type();
        let doo_result_struct_type = self
            .context
            .struct_type(&[i32_type.into(), ptr_type.into(), i8_type.into()], false);

        // GEP to field 0 (tag)
        let tag_ptr = self
            .builder
            .build_struct_gep(doo_result_struct_type, result_ptr, 0, "doo_result_tag_ptr")
            .unwrap();
        let tag_value = self
            .builder
            .build_load(i32_type, tag_ptr, "doo_result_tag")
            .unwrap();

        // RUNTIME DEBUG: Print tag value
        let debug_msg_tag = self
            .builder
            .build_global_string_ptr("[COMPILER_RUNTIME] DooResult.tag = %d\n", "debug_tag_fmt")
            .unwrap();
        self.builder
            .build_call(
                printf_fn,
                &[debug_msg_tag.as_pointer_value().into(), tag_value.into()],
                "",
            )
            .unwrap();

        // GEP to field 1 (value pointer - the actual string pointer)
        let value_ptr_ptr = self
            .builder
            .build_struct_gep(
                doo_result_struct_type,
                result_ptr,
                1,
                "doo_result_value_ptr_ptr",
            )
            .unwrap();
        let safe_string = self
            .builder
            .build_load(ptr_type, value_ptr_ptr, "doo_result_string_ptr")
            .unwrap();

        let rc_string_ptr = self.clone_ffi_string_to_rc(safe_string.into_pointer_value());

        // IMPORTANT: The returned Result will hold onto this RC-managed string pointer.
        // This builtin function also keeps a local reference (e.g. via a symbol/allocation)
        // which is decref'd during function-exit cleanup. Without an extra incref here,
        // the cleanup decref would drop RC to 0 and free the buffer, leaving the returned
        // pointer dangling (causing UTF-8 failures and empty responses).
        let incref = self
            .incref_fn
            .expect("RC runtime not initialized (incref_fn missing)");
        let rc_header_for_return = unsafe {
            self.builder.build_in_bounds_gep(
                self.context.i8_type(),
                rc_string_ptr,
                &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                "db_raw_rc_header_for_return",
            )
        }
        .unwrap();
        self.builder
            .build_call(incref, &[rc_header_for_return.into()], "")
            .unwrap();

        let free_string_fn_type = self.context.void_type().fn_type(&[ptr_type.into()], false);
        let free_string_fn = self
            .module
            .get_function("doo_db_free_string")
            .unwrap_or_else(|| self.module.add_function("doo_db_free_string", free_string_fn_type, None));
        let free_string_fn_ptr = free_string_fn.as_global_value().as_pointer_value();
        let free_string_fn_ptr_type = free_string_fn_type.ptr_type(AddressSpace::default());
        let free_string_fn_ptr_cast = self
            .builder
            .build_pointer_cast(
                free_string_fn_ptr,
                free_string_fn_ptr_type,
                "doo_db_free_string_cast",
            )
            .unwrap();
        self.builder
            .build_indirect_call(
                free_string_fn_type,
                free_string_fn_ptr_cast,
                &[safe_string.into()],
                "",
            )
            .unwrap();

        let free_result_fn = self
            .module
            .get_function("doo_db_result_free")
            .unwrap_or_else(|| {
                let fn_type = self.context.void_type().fn_type(&[ptr_type.into()], false);
                self.module.add_function("doo_db_result_free", fn_type, None)
            });
        self.builder
            .build_call(free_result_fn, &[result_ptr.into()], "")
            .unwrap();

        // RUNTIME DEBUG: Print string pointer
        let debug_msg_string_ptr = self
            .builder
            .build_global_string_ptr(
                "[COMPILER_RUNTIME] DooResult.value (string ptr) = %p\n",
                "debug_string_ptr_fmt",
            )
            .unwrap();
        self.builder
            .build_call(
                printf_fn,
                &[
                    debug_msg_string_ptr.as_pointer_value().into(),
                    safe_string.into(),
                ],
                "",
            )
            .unwrap();

        // GEP to field 2 (owner)
        let owner_ptr = self
            .builder
            .build_struct_gep(
                doo_result_struct_type,
                result_ptr,
                2,
                "doo_result_owner_ptr",
            )
            .unwrap();
        let owner_value = self
            .builder
            .build_load(i8_type, owner_ptr, "doo_result_owner")
            .unwrap();

        // RUNTIME DEBUG: Print owner value
        let debug_msg_owner = self
            .builder
            .build_global_string_ptr(
                "[COMPILER_RUNTIME] DooResult.owner = %d\n",
                "debug_owner_fmt",
            )
            .unwrap();
        let owner_as_i32 = self
            .builder
            .build_int_z_extend(owner_value.into_int_value(), i32_type, "owner_i32")
            .unwrap();
        self.builder
            .build_call(
                printf_fn,
                &[
                    debug_msg_owner.as_pointer_value().into(),
                    owner_as_i32.into(),
                ],
                "",
            )
            .unwrap();

        // Check if string pointer is null
        let is_null = self
            .builder
            .build_is_null(safe_string.into_pointer_value(), "is_string_null")
            .unwrap();

        let current_block = self.builder.get_insert_block().unwrap();
        let target_fn = current_block.get_parent().unwrap();
        let null_block = self
            .context
            .append_basic_block(target_fn, "string_null_check");
        let not_null_block = self
            .context
            .append_basic_block(target_fn, "string_not_null");

        self.builder
            .build_conditional_branch(is_null, null_block, not_null_block)
            .unwrap();

        // Null block
        self.builder.position_at_end(null_block);
        let debug_msg_null = self
            .builder
            .build_global_string_ptr(
                "[COMPILER_RUNTIME] ERROR: DooResult.value is NULL!\n",
                "debug_null",
            )
            .unwrap();
        self.builder
            .build_call(printf_fn, &[debug_msg_null.as_pointer_value().into()], "")
            .unwrap();
        self.builder
            .build_unconditional_branch(not_null_block)
            .unwrap();

        // Not null block - continue
        self.builder.position_at_end(not_null_block);
        let debug_msg_not_null = self
            .builder
            .build_global_string_ptr(
                "[COMPILER_RUNTIME] DooResult.value is valid continuing\n",
                "debug_not_null",
            )
            .unwrap();
        self.builder
            .build_call(
                printf_fn,
                &[debug_msg_not_null.as_pointer_value().into()],
                "",
            )
            .unwrap();

        // NOTE: We free the DooResult wrapper and the original FFI string above.
        // From here on, we only work with an RC-managed string.

        // RUNTIME DEBUG: About to wrap in Result struct
        let debug_msg_wrap = self
            .builder
            .build_global_string_ptr(
                "[COMPILER_RUNTIME] @@@ Creating Result struct @@@\n",
                "debug_wrap",
            )
            .unwrap();
        self.builder
            .build_call(printf_fn, &[debug_msg_wrap.as_pointer_value().into()], "")
            .unwrap();

        // Wrap in Result struct { i32 tag, ptr value } for handler
        let i32_type = self.context.i32_type();
        let result_struct_type = self
            .context
            .struct_type(&[i32_type.into(), ptr_type.into()], false);

        // Allocate Result struct on stack
        let result_alloca = self
            .builder
            .build_alloca(result_struct_type, "db_result_struct")
            .unwrap();

        // Store tag = 0 (Ok) in field 0
        let tag_ptr_field = self
            .builder
            .build_struct_gep(result_struct_type, result_alloca, 0, "tag_ptr")
            .unwrap();
        self.builder
            .build_store(tag_ptr_field, i32_type.const_zero())
            .unwrap();

        // Store string pointer in field 1 (the extracted value from DooResult)
        let value_ptr_field = self
            .builder
            .build_struct_gep(result_struct_type, result_alloca, 1, "value_ptr")
            .unwrap();
        self.builder
            .build_store(value_ptr_field, rc_string_ptr)
            .unwrap();

        // RUNTIME DEBUG: Print the string pointer being stored
        let debug_msg_storing = self
            .builder
            .build_global_string_ptr(
                "[COMPILER_RUNTIME] Storing string ptr %p in Result struct\n",
                "debug_storing_fmt",
            )
            .unwrap();
        self.builder
            .build_call(
                printf_fn,
                &[
                    debug_msg_storing.as_pointer_value().into(),
                    rc_string_ptr.into(),
                ],
                "",
            )
            .unwrap();

        // Load the complete struct
        let result_struct = self
            .builder
            .build_load(result_struct_type, result_alloca, "db_result_final")
            .unwrap();

        // RUNTIME DEBUG: Print wrapping complete
        let debug_msg_done = self
            .builder
            .build_global_string_ptr(
                "[COMPILER_RUNTIME] ===== Result struct LOADED successfully =====\n",
                "debug_done",
            )
            .unwrap();
        self.builder
            .build_call(printf_fn, &[debug_msg_done.as_pointer_value().into()], "")
            .unwrap();

        // CRITICAL FIX: Allocate Result struct on heap and store POINTER in temp_values
        // TryPropagate expects to load from a pointer, not a struct value
        // IMPORTANT: Use dooruntime_malloc to match dooruntime_free in TryPropagate
        let malloc_fn = self.get_or_declare_malloc();
        let struct_size_raw = result_struct_type.size_of().unwrap();
        let struct_size = self
            .builder
            .build_int_z_extend(struct_size_raw, self.context.i64_type(), "struct_size_i64")
            .unwrap();
        let result_heap_ptr = self
            .builder
            .build_call(malloc_fn, &[struct_size.into()], "result_heap_alloc")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();
        self.builder
            .build_store(result_heap_ptr, result_struct)
            .unwrap();

        // RUNTIME DEBUG: Print allocated pointer
        let debug_msg_alloc = self
            .builder
            .build_global_string_ptr(
                "[COMPILER_RUNTIME] Allocated Result struct at %p\n",
                "debug_alloc_fmt",
            )
            .unwrap();
        self.builder
            .build_call(
                printf_fn,
                &[
                    debug_msg_alloc.as_pointer_value().into(),
                    result_heap_ptr.into(),
                ],
                "",
            )
            .unwrap();

        // Store the Result struct POINTER - TryPropagate will unwrap it
        self.temp_values
            .insert(dest.to_string(), result_heap_ptr.into());

        // CRITICAL: Store the pointer into the variable's alloca so LLVM code can load it
        if let Some(sym) = self.symbols.get(dest) {
            self.builder.build_store(sym.ptr, result_heap_ptr).unwrap();

            // RUNTIME DEBUG: Confirm store
            let debug_msg_stored = self
                .builder
                .build_global_string_ptr(
                    "[COMPILER_RUNTIME] Stored Result ptr into symbol alloca\n",
                    "debug_stored",
                )
                .unwrap();
            self.builder
                .build_call(printf_fn, &[debug_msg_stored.as_pointer_value().into()], "")
                .unwrap();
        } else {
            // RUNTIME DEBUG: Symbol not found
            let debug_msg_nosym = self
                .builder
                .build_global_string_ptr(
                    "[COMPILER_RUNTIME] WARNING: Symbol not found for dest, cannot store!\n",
                    "debug_nosym",
                )
                .unwrap();
            self.builder
                .build_call(printf_fn, &[debug_msg_nosym.as_pointer_value().into()], "")
                .unwrap();
        }

        // Determine the expected Ok type for the Result
        // Priority: 1) Explicit variable type annotation, 2) Function return type
        let expected_ok_type = self
            .variable_types
            .get(dest)
            .cloned()
            .or_else(|| {
                // Fall back to function return type if no explicit annotation
                if let Some(func_name) = &self.current_function_name {
                    self.function_return_types.get(func_name).cloned()
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "Str".to_string());

        // Store result_types for TryPropagate to use when unwrapping
        // Result<OkType, DatabaseError>
        self.result_types.insert(
            dest.to_string(),
            (expected_ok_type.clone(), "DatabaseError".to_string()),
        );

        // ALWAYS mark db.raw() results as potentially needing JSON parsing
        // The actual parsing decision will be made at TryPropagate time based on:
        // 1. Explicit variable type annotation (let result: [User] = ...)
        // 2. Function return type (fn foo() -> [User])
        // If neither specifies a non-Str type, it will remain as JSON string
        self.temp_values
            .insert(format!("{}_needs_json_parse", dest), result_struct);

        Some(result_struct)
    }

    /// Generate CORS configuration - calls doo_http_cors FFI
    fn generate_cors_config(
        &mut self,
        dest: &str,
        object: &str,
        _args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        use inkwell::AddressSpace;

        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Get the server object
        let server_val = self.resolve_value(object);

        // Declare doo_http_cors FFI function - takes only server pointer
        let cors_fn = if let Some(f) = self.module.get_function("doo_http_cors") {
            f
        } else {
            let fn_type = ptr_type.fn_type(&[ptr_type.into()], false);
            let func = self.module.add_function("doo_http_cors", fn_type, None);
            func.set_linkage(inkwell::module::Linkage::External);
            func
        };

        // Call doo_http_cors(server)
        let result = self
            .builder
            .build_call(cors_fn, &[server_val.into()], "cors_result")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap();

        // Store result (server pointer for chaining)
        self.temp_values.insert(dest.to_string(), result);

        self.variable_types
            .insert(dest.to_string(), "Server".to_string());
        self.struct_instance_types
            .insert(dest.to_string(), "Server".to_string());
        if dest.starts_with('%') {
            self.variable_types.insert(
                dest.trim_start_matches('%').to_string(),
                "Server".to_string(),
            );
            self.struct_instance_types.insert(
                dest.trim_start_matches('%').to_string(),
                "Server".to_string(),
            );
        } else {
            self.variable_types
                .insert(format!("%{}", dest), "Server".to_string());
            self.struct_instance_types
                .insert(format!("%{}", dest), "Server".to_string());
        }

        Some(result)
    }

    /// Generate rate limiting configuration - calls doo_http_ratelimit FFI
    fn generate_ratelimit_config(
        &mut self,
        dest: &str,
        object: &str,
        _args: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        use inkwell::AddressSpace;

        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Get the server object
        let server_val = self.resolve_value(object);

        // Declare doo_http_ratelimit FFI function - takes only server pointer
        let ratelimit_fn = if let Some(f) = self.module.get_function("doo_http_ratelimit") {
            f
        } else {
            let fn_type = ptr_type.fn_type(&[ptr_type.into()], false);
            let func = self
                .module
                .add_function("doo_http_ratelimit", fn_type, None);
            func.set_linkage(inkwell::module::Linkage::External);
            func
        };

        // Call doo_http_ratelimit(server)
        let result = self
            .builder
            .build_call(ratelimit_fn, &[server_val.into()], "ratelimit_result")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap();

        // Store result (server pointer for chaining)
        self.temp_values.insert(dest.to_string(), result);

        self.variable_types
            .insert(dest.to_string(), "Server".to_string());
        self.struct_instance_types
            .insert(dest.to_string(), "Server".to_string());
        if dest.starts_with('%') {
            self.variable_types.insert(
                dest.trim_start_matches('%').to_string(),
                "Server".to_string(),
            );
            self.struct_instance_types.insert(
                dest.trim_start_matches('%').to_string(),
                "Server".to_string(),
            );
        } else {
            self.variable_types
                .insert(format!("%{}", dest), "Server".to_string());
            self.struct_instance_types
                .insert(format!("%{}", dest), "Server".to_string());
        }

        Some(result)
    }
}
