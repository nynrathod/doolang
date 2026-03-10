//! Local variable and temporary value management.

use doo_core::doo_debug;
use doo_core::types::TypeId;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValueEnum, PointerValue};

use super::CodegenContext;

impl<'ctx> CodegenContext<'ctx> {
    // ========================================================================
    // Local Variable Management
    // ========================================================================

    /// Create an alloca (local variable) in the function's entry block.
    pub fn create_local(&mut self, name: &str, ty: BasicTypeEnum<'ctx>) -> PointerValue<'ctx> {
        let alloca = self
            .alloca_in_entry_block(ty, name)
            .expect("ICE: failed to build alloca for local variable");
        self.locals.insert(name.to_string(), (alloca, ty));
        alloca
    }

    /// Replace a local variable's alloca pointer with an external pointer.
    /// Used by reference capture: the spawn function uses the OUTER function's
    /// alloca directly, so writes propagate back to the parent scope.
    pub fn replace_local_ptr(
        &mut self,
        name: &str,
        ptr: PointerValue<'ctx>,
        ty: BasicTypeEnum<'ctx>,
    ) {
        self.locals.insert(name.to_string(), (ptr, ty));
    }

    /// Get a local variable pointer.
    pub fn get_local(&self, name: &str) -> Option<PointerValue<'ctx>> {
        self.locals.get(name).map(|(ptr, _)| *ptr)
    }

