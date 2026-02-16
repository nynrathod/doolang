//! LLVM type caching and struct type management.

use doo_core::types::{builtin, TypeId, TypeKind};
use inkwell::types::{BasicTypeEnum, StructType};
use inkwell::AddressSpace;

use super::CodegenContext;

impl<'ctx> CodegenContext<'ctx> {
    // ========================================================================
    // Type Mapping (Single Source of Truth)
    // ========================================================================

    /// Get LLVM type for a Doo TypeId.
    pub fn get_llvm_type(&mut self, type_id: TypeId) -> BasicTypeEnum<'ctx> {
        if let Some(t) = self.type_cache.get(&type_id) {
            return *t;
        }

        let llvm_type = self.create_llvm_type(type_id);
        self.type_cache.insert(type_id, llvm_type);
        llvm_type
    }

    /// Create LLVM type for TypeId (internal).
    fn create_llvm_type(&self, type_id: TypeId) -> BasicTypeEnum<'ctx> {
        // Builtins
        if type_id == builtin::INT {
            return self.context.i64_type().into();
        }
        if type_id == builtin::FLOAT {
            return self.context.f64_type().into();
        }
        if type_id == builtin::BOOL {
            // Use i8 (not i1) for Bool type — C ABI uses i8/i32 for bools.
            // i1 in struct fields causes LLVM to use bitfield packing, creating
            // layout mismatches with FFI structs. i8 ensures predictable 1-byte alignment.
            return self.context.i8_type().into();
        }
        if type_id == builtin::STR {
            return self
                .context
                .i8_type()
                .ptr_type(AddressSpace::default())
                .into();
        }
        if type_id == builtin::VOID {
            return self.context.i8_type().into();
        }
        if type_id == builtin::ANY {
            return self
                .context
                .i8_type()
                .ptr_type(AddressSpace::default())
                .into();
        }

        // Canonical types from registry
        if let Some(info) = self.type_registry.get(type_id) {
            match &info.kind {
                TypeKind::Void => self.context.i8_type().into(),
                TypeKind::Bool => self.context.i8_type().into(),
                TypeKind::Int => self.context.i64_type().into(),
                TypeKind::Float => self.context.f64_type().into(),
                TypeKind::Str => self
                    .context
                    .i8_type()
                    .ptr_type(AddressSpace::default())
                    .into(),
                TypeKind::Any | TypeKind::Error => self
                    .context
                    .i8_type()
                    .ptr_type(AddressSpace::default())
                    .into(),

                TypeKind::Array { .. }
                | TypeKind::Map { .. }
                | TypeKind::Optional { .. }
                | TypeKind::Result { .. }
                | TypeKind::Tuple { .. }
                | TypeKind::Struct { .. }
                | TypeKind::Function { .. }
                | TypeKind::TypeRef { .. } => self
                    .context
                    .i8_type()
                    .ptr_type(AddressSpace::default())
                    .into(),

                TypeKind::Enum { .. } => {
                    // Enum layout: { i32 tag, ptr payload }
                    let ptr_type = self.context.i8_type().ptr_type(AddressSpace::default());
                    self.context
                        .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false)
                        .into()
                }
            }
        } else {
            // Unknown type: treat as opaque pointer
            self.context
                .i8_type()
                .ptr_type(AddressSpace::default())
                .into()
        }
    }

    /// Get or create a struct type.
    pub fn get_struct_type(
        &mut self,
        name: &str,
        field_types: &[BasicTypeEnum<'ctx>],
    ) -> StructType<'ctx> {
        if let Some(t) = self.struct_cache.get(name) {
            return *t;
        }

        let struct_type = self.context.opaque_struct_type(name);
        struct_type.set_body(field_types, false);
        self.struct_cache.insert(name.to_string(), struct_type);
        struct_type
    }

    /// Get cached struct type by name.
    pub fn lookup_struct_type(&self, name: &str) -> Option<StructType<'ctx>> {
        self.struct_cache.get(name).copied()
    }

    /// Get or build a struct type by name, using the type registry if not cached.
    /// This handles imported/cross-module types that weren't pre-declared.
    pub fn get_or_build_struct_type(&mut self, name: &str) -> Option<StructType<'ctx>> {
        // First try the cache
        if let Some(st) = self.struct_cache.get(name).copied() {
            // Also ensure struct_metadata is populated
            if !self.struct_metadata.contains_key(name) {
                // Search for the struct definition to populate metadata
                if let Some(field_names) = self.find_struct_field_names(name) {
                    self.struct_metadata.insert(name.to_string(), field_names);
                }
            }
            return Some(st);
        }

        // Try to build from type registry
        // Search all types for a Struct with matching name
        // (handles TypeRef cases where lookup returns the TypeRef, not the actual struct)
        let (field_type_ids, field_names): (Vec<TypeId>, Vec<String>) = {
            let mut found = None;
            for type_id in self.type_registry.all_type_ids() {
                if let Some(info) = self.type_registry.get(type_id) {
                    if let TypeKind::Struct {
                        name: sname,
                        fields,
                    } = &info.kind
                    {
                        if sname == name {
                            let ids: Vec<TypeId> = fields.iter().map(|(_, tid, _)| *tid).collect();
                            let names: Vec<String> =
                                fields.iter().map(|(n, _, _)| n.clone()).collect();
                            found = Some((ids, names));
                            break;
                        }
                    }
                }
            }
            found?
        };

        // Now we can call self methods without borrow conflict
        let field_types: Vec<BasicTypeEnum<'ctx>> = field_type_ids
            .iter()
            .map(|tid| self.get_llvm_type(*tid))
            .collect();

        // Create and cache the struct type
        let struct_type = self.context.opaque_struct_type(name);
        struct_type.set_body(&field_types, false);
        self.struct_cache.insert(name.to_string(), struct_type);
        self.struct_metadata.insert(name.to_string(), field_names);

        Some(struct_type)
    }

    /// Helper to find struct field names from the type registry
    fn find_struct_field_names(&self, name: &str) -> Option<Vec<String>> {
        for type_id in self.type_registry.all_type_ids() {
            if let Some(info) = self.type_registry.get(type_id) {
                if let TypeKind::Struct {
                    name: sname,
                    fields,
                } = &info.kind
                {
                    if sname == name {
                        return Some(fields.iter().map(|(n, _, _)| n.clone()).collect());
                    }
                }
            }
        }
        None
    }
}
