use crate::codegen::core::helpers::parse_tuple_types;
use crate::codegen::core::CodeGen;
use inkwell::types::BasicTypeEnum;
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
        // Chain alias lookups: File::Write -> Write -> doo_file_write
        let mut actual_func_name = func.to_string();
        loop {
            if let Some(resolved) = self.function_aliases.get(&actual_func_name) {
                let next_name = resolved.clone();
                if next_name == actual_func_name {
                    break; // Prevent infinite loop
                }
                actual_func_name = next_name;
            } else {
                break;
            }
        }

        let callee = self.module.get_function(&actual_func_name).expect(&format!(
            "Function '{}' not found. Make sure it's declared before calling. (Original: '{}')",
            actual_func_name, func
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
                // Check if this is an FFI function with error type</parameter>
                // FFI functions return a pointer to Result struct, not struct by value
                let is_ffi_result = {
                    // Check both the original Doo function name and the resolved C symbol name</parameter>
                    // Also check without namespace prefix (File::Write -> Write)
                    let func_without_namespace = func.split("::").last().unwrap_or(func);
                    let has_error_type = self.function_error_types.contains_key(&actual_func_name)
                        || self.function_error_types.contains_key(func)
                        || self
                            .function_error_types
                            .contains_key(func_without_namespace);

                    // Check if this is FFI by looking for the function in our aliases or checking linkage
                    let is_ffi = self
                        .function_aliases
                        .values()
                        .any(|v| v == &actual_func_name)
                        || self.function_aliases.contains_key(func)
                        || callee.get_linkage() == inkwell::module::Linkage::External;

                    has_error_type && is_ffi && result.is_pointer_value()
                };

                // If FFI returned a pointer to Result, load the struct</parameter>
                let actual_result = if is_ffi_result {
                    let result_ptr = result.into_pointer_value();
                    let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                    let result_struct_type = self
                        .context
                        .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false);
                    self.builder
                        .build_load(result_struct_type, result_ptr, "ffi_result_load")
                        .unwrap()
                } else {
                    result
                };

                // Check if this is a Result type by checking BOTH struct signature AND error type
                // Result structs have { i32 tag, ptr value } AND the function must have error_type
                // Plain tuples can also be { i32, ptr } but don't have error_type
                let func_without_namespace = func.split("::").last().unwrap_or(func);
                let has_error_type = self.function_error_types.contains_key(&actual_func_name)
                    || self.function_error_types.contains_key(func)
                    || self
                        .function_error_types
                        .contains_key(func_without_namespace);

                let is_result_type = has_error_type && actual_result.is_struct_value() && {
                    let struct_type = actual_result.get_type();
                    if let BasicTypeEnum::StructType(st) = struct_type {
                        if st.count_fields() == 2 {
                            // Check if first field is i32 (Result tag field)
                            if let Some(field0_type) = st.get_field_type_at_index(0) {
                                if let BasicTypeEnum::IntType(int_type) = field0_type {
                                    int_type.get_bit_width() == 32
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };

                // If result is Result type and function returns multiple values, set up tuple metadata
                // This handles both:
                // 1. let a, b, err = GetPair() - with explicit error extraction (dest.len() > 1)
                // 2. let a, b = GetPair() - without error extraction (dest.len() == 1, needs unwrapping)
                if is_result_type {
                    let return_type_str_opt = self
                        .function_return_types
                        .get(&actual_func_name)
                        .or_else(|| self.function_return_types.get(func))
                        .cloned();

                    // Check if the function returns multiple values (tuple inside Result)
                    let is_multi_value_return = return_type_str_opt
                        .as_ref()
                        .map_or(false, |s| s.contains(','));

                    if is_multi_value_return && dest.len() >= 1 {
                        // Multi-value Result return - set up metadata for TupleExtract
                        // The Result struct is { i32 tag, ptr value } where ptr points to a heap tuple
                        let result_struct = actual_result.into_struct_value();
                        let tuple_ptr_value = self
                            .builder
                            .build_extract_value(result_struct, 1, "unwrapped_ptr")
                            .unwrap();

                        // The extracted value should be a pointer
                        let tuple_ptr = if tuple_ptr_value.is_pointer_value() {
                            tuple_ptr_value.into_pointer_value()
                        } else {
                            // Shouldn't happen, but handle gracefully
                            return None;
                        };

                        // Set up tuple type metadata for TupleExtract to use
                        if let Some(return_type_str) = return_type_str_opt {
                            // Strip "Tuple()" wrapper if present before parsing
                            let inner_types = if return_type_str.starts_with("Tuple(")
                                && return_type_str.ends_with(")")
                            {
                                &return_type_str[6..return_type_str.len() - 1]
                            } else {
                                &return_type_str[..]
                            };

                            let types = parse_tuple_types(inner_types);
                            // Don't double-wrap if return_type_str already starts with "Tuple("
                            let tuple_type_str = if return_type_str.starts_with("Tuple(") {
                                return_type_str.clone()
                            } else {
                                format!("Tuple({})", return_type_str)
                            };

                            // Build tuple struct type
                            let tuple_field_types: Vec<inkwell::types::BasicTypeEnum> =
                                types.iter().map(|t| self.map_type_str_to_llvm(t)).collect();
                            let tuple_type = self.context.struct_type(&tuple_field_types, false);

                            // Store tuple metadata
                            self.tuple_struct_types
                                .insert(tuple_type_str.clone(), tuple_type);

                            // CRITICAL: Also store tuple_field_types for reconstruction
                            self.tuple_field_types
                                .insert(dest[0].clone(), tuple_field_types.clone());

                            // Get error type for result_types
                            let err_type = self
                                .function_error_types
                                .get(&actual_func_name)
                                .or_else(|| self.function_error_types.get(func))
                                .cloned()
                                .unwrap_or_else(|| "Str".to_string());

                            // Store the tuple pointer with the first dest name, and tuple metadata
                            // so TupleExtract can find it
                            let tuple_holder_name = format!("{}_tuple_ptr", dest[0]);
                            self.temp_values
                                .insert(tuple_holder_name.clone(), tuple_ptr.into());
                            self.tuple_types
                                .insert(tuple_holder_name.clone(), tuple_type_str.clone());

                            // Also store in symbols with pointer type
                            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                            let ptr_alloca = self
                                .builder
                                .build_alloca(ptr_type, &format!("{}_ptr_storage", dest[0]))
                                .unwrap();
                            self.builder.build_store(ptr_alloca, tuple_ptr).unwrap();
                            self.symbols.insert(
                                tuple_holder_name.clone(),
                                crate::codegen::Symbol {
                                    ptr: ptr_alloca,
                                    ty: ptr_type.into(),
                                },
                            );

                            // CRITICAL: Store result_types and tuple metadata for the dest[0] temp
                            // This is used by TupleExtract to know the field types
                            self.result_types.insert(
                                dest[0].clone(),
                                (return_type_str.clone(), err_type.clone()),
                            );
                            self.tuple_types
                                .insert(dest[0].clone(), tuple_type_str.clone());

                            // Store the Result struct itself in temp_values for the dest
                            self.temp_values.insert(dest[0].clone(), actual_result);

                            // For multi-dest case, set up aliases for each dest name
                            if dest.len() > 1 {
                                for dest_name in dest.iter() {
                                    // Create a mapping from dest_name to the tuple holder
                                    self.tuple_types
                                        .insert(dest_name.clone(), tuple_type_str.clone());
                                    self.temp_values.insert(dest_name.clone(), tuple_ptr.into());
                                    // Also store result_types for each dest
                                    self.result_types.insert(
                                        dest_name.clone(),
                                        (return_type_str.clone(), err_type.clone()),
                                    );
                                }
                            }
                        }

                        // Return the full Result struct so it can be stored if needed
                        return Some(actual_result);
                    }
                }

                let dest_name = &dest[0];

                // CRITICAL FIX: For Result types, store the FULL struct (with tag)
                // Don't unwrap it here - let print and other operations handle the tag checking
                let final_result = actual_result;

                // Track that this is a Result type so downstream operations know to handle it specially
                if is_result_type {
                    // Get the Ok type from function_return_types
                    // Handle namespaced names (File::Write -> Write)
                    let func_without_namespace = func.split("::").last().unwrap_or(func);
                    let ok_type = self
                        .function_return_types
                        .get(&actual_func_name)
                        .or_else(|| self.function_return_types.get(func))
                        .or_else(|| self.function_return_types.get(func_without_namespace))
                        .cloned()
                        .unwrap_or_else(|| "Unknown".to_string());

                    // Get the Err type from function_error_types
                    let err_type = self
                        .function_error_types
                        .get(&actual_func_name)
                        .or_else(|| self.function_error_types.get(func))
                        .or_else(|| self.function_error_types.get(func_without_namespace))
                        .cloned()
                        .unwrap_or_else(|| "Str".to_string());

                    // CRITICAL: Store result_types under BOTH the temp name and dest name
                    // ManualErrorExtract looks up by temp name (e.g., "%19")
                    // Other code looks up by dest name (e.g., "size")
                    self.result_types
                        .insert(dest_name.clone(), (ok_type.clone(), err_type.clone()));

                    // Also store under all dest names if there are multiple
                    for dn in dest.iter() {
                        self.result_types
                            .insert(dn.clone(), (ok_type.clone(), err_type.clone()));
                    }

                    // Check if ok_type is a tuple/multi-value type
                    // If so, set up tuple_types so TupleExtract can work properly
                    if ok_type.contains(',') || ok_type.starts_with("Tuple(") {
                        let tuple_type_str = if ok_type.starts_with("Tuple(") {
                            ok_type.clone()
                        } else {
                            format!("Tuple({})", ok_type)
                        };

                        // Build tuple struct type
                        // Strip "Tuple(...)" wrapper before parsing
                        let inner_types = if tuple_type_str.starts_with("Tuple(")
                            && tuple_type_str.ends_with(')')
                        {
                            &tuple_type_str[6..tuple_type_str.len() - 1]
                        } else {
                            &tuple_type_str
                        };
                        let types = crate::codegen::core::helpers::parse_tuple_types(inner_types);
                        let tuple_field_types: Vec<inkwell::types::BasicTypeEnum> =
                            types.iter().map(|t| self.map_type_str_to_llvm(t)).collect();
                        let tuple_type = self.context.struct_type(&tuple_field_types, false);

                        // Store tuple metadata
                        self.tuple_struct_types
                            .insert(tuple_type_str.clone(), tuple_type);

                        // Set tuple_types for this Result so TupleExtract can find it
                        self.tuple_types.insert(dest_name.clone(), tuple_type_str);
                    }

                    // Store the Result struct in temp_values AND in symbols
                    // This ensures both resolve_value and print can access it properly
                    self.temp_values.insert(dest_name.clone(), final_result);

                    // CRITICAL: Store result_types for the actual temp variable name too
                    // The call instruction stores the result in a temp like "%19"
                    // We need to be able to look up result_types by that temp name
                    // Look through temp_values to find which temp holds this result
                    for (temp_name, temp_val) in self.temp_values.iter() {
                        if temp_name.starts_with('%') {
                            // Check if this temp value is the same as our result
                            if temp_val.is_struct_value() && final_result.is_struct_value() {
                                let temp_struct = temp_val.into_struct_value();
                                let final_struct = final_result.into_struct_value();
                                // If they're the same pointer/value, this is our temp
                                if temp_struct.as_instruction() == final_struct.as_instruction() {
                                    self.result_types.insert(
                                        temp_name.clone(),
                                        (ok_type.clone(), err_type.clone()),
                                    );
                                    break;
                                }
                            }
                        }
                    }

                    // Also store the Result struct on the stack for proper resolution
                    // This ensures that resolve_value can load it as a struct from memory
                    let result_struct_val = final_result.into_struct_value();
                    let result_struct_type = result_struct_val.get_type();
                    let struct_alloca = self
                        .builder
                        .build_alloca(result_struct_type, &format!("{}_result_storage", dest_name))
                        .unwrap();
                    self.builder
                        .build_store(struct_alloca, result_struct_val)
                        .unwrap();
                    self.symbols.insert(
                        dest_name.clone(),
                        crate::codegen::Symbol {
                            ptr: struct_alloca,
                            ty: result_struct_type.into(),
                        },
                    );

                    // Mark variable type as Result so it doesn't get unwrapped
                    self.variable_types
                        .insert(dest_name.clone(), "Result".to_string());

                    // Return early - do NOT continue with normal single-value handling
                    return Some(actual_result);
                } else {
                    // Non-Result type - check if tuple before storing
                    // IMPORTANT: Check for struct return type FIRST and track it
                    // This must happen before tuple checks or any other logic
                    let return_type_str = self
                        .function_return_types
                        .get(&actual_func_name)
                        .or_else(|| self.function_return_types.get(func));

                    // Check if this is a tuple return EARLY so we don't store struct in temp_values
                    let is_tuple_return = if let Some(ret_type_str) = return_type_str {
                        if ret_type_str.starts_with("Tuple(") {
                            true
                        } else {
                            let parsed = parse_tuple_types(ret_type_str);
                            parsed.len() > 1
                        }
                    } else {
                        false
                    };

                    // Only store in temp_values if NOT a tuple (tuples will be stored as alloca pointer later)
                    if !is_tuple_return {
                        self.temp_values.insert(dest_name.clone(), final_result);
                    }

                    if let Some(return_type_str) = return_type_str {
                        // Check if this is a struct type - handle both "Struct(Name)" and bare "Name" formats
                        let struct_name_opt = if return_type_str.starts_with("Struct(")
                            && return_type_str.ends_with(")")
                        {
                            // Struct(Name) format - extract the name
                            Some(&return_type_str[7..return_type_str.len() - 1])
                        } else if !return_type_str.contains('(')
                            && !return_type_str.contains(',')
                            && return_type_str != "Int"
                            && return_type_str != "Float"
                            && return_type_str != "Bool"
                            && return_type_str != "Str"
                            && return_type_str != "Void"
                            && self.struct_metadata.contains_key(return_type_str)
                        {
                            // Bare struct name format (TypeRef) - use as is if it's in struct_metadata
                            Some(return_type_str.as_str())
                        } else {
                            None
                        };

                        if let Some(struct_name) = struct_name_opt {
                            // Always track struct instances for printing with multiple name variations
                            self.struct_instance_types
                                .insert(dest_name.clone(), struct_name.to_string());
                            // Also store without % prefix if dest has it
                            if dest_name.starts_with('%') {
                                self.struct_instance_types.insert(
                                    dest_name.trim_start_matches('%').to_string(),
                                    struct_name.to_string(),
                                );
                            }
                            // Also store with % prefix if dest doesn't have it
                            if !dest_name.starts_with('%') {
                                self.struct_instance_types
                                    .insert(format!("%{}", dest_name), struct_name.to_string());
                            }

                            // Also track in variable_types so temp variables can resolve back to struct type
                            self.variable_types
                                .insert(dest_name.clone(), struct_name.to_string());
                        }
                    }
                }

                // Check if this is a tuple return (multi-value)
                // Only treat as tuple if it explicitly starts with "Tuple(" or has top-level commas
                // BUT: Skip this for Result types - they have their own handling above
                if !is_result_type {
                    // Try to get return type, checking both actual_func_name and original func name
                    let return_type_str = self
                        .function_return_types
                        .get(&actual_func_name)
                        .or_else(|| self.function_return_types.get(func));

                    if let Some(return_type_str) = return_type_str {
                        // Parse types respecting nested parentheses to detect true tuples
                        let is_tuple_return = if return_type_str.starts_with("Tuple(") {
                            true
                        } else {
                            // Check if there are multiple top-level types (comma not inside parentheses)
                            let parsed = parse_tuple_types(return_type_str);
                            parsed.len() > 1
                        };

                        if is_tuple_return {
                            // This is a tuple return - store tuple type info
                            // Don't double-wrap if already wrapped
                            let tuple_type_str = if return_type_str.starts_with("Tuple(") {
                                return_type_str.clone()
                            } else {
                                format!("Tuple({})", return_type_str)
                            };
                            self.tuple_types
                                .insert(dest_name.clone(), tuple_type_str.clone());

                            // Store the struct type if not already cached
                            let struct_type = if let Some(cached_type) =
                                self.tuple_struct_types.get(&tuple_type_str)
                            {
                                *cached_type
                            } else {
                                // Strip Tuple() wrapper if present before parsing
                                let inner_types = if return_type_str.starts_with("Tuple(")
                                    && return_type_str.ends_with(')')
                                {
                                    &return_type_str[6..return_type_str.len() - 1]
                                } else {
                                    return_type_str.as_str()
                                };
                                let types = parse_tuple_types(inner_types);
                                let mut field_types: Vec<inkwell::types::BasicTypeEnum> =
                                    Vec::new();

                                for type_str in &types {
                                    let llvm_type = if type_str.contains("String")
                                        || type_str.contains("Str")
                                    {
                                        self.context
                                            .ptr_type(inkwell::AddressSpace::default())
                                            .into()
                                    } else if type_str.contains("Array") || type_str.contains("Map")
                                    {
                                        self.context
                                            .ptr_type(inkwell::AddressSpace::default())
                                            .into()
                                    } else if type_str.contains("Float") {
                                        self.context.f64_type().into()
                                    } else if type_str.contains("Bool") {
                                        self.context.bool_type().into()
                                    } else {
                                        self.context.i32_type().into()
                                    };
                                    field_types.push(llvm_type);
                                }

                                let new_struct_type = self.context.struct_type(&field_types, false);
                                self.tuple_struct_types
                                    .insert(tuple_type_str.clone(), new_struct_type);
                                new_struct_type
                            };

                            // Allocate and store the tuple struct
                            let struct_alloca = self
                                .builder
                                .build_alloca(struct_type, &format!("{}_tuple", dest_name))
                                .unwrap();
                            self.builder
                                .build_store(struct_alloca, final_result)
                                .unwrap();
                            self.temp_values
                                .insert(dest_name.clone(), struct_alloca.into());
                        } else {
                            // Single-value return - track the type in variable_types
                            // Try both actual_func_name and original func name
                            let return_type_str = self
                                .function_return_types
                                .get(&actual_func_name)
                                .or_else(|| self.function_return_types.get(func));

                            if let Some(return_type_str) = return_type_str {
                                // Extract the base type for single-value returns
                                if return_type_str.contains("Bool") {
                                    self.variable_types
                                        .insert(dest_name.clone(), "Bool".to_string());
                                    self.boolean_temps.insert(dest_name.clone());
                                } else if return_type_str.contains("Float") {
                                    self.variable_types
                                        .insert(dest_name.clone(), "Float".to_string());
                                } else if return_type_str.contains("Str") {
                                    self.variable_types
                                        .insert(dest_name.clone(), "Str".to_string());
                                } else if return_type_str.contains("Int") {
                                    self.variable_types
                                        .insert(dest_name.clone(), "Int".to_string());
                                }
                                // Note: Struct type tracking is now handled earlier (above)
                                // to ensure it always executes before other conditional paths
                            }
                        }
                    }
                }

                // Check if this function is known to return heap-allocated values
                // Use actual_func_name in case func was an alias
                if self.functions_returning_heap.contains(&actual_func_name) {
                    if result.is_pointer_value() {
                        // Mark the result as heap-allocated based on return type
                        if let Some(return_type_str) =
                            self.function_return_types.get(&actual_func_name)
                        {
                            if return_type_str.contains("Array") && !return_type_str.contains(',') {
                                self.heap_arrays.insert(dest_name.clone());
                                self.variable_types
                                    .insert(dest_name.clone(), "Array".to_string());

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
                                self.variable_types
                                    .insert(dest_name.clone(), "Map".to_string());

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

            // Check if this is a regular struct instance FIRST (before array/map check)
            // This prevents structs from being misidentified as empty arrays
            let struct_instance_name = self
                .struct_instance_types
                .get(value)
                .cloned()
                .or_else(|| {
                    self.struct_instance_types
                        .get(&value.trim_start_matches('%').to_string())
                        .cloned()
                })
                .or_else(|| {
                    // If not found in struct_instance_types, check variable_types
                    // The variable_types map might have the type information
                    if let Some(type_str) = self.variable_types.get(value) {
                        // Check if this type is a struct type
                        if type_str.starts_with("Struct(") && type_str.ends_with(")") {
                            // Extract struct name from "Struct(StructName)"
                            let struct_name = &type_str[7..type_str.len() - 1];
                            if self.struct_metadata.contains_key(struct_name) {
                                return Some(struct_name.to_string());
                            }
                        } else if self.struct_metadata.contains_key(type_str) {
                            // Direct struct name (no "Struct(...)" wrapper)
                            return Some(type_str.clone());
                        }
                    }
                    None
                });

            // Check if this value is an array or map by looking at metadata
            // But NEVER treat loop iteration variables or struct instances as arrays/maps
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
            // Also exclude struct instances from being treated as arrays
            let resolved_val = self.resolve_value(value);
            let is_actually_pointer = resolved_val.is_pointer_value();

            let is_array = !is_loop_var
                && struct_instance_name.is_none()
                && is_actually_pointer
                && (has_array_metadata || self.heap_arrays.contains(value));
            let is_map = !is_loop_var
                && struct_instance_name.is_none()
                && is_actually_pointer
                && (has_map_metadata || self.heap_maps.contains(value));

            // Check if this value is a Result struct by checking result_types
            // Also check if the resolved value is a struct (fallback detection)
            let resolved_val_for_result_check = self.resolve_value(value);
            let is_struct_value = resolved_val_for_result_check.is_struct_value();
            let is_result_struct_value = is_struct_value && {
                let struct_type = resolved_val_for_result_check.get_type();
                if let BasicTypeEnum::StructType(st) = struct_type {
                    st.count_fields() == 2
                        && st
                            .get_field_type_at_index(0)
                            .map(|f| {
                                if let BasicTypeEnum::IntType(it) = f {
                                    it.get_bit_width() == 32
                                } else {
                                    false
                                }
                            })
                            .unwrap_or(false)
                } else {
                    false
                }
            };
            let is_result = (self.result_types.contains_key(value)
                || self
                    .result_types
                    .contains_key(&value.trim_start_matches('%').to_string()))
                || is_result_struct_value;

            // Check for regular struct instances FIRST before arrays/maps
            if struct_instance_name.is_some() && !is_result {
                // Print struct with actual field values
                let struct_name = struct_instance_name.unwrap();
                let struct_val = self.resolve_value(value);

                if let Some(metadata) = self.struct_metadata.get(&struct_name) {
                    // Check if struct_val is a pointer - if so, print with actual values
                    // If not, use a safe fallback showing field names only
                    let struct_ptr_opt = if struct_val.is_pointer_value() {
                        Some(struct_val.into_pointer_value())
                    } else {
                        // For non-pointer values, use fallback showing field names only
                        None
                    };

                    if let Some(struct_ptr) = struct_ptr_opt {
                        // We have a valid struct pointer - print with actual field values
                        // Start with struct name and opening brace
                        let opening = format!("{} {{ ", struct_name);
                        let opening_global = self
                            .builder
                            .build_global_string_ptr(&opening, "struct_opening")
                            .unwrap();
                        self.builder
                            .build_call(
                                printf_fn,
                                &[opening_global.as_pointer_value().into()],
                                "print_struct_opening",
                            )
                            .unwrap();

                        // Get the canonical struct type
                        let struct_type = if let Some(canonical_type) =
                            self.canonical_struct_types.get(&struct_name)
                        {
                            *canonical_type
                        } else {
                            // Fallback: reconstruct from metadata
                            let field_llvm_types: Vec<inkwell::types::BasicTypeEnum> = metadata
                                .field_types
                                .iter()
                                .map(|type_name| match type_name.as_str() {
                                    "Int" => self.context.i32_type().into(),
                                    "Float" => self.context.f64_type().into(),
                                    "Bool" => self.context.bool_type().into(),
                                    "Str" | "String" => self
                                        .context
                                        .ptr_type(inkwell::AddressSpace::default())
                                        .into(),
                                    _ => self.context.i32_type().into(),
                                })
                                .collect();
                            self.context.struct_type(&field_llvm_types, false)
                        };

                        for (field_idx, field_name) in metadata.field_names.iter().enumerate() {
                            // Print field name
                            let field_name_str = format!("{}: ", field_name);
                            let field_name_global = self
                                .builder
                                .build_global_string_ptr(&field_name_str, "field_name")
                                .unwrap();
                            self.builder
                                .build_call(
                                    printf_fn,
                                    &[field_name_global.as_pointer_value().into()],
                                    "print_field_name",
                                )
                                .unwrap();

                            // Get field type from metadata
                            let field_type = metadata
                                .field_types
                                .get(field_idx)
                                .map(|s| s.as_str())
                                .unwrap_or("");

                            // Get the field LLVM type from the struct type
                            let field_llvm_type = struct_type
                                .get_field_type_at_index(field_idx as u32)
                                .unwrap_or_else(|| self.context.i32_type().into());

                            // Access field using GEP
                            let field_ptr = self
                                .builder
                                .build_struct_gep(
                                    struct_type,
                                    struct_ptr,
                                    field_idx as u32,
                                    &format!("field_{}_ptr", field_name),
                                )
                                .unwrap();

                            // Load the field value
                            let field_value = self
                                .builder
                                .build_load(
                                    field_llvm_type,
                                    field_ptr,
                                    &format!("field_{}", field_name),
                                )
                                .unwrap();

                            // Print field value based on type
                            if field_type == "Str" || field_type == "String" {
                                let format_str = "%s";
                                let format_global = self
                                    .builder
                                    .build_global_string_ptr(format_str, "field_str_fmt")
                                    .unwrap();
                                self.builder
                                    .build_call(
                                        printf_fn,
                                        &[
                                            format_global.as_pointer_value().into(),
                                            field_value.into(),
                                        ],
                                        "print_field_str",
                                    )
                                    .unwrap();
                            } else if field_type == "Int" {
                                let format_str = "%d";
                                let format_global = self
                                    .builder
                                    .build_global_string_ptr(format_str, "field_int_fmt")
                                    .unwrap();
                                self.builder
                                    .build_call(
                                        printf_fn,
                                        &[
                                            format_global.as_pointer_value().into(),
                                            field_value.into(),
                                        ],
                                        "print_field_int",
                                    )
                                    .unwrap();
                            } else if field_type == "Float" {
                                let format_str = "%f";
                                let format_global = self
                                    .builder
                                    .build_global_string_ptr(format_str, "field_float_fmt")
                                    .unwrap();
                                self.builder
                                    .build_call(
                                        printf_fn,
                                        &[
                                            format_global.as_pointer_value().into(),
                                            field_value.into(),
                                        ],
                                        "print_field_float",
                                    )
                                    .unwrap();
                            } else if field_type == "Bool" {
                                let int_val = field_value.into_int_value();
                                let zero = self.context.i32_type().const_int(0, false);
                                let is_true = self
                                    .builder
                                    .build_int_compare(
                                        inkwell::IntPredicate::NE,
                                        int_val,
                                        zero,
                                        "is_true_field",
                                    )
                                    .unwrap();

                                let true_global = self
                                    .builder
                                    .build_global_string_ptr("true", "bool_true_field")
                                    .unwrap();
                                let false_global = self
                                    .builder
                                    .build_global_string_ptr("false", "bool_false_field")
                                    .unwrap();

                                let selected_str = self
                                    .builder
                                    .build_select(
                                        is_true,
                                        true_global.as_pointer_value(),
                                        false_global.as_pointer_value(),
                                        "select_bool_str_field",
                                    )
                                    .unwrap()
                                    .into_pointer_value();

                                self.builder
                                    .build_call(
                                        printf_fn,
                                        &[selected_str.into()],
                                        "print_bool_field",
                                    )
                                    .unwrap();
                            } else {
                                // Fallback for unknown types
                                let format_str = "%d";
                                let format_global = self
                                    .builder
                                    .build_global_string_ptr(format_str, "field_default_fmt")
                                    .unwrap();
                                self.builder
                                    .build_call(
                                        printf_fn,
                                        &[
                                            format_global.as_pointer_value().into(),
                                            field_value.into(),
                                        ],
                                        "print_field_default",
                                    )
                                    .unwrap();
                            }

                            // Print comma separator if not last field
                            if field_idx < metadata.field_names.len() - 1 {
                                let comma_global =
                                    self.builder.build_global_string_ptr(", ", "comma").unwrap();
                                self.builder
                                    .build_call(
                                        printf_fn,
                                        &[comma_global.as_pointer_value().into()],
                                        "print_comma",
                                    )
                                    .unwrap();
                            }
                        }

                        // Print closing brace
                        let closing = if idx < values.len() - 1 { " } " } else { " }" };
                        let closing_global = self
                            .builder
                            .build_global_string_ptr(closing, "struct_closing")
                            .unwrap();
                        self.builder
                            .build_call(
                                printf_fn,
                                &[closing_global.as_pointer_value().into()],
                                "print_struct_closing",
                            )
                            .unwrap();
                    } else {
                        // Fallback: show field names only (for structs returned from functions)
                        let field_list = metadata.field_names.join(", ");
                        let field_info = format!("{} {{ {} }}", struct_name, field_list);
                        let format_str = if idx < values.len() - 1 { "%s " } else { "%s" };
                        let format_global = self
                            .builder
                            .build_global_string_ptr(format_str, "print_struct_fmt")
                            .unwrap();
                        let info_global = self
                            .builder
                            .build_global_string_ptr(&field_info, "struct_info")
                            .unwrap();
                        self.builder
                            .build_call(
                                printf_fn,
                                &[
                                    format_global.as_pointer_value().into(),
                                    info_global.as_pointer_value().into(),
                                ],
                                "print_struct",
                            )
                            .unwrap();
                    }
                } else {
                    // Fallback if no metadata available
                    let placeholder = format!("<{}>", struct_name);
                    let format_str = if idx < values.len() - 1 { "%s " } else { "%s" };
                    let format_global = self
                        .builder
                        .build_global_string_ptr(format_str, "print_struct_fmt")
                        .unwrap();
                    let placeholder_global = self
                        .builder
                        .build_global_string_ptr(&placeholder, "struct_placeholder")
                        .unwrap();
                    self.builder
                        .build_call(
                            printf_fn,
                            &[
                                format_global.as_pointer_value().into(),
                                placeholder_global.as_pointer_value().into(),
                            ],
                            "print_struct_placeholder",
                        )
                        .unwrap();
                }
            } else if is_array {
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
            } else if is_result {
                // Special handling for Result structs: check tag at runtime
                let val = self.resolve_value(value);

                // Result struct is { i32 tag, ptr value }
                if val.is_struct_value() {
                    let result_struct = val.into_struct_value();

                    // Extract tag (field 0)
                    let tag = self
                        .builder
                        .build_extract_value(result_struct, 0, "result_tag")
                        .unwrap()
                        .into_int_value();

                    // Extract value (field 1)
                    let value_ptr = self
                        .builder
                        .build_extract_value(result_struct, 1, "result_value_ptr")
                        .unwrap()
                        .into_pointer_value();

                    // Create two blocks: one for Ok, one for Err
                    let func = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let ok_block = self.context.append_basic_block(func, "print_ok_block");
                    let err_block = self.context.append_basic_block(func, "print_err_block");
                    let continue_block = self
                        .context
                        .append_basic_block(func, "print_result_continue");

                    // Compare tag with 0: if 0 (Ok) branch to ok_block, else branch to err_block
                    let is_ok = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            tag,
                            self.context.i32_type().const_int(0, false),
                            "is_ok_tag",
                        )
                        .unwrap();

                    self.builder
                        .build_conditional_branch(is_ok, ok_block, err_block)
                        .unwrap();

                    // === OK BLOCK: Print the Ok value ===
                    self.builder.position_at_end(ok_block);
                    let ok_type = self.result_types.get(value).cloned().map(|(t, _)| t);

                    // Check if this is a multi-value tuple (not a single string)
                    let is_multi_tuple = ok_type.as_ref().map_or(false, |t| {
                        t.starts_with("Tuple(") && !t.starts_with("Tuple(Str)")
                    });

                    // Check if this is a struct type
                    let is_struct_type =
                        ok_type.as_ref().map_or(false, |t| t.starts_with("Struct("));

                    if is_multi_tuple {
                        // Multi-value tuple - can't print as single value, show placeholder
                        let placeholder = "<multi-value Ok>";
                        let format_str = if idx < values.len() - 1 { "%s " } else { "%s" };
                        let format_global = self
                            .builder
                            .build_global_string_ptr(format_str, "print_tuple_fmt")
                            .unwrap();
                        let placeholder_global = self
                            .builder
                            .build_global_string_ptr(placeholder, "tuple_placeholder")
                            .unwrap();
                        self.builder
                            .build_call(
                                printf_fn,
                                &[
                                    format_global.as_pointer_value().into(),
                                    placeholder_global.as_pointer_value().into(),
                                ],
                                "print_tuple",
                            )
                            .unwrap();
                    } else if is_struct_type {
                        // Struct - print struct representation
                        let struct_name = ok_type
                            .as_ref()
                            .map(|t| {
                                if t.starts_with("Struct(") && t.ends_with(")") {
                                    &t[7..t.len() - 1]
                                } else {
                                    "Unknown"
                                }
                            })
                            .unwrap_or("Unknown");

                        let placeholder = format!("<{}>", struct_name);
                        let format_str = if idx < values.len() - 1 { "%s " } else { "%s" };
                        let format_global = self
                            .builder
                            .build_global_string_ptr(format_str, "print_struct_fmt")
                            .unwrap();
                        let placeholder_global = self
                            .builder
                            .build_global_string_ptr(&placeholder, "struct_placeholder")
                            .unwrap();
                        self.builder
                            .build_call(
                                printf_fn,
                                &[
                                    format_global.as_pointer_value().into(),
                                    placeholder_global.as_pointer_value().into(),
                                ],
                                "print_struct",
                            )
                            .unwrap();
                    } else if ok_type
                        .as_ref()
                        .map_or(true, |t| t.contains("Str") || t.contains("String"))
                    {
                        // String - print as string
                        let format_str = if idx < values.len() - 1 { "%s " } else { "%s" };
                        let format_global = self
                            .builder
                            .build_global_string_ptr(format_str, "print_ok_fmt")
                            .unwrap();
                        self.builder
                            .build_call(
                                printf_fn,
                                &[format_global.as_pointer_value().into(), value_ptr.into()],
                                "print_ok_str",
                            )
                            .unwrap();
                    } else if ok_type.as_ref().map_or(false, |t| t.contains("Float")) {
                        // Float: convert pointer back to i64 then to f64
                        let i64_val = self
                            .builder
                            .build_ptr_to_int(value_ptr, self.context.i64_type(), "ptr_to_i64")
                            .unwrap();
                        let alloca = self
                            .builder
                            .build_alloca(self.context.i64_type(), "i64_tmp_ok")
                            .unwrap();
                        self.builder.build_store(alloca, i64_val).unwrap();
                        let f64_ptr = self
                            .builder
                            .build_pointer_cast(
                                alloca,
                                self.context.ptr_type(inkwell::AddressSpace::default()),
                                "f64_ptr_ok",
                            )
                            .unwrap();
                        let f64_val = self
                            .builder
                            .build_load(self.context.f64_type(), f64_ptr, "i64_as_float_ok")
                            .unwrap();
                        let format_str = if idx < values.len() - 1 { "%f " } else { "%f" };
                        let format_global = self
                            .builder
                            .build_global_string_ptr(format_str, "print_ok_fmt_float")
                            .unwrap();
                        self.builder
                            .build_call(
                                printf_fn,
                                &[format_global.as_pointer_value().into(), f64_val.into()],
                                "print_ok_float",
                            )
                            .unwrap();
                    } else if ok_type.as_ref().map_or(false, |t| t.contains("Bool")) {
                        // Bool: convert pointer back to i32, then print as "true"/"false"
                        let i64_val = self
                            .builder
                            .build_ptr_to_int(value_ptr, self.context.i64_type(), "ptr_to_i64_ok")
                            .unwrap();
                        let i32_val = self
                            .builder
                            .build_int_truncate(i64_val, self.context.i32_type(), "ptr_to_i32_ok")
                            .unwrap();

                        // Check if value is 0 (false) or non-zero (true)
                        let zero = self.context.i32_type().const_int(0, false);
                        let is_true = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::NE,
                                i32_val,
                                zero,
                                "is_true_ok",
                            )
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
                            .build_global_string_ptr(true_str, "bool_true_ok")
                            .unwrap();
                        let false_global = self
                            .builder
                            .build_global_string_ptr(false_str, "bool_false_ok")
                            .unwrap();

                        // Use select instruction to choose the correct string
                        let selected_str = self
                            .builder
                            .build_select(
                                is_true,
                                true_global.as_pointer_value(),
                                false_global.as_pointer_value(),
                                "select_bool_str_ok",
                            )
                            .unwrap()
                            .into_pointer_value();

                        // Print the selected string
                        self.builder
                            .build_call(printf_fn, &[selected_str.into()], "print_bool_ok")
                            .unwrap();
                    } else {
                        // Int or other: convert pointer back to i32
                        let i64_val = self
                            .builder
                            .build_ptr_to_int(value_ptr, self.context.i64_type(), "ptr_to_i64_ok")
                            .unwrap();
                        let i32_val = self
                            .builder
                            .build_int_truncate(i64_val, self.context.i32_type(), "ptr_to_i32_ok")
                            .unwrap();
                        let format_str = if idx < values.len() - 1 { "%d " } else { "%d" };
                        let format_global = self
                            .builder
                            .build_global_string_ptr(format_str, "print_ok_fmt_int")
                            .unwrap();
                        self.builder
                            .build_call(
                                printf_fn,
                                &[format_global.as_pointer_value().into(), i32_val.into()],
                                "print_ok_int",
                            )
                            .unwrap();
                    }
                    self.builder
                        .build_unconditional_branch(continue_block)
                        .unwrap();

                    // === ERR BLOCK: Print the error message ===
                    self.builder.position_at_end(err_block);
                    // Error value is a string pointer - print as string
                    let format_str = if idx < values.len() - 1 { "%s " } else { "%s" };
                    let format_global = self
                        .builder
                        .build_global_string_ptr(format_str, "print_err_fmt")
                        .unwrap();
                    self.builder
                        .build_call(
                            printf_fn,
                            &[format_global.as_pointer_value().into(), value_ptr.into()],
                            "print_err_str",
                        )
                        .unwrap();
                    self.builder
                        .build_unconditional_branch(continue_block)
                        .unwrap();

                    // Continue after the Result printing
                    self.builder.position_at_end(continue_block);
                } else {
                    // Fallback: shouldn't happen, but treat as pointer
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
            } else {
                let val = self.resolve_value(value);

                // Struct instances are now handled above, so skip redundant check
                if self.is_boolean_value(value) {
                    // Use a simple approach to avoid crashes
                    let bool_val = self.resolve_value(value);
                    let int_val = bool_val.into_int_value();

                    // Check if value is 0 (false) or non-zero (true)
                    let zero = self.context.i32_type().const_int(0, false);
                    let is_true = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::NE, int_val, zero, "is_true")
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
                            is_true,
                            true_global.as_pointer_value(),
                            false_global.as_pointer_value(),
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
                    // Check variable_types to determine if this is a string or struct pointer
                    let var_type = self.variable_types.get(value).cloned();
                    let is_string_type = var_type
                        .as_ref()
                        .map(|t| t == "Str" || t == "String")
                        .unwrap_or(true); // Default to string if unknown

                    // Check if it's a struct pointer (not a string)
                    let is_struct_ptr = var_type
                        .as_ref()
                        .map(|t| self.struct_metadata.contains_key(t))
                        .unwrap_or(false);

                    if is_struct_ptr {
                        // This is a struct pointer - print as pointer address (not a string)
                        let ptr_val = val.into_pointer_value();
                        let ptr_as_int = self
                            .builder
                            .build_ptr_to_int(ptr_val, self.context.i64_type(), "ptr_as_int")
                            .unwrap();
                        let format_str = if idx < values.len() - 1 {
                            "%lld "
                        } else {
                            "%lld"
                        };
                        let format_global = self
                            .builder
                            .build_global_string_ptr(format_str, "print_ptr_fmt")
                            .unwrap();
                        self.builder
                            .build_call(
                                printf_fn,
                                &[format_global.as_pointer_value().into(), ptr_as_int.into()],
                                "print_ptr_call",
                            )
                            .unwrap();
                    } else if is_string_type {
                        // This is a string pointer - check if null and print accordingly
                        let ptr_val = val.into_pointer_value();

                        // Check if pointer is null
                        let null_ptr = self
                            .context
                            .ptr_type(inkwell::AddressSpace::default())
                            .const_null();

                        let ptr_as_int = self
                            .builder
                            .build_ptr_to_int(ptr_val, self.context.i64_type(), "ptr_as_int")
                            .unwrap();
                        let null_as_int = self
                            .builder
                            .build_ptr_to_int(null_ptr, self.context.i64_type(), "null_as_int")
                            .unwrap();

                        let is_null = self
                            .builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                ptr_as_int,
                                null_as_int,
                                "is_null_ptr",
                            )
                            .unwrap();

                        // Create blocks for null and non-null cases
                        let func = self
                            .builder
                            .get_insert_block()
                            .unwrap()
                            .get_parent()
                            .unwrap();
                        let null_block = self.context.append_basic_block(func, "print_null");
                        let non_null_block =
                            self.context.append_basic_block(func, "print_non_null");
                        let cont_block = self.context.append_basic_block(func, "print_ptr_cont");

                        self.builder
                            .build_conditional_branch(is_null, null_block, non_null_block)
                            .unwrap();

                        // Null block: print "null"
                        self.builder.position_at_end(null_block);
                        let null_str = if idx < values.len() - 1 {
                            "null "
                        } else {
                            "null"
                        };
                        let null_global = self
                            .builder
                            .build_global_string_ptr(null_str, "null_str")
                            .unwrap();
                        self.builder
                            .build_call(
                                printf_fn,
                                &[null_global.as_pointer_value().into()],
                                "print_null",
                            )
                            .unwrap();
                        self.builder.build_unconditional_branch(cont_block).unwrap();

                        // Non-null block: print pointer as string
                        self.builder.position_at_end(non_null_block);
                        let format_str = if idx < values.len() - 1 { "%s " } else { "%s" };
                        let format_global = self
                            .builder
                            .build_global_string_ptr(format_str, "print_fmt")
                            .unwrap();

                        self.builder
                            .build_call(
                                printf_fn,
                                &[format_global.as_pointer_value().into(), ptr_val.into()],
                                "print_call",
                            )
                            .unwrap();
                        self.builder.build_unconditional_branch(cont_block).unwrap();

                        // Continue block
                        self.builder.position_at_end(cont_block);
                    } else {
                        // Unknown type - print as string (default behavior)
                        let ptr_val = val.into_pointer_value();
                        let format_str = if idx < values.len() - 1 { "%s " } else { "%s" };
                        let format_global = self
                            .builder
                            .build_global_string_ptr(format_str, "print_fmt")
                            .unwrap();
                        self.builder
                            .build_call(
                                printf_fn,
                                &[format_global.as_pointer_value().into(), ptr_val.into()],
                                "print_call",
                            )
                            .unwrap();
                    }
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
