use crate::codegen::core::CodeGen;
use inkwell::types::BasicType;
use inkwell::values::BasicValue;
use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

impl<'ctx> CodeGen<'ctx> {
    pub fn generate_map_with_metadata(
        &mut self,
        name: &str,
        entries: &[(String, String)],
        key_type_hint: Option<&str>,
        value_type_hint: Option<&str>,
    ) -> Option<BasicValueEnum<'ctx>> {
        if entries.is_empty() {
            // Use type hints if available, otherwise default to Int
            let key_type_str = key_type_hint.unwrap_or("Int");
            let value_type_str = value_type_hint.unwrap_or("Int");
            let key_is_string = key_type_str == "Str";
            let value_is_string = value_type_str == "Str";

            let ptr = self.context.ptr_type(AddressSpace::default()).const_null();
            self.temp_values
                .insert(name.to_string(), ptr.as_basic_value_enum());

            // Insert metadata for empty map with correct types
            self.map_metadata.insert(
                name.to_string(),
                crate::codegen::MapMetadata {
                    length: 0,
                    key_type: key_type_str.to_string(),
                    value_type: value_type_str.to_string(),
                    key_is_string,
                    value_is_string,
                    key_needs_rc: false,
                    value_needs_rc: false,
                },
            );

            return Some(ptr.as_basic_value_enum());
        }

        // Track string keys and values
        let mut str_temps = Vec::new();

        // Resolve first key/value to determine types
        let first_key = self.resolve_value(&entries[0].0);
        let first_val = self.resolve_value(&entries[0].1);
        let key_type = first_key.get_type();
        let val_type = first_val.get_type();

        // Determine if keys/values are strings based on actual type
        // BUT: exclude string constants (which are globals, not heap-allocated)
        // String constants don't have RC headers, so we should NOT incref them
        // Note: we'll update these based on type hints if available
        let key_is_string = key_type.is_pointer_type();
        let value_is_string = val_type.is_pointer_type();

        // Check if these are heap-allocated strings (not constants)
        // String constants are named like "str_const_%" in entries
        let key_is_heap_string = key_is_string && self.heap_strings.contains(&entries[0].0);
        let value_is_heap_string = value_is_string && self.heap_strings.contains(&entries[0].1);

        // Track string temps for cleanup
        for (k, v) in entries {
            if self.heap_strings.contains(k) || key_is_string {
                str_temps.push(k.clone());
            }
            if self.heap_strings.contains(v) || value_is_string {
                str_temps.push(v.clone());
            }
        }

        if !str_temps.is_empty() {
            self.composite_strings.insert(name.to_string(), str_temps);
        }

        // Use type hints if available, otherwise infer from LLVM types
        let key_type_name = if let Some(hint) = key_type_hint {
            hint
        } else if key_type.is_int_type() {
            "Int"
        } else if key_type.is_float_type() {
            "Float"
        } else if key_type.is_pointer_type() {
            "Str"
        } else {
            "Unknown"
        };

        let val_type_name = if let Some(hint) = value_type_hint {
            hint
        } else if val_type.is_int_type() {
            "Int"
        } else if val_type.is_float_type() {
            "Float"
        } else if val_type.is_pointer_type() {
            "Str"
        } else {
            "Unknown"
        };

        // Update string flags based on actual type names
        let key_is_string_actual = key_type_name == "Str";
        let value_is_string_actual = val_type_name == "Str";

        self.map_metadata.insert(
            name.to_string(),
            crate::codegen::MapMetadata {
                length: entries.len(),
                key_type: key_type_name.to_string(),
                value_type: val_type_name.to_string(),
                key_is_string: key_is_string_actual,
                value_is_string: value_is_string_actual,
                key_needs_rc: key_is_heap_string,
                value_needs_rc: value_is_heap_string,
            },
        );

        let pair_type = self.context.struct_type(&[key_type, val_type], false);
        let map_type = pair_type.array_type(entries.len() as u32);

        // HEAP ALLOCATE with RC header (4 bytes) + length field (4 bytes) = 8 bytes header
        let malloc_fn = self.get_or_declare_malloc();
        let map_size = map_type.size_of().unwrap();
        let header_size = self.context.i64_type().const_int(8, false); // RC (4) + Length (4)
        let total_size = self
            .builder
            .build_int_add(header_size, map_size, "total_size")
            .unwrap();

        let heap_ptr = self
            .builder
            .build_call(malloc_fn, &[total_size.into()], "heap_map")
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