    /// Load a value from a local variable's alloca, bypassing the temp cache.
    /// Returns None if no alloca exists for this name.
    /// This is critical for cross-block values (match/if results) where the temp
    /// in the HashMap was defined in a non-dominating block.
    pub fn load_from_local(&self, name: &str) -> Option<BasicValueEnum<'ctx>> {
        if let Some((ptr, ty)) = self.locals.get(name) {
            self.builder.build_load(*ty, *ptr, name).ok()
        } else {
            None
        }
    }

    /// Get the LLVM type of a local variable.
    pub fn get_local_type(&self, name: &str) -> Option<BasicTypeEnum<'ctx>> {
        self.locals.get(name).map(|(_, ty)| *ty)
    }

    /// Store a value to a local variable.
    /// If an alloca exists (from create_local) with matching type, stores to it.
    /// If there's a type mismatch (e.g., variable shadowing with different type),
    /// stores as temp so get_value finds it first.
    /// Otherwise, stores as a temp value.
    pub fn set_local(&mut self, name: String, value: BasicValueEnum<'ctx>) {
        // If we have an alloca for this variable, check type compatibility
        if let Some((ptr, alloca_ty)) = self.locals.get(&name) {
            // Check if value type matches alloca type
            // For type mismatches (e.g., same variable name but different type due to shadowing),
            // store as temp instead to avoid LLVM type errors
            let value_type = value.get_type();
            let types_match = *alloca_ty == value_type;
            if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                doo_debug!(
                    "CODEGEN",
                    "set_local '{}': alloca_ty={:?}, value_ty={:?}, match={}",
                    name,
                    alloca_ty,
                    value_type,
                    types_match
                );
            }
            if types_match {
                // Types match - store to alloca
                let _ = self.builder.build_store(*ptr, value);
                // Clear any stale temp entry - the alloca is the source of truth now
                // This prevents get_value from returning an old SSA value from a different block
                self.temps.remove(&name);
            } else {
                // Type mismatch - try implicit conversion before falling back to temp
                let ptr = *ptr;
                let alloca_ty = *alloca_ty;

                // ptr -> int conversion: reverses the inttoptr done by UnwrapOk
                // This handles cases like `let total: Int = db.rawWithParams(...)?;`
                // where the FFI result was originally an i64, converted to ptr by UnwrapOk,
                // and now needs to be stored back as an integer.
                if alloca_ty.is_int_type() && value.is_pointer_value() {
                    if let Ok(converted) = self.builder.build_ptr_to_int(
                        value.into_pointer_value(),
                        alloca_ty.into_int_type(),
                        &format!("{}_ptrtoint", name),
                    ) {
                        let _ = self.builder.build_store(ptr, converted);
                        self.temps.remove(&name);
                        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                            doo_debug!(
                                "CODEGEN",
                                "set_local '{}': converted ptr->int via ptrtoint",
                                name
                            );
                        }
                        return;
                    }
                }

                // int -> ptr conversion: handles tuple-destructured values that are
                // stored as i64 (because TupleCreate uses uniform i64 layout) but the
                // variable was declared as a string/pointer type.
                // Example: `let resultJson, err = Process.run(...)` where resultJson
                // is Str (ptr alloca) but TupleGet extracts it as i64.
                // SAFETY: Validate the integer is a plausible user-space address.
                // Zero and negative values (like -1/0xFFFFFFFFFFFFFFFF) produce
                // invalid pointers that crash on dereference.
                if alloca_ty.is_pointer_type() && value.is_int_value() {
                    let int_val = value.into_int_value();
                    let ptr_type = alloca_ty.into_pointer_type();
                    let is_positive = self.builder.build_int_compare(
                        inkwell::IntPredicate::SGT,
                        int_val,
                        int_val.get_type().const_zero(),
                        "is_valid_addr",
                    );
                    let safe_ptr = if let Ok(is_valid) = is_positive {
                        let null_ptr = ptr_type.const_null();
                        if let Ok(as_ptr) = self.builder.build_int_to_ptr(
                            int_val,
                            ptr_type,
                            &format!("{}_inttoptr", name),
                        ) {
                            self.builder
                                .build_select(is_valid, as_ptr, null_ptr, &format!("{}_safe", name))
                                .ok()
                                .map(|v| v.into_pointer_value())
                        } else {
                            None
                        }
                    } else {
                        self.builder
                            .build_int_to_ptr(int_val, ptr_type, &format!("{}_inttoptr", name))
                            .ok()
                    };
                    if let Some(safe) = safe_ptr {
                        let converted_val: BasicValueEnum = safe.into();
                        let _ = self.builder.build_store(ptr, converted_val);
                        self.temps.remove(&name);
                        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                            doo_debug!(
                                "CODEGEN",
                                "set_local '{}': converted int->ptr via safe inttoptr",
                                name
                            );
                        }
                        return;
                    }
                }

                // No conversion possible - recreate alloca with correct type in entry block
                // This handles match/if expression results where the alloca was created
                // with a generic type (ptr) but the actual values are concrete (i64, f64, etc.)
                // CRITICAL: Without this, cross-block values stored as temps cause
                // "Instruction does not dominate all uses" LLVM verification errors
                if let Some(current_bb) = self.builder.get_insert_block() {
                    if let Some(func) = current_bb.get_parent() {
                        if let Some(entry_bb) = func.get_first_basic_block() {
                            // Position at the end of allocas in entry block
                            // (before any non-alloca instructions)
                            let insert_point = entry_bb.get_first_instruction();
                            if let Some(first_instr) = insert_point {
                                // Find the last alloca instruction
                                let mut last_alloca = None;
                                let mut instr = Some(first_instr);
                                while let Some(i) = instr {
                                    if i.get_opcode() == inkwell::values::InstructionOpcode::Alloca
                                    {
                                        last_alloca = Some(i);
                                    } else {
                                        break; // Allocas are always at the start
                                    }
                                    instr = i.get_next_instruction();
                                }
                                // Position after last alloca (or at start if no allocas)
                                if let Some(la) = last_alloca {
                                    if let Some(next) = la.get_next_instruction() {
                                        self.builder.position_before(&next);
                                    } else {
                                        self.builder.position_at_end(entry_bb);
                                    }
                                } else {
                                    self.builder.position_before(&first_instr);
                                }
                            } else {
                                self.builder.position_at_end(entry_bb);
                            }
                            // Create new alloca with the correct type
                            if let Ok(new_alloca) = self.builder.build_alloca(value_type, &name) {
                                // Restore position to current block
                                self.builder.position_at_end(current_bb);
                                let _ = self.builder.build_store(new_alloca, value);
                                self.locals.insert(name.clone(), (new_alloca, value_type));
                                self.temps.remove(&name);
                                if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                                    doo_debug!(
                                        "CODEGEN",
                                        "set_local '{}': recreated alloca with correct type {:?}",
                                        name,
                                        value_type
                                    );
                                }
                                return;
                            }
                            // Restore position if alloca creation failed
                            self.builder.position_at_end(current_bb);
                        }
                    }
                }

                // Last resort - store as temp (shadows the local for this scope)
                // get_value checks temps first, so this will be found before the alloca
                if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                    doo_debug!(
                        "CODEGEN",
                        "set_local type mismatch for '{}': using temp instead",
                        name
                    );
                }
                self.temps.insert(name, value);
            }
        } else {
            // Fallback to temp storage (for temporaries without allocas)
            self.temps.insert(name, value);
        }
    }

    /// Register a borrow: track that temp_name is a borrow of local_name.
    /// Used for mutating operations to store back to the original local.
    pub fn set_borrow_origin(&mut self, temp_name: &str, local_name: &str) {
        self.borrow_origins
            .insert(temp_name.to_string(), local_name.to_string());
    }

    /// Get the original local name for a borrowed temp.
    /// Returns None if the name is not a borrowed temp.
    pub fn get_borrow_origin(&self, name: &str) -> Option<&str> {
        self.borrow_origins.get(name).map(|s| s.as_str())
    }

    /// Get the alloca pointer for a name, checking both locals and borrow origins.
    /// For borrowed temps, returns the original local's alloca.
    pub fn get_local_or_borrow_origin(&self, name: &str) -> Option<PointerValue<'ctx>> {
        // First try direct local
        if let Some(ptr) = self.get_local(name) {
            return Some(ptr);
        }
        // Then try borrow origin
        if let Some(origin) = self.get_borrow_origin(name) {
            return self.get_local(origin);
        }
        None
    }

    /// Clear locals (for new function).
    pub fn clear_locals(&mut self) {
        self.locals.clear();
        self.temps.clear();
        self.borrow_origins.clear();
        self.array_element_types.clear();
        self.array_element_temps.clear();
        self.variable_types.clear();
        self.temp_struct_types.clear();
    }

    // ========================================================================
    // Temporary Management
    // ========================================================================

    /// Store a temporary value.
    pub fn set_temp(&mut self, name: &str, value: BasicValueEnum<'ctx>) {
        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
            doo_debug!("CODEGEN", "set_temp: {} = {:?}", name, value);
        }
        self.temps.insert(name.to_string(), value);
    }

    /// Clear a temporary value (remove from temps map).
    /// Used when storing to an alloca to ensure get_value loads from the alloca.
    pub fn clear_temp(&mut self, name: &str) {
        self.temps.remove(name);
    }

    /// Get a temporary value.
    pub fn get_temp(&self, name: &str) -> Option<BasicValueEnum<'ctx>> {
        self.temps.get(name).copied()
    }

    /// Get value by name (local or temp).
    pub fn get_value(&self, name: &str) -> Option<BasicValueEnum<'ctx>> {
        // Check temps first
        if let Some(v) = self.temps.get(name) {
            if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                doo_debug!("CODEGEN", "get_value({}) found in temps", name);
            }
            return Some(*v);
        }
        // Check locals - return loaded value
        if let Some((ptr, ty)) = self.locals.get(name) {
            if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                doo_debug!(
                    "CODEGEN",
                    "get_value({}) loading from local, ty={:?}",
                    name,
                    ty
                );
            }
            let result = self.builder.build_load(*ty, *ptr, name);
            if result.is_err() {
                if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                    doo_debug!(
                        "CODEGEN",
                        "ERROR: build_load failed for {}: {:?}",
                        name,
                        result
                    );
                }
            } else if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
                doo_debug!("CODEGEN", "get_value({}) loaded successfully", name);
            }
            return result.ok();
        }
        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
            doo_debug!(
                "CODEGEN",
                "WARNING: Variable {} not found in temps or locals",
                name
            );
        }
        None
    }

    // ========================================================================
    // Variable Type Tracking (for Clone/Drop)
    // ========================================================================

    /// Register the Doo TypeId for a variable.
    ///
    /// Called when creating locals to track their type for deep clone/drop.
    pub fn set_variable_type(&mut self, var_name: &str, type_id: TypeId) {
        self.variable_types.insert(var_name.to_string(), type_id);
    }

    /// Get the Doo TypeId for a variable.
    ///
    /// Used in Clone/Drop to determine the cloning/cleanup strategy.
    pub fn get_variable_type(&self, var_name: &str) -> Option<TypeId> {
        self.variable_types.get(var_name).copied()
    }
}
