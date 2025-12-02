use crate::codegen::core::CodeGen;
use inkwell::values::BasicValueEnum;

use inkwell::{FloatPredicate, IntPredicate};

impl<'ctx> CodeGen<'ctx> {
    pub fn generate_binary_op(
        &mut self,
        op: &str,
        dst: &str,
        lhs: &str,
        rhs: &str,
    ) -> Option<inkwell::values::BasicValueEnum<'ctx>> {
        // Check if this is a string concatenation (add operation with pointer operands)
        let mut lhs_val = self.resolve_value(lhs);
        let mut rhs_val = self.resolve_value(rhs);

        // Support op:type format for int/float operations
        let parts: Vec<&str> = op.split(':').collect();
        let op_name = parts[0];
        let op_type = parts.get(1).copied().unwrap_or("int");

        // Handle pointer-to-nil comparisons (e.g., err != nil)
        // Convert nil (int 0) to null pointer for comparison
        if (op_name == "eq" || op_name == "ne") {
            if lhs_val.is_pointer_value() && rhs_val.is_int_value() {
                let rhs_int = rhs_val.into_int_value();
                if rhs_int.is_const() && rhs_int.get_zero_extended_constant() == Some(0) {
                    // rhs is nil (0), convert to null pointer
                    rhs_val = self
                        .context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into();
                }
            } else if rhs_val.is_pointer_value() && lhs_val.is_int_value() {
                let lhs_int = lhs_val.into_int_value();
                if lhs_int.is_const() && lhs_int.get_zero_extended_constant() == Some(0) {
                    // lhs is nil (0), convert to null pointer
                    lhs_val = self
                        .context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null()
                        .into();
                }
            }
            // Handle bool (i1) to nil comparisons
            // Bool error types are i1, and need to be compared with nil (i32 0)
            else if lhs_val.is_int_value() && rhs_val.is_int_value() {
                let lhs_int = lhs_val.into_int_value();
                let rhs_int = rhs_val.into_int_value();

                // Check if one is i1 (bool) and the other is i32 (nil)
                let lhs_is_bool = lhs_int.get_type().get_bit_width() == 1;
                let rhs_is_bool = rhs_int.get_type().get_bit_width() == 1;
                let lhs_is_i32 = lhs_int.get_type().get_bit_width() == 32;
                let rhs_is_i32 = rhs_int.get_type().get_bit_width() == 32;

                if lhs_is_bool && rhs_is_i32 {
                    // Extend i1 to i32 for comparison
                    lhs_val = self
                        .builder
                        .build_int_z_extend(lhs_int, self.context.i32_type(), "bool_to_i32")
                        .unwrap()
                        .into();
                } else if rhs_is_bool && lhs_is_i32 {
                    // Extend i1 to i32 for comparison
                    rhs_val = self
                        .builder
                        .build_int_z_extend(rhs_int, self.context.i32_type(), "bool_to_i32")
                        .unwrap()
                        .into();
                }
            }
        }

        // If both are pointers and operation is "add", treat as string concatenation
        if op == "add" && lhs_val.is_pointer_value() && rhs_val.is_pointer_value() {
            // Delegate to string concatenation logic
            return self.generate_instr(&crate::mir::MirInstr::StringConcat {
                name: dst.to_string(),
                left: lhs.to_string(),
                right: rhs.to_string(),
            });
        }

        // Track that this destination is a boolean if the operation type is "bool"
        if op_type == "bool" {
            self.boolean_temps.insert(dst.to_string());
        }

        // String concatenation for pointers
        if op_name == "add" && lhs_val.is_pointer_value() && rhs_val.is_pointer_value() {
            return self.generate_instr(&crate::mir::MirInstr::StringConcat {
                name: dst.to_string(),
                left: lhs.to_string(),
                right: rhs.to_string(),
            });
        }

