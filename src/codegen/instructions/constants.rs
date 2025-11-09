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

    pub fn generate_cast(
        &mut self,
        name: &str,
        value: &str,
        target_type: &str,
    ) -> Option<BasicValueEnum<'ctx>> {
        // Resolve the source value
        let source_val = self.resolve_value(value);

        let result = match target_type {
            // Int to Float
            "Float" => {
                if source_val.is_int_value() {
                    let int_val = source_val.into_int_value();
                    let float_val = self
                        .builder
                        .build_signed_int_to_float(int_val, self.context.f64_type(), "cast_i_to_f")
                        .unwrap();
                    float_val.into()
                } else {
                    source_val
                }
            }
            // Float to Int
            "Int" => {
                if source_val.is_float_value() {
                    let float_val = source_val.into_float_value();
                    let int_val = self
                        .builder
                        .build_float_to_signed_int(
                            float_val,
                            self.context.i32_type(),
                            "cast_f_to_i",
                        )
                        .unwrap();
                    int_val.into()
                } else {
                    source_val
                }
            }
            // Int to String
            "String" => {
                // Call IntToString function if available, otherwise use sprintf-based conversion
                if source_val.is_int_value() {
                    if let Some(fn_val) = self.module.get_function("IntToString") {
                        let int_val = source_val.into_int_value();
                        let call_result = self
                            .builder
                            .build_call(fn_val, &[int_val.into()], "int_to_str")
                            .unwrap();
                        call_result
                            .try_as_basic_value()
                            .left()
                            .unwrap_or(source_val)
                    } else {
                        // Fallback: use sprintf to convert int to string
                        self.convert_int_to_string_via_sprintf(source_val.into_int_value())
                    }
                } else if source_val.is_float_value() {
                    if let Some(fn_val) = self.module.get_function("FloatToString") {
                        let float_val = source_val.into_float_value();
                        let call_result = self
                            .builder
                            .build_call(fn_val, &[float_val.into()], "float_to_str")
                            .unwrap();
                        call_result
                            .try_as_basic_value()
                            .left()
                            .unwrap_or(source_val)
                    } else {
                        // Fallback: use sprintf to convert float to string
                        self.convert_float_to_string_via_sprintf(source_val.into_float_value())
                    }
                } else {
                    source_val
                }
            }
            // String to Int
            "Int" if source_val.is_pointer_value() => {
                // Call StringToInt function if available
                if let Some(fn_val) = self.module.get_function("StringToInt") {
                    let call_result = self
                        .builder
                        .build_call(fn_val, &[source_val.into()], "str_to_int")
                        .unwrap();
                    call_result
                        .try_as_basic_value()
                        .left()
                        .unwrap_or(source_val)
                } else {
                    source_val
                }
            }
            // String to Float
            "Float" if source_val.is_pointer_value() => {
                // Call StringToFloat function if available
                if let Some(fn_val) = self.module.get_function("StringToFloat") {
                    let call_result = self
                        .builder
                        .build_call(fn_val, &[source_val.into()], "str_to_float")
                        .unwrap();
                    call_result
                        .try_as_basic_value()
                        .left()
                        .unwrap_or(source_val)
                } else {
                    source_val
                }
            }
            // Bool conversions
            "Bool" => source_val,
            // Same type - no cast needed
            _ => source_val,
        };

        self.temp_values.insert(name.to_string(), result);
        Some(result)
    }
}
