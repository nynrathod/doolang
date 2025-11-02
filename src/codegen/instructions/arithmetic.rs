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
        let lhs_val = self.resolve_value(lhs);
        let rhs_val = self.resolve_value(rhs);

        // If both are pointers and operation is "add", treat as string concatenation
        if op == "add" && lhs_val.is_pointer_value() && rhs_val.is_pointer_value() {
            // Delegate to string concatenation logic
            return self.generate_instr(&crate::mir::MirInstr::StringConcat {
                name: dst.to_string(),
                left: lhs.to_string(),
                right: rhs.to_string(),
            });
        }

        // Support op:type format for int/float operations
        let parts: Vec<&str> = op.split(':').collect();
        let op_name = parts[0];
        let op_type = parts.get(1).copied().unwrap_or("int");

        // String concatenation for pointers
        if op_name == "add" && lhs_val.is_pointer_value() && rhs_val.is_pointer_value() {
            return self.generate_instr(&crate::mir::MirInstr::StringConcat {
                name: dst.to_string(),
                left: lhs.to_string(),
                right: rhs.to_string(),
            });
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
                    "eq" => self
                        .builder
                        .build_float_compare(FloatPredicate::OEQ, lhs_float, rhs_float, "feq_tmp")
                        .unwrap()
                        .into(),
                    "ne" => self
                        .builder
                        .build_float_compare(FloatPredicate::ONE, lhs_float, rhs_float, "fne_tmp")
                        .unwrap()
                        .into(),
                    "lt" => self
                        .builder
                        .build_float_compare(FloatPredicate::OLT, lhs_float, rhs_float, "flt_tmp")
                        .unwrap()
                        .into(),
                    "le" => self
                        .builder
                        .build_float_compare(FloatPredicate::OLE, lhs_float, rhs_float, "fle_tmp")
                        .unwrap()
                        .into(),
                    "gt" => self
                        .builder
                        .build_float_compare(FloatPredicate::OGT, lhs_float, rhs_float, "fgt_tmp")
                        .unwrap()
                        .into(),
                    "ge" => self
                        .builder
                        .build_float_compare(FloatPredicate::OGE, lhs_float, rhs_float, "fge_tmp")
                        .unwrap()
                        .into(),
                    _ => {
                        debug_assert!(false, "Unsupported float binary op: {}", op);
                        self.builder
                            .build_float_add(lhs_float, rhs_float, "fallback_add")
                            .unwrap()
                            .into()
                    }
                }
            } else {
                debug_assert!(
                    false,
                    "Float arithmetic expects both operands to be float values, got {:?} and {:?}",
                    lhs_val, rhs_val
                );
                self.context.f64_type().const_float(0.0).into()
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
                    "eq" => self
                        .builder
                        .build_int_compare(IntPredicate::EQ, lhs_int, rhs_int, "eq_tmp")
                        .unwrap()
                        .into(),
                    "ne" => self
                        .builder
                        .build_int_compare(IntPredicate::NE, lhs_int, rhs_int, "ne_tmp")
                        .unwrap()
                        .into(),
                    "lt" => self
                        .builder
                        .build_int_compare(IntPredicate::SLT, lhs_int, rhs_int, "lt_tmp")
                        .unwrap()
                        .into(),
                    "le" => self
                        .builder
                        .build_int_compare(IntPredicate::SLE, lhs_int, rhs_int, "le_tmp")
                        .unwrap()
                        .into(),
                    "gt" => self
                        .builder
                        .build_int_compare(IntPredicate::SGT, lhs_int, rhs_int, "gt_tmp")
                        .unwrap()
                        .into(),
                    "ge" => self
                        .builder
                        .build_int_compare(IntPredicate::SGE, lhs_int, rhs_int, "ge_tmp")
                        .unwrap()
                        .into(),
                    "and" => self
                        .builder
                        .build_and(lhs_int, rhs_int, "and_tmp")
                        .unwrap()
                        .into(),
                    "or" => self
                        .builder
                        .build_or(lhs_int, rhs_int, "or_tmp")
                        .unwrap()
                        .into(),
                    _ => {
                        debug_assert!(false, "Unsupported int binary op: {}", op);
                        self.builder
                            .build_int_add(lhs_int, rhs_int, "fallback_add")
                            .unwrap()
                            .into()
                    }
                }
            } else {
                debug_assert!(
                    false,
                    "Int arithmetic expects both operands to be int values, got {:?} and {:?}",
                    lhs_val, rhs_val
                );
                self.context.i32_type().const_int(0, false).into()
            }
        };

        self.temp_values.insert(dst.to_string(), res.into());
        if let Some(sym) = self.symbols.get(dst) {
            self.builder.build_store(sym.ptr, res).unwrap();
        }
        Some(res.into())
    }
}
