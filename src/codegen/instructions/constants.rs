use crate::codegen::core::CodeGen;
use inkwell::types::BasicType;
use inkwell::values::BasicValueEnum;
use inkwell::values::FunctionValue;
use inkwell::AddressSpace;

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

        // Store the actual string value in temp_strings for handler name resolution
        self.temp_strings
            .insert(name.to_string(), value.to_string());

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
                } else if source_type.starts_with("Struct(") {
                    // Struct to String: convert to actual string representation
                    let struct_name = source_type
                        .strip_prefix("Struct(")
                        .and_then(|s| s.strip_suffix(")"))
                        .unwrap_or("Unknown");
                    self.struct_to_string(value, struct_name, source_val)
                } else if source_type.starts_with("Enum(") {
                    // Enum to String: convert to actual string representation
                    let enum_name = source_type
                        .strip_prefix("Enum(")
                        .and_then(|s| s.strip_suffix(")"))
                        .unwrap_or("Unknown");
                    self.enum_to_string(value, enum_name, source_val)
                } else if source_type.starts_with("Array(") {
                    // Array to String: convert to actual string representation
                    self.array_to_string(value, source_type)
                } else if source_type.starts_with("Map(") {
                    // Map to String: convert to actual string representation
                    self.map_to_string(value, source_type)
                } else if source_type.starts_with("Tuple(") {
                    // Tuple to String: return placeholder
                    let placeholder = format!("<{}>", source_type);
                    let str_ptr = self
                        .builder
                        .build_global_string_ptr(&placeholder, "tuple_to_str")
                        .unwrap();
                    str_ptr.as_pointer_value().into()
                } else if source_type.starts_with("Result(") {
                    // Result to String: return placeholder
                    let placeholder = format!("<{}>", source_type);
                    let str_ptr = self
                        .builder
                        .build_global_string_ptr(&placeholder, "result_to_str")
                        .unwrap();
                    str_ptr.as_pointer_value().into()
                } else if source_type == "Any" {
                    // Any to String: return placeholder
                    let placeholder = "<Any>";
                    let str_ptr = self
                        .builder
                        .build_global_string_ptr(placeholder, "any_to_str")
                        .unwrap();
                    str_ptr.as_pointer_value().into()
                } else if source_val.is_int_value() {
                    // Int to String: use sprintf to convert int to string
                    self.convert_int_to_string_via_sprintf(source_val.into_int_value())
                } else if source_val.is_float_value() {
                    // Float to String: use sprintf to convert float to string
                    self.convert_float_to_string_via_sprintf(source_val.into_float_value())
                } else if source_val.is_pointer_value() {
                    // Already a pointer (likely a string), return as-is
                    source_val
                } else {
                    // Fallback: return a generic placeholder string
                    let placeholder = "<value>";
                    let str_ptr = self
                        .builder
                        .build_global_string_ptr(placeholder, "unknown_to_str")
                        .unwrap();
                    str_ptr.as_pointer_value().into()
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

    /// Convert an array to a string representation like "[1, 2, 3]"
    fn array_to_string(&mut self, value_name: &str, _source_type: &str) -> BasicValueEnum<'ctx> {
        // Try to find array metadata
        let metadata = self.array_metadata.get(value_name).cloned().or_else(|| {
            // Try variations of the name
            let variations = vec![
                value_name.trim_start_matches('%').to_string(),
                value_name.trim_end_matches("_array").to_string(),
                format!("{}_array", value_name),
            ];
            for var in variations {
                if let Some(meta) = self.array_metadata.get(&var).cloned() {
                    return Some(meta);
                }
            }
            None
        });

        let Some(metadata) = metadata else {
            // Fallback to placeholder if no metadata
            let placeholder = "[]";
            let str_ptr = self
                .builder
                .build_global_string_ptr(placeholder, "array_to_str_fallback")
                .unwrap();
            return str_ptr.as_pointer_value().into();
        };

        // Get array pointer
        let array_ptr = if self.symbols.contains_key(value_name) {
            let var_alloca = self.resolve_pointer(value_name);
            self.builder
                .build_load(
                    self.context.ptr_type(AddressSpace::default()),
                    var_alloca,
                    "array_data_ptr",
                )
                .unwrap()
                .into_pointer_value()
        } else {
            self.resolve_value(value_name).into_pointer_value()
        };

        // Allocate buffer for the string (generous size)
        let malloc_fn = self.get_or_declare_malloc();
        let buffer_size = self.context.i64_type().const_int(8192, false);
        let buffer = self
            .builder
            .build_call(malloc_fn, &[buffer_size.into()], "arr_str_buffer")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Get sprintf and strcat
        let sprintf_fn = self.get_or_declare_sprintf();
        let strcat_fn = self.get_or_declare_strcat();
        let strcpy_fn = self.get_or_declare_strcpy();

        // Check if array is null (empty)
        let current_fn = self
            .builder
            .get_insert_block()
            .unwrap()
            .get_parent()
            .unwrap();
        let null_block = self.context.append_basic_block(current_fn, "arr_str_null");
        let non_null_block = self
            .context
            .append_basic_block(current_fn, "arr_str_non_null");
        let after_block = self.context.append_basic_block(current_fn, "arr_str_after");

        let is_null = self
            .builder
            .build_is_null(array_ptr, "is_null_arr")
            .unwrap();
        self.builder
            .build_conditional_branch(is_null, null_block, non_null_block)
            .unwrap();

        // Null block: return "[]"
        self.builder.position_at_end(null_block);
        let empty_str = self
            .builder
            .build_global_string_ptr("[]", "empty_arr_str")
            .unwrap();
        self.builder
            .build_call(
                strcpy_fn,
                &[buffer.into(), empty_str.as_pointer_value().into()],
                "",
            )
            .unwrap();
        self.builder
            .build_unconditional_branch(after_block)
            .unwrap();

        // Non-null block: build the string
        self.builder.position_at_end(non_null_block);

        // Start with "["
        let open_bracket = self
            .builder
            .build_global_string_ptr("[", "open_bracket")
            .unwrap();
        self.builder
            .build_call(
                strcpy_fn,
                &[buffer.into(), open_bracket.as_pointer_value().into()],
                "",
            )
            .unwrap();

        // Get element type
        let elem_type = if metadata.element_type == "Str" {
            self.context
                .ptr_type(AddressSpace::default())
                .as_basic_type_enum()
        } else if metadata.element_type == "Float" {
            self.context.f64_type().as_basic_type_enum()
        } else if metadata.element_type == "Bool" {
            self.context.i32_type().as_basic_type_enum()
        } else {
            self.context.i32_type().as_basic_type_enum()
        };

        // Read heap length
        let rc_header_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    array_ptr,
                    &[self.context.i32_type().const_int((-8_i32) as u64, true)],
                    "rc_header_ptr",
                )
                .unwrap()
        };
        let len_field_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    rc_header_ptr,
                    &[self.context.i32_type().const_int(4, false)],
                    "len_field_ptr",
                )
                .unwrap()
        };
        let len_ptr_cast = self
            .builder
            .build_pointer_cast(
                len_field_ptr,
                self.context.ptr_type(AddressSpace::default()),
                "len_ptr_cast",
            )
            .unwrap();
        let heap_len = self
            .builder
            .build_load(self.context.i32_type(), len_ptr_cast, "heap_len")
            .unwrap()
            .into_int_value();

        // Create loop to build string
        let loop_header = self
            .context
            .append_basic_block(current_fn, "arr_str_loop_header");
        let loop_body = self
            .context
            .append_basic_block(current_fn, "arr_str_loop_body");
        let loop_end = self
            .context
            .append_basic_block(current_fn, "arr_str_loop_end");

        let counter_alloca = self
            .builder
            .build_alloca(self.context.i32_type(), "arr_str_counter")
            .unwrap();
        self.builder
            .build_store(counter_alloca, self.context.i32_type().const_zero())
            .unwrap();
        self.builder
            .build_unconditional_branch(loop_header)
            .unwrap();

        // Loop header
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

        // Loop body
        self.builder.position_at_end(loop_body);

        // Get element
        let elem_ptr = unsafe {
            self.builder
                .build_gep(elem_type, array_ptr, &[counter_val], "elem_ptr")
                .unwrap()
        };
        let elem_val = self
            .builder
            .build_load(elem_type, elem_ptr, "elem")
            .unwrap();

        // Allocate temp buffer for element
        let elem_buffer = self
            .builder
            .build_call(
                malloc_fn,
                &[self.context.i64_type().const_int(256, false).into()],
                "elem_buffer",
            )
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Format element based on type
        if metadata.element_type == "Str" {
            let fmt = self
                .builder
                .build_global_string_ptr("\"%s\"", "str_fmt")
                .unwrap();
            self.builder
                .build_call(
                    sprintf_fn,
                    &[
                        elem_buffer.into(),
                        fmt.as_pointer_value().into(),
                        elem_val.into(),
                    ],
                    "",
                )
                .unwrap();
        } else if metadata.element_type == "Float" {
            let fmt = self
                .builder
                .build_global_string_ptr("%g", "float_fmt")
                .unwrap();
            self.builder
                .build_call(
                    sprintf_fn,
                    &[
                        elem_buffer.into(),
                        fmt.as_pointer_value().into(),
                        elem_val.into(),
                    ],
                    "",
                )
                .unwrap();
        } else if metadata.element_type == "Bool" {
            let int_val = elem_val.into_int_value();
            let zero = self.context.i32_type().const_int(0, false);
            let is_true = self
                .builder
                .build_int_compare(inkwell::IntPredicate::NE, int_val, zero, "is_true")
                .unwrap();
            let true_str = self
                .builder
                .build_global_string_ptr("true", "true_str")
                .unwrap();
            let false_str = self
                .builder
                .build_global_string_ptr("false", "false_str")
                .unwrap();
            let bool_str = self
                .builder
                .build_select(
                    is_true,
                    true_str.as_pointer_value(),
                    false_str.as_pointer_value(),
                    "bool_str",
                )
                .unwrap();
            self.builder
                .build_call(strcpy_fn, &[elem_buffer.into(), bool_str.into()], "")
                .unwrap();
        } else {
            let fmt = self
                .builder
                .build_global_string_ptr("%d", "int_fmt")
                .unwrap();
            self.builder
                .build_call(
                    sprintf_fn,
                    &[
                        elem_buffer.into(),
                        fmt.as_pointer_value().into(),
                        elem_val.into(),
                    ],
                    "",
                )
                .unwrap();
        }

        // Append element to buffer
        self.builder
            .build_call(strcat_fn, &[buffer.into(), elem_buffer.into()], "")
            .unwrap();

        // Add comma if not last element
        let next_counter = self
            .builder
            .build_int_add(
                counter_val,
                self.context.i32_type().const_int(1, false),
                "next_counter",
            )
            .unwrap();
        let is_last = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, next_counter, heap_len, "is_last")
            .unwrap();

        let comma_block = self.context.append_basic_block(current_fn, "add_comma");
        let no_comma_block = self.context.append_basic_block(current_fn, "no_comma");

        self.builder
            .build_conditional_branch(is_last, no_comma_block, comma_block)
            .unwrap();

        self.builder.position_at_end(comma_block);
        let comma = self.builder.build_global_string_ptr(", ", "comma").unwrap();
        self.builder
            .build_call(
                strcat_fn,
                &[buffer.into(), comma.as_pointer_value().into()],
                "",
            )
            .unwrap();
        self.builder
            .build_unconditional_branch(no_comma_block)
            .unwrap();

        self.builder.position_at_end(no_comma_block);
        // Free elem_buffer
        let free_fn = self.get_or_declare_free();
        self.builder
            .build_call(free_fn, &[elem_buffer.into()], "")
            .unwrap();

        self.builder
            .build_store(counter_alloca, next_counter)
            .unwrap();
        self.builder
            .build_unconditional_branch(loop_header)
            .unwrap();

        // Loop end
        self.builder.position_at_end(loop_end);
        let close_bracket = self
            .builder
            .build_global_string_ptr("]", "close_bracket")
            .unwrap();
        self.builder
            .build_call(
                strcat_fn,
                &[buffer.into(), close_bracket.as_pointer_value().into()],
                "",
            )
            .unwrap();
        self.builder
            .build_unconditional_branch(after_block)
            .unwrap();

        // After block
        self.builder.position_at_end(after_block);
        buffer.into()
    }

    /// Convert a map to a string representation like "{\"a\": 1, \"b\": 2}"
    fn map_to_string(&mut self, value_name: &str, _source_type: &str) -> BasicValueEnum<'ctx> {
        // Try to find map metadata
        let metadata = self.map_metadata.get(value_name).cloned().or_else(|| {
            let variations = vec![
                value_name.trim_start_matches('%').to_string(),
                format!("{}_map", value_name),
            ];
            for var in variations {
                if let Some(meta) = self.map_metadata.get(&var).cloned() {
                    return Some(meta);
                }
            }
            None
        });

        let Some(metadata) = metadata else {
            let placeholder = "{}";
            let str_ptr = self
                .builder
                .build_global_string_ptr(placeholder, "map_to_str_fallback")
                .unwrap();
            return str_ptr.as_pointer_value().into();
        };

        // Get map pointer
        let map_ptr = if self.symbols.contains_key(value_name) {
            let var_alloca = self.resolve_pointer(value_name);
            self.builder
                .build_load(
                    self.context.ptr_type(AddressSpace::default()),
                    var_alloca,
                    "map_data_ptr",
                )
                .unwrap()
                .into_pointer_value()
        } else {
            self.resolve_value(value_name).into_pointer_value()
        };

        // Allocate buffer
        let malloc_fn = self.get_or_declare_malloc();
        let buffer_size = self.context.i64_type().const_int(16384, false);
        let buffer = self
            .builder
            .build_call(malloc_fn, &[buffer_size.into()], "map_str_buffer")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        let sprintf_fn = self.get_or_declare_sprintf();
        let strcat_fn = self.get_or_declare_strcat();
        let strcpy_fn = self.get_or_declare_strcpy();

        let current_fn = self
            .builder
            .get_insert_block()
            .unwrap()
            .get_parent()
            .unwrap();
        let null_block = self.context.append_basic_block(current_fn, "map_str_null");
        let non_null_block = self
            .context
            .append_basic_block(current_fn, "map_str_non_null");
        let after_block = self.context.append_basic_block(current_fn, "map_str_after");

        let is_null = self.builder.build_is_null(map_ptr, "is_null_map").unwrap();
        self.builder
            .build_conditional_branch(is_null, null_block, non_null_block)
            .unwrap();

        // Null block
        self.builder.position_at_end(null_block);
        let empty_str = self
            .builder
            .build_global_string_ptr("{}", "empty_map_str")
            .unwrap();
        self.builder
            .build_call(
                strcpy_fn,
                &[buffer.into(), empty_str.as_pointer_value().into()],
                "",
            )
            .unwrap();
        self.builder
            .build_unconditional_branch(after_block)
            .unwrap();

        // Non-null block
        self.builder.position_at_end(non_null_block);
        let open_brace = self
            .builder
            .build_global_string_ptr("{", "open_brace")
            .unwrap();
        self.builder
            .build_call(
                strcpy_fn,
                &[buffer.into(), open_brace.as_pointer_value().into()],
                "",
            )
            .unwrap();

        // Determine key and value types
        let key_type: inkwell::types::BasicTypeEnum = if metadata.key_is_string {
            self.context.ptr_type(AddressSpace::default()).into()
        } else if metadata.key_type == "Float" {
            self.context.f64_type().into()
        } else if metadata.key_type == "Bool" {
            self.context.i32_type().into()
        } else {
            self.context.i32_type().into()
        };

        let val_type: inkwell::types::BasicTypeEnum = if metadata.value_is_string {
            self.context.ptr_type(AddressSpace::default()).into()
        } else if metadata.value_type == "Float" {
            self.context.f64_type().into()
        } else if metadata.value_type == "Bool" {
            self.context.i32_type().into()
        } else {
            self.context.i32_type().into()
        };

        // Read heap length
        let len_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    map_ptr,
                    &[self.context.i32_type().const_int((-4_i32) as u64, true)],
                    "map_len_ptr",
                )
                .unwrap()
        };
        let len_ptr_cast = self
            .builder
            .build_pointer_cast(
                len_ptr,
                self.context.ptr_type(AddressSpace::default()),
                "map_len_cast",
            )
            .unwrap();
        let heap_len = self
            .builder
            .build_load(self.context.i32_type(), len_ptr_cast, "map_heap_len")
            .unwrap()
            .into_int_value();

        // Create entry struct type
        let entry_type = self.context.struct_type(&[key_type, val_type], false);

        // Loop to build string
        let loop_header = self
            .context
            .append_basic_block(current_fn, "map_str_loop_header");
        let loop_body = self
            .context
            .append_basic_block(current_fn, "map_str_loop_body");
        let loop_end = self
            .context
            .append_basic_block(current_fn, "map_str_loop_end");

        let counter_alloca = self
            .builder
            .build_alloca(self.context.i32_type(), "map_str_counter")
            .unwrap();
        self.builder
            .build_store(counter_alloca, self.context.i32_type().const_zero())
            .unwrap();
        self.builder
            .build_unconditional_branch(loop_header)
            .unwrap();

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

        self.builder.position_at_end(loop_body);

        // Get entry at index
        let entry_ptr = unsafe {
            self.builder
                .build_gep(entry_type, map_ptr, &[counter_val], "entry_ptr")
                .unwrap()
        };

        // Get key
        let key_ptr = self
            .builder
            .build_struct_gep(entry_type, entry_ptr, 0, "key_ptr")
            .unwrap();
        let key_val = self
            .builder
            .build_load(key_type, key_ptr, "key_val")
            .unwrap();

        // Get value
        let val_ptr = self
            .builder
            .build_struct_gep(entry_type, entry_ptr, 1, "val_ptr")
            .unwrap();
        let val_val = self
            .builder
            .build_load(val_type, val_ptr, "val_val")
            .unwrap();

        // Format key:value pair
        let elem_buffer = self
            .builder
            .build_call(
                malloc_fn,
                &[self.context.i64_type().const_int(512, false).into()],
                "kv_buffer",
            )
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Format key
        if metadata.key_is_string {
            let fmt = self
                .builder
                .build_global_string_ptr("\"%s\": ", "key_str_fmt")
                .unwrap();
            self.builder
                .build_call(
                    sprintf_fn,
                    &[
                        elem_buffer.into(),
                        fmt.as_pointer_value().into(),
                        key_val.into(),
                    ],
                    "",
                )
                .unwrap();
        } else if metadata.key_type == "Float" {
            let fmt = self
                .builder
                .build_global_string_ptr("%g: ", "key_float_fmt")
                .unwrap();
            self.builder
                .build_call(
                    sprintf_fn,
                    &[
                        elem_buffer.into(),
                        fmt.as_pointer_value().into(),
                        key_val.into(),
                    ],
                    "",
                )
                .unwrap();
        } else if metadata.key_type == "Bool" {
            let int_val = key_val.into_int_value();
            let zero = self.context.i32_type().const_int(0, false);
            let is_true = self
                .builder
                .build_int_compare(inkwell::IntPredicate::NE, int_val, zero, "is_true")
                .unwrap();
            let true_str = self
                .builder
                .build_global_string_ptr("true: ", "true_key")
                .unwrap();
            let false_str = self
                .builder
                .build_global_string_ptr("false: ", "false_key")
                .unwrap();
            let key_str = self
                .builder
                .build_select(
                    is_true,
                    true_str.as_pointer_value(),
                    false_str.as_pointer_value(),
                    "key_str",
                )
                .unwrap();
            self.builder
                .build_call(strcpy_fn, &[elem_buffer.into(), key_str.into()], "")
                .unwrap();
        } else {
            let fmt = self
                .builder
                .build_global_string_ptr("%d: ", "key_int_fmt")
                .unwrap();
            self.builder
                .build_call(
                    sprintf_fn,
                    &[
                        elem_buffer.into(),
                        fmt.as_pointer_value().into(),
                        key_val.into(),
                    ],
                    "",
                )
                .unwrap();
        }

        self.builder
            .build_call(strcat_fn, &[buffer.into(), elem_buffer.into()], "")
            .unwrap();

        // Format value
        if metadata.value_is_string {
            let fmt = self
                .builder
                .build_global_string_ptr("\"%s\"", "val_str_fmt")
                .unwrap();
            self.builder
                .build_call(
                    sprintf_fn,
                    &[
                        elem_buffer.into(),
                        fmt.as_pointer_value().into(),
                        val_val.into(),
                    ],
                    "",
                )
                .unwrap();
        } else if metadata.value_type == "Float" {
            let fmt = self
                .builder
                .build_global_string_ptr("%g", "val_float_fmt")
                .unwrap();
            self.builder
                .build_call(
                    sprintf_fn,
                    &[
                        elem_buffer.into(),
                        fmt.as_pointer_value().into(),
                        val_val.into(),
                    ],
                    "",
                )
                .unwrap();
        } else if metadata.value_type == "Bool" {
            let int_val = val_val.into_int_value();
            let zero = self.context.i32_type().const_int(0, false);
            let is_true = self
                .builder
                .build_int_compare(inkwell::IntPredicate::NE, int_val, zero, "is_true")
                .unwrap();
            let true_str = self
                .builder
                .build_global_string_ptr("true", "true_val")
                .unwrap();
            let false_str = self
                .builder
                .build_global_string_ptr("false", "false_val")
                .unwrap();
            let val_str = self
                .builder
                .build_select(
                    is_true,
                    true_str.as_pointer_value(),
                    false_str.as_pointer_value(),
                    "val_str",
                )
                .unwrap();
            self.builder
                .build_call(strcpy_fn, &[elem_buffer.into(), val_str.into()], "")
                .unwrap();
        } else {
            let fmt = self
                .builder
                .build_global_string_ptr("%d", "val_int_fmt")
                .unwrap();
            self.builder
                .build_call(
                    sprintf_fn,
                    &[
                        elem_buffer.into(),
                        fmt.as_pointer_value().into(),
                        val_val.into(),
                    ],
                    "",
                )
                .unwrap();
        }

        self.builder
            .build_call(strcat_fn, &[buffer.into(), elem_buffer.into()], "")
            .unwrap();

        // Add comma if not last
        let next_counter = self
            .builder
            .build_int_add(
                counter_val,
                self.context.i32_type().const_int(1, false),
                "next_counter",
            )
            .unwrap();
        let is_last = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, next_counter, heap_len, "is_last")
            .unwrap();

        let comma_block = self.context.append_basic_block(current_fn, "map_add_comma");
        let no_comma_block = self.context.append_basic_block(current_fn, "map_no_comma");

        self.builder
            .build_conditional_branch(is_last, no_comma_block, comma_block)
            .unwrap();

        self.builder.position_at_end(comma_block);
        let comma = self.builder.build_global_string_ptr(", ", "comma").unwrap();
        self.builder
            .build_call(
                strcat_fn,
                &[buffer.into(), comma.as_pointer_value().into()],
                "",
            )
            .unwrap();
        self.builder
            .build_unconditional_branch(no_comma_block)
            .unwrap();

        self.builder.position_at_end(no_comma_block);
        let free_fn = self.get_or_declare_free();
        self.builder
            .build_call(free_fn, &[elem_buffer.into()], "")
            .unwrap();
        self.builder
            .build_store(counter_alloca, next_counter)
            .unwrap();
        self.builder
            .build_unconditional_branch(loop_header)
            .unwrap();

        self.builder.position_at_end(loop_end);
        let close_brace = self
            .builder
            .build_global_string_ptr("}", "close_brace")
            .unwrap();
        self.builder
            .build_call(
                strcat_fn,
                &[buffer.into(), close_brace.as_pointer_value().into()],
                "",
            )
            .unwrap();
        self.builder
            .build_unconditional_branch(after_block)
            .unwrap();

        self.builder.position_at_end(after_block);
        buffer.into()
    }

    /// Convert a struct to a string representation like "Point { x: 10, y: 20 }"
    fn struct_to_string(
        &mut self,
        _value_name: &str,
        struct_name: &str,
        source_val: BasicValueEnum<'ctx>,
    ) -> BasicValueEnum<'ctx> {
        let metadata = self.struct_metadata.get(struct_name).cloned();
        let canonical_type = self.canonical_struct_types.get(struct_name).cloned();

        let (Some(metadata), Some(canonical_type)) = (metadata, canonical_type) else {
            let placeholder = format!("{} {{ ... }}", struct_name);
            let str_ptr = self
                .builder
                .build_global_string_ptr(&placeholder, "struct_to_str_fallback")
                .unwrap();
            return str_ptr.as_pointer_value().into();
        };

        let malloc_fn = self.get_or_declare_malloc();
        let sprintf_fn = self.get_or_declare_sprintf();
        let strcat_fn = self.get_or_declare_strcat();
        let strcpy_fn = self.get_or_declare_strcpy();

        let buffer_size = self.context.i64_type().const_int(4096, false);
        let buffer = self
            .builder
            .build_call(malloc_fn, &[buffer_size.into()], "struct_str_buffer")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Start with "StructName { "
        let header = format!("{} {{ ", struct_name);
        let header_str = self
            .builder
            .build_global_string_ptr(&header, "struct_header")
            .unwrap();
        self.builder
            .build_call(
                strcpy_fn,
                &[buffer.into(), header_str.as_pointer_value().into()],
                "",
            )
            .unwrap();

        // Get struct pointer
        let struct_ptr = if source_val.is_pointer_value() {
            source_val.into_pointer_value()
        } else {
            let struct_alloca = self
                .builder
                .build_alloca(canonical_type, "struct_tmp_alloca")
                .unwrap();
            self.builder.build_store(struct_alloca, source_val).unwrap();
            struct_alloca
        };

        let elem_buffer = self
            .builder
            .build_call(
                malloc_fn,
                &[self.context.i64_type().const_int(256, false).into()],
                "field_buffer",
            )
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        // Iterate through fields
        for (field_idx, (field_name, field_type)) in metadata
            .field_names
            .iter()
            .zip(metadata.field_types.iter())
            .enumerate()
        {
            // Get field value
            let field_ptr = self
                .builder
                .build_struct_gep(
                    canonical_type,
                    struct_ptr,
                    field_idx as u32,
                    &format!("field_{}_ptr", field_name),
                )
                .unwrap();

            let llvm_field_type = self.type_string_to_llvm(field_type);
            let field_val = self
                .builder
                .build_load(
                    llvm_field_type,
                    field_ptr,
                    &format!("field_{}_val", field_name),
                )
                .unwrap();

            // Format field name
            let field_name_fmt = format!("{}: ", field_name);
            let field_name_str = self
                .builder
                .build_global_string_ptr(&field_name_fmt, "field_name")
                .unwrap();
            self.builder
                .build_call(
                    strcat_fn,
                    &[buffer.into(), field_name_str.as_pointer_value().into()],
                    "",
                )
                .unwrap();

            // Format field value based on type
            if field_type == "Str" || field_type == "String" {
                let fmt = self
                    .builder
                    .build_global_string_ptr("\"%s\"", "str_fmt")
                    .unwrap();
                self.builder
                    .build_call(
                        sprintf_fn,
                        &[
                            elem_buffer.into(),
                            fmt.as_pointer_value().into(),
                            field_val.into(),
                        ],
                        "",
                    )
                    .unwrap();
            } else if field_type == "Float" {
                let fmt = self
                    .builder
                    .build_global_string_ptr("%g", "float_fmt")
                    .unwrap();
                self.builder
                    .build_call(
                        sprintf_fn,
                        &[
                            elem_buffer.into(),
                            fmt.as_pointer_value().into(),
                            field_val.into(),
                        ],
                        "",
                    )
                    .unwrap();
            } else if field_type == "Bool" {
                let int_val = field_val.into_int_value();
                let zero = self.context.i32_type().const_int(0, false);
                let is_true = self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::NE, int_val, zero, "is_true")
                    .unwrap();
                let true_str = self
                    .builder
                    .build_global_string_ptr("true", "true_str")
                    .unwrap();
                let false_str = self
                    .builder
                    .build_global_string_ptr("false", "false_str")
                    .unwrap();
                let bool_str = self
                    .builder
                    .build_select(
                        is_true,
                        true_str.as_pointer_value(),
                        false_str.as_pointer_value(),
                        "bool_str",
                    )
                    .unwrap();
                self.builder
                    .build_call(strcpy_fn, &[elem_buffer.into(), bool_str.into()], "")
                    .unwrap();
            } else {
                let fmt = self
                    .builder
                    .build_global_string_ptr("%d", "int_fmt")
                    .unwrap();
                self.builder
                    .build_call(
                        sprintf_fn,
                        &[
                            elem_buffer.into(),
                            fmt.as_pointer_value().into(),
                            field_val.into(),
                        ],
                        "",
                    )
                    .unwrap();
            }

            self.builder
                .build_call(strcat_fn, &[buffer.into(), elem_buffer.into()], "")
                .unwrap();

            // Add separator if not last field
            if field_idx < metadata.field_names.len() - 1 {
                let sep = self.builder.build_global_string_ptr(", ", "sep").unwrap();
                self.builder
                    .build_call(
                        strcat_fn,
                        &[buffer.into(), sep.as_pointer_value().into()],
                        "",
                    )
                    .unwrap();
            }
        }

        // Close with " }"
        let close = self
            .builder
            .build_global_string_ptr(" }", "close_brace")
            .unwrap();
        self.builder
            .build_call(
                strcat_fn,
                &[buffer.into(), close.as_pointer_value().into()],
                "",
            )
            .unwrap();

        let free_fn = self.get_or_declare_free();
        self.builder
            .build_call(free_fn, &[elem_buffer.into()], "")
            .unwrap();

        buffer.into()
    }

    /// Convert an enum to a string representation like "Status::Active"
    fn enum_to_string(
        &mut self,
        _value_name: &str,
        enum_name: &str,
        source_val: BasicValueEnum<'ctx>,
    ) -> BasicValueEnum<'ctx> {
        // Get enum variant names from enum_variant_order or enum_table
        let variant_names: Option<Vec<String>> = self
            .enum_variant_order
            .get(enum_name)
            .map(|v| v.iter().map(|(name, _)| name.clone()).collect())
            .or_else(|| {
                // Fallback: try enum_table (for enums declared inside functions)
                self.enum_table.get(enum_name).map(|variants_map| {
                    let mut names: Vec<String> = variants_map.keys().cloned().collect();
                    names.sort(); // Sort for deterministic ordering
                    names
                })
            });

        let Some(variants) = variant_names else {
            let placeholder = format!("{}::?", enum_name);
            let str_ptr = self
                .builder
                .build_global_string_ptr(&placeholder, "enum_to_str_fallback")
                .unwrap();
            return str_ptr.as_pointer_value().into();
        };

        // Get the tag value
        let tag_val = if source_val.is_int_value() {
            source_val.into_int_value()
        } else if source_val.is_struct_value() {
            // Enum is represented as a struct {i32, ptr} - extract the tag (first field)
            let struct_val = source_val.into_struct_value();
            self.builder
                .build_extract_value(struct_val, 0, "enum_tag")
                .unwrap()
                .into_int_value()
        } else if source_val.is_pointer_value() {
            // Load tag from struct pointer
            let enum_ptr = source_val.into_pointer_value();
            self.builder
                .build_load(self.context.i32_type(), enum_ptr, "enum_tag")
                .unwrap()
                .into_int_value()
        } else {
            // Fallback
            let placeholder = format!("{}::?", enum_name);
            let str_ptr = self
                .builder
                .build_global_string_ptr(&placeholder, "enum_to_str_fallback2")
                .unwrap();
            return str_ptr.as_pointer_value().into();
        };

        // Build a switch to select the right variant string
        let current_fn = self
            .builder
            .get_insert_block()
            .unwrap()
            .get_parent()
            .unwrap();
        let after_block = self
            .context
            .append_basic_block(current_fn, "enum_str_after");

        // Allocate result pointer
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let result_alloca = self
            .builder
            .build_alloca(ptr_type, "enum_str_result")
            .unwrap();

        // Create blocks for each variant
        let mut cases = Vec::new();
        for (idx, variant_name) in variants.iter().enumerate() {
            let variant_block = self
                .context
                .append_basic_block(current_fn, &format!("enum_variant_{}", idx));
            cases.push((
                self.context.i32_type().const_int(idx as u64, false),
                variant_block,
            ));
        }

        let default_block = self.context.append_basic_block(current_fn, "enum_default");

        self.builder
            .build_switch(tag_val, default_block, &cases)
            .unwrap();

        // Generate each variant block
        for (idx, variant_name) in variants.iter().enumerate() {
            self.builder.position_at_end(cases[idx].1);
            let variant_str = format!("{}::{}", enum_name, variant_name);
            let str_ptr = self
                .builder
                .build_global_string_ptr(&variant_str, &format!("enum_str_{}", idx))
                .unwrap();
            self.builder
                .build_store(result_alloca, str_ptr.as_pointer_value())
                .unwrap();
            self.builder
                .build_unconditional_branch(after_block)
                .unwrap();
        }

        // Default block
        self.builder.position_at_end(default_block);
        let default_str = format!("{}::?", enum_name);
        let default_ptr = self
            .builder
            .build_global_string_ptr(&default_str, "enum_default_str")
            .unwrap();
        self.builder
            .build_store(result_alloca, default_ptr.as_pointer_value())
            .unwrap();
        self.builder
            .build_unconditional_branch(after_block)
            .unwrap();

        // After block
        self.builder.position_at_end(after_block);
        let result = self
            .builder
            .build_load(ptr_type, result_alloca, "enum_str_final")
            .unwrap();
        result
    }

    /// Helper to get or declare strcat function
    fn get_or_declare_strcat(&self) -> FunctionValue<'ctx> {
        if let Some(func) = self.module.get_function("strcat") {
            return func;
        }
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
        self.module.add_function("strcat", fn_type, None)
    }

    /// Helper to get or declare strcpy function
    fn get_or_declare_strcpy(&self) -> FunctionValue<'ctx> {
        if let Some(func) = self.module.get_function("strcpy") {
            return func;
        }
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let fn_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
        self.module.add_function("strcpy", fn_type, None)
    }

    /// Helper to convert type string to LLVM type
    fn type_string_to_llvm(&self, type_str: &str) -> inkwell::types::BasicTypeEnum<'ctx> {
        match type_str {
            "Int" => self.context.i32_type().as_basic_type_enum(),
            "Float" => self.context.f64_type().as_basic_type_enum(),
            "Bool" => self.context.i32_type().as_basic_type_enum(),
            "Str" | "String" => self
                .context
                .ptr_type(AddressSpace::default())
                .as_basic_type_enum(),
            _ => self.context.i32_type().as_basic_type_enum(),
        }
    }
}