        // Handle pointer comparisons (including nil checks)
        if (op_name == "eq" || op_name == "ne")
            && lhs_val.is_pointer_value()
            && rhs_val.is_pointer_value()
        {
            // Check if this is a nil comparison (null pointer check)
            let lhs_ptr = lhs_val.into_pointer_value();
            let rhs_ptr = rhs_val.into_pointer_value();

            // Use pointer comparison for nil checks and other pointer comparisons
            let null_ptr = self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .const_null();

            let is_nil_check = (lhs_ptr.is_const() && lhs_ptr.is_null())
                || (rhs_ptr.is_const() && rhs_ptr.is_null());

            if is_nil_check || op_type == "array" || op_type == "map" {
                // Direct pointer comparison for nil checks, arrays, and maps
                let ptr_as_int_lhs = self
                    .builder
                    .build_ptr_to_int(lhs_ptr, self.context.i64_type(), "ptr_to_int_lhs")
                    .unwrap();
                let ptr_as_int_rhs = self
                    .builder
                    .build_ptr_to_int(rhs_ptr, self.context.i64_type(), "ptr_to_int_rhs")
                    .unwrap();

                let cmp_i1 = if op_name == "eq" {
                    self.builder
                        .build_int_compare(
                            IntPredicate::EQ,
                            ptr_as_int_lhs,
                            ptr_as_int_rhs,
                            "ptreq_tmp",
                        )
                        .unwrap()
                } else {
                    self.builder
                        .build_int_compare(
                            IntPredicate::NE,
                            ptr_as_int_lhs,
                            ptr_as_int_rhs,
                            "ptrne_tmp",
                        )
                        .unwrap()
                };

                let result = self
                    .builder
                    .build_int_z_extend(cmp_i1, self.context.i32_type(), "ptr_cmp_ext")
                    .unwrap();

                self.boolean_temps.insert(dst.to_string());
                self.temp_values.insert(dst.to_string(), result.into());
                if let Some(sym) = self.symbols.get(dst) {
                    self.builder.build_store(sym.ptr, result).unwrap();
                }
                self.variable_types
                    .insert(dst.to_string(), "Bool".to_string());
                return Some(result.into());
            }

            // String comparison using strcmp for non-nil checks
            // Declare/get strcmp function
            let strcmp_fn = self.module.get_function("strcmp").unwrap_or_else(|| {
                let i8_ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
                let fn_type = self
                    .context
                    .i32_type()
                    .fn_type(&[i8_ptr_type.into(), i8_ptr_type.into()], false);
                self.module.add_function("strcmp", fn_type, None)
            });

            // Call strcmp
            let cmp_result = self
                .builder
                .build_call(
                    strcmp_fn,
                    &[lhs_ptr.into(), rhs_ptr.into()],
                    "strcmp_result",
                )
                .unwrap()
                .try_as_basic_value()
                .left()
                .unwrap()
                .into_int_value();

            // Compare result with 0
            let zero = self.context.i32_type().const_int(0, false);
            let cmp_i1 = if op_name == "eq" {
                self.builder
                    .build_int_compare(IntPredicate::EQ, cmp_result, zero, "streq_tmp")
                    .unwrap()
            } else {
                self.builder
                    .build_int_compare(IntPredicate::NE, cmp_result, zero, "strne_tmp")
                    .unwrap()
            };

            // Extend i1 to i32 for consistency
            let result = self
                .builder
                .build_int_z_extend(cmp_i1, self.context.i32_type(), "str_cmp_ext")
                .unwrap();

            // Track as boolean for print formatting
            self.boolean_temps.insert(dst.to_string());
            self.temp_values.insert(dst.to_string(), result.into());
            if let Some(sym) = self.symbols.get(dst) {
                self.builder.build_store(sym.ptr, result).unwrap();
            }
            return Some(result.into());
        }

