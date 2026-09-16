//! Struct field metadata and decorator helpers.

use doo_core::types::{TypeId, TypeKind};

use super::CodegenContext;
use rustc_hash::FxHashMap;

/// Generic metadata cache for codegen.
///
/// Stores debug info, function metadata, and struct field layout info.
/// Contains NO framework-specific metadata (Audit §3.6) — only language-level
/// metadata needed by the codegen pipeline.
pub struct MetadataCache {
    /// Struct field names in declaration order (for FieldGet/FieldSet dispatch).
    struct_fields: FxHashMap<String, Vec<String>>,
    /// P06 field reordering map: logical index -> physical index.
    struct_field_remap: FxHashMap<String, Vec<usize>>,
    /// Temp variables that hold struct instances (name -> struct type name).
    temp_struct_types: FxHashMap<String, String>,
}

impl MetadataCache {
    /// Create an empty metadata cache.
    pub fn new() -> Self {
        Self {
            struct_fields: FxHashMap::default(),
            struct_field_remap: FxHashMap::default(),
            temp_struct_types: FxHashMap::default(),
        }
    }

    /// Register struct field names for a named struct type.
    pub fn register_struct(&mut self, name: &str, fields: Vec<String>) {
        self.struct_fields.insert(name.to_string(), fields);
    }

    /// Get field names for a struct type.
    pub fn get_struct_fields(&self, name: &str) -> Option<&Vec<String>> {
        self.struct_fields.get(name)
    }

    /// Register a P06 field reordering map.
    pub fn register_field_remap(&mut self, name: &str, remap: Vec<usize>) {
        self.struct_field_remap.insert(name.to_string(), remap);
    }

    /// Get the P06 field reordering map for a struct.
    pub fn get_field_remap(&self, name: &str) -> Option<&Vec<usize>> {
        self.struct_field_remap.get(name)
    }

    /// Track that a temp variable holds a struct instance.
    pub fn set_temp_struct_type(&mut self, var_name: &str, struct_name: &str) {
        self.temp_struct_types
            .insert(var_name.to_string(), struct_name.to_string());
    }

    /// Get the struct type name for a temp variable.
    pub fn get_temp_struct_type(&self, var_name: &str) -> Option<&String> {
        self.temp_struct_types.get(var_name)
    }

    /// Clear all metadata (e.g., between functions).
    pub fn clear(&mut self) {
        self.struct_fields.clear();
        self.struct_field_remap.clear();
        self.temp_struct_types.clear();
    }
}

impl Default for MetadataCache {
    fn default() -> Self {
        Self::new()
    }
}

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
    /// Returns the PHYSICAL index of the field in the LLVM struct layout.
    /// If P06 field reordering is active for this struct, the logical
    /// (declaration-order) index is remapped to the physical position.
    /// First checks the struct_metadata cache, then falls back to the type registry
    /// for imported/cross-module types.
    pub fn get_field_index(&self, struct_name: &str, field_name: &str) -> Option<u32> {
        // First try the cached struct_metadata
        let logical_idx = self
            .struct_metadata
            .get(struct_name)
            .and_then(|fields| fields.iter().position(|f| f == field_name))
            .map(|idx| idx as u32);

        if let Some(logical) = logical_idx {
            // Apply P06 remapping if active for this struct
            if let Some(remap) = self.struct_field_remap.get(struct_name) {
                if let Some(&physical) = remap.get(logical as usize) {
                    return Some(physical as u32);
                }
            }
            return Some(logical);
        }

        // Fall back to type registry - search all types for struct with matching name
        // This handles TypeRef cases where lookup returns the TypeRef, not the actual struct
        for type_id in self.type_registry.all_type_ids() {
            if let Some(info) = self.type_registry.get(type_id) {
                if let TypeKind::Struct { def } = &info.kind {
                    if def.name.as_ref() == struct_name {
                        let logical = def
                            .fields
                            .iter()
                            .position(|f| f.name.as_ref() == field_name)
                            .map(|idx| idx as u32)?;
                        // Apply P06 remapping
                        if let Some(remap) = self.struct_field_remap.get(struct_name) {
                            if let Some(&physical) = remap.get(logical as usize) {
                                return Some(physical as u32);
                            }
                        }
                        return Some(logical);
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
                if let TypeKind::Struct { def } = &info.kind {
                    if def.name.as_ref() == struct_name {
                        return def
                            .fields
                            .iter()
                            .find(|f| f.name.as_ref() == field_name)
                            .map(|f| f.type_id);
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
        if let TypeKind::Struct { def } = &type_info.kind {
            Some(def.name.resolve().to_string())
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
                if let TypeKind::Struct { def } = &info.kind {
                    if def.name.as_ref() == struct_name {
                        return Some(def.fields.iter().map(|f| f.type_id).collect());
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
