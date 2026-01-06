use crate::codegen::core::CodeGen;
use inkwell::types::BasicType;
use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

impl<'ctx> CodeGen<'ctx> {
    /// Generate array with metadata, using explicit element type if provided
    pub fn generate_array_with_metadata_typed(
        &mut self,
        name: &str,
        elements: &[String],
        explicit_element_type: Option<&str>,
    ) -> Option<BasicValueEnum<'ctx>> {
        self.generate_array_with_metadata_inner(name, elements, explicit_element_type)
    }



    // Helper to convert Enum to String at runtime
    fn convert_enum_to_string_generic(
        &mut self,
        val: BasicValueEnum<'ctx>,
        type_str: &str,
    ) -> BasicValueEnum<'ctx> {
        // Strip Enum() wrapper
        let enum_name = if type_str.starts_with("Enum(") && type_str.ends_with(")") {
             &type_str[5..type_str.len() - 1]
        } else {
             type_str
        };

        // Get variants
        let variants = if let Some(v) = self.enum_variants.get(enum_name) {
             v.clone()
        } else {
             // Try stripping namespace (e.g. Models::Status -> Status)
            let simple_name = if let Some(idx) = enum_name.rfind("::") {
                &enum_name[idx + 2..]
            } else {
                enum_name
            };
            self.enum_variants.get(simple_name).cloned().unwrap_or_default()
        };

        if variants.is_empty() {
             let s = self
                 .builder
                 .build_global_string_ptr("Unknown", "enum_unknown")
                 .unwrap();
             return self.clone_ffi_string_to_rc(s.as_pointer_value()).into();
        }

        // Extract tag from enum struct {i32, ptr}
        let struct_val = val.into_struct_value();
        let tag = self.builder.build_extract_value(struct_val, 0, "enum_tag").unwrap().into_int_value();

        // Create switch
        let current_block = self.builder.get_insert_block().unwrap();
        let current_fn = current_block.get_parent().unwrap();
        let merge_block = self.context.append_basic_block(current_fn, "enum_to_str_merge");
        let default_block = self.context.append_basic_block(current_fn, "enum_to_str_default");

        let mut cases = Vec::new();
        let mut incoming: Vec<(BasicValueEnum<'ctx>, inkwell::basic_block::BasicBlock<'ctx>)> = Vec::new();

        for (name, idx) in &variants {
            let case_block = self.context.append_basic_block(current_fn, &format!("case_{}", name));
            cases.push((self.context.i32_type().const_int((*idx).into(), false), case_block));

            self.builder.position_at_end(case_block);
            let str_val = self
                .builder
                .build_global_string_ptr(name, &format!("enum_{}", name))
                .unwrap();
            let rc_str = self.clone_ffi_string_to_rc(str_val.as_pointer_value());
            incoming.push((rc_str.into(), case_block));
            self.builder.build_unconditional_branch(merge_block).unwrap();
        }

        self.builder.position_at_end(current_block);
        self.builder.build_switch(tag, default_block, &cases).unwrap();

        // Default case
        self.builder.position_at_end(default_block);
        let default_str = self
            .builder
            .build_global_string_ptr("Unknown", "enum_unknown")
            .unwrap();
        let default_rc = self.clone_ffi_string_to_rc(default_str.as_pointer_value());
        incoming.push((default_rc.into(), default_block));
        self.builder.build_unconditional_branch(merge_block).unwrap();

        // Merge
        self.builder.position_at_end(merge_block);
        let phi = self.builder.build_phi(self.context.ptr_type(AddressSpace::default()), "enum_str").unwrap();
        for (val, block) in &incoming {
             phi.add_incoming(&[(val, *block)]);
        }

        phi.as_basic_value()
    }

    pub fn generate_array_with_metadata(
        &mut self,
        name: &str,
        elements: &[String],
    ) -> Option<BasicValueEnum<'ctx>> {
        self.generate_array_with_metadata_inner(name, elements, None)
    }

    fn generate_array_with_metadata_inner(
        &mut self,
        name: &str,
        elements: &[String],
        explicit_element_type: Option<&str>,
    ) -> Option<BasicValueEnum<'ctx>> {
        // Handle spread elements by expanding them
        let mut expanded_elements: Vec<String> = Vec::new();
        for el in elements {
            if el.starts_with("SPREAD:") {
                // Extract the array to spread
                let array_name = &el[7..]; // Remove "SPREAD:" prefix
                                           // Get the array metadata to know how many elements to expand
                if let Some(meta) = self.array_metadata.get(array_name).cloned() {
                    // Generate ArrayGet instructions for each element
                    for i in 0..meta.length {
                        let elem_tmp = format!("{}[{}]", array_name, i);
                        expanded_elements.push(elem_tmp);
                    }
                }
            } else {
                expanded_elements.push(el.clone());
            }
        }

        // Check for mixed Enum types to handle auto-promotion to Array<Str>
        let mut enum_types = std::collections::HashSet::new();
        let mut has_enums = false;
        let mut has_strings = false;
        let mut has_others = false;

        for el in &expanded_elements {
            // Strategy 1: Check variable_types
            if let Some(type_str) = self.variable_types.get(el) {
                if type_str.starts_with("Enum(") {
                    enum_types.insert(type_str.clone());
                    has_enums = true;
                } else if type_str == "Str" || type_str == "String" {
                    has_strings = true;
                } else if type_str != "Unknown" {
                    has_others = true;
                }
            } else if el.starts_with('"') {
                // String literal
                has_strings = true;
            } else if el.contains("::") {
                // Strategy 2: Inline enum syntax like Priority::High or Status::Done
                let parts: Vec<&str> = el.split("::").collect();
                if let Some(enum_name) = parts.first() {
                    let clean_name = enum_name.trim_start_matches('%');
                    if self.enum_variants.contains_key(clean_name) {
                        enum_types.insert(format!("Enum({})", clean_name));
                        has_enums = true;
                    }
                }
            } else {
                // Strategy 3: Check if stripped element name matches an enum variant temp
                let stripped = el.trim_start_matches('%');
                if let Some(type_str) = self.variable_types.get(stripped) {
                    if type_str.starts_with("Enum(") {
                        enum_types.insert(type_str.clone());
                        has_enums = true;
                    }
                }
            }
        }

        // Force string array if we have ANY enums - for DB queries, enums must be strings
        // This includes: single enum arrays, mixed enum arrays, or enums mixed with other types
        let force_string_array = has_enums
            || (has_enums && has_strings)
            || (has_enums && has_others);

        let element_values: Vec<BasicValueEnum<'ctx>> = expanded_elements
            .iter()
            .map(|el| {
                // Handle indexed access notation for spread expansion
                if el.contains("[") && el.contains("]") {
                    let parts: Vec<&str> = el.split('[').collect();
                    if parts.len() == 2 {
                        let array_name = parts[0];
                        let index_str = parts[1].trim_end_matches(']');
                        if let Ok(index) = index_str.parse::<usize>() {
                            // Perform array access
                            let array_val = self.resolve_value(array_name);
                            if array_val.is_pointer_value() {
                                let array_ptr = array_val.into_pointer_value();
                                let index_val =
                                    self.context.i32_type().const_int(index as u64, false);

                                // Get element type from metadata
                                if let Some(meta) = self.array_metadata.get(array_name).cloned() {
                                    let elem_type = match meta.element_type.as_str() {
                                        "Int" => self.context.i32_type().as_basic_type_enum(),
                                        "Float" => self.context.f64_type().as_basic_type_enum(),
                                        "Bool" => self.context.bool_type().as_basic_type_enum(),
                                        "Str" => self
                                            .context
                                            .ptr_type(inkwell::AddressSpace::default())
                                            .as_basic_type_enum(),
                                        _ => self.context.i32_type().as_basic_type_enum(),
                                    };

                                    let elem_ptr = unsafe {
                                        self.builder
                                            .build_gep(
                                                elem_type,
                                                array_ptr,
                                                &[index_val],
                                                "spread_elem",
                                            )
                                            .unwrap()
                                    };
                                    let mut elem_val = self
                                        .builder
                                        .build_load(elem_type, elem_ptr, "spread_load")
                                        .unwrap();

                                    // Handle forced string conversion for spread elements
                                    if force_string_array {
                                        let meta_type = meta.element_type.clone();
                                        if meta_type.starts_with("Enum(") {
                                             elem_val = self.convert_enum_to_string_generic(elem_val, &meta_type);
                                        }
                                    }
                                    return elem_val;
                                }
                            }
                        }
                    }
                }

                let val = self.resolve_value(el);

                if force_string_array {
                    // Strategy 1: Check variable_types directly
                    if let Some(type_str) = self.variable_types.get(el).cloned() {
                        if type_str.starts_with("Enum(") {
                            return self.convert_enum_to_string_generic(val, &type_str);
                        }
                    }
                    // Strategy 2: Check inline enum syntax
                    else if el.contains("::") {
                        let parts: Vec<&str> = el.split("::").collect();
                        if let Some(enum_name) = parts.first() {
                            let clean_name = enum_name.trim_start_matches('%');
                            if self.enum_variants.contains_key(clean_name) {
                                let type_str = format!("Enum({})", clean_name);
                                return self.convert_enum_to_string_generic(val, &type_str);
                            }
                        }
                    }
                    // Strategy 3: Check with stripped % prefix
                    else {
                        let stripped = el.trim_start_matches('%');
                        if let Some(type_str) = self.variable_types.get(stripped).cloned() {
                            if type_str.starts_with("Enum(") {
                                return self.convert_enum_to_string_generic(val, &type_str);
                            }
                        }
                    }
                }

                val
            })
            .collect();

        // Allow empty arrays: use explicit type if provided, otherwise default to Int
        let elem_type = if force_string_array {
             self.context
                .ptr_type(inkwell::AddressSpace::default())
                .as_basic_type_enum()
        } else if element_values.is_empty() {
            if let Some(et) = explicit_element_type {
                // Map element type string to LLVM type
                match et {
                    "Int" => self.context.i32_type().as_basic_type_enum(),
                    "Float" => self.context.f64_type().as_basic_type_enum(),
                    "Bool" => self.context.bool_type().as_basic_type_enum(),
                    "Str" | "String" => self
                        .context
                        .ptr_type(inkwell::AddressSpace::default())
                        .as_basic_type_enum(),
                    _ if self.struct_metadata.contains_key(et) => {
                        // Struct arrays store pointers to structs
                        self.context
                            .ptr_type(inkwell::AddressSpace::default())
                            .as_basic_type_enum()
                    }
                    _ => self.context.i32_type().as_basic_type_enum(),
                }
            } else {
                self.context.i32_type().as_basic_type_enum()
            }
        } else {
            element_values[0].get_type()
        };

        let array_type = elem_type.array_type(expanded_elements.len() as u32);

        // Track string pointers
        let str_ptrs: Vec<BasicValueEnum<'ctx>> = element_values
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                *i < expanded_elements.len() && self.heap_strings.contains(&expanded_elements[*i])
            })
            .map(|(_, val)| *val)
            .collect();

        let contains_strings = !str_ptrs.is_empty();

        if contains_strings {
            self.composite_string_ptrs
                .insert(name.to_string(), str_ptrs);
        }

        // Store metadata
        // Use explicit element type if provided (from MIR), otherwise infer from LLVM type
        // This correctly distinguishes [true, false] (Bool) from [1, 0] (Int)
        let element_type_name = if let Some(et) = explicit_element_type {
            et
        } else if elem_type.is_float_type() {
            "Float"
        } else if elem_type.is_int_type() {
            // Check if it's actually a 1-bit integer (i1 = bool)
            if elem_type.into_int_type().get_bit_width() == 1 {
                "Bool"
            } else {
                "Int"
            }
        } else if elem_type.is_pointer_type() {
            "Str"
        } else {
            "Unknown"
        };

        let metadata = crate::codegen::ArrayMetadata {
            length: expanded_elements.len(),
            element_type: element_type_name.to_string(),
            contains_strings,
        };

        // Register metadata under EXTENSIVE name variations for better lookup
        // This is CRITICAL for arrays created inside loops
        let base_name = name.trim_start_matches('%').trim_end_matches("_array");
        let name_variations = vec![
            name.to_string(),
            name.trim_end_matches("_array").to_string(),
            name.trim_start_matches('%').to_string(),
            format!("{}_array", name),
            format!("{}_array", name.trim_start_matches('%')),
            format!("{}_array", base_name),
            base_name.to_string(),
            format!("{}item_array", base_name),
            format!("{}item", base_name),
        ];

        for variation in name_variations {
            self.array_metadata.insert(variation, metadata.clone());
        }

        // HEAP ALLOCATE with RC header and length field
        // Layout: [RC: 4 bytes][Length: 4 bytes][data...]
        let malloc_fn = self.get_or_declare_malloc();
        let array_size = array_type.size_of().unwrap();
        let header_size = self.context.i64_type().const_int(8, false); // RC + Length = 8 bytes (use i64)
        let total_size = self
            .builder
            .build_int_add(header_size, array_size, "total_size")
            .unwrap();

        let heap_ptr = self
            .builder
            .build_call(malloc_fn, &[total_size.into()], "heap_array")
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
                self.context.ptr_type(AddressSpace::default()),
                "rc_ptr",
            )
            .unwrap();
        self.builder
            .build_store(rc_ptr, self.context.i32_type().const_int(1, false))
            .unwrap();

        // Store array length at offset 4
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
                self.context.ptr_type(AddressSpace::default()),
                "len_ptr_cast",
            )
            .unwrap();
        self.builder
            .build_store(
                len_ptr_cast,
                self.context
                    .i32_type()
                    .const_int(expanded_elements.len() as u64, false),
            )
            .unwrap();

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

        // Cast to array type pointer
        let array_ptr = self
            .builder
            .build_pointer_cast(
                data_ptr,
                self.context.ptr_type(AddressSpace::default()),
                "array_ptr",
            )
            .unwrap();

        // Store elements
        for (i, val) in element_values.iter().enumerate() {
            let idx = self.context.i32_type().const_int(i as u64, false);
            let elem_ptr = unsafe {
                self.builder
                    .build_gep(
                        array_type,
                        array_ptr,
                        &[self.context.i32_type().const_zero(), idx],
                        &format!("elem_{}", i),
                    )
                    .unwrap()
            };
            self.builder.build_store(elem_ptr, *val).unwrap();
        }

        // CRITICAL: Remove element strings from heap_strings - they're now owned by the array
        // The array's composite_string_ptrs tracking will handle their cleanup
        for elem_name in elements {
            if self.heap_strings.contains(elem_name) {
                self.heap_strings.remove(elem_name);
            }
        }

        self.temp_values.insert(name.to_string(), data_ptr.into());
        self.heap_arrays.insert(name.to_string());

        // Store full heap pointer for tuple returns
        self.heap_pointers.insert(name.to_string(), heap_ptr);

        // CRITICAL: If there's already a symbol (alloca) for this name, update it too
        // This happens for cross-block variables that were pre-allocated
        // Without this, resolve_value will load from the uninitialized alloca instead of temp_values
        if let Some(sym) = self.symbols.get(name) {
            self.builder.build_store(sym.ptr, data_ptr).unwrap();
        }

        Some(data_ptr.into())
    }

    /// Helper implementations for array and map operations with RC
    pub fn get_array_length(&self, array_name: &str) -> inkwell::values::IntValue<'ctx> {
        // STEP 1: Direct metadata lookup
        if let Some(metadata) = self.array_metadata.get(array_name) {
            return self
                .context
                .i32_type()
                .const_int(metadata.length as u64, false);
        }

        // STEP 2: Try name variations
        let search_names = vec![
            array_name.trim_end_matches("_array").to_string(),
            format!("{}_array", array_name),
            array_name.trim_start_matches('%').to_string(),
        ];

        for search_name in &search_names {
            if let Some(metadata) = self.array_metadata.get(search_name) {
                return self
                    .context
                    .i32_type()
                    .const_int(metadata.length as u64, false);
            }
        }

        // STEP 3: Try pointer equality matching
        if let Some(sym) = self.symbols.get(array_name) {
            if let Ok(loaded) = self.builder.build_load(sym.ty, sym.ptr, "check_ptr") {
                if loaded.is_pointer_value() {
                    let ptr_val = loaded.into_pointer_value();

                    for (other_name, metadata) in &self.array_metadata {
                        if let Some(other_val) = self.temp_values.get(other_name) {
                            if other_val.is_pointer_value()
                                && other_val.into_pointer_value() == ptr_val
                            {
                                return self
                                    .context
                                    .i32_type()
                                    .const_int(metadata.length as u64, false);
                            }
                        }
                    }
                }
            }
        }

        // STEP 4: CRITICAL - Runtime length extraction from heap header
        // For dynamically created arrays (like innerarr), extract length at runtime

        // Try symbols first
        if let Some(sym) = self.symbols.get(array_name) {
            if let Ok(loaded) = self.builder.build_load(sym.ty, sym.ptr, "runtime_load") {
                if loaded.is_pointer_value() {
                    let arr_ptr = loaded.into_pointer_value();

                    // Array layout: [RC: 4 bytes][Length: 4 bytes][data at offset 8]
                    // arr_ptr points to data, so length is at offset -4
                    let len_ptr_result = unsafe {
                        self.builder.build_in_bounds_gep(
                            self.context.i8_type(),
                            arr_ptr,
                            &[self.context.i32_type().const_int((-4_i32) as u64, true)],
                            &format!("{}_runtime_len_ptr", array_name),
                        )
                    };

                    if let Ok(len_ptr) = len_ptr_result {
                        let len_ptr_cast_result = self.builder.build_pointer_cast(
                            len_ptr,
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            &format!("{}_len_ptr_cast", array_name),
                        );

                        if let Ok(len_ptr_cast) = len_ptr_cast_result {
                            if let Ok(runtime_len) = self.builder.build_load(
                                self.context.i32_type(),
                                len_ptr_cast,
                                &format!("{}_runtime_len", array_name),
                            ) {
                                return runtime_len.into_int_value();
                            }
                        }
                    }
                }
            }
        }

        // STEP 5: Try temp_values (for tuple-extracted arrays)
        if let Some(temp_val) = self.temp_values.get(array_name) {
            if temp_val.is_pointer_value() {
                let arr_ptr = temp_val.into_pointer_value();

                // Array layout: [RC: 4 bytes][Length: 4 bytes][data at offset 8]
                // arr_ptr points to data, so length is at offset -4
                let len_ptr_result = unsafe {
                    self.builder.build_in_bounds_gep(
                        self.context.i8_type(),
                        arr_ptr,
                        &[self.context.i32_type().const_int((-4_i32) as u64, true)],
                        &format!("{}_runtime_len_ptr", array_name),
                    )
                };

                if let Ok(len_ptr) = len_ptr_result {
                    let len_ptr_cast_result = self.builder.build_pointer_cast(
                        len_ptr,
                        self.context.ptr_type(inkwell::AddressSpace::default()),
                        &format!("{}_len_ptr_cast", array_name),
                    );

                    if let Ok(len_ptr_cast) = len_ptr_cast_result {
                        if let Ok(runtime_len) = self.builder.build_load(
                            self.context.i32_type(),
                            len_ptr_cast,
                            &format!("{}_runtime_len", array_name),
                        ) {
                            return runtime_len.into_int_value();
                        }
                    }
                }
            }
        }

        // FINAL FALLBACK: Return 0 to skip loop safely
        self.context.i32_type().const_int(0, false)
    }

    pub fn get_array_element_type(&self, array_name: &str) -> inkwell::types::BasicTypeEnum<'ctx> {
        if let Some(metadata) = self.array_metadata.get(array_name) {
            match metadata.element_type.as_str() {
                "Int" => self.context.i32_type().into(), // Only i32 for integers
                "Float" => self.context.f64_type().into(), // f64 for floating point
                "Bool" => self.context.i32_type().into(), // Bool stored as i32 (not i1) for consistency
                "Str" => self.context.ptr_type(AddressSpace::default()).into(),
                _ => {
                    // Check if it's a struct type - structs are stored as pointers
                    if self.struct_metadata.contains_key(&metadata.element_type) {
                        self.context.ptr_type(AddressSpace::default()).into()
                    } else {
                        self.context.i32_type().into()
                    }
                }
            }
        } else {
            // Fallback: check variable_types for Array(Type)
            // This handles arrays loaded from struct fields where array_metadata might be missing
            if let Some(type_str) = self.variable_types.get(array_name) {
                if type_str.starts_with("Array(") && type_str.ends_with(")") {
                    let inner_type = &type_str[6..type_str.len() - 1];
                    // Inline type resolution logic since type_string_to_llvm_type is not pub
                    return match inner_type {
                        "Int" | "i32" => self.context.i32_type().into(),
                        "Float" | "f64" => self.context.f64_type().into(),
                        "Bool" => self.context.i32_type().into(),
                        "Str" | "String" => self
                            .context
                            .ptr_type(inkwell::AddressSpace::default())
                            .into(),
                        _ => {
                            if inner_type.starts_with("Array")
                                || inner_type.starts_with("Map")
                                || inner_type.starts_with("Struct")
                                || self.struct_metadata.contains_key(inner_type)
                            {
                                self.context
                                    .ptr_type(inkwell::AddressSpace::default())
                                    .into()
                            } else {
                                self.context.i32_type().into()
                            }
                        }
                    };
                }
            }
            self.context.i32_type().into()
        }
    }

    /// Returns true if the array contains string elements.
    pub fn array_contains_strings(&self, array_name: &str) -> bool {
        if let Some(metadata) = self.array_metadata.get(array_name) {
            metadata.contains_strings
        } else {
            false
        }
    }

    /// Load array element with proper RC management for strings
    pub fn load_array_element_with_rc(
        &mut self,
        array_ptr: inkwell::values::PointerValue<'ctx>,
        index: inkwell::values::IntValue<'ctx>,
        elem_type: inkwell::types::BasicTypeEnum<'ctx>,
        is_string: bool,
    ) -> inkwell::values::BasicValueEnum<'ctx> {
        // GEP to get element pointer
        let elem_ptr = unsafe {
            self.builder.build_gep(
                elem_type.array_type(0), // We need actual array type here
                array_ptr,
                &[self.context.i32_type().const_zero(), index],
                "elem_ptr",
            )
        }
        .unwrap();

        // Load element
        let elem_val = self
            .builder
            .build_load(elem_type, elem_ptr, "elem")
            .unwrap();

        // If it's a heap-allocated string, increment RC
        if is_string {
            let str_ptr = elem_val.into_pointer_value();
            let rc_header = unsafe {
                self.builder.build_in_bounds_gep(
                    self.context.i8_type(),
                    str_ptr,
                    &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                    "rc_header",
                )
            }
            .unwrap();

            let incref = self.incref_fn.unwrap();
            self.builder
                .build_call(incref, &[rc_header.into()], "")
                .unwrap();
        }

        elem_val
    }

    /// Generate cleanup when exiting a loop (called from loops.rs)
    /// This generates the LLVM IR for cleanup but does NOT pop the loop context.
    /// The loop context is popped separately after the body block is fully generated.
    pub fn generate_loop_exit_cleanup(&mut self) {
        // Peek at the current loop context without popping it
        // We only generate cleanup code here; the actual context pop happens later
        if let Some(loop_ctx) = self.loop_stack.last().cloned() {
            // Clean up any heap-allocated loop variables (generate LLVM IR for decref)
            for var in &loop_ctx.loop_vars {
                if self.heap_strings.contains(var) {
                    self.emit_decref(var);
                }
                if self.heap_arrays.contains(var) {
                    // Free the array - __decref will handle element cleanup recursively
                    self.emit_decref(var);
                } else if self.heap_maps.contains(var) {
                    // Clean up strings in map if needed
                    if let Some(str_names) = self.composite_strings.get(var) {
                        for str_name in str_names.clone() {
                            if let Some(val) = self.temp_values.get(&str_name) {
                                if val.is_pointer_value() {
                                    let data_ptr = val.into_pointer_value();
                                    let rc_header = unsafe {
                                        self.builder.build_in_bounds_gep(
                                            self.context.i8_type(),
                                            data_ptr,
                                            &[self
                                                .context
                                                .i32_type()
                                                .const_int((-8_i32) as u64, true)],
                                            "rc_header",
                                        )
                                    }
                                    .unwrap();

                                    let decref = self.decref_fn.unwrap();
                                    self.builder
                                        .build_call(decref, &[rc_header.into()], "")
                                        .unwrap();
                                }
                            }
                        }
                    }
                    self.emit_decref(var);
                }
            }
            // NOTE: We do NOT remove from heap_strings/heap_arrays/heap_maps here
            // because that would affect codegen state. Those are cleaned up in finalize_loop_cleanup.
        }
    }

    /// Finalize loop cleanup: pop the loop context and remove metadata.
    /// This should be called AFTER the loop body block has been fully generated.
    pub fn finalize_loop_cleanup(&mut self) {
        if let Some(loop_ctx) = self.exit_loop() {
            for var in &loop_ctx.loop_vars {
                // Now safe to remove from tracking sets
                self.heap_strings.remove(var);
                self.heap_arrays.remove(var);
                self.heap_maps.remove(var);
                // Remove metadata
                self.symbols.remove(var);
                self.array_metadata.remove(var);
                self.map_metadata.remove(var);
                self.temp_values.remove(var);
                self.variable_types.remove(var);
                self.struct_instance_types.remove(var);
                self.arrayget_sources.remove(var);
                self.loop_local_vars.remove(var);
                self.heap_pointers.remove(var);
                self.composite_strings.remove(var);
                self.composite_string_ptrs.remove(var);
            }
        }
    }

    /// Helper method to print an array
    pub fn print_array(&mut self, array_name: &str) {
        let printf_fn = self.get_or_declare_printf();

        // Get array metadata - try multiple name variations
        let mut metadata = self.array_metadata.get(array_name).cloned();

        // If not found, try variations
        if metadata.is_none() {
            let variations = vec![
                array_name.trim_start_matches('%').to_string(),
                array_name.trim_end_matches("_array").to_string(),
                format!("{}_array", array_name),
                format!("{}_array", array_name.trim_start_matches('%')),
            ];

            for var in variations {
                if let Some(meta) = self.array_metadata.get(&var).cloned() {
                    metadata = Some(meta);
                    break;
                }
            }
        }

        if let Some(metadata) = metadata {
            // Get pointer to the array data
            let array_ptr = if self.symbols.contains_key(array_name) {
                // Variable case: resolve_pointer gives us the alloca,
                // we need to load the actual array pointer from it
                let var_alloca = self.resolve_pointer(array_name);
                self.builder
                    .build_load(
                        self.context.ptr_type(AddressSpace::default()),
                        var_alloca,
                        "array_data_ptr",
                    )
                    .unwrap()
                    .into_pointer_value()
            } else {
                // For temporary arrays, resolve_value should work
                self.resolve_value(array_name).into_pointer_value()
            };

            // Check if array pointer is null (empty array case)
            let current_fn = self
                .builder
                .get_insert_block()
                .unwrap()
                .get_parent()
                .unwrap();
            let null_block = self
                .context
                .append_basic_block(current_fn, "print_null_array");
            let non_null_block = self
                .context
                .append_basic_block(current_fn, "print_non_null_array");
            let after_print_block = self
                .context
                .append_basic_block(current_fn, "after_print_array");

            let is_null = self
                .builder
                .build_is_null(array_ptr, "is_null_array")
                .unwrap();
            self.builder
                .build_conditional_branch(is_null, null_block, non_null_block)
                .unwrap();

            // Null case: just print []
            self.builder.position_at_end(null_block);
            let empty_array_str = self
                .builder
                .build_global_string_ptr("[]", "empty_array_str")
                .unwrap();
            self.builder
                .build_call(printf_fn, &[empty_array_str.as_pointer_value().into()], "")
                .unwrap();
            self.builder
                .build_unconditional_branch(after_print_block)
                .unwrap();

            // Non-null case: print array contents
            self.builder.position_at_end(non_null_block);

            // Print opening bracket
            let open_bracket = self
                .builder
                .build_global_string_ptr("[", "open_bracket")
                .unwrap();
            self.builder
                .build_call(printf_fn, &[open_bracket.as_pointer_value().into()], "")
                .unwrap();

            // Check if element type is a struct
            let is_struct_element = self.struct_metadata.contains_key(&metadata.element_type);

            let elem_type = if metadata.element_type == "Str" {
                self.context
                    .ptr_type(AddressSpace::default())
                    .as_basic_type_enum()
            } else if metadata.element_type == "Float" {
                self.context.f64_type().as_basic_type_enum()
            } else if is_struct_element {
                // Struct elements are stored as pointers
                self.context
                    .ptr_type(AddressSpace::default())
                    .as_basic_type_enum()
            } else {
                self.context.i32_type().as_basic_type_enum()
            };

            let array_type = elem_type.array_type(metadata.length as u32);
            let typed_array_ptr = self
                .builder
                .build_pointer_cast(
                    array_ptr,
                    self.context.ptr_type(AddressSpace::default()),
                    "typed_array_ptr",
                )
                .unwrap();

            // Read heap length from header
            // Array layout: [RC: 4 bytes][Length: 4 bytes][data...]
            // Data pointer is at offset +8, so RC header is at -8
            // Length field is at -8 + 4 = -4
            let rc_header_ptr = unsafe {
                self.builder
                    .build_gep(
                        self.context.i8_type(),
                        array_ptr,
                        &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                        "rc_header_ptr_print",
                    )
                    .unwrap()
            };

            let len_field_ptr = unsafe {
                self.builder
                    .build_gep(
                        self.context.i8_type(),
                        rc_header_ptr,
                        &[self.context.i32_type().const_int(4, false)],
                        "len_field_ptr_print",
                    )
                    .unwrap()
            };

            let len_ptr_cast = self
                .builder
                .build_pointer_cast(
                    len_field_ptr,
                    self.context.ptr_type(AddressSpace::default()),
                    "len_ptr_cast_print",
                )
                .unwrap();
            let heap_len = self
                .builder
                .build_load(self.context.i32_type(), len_ptr_cast, "heap_len_print")
                .unwrap()
                .into_int_value();

            // Create a dynamic loop to print elements based on runtime heap_len
            let print_fn = self
                .builder
                .get_insert_block()
                .unwrap()
                .get_parent()
                .unwrap();

            let loop_header = self
                .context
                .append_basic_block(print_fn, "print_loop_header");
            let loop_body = self.context.append_basic_block(print_fn, "print_loop_body");
            let loop_end = self.context.append_basic_block(print_fn, "print_loop_end");

            // Create a loop counter
            let counter_alloca = self
                .builder
                .build_alloca(self.context.i32_type(), "print_counter")
                .unwrap();
            self.builder
                .build_store(counter_alloca, self.context.i32_type().const_zero())
                .unwrap();

            // Jump to loop header
            self.builder
                .build_unconditional_branch(loop_header)
                .unwrap();

            // Loop header: check if counter < heap_len
            self.builder.position_at_end(loop_header);
            let counter_val = self
                .builder
                .build_load(self.context.i32_type(), counter_alloca, "counter")
                .unwrap()
                .into_int_value();
            let should_continue = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::ULT,
                    counter_val,
                    heap_len,
                    "should_continue",
                )
                .unwrap();
            self.builder
                .build_conditional_branch(should_continue, loop_body, loop_end)
                .unwrap();

            // Loop body: print element at counter_val index
            self.builder.position_at_end(loop_body);

            let elem_ptr = unsafe {
                self.builder.build_gep(
                    array_type,
                    typed_array_ptr,
                    &[self.context.i32_type().const_zero(), counter_val],
                    "elem_ptr",
                )
            }
            .unwrap();

            let elem_val = self
                .builder
                .build_load(elem_type, elem_ptr, "elem")
                .unwrap();

            // Determine if this is the last element
            let next_counter = self
                .builder
                .build_int_add(
                    counter_val,
                    self.context.i32_type().const_int(1, false),
                    "next_counter",
                )
                .unwrap();
            let is_last_element = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::EQ,
                    next_counter,
                    heap_len,
                    "is_last_element",
                )
                .unwrap();

            // Print the element based on its type
            if is_struct_element {
                // Handle struct array element - print struct with its fields
                let struct_name = metadata.element_type.clone();
                let struct_ptr = elem_val.into_pointer_value();

                if let Some(struct_meta) = self.struct_metadata.get(&struct_name).cloned() {
                    // Print struct name and opening brace
                    let opening = format!("{} {{ ", struct_name);
                    let opening_global = self
                        .builder
                        .build_global_string_ptr(&opening, "arr_struct_opening")
                        .unwrap();
                    self.builder
                        .build_call(printf_fn, &[opening_global.as_pointer_value().into()], "")
                        .unwrap();

                    // Get canonical struct type for GEP
                    if let Some(canonical_type) =
                        self.canonical_struct_types.get(&struct_name).cloned()
                    {
                        for (field_idx, field_name) in struct_meta.field_names.iter().enumerate() {
                            let field_type = struct_meta
                                .field_types
                                .get(field_idx)
                                .map(|s| s.as_str())
                                .unwrap_or("");

                            // Print field name
                            let field_label = format!("{}: ", field_name);
                            let field_label_global = self
                                .builder
                                .build_global_string_ptr(&field_label, "arr_field_label")
                                .unwrap();
                            self.builder
                                .build_call(
                                    printf_fn,
                                    &[field_label_global.as_pointer_value().into()],
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
                                    &format!("arr_field_{}_ptr", field_name),
                                )
                                .unwrap();

                            // Get field LLVM type
                            let field_llvm_type = canonical_type
                                .get_field_type_at_index(field_idx as u32)
                                .unwrap_or_else(|| self.context.i32_type().into());

                            // Load and print field value
                            let field_val = self
                                .builder
                                .build_load(
                                    field_llvm_type,
                                    field_ptr,
                                    &format!("arr_field_{}", field_name),
                                )
                                .unwrap();

                            if field_type == "Str" || field_type == "String" {
                                let fmt = self
                                    .builder
                                    .build_global_string_ptr("\"%s\"", "arr_str_fmt")
                                    .unwrap();
                                self.builder
                                    .build_call(
                                        printf_fn,
                                        &[fmt.as_pointer_value().into(), field_val.into()],
                                        "",
                                    )
                                    .unwrap();
                            } else if field_type == "Int" {
                                let fmt = self
                                    .builder
                                    .build_global_string_ptr("%d", "arr_int_fmt")
                                    .unwrap();
                                self.builder
                                    .build_call(
                                        printf_fn,
                                        &[fmt.as_pointer_value().into(), field_val.into()],
                                        "",
                                    )
                                    .unwrap();
                            } else if field_type == "Float" {
                                let fmt = self
                                    .builder
                                    .build_global_string_ptr("%f", "arr_float_fmt")
                                    .unwrap();
                                self.builder
                                    .build_call(
                                        printf_fn,
                                        &[fmt.as_pointer_value().into(), field_val.into()],
                                        "",
                                    )
                                    .unwrap();
                            } else if field_type == "Bool" {
                                let int_val = field_val.into_int_value();
                                let zero = self.context.i32_type().const_int(0, false);
                                let is_true = self
                                    .builder
                                    .build_int_compare(
                                        inkwell::IntPredicate::NE,
                                        int_val,
                                        zero,
                                        "arr_bool_check",
                                    )
                                    .unwrap();
                                let true_str = self
                                    .builder
                                    .build_global_string_ptr("true", "arr_true")
                                    .unwrap();
                                let false_str = self
                                    .builder
                                    .build_global_string_ptr("false", "arr_false")
                                    .unwrap();
                                let bool_str = self
                                    .builder
                                    .build_select(
                                        is_true,
                                        true_str.as_pointer_value(),
                                        false_str.as_pointer_value(),
                                        "arr_bool_sel",
                                    )
                                    .unwrap()
                                    .into_pointer_value();
                                self.builder
                                    .build_call(printf_fn, &[bool_str.into()], "")
                                    .unwrap();
                            } else if self.enum_table.contains_key(field_type) {
                                // Enum field - print variant name
                                // Extract tag from enum struct { i32 tag, ptr payload }
                                let tag_val = if field_val.is_struct_value() {
                                    self.builder
                                        .build_extract_value(
                                            field_val.into_struct_value(),
                                            0,
                                            "arr_enum_tag",
                                        )
                                        .unwrap()
                                        .into_int_value()
                                } else {
                                    field_val.into_int_value()
                                };
                                if let Some(variants) = self.enum_table.get(field_type) {
                                    let mut sorted_variants: Vec<_> = variants.iter().collect();
                                    sorted_variants.sort_by_key(|(name, _)| *name);

                                    // For simplicity, print as EnumName::tag_value
                                    // A full implementation would use switch, but for now just print the tag
                                    let enum_fmt = self
                                        .builder
                                        .build_global_string_ptr(
                                            &format!("{}::", field_type),
                                            "arr_enum_fmt",
                                        )
                                        .unwrap();
                                    self.builder
                                        .build_call(
                                            printf_fn,
                                            &[enum_fmt.as_pointer_value().into()],
                                            "",
                                        )
                                        .unwrap();
                                    let tag_fmt = self
                                        .builder
                                        .build_global_string_ptr("%d", "arr_tag_fmt")
                                        .unwrap();
                                    self.builder
                                        .build_call(
                                            printf_fn,
                                            &[tag_fmt.as_pointer_value().into(), tag_val.into()],
                                            "",
                                        )
                                        .unwrap();
                                } else {
                                    let fmt = self
                                        .builder
                                        .build_global_string_ptr("%d", "arr_enum_tag")
                                        .unwrap();
                                    self.builder
                                        .build_call(
                                            printf_fn,
                                            &[fmt.as_pointer_value().into(), field_val.into()],
                                            "",
                                        )
                                        .unwrap();
                                }
                            } else {
                                // Unknown type - print as int
                                let fmt = self
                                    .builder
                                    .build_global_string_ptr("%d", "arr_unknown_fmt")
                                    .unwrap();
                                self.builder
                                    .build_call(
                                        printf_fn,
                                        &[fmt.as_pointer_value().into(), field_val.into()],
                                        "",
                                    )
                                    .unwrap();
                            }

                            // Print comma if not last field
                            if field_idx < struct_meta.field_names.len() - 1 {
                                let comma = self
                                    .builder
                                    .build_global_string_ptr(", ", "arr_field_comma")
                                    .unwrap();
                                self.builder
                                    .build_call(printf_fn, &[comma.as_pointer_value().into()], "")
                                    .unwrap();
                            }
                        }
                    }

                    // Print closing brace
                    let closing_global = self
                        .builder
                        .build_global_string_ptr(" }", "arr_struct_closing")
                        .unwrap();
                    self.builder
                        .build_call(printf_fn, &[closing_global.as_pointer_value().into()], "")
                        .unwrap();
                } else {
                    // No metadata - just print placeholder
                    let placeholder = format!("<{}>", struct_name);
                    let placeholder_global = self
                        .builder
                        .build_global_string_ptr(&placeholder, "arr_struct_placeholder")
                        .unwrap();
                    self.builder
                        .build_call(
                            printf_fn,
                            &[placeholder_global.as_pointer_value().into()],
                            "",
                        )
                        .unwrap();
                }

                // Print comma if not last element
                let comma_block = self
                    .context
                    .append_basic_block(print_fn, "arr_struct_comma");
                let no_comma_block = self
                    .context
                    .append_basic_block(print_fn, "arr_struct_no_comma");
                let after_comma_block = self
                    .context
                    .append_basic_block(print_fn, "arr_struct_after_comma");

                self.builder
                    .build_conditional_branch(is_last_element, no_comma_block, comma_block)
                    .unwrap();

                self.builder.position_at_end(comma_block);
                let comma = self
                    .builder
                    .build_global_string_ptr(", ", "arr_elem_comma")
                    .unwrap();
                self.builder
                    .build_call(printf_fn, &[comma.as_pointer_value().into()], "")
                    .unwrap();
                self.builder
                    .build_unconditional_branch(after_comma_block)
                    .unwrap();

                self.builder.position_at_end(no_comma_block);
                self.builder
                    .build_unconditional_branch(after_comma_block)
                    .unwrap();

                self.builder.position_at_end(after_comma_block);
            } else if metadata.element_type == "Str" {
                let with_comma = self
                    .builder
                    .build_global_string_ptr("\"%s\", ", "array_elem_fmt_comma")
                    .unwrap();
                let without_comma = self
                    .builder
                    .build_global_string_ptr("\"%s\"", "array_elem_fmt_no_comma")
                    .unwrap();

                let format_global = self
                    .builder
                    .build_select(
                        is_last_element,
                        without_comma.as_pointer_value(),
                        with_comma.as_pointer_value(),
                        "select_format_str",
                    )
                    .unwrap()
                    .into_pointer_value();

                self.builder
                    .build_call(printf_fn, &[format_global.into(), elem_val.into()], "")
                    .unwrap();
            } else if metadata.element_type == "Bool" {
                // Check if bool value is true (non-zero) or false (zero)
                let int_val = elem_val.into_int_value();
                let zero = self.context.i32_type().const_int(0, false);
                let is_true = self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::NE, int_val, zero, "bool_is_true")
                    .unwrap();

                // Build format strings for true/false with and without commas
                let true_with_comma = self
                    .builder
                    .build_global_string_ptr("true, ", "array_bool_true_comma")
                    .unwrap();
                let true_without_comma = self
                    .builder
                    .build_global_string_ptr("true", "array_bool_true_no_comma")
                    .unwrap();
                let false_with_comma = self
                    .builder
                    .build_global_string_ptr("false, ", "array_bool_false_comma")
                    .unwrap();
                let false_without_comma = self
                    .builder
                    .build_global_string_ptr("false", "array_bool_false_no_comma")
                    .unwrap();

                // Select based on is_true
                let true_format = self
                    .builder
                    .build_select(
                        is_last_element,
                        true_without_comma.as_pointer_value(),
                        true_with_comma.as_pointer_value(),
                        "select_true_format",
                    )
                    .unwrap()
                    .into_pointer_value();

                let false_format = self
                    .builder
                    .build_select(
                        is_last_element,
                        false_without_comma.as_pointer_value(),
                        false_with_comma.as_pointer_value(),
                        "select_false_format",
                    )
                    .unwrap()
                    .into_pointer_value();

                let final_format = self
                    .builder
                    .build_select(is_true, true_format, false_format, "select_bool_format")
                    .unwrap()
                    .into_pointer_value();

                self.builder
                    .build_call(printf_fn, &[final_format.into()], "")
                    .unwrap();
            } else if metadata.element_type == "Float" {
                let with_comma = self
                    .builder
                    .build_global_string_ptr("%f, ", "array_elem_fmt_comma_float")
                    .unwrap();
                let without_comma = self
                    .builder
                    .build_global_string_ptr("%f", "array_elem_fmt_no_comma_float")
                    .unwrap();

                let format_global = self
                    .builder
                    .build_select(
                        is_last_element,
                        without_comma.as_pointer_value(),
                        with_comma.as_pointer_value(),
                        "select_format_str_float",
                    )
                    .unwrap()
                    .into_pointer_value();

                self.builder
                    .build_call(printf_fn, &[format_global.into(), elem_val.into()], "")
                    .unwrap();
            } else {
                let with_comma = self
                    .builder
                    .build_global_string_ptr("%d, ", "array_elem_fmt_comma_int")
                    .unwrap();
                let without_comma = self
                    .builder
                    .build_global_string_ptr("%d", "array_elem_fmt_no_comma_int")
                    .unwrap();

                let format_global = self
                    .builder
                    .build_select(
                        is_last_element,
                        without_comma.as_pointer_value(),
                        with_comma.as_pointer_value(),
                        "select_format_str_int",
                    )
                    .unwrap()
                    .into_pointer_value();

                self.builder
                    .build_call(printf_fn, &[format_global.into(), elem_val.into()], "")
                    .unwrap();
            }

            // Increment counter and loop back
            self.builder
                .build_store(counter_alloca, next_counter)
                .unwrap();
            self.builder
                .build_unconditional_branch(loop_header)
                .unwrap();

            // Loop end - exit the loop
            self.builder.position_at_end(loop_end);

            // Print closing bracket
            let close_bracket = self
                .builder
                .build_global_string_ptr("]", "close_bracket")
                .unwrap();
            self.builder
                .build_call(printf_fn, &[close_bracket.as_pointer_value().into()], "")
                .unwrap();

            // Branch to after block
            self.builder
                .build_unconditional_branch(after_print_block)
                .unwrap();

            // Position at after block for continuation
            self.builder.position_at_end(after_print_block);
        }
    }

    pub fn print_struct_with_fields(
        &mut self,
        struct_var: &str,
        struct_name: &str,
        idx: usize,
        total_values: usize,
    ) {
        let printf_fn = self.get_or_declare_printf();

        // Get struct metadata
        if let Some(metadata) = self.struct_metadata.get(struct_name).cloned() {
            // Try to get canonical struct type - if not available, just print struct name with field names
            if let Some(canonical_type) = self.canonical_struct_types.get(struct_name).cloned() {
                // Print struct name and opening brace
                let struct_header = format!("{} {{ ", struct_name);
                let header_fmt = self
                    .builder
                    .build_global_string_ptr("%s", "struct_header_fmt")
                    .unwrap();
                let header_str = self
                    .builder
                    .build_global_string_ptr(&struct_header, "struct_header_str")
                    .unwrap();
                self.builder
                    .build_call(
                        printf_fn,
                        &[
                            header_fmt.as_pointer_value().into(),
                            header_str.as_pointer_value().into(),
                        ],
                        "print_struct_header",
                    )
                    .unwrap();

                // Try to get the struct value - it may be in temp_values or symbols
                let struct_val_opt = self.temp_values.get(struct_var).cloned().or_else(|| {
                    if let Some(sym) = self.symbols.get(struct_var) {
                        Some(
                            self.builder
                                .build_load(sym.ty, sym.ptr, &format!("load_{}", struct_var))
                                .unwrap(),
                        )
                    } else {
                        None
                    }
                });

                if let Some(struct_val) = struct_val_opt {
                    // Handle struct value - could be pointer or direct struct
                    let struct_ptr = if struct_val.is_pointer_value() {
                        struct_val.into_pointer_value()
                    } else {
                        // If it's a direct struct value, allocate and store it
                        let struct_alloca = self
                            .builder
                            .build_alloca(canonical_type, "struct_print_alloca")
                            .unwrap();
                        self.builder.build_store(struct_alloca, struct_val).unwrap();
                        struct_alloca
                    };

                    // Iterate through fields and print each one
                    for (field_idx, (field_name, field_type)) in metadata
                        .field_names
                        .iter()
                        .zip(metadata.field_types.iter())
                        .enumerate()
                    {
                        // Get field pointer using struct GEP
                        let field_ptr = self
                            .builder
                            .build_struct_gep(
                                canonical_type,
                                struct_ptr,
                                field_idx as u32,
                                &format!("field_{}_ptr", field_name),
                            )
                            .unwrap();

                        // Print field name and colon
                        let field_label = format!("{}: ", field_name);
                        let label_fmt = self
                            .builder
                            .build_global_string_ptr("%s", "field_label_fmt")
                            .unwrap();
                        let label_str = self
                            .builder
                            .build_global_string_ptr(&field_label, "field_label_str")
                            .unwrap();
                        self.builder
                            .build_call(
                                printf_fn,
                                &[
                                    label_fmt.as_pointer_value().into(),
                                    label_str.as_pointer_value().into(),
                                ],
                                "print_field_label",
                            )
                            .unwrap();

                        // Load field value and print based on type
                        match field_type.as_str() {
                            "Str" => {
                                let field_val = self
                                    .builder
                                    .build_load(
                                        self.context.ptr_type(AddressSpace::default()),
                                        field_ptr,
                                        &format!("field_{}_val", field_name),
                                    )
                                    .unwrap()
                                    .into_pointer_value();

                                let fmt = self
                                    .builder
                                    .build_global_string_ptr("\"%s\"", "str_field_fmt")
                                    .unwrap();
                                self.builder
                                    .build_call(
                                        printf_fn,
                                        &[fmt.as_pointer_value().into(), field_val.into()],
                                        "print_str_field",
                                    )
                                    .unwrap();
                            }
                            "Int" => {
                                // Check if this is an i64 field (for FileMetadata: size, created, modified, accessed)
                                let actual_field_type = canonical_type
                                    .get_field_type_at_index(field_idx as u32)
                                    .unwrap();

                                if actual_field_type.is_int_type()
                                    && actual_field_type.into_int_type().get_bit_width() == 64
                                {
                                    // i64 field
                                    let field_val = self
                                        .builder
                                        .build_load(
                                            self.context.i64_type(),
                                            field_ptr,
                                            &format!("field_{}_val", field_name),
                                        )
                                        .unwrap()
                                        .into_int_value();

                                    // Special handling for "size" field - print with " bytes" unit
                                    if field_name == "size" && struct_name == "FileMetadata" {
                                        let fmt_bytes = self
                                            .builder
                                            .build_global_string_ptr("%lld bytes", "fmt_size_bytes")
                                            .unwrap();
                                        self.builder
                                            .build_call(
                                                printf_fn,
                                                &[
                                                    fmt_bytes.as_pointer_value().into(),
                                                    field_val.into(),
                                                ],
                                                "print_size_bytes",
                                            )
                                            .unwrap();
                                    } else {
                                        // Regular i64 field - just print the number
                                        let fmt = self
                                            .builder
                                            .build_global_string_ptr("%lld", "i64_field_fmt")
                                            .unwrap();
                                        self.builder
                                            .build_call(
                                                printf_fn,
                                                &[fmt.as_pointer_value().into(), field_val.into()],
                                                "print_i64_field",
                                            )
                                            .unwrap();
                                    }
                                } else {
                                    // i32 field - use %d format
                                    let field_val = self
                                        .builder
                                        .build_load(
                                            self.context.i32_type(),
                                            field_ptr,
                                            &format!("field_{}_val", field_name),
                                        )
                                        .unwrap()
                                        .into_int_value();

                                    let fmt = self
                                        .builder
                                        .build_global_string_ptr("%d", "int_field_fmt")
                                        .unwrap();
                                    self.builder
                                        .build_call(
                                            printf_fn,
                                            &[fmt.as_pointer_value().into(), field_val.into()],
                                            "print_int_field",
                                        )
                                        .unwrap();
                                }
                            }
                            "Float" => {
                                let field_val = self
                                    .builder
                                    .build_load(
                                        self.context.f64_type(),
                                        field_ptr,
                                        &format!("field_{}_val", field_name),
                                    )
                                    .unwrap()
                                    .into_float_value();

                                let fmt = self
                                    .builder
                                    .build_global_string_ptr("%.15g", "float_field_fmt")
                                    .unwrap();
                                self.builder
                                    .build_call(
                                        printf_fn,
                                        &[fmt.as_pointer_value().into(), field_val.into()],
                                        "print_float_field",
                                    )
                                    .unwrap();
                            }
                            "Bool" => {
                                let field_val = self
                                    .builder
                                    .build_load(
                                        self.context.bool_type(),
                                        field_ptr,
                                        &format!("field_{}_val", field_name),
                                    )
                                    .unwrap()
                                    .into_int_value();

                                let true_str = self
                                    .builder
                                    .build_global_string_ptr("true", "bool_true_str")
                                    .unwrap();
                                let false_str = self
                                    .builder
                                    .build_global_string_ptr("false", "bool_false_str")
                                    .unwrap();

                                let is_true = self
                                    .builder
                                    .build_int_compare(
                                        inkwell::IntPredicate::NE,
                                        field_val,
                                        field_val.get_type().const_zero(),
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

                                let fmt = self
                                    .builder
                                    .build_global_string_ptr("%s", "bool_field_fmt")
                                    .unwrap();
                                self.builder
                                    .build_call(
                                        printf_fn,
                                        &[fmt.as_pointer_value().into(), selected.into()],
                                        "print_bool_field",
                                    )
                                    .unwrap();
                            }
                            _ => {
                                // Unknown type - print placeholder
                                let placeholder = self
                                    .builder
                                    .build_global_string_ptr("?", "unknown_field_fmt")
                                    .unwrap();
                                let fmt = self
                                    .builder
                                    .build_global_string_ptr("%s", "unknown_fmt")
                                    .unwrap();
                                self.builder
                                    .build_call(
                                        printf_fn,
                                        &[
                                            fmt.as_pointer_value().into(),
                                            placeholder.as_pointer_value().into(),
                                        ],
                                        "print_unknown_field",
                                    )
                                    .unwrap();
                            }
                        }

                        // Print comma and space if not last field
                        if field_idx < metadata.field_names.len() - 1 {
                            let comma_fmt = self
                                .builder
                                .build_global_string_ptr(", ", "comma_fmt")
                                .unwrap();
                            self.builder
                                .build_call(
                                    printf_fn,
                                    &[comma_fmt.as_pointer_value().into()],
                                    "print_comma",
                                )
                                .unwrap();
                        }
                    }
                } else {
                    // Can't access struct value, print field names as fallback
                    let field_list = metadata.field_names.join(", ");
                    let fallback = format!("{}", field_list);
                    let fmt = self
                        .builder
                        .build_global_string_ptr("%s", "struct_fallback_fmt")
                        .unwrap();
                    let fallback_str = self
                        .builder
                        .build_global_string_ptr(&fallback, "struct_fallback_str")
                        .unwrap();
                    self.builder
                        .build_call(
                            printf_fn,
                            &[
                                fmt.as_pointer_value().into(),
                                fallback_str.as_pointer_value().into(),
                            ],
                            "print_struct_fallback",
                        )
                        .unwrap();
                }

                // Print closing brace
                let closing = if idx < total_values - 1 { " } " } else { " }" };
                let close_fmt = self
                    .builder
                    .build_global_string_ptr("%s", "struct_close_fmt")
                    .unwrap();
                let close_str = self
                    .builder
                    .build_global_string_ptr(closing, "struct_close_str")
                    .unwrap();
                self.builder
                    .build_call(
                        printf_fn,
                        &[
                            close_fmt.as_pointer_value().into(),
                            close_str.as_pointer_value().into(),
                        ],
                        "print_struct_close",
                    )
                    .unwrap();
            } else {
                // No canonical type - just print struct name with field names
                let field_list = metadata.field_names.join(", ");
                let info = format!("{} {{ {} }}", struct_name, field_list);
                let fmt = self
                    .builder
                    .build_global_string_ptr("%s", "struct_no_type_fmt")
                    .unwrap();
                let info_str = self
                    .builder
                    .build_global_string_ptr(&info, "struct_no_type_str")
                    .unwrap();
                self.builder
                    .build_call(
                        printf_fn,
                        &[
                            fmt.as_pointer_value().into(),
                            info_str.as_pointer_value().into(),
                        ],
                        "print_struct_no_type",
                    )
                    .unwrap();
            }
        }
    }
}
