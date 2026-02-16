//! Type System Interface
//!
//! Centralized Doo type to LLVM type mapping.
//! Single source of truth for type conversions.

use inkwell::context::Context;
use inkwell::types::{BasicType, BasicTypeEnum, StructType};
use inkwell::AddressSpace;
use doo_core::types::{TypeId, builtin};

/// Type mapper - converts Doo types to LLVM types.
pub struct TypeMapper<'ctx> {
    context: &'ctx Context,
}

impl<'ctx> TypeMapper<'ctx> {
    /// Create a new type mapper.
    pub fn new(context: &'ctx Context) -> Self {
        Self { context }
    }

    /// Map TypeId to LLVM BasicTypeEnum.
    pub fn map_type(&self, type_id: TypeId) -> BasicTypeEnum<'ctx> {
        // Use builtin constants for comparison
        if type_id == builtin::INT {
            return self.context.i64_type().into();
        }
        if type_id == builtin::FLOAT {
            return self.context.f64_type().into();
        }
        if type_id == builtin::BOOL {
            // Use i8 (not i1) for Bool — C ABI compatible
            return self.context.i8_type().into();
        }
        if type_id == builtin::STR {
            return self.context.i8_type().ptr_type(AddressSpace::default()).into();
        }
        if type_id == builtin::VOID {
            return self.context.i8_type().into();
        }
        if type_id == builtin::ANY {
            return self.context.i8_type().ptr_type(AddressSpace::default()).into();
        }
        
        // Default - i64 for unknown types
        self.context.i64_type().into()
    }

    /// Create an array type.
    pub fn array_type(&self, _element_type: BasicTypeEnum<'ctx>) -> StructType<'ctx> {
        // Array: { i64 len, i64 cap, ptr data }
        self.context.struct_type(
            &[
                self.context.i64_type().into(),  // len
                self.context.i64_type().into(),  // cap
                self.context.i8_type().ptr_type(AddressSpace::default()).into(),  // data
            ],
            false,
        )
    }

    /// Create a map type.
    pub fn map_type_struct(&self) -> StructType<'ctx> {
        // Map: { i64 len, ptr keys, ptr values }
        self.context.struct_type(
            &[
                self.context.i64_type().into(),  // len
                self.context.i8_type().ptr_type(AddressSpace::default()).into(),  // keys
                self.context.i8_type().ptr_type(AddressSpace::default()).into(),  // values
            ],
            false,
        )
    }

    /// Create a Result type.
    pub fn result_type(&self, _ok_type: BasicTypeEnum<'ctx>, _err_type: BasicTypeEnum<'ctx>) -> StructType<'ctx> {
        // Result: { i8 is_ok, ptr value } — i8 for C ABI compatibility
        self.context.struct_type(
            &[
                self.context.i8_type().into(),  // is_ok (i8 for ABI)
                self.context.i8_type().ptr_type(AddressSpace::default()).into(),  // value
            ],
            false,
        )
    }

    /// Create an optional type.
    pub fn optional_type(&self, inner_type: BasicTypeEnum<'ctx>) -> StructType<'ctx> {
        // Optional: { i8 has_value, T value } — i8 for C ABI compatibility
        self.context.struct_type(
            &[
                self.context.i8_type().into(),  // has_value (i8 for ABI)
                inner_type,  // value
            ],
            false,
        )
    }

    /// Create a tuple type.
    pub fn tuple_type(&self, element_types: &[BasicTypeEnum<'ctx>]) -> StructType<'ctx> {
        self.context.struct_type(element_types, false)
    }

    /// Create a closure type.
    pub fn closure_type(&self) -> StructType<'ctx> {
        // Closure: { ptr func, ptr env }
        self.context.struct_type(
            &[
                self.context.i8_type().ptr_type(AddressSpace::default()).into(),  // func
                self.context.i8_type().ptr_type(AddressSpace::default()).into(),  // env
            ],
            false,
        )
    }

    /// Check if a type is a primitive (Copy type).
    pub fn is_primitive(&self, type_id: TypeId) -> bool {
        type_id == builtin::INT ||
        type_id == builtin::FLOAT ||
        type_id == builtin::BOOL ||
        type_id == builtin::VOID
    }

    /// Check if a type needs Drop (non-primitive, owns heap data).
    pub fn needs_drop(&self, type_id: TypeId) -> bool {
        !self.is_primitive(type_id)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_primitive_types() {
        let ctx = Context::create();
        let mapper = TypeMapper::new(&ctx);

        let int_type = mapper.map_type(builtin::INT);
        assert!(int_type.is_int_type());

        let float_type = mapper.map_type(builtin::FLOAT);
        assert!(float_type.is_float_type());

        let bool_type = mapper.map_type(builtin::BOOL);
        assert!(bool_type.is_int_type());
    }

    #[test]
    fn test_is_primitive() {
        let ctx = Context::create();
        let mapper = TypeMapper::new(&ctx);

        assert!(mapper.is_primitive(builtin::INT));
        assert!(mapper.is_primitive(builtin::BOOL));
        assert!(!mapper.is_primitive(builtin::STR));
    }
}