        // Store length at offset 4
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
        let len_ptr = self
            .builder
            .build_pointer_cast(
                len_field_ptr,
                self.context.ptr_type(AddressSpace::default()),
                "len_ptr",
            )
            .unwrap();
        self.builder
            .build_store(
                len_ptr,
                self.context
                    .i32_type()
                    .const_int(entries.len() as u64, false),
            )
            .unwrap();

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

        let map_ptr = self
            .builder
            .build_pointer_cast(
                data_ptr,
                self.context.ptr_type(AddressSpace::default()),
                "map_ptr",
            )
            .unwrap();

        // Store key-value pairs
        for (i, (k, v)) in entries.iter().enumerate() {
            let key_val = self.resolve_value(k);
            let val_val = self.resolve_value(v);

            let idx = self.context.i32_type().const_int(i as u64, false);
            let pair_ptr = unsafe {
                self.builder
                    .build_gep(
                        map_type,
                        map_ptr,
                        &[self.context.i32_type().const_zero(), idx],
                        &format!("pair_{}", i),
                    )
                    .unwrap()
            };

            let key_ptr = self
                .builder
                .build_struct_gep(pair_type, pair_ptr, 0, "key_ptr")
                .unwrap();
            self.builder.build_store(key_ptr, key_val).unwrap();

            let val_ptr = self
                .builder
                .build_struct_gep(pair_type, pair_ptr, 1, "val_ptr")
                .unwrap();
            self.builder.build_store(val_ptr, val_val).unwrap();
        }

        // CRITICAL: Remove key/value strings from heap_strings - they're now owned by the map
        // The map's composite_string_ptrs tracking will handle their cleanup
        for (k, v) in entries {
            if self.heap_strings.contains(k) {
                self.heap_strings.remove(k);
            }
            if self.heap_strings.contains(v) {
                self.heap_strings.remove(v);
            }
        }

        self.temp_values.insert(name.to_string(), data_ptr.into());
        self.heap_maps.insert(name.to_string());

        // Store full heap pointer for tuple returns
        self.heap_pointers.insert(name.to_string(), heap_ptr);