        // Handle array and map comparisons (only eq and ne are supported)
        if (op_type == "array" || op_type == "map")
            && lhs_val.is_pointer_value()
            && rhs_val.is_pointer_value()
        {
            let lhs_ptr = lhs_val.into_pointer_value();
            let rhs_ptr = rhs_val.into_pointer_value();

            // For array/map comparisons, we compare pointer values using ptrtoint
            let ptr_type = self.context.i64_type();
            let lhs_int = self
                .builder
                .build_ptr_to_int(lhs_ptr, ptr_type, "lhs_ptr_int")
                .unwrap();
            let rhs_int = self
                .builder
                .build_ptr_to_int(rhs_ptr, ptr_type, "rhs_ptr_int")
                .unwrap();

            let result = if op_name == "eq" {
                self.builder
                    .build_int_compare(inkwell::IntPredicate::EQ, lhs_int, rhs_int, "array_eq_tmp")
                    .unwrap()
            } else if op_name == "ne" {
                self.builder
                    .build_int_compare(inkwell::IntPredicate::NE, lhs_int, rhs_int, "array_ne_tmp")
                    .unwrap()
            } else {
                debug_assert!(
                    false,
                    "Only eq and ne operations are supported for arrays/maps"
                );
                return Some(self.context.i32_type().const_int(0, false).into());
            };

            self.temp_values.insert(dst.to_string(), result.into());
            if let Some(sym) = self.symbols.get(dst) {
                self.builder.build_store(sym.ptr, result).unwrap();
            }
            return Some(result.into());
        }

