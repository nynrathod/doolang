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

        // Process escape sequences
        let processed_value = Self::process_escape_sequences(value);

        let str_global = self
            .builder
            .build_global_string_ptr(&processed_value, &format!("str_const_{}", name))
            .expect("Failed to create string constant");

        let data_ptr = str_global.as_pointer_value();

        // Store in temp_values so it can be resolved by name
        self.temp_values.insert(name.to_string(), data_ptr.into());

        Some(data_ptr.into())
    }

    /// Process escape sequences in a string literal
    fn process_escape_sequences(value: &str) -> String {
        let mut result = String::new();
        let mut chars = value.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '\\' {
                if let Some(&next_ch) = chars.peek() {
                    match next_ch {
                        'n' => {
                            result.push('\n');
                            chars.next();
                        }
                        't' => {
                            result.push('\t');
                            chars.next();
                        }
                        'r' => {
                            result.push('\r');
                            chars.next();
                        }
                        '\\' => {
                            result.push('\\');
                            chars.next();
                        }
                        '"' => {
                            result.push('"');
                            chars.next();
                        }
                        '0' => {
                            result.push('\0');
                            chars.next();
                        }
                        'u' => {
                            // Unicode escape sequence: \u{XXXX} or \u{XXXXXX}
                            chars.next(); // consume 'u'
                            if chars.peek() == Some(&'{') {
                                chars.next(); // consume '{'
                                let mut hex_str = String::new();
                                while let Some(&c) = chars.peek() {
                                    if c == '}' {
                                        chars.next(); // consume '}'
                                        break;
                                    }
                                    hex_str.push(c);
                                    chars.next();
                                }
                                // Parse the hex string to a Unicode code point
                                if let Ok(code_point) = u32::from_str_radix(&hex_str, 16) {
                                    if let Some(unicode_char) = char::from_u32(code_point) {
                                        result.push(unicode_char);
                                    } else {
                                        // Invalid code point, keep original
                                        result.push_str("\\u{");
                                        result.push_str(&hex_str);
                                        result.push('}');
                                    }
                                } else {
                                    // Invalid hex, keep original
                                    result.push_str("\\u{");
                                    result.push_str(&hex_str);
                                    result.push('}');
                                }
                            } else {
                                // No brace, keep as literal \u
                                result.push_str("\\u");
                            }
                        }
                        _ => {
                            result.push(ch);
                        }
                    }
                } else {
                    result.push(ch);
                }
            } else {
                result.push(ch);
            }
        }

        result
    }

    pub fn generate_cast(
        &mut self,
        name: &str,
        value: &str,
        source_type: &str,
        target_type: &str,
    ) -> Option<BasicValueEnum<'ctx>> {
        // Resolve the source value
        let source_val = self.resolve_value(value);

        let result = match target_type {
            "Float" => {
                if source_val.is_int_value() {
                    // Int → Float
                    let int_val = source_val.into_int_value();
                    let float_val = self
                        .builder
                        .build_signed_int_to_float(int_val, self.context.f64_type(), "cast_i_to_f")
                        .unwrap();
                    float_val.into()
                } else if source_val.is_pointer_value() {
                    // String → Float with validation using strtod

                    let strtod_fn = self.module.get_function("strtod").unwrap_or_else(|| {
                        let f64_type = self.context.f64_type();
                        let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                        let fn_type = f64_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
                        self.module.add_function("strtod", fn_type, None)
                    });

                    let trimmed_val = self
                        .generate_string_method("tmp_trimmed_float", "", source_val, "trim", &[])
                        .unwrap();
                    let str_ptr = trimmed_val.into_pointer_value();

                    // Allocate endptr
                    let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                    let endptr_ptr = self.builder.build_alloca(ptr_type, "endptr_ptr").unwrap();
                    let null_ptr = ptr_type.const_null();

                    // Call strtod(str, &endptr)
                    let parsed_val = self
                        .builder
                        .build_call(
                            strtod_fn,
                            &[str_ptr.into(), endptr_ptr.into()],
                            "strtod_result",
                        )
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_float_value();

                    // Load endptr
                    let endptr = self
                        .builder
                        .build_load(ptr_type, endptr_ptr, "endptr")
                        .unwrap()
                        .into_pointer_value();

                    // Load *endptr (character after parsed float)
                    let end_char = self
                        .builder
                        .build_load(self.context.i8_type(), endptr, "end_char")
                        .unwrap()
                        .into_int_value();

                    let zero = self.context.i8_type().const_int(0, false);

                    // Valid if end_char == 0 (string fully consumed)
                    let is_valid = self
                        .builder
                        .build_int_compare(inkwell::IntPredicate::EQ, end_char, zero, "valid_float")
                        .unwrap();

                    // Create two blocks
                    let parent = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let ok_block = self.context.append_basic_block(parent, "float_ok");
                    let err_block = self.context.append_basic_block(parent, "float_err");

                    self.builder
                        .build_conditional_branch(is_valid, ok_block, err_block)
                        .unwrap();

                    // Error block
                    self.builder.position_at_end(err_block);
                    let panic_msg = self
                        .builder
                        .build_global_string_ptr(
                            "Runtime error: invalid string to Float conversion\n",
                            "panic_float",
                        )
                        .unwrap();

                    let printf_fn = self.module.get_function("printf").unwrap_or_else(|| {
                        let fn_type = self.context.i32_type().fn_type(
                            &[self
                                .context
                                .ptr_type(inkwell::AddressSpace::default())
                                .into()],
                            true,
                        );
                        self.module.add_function("printf", fn_type, None)
                    });

                    self.builder
                        .build_call(
                            printf_fn,
                            &[panic_msg.as_pointer_value().into()],
                            "printf_err",
                        )
                        .unwrap();

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
                            "exit_call",
                        )
                        .unwrap();
                    self.builder.build_unreachable().unwrap();

                    // OK block: continue with parsed value
                    self.builder.position_at_end(ok_block);
                    parsed_val.into()
                } else {
                    source_val
                }
            }

            "Int" => {
                if source_val.is_float_value() {
                    // Float to Int: check for special values (Inf, -Inf, NaN)
                    let float_val = source_val.into_float_value();

                    // Check if the float is NaN
                    let is_nan = self
                        .builder
                        .build_float_compare(
                            inkwell::FloatPredicate::UNO,
                            float_val,
                            float_val,
                            "is_nan",
                        )
                        .unwrap();

                    // Check if the float is infinite (positive or negative)
                    let pos_inf = self.context.f64_type().const_float(f64::INFINITY);
                    let neg_inf = self.context.f64_type().const_float(f64::NEG_INFINITY);

                    let is_pos_inf = self
                        .builder
                        .build_float_compare(
                            inkwell::FloatPredicate::OEQ,
                            float_val,
                            pos_inf,
                            "is_pos_inf",
                        )
                        .unwrap();

                    let is_neg_inf = self
                        .builder
                        .build_float_compare(
                            inkwell::FloatPredicate::OEQ,
                            float_val,
                            neg_inf,
                            "is_neg_inf",
                        )
                        .unwrap();

                    // Combine: is_special = is_nan || is_pos_inf || is_neg_inf
                    let is_nan_or_inf = self
                        .builder
                        .build_or(is_nan, is_pos_inf, "is_nan_or_pos_inf")
                        .unwrap();

                    let is_special = self
                        .builder
                        .build_or(is_nan_or_inf, is_neg_inf, "is_special_float")
                        .unwrap();

                    // Create blocks for normal vs special handling
                    let parent = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let normal_block = self
                        .context
                        .append_basic_block(parent, "float_to_int_normal");
                    let special_block = self
                        .context
                        .append_basic_block(parent, "float_to_int_special");

                    self.builder
                        .build_conditional_branch(is_special, special_block, normal_block)
                        .unwrap();

                    // Special block: print error and exit
                    self.builder.position_at_end(special_block);
                    let error_msg = self
                        .builder
                        .build_global_string_ptr(
                            "Runtime error: cannot convert Infinity or NaN to Int\n",
                            "error_msg_special_float",
                        )
                        .unwrap();

                    let printf_fn = self.module.get_function("printf").unwrap_or_else(|| {
                        let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                        let fn_type = self.context.i32_type().fn_type(&[ptr_type.into()], true);
                        self.module.add_function("printf", fn_type, None)
                    });

                    self.builder
                        .build_call(
                            printf_fn,
                            &[error_msg.as_pointer_value().into()],
                            "printf_special_float",
                        )
                        .unwrap();

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
                            "exit_call_special",
                        )
                        .unwrap();
                    self.builder.build_unreachable().unwrap();

                    // Normal block: check for overflow before conversion
                    self.builder.position_at_end(normal_block);

                    // Check if float is within valid int range: [-2147483648, 2147483647]
                    let int_max = self.context.f64_type().const_float(2147483647.0);
                    let int_min = self.context.f64_type().const_float(-2147483648.0);

                    let exceeds_max = self
                        .builder
                        .build_float_compare(
                            inkwell::FloatPredicate::OGT,
                            float_val,
                            int_max,
                            "exceeds_max",
                        )
                        .unwrap();

                    let exceeds_min = self
                        .builder
                        .build_float_compare(
                            inkwell::FloatPredicate::OLT,
                            float_val,
                            int_min,
                            "exceeds_min",
                        )
                        .unwrap();

                    let is_overflow = self
                        .builder
                        .build_or(exceeds_max, exceeds_min, "is_overflow")
                        .unwrap();

                    // Create blocks for overflow check
                    let overflow_block = self
                        .context
                        .append_basic_block(parent, "float_to_int_overflow");
                    let safe_block = self.context.append_basic_block(parent, "float_to_int_safe");

                    self.builder
                        .build_conditional_branch(is_overflow, overflow_block, safe_block)
                        .unwrap();

                    // Overflow block: print error and exit
                    self.builder.position_at_end(overflow_block);
                    let overflow_msg = self
                        .builder
                        .build_global_string_ptr(
                            "Runtime error: Float to Int conversion overflow (value outside [-2147483648, 2147483647])\n",
                            "error_msg_overflow",
                        )
                        .unwrap();

                    let printf_fn_overflow =
                        self.module.get_function("printf").unwrap_or_else(|| {
                            let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                            let fn_type = self.context.i32_type().fn_type(&[ptr_type.into()], true);
                            self.module.add_function("printf", fn_type, None)
                        });

                    self.builder
                        .build_call(
                            printf_fn_overflow,
                            &[overflow_msg.as_pointer_value().into()],
                            "printf_overflow",
                        )
                        .unwrap();

                    let exit_fn_overflow = self.module.get_function("exit").unwrap_or_else(|| {
                        let fn_type = self
                            .context
                            .void_type()
                            .fn_type(&[self.context.i32_type().into()], false);
                        self.module.add_function("exit", fn_type, None)
                    });

                    self.builder
                        .build_call(
                            exit_fn_overflow,
                            &[self.context.i32_type().const_int(1, false).into()],
                            "exit_call_overflow",
                        )
                        .unwrap();
                    self.builder.build_unreachable().unwrap();

                    // Safe block: perform the conversion
                    self.builder.position_at_end(safe_block);
                    let int_val = self
                        .builder
                        .build_float_to_signed_int(
                            float_val,
                            self.context.i32_type(),
                            "cast_f_to_i",
                        )
                        .unwrap();
                    int_val.into()
                } else if source_val.is_pointer_value() {
                    let trimmed_val = self
                        .generate_string_method(
                            "tmp_trimmed",
                            "", // object name if needed
                            source_val,
                            "trim",
                            &[],
                        )
                        .unwrap();

                    // Use strtol to parse the trimmed string as Int with validation
                    let strtol_fn = self.module.get_function("strtol").unwrap_or_else(|| {
                        let i64_type = self.context.i64_type();
                        let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                        let fn_type = i64_type.fn_type(
                            &[
                                ptr_type.into(),
                                ptr_type.into(),
                                self.context.i32_type().into(),
                            ],
                            false,
                        );
                        self.module.add_function("strtol", fn_type, None)
                    });

                    let trimmed_ptr = trimmed_val.into_pointer_value();

                    // Allocate space for endptr output
                    let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                    let endptr_ptr = self.builder.build_alloca(ptr_type, "endptr_ptr").unwrap();

                    let base_10 = self.context.i32_type().const_int(10, false);

                    let result_i64 = self
                        .builder
                        .build_call(
                            strtol_fn,
                            &[trimmed_ptr.into(), endptr_ptr.into(), base_10.into()],
                            "strtol_result",
                        )
                        .unwrap()
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_int_value();

                    // Load the endptr to check if entire string was consumed
                    let endptr = self
                        .builder
                        .build_load(ptr_type, endptr_ptr, "endptr")
                        .unwrap()
                        .into_pointer_value();

                    // Check if endptr points to a non-null character (parsing failed or incomplete)
                    let endptr_char = unsafe {
                        self.builder
                            .build_in_bounds_gep(self.context.i8_type(), endptr, &[], "endptr_char")
                            .unwrap()
                    };

                    let endptr_val = self
                        .builder
                        .build_load(self.context.i8_type(), endptr_char, "endptr_val")
                        .unwrap()
                        .into_int_value();

                    let zero_u8 = self.context.i8_type().const_int(0, false);
                    let is_valid = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            endptr_val,
                            zero_u8,
                            "parse_valid",
                        )
                        .unwrap();

                    // If not valid, print error and exit
                    let panic_msg = self
                        .builder
                        .build_global_string_ptr(
                            "Runtime error: invalid string to Int conversion\n",
                            "panic_msg_str_int",
                        )
                        .unwrap();

                    let valid_block = self.context.append_basic_block(
                        self.builder
                            .get_insert_block()
                            .unwrap()
                            .get_parent()
                            .unwrap(),
                        "parse_valid",
                    );
                    let panic_block = self.context.append_basic_block(
                        self.builder
                            .get_insert_block()
                            .unwrap()
                            .get_parent()
                            .unwrap(),
                        "parse_panic",
                    );

                    self.builder
                        .build_conditional_branch(is_valid, valid_block, panic_block)
                        .unwrap();

                    // Panic block: print error and exit
                    self.builder.position_at_end(panic_block);

                    // Get printf function
                    let printf_fn = self.module.get_function("printf").unwrap_or_else(|| {
                        let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                        let fn_type = self.context.i32_type().fn_type(&[ptr_type.into()], true);
                        self.module.add_function("printf", fn_type, None)
                    });

                    // Print error message
                    self.builder
                        .build_call(
                            printf_fn,
                            &[panic_msg.as_pointer_value().into()],
                            "printf_error",
                        )
                        .unwrap();

                    // Get exit function
                    let exit_fn = self.module.get_function("exit").unwrap_or_else(|| {
                        let fn_type = self
                            .context
                            .void_type()
                            .fn_type(&[self.context.i32_type().into()], false);
                        self.module.add_function("exit", fn_type, None)
                    });

                    // Call exit(1)
                    self.builder
                        .build_call(
                            exit_fn,
                            &[self.context.i32_type().const_int(1, false).into()],
                            "exit_call",
                        )
                        .unwrap();
                    self.builder.build_unreachable().unwrap();

                    // Valid block - return the result
                    self.builder.position_at_end(valid_block);
                    let result_i32 = self
                        .builder
                        .build_int_truncate(result_i64, self.context.i32_type(), "strtol_i32")
                        .unwrap();

                    result_i32.into()
                } else {
                    source_val
                }
            }

            // String conversions
            "String" => {
                if source_type == "Bool" {
                    // Bool to String: "true" or "false"
                    if source_val.is_int_value() {
                        let int_val = source_val.into_int_value();
                        // Check if the value is 0 or non-zero
                        let zero = self.context.i32_type().const_int(0, false);
                        let is_zero = self
                            .builder
                            .build_int_compare(inkwell::IntPredicate::EQ, int_val, zero, "is_zero")
                            .unwrap();

                        let true_str = self
                            .builder
                            .build_global_string_ptr("true", "bool_true_str")
                            .unwrap();
                        let false_str = self
                            .builder
                            .build_global_string_ptr("false", "bool_false_str")
                            .unwrap();

                        self.builder
                            .build_select(
                                is_zero,
                                false_str.as_pointer_value(),
                                true_str.as_pointer_value(),
                                "bool_to_str_select",
                            )
                            .unwrap()
                            .into()
                    } else {
                        source_val
                    }
                } else if source_val.is_int_value() {
                    // Int to String: use sprintf to convert int to string
                    self.convert_int_to_string_via_sprintf(source_val.into_int_value())
                } else if source_val.is_float_value() {
                    // Float to String: use sprintf to convert float to string
                    self.convert_float_to_string_via_sprintf(source_val.into_float_value())
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

        // Track the type of the cast result for typeOf()
        // Convert "String" to "Str" for consistency with type display format
        let display_type = if target_type == "String" {
            "Str".to_string()
        } else {
            target_type.to_string()
        };
        self.variable_types.insert(name.to_string(), display_type);

        Some(result)
    }
}