        Some(data_ptr.into())
    }

    pub fn get_map_length(&self, map_name: &str) -> inkwell::values::IntValue<'ctx> {
        if let Some(metadata) = self.map_metadata.get(map_name) {
            self.context
                .i32_type()
                .const_int(metadata.length as u64, false)
        } else {
            // Try one more time to find by pointer equality before giving up
            if let Some(sym) = self.symbols.get(map_name) {
                if let Ok(loaded) = self.builder.build_load(sym.ty, sym.ptr, "check_map_len") {
                    if loaded.is_pointer_value() {
                        let ptr_val = loaded.into_pointer_value();
                        for (other_name, metadata) in &self.map_metadata {
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

            eprintln!(
                "Warning: No map metadata found for '{}' in get_map_length",
                map_name
            );
            eprintln!(
                "Available map metadata: {:?}",
                self.map_metadata.keys().collect::<Vec<_>>()
            );
            self.context.i32_type().const_int(0, false)
        }
    }

    pub fn get_map_types(
        &self,
        map_name: &str,
    ) -> (
        inkwell::types::BasicTypeEnum<'ctx>,
        inkwell::types::BasicTypeEnum<'ctx>,
    ) {
        if let Some(metadata) = self.map_metadata.get(map_name) {
            // IMPORTANT: Use i32 for Bool to match how bools are stored in maps
            // (bools are resolved as i32 in resolve_value, so map struct uses i32)
            let key_type = match metadata.key_type.as_str() {
                "Int" => self.context.i32_type().into(),
                "Float" => self.context.f64_type().into(),
                "Bool" => self.context.i32_type().into(),
                "Str" => self.context.ptr_type(AddressSpace::default()).into(),
                _ => {
                    eprintln!(
                        "WARNING: Unknown key type '{}' for map '{}', defaulting to i32",
                        metadata.key_type, map_name
                    );
                    self.context.i32_type().into()
                }
            };

            // IMPORTANT: Use i32 for Bool to match how bools are stored in maps
            let val_type = match metadata.value_type.as_str() {
                "Int" => self.context.i32_type().into(),
                "Float" => self.context.f64_type().into(),
                "Bool" => self.context.i32_type().into(),
                "Str" => self.context.ptr_type(AddressSpace::default()).into(),
                _ => {
                    eprintln!(
                        "WARNING: Unknown value type '{}' for map '{}', defaulting to i32",
                        metadata.value_type, map_name
                    );
                    self.context.i32_type().into()
                }
            };

            (key_type, val_type)
        } else {
            eprintln!("\n╔════════════════════════════════════════════════════════════════════╗");
            eprintln!("║ ERROR: No metadata found for map '{}'", map_name);
            eprintln!("╚════════════════════════════════════════════════════════════════════╝");
            eprintln!("\n📊 Available map metadata:");
            if self.map_metadata.is_empty() {
                eprintln!("  (none)");
            } else {
                for (key, meta) in &self.map_metadata {
                    eprintln!(
                        "  • '{}' → {{{}:{}}}, length={}",
                        key, meta.key_type, meta.value_type, meta.length
                    );
                }
            }
            eprintln!("\n⚠️  Cannot determine map types without metadata - IR will be incorrect!");
            eprintln!("═══════════════════════════════════════════════════════════════════\n");

            // Return dummy types, but this will produce incorrect IR
            debug_assert!(
                false,
                "FATAL: Cannot proceed without map metadata for '{}'",
                map_name
            );

            // Return fallback types for release builds
            (
                self.context.i32_type().into(),
                self.context.i32_type().into(),
            )
        }
    }

    /// Returns the pair type string for a map (used for struct type).
    pub fn get_map_pair_type(&self, map_name: &str) -> inkwell::types::StructType<'ctx> {
        let (key_type, val_type) = self.get_map_types(map_name);
        self.context.struct_type(&[key_type, val_type], false)
    }

    /// Returns true if the map contains string keys or values.
    pub fn map_contains_strings(&self, map_name: &str) -> (bool, bool) {
        if let Some(metadata) = self.map_metadata.get(map_name) {
            (metadata.key_is_string, metadata.value_is_string)
        } else {
            (false, false)
        }
    }

    /// Returns true if the map strings need RC (heap-allocated, not constants).
    pub fn map_strings_need_rc(&self, map_name: &str) -> (bool, bool) {
        if let Some(metadata) = self.map_metadata.get(map_name) {
            (metadata.key_needs_rc, metadata.value_needs_rc)
        } else {
            (false, false)
        }
    }

    /// Extract map key-value pair with RC handling
    pub fn load_map_pair_with_rc(
        &mut self,
        map_ptr: inkwell::values::PointerValue<'ctx>,
        index: inkwell::values::IntValue<'ctx>,
        pair_type: inkwell::types::StructType<'ctx>,
        key_is_string: bool,
        val_is_string: bool,
    ) -> (BasicValueEnum<'ctx>, BasicValueEnum<'ctx>) {
        // GEP to get pair pointer
        let pair_ptr = unsafe {
            self.builder.build_gep(
                pair_type.array_type(0),
                map_ptr,
                &[self.context.i32_type().const_zero(), index],
                "pair_ptr",
            )
        }
        .unwrap();

        // Extract key (field 0)
        let key_ptr = self
            .builder
            .build_struct_gep(pair_type, pair_ptr, 0, "key_ptr")
            .unwrap();
        let key_val = self
            .builder
            .build_load(
                pair_type.get_field_type_at_index(0).unwrap(),
                key_ptr,
                "key",
            )
            .unwrap();

        // Extract value (field 1)
        let val_ptr = self
            .builder
            .build_struct_gep(pair_type, pair_ptr, 1, "val_ptr")
            .unwrap();
        let val_val = self
            .builder
            .build_load(
                pair_type.get_field_type_at_index(1).unwrap(),
                val_ptr,
                "val",
            )
            .unwrap();

        // Handle RC for strings
        if key_is_string {
            let str_ptr = key_val.into_pointer_value();
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

        if val_is_string {
            let str_ptr = val_val.into_pointer_value();
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

        (key_val, val_val)
    }

    /// Get or declare strlen function for string length calculation

    /// Helper method to print a map
    pub fn print_map(&mut self, map_name: &str) {
        let printf_fn = self.get_or_declare_printf();

        // Print opening brace
        let open_brace = self
            .builder
            .build_global_string_ptr("{", "open_brace")
            .unwrap();
        self.builder
            .build_call(printf_fn, &[open_brace.as_pointer_value().into()], "")
            .unwrap();

        // Get map metadata - try multiple name variations
        let mut metadata = self.map_metadata.get(map_name).cloned();

        // If not found, try variations
        if metadata.is_none() {
            let variations = vec![
                map_name.trim_start_matches('%').to_string(),
                map_name.trim_end_matches("_map").to_string(),
                format!("{}_map", map_name),
                format!("{}_map", map_name.trim_start_matches('%')),
            ];

            for var in variations {
                if let Some(meta) = self.map_metadata.get(&var).cloned() {
                    metadata = Some(meta);
                    break;
                }
            }
        }

        if let Some(metadata) = metadata {
            // NOTE: Don't early-return for length==0 here because:
            // 1. For struct fields, we pass length=0 but actual length is in heap header
            // 2. The null pointer check and runtime length loop below handle empty maps correctly

            // Get pointer to the map data
            let map_ptr = if self.symbols.contains_key(map_name) {
                // Variable case: resolve_pointer gives us the alloca,
                // we need to load the actual map pointer from it
                let var_alloca = self.resolve_pointer(map_name);
                self.builder
                    .build_load(
                        self.context.ptr_type(AddressSpace::default()),
                        var_alloca,
                        "map_data_ptr",
                    )
                    .unwrap()
                    .into_pointer_value()
            } else {
                self.resolve_value(map_name).into_pointer_value()
            };

            // Check for null pointer (empty map at runtime)
            let is_null = self.builder.build_is_null(map_ptr, "is_null_map").unwrap();

            let current_fn = self
                .builder
                .get_insert_block()
                .unwrap()
                .get_parent()
                .unwrap();
            let print_map_block = self
                .context
                .append_basic_block(current_fn, "print_map_contents");
            let skip_map_block = self
                .context
                .append_basic_block(current_fn, "print_map_empty");
            let merge_block = self
                .context
                .append_basic_block(current_fn, "print_map_merge");

            self.builder
                .build_conditional_branch(is_null, skip_map_block, print_map_block)
                .unwrap();

            // Skip block - map is null/empty, just close brace
            self.builder.position_at_end(skip_map_block);
            let close_brace_empty = self
                .builder
                .build_global_string_ptr("}", "close_brace_null")
                .unwrap();
            self.builder
                .build_call(
                    printf_fn,
                    &[close_brace_empty.as_pointer_value().into()],
                    "",
                )
                .unwrap();
            self.builder
                .build_unconditional_branch(merge_block)
                .unwrap();

            // Print block - map has contents
            self.builder.position_at_end(print_map_block);

            let key_type = match metadata.key_type.as_str() {
                "Str" => self
                    .context
                    .ptr_type(AddressSpace::default())
                    .as_basic_type_enum(),
                "Float" => self.context.f64_type().as_basic_type_enum(),
                "Bool" => self.context.i32_type().as_basic_type_enum(),
                _ => self.context.i32_type().as_basic_type_enum(), // Int or default
            };

            let val_type = match metadata.value_type.as_str() {
                "Str" => self
                    .context
                    .ptr_type(AddressSpace::default())
                    .as_basic_type_enum(),
                "Float" => self.context.f64_type().as_basic_type_enum(),
                "Bool" => self.context.i32_type().as_basic_type_enum(),
                _ => self.context.i32_type().as_basic_type_enum(), // Int or default
            };

            let pair_type = self.context.struct_type(&[key_type, val_type], false);

            let typed_map_ptr = self
                .builder
                .build_pointer_cast(
                    map_ptr,
                    self.context.ptr_type(AddressSpace::default()),
                    "typed_map_ptr",
                )
                .unwrap();

            // Read runtime length from map header (at offset -4 from data pointer)
            let heap_ptr = unsafe {
                self.builder
                    .build_gep(
                        self.context.i8_type(),
                        typed_map_ptr,
                        &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                        "heap_ptr_print",
                    )
                    .unwrap()
            };

            let len_field_ptr = unsafe {
                self.builder
                    .build_gep(
                        self.context.i8_type(),
                        heap_ptr,
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

            let runtime_len = self
                .builder
                .build_load(self.context.i32_type(), len_ptr_cast, "runtime_len_print")
                .unwrap()
                .into_int_value();

            // Create blocks for dynamic loop
            let current_fn = self
                .builder
                .get_insert_block()
                .unwrap()
                .get_parent()
                .unwrap();
            let loop_block = self
                .context
                .append_basic_block(current_fn, "print_map_loop");
            let loop_body = self
                .context
                .append_basic_block(current_fn, "print_map_body");
            let loop_done = self
                .context
                .append_basic_block(current_fn, "print_map_done");

            // Counter for loop
            let counter_ptr = self
                .builder
                .build_alloca(self.context.i32_type(), "print_counter")
                .unwrap();
            self.builder
                .build_store(counter_ptr, self.context.i32_type().const_zero())
                .unwrap();

            self.builder.build_unconditional_branch(loop_block).unwrap();

            // Loop check
            self.builder.position_at_end(loop_block);
            let counter = self
                .builder
                .build_load(self.context.i32_type(), counter_ptr, "counter")
                .unwrap()
                .into_int_value();
            let cmp = self
                .builder
                .build_int_compare(inkwell::IntPredicate::ULT, counter, runtime_len, "cmp")
                .unwrap();
            self.builder
                .build_conditional_branch(cmp, loop_body, loop_done)
                .unwrap();

            // Loop body: print each key-value pair
            self.builder.position_at_end(loop_body);

            // Use byte-level GEP to avoid array type size issues
            let pair_size = pair_type.size_of().unwrap();
            let counter_64 = self
                .builder
                .build_int_z_extend(counter, self.context.i64_type(), "counter_64")
                .unwrap();
            let byte_offset = self
                .builder
                .build_int_mul(counter_64, pair_size, "byte_offset")
                .unwrap();

            let pair_ptr_bytes = unsafe {
                self.builder
                    .build_gep(
                        self.context.i8_type(),
                        typed_map_ptr,
                        &[byte_offset],
                        "pair_ptr_bytes",
                    )
                    .unwrap()
            };

            let pair_ptr = self
                .builder
                .build_pointer_cast(
                    pair_ptr_bytes,
                    self.context.ptr_type(AddressSpace::default()),
                    "pair_ptr",
                )
                .unwrap();

            // Extract key
            let key_ptr = self
                .builder
                .build_struct_gep(pair_type, pair_ptr, 0, "key_ptr")
                .unwrap();
            let key_val = self.builder.build_load(key_type, key_ptr, "key").unwrap();

            // Extract value
            let val_ptr = self
                .builder
                .build_struct_gep(pair_type, pair_ptr, 1, "val_ptr")
                .unwrap();
            let val_val = self.builder.build_load(val_type, val_ptr, "val").unwrap();

            // Print key
            if metadata.key_type == "Str" {
                let key_fmt = self
                    .builder
                    .build_global_string_ptr("\"%s\": ", "key_fmt")
                    .unwrap();
                self.builder
                    .build_call(
                        printf_fn,
                        &[key_fmt.as_pointer_value().into(), key_val.into()],
                        "",
                    )
                    .unwrap();
            } else if metadata.key_type == "Float" {
                let key_fmt = self
                    .builder
                    .build_global_string_ptr("%.6g: ", "key_fmt")
                    .unwrap();
                let key_float = key_val.into_float_value();
                self.builder
                    .build_call(
                        printf_fn,
                        &[key_fmt.as_pointer_value().into(), key_float.into()],
                        "",
                    )
                    .unwrap();
            } else if metadata.key_type == "Bool" {
                // Bool: print as "true" or "false"
                let key_int = key_val.into_int_value();
                let is_true = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        key_int,
                        self.context.i32_type().const_zero(),
                        "is_true_key",
                    )
                    .unwrap();
                let true_str = self
                    .builder
                    .build_global_string_ptr("true: ", "true_str_key")
                    .unwrap();
                let false_str = self
                    .builder
                    .build_global_string_ptr("false: ", "false_str_key")
                    .unwrap();
                let key_fmt = self
                    .builder
                    .build_select(is_true, true_str, false_str, "key_fmt_bool")
                    .unwrap();
                self.builder
                    .build_call(printf_fn, &[key_fmt.into()], "")
                    .unwrap();
            } else {
                // Int
                let key_fmt = self
                    .builder
                    .build_global_string_ptr("%d: ", "key_fmt")
                    .unwrap();
                self.builder
                    .build_call(
                        printf_fn,
                        &[key_fmt.as_pointer_value().into(), key_val.into()],
                        "",
                    )
                    .unwrap();
            }

            // Check if this is the last element for comma formatting
            let next_counter = self
                .builder
                .build_int_add(
                    counter,
                    self.context.i32_type().const_int(1, false),
                    "next_counter",
                )
                .unwrap();
            let is_last = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::EQ,
                    next_counter,
                    runtime_len,
                    "is_last",
                )
                .unwrap();

            // Print value with conditional comma
            if metadata.value_type == "Str" {
                let val_fmt_with_comma = self
                    .builder
                    .build_global_string_ptr("\"%s\", ", "val_fmt_comma")
                    .unwrap();
                let val_fmt_no_comma = self
                    .builder
                    .build_global_string_ptr("\"%s\"", "val_fmt_no_comma")
                    .unwrap();

                let val_fmt = self
                    .builder
                    .build_select(is_last, val_fmt_no_comma, val_fmt_with_comma, "val_fmt")
                    .unwrap();

                self.builder
                    .build_call(printf_fn, &[val_fmt.into(), val_val.into()], "")
                    .unwrap();
            } else if metadata.value_type == "Float" {
                let val_fmt_with_comma = self
                    .builder
                    .build_global_string_ptr("%.6g, ", "val_fmt_comma")
                    .unwrap();
                let val_fmt_no_comma = self
                    .builder
                    .build_global_string_ptr("%.6g", "val_fmt_no_comma")
                    .unwrap();

                let val_fmt = self
                    .builder
                    .build_select(is_last, val_fmt_no_comma, val_fmt_with_comma, "val_fmt")
                    .unwrap();

                let val_float = val_val.into_float_value();
                self.builder
                    .build_call(printf_fn, &[val_fmt.into(), val_float.into()], "")
                    .unwrap();
            } else if metadata.value_type == "Bool" {
                // Bool: print as "true" or "false" with conditional comma
                let val_int = val_val.into_int_value();
                let is_true = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        val_int,
                        self.context.i32_type().const_zero(),
                        "is_true_val",
                    )
                    .unwrap();

                let true_str_comma = self
                    .builder
                    .build_global_string_ptr("true, ", "true_str_val_comma")
                    .unwrap();
                let true_str_no_comma = self
                    .builder
                    .build_global_string_ptr("true", "true_str_val_no_comma")
                    .unwrap();
                let false_str_comma = self
                    .builder
                    .build_global_string_ptr("false, ", "false_str_val_comma")
                    .unwrap();
                let false_str_no_comma = self
                    .builder
                    .build_global_string_ptr("false", "false_str_val_no_comma")
                    .unwrap();

                let val_fmt_comma = self
                    .builder
                    .build_select(
                        is_true,
                        true_str_comma,
                        false_str_comma,
                        "val_fmt_bool_comma",
                    )
                    .unwrap();
                let val_fmt_no_comma = self
                    .builder
                    .build_select(
                        is_true,
                        true_str_no_comma,
                        false_str_no_comma,
                        "val_fmt_bool_no_comma",
                    )
                    .unwrap();

                let val_fmt = self
                    .builder
                    .build_select(is_last, val_fmt_no_comma, val_fmt_comma, "val_fmt_bool")
                    .unwrap();

                self.builder
                    .build_call(printf_fn, &[val_fmt.into()], "")
                    .unwrap();
            } else {
                // Int
                let val_fmt_with_comma = self
                    .builder
                    .build_global_string_ptr("%d, ", "val_fmt_comma")
                    .unwrap();
                let val_fmt_no_comma = self
                    .builder
                    .build_global_string_ptr("%d", "val_fmt_no_comma")
                    .unwrap();

                let val_fmt = self
                    .builder
                    .build_select(is_last, val_fmt_no_comma, val_fmt_with_comma, "val_fmt")
                    .unwrap();

                self.builder
                    .build_call(printf_fn, &[val_fmt.into(), val_val.into()], "")
                    .unwrap();
            }

            // Increment counter and loop back
            self.builder.build_store(counter_ptr, next_counter).unwrap();
            self.builder.build_unconditional_branch(loop_block).unwrap();

            // Loop done
            self.builder.position_at_end(loop_done);

            // Print closing brace
            let close_brace = self
                .builder
                .build_global_string_ptr("}", "close_brace")
                .unwrap();
            self.builder
                .build_call(printf_fn, &[close_brace.as_pointer_value().into()], "")
                .unwrap();
            self.builder
                .build_unconditional_branch(merge_block)
                .unwrap();

            // Merge block - continue after printing
            self.builder.position_at_end(merge_block);
        } else {
            // No metadata - just print closing brace
            let close_brace = self
                .builder
                .build_global_string_ptr("}", "close_brace_no_meta")
                .unwrap();
            self.builder
                .build_call(printf_fn, &[close_brace.as_pointer_value().into()], "")
                .unwrap();
        }
    }
}
