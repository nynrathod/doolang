use crate::codegen::core::CodeGen;
use inkwell::types::BasicTypeEnum;
use inkwell::values::FunctionValue;
use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::AddressSpace;

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
            // Special handling for array/map variables - they should always be pointers
            let load_type =
                if (name.contains("_array") || name.contains("_map")) && sym.ty.is_int_type() {
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

        debug_assert!(
            false,
            "Unknown variable or literal: {} - check your MIR generation",
            name
        );
        // Return a zero value as fallback for release builds
        self.context.i32_type().const_int(0, false).into()
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
}
