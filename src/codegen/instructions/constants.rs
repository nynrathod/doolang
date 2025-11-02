use crate::codegen::core::CodeGen;
use inkwell::values::BasicValueEnum;

impl<'ctx> CodeGen<'ctx> {
    pub fn generate_const_int(&mut self, name: &str, value: i32) -> Option<BasicValueEnum<'ctx>> {
        let val = self.context.i32_type().const_int(value as u64, true);
        // If this temp was pre-allocated as a symbol (cross-block usage), store it there
        if let Some(sym) = self.symbols.get(name) {
            self.builder.build_store(sym.ptr, val).unwrap();
        }
        self.temp_values.insert(name.to_string(), val.into());
        Some(val.into())
    }

    pub fn generate_const_float(&mut self, name: &str, value: f64) -> Option<BasicValueEnum<'ctx>> {
        let val = self.context.f64_type().const_float(value);
        if let Some(sym) = self.symbols.get(name) {
            self.builder.build_store(sym.ptr, val).unwrap();
        }
        self.temp_values.insert(name.to_string(), val.into());
        Some(val.into())
    }

    pub fn generate_const_bool(&mut self, name: &str, value: bool) -> Option<BasicValueEnum<'ctx>> {
        // Use i32 instead of i1 for consistency with rest of codegen
        let val = self.context.i32_type().const_int(value as u64, false);
        // If this temp was pre-allocated as a symbol (cross-block usage), store it there
        if let Some(sym) = self.symbols.get(name) {
            self.builder.build_store(sym.ptr, val).unwrap();
        }
        self.temp_values.insert(name.to_string(), val.into());
        Some(val.into())
    }

    pub fn generate_const_string(
        &mut self,
        name: &str,
        value: &str,
    ) -> Option<BasicValueEnum<'ctx>> {
        // String constants should be module-level static constants, not heap allocations.
        // This avoids memory leaks and unnecessary malloc/free overhead.
        // The string data is stored in the read-only data section of the binary.

        let str_global = self
            .builder
            .build_global_string_ptr(value, &format!("str_const_{}", name))
            .expect("Failed to create string constant");

        let data_ptr = str_global.as_pointer_value();

        // Store in temp_values so it can be resolved by name
        self.temp_values.insert(name.to_string(), data_ptr.into());

        Some(data_ptr.into())
    }
}
