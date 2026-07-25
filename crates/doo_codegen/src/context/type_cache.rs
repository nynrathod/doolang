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
                .ptr_type(AddressSpace::default())
                .into();
        }
        if type_id == builtin::VOID {
            return self.context.i8_type().into();
        }
        if type_id == builtin::ANY {
            return self
                .context
                .ptr_type(AddressSpace::default())
                .into();
        }

        // Canonical types from registry
        if let Some(info) = self.type_registry.get(type_id) {
            match &info.kind {
                TypeKind::Void => self.context.i8_type().into(),
                TypeKind::Bool => self.context.i8_type().into(),
                TypeKind::Int => self.context.i64_type().into(),
                TypeKind::Float32 | TypeKind::Float64 => self.context.f64_type().into(),
                TypeKind::Str => self
                    .context
                    .ptr_type(AddressSpace::default())
                    .into(),


                TypeKind::Array { .. }
                | TypeKind::Map { .. }
                | TypeKind::Set { .. }
                | TypeKind::Optional { .. }
                | TypeKind::Result { .. }
                | TypeKind::Box { .. }
                | TypeKind::Tuple { .. }
                | TypeKind::Struct { .. }
                | TypeKind::Function { .. }
                | TypeKind::TypeRef { .. }
                | TypeKind::TypeParam { .. }
                | TypeKind::Never
                | TypeKind::Char
                | TypeKind::Int8
                | TypeKind::Int16
                | TypeKind::Int32
                | TypeKind::Int64
                | TypeKind::UInt8
                | TypeKind::UInt16
                | TypeKind::UInt32
                | TypeKind::UInt64
                | TypeKind::UInt
                | TypeKind::SelfType
                | TypeKind::Any
                | TypeKind::Error => self
                    .context
                    .ptr_type(AddressSpace::default())
                    .into(),

                TypeKind::Enum { .. } => {
                    // Enum layout: { i32 tag, ptr payload }
                    let ptr_type = self.context.ptr_type(AddressSpace::default());
                    self.context
                        .struct_type(&[self.context.i32_type().into(), ptr_type.into()], false)
                        .into()
                }
                TypeKind::Interface { .. } => {
                    // Interface layout: fat pointer { data_ptr, vtable_ptr }
                    let ptr_type = self.context.ptr_type(AddressSpace::default());
                    self.context
                        .struct_type(&[ptr_type.into(), ptr_type.into()], false)
                        .into()
                }
            }
        } else {
            // Unknown type: treat as opaque pointer
            self.context
                .ptr_type(AddressSpace::default())
                .into()
        }
    }

    /// Get or create a struct type.
    ///
    /// P06: When creating a new struct type, fields are reordered by alignment
    /// (largest first) to minimize padding. The logical→physical index mapping
    /// is stored in `struct_field_remap` for use by FieldGet/FieldSet/Drop/Clone.
    /// Callers should always pass field types in declaration (logical) order.
    pub fn get_struct_type(
        &mut self,
        name: &str,
        field_types: &[BasicTypeEnum<'ctx>],
    ) -> StructType<'ctx> {
        if let Some(t) = self.struct_cache.get(name) {
            return *t;
        }

        let field_count = field_types.len();
        let (reordered_types, remap) = if field_count > 1 {
            // Compute alignment for each field from its LLVM type
            let mut indices: Vec<usize> = (0..field_count).collect();
            indices.sort_by(|&a, &b| {
                let align_a = Self::llvm_type_alignment(&field_types[a]);
                let align_b = Self::llvm_type_alignment(&field_types[b]);
                align_b.cmp(&align_a) // largest alignment first
            });

            // Check if reorder is identity (no change needed)
            let is_identity = indices.iter().enumerate().all(|(i, &v)| i == v);
            if is_identity {
                (field_types.to_vec(), None)
            } else {
                // Build logical→physical map
                let mut logical_to_physical = vec![0usize; field_count];
                for (physical, &logical) in indices.iter().enumerate() {
                    logical_to_physical[logical] = physical;
                }
                let reordered: Vec<BasicTypeEnum<'ctx>> =
                    indices.iter().map(|&i| field_types[i]).collect();
                (reordered, Some(logical_to_physical))
            }
        } else {
            (field_types.to_vec(), None)
        };

        // Store the remap if reordering happened
        if let Some(remap) = remap {
            self.struct_field_remap.insert(name.to_string(), remap);
        }

        let struct_type = self.context.opaque_struct_type(name);
        struct_type.set_body(&reordered_types, false);
        self.struct_cache.insert(name.to_string(), struct_type);
        struct_type
    }

    /// Get alignment in bytes for an LLVM BasicTypeEnum (for P06 struct field reordering).
    fn llvm_type_alignment(ty: &BasicTypeEnum) -> u64 {
        match ty {
            BasicTypeEnum::IntType(it) => {
                let bits = it.get_bit_width();
                if bits <= 8 {
                    1
                } else if bits <= 32 {
                    4
                } else {
                    8
                }
            }
            BasicTypeEnum::FloatType(_) => 8, // f64
            BasicTypeEnum::PointerType(_) => 8,
            BasicTypeEnum::StructType(_) => 8, // nested structs have pointer alignment
            BasicTypeEnum::ArrayType(_) => 8,
            BasicTypeEnum::VectorType(_) => 8,
            _ => 8, // ScalableVectorType and future variants
        }
    }

    /// Get the physical field index for a logical (declaration-order) field index.
    /// Returns the logical index unchanged if no P06 remap exists for this struct.
    pub fn physical_field_index(&self, struct_name: &str, logical_idx: usize) -> usize {
        self.struct_field_remap
            .get(struct_name)
            .and_then(|r| r.get(logical_idx).copied())
            .unwrap_or(logical_idx)
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
                    if let TypeKind::Struct { def } = &info.kind
                    {
                        if def.name.as_ref() == name {
                            let ids: Vec<TypeId> = def.fields.iter().map(|f| f.type_id).collect();
                            let names: Vec<String> =
                                def.fields.iter().map(|f| f.name.resolve().to_string()).collect();
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

        // Create and cache the struct type via get_struct_type (applies P06 reordering)
        let struct_type = self.get_struct_type(name, &field_types);
        self.struct_metadata.insert(name.to_string(), field_names);

        Some(struct_type)
    }

    /// Helper to find struct field names from the type registry
    fn find_struct_field_names(&self, name: &str) -> Option<Vec<String>> {
        for type_id in self.type_registry.all_type_ids() {
            if let Some(info) = self.type_registry.get(type_id) {
                if let TypeKind::Struct { def } = &info.kind
                {
                    if def.name.as_ref() == name {
                        return Some(def.fields.iter().map(|f| f.name.resolve().to_string()).collect());
                    }
                }
            }
        }
        None
    }
}
