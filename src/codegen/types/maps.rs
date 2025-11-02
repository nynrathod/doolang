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
    ) -> Option<BasicValueEnum<'ctx>> {
        if entries.is_empty() {
            // Allow empty maps: use i32 as default key/value type
            let ptr = self.context.ptr_type(AddressSpace::default()).const_null();
            self.temp_values
                .insert(name.to_string(), ptr.as_basic_value_enum());

            // Insert metadata for empty map so print_map knows to print {}
            self.map_metadata.insert(
                name.to_string(),
                crate::codegen::MapMetadata {
                    length: 0,
                    key_type: "Int".to_string(),
                    value_type: "Int".to_string(),
                    key_is_string: false,
                    value_is_string: false,
                },
            );

            return Some(ptr.as_basic_value_enum());
        }

        // Track string keys and values
        let mut str_temps = Vec::new();
        let mut key_is_string = false;
        let mut value_is_string = false;

        for (k, v) in entries {
            if self.heap_strings.contains(k) {
                str_temps.push(k.clone());
                key_is_string = true;
            }
            if self.heap_strings.contains(v) {
                str_temps.push(v.clone());
                value_is_string = true;
            }
        }

        if !str_temps.is_empty() {
            self.composite_strings.insert(name.to_string(), str_temps);
        }

        let first_key = self.resolve_value(&entries[0].0);
        let first_val = self.resolve_value(&entries[0].1);
        let key_type = first_key.get_type();
        let val_type = first_val.get_type();

        let key_type_name = if key_type.is_int_type() {
            "Int"
        } else if key_type.is_pointer_type() {
            "Str"
        } else {
            "Unknown"
        };

        let val_type_name = if val_type.is_int_type() {
            "Int"
        } else if val_type.is_pointer_type() {
            "Str"
        } else {
            "Unknown"
        };

        self.map_metadata.insert(
            name.to_string(),
            crate::codegen::MapMetadata {
                length: entries.len(),
                key_type: key_type_name.to_string(),
                value_type: val_type_name.to_string(),
                key_is_string,
                value_is_string,
            },
        );

        let pair_type = self.context.struct_type(&[key_type, val_type], false);
        let map_type = pair_type.array_type(entries.len() as u32);

        // HEAP ALLOCATE with RC header
        let malloc_fn = self.get_or_declare_malloc();
        let map_size = map_type.size_of().unwrap();
        let total_size = self.context.i64_type().const_int(8, false); // Use i64 for header size
        let total_size = self
            .builder
            .build_int_add(total_size, map_size, "total_size")
            .unwrap();

        let heap_ptr = self
            .builder
            .build_call(malloc_fn, &[total_size.into()], "heap_map")
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
                self.context.ptr_type(AddressSpace::default()),
                "rc_ptr",
            )
            .unwrap();
        self.builder
            .build_store(rc_ptr, self.context.i32_type().const_int(1, false))
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
            let key_type = match metadata.key_type.as_str() {
                "Int" => self.context.i32_type().into(),
                "Bool" => self.context.bool_type().into(),
                "Str" => self.context.ptr_type(AddressSpace::default()).into(),
                _ => {
                    eprintln!(
                        "WARNING: Unknown key type '{}' for map '{}', defaulting to i32",
                        metadata.key_type, map_name
                    );
                    self.context.i32_type().into()
                }
            };

            let val_type = match metadata.value_type.as_str() {
                "Int" => self.context.i32_type().into(),
                "Bool" => self.context.bool_type().into(),
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

        // Get map metadata
        let metadata = self.map_metadata.get(map_name).cloned();

        if let Some(metadata) = metadata {
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

            let key_type = if metadata.key_type == "Str" {
                self.context
                    .ptr_type(AddressSpace::default())
                    .as_basic_type_enum()
            } else {
                self.context.i32_type().as_basic_type_enum()
            };

            let val_type = if metadata.value_type == "Str" {
                self.context
                    .ptr_type(AddressSpace::default())
                    .as_basic_type_enum()
            } else {
                self.context.i32_type().as_basic_type_enum()
            };

            let pair_type = self.context.struct_type(&[key_type, val_type], false);
            let map_array_type = pair_type.array_type(metadata.length as u32);

            let typed_map_ptr = self
                .builder
                .build_pointer_cast(
                    map_ptr,
                    self.context.ptr_type(AddressSpace::default()),
                    "typed_map_ptr",
                )
                .unwrap();

            // Print each key-value pair
            for i in 0..metadata.length {
                let index = self.context.i32_type().const_int(i as u64, false);
                let pair_ptr = unsafe {
                    self.builder.build_gep(
                        map_array_type,
                        typed_map_ptr,
                        &[self.context.i32_type().const_zero(), index],
                        "pair_ptr",
                    )
                }
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
                } else {
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

                // Print value
                if metadata.value_type == "Str" {
                    let val_fmt = if i < metadata.length - 1 {
                        "\"%s\", "
                    } else {
                        "\"%s\""
                    };
                    let val_fmt_global = self
                        .builder
                        .build_global_string_ptr(val_fmt, "val_fmt")
                        .unwrap();
                    self.builder
                        .build_call(
                            printf_fn,
                            &[val_fmt_global.as_pointer_value().into(), val_val.into()],
                            "",
                        )
                        .unwrap();
                } else {
                    let val_fmt = if i < metadata.length - 1 {
                        "%d, "
                    } else {
                        "%d"
                    };
                    let val_fmt_global = self
                        .builder
                        .build_global_string_ptr(val_fmt, "val_fmt")
                        .unwrap();
                    self.builder
                        .build_call(
                            printf_fn,
                            &[val_fmt_global.as_pointer_value().into(), val_val.into()],
                            "",
                        )
                        .unwrap();
                }
            }
        }

        // Print closing brace
        let close_brace = self
            .builder
            .build_global_string_ptr("}", "close_brace")
            .unwrap();
        self.builder
            .build_call(printf_fn, &[close_brace.as_pointer_value().into()], "")
            .unwrap();
    }
}