        let res: BasicValueEnum<'ctx> = if op_type == "float" {
            if lhs_val.is_float_value() && rhs_val.is_float_value() {
                let lhs_float = lhs_val.into_float_value();
                let rhs_float = rhs_val.into_float_value();
                match op_name {
                    "add" => self
                        .builder
                        .build_float_add(lhs_float, rhs_float, "fadd_tmp")
                        .unwrap()
                        .into(),
                    "sub" => self
                        .builder
                        .build_float_sub(lhs_float, rhs_float, "fsub_tmp")
                        .unwrap()
                        .into(),
                    "mul" => self
                        .builder
                        .build_float_mul(lhs_float, rhs_float, "fmul_tmp")
                        .unwrap()
                        .into(),
                    "div" => self
                        .builder
                        .build_float_div(lhs_float, rhs_float, "fdiv_tmp")
                        .unwrap()
                        .into(),
                    "eq" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::OEQ,
                                lhs_float,
                                rhs_float,
                                "feq_tmp",
                            )
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "feq_ext")
                            .unwrap()
                            .into()
                    }
                    "ne" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::ONE,
                                lhs_float,
                                rhs_float,
                                "fne_tmp",
                            )
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "fne_ext")
                            .unwrap()
                            .into()
                    }
                    "lt" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::OLT,
                                lhs_float,
                                rhs_float,
                                "flt_tmp",
                            )
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "flt_ext")
                            .unwrap()
                            .into()
                    }
                    "le" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::OLE,
                                lhs_float,
                                rhs_float,
                                "fle_tmp",
                            )
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "fle_ext")
                            .unwrap()
                            .into()
                    }
                    "gt" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::OGT,
                                lhs_float,
                                rhs_float,
                                "fgt_tmp",
                            )
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "fgt_ext")
                            .unwrap()
                            .into()
                    }
                    "ge" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::OGE,
                                lhs_float,
                                rhs_float,
                                "fge_tmp",
                            )
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "fge_ext")
                            .unwrap()
                            .into()
                    }
                    _ => {
                        debug_assert!(false, "Unsupported float binary op: {}", op);
                        self.builder
                            .build_float_add(lhs_float, rhs_float, "fallback_add")
                            .unwrap()
                            .into()
                    }
                }
            } else {
                // Handle mixed int/float arithmetic - convert int to float
                let lhs_float = if lhs_val.is_int_value() {
                    let int_val = lhs_val.into_int_value();
                    self.builder
                        .build_signed_int_to_float(
                            int_val,
                            self.context.f64_type(),
                            "cast_lhs_i_to_f",
                        )
                        .unwrap()
                } else {
                    lhs_val.into_float_value()
                };

                let rhs_float = if rhs_val.is_int_value() {
                    let int_val = rhs_val.into_int_value();
                    self.builder
                        .build_signed_int_to_float(
                            int_val,
                            self.context.f64_type(),
                            "cast_rhs_i_to_f",
                        )
                        .unwrap()
                } else {
                    rhs_val.into_float_value()
                };

                match op_name {
                    "add" => self
                        .builder
                        .build_float_add(lhs_float, rhs_float, "fadd_tmp")
                        .unwrap()
                        .into(),
                    "sub" => self
                        .builder
                        .build_float_sub(lhs_float, rhs_float, "fsub_tmp")
                        .unwrap()
                        .into(),
                    "mul" => self
                        .builder
                        .build_float_mul(lhs_float, rhs_float, "fmul_tmp")
                        .unwrap()
                        .into(),
                    "div" => self
                        .builder
                        .build_float_div(lhs_float, rhs_float, "fdiv_tmp")
                        .unwrap()
                        .into(),
                    "mod" => {
                        // Modulo doesn't exist for floats in LLVM, so convert both to int
                        let lhs_int = self
                            .builder
                            .build_float_to_signed_int(
                                lhs_float,
                                self.context.i32_type(),
                                "fmod_lhs_to_i",
                            )
                            .unwrap();
                        let rhs_int = self
                            .builder
                            .build_float_to_signed_int(
                                rhs_float,
                                self.context.i32_type(),
                                "fmod_rhs_to_i",
                            )
                            .unwrap();
                        self.builder
                            .build_int_signed_rem(lhs_int, rhs_int, "fmod_tmp")
                            .unwrap()
                            .into()
                    }
                    "eq" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::OEQ,
                                lhs_float,
                                rhs_float,
                                "feq_tmp",
                            )
                            .unwrap();
                        // Extend i1 to i32 for proper storage and loading
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "feq_ext")
                            .unwrap()
                            .into()
                    }
                    "ne" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::ONE,
                                lhs_float,
                                rhs_float,
                                "fne_tmp",
                            )
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "fne_ext")
                            .unwrap()
                            .into()
                    }
                    "lt" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::OLT,
                                lhs_float,
                                rhs_float,
                                "flt_tmp",
                            )
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "flt_ext")
                            .unwrap()
                            .into()
                    }
                    "le" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::OLE,
                                lhs_float,
                                rhs_float,
                                "fle_tmp",
                            )
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "fle_ext")
                            .unwrap()
                            .into()
                    }
                    "gt" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::OGT,
                                lhs_float,
                                rhs_float,
                                "fgt_tmp",
                            )
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "fgt_ext")
                            .unwrap()
                            .into()
                    }
                    "ge" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::OGE,
                                lhs_float,
                                rhs_float,
                                "fge_tmp",
                            )
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "fge_ext")
                            .unwrap()
                            .into()
                    }
                    _ => {
                        debug_assert!(false, "Unsupported mixed float binary op: {}", op);
                        self.builder
                            .build_float_add(lhs_float, rhs_float, "fallback_add")
                            .unwrap()
                            .into()
                    }
                }
            }
        } else {
            if lhs_val.is_int_value() && rhs_val.is_int_value() {
                let lhs_int = lhs_val.into_int_value();
                let rhs_int = rhs_val.into_int_value();
                match op_name {
                    "add" => self
                        .builder
                        .build_int_add(lhs_int, rhs_int, "add_tmp")
                        .unwrap()
                        .into(),
                    "sub" => self
                        .builder
                        .build_int_sub(lhs_int, rhs_int, "sub_tmp")
                        .unwrap()
                        .into(),
                    "mul" => self
                        .builder
                        .build_int_mul(lhs_int, rhs_int, "mul_tmp")
                        .unwrap()
                        .into(),
                    "div" => self
                        .builder
                        .build_int_signed_div(lhs_int, rhs_int, "div_tmp")
                        .unwrap()
                        .into(),
                    "mod" => self
                        .builder
                        .build_int_signed_rem(lhs_int, rhs_int, "mod_tmp")
                        .unwrap()
                        .into(),
                    "eq" => {
                        let cmp_result = self
                            .builder
                            .build_int_compare(IntPredicate::EQ, lhs_int, rhs_int, "eq_tmp")
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "eq_ext")
                            .unwrap()
                            .into()
                    }
                    "ne" => {
                        let cmp_result = self
                            .builder
                            .build_int_compare(IntPredicate::NE, lhs_int, rhs_int, "ne_tmp")
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "ne_ext")
                            .unwrap()
                            .into()
                    }
                    "lt" => {
                        let cmp_result = self
                            .builder
                            .build_int_compare(IntPredicate::SLT, lhs_int, rhs_int, "lt_tmp")
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "lt_ext")
                            .unwrap()
                            .into()
                    }
                    "le" => {
                        let cmp_result = self
                            .builder
                            .build_int_compare(IntPredicate::SLE, lhs_int, rhs_int, "le_tmp")
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "le_ext")
                            .unwrap()
                            .into()
                    }
                    "gt" => {
                        let cmp_result = self
                            .builder
                            .build_int_compare(IntPredicate::SGT, lhs_int, rhs_int, "gt_tmp")
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "gt_ext")
                            .unwrap()
                            .into()
                    }
                    "ge" => {
                        let cmp_result = self
                            .builder
                            .build_int_compare(IntPredicate::SGE, lhs_int, rhs_int, "ge_tmp")
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "ge_ext")
                            .unwrap()
                            .into()
                    }
                    "and" => {
                        // Normalize operands to i32 if they're i1 (bool)
                        let lhs_normalized = if lhs_int.get_type().get_bit_width() == 1 {
                            self.builder
                                .build_int_z_extend(lhs_int, self.context.i32_type(), "and_lhs_ext")
                                .unwrap()
                        } else {
                            lhs_int
                        };
                        let rhs_normalized = if rhs_int.get_type().get_bit_width() == 1 {
                            self.builder
                                .build_int_z_extend(rhs_int, self.context.i32_type(), "and_rhs_ext")
                                .unwrap()
                        } else {
                            rhs_int
                        };
                        self.builder
                            .build_and(lhs_normalized, rhs_normalized, "and_tmp")
                            .unwrap()
                            .into()
                    }
                    "or" => {
                        // Normalize operands to i32 if they're i1 (bool)
                        let lhs_normalized = if lhs_int.get_type().get_bit_width() == 1 {
                            self.builder
                                .build_int_z_extend(lhs_int, self.context.i32_type(), "or_lhs_ext")
                                .unwrap()
                        } else {
                            lhs_int
                        };
                        let rhs_normalized = if rhs_int.get_type().get_bit_width() == 1 {
                            self.builder
                                .build_int_z_extend(rhs_int, self.context.i32_type(), "or_rhs_ext")
                                .unwrap()
                        } else {
                            rhs_int
                        };
                        self.builder
                            .build_or(lhs_normalized, rhs_normalized, "or_tmp")
                            .unwrap()
                            .into()
                    }
                    _ => {
                        debug_assert!(false, "Unsupported int binary op: {}", op);
                        self.builder
                            .build_int_add(lhs_int, rhs_int, "fallback_add")
                            .unwrap()
                            .into()
                    }
                }
            } else if (lhs_val.is_int_value() || lhs_val.is_float_value())
                && (rhs_val.is_int_value() || rhs_val.is_float_value())
            {
                // Handle mixed int/float arithmetic - convert to float and return as int
                let lhs_float = if lhs_val.is_int_value() {
                    let int_val = lhs_val.into_int_value();
                    self.builder
                        .build_signed_int_to_float(
                            int_val,
                            self.context.f64_type(),
                            "cast_lhs_i_to_f",
                        )
                        .unwrap()
                } else {
                    lhs_val.into_float_value()
                };

                let rhs_float = if rhs_val.is_int_value() {
                    let int_val = rhs_val.into_int_value();
                    self.builder
                        .build_signed_int_to_float(
                            int_val,
                            self.context.f64_type(),
                            "cast_rhs_i_to_f",
                        )
                        .unwrap()
                } else {
                    rhs_val.into_float_value()
                };

                match op_name {
                    "add" => self
                        .builder
                        .build_float_add(lhs_float, rhs_float, "fadd_tmp")
                        .unwrap()
                        .into(),
                    "sub" => self
                        .builder
                        .build_float_sub(lhs_float, rhs_float, "fsub_tmp")
                        .unwrap()
                        .into(),
                    "mul" => self
                        .builder
                        .build_float_mul(lhs_float, rhs_float, "fmul_tmp")
                        .unwrap()
                        .into(),
                    "div" => self
                        .builder
                        .build_float_div(lhs_float, rhs_float, "fdiv_tmp")
                        .unwrap()
                        .into(),
                    "eq" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::OEQ,
                                lhs_float,
                                rhs_float,
                                "feq_tmp",
                            )
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "feq_ext")
                            .unwrap()
                            .into()
                    }
                    "ne" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::ONE,
                                lhs_float,
                                rhs_float,
                                "fne_tmp",
                            )
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "fne_ext")
                            .unwrap()
                            .into()
                    }
                    "lt" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::OLT,
                                lhs_float,
                                rhs_float,
                                "flt_tmp",
                            )
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "flt_ext")
                            .unwrap()
                            .into()
                    }
                    "le" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::OLE,
                                lhs_float,
                                rhs_float,
                                "fle_tmp",
                            )
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "fle_ext")
                            .unwrap()
                            .into()
                    }
                    "gt" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::OGT,
                                lhs_float,
                                rhs_float,
                                "fgt_tmp",
                            )
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "fgt_ext")
                            .unwrap()
                            .into()
                    }
                    "ge" => {
                        let cmp_result = self
                            .builder
                            .build_float_compare(
                                FloatPredicate::OGE,
                                lhs_float,
                                rhs_float,
                                "fge_tmp",
                            )
                            .unwrap();
                        self.builder
                            .build_int_z_extend(cmp_result, self.context.i32_type(), "fge_ext")
                            .unwrap()
                            .into()
                    }
                    _ => {
                        debug_assert!(false, "Unsupported mixed int/float binary op: {}", op);
                        self.builder
                            .build_float_add(lhs_float, rhs_float, "fallback_add")
                            .unwrap()
                            .into()
                    }
                }
            } else {
                debug_assert!(
                    false,
                    "Int arithmetic expects both operands to be int or float values, got {:?} and {:?}",
                    lhs_val, rhs_val
                );
                self.context.i32_type().const_int(0, false).into()
            }
        };

        self.temp_values.insert(dst.to_string(), res.into());
        if let Some(sym) = self.symbols.get(dst) {
            self.builder.build_store(sym.ptr, res).unwrap();
        }

        // Track the type of the result for later printing/formatting
        if matches!(
            op_name,
            "eq" | "ne" | "lt" | "le" | "gt" | "ge" | "and" | "or"
        ) {
            self.variable_types
                .insert(dst.to_string(), "Bool".to_string());
        } else if op_type == "float" {
            self.variable_types
                .insert(dst.to_string(), "Float".to_string());
        } else {
            self.variable_types
                .insert(dst.to_string(), "Int".to_string());
        }

        Some(res.into())
    }

    /// Generate code for increment/decrement statements (i++, i--)
    /// Converts to equivalent binary operations: i = i + 1 or i = i - 1
    pub fn generate_increment_decrement(&mut self, variable: &str, op: &str) {
        // Resolve the variable value
        let var_val = self.resolve_value(variable);

        // Determine if the variable is int or float and perform the operation
        let result: BasicValueEnum<'ctx> = if var_val.is_float_value() {
            let var_float = var_val.into_float_value();
            let one_float = self.context.f64_type().const_float(1.0);
            match op {
                "++" => self
                    .builder
                    .build_float_add(var_float, one_float, "inc_float")
                    .unwrap()
                    .into(),
                "--" => self
                    .builder
                    .build_float_sub(var_float, one_float, "dec_float")
                    .unwrap()
                    .into(),
                _ => var_val, // Should not happen due to analyzer validation
            }
        } else {
            let var_int = var_val.into_int_value();
            let one_int = self.context.i32_type().const_int(1, false);
            match op {
                "++" => self
                    .builder
                    .build_int_add(var_int, one_int, "inc_int")
                    .unwrap()
                    .into(),
                "--" => self
                    .builder
                    .build_int_sub(var_int, one_int, "dec_int")
                    .unwrap()
                    .into(),
                _ => var_val, // Should not happen due to analyzer validation
            }
        };

        // Store the result back to the variable
        if let Some(sym) = self.symbols.get(variable) {
            self.builder.build_store(sym.ptr, result).unwrap();
        }

        // Update temp_values
        self.temp_values.insert(variable.to_string(), result);
    }
}
