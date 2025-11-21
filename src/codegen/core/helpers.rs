use crate::codegen::core::CodeGen;
use inkwell::types::BasicTypeEnum;
use inkwell::values::FunctionValue;
use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::AddressSpace;

/// Parse tuple types from a comma-separated string, respecting nested parentheses
/// E.g., "Map(Str,Int), Int" -> ["Map(Str,Int)", "Int"]
pub fn parse_tuple_types(type_str: &str) -> Vec<String> {
    let mut types = Vec::new();
    let mut current = String::new();
    let mut depth = 0;

    for ch in type_str.chars() {
        match ch {
            '(' | '[' | '{' => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                // Top-level comma, split here
                if !current.trim().is_empty() {
                    types.push(current.trim().to_string());
                }
                current.clear();
            }
            _ => {
                current.push(ch);
            }
        }
    }

    // Add the last type
    if !current.trim().is_empty() {
        types.push(current.trim().to_string());
    }

    types
}

impl<'ctx> CodeGen<'ctx> {
    /// Resolves a variable or constant name to its pointer (for arrays/maps).
    /// Used when we need the actual pointer, not the loaded value.
    pub fn resolve_pointer(&self, name: &str) -> PointerValue<'ctx> {
        if let Some(sym) = self.symbols.get(name) {
            return sym.ptr;
        }

