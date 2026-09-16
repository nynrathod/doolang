//! Local variable and temporary value management.

use doo_core::doo_debug;
use doo_core::types::TypeId;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValueEnum, PointerValue};

use super::CodegenContext;

use rustc_hash::FxHashMap;

/// Maps MIR local names to LLVM alloca pointers with their types.
///
/// Each local variable gets an alloca at the function entry block.
/// Stores both the pointer and its LLVM type for later loads and stores.
pub struct LocalMap<'ctx> {
    allocas: FxHashMap<String, (PointerValue<'ctx>, BasicTypeEnum<'ctx>)>,
}

impl<'ctx> LocalMap<'ctx> {
    /// Create an empty local map.
    pub fn new() -> Self {
        Self {
            allocas: FxHashMap::default(),
        }
    }

    /// Insert a local variable's alloca pointer and type.
    pub fn insert(&mut self, name: String, ptr: PointerValue<'ctx>, ty: BasicTypeEnum<'ctx>) {
        self.allocas.insert(name, (ptr, ty));
    }

    /// Get the alloca pointer for a local variable.
    pub fn get(&self, name: &str) -> Option<PointerValue<'ctx>> {
        self.allocas.get(name).map(|(ptr, _)| *ptr)
    }

    /// Get both the alloca pointer and its LLVM type.
    pub fn get_with_type(&self, name: &str) -> Option<(PointerValue<'ctx>, BasicTypeEnum<'ctx>)> {
        self.allocas.get(name).copied()
    }

    /// Remove a local variable from the map.
    pub fn remove(&mut self, name: &str) {
        self.allocas.remove(name);
    }

    /// Clear all locals (e.g., between functions).
    pub fn clear(&mut self) {
        self.allocas.clear();
    }

    /// Check if a local variable exists.
    pub fn contains(&self, name: &str) -> bool {
        self.allocas.contains_key(name)
    }

    /// Iterate over all local variables.
    pub fn iter(
        &self,
    ) -> impl Iterator<Item = (&String, &(PointerValue<'ctx>, BasicTypeEnum<'ctx>))> {
        self.allocas.iter()
    }
}

impl<'ctx> Default for LocalMap<'ctx> {
    fn default() -> Self {
        Self::new()
    }
}

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
    ///
    /// When value type doesn't match alloca type:
    /// 1. FIRST: Recreate alloca with correct type (preserves pointer provenance)
    /// 2. THEN: Try int→ptr or ptr→int as fallback
    /// 3. LAST: Store as temp
    pub fn set_local(&mut self, name: String, value: BasicValueEnum<'ctx>) {
        if let Some((ptr, alloca_ty)) = self.locals.get(&name) {
            let value_type = value.get_type();
            let types_match = *alloca_ty == value_type;
            if types_match {
                let _ = self.builder.build_store(*ptr, value);
                self.temps.remove(&name);
            } else {
                let ptr = *ptr;
                let alloca_ty = *alloca_ty;

                // ============================================================
                // When value is a pointer but alloca is int type,
                // recreate the alloca with the correct pointer type FIRST.
                // This prevents ptr→i64 conversion that breaks ALL array/map/
                // string operations (ArrayGet, ArraySet, MethodCall, Drop, etc.)
                //
                // ============================================================
                if value.is_pointer_value() && alloca_ty.is_int_type() {
                    // Try to recreate alloca with correct pointer type
                    if let Some(current_bb) = self.builder.get_insert_block() {
                        if let Some(func) = current_bb.get_parent() {
                            if let Some(entry_bb) = func.get_first_basic_block() {
                                let insert_point = entry_bb.get_first_instruction();
                                if let Some(first_instr) = insert_point {
                                    let mut last_alloca = None;
                                    let mut instr = Some(first_instr);
                                    while let Some(i) = instr {
                                        if i.get_opcode()
                                            == inkwell::values::InstructionOpcode::Alloca
                                        {
                                            last_alloca = Some(i);
                                        } else {
                                            break;
                                        }
                                        instr = i.get_next_instruction();
                                    }
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
                                if let Ok(new_alloca) = self.builder.build_alloca(value_type, &name)
                                {
                                    self.builder.position_at_end(current_bb);
                                    let _ = self.builder.build_store(new_alloca, value);
                                    self.locals.insert(name.clone(), (new_alloca, value_type));
                                    self.temps.remove(&name);
                                    return;
                                }
                                self.builder.position_at_end(current_bb);
                            }
                        }
                    }
                    // If alloca recreation failed, fall through to ptr→int below
                }

                // ============================================================
                // Fallback conversions (only reached if alloca recreation failed)
                // ============================================================

                // ptr -> int conversion (last resort for pointer in int alloca)
                if alloca_ty.is_int_type() && value.is_pointer_value() {
                    if let Ok(converted) = self.builder.build_ptr_to_int(
                        value.into_pointer_value(),
                        alloca_ty.into_int_type(),
                        &format!("{}_ptrtoint", name),
                    ) {
                        let _ = self.builder.build_store(ptr, converted);
                        self.temps.remove(&name);
                        return;
                    }
                }

                // int -> ptr conversion (for tuple-destructured values stored as i64)
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
                        return;
                    }
                }

                // Last resort: recreate alloca with correct type
                if let Some(current_bb) = self.builder.get_insert_block() {
                    if let Some(func) = current_bb.get_parent() {
                        if let Some(entry_bb) = func.get_first_basic_block() {
                            let insert_point = entry_bb.get_first_instruction();
                            if let Some(first_instr) = insert_point {
                                let mut last_alloca = None;
                                let mut instr = Some(first_instr);
                                while let Some(i) = instr {
                                    if i.get_opcode() == inkwell::values::InstructionOpcode::Alloca
                                    {
                                        last_alloca = Some(i);
                                    } else {
                                        break;
                                    }
                                    instr = i.get_next_instruction();
                                }
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
                            if let Ok(new_alloca) = self.builder.build_alloca(value_type, &name) {
                                self.builder.position_at_end(current_bb);
                                let _ = self.builder.build_store(new_alloca, value);
                                self.locals.insert(name.clone(), (new_alloca, value_type));
                                self.temps.remove(&name);
                                return;
                            }
                            self.builder.position_at_end(current_bb);
                        }
                    }
                }

                // Final fallback: store as temp
                self.temps.insert(name, value);
            }
        } else {
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
        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {}
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
            if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {}
            return Some(*v);
        }
        // Check locals - return loaded value
        if let Some((ptr, ty)) = self.locals.get(name) {
            if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {}
            let result = self.builder.build_load(*ty, *ptr, name);
            if result.is_err() {
            } else if std::env::var(doo_core::constants::env_vars::DOO_DEBUG).is_ok() {
            }
            return result.ok();
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
