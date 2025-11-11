use crate::codegen::core::{CodeGen, Symbol};
use inkwell::types::BasicType;
impl<'ctx> CodeGen<'ctx> {
    pub fn generate_load_array_element(
        &mut self,
        dest: &str,
        array: &str,
        index: &str,
    ) -> Option<inkwell::values::BasicValueEnum<'ctx>> {
        let array_ptr = self.resolve_value(array).into_pointer_value();
        let index_val = self.resolve_value(index).into_int_value();

        let is_string = self.array_contains_strings(array);
        let elem_type = self.get_array_element_type(array);

        // Get array metadata to know the array type
        let array_len = if let Some(metadata) = self.array_metadata.get(array) {
            metadata.length as u32
        } else {
            0
        };

        let array_type = elem_type.array_type(array_len);

        // Cast data pointer to array pointer
        let typed_array_ptr = self
            .builder
            .build_pointer_cast(
                array_ptr,
                self.context.ptr_type(inkwell::AddressSpace::default()),
                "array_ptr_typed",
            )
            .unwrap();

        // GEP to get element pointer
        let elem_ptr = unsafe {
            self.builder.build_gep(
                array_type,
                typed_array_ptr,
                &[self.context.i32_type().const_zero(), index_val],
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
        if is_string && elem_val.is_pointer_value() {
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

            // Mark this variable as heap string for cleanup
            self.heap_strings.insert(dest.to_string());
        }

        // Store in destination variable
        if let Some(symbol) = self.symbols.get(dest) {
            self.builder.build_store(symbol.ptr, elem_val).unwrap();
        } else {
            // Create new variable
            let alloca = self.builder.build_alloca(elem_type, dest).unwrap();
            self.builder.build_store(alloca, elem_val).unwrap();

            self.symbols.insert(
                dest.to_string(),
                Symbol {
                    ptr: alloca,
                    ty: elem_type,
                },
            );
        }

        Some(elem_val)
    }

    pub fn generate_load_map_pair(
        &mut self,
        key_dest: &str,
        val_dest: &str,
        map: &str,
        index: &str,
    ) -> Option<inkwell::values::BasicValueEnum<'ctx>> {
        let map_resolved = self.resolve_value(map);

        // Check if we got a valid pointer
        if !map_resolved.is_pointer_value() {
            eprintln!("ERROR: Map '{}' did not resolve to a pointer value", map);
            eprintln!("Resolved value type: {:?}", map_resolved.get_type());
            eprintln!(
                "Available in temp_values: {}",
                self.temp_values.contains_key(map)
            );
            eprintln!("Available in symbols: {}", self.symbols.contains_key(map));
            // Return None to avoid crash
            return None;
        }

        let map_ptr = map_resolved.into_pointer_value();
        let index_val = self.resolve_value(index).into_int_value();

        let (key_is_string, val_is_string) = self.map_contains_strings(map);
        let (key_type, val_type) = self.get_map_types(map);
        let pair_type = self.context.struct_type(&[key_type, val_type], false);

        // Get map length for array type
        let map_len = if let Some(metadata) = self.map_metadata.get(map) {
            metadata.length as u32
        } else {
            0
        };

        let map_array_type = pair_type.array_type(map_len);

        // Cast to typed map pointer
        let typed_map_ptr = self
            .builder
            .build_pointer_cast(
                map_ptr,
                self.context.ptr_type(inkwell::AddressSpace::default()),
                "map_ptr_typed",
            )
            .unwrap();

        // GEP to get pair pointer
        let pair_ptr = unsafe {
            self.builder.build_gep(
                map_array_type,
                typed_map_ptr,
                &[self.context.i32_type().const_zero(), index_val],
                "pair_ptr",
            )
        }
        .unwrap();

        // Extract key (field 0)
        let key_ptr = self
            .builder
            .build_struct_gep(pair_type, pair_ptr, 0, "key_ptr")
            .unwrap();
        let key_val = self.builder.build_load(key_type, key_ptr, "key").unwrap();

        // Extract value (field 1)
        let val_ptr = self
            .builder
            .build_struct_gep(pair_type, pair_ptr, 1, "val_ptr")
            .unwrap();
        let val_val = self.builder.build_load(val_type, val_ptr, "val").unwrap();

        // Handle RC for key if string
        if key_is_string && key_val.is_pointer_value() {
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
            self.heap_strings.insert(key_dest.to_string());
        }

        // Handle RC for value if string
        if val_is_string && val_val.is_pointer_value() {
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
            self.heap_strings.insert(val_dest.to_string());
        }

        // Store key
        if let Some(symbol) = self.symbols.get(key_dest) {
            self.builder.build_store(symbol.ptr, key_val).unwrap();
        } else {
            let alloca = self.builder.build_alloca(key_type, key_dest).unwrap();
            self.builder.build_store(alloca, key_val).unwrap();
            self.symbols.insert(
                key_dest.to_string(),
                Symbol {
                    ptr: alloca,
                    ty: key_type,
                },
            );
        }

        // Store value
        if let Some(symbol) = self.symbols.get(val_dest) {
            self.builder.build_store(symbol.ptr, val_val).unwrap();
        } else {
            let alloca = self.builder.build_alloca(val_type, val_dest).unwrap();
            self.builder.build_store(alloca, val_val).unwrap();
            self.symbols.insert(
                val_dest.to_string(),
                Symbol {
                    ptr: alloca,
                    ty: val_type,
                },
            );
        }

        None
    }
}