        debug_assert!(
            false,
            "Unknown variable for pointer resolution: {} - check your MIR generation",
            name
        );
        // Return a null pointer as fallback for release builds
        self.context
            .ptr_type(inkwell::AddressSpace::default())
            .const_null()
    }

    /// Resolve value (unchanged)
    /// Resolves a variable or constant name to its LLVM value.
    /// Used for looking up values in the symbol table or temporary values.
    pub fn resolve_value(&self, name: &str) -> BasicValueEnum<'ctx> {
        if let Some(val) = self.temp_values.get(name) {
            return *val;
        }

        if let Some(sym) = self.symbols.get(name) {
            // Special handling for array/map/struct variables - they should always be pointers
            // Check both naming convention and heap tracking sets
            let is_array_or_map = (name.contains("_array") || name.contains("_map"))
                || self.heap_arrays.contains(name)
                || self.heap_maps.contains(name);

            // Check if this is a struct by looking at variable_types
            // Structs can be stored as "Struct(Name)" or just "Name" (if it's in struct_metadata)
            let is_struct = self
                .variable_types
                .get(name)
                .map(|t| t.contains("Struct(") || self.struct_metadata.contains_key(t))
                .unwrap_or(false);

            let load_type = if is_array_or_map || is_struct {
                // Arrays, maps, and structs are always pointers, regardless of how they were stored
                self.context
                    .ptr_type(inkwell::AddressSpace::default())
                    .into()
            } else {
                sym.ty
            };

            return self
                .builder
                .build_load(load_type, sym.ptr, name)
                .expect("Failed to load value");
        }

        if let Ok(val) = name.parse::<i32>() {
            return self.context.i32_type().const_int(val as u64, true).into();
        }
        if let Ok(val) = name.parse::<f64>() {
            return self.context.f64_type().const_float(val).into();
        }
        if name == "true" {
            return self.context.i32_type().const_int(1, false).into();
        }
        if name == "false" {
            return self.context.i32_type().const_int(0, false).into();
        }

        eprintln!("ERROR: Unknown variable or literal: {}", name);
        eprintln!(
            "Available in temp_values: {:?}",
            self.temp_values.keys().collect::<Vec<_>>()
        );
        eprintln!(
            "Available in symbols: {:?}",
            self.symbols.keys().collect::<Vec<_>>()
        );
        panic!(
            "Unknown variable or literal: {} - check your MIR generation",
            name
        );
    }

    /// Returns the LLVM type corresponding to a type name string.
    /// Used for type resolution during codegen.
    pub fn get_llvm_type(&self, type_name: &str) -> BasicTypeEnum<'ctx> {
        match type_name {
            "Int" => self.context.i32_type().into(), // Only i32 for integers
            "Float" => self.context.f64_type().into(), // f64 for floating point
            "Bool" => self.context.bool_type().into(),
            "Str" => self.context.ptr_type(AddressSpace::default()).into(),
            _ => self.context.i32_type().into(),
        }
    }

    /// Get or declare printf function for print statements
    pub fn get_or_declare_printf(&self) -> FunctionValue<'ctx> {
        if let Some(func) = self.module.get_function("printf") {
            return func;
        }

        let i8_ptr_type = self.context.ptr_type(AddressSpace::default());
        let printf_type = self.context.i32_type().fn_type(&[i8_ptr_type.into()], true);
        self.module.add_function("printf", printf_type, None)
    }

    /// Convert an integer to a string using sprintf
    pub fn convert_int_to_string_via_sprintf(
        &mut self,
        int_val: inkwell::values::IntValue<'ctx>,
    ) -> BasicValueEnum<'ctx> {
        let sprintf_fn = self.get_or_declare_sprintf();
        let format_str = self
            .builder
            .build_global_string_ptr("%d", "int_fmt")
            .unwrap();

        // Allocate buffer for the result (max 12 chars for i32: "-2147483648")
        let malloc_fn = self.get_or_declare_malloc_libc();
        let buffer_size = self.context.i32_type().const_int(32, false);
        let buffer_ptr = self
            .builder
            .build_call(malloc_fn, &[buffer_size.into()], "int_buf")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Call sprintf
        let _sprintf_result = self
            .builder
            .build_call(
                sprintf_fn,
                &[
                    buffer_ptr.into(),
                    format_str.as_pointer_value().into(),
                    int_val.into(),
                ],
                "sprintf_int",
            )
            .unwrap();

        buffer_ptr.into()
    }

    /// Convert a float to a string using sprintf
    pub fn convert_float_to_string_via_sprintf(
        &mut self,
        float_val: inkwell::values::FloatValue<'ctx>,
    ) -> BasicValueEnum<'ctx> {
        let sprintf_fn = self.get_or_declare_sprintf();
        let format_str = self
            .builder
            .build_global_string_ptr("%.2f", "float_fmt")
            .unwrap();

        // Allocate buffer for the result
        let malloc_fn = self.get_or_declare_malloc_libc();
        let buffer_size = self.context.i32_type().const_int(32, false);
        let buffer_ptr = self
            .builder
            .build_call(malloc_fn, &[buffer_size.into()], "float_buf")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Call sprintf
        let _sprintf_result = self
            .builder
            .build_call(
                sprintf_fn,
                &[
                    buffer_ptr.into(),
                    format_str.as_pointer_value().into(),
                    float_val.into(),
                ],
                "sprintf_float",
            )
            .unwrap();

        buffer_ptr.into()
    }

    /// Get or declare sprintf function
    pub fn get_or_declare_sprintf(&self) -> FunctionValue<'ctx> {
        if let Some(func) = self.module.get_function("sprintf") {
            return func;
        }

        let i8_ptr_type = self.context.ptr_type(AddressSpace::default());
        let sprintf_type = self
            .context
            .i32_type()
            .fn_type(&[i8_ptr_type.into(), i8_ptr_type.into()], true);
        self.module.add_function("sprintf", sprintf_type, None)
    }

    /// Get or declare malloc function (internal for string conversion)
    pub fn get_or_declare_malloc_libc(&self) -> FunctionValue<'ctx> {
        if let Some(func) = self.module.get_function("malloc") {
            return func;
        }

        let i8_ptr_type = self.context.ptr_type(AddressSpace::default());
        let malloc_type = i8_ptr_type.fn_type(&[self.context.i32_type().into()], false);
        self.module.add_function("malloc", malloc_type, None)
    }
}
