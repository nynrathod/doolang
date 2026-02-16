//! Struct field metadata and decorator helpers.

use doo_core::types::{TypeId, TypeKind};

use super::CodegenContext;

impl<'ctx> CodegenContext<'ctx> {
    // ========================================================================
    // Struct Metadata Management (for FieldGet/FieldSet)
    // ========================================================================

    /// Register a struct type with its field names.
    ///
    /// Called during StructCreate to track the struct's field layout.
    pub fn register_struct_metadata(&mut self, struct_name: &str, field_names: Vec<String>) {
        self.struct_metadata
            .insert(struct_name.to_string(), field_names);
    }

    /// Get field names for a struct type.
    pub fn get_struct_fields(&self, struct_name: &str) -> Option<&Vec<String>> {
        self.struct_metadata.get(struct_name)
    }

    /// Get field index by name for a struct type.
    ///
    /// Returns the index of the field in the struct, or None if not found.
    /// First checks the struct_metadata cache, then falls back to the type registry
    /// for imported/cross-module types.
    pub fn get_field_index(&self, struct_name: &str, field_name: &str) -> Option<u32> {
        // First try the cached struct_metadata
        if let Some(idx) = self
            .struct_metadata
            .get(struct_name)
            .and_then(|fields| fields.iter().position(|f| f == field_name))
            .map(|idx| idx as u32)
        {
            return Some(idx);
        }

        // Fall back to type registry - search all types for struct with matching name
        // This handles TypeRef cases where lookup returns the TypeRef, not the actual struct
        for type_id in self.type_registry.all_type_ids() {
            if let Some(info) = self.type_registry.get(type_id) {
                if let TypeKind::Struct { name, fields } = &info.kind {
                    if name == struct_name {
                        return fields
                            .iter()
                            .position(|(n, _, _)| n == field_name)
                            .map(|idx| idx as u32);
                    }
                }
            }
        }
        None
    }

    /// Get the TypeId for a struct field from the type registry.
    pub fn get_struct_field_type(&self, struct_name: &str, field_name: &str) -> Option<TypeId> {
        // Search all types for struct with matching name
        // This handles TypeRef cases where lookup returns the TypeRef, not the actual struct
        for type_id in self.type_registry.all_type_ids() {
            if let Some(info) = self.type_registry.get(type_id) {
                if let TypeKind::Struct { name, fields } = &info.kind {
                    if name == struct_name {
                        return fields
                            .iter()
                            .find(|(n, _, _)| n == field_name)
                            .map(|(_, type_id, _)| *type_id);
                    }
                }
            }
        }
        None
    }

    /// Get the struct name from a TypeId if it's a struct type.
    /// Returns None if the TypeId doesn't refer to a struct.
    pub fn get_struct_name_from_type_id(&self, type_id: TypeId) -> Option<String> {
        let type_info = self.type_registry.get(type_id)?;
        if let TypeKind::Struct { name, .. } = &type_info.kind {
            Some(name.clone())
        } else {
            None
        }
    }

    /// Get all field TypeIds for a struct from the type registry.
    /// Returns field types in declaration order.
    pub fn get_struct_field_types(&self, struct_name: &str) -> Option<Vec<TypeId>> {
        // Search all types for struct with matching name
        // This handles TypeRef cases where lookup returns the TypeRef, not the actual struct
        for type_id in self.type_registry.all_type_ids() {
            if let Some(info) = self.type_registry.get(type_id) {
                if let TypeKind::Struct { name, fields } = &info.kind {
                    if name == struct_name {
                        return Some(fields.iter().map(|(_, type_id, _)| *type_id).collect());
                    }
                }
            }
        }
        None
    }

    /// Track that a temp/local variable holds a struct instance of a specific type.
    ///
    /// Called after StructCreate to associate the variable with its struct type.
    pub fn set_temp_struct_type(&mut self, var_name: &str, struct_name: &str) {
        self.temp_struct_types
            .insert(var_name.to_string(), struct_name.to_string());
    }

    /// Get the struct type name for a temp/local variable.
    ///
    /// Used in FieldGet/FieldSet to determine the correct struct type.
    pub fn get_temp_struct_type(&self, var_name: &str) -> Option<&String> {
        self.temp_struct_types.get(var_name)
    }
}
