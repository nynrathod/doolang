use crate::codegen::core::{CodeGen, Symbol};
use crate::mir::mir::MirInstr;
use inkwell::types::{AsTypeRef, BasicType, BasicTypeEnum};
use inkwell::values::{AsValueRef, BasicValue, BasicValueEnum};

/// This module provides functions for generating LLVM IR for global variables, constants, arrays, maps, and string operations.
/// It handles the translation of MIR instructions into LLVM global definitions, including constant folding and compile-time string concatenation.
/// The code here is essential for setting up the global state of a program before function-level code generation begins.
/// External function from the LLVM C API needed to create constant arrays
/// of complex types (like arrays of structs, or nested arrays).
use llvm_sys::core::LLVMConstArray;
use llvm_sys::prelude::LLVMValueRef;

/// Implements global code generation logic for the CodeGen struct.
/// This includes helpers for building constant arrays, generating global variables/constants,
/// resolving constant values, and handling compile-time string/map/array operations.
impl<'ctx> CodeGen<'ctx> {
    /// Helper function using FFI (Foreign Function Interface) to manually create an
    /// LLVM constant array from a vector of values.
    /// This is essential for arrays containing complex types (structs, other arrays)
    /// where Inkwell's direct API is insufficient or requires advanced usage.
    ///
    /// # Safety
    /// This function uses raw LLVM pointers and should be used with care.
    unsafe fn build_const_array(
        elem_type: BasicTypeEnum<'ctx>,
        values: Vec<BasicValueEnum<'ctx>>,
    ) -> BasicValueEnum<'ctx> {
        // Convert Inkwell values to raw LLVM value references.
        let mut raw: Vec<LLVMValueRef> = values.iter().map(|v| v.as_value_ref()).collect();
        // Call the raw LLVM function to build the constant array.
        let arr_ref = LLVMConstArray(elem_type.as_type_ref(), raw.as_mut_ptr(), raw.len() as u32);
        // Convert the raw reference back into an Inkwell ArrayValue.
        unsafe { inkwell::values::ArrayValue::new(arr_ref) }.as_basic_value_enum()
    }

    /// Generates the LLVM IR definition for a global constant or mutable variable.
    /// This function matches on MIR instructions and creates the corresponding LLVM global objects.
    /// It supports integers, booleans, strings, arrays, maps, binary operations, assignments, and string concatenation.
    ///
    /// - For constants, it inserts them into the temp_values map.
    /// - For assignments, it registers the symbol in the symbol table.
    /// - For arrays and maps, it builds constant initializers using FFI if needed.
    /// - For string concatenation, it creates a new global string.
    pub fn generate_global(&mut self, instr: &MirInstr) {
        match instr {
            // Integer constant global (only i32 for integers)
            MirInstr::ConstInt { name, value } => {
                let val = self.context.i32_type().const_int(*value as u64, true);
                self.temp_values.insert(name.clone(), val.into());
            }
            // Boolean constant global
            MirInstr::ConstBool { name, value } => {
                // Use i32 instead of i1 for consistency with rest of codegen
                let val = self.context.i32_type().const_int(*value as u64, false);
                self.temp_values.insert(name.clone(), val.into());
            }
            // String constant global
            MirInstr::ConstString { name, value } => {
                // For global strings, just create static constant (no RC)
                // RC will be handled in functions, not globals
                if self.strings_to_concat.contains(name) {
                    self.temp_strings.insert(name.clone(), value.clone());
                } else {
                    let s = self.module.add_global(
                        self.context.i8_type().array_type(value.len() as u32 + 1),
                        None,
                        name,
                    );
                    s.set_initializer(&self.context.const_string(value.as_bytes(), true));
                    self.temp_values
                        .insert(name.clone(), s.as_pointer_value().into());
                }
            }
            // Handles constant-time binary operations (e.g., global `a = 5 + 2`).
            MirInstr::BinaryOp(op, dst, lhs, rhs) => {
                // Resolve the constant values of the operands.
                let lhs_val = self.resolve_global_value(lhs).into_int_value();
                let rhs_val = self.resolve_global_value(rhs).into_int_value();

                // Perform the constant fold (calculation) directly.
                let res = match op.as_str() {
                    "add" => lhs_val.const_add(rhs_val),
                    "sub" => lhs_val.const_sub(rhs_val),
                    "mul" => lhs_val.const_mul(rhs_val),
                    "eq" => lhs_val.const_int_compare(inkwell::IntPredicate::EQ, rhs_val),
                    "ne" => lhs_val.const_int_compare(inkwell::IntPredicate::NE, rhs_val),
                    "lt" => lhs_val.const_int_compare(inkwell::IntPredicate::SLT, rhs_val),
                    "le" => lhs_val.const_int_compare(inkwell::IntPredicate::SLE, rhs_val),
                    "gt" => lhs_val.const_int_compare(inkwell::IntPredicate::SGT, rhs_val),
                    "ge" => lhs_val.const_int_compare(inkwell::IntPredicate::SGE, rhs_val),
                    _ => {
                        debug_assert!(false, "Unsupported binary op in globals: {}. Note: div and mod are not supported for global constants.", op);
                        lhs_val // Fallback: return left operand
                    }
                };
                // Store the result as a new constant value.
                self.temp_values.insert(dst.clone(), res.into());
            }
            // Handles the final assignment of a constant/variable to its named global location.
            MirInstr::Assign {
                name,
                value,
                mutable,
            } => {
                // Special handling for constant strings to rename the temporary global
                // created by ConstString, avoiding redundant memory allocation.
                if let Some(string_ptr) = self.temp_values.get(value) {
                    if string_ptr.is_pointer_value() {
                        // Find and rename the temporary string global, set mutability, and register symbol.
                        for global in self.module.get_globals() {
                            if global.as_pointer_value() == string_ptr.into_pointer_value() {
                                if let Some(initializer) = global.get_initializer() {
                                    global.set_name(name);
                                    global.set_constant(!*mutable); // Set constant based on mutability

                                    // Register the final symbol in the symbol table.
                                    self.symbols.insert(
                                        name.clone(),
                                        Symbol {
                                            ptr: global.as_pointer_value(),
                                            ty: initializer.get_type(),
                                        },
                                    );
                                    self.temp_values.remove(value); // Clean up temp value
                                    return;
                                }
                            }
                        }
                    }
                }

                // Standard global variable/constant definition for non-string values.
                let val = self.resolve_global_value(value);
                let g = self.module.add_global(val.get_type(), None, name);
                g.set_initializer(&val); // Set the initial constant value.
                g.set_constant(!*mutable); // Set constant flag.

                // Register the final symbol in the symbol table.
                self.symbols.insert(
                    name.clone(),
                    Symbol {
                        ptr: g.as_pointer_value(),
                        ty: val.get_type(),
                    },
                );

                // Copy array metadata if the value has it
                if let Some(metadata) = self.array_metadata.get(value).cloned() {
                    self.array_metadata.insert(name.clone(), metadata);
                }

                // Copy map metadata if the value has it
                if let Some(metadata) = self.map_metadata.get(value).cloned() {
                    self.map_metadata.insert(name.clone(), metadata);
                }
            }
            // Handles string concatenation at compile time (global scope).
            MirInstr::StringConcat { name, left, right } => {
                // Resolve raw string contents from the temp_strings map.
                let left_val = self.temp_strings.get(left).expect("Left string not found");
                let right_val = self
                    .temp_strings
                    .get(right)
                    .expect("Right string not found");
                let result = format!("{}{}", left_val, right_val);

                // Store the result string for potential further concatenation.
                self.temp_strings.insert(name.clone(), result.clone());

                // Define the new concatenated global string variable.
                let str_bytes = result.as_bytes();
                let str_len = str_bytes.len() + 1;
                let array_type = self.context.i8_type().array_type(str_len as u32);

                let g = self.module.add_global(array_type, None, name);
                g.set_initializer(&self.context.const_string(str_bytes, true));

                // Store its pointer for assignment/lookup.
                self.temp_values
                    .insert(name.clone(), g.as_pointer_value().into());
            }
            // Handles constant array initialization, including nested aggregates.
            MirInstr::Array { name, elements } => {
                // Resolve the LLVM constant value for ALL elements.
                let element_values: Vec<BasicValueEnum<'ctx>> = elements
                    .iter()
                    .map(|el| self.resolve_global_value(el))
                    .collect();

                // Determine the uniform type of the elements (using the first element).
                let first_val = &element_values[0];
                let elem_type = first_val.get_type();
                let _array_type = elem_type.array_type(elements.len() as u32);

                // Determine element type name and if it contains strings
                let element_type_name = if elem_type.is_int_type() {
                    "Int"
                } else if elem_type.is_pointer_type() {
                    "Str"
                } else {
                    "Unknown"
                };
                let contains_strings = elem_type.is_pointer_type();

                // Create the constant array initializer based on element type.
                let const_array = if elem_type.is_int_type() {
                    // Use Inkwell's simple const_array for arrays of primitive integers.
                    let int_values: Vec<_> = element_values
                        .into_iter()
                        .map(|v| v.into_int_value())
                        .collect();
                    elem_type
                        .into_int_type()
                        .const_array(&int_values)
                        .as_basic_value_enum()
                } else {
                    // Use the FFI helper for complex types (structs, pointers, nested arrays).
                    unsafe { Self::build_const_array(elem_type, element_values) }
                };

                // Store the final constant array value.
                self.temp_values.insert(name.clone(), const_array);

                // Create and store metadata for the array
                let metadata = crate::codegen::ArrayMetadata {
                    length: elements.len(),
                    element_type: element_type_name.to_string(),
                    contains_strings,
                };
                self.array_metadata.insert(name.clone(), metadata);
            }
            // Handles constant map initialization, represented as an array of structs.
            MirInstr::Map {
                name,
                entries,
                key_type,
                value_type,
            } => {
                // Determine the types of the key and value from the first entry.
                let first_key = self.resolve_global_value(&entries[0].0);
                let first_val = self.resolve_global_value(&entries[0].1);
                let key_type = first_key.get_type();
                let val_type = first_val.get_type();
                // Define the structure type {KeyType, ValueType}.
                let pair_type = self.context.struct_type(&[key_type, val_type], false);

                // Determine type names for metadata
                let key_type_name = if key_type.is_int_type() {
                    "Int"
                } else if key_type.is_pointer_type() {
                    "Str"
                } else {
                    "Unknown"
                };
                let value_type_name = if val_type.is_int_type() {
                    "Int"
                } else if val_type.is_pointer_type() {
                    "Str"
                } else {
                    "Unknown"
                };

                // Build ALL struct entries using the defined pair type.
                let struct_values: Vec<BasicValueEnum<'ctx>> = entries
                    .iter()
                    .map(|(k, v)| {
                        let key_val = self.resolve_global_value(k);
                        let val_val = self.resolve_global_value(v);
                        // Create a constant struct for each key-value pair.
                        pair_type
                            .const_named_struct(&[key_val, val_val])
                            .as_basic_value_enum()
                    })
                    .collect();

                // Create the final constant array of ALL structs using the FFI helper.
                let const_array =
                    unsafe { Self::build_const_array(pair_type.into(), struct_values) };

                // Store the final constant map value.
                self.temp_values.insert(name.clone(), const_array);

                // Create and store metadata for the map
                let metadata = crate::codegen::MapMetadata {
                    length: entries.len(),
                    key_type: key_type_name.to_string(),
                    value_type: value_type_name.to_string(),
                    key_is_string: key_type.is_pointer_type(),
                    value_is_string: val_type.is_pointer_type(),
                };
                self.map_metadata.insert(name.clone(), metadata);
            }
            // Ignore other MIR instructions
            _ => {}
        }
    }

    /// Resolves a name (which can be a temp variable or a literal) to its LLVM constant value.
    /// This is used recursively for building nested constants.
    ///
    /// - Checks temp_values for previously computed constants.
    /// - Checks symbols for previously defined global variables.
    /// - Handles immediate literals (integers, booleans, strings).
    /// - Panics if the value cannot be resolved.
    pub fn resolve_global_value(&self, name: &str) -> BasicValueEnum<'ctx> {
        // Look up results of previous global instructions
        if let Some(val) = self.temp_values.get(name) {
            return *val;
        }

        // Check if it's a previously defined global variable in symbols
        if let Some(symbol) = self.symbols.get(name) {
            let global = unsafe { inkwell::values::GlobalValue::new(symbol.ptr.as_value_ref()) };
            if let Some(initializer) = global.get_initializer() {
                return initializer;
            }
        }

        // Handle immediate literals (integers using i32, booleans).
        if let Ok(val) = name.parse::<i32>() {
            return self.context.i32_type().const_int(val as u64, true).into();
        }
        if name == "true" {
            return self.context.i32_type().const_int(1, false).into();
        }
        if name == "false" {
            return self.context.i32_type().const_int(0, false).into();
        }

        // Handle immediate string literals (e.g., `"hello"`).
        if name.starts_with('\"') && name.ends_with('\"') {
            let str_content = &name[1..name.len() - 1];
            // Define an anonymous global string for the literal.
            let s = self.module.add_global(
                self.context
                    .i8_type()
                    .array_type(str_content.len() as u32 + 1),
                None,
                &format!("str_literal_{}", self.temp_values.len()),
            );
            s.set_initializer(&self.context.const_string(str_content.as_bytes(), true));
            return s.as_pointer_value().into();
        }

        // If none of the above, return a default value
        debug_assert!(false, "Unknown global variable or literal: {}", name);
        self.context.i32_type().const_int(0, false).into()
    }

    /// Finds a specific MIR instruction (e.g., a ConstString) by its destination name.
    /// This might be used to retrieve details about a globally defined value.
    ///
    /// Returns Some(&MirInstr) if found, otherwise None.
    pub fn find_instr_by_name(&self, name: &str) -> Option<&MirInstr> {
        self.globals.iter().find(|instr| match instr {
            MirInstr::ConstString { name: n, .. } => n == name,
            _ => false,
        })
    }
}
