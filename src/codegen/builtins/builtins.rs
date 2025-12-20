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
            if self.array_metadata.contains_key(params_var) || self.heap_arrays.contains(params_var) {
                is_array = true;
                if let Some(meta) = self.array_metadata.get(params_var) {
                    elem_type = meta.element_type.clone();
                } else if let Some(type_str) = self.variable_types.get(params_var) {
                     if type_str.starts_with("Array(") && type_str.ends_with(")") {
                         elem_type = type_str[6..type_str.len()-1].to_string();
                     } else if type_str.starts_with("[") && type_str.ends_with("]") {
                         elem_type = type_str[1..type_str.len()-1].to_string();
                     }
                }
            } else if let Some(type_str) = self.variable_types.get(params_var) {
                 if type_str.starts_with("Array(") || type_str.starts_with("[") || type_str == "Array" {
                     is_array = true;
                     if type_str.starts_with("Array(") && type_str.ends_with(")") {
                         elem_type = type_str[6..type_str.len()-1].to_string();
                     } else if type_str.starts_with("[") && type_str.ends_with("]") {
                         elem_type = type_str[1..type_str.len()-1].to_string();
                     }
                 }
            }
            
            // Strategy 2: Try with % prefix stripped
            if !is_array {
                let stripped = params_var.trim_start_matches('%');
                if self.array_metadata.contains_key(stripped) || self.heap_arrays.contains(stripped) {
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
                    if key.contains(params_var.trim_start_matches('%')) || params_var.contains(key.trim_start_matches('%')) {
                        is_array = true;
                        elem_type = meta.element_type.clone();
                        break;
                    }
                }
            }

            if is_array && elem_type != "Int" && elem_type != "Float" && elem_type != "Bool" && elem_type != "Str" {
                 // Check if it's a known Enum to handle variants
                 if !self.enum_variants.contains_key(&elem_type) {
                     // Try stripping namespace
                     let mut found = false;
                     
                     // Handle Enum(Type) wrapper
                     if elem_type.starts_with("Enum(") && elem_type.ends_with(")") {
                         elem_type = elem_type[5..elem_type.len()-1].to_string();
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
                     let alloca = self.builder.build_alloca(params_arg_raw.get_type(), "array_ptr_tmp").unwrap();
                     self.builder.build_store(alloca, params_arg_raw).unwrap();
                     alloca
                 };

                 let mut is_enum = false;
                 let mut variants_str = String::new();
                 
                 if let Some(variants) = self.enum_variants.get(&elem_type) {
                     is_enum = true;
                     // Trim variants to avoid issues with parser including spaces
                     variants_str = variants.iter().map(|(v, _)| v.as_str().trim()).collect::<Vec<&str>>().join(",");
                 }

                 if is_enum {
                     let serialize_fn = self.module.get_function("doo_db_serialize_enum_array").unwrap_or_else(|| {
                         let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                         let i32_type = self.context.i32_type();
                         // (ptr, str_ptr, stride)
                         let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into(), i32_type.into()], false);
                         self.module.add_function("doo_db_serialize_enum_array", fn_type, None)
                     });
                     
                     let variants_arg = self.builder.build_global_string_ptr(&variants_str, &format!("{}_vars", elem_type)).unwrap();
                     // Stride is 16 for Enum { i32, ptr }
                     let stride_arg = self.context.i32_type().const_int(16, false); 
                     
                     self.builder.build_call(serialize_fn, &[array_ptr.into(), variants_arg.as_pointer_value().into(), stride_arg.into()], "json_params")
                         .unwrap()
                         .try_as_basic_value()
                         .left()
                         .unwrap()
                 } else {
                     let serialize_fn = self.module.get_function("doo_db_serialize_array").unwrap_or_else(|| {
                         let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                         let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                         self.module.add_function("doo_db_serialize_array", fn_type, None)
                     });
                     
                     let type_arg = self.builder.build_global_string_ptr(&elem_type, "arr_type").unwrap();
                     self.builder.build_call(serialize_fn, &[array_ptr.into(), type_arg.as_pointer_value().into()], "serialized_params")
                         .unwrap()
                         .try_as_basic_value()
                         .left()
                         .unwrap()
                 }
            } else if params_arg_raw.is_struct_value() {
                 let struct_val = params_arg_raw.into_struct_value();
                 
                 // Infer type name: variable_type or from syntax "Type::Val"
                 let mut type_name = self.variable_types.get(params_var).map(|s| s.clone()).unwrap_or_default();
                 
                 // Handle Enum(Type) wrapper
                 if type_name.starts_with("Enum(") && type_name.ends_with(")") {
                     type_name = type_name[5..type_name.len()-1].to_string();
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
                        let tag = self.builder.build_extract_value(struct_val, 0, "tag").unwrap().into_int_value();
                        
                        let current_block = self.builder.get_insert_block().unwrap();
                        let target_fn = current_block.get_parent().unwrap();
                        let merge_block = self.context.append_basic_block(target_fn, "enum_merge");
                        let default_block = self.context.append_basic_block(target_fn, "enum_default");
                        
                        self.builder.position_at_end(default_block);
                        // Wrap Unknown in JSON array just in case
                        let unk_str = self.builder.build_global_string_ptr("[\"Unknown\"]", "str_unk").unwrap();
                        self.builder.build_unconditional_branch(merge_block);
                        
                        let phi_type = self.context.ptr_type(inkwell::AddressSpace::default());
                        
                        // We need to keep values alive to reference them
                        let mut incoming_vals: Vec<inkwell::values::BasicValueEnum> = Vec::new();
                        let mut incoming_blocks: Vec<inkwell::basic_block::BasicBlock> = Vec::new();
                        let mut cases: Vec<(inkwell::values::IntValue, inkwell::basic_block::BasicBlock)> = Vec::new();
                        
                        incoming_vals.push(unk_str.as_pointer_value().as_basic_value_enum());
                        incoming_blocks.push(default_block);

                        for (i, variant_tuple) in variants.iter().enumerate() {
                             let variant = &variant_tuple.0;
                             let clean_variant = variant.trim();
                             let case_block = self.context.append_basic_block(target_fn, &format!("case_{}", clean_variant));
                             self.builder.position_at_end(case_block);
                             // Wrap in JSON array [] to ensure FFI treats it as parameter list matching the user expectation "work without []"
                             let json_val = format!("[\"{}\"]", clean_variant);
                             let s_ptr = self.builder.build_global_string_ptr(&json_val, &format!("str_{}", clean_variant)).unwrap();
                             self.builder.build_unconditional_branch(merge_block);
                             
                             cases.push((self.context.i32_type().const_int(i as u64, false), case_block));
                             incoming_vals.push(s_ptr.as_pointer_value().as_basic_value_enum());
                             incoming_blocks.push(case_block);
                        }
                        
                        self.builder.position_at_end(current_block);
                        self.builder.build_switch(tag, default_block, &cases);
                        
                        self.builder.position_at_end(merge_block);
                        let phi = self.builder.build_phi(phi_type, "enum_str").unwrap();
                        
                        // Construct slice of references
                        let incoming_refs: Vec<(&dyn inkwell::values::BasicValue, inkwell::basic_block::BasicBlock)> = 
                            incoming_vals.iter().zip(incoming_blocks.iter())
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
                        let tag_extract = self.builder.build_extract_value(struct_val, 0, "enum_tag").unwrap();
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
                     let true_str = self.builder.build_global_string_ptr("true", "str_true").unwrap();
                     let false_str = self.builder.build_global_string_ptr("false", "str_false").unwrap();
                     
                     let is_true = if int_val.get_type().get_bit_width() == 1 {
                         int_val
                     } else {
                         self.builder.build_int_compare(inkwell::IntPredicate::NE, int_val, self.context.i32_type().const_zero(), "is_true").unwrap()
                     };
                     
                     self.builder.build_select(is_true, true_str.as_pointer_value(), false_str.as_pointer_value(), "bool_str")
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

        // Store the raw result - TryPropagate will unwrap it
        self.temp_values.insert(dest.to_string(), raw_result);

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
            .insert(format!("{}_needs_json_parse", dest), raw_result);

        Some(raw_result)
    }
}
