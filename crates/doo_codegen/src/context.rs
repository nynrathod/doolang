//! Codegen Context
//!
//! Central context for LLVM code generation.
//! Holds the LLVM module, builder, and type mappings.

use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::builder::Builder;
use inkwell::types::{BasicType, BasicTypeEnum, StructType};
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};
use inkwell::AddressSpace;
use rustc_hash::FxHashMap;
use doo_core::types::{TypeId, TypeKind, TypeRegistry, builtin};
use std::sync::Arc;

/// Code generation context.
///
/// Single source of truth for LLVM objects during codegen.
pub struct CodegenContext<'ctx> {
    /// LLVM context.
    pub context: &'ctx Context,
    /// LLVM module being built.
    pub module: Module<'ctx>,
    /// LLVM IR builder.
    pub builder: Builder<'ctx>,
    /// Type cache: TypeId -> LLVM type.
    type_cache: FxHashMap<TypeId, BasicTypeEnum<'ctx>>,
    /// Struct type cache: name -> LLVM struct type.
    struct_cache: FxHashMap<String, StructType<'ctx>>,
    /// Function cache: name -> LLVM function.
    function_cache: FxHashMap<String, FunctionValue<'ctx>>,
    /// Local variables: name -> LLVM alloca pointer.
    locals: FxHashMap<String, (PointerValue<'ctx>, BasicTypeEnum<'ctx>)>,
    /// Temporaries: name -> LLVM value.
    temps: FxHashMap<String, BasicValueEnum<'ctx>>,

    /// Canonical type registry (single source of truth for type/kind).
    pub type_registry: Arc<TypeRegistry>,
}

impl<'ctx> CodegenContext<'ctx> {
    /// Create a new codegen context.
    pub fn new(context: &'ctx Context, module_name: &str, type_registry: Arc<TypeRegistry>) -> Self {
        let module = context.create_module(module_name);
        let builder = context.create_builder();

        Self {
            context,
            module,
            builder,
            type_cache: FxHashMap::default(),
            struct_cache: FxHashMap::default(),
            function_cache: FxHashMap::default(),
            locals: FxHashMap::default(),
            temps: FxHashMap::default(),
            type_registry,
        }
    }

    pub fn get_type_kind(&self, type_id: TypeId) -> Option<TypeKind> {
        self.type_registry.get(type_id).map(|info| info.kind.clone())
    }

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
            return self.context.bool_type().into();
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

        // Canonical types from registry
        if let Some(info) = self.type_registry.get(type_id) {
            match &info.kind {
                TypeKind::Void => self.context.i8_type().into(),
                TypeKind::Bool => self.context.bool_type().into(),
                TypeKind::Int => self.context.i64_type().into(),
                TypeKind::Float => self.context.f64_type().into(),
                TypeKind::Str => self.context.i8_type().ptr_type(AddressSpace::default()).into(),
                TypeKind::Any | TypeKind::Error => self.context.i8_type().ptr_type(AddressSpace::default()).into(),

                TypeKind::Array { .. }
                | TypeKind::Map { .. }
                | TypeKind::Optional { .. }
                | TypeKind::Result { .. }
                | TypeKind::Tuple { .. }
                | TypeKind::Struct { .. }
                | TypeKind::Enum { .. }
                | TypeKind::Function { .. }
                | TypeKind::TypeRef { .. } => {
                    self.context.i8_type().ptr_type(AddressSpace::default()).into()
                }
            }
        } else {
            // Unknown type: treat as opaque pointer
            self.context.i8_type().ptr_type(AddressSpace::default()).into()
        }
    }

    /// Get or create a struct type.
    pub fn get_struct_type(&mut self, name: &str, field_types: &[BasicTypeEnum<'ctx>]) -> StructType<'ctx> {
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

    // ========================================================================
    // Function Management
    // ========================================================================

    /// Declare a function.
    pub fn declare_function(
        &mut self,
        name: &str,
        param_types: &[BasicTypeEnum<'ctx>],
        return_type: Option<BasicTypeEnum<'ctx>>,
    ) -> FunctionValue<'ctx> {
        let fn_type = match return_type {
            Some(ret) => ret.fn_type(
                &param_types.iter().map(|t| (*t).into()).collect::<Vec<_>>(),
                false,
            ),
            None => self.context.void_type().fn_type(
                &param_types.iter().map(|t| (*t).into()).collect::<Vec<_>>(),
                false,
            ),
        };

        let func = self.module.add_function(name, fn_type, None);
        self.function_cache.insert(name.to_string(), func);
        func
    }

    /// Get a function by name.
    pub fn get_function(&self, name: &str) -> Option<FunctionValue<'ctx>> {
        self.function_cache.get(name).copied()
            .or_else(|| self.module.get_function(name))
    }

    // ========================================================================
    // Local Variable Management
    // ========================================================================

    /// Create an alloca (local variable).
    pub fn create_local(&mut self, name: &str, ty: BasicTypeEnum<'ctx>) -> PointerValue<'ctx> {
        let alloca = self.builder.build_alloca(ty, name).unwrap();
        self.locals.insert(name.to_string(), (alloca, ty));
        alloca
    }

    /// Get a local variable.
    pub fn get_local(&self, name: &str) -> Option<PointerValue<'ctx>> {
        self.locals.get(name).map(|(ptr, _)| *ptr)
    }

    /// Get the LLVM type of a local variable.
    pub fn get_local_type(&self, name: &str) -> Option<BasicTypeEnum<'ctx>> {
        self.locals.get(name).map(|(_, ty)| *ty)
    }

    /// Clear locals (for new function).
    pub fn clear_locals(&mut self) {
        self.locals.clear();
        self.temps.clear();
    }

    // ========================================================================
    // Temporary Management
    // ========================================================================

    /// Store a temporary value.
    pub fn set_temp(&mut self, name: &str, value: BasicValueEnum<'ctx>) {
        self.temps.insert(name.to_string(), value);
    }

    /// Get a temporary value.
    pub fn get_temp(&self, name: &str) -> Option<BasicValueEnum<'ctx>> {
        self.temps.get(name).copied()
    }

    /// Get value by name (local or temp).
    pub fn get_value(&self, name: &str) -> Option<BasicValueEnum<'ctx>> {
        // Check temps first
        if let Some(v) = self.temps.get(name) {
            return Some(*v);
        }
        // Check locals - return loaded value
        if let Some((ptr, ty)) = self.locals.get(name) {
            return self.builder.build_load(*ty, *ptr, name).ok();
        }
        None
    }

    // ========================================================================
    // Helper Methods
    // ========================================================================

    /// Get the i64 type.
    pub fn i64_type(&self) -> inkwell::types::IntType<'ctx> {
        self.context.i64_type()
    }

    /// Get the i32 type.
    pub fn i32_type(&self) -> inkwell::types::IntType<'ctx> {
        self.context.i32_type()
    }

    /// Get the i8 type.
    pub fn i8_type(&self) -> inkwell::types::IntType<'ctx> {
        self.context.i8_type()
    }

    /// Get the bool type.
    pub fn bool_type(&self) -> inkwell::types::IntType<'ctx> {
        self.context.bool_type()
    }

    /// Get the f64 type.
    pub fn f64_type(&self) -> inkwell::types::FloatType<'ctx> {
        self.context.f64_type()
    }

    /// Get the pointer type.
    pub fn ptr_type(&self) -> inkwell::types::PointerType<'ctx> {
        self.context.i8_type().ptr_type(AddressSpace::default())
    }

    /// Create an i64 constant.
    pub fn const_i64(&self, val: i64) -> inkwell::values::IntValue<'ctx> {
        self.context.i64_type().const_int(val as u64, true)
    }

    /// Create an f64 constant.
    pub fn const_f64(&self, val: f64) -> inkwell::values::FloatValue<'ctx> {
        self.context.f64_type().const_float(val)
    }

    /// Create a bool constant.
    pub fn const_bool(&self, val: bool) -> inkwell::values::IntValue<'ctx> {
        self.context.bool_type().const_int(val as u64, false)
    }

    /// Create a global string constant.
    pub fn const_string(&self, val: &str) -> PointerValue<'ctx> {
        let global = self.builder.build_global_string_ptr(val, "str").unwrap();
        global.as_pointer_value()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_context_creation() {
        let ctx = Context::create();
        let registry = Arc::new(TypeRegistry::new());
        let codegen = CodegenContext::new(&ctx, "test", registry);
        assert!(codegen.module.get_name().to_str().unwrap() == "test");
    }

    #[test]
    fn test_type_mapping() {
        let ctx = Context::create();
        let registry = Arc::new(TypeRegistry::new());
        let mut codegen = CodegenContext::new(&ctx, "test", registry);
        
        let int_type = codegen.get_llvm_type(builtin::INT);
        assert!(int_type.is_int_type());
        
        let float_type = codegen.get_llvm_type(builtin::FLOAT);
        assert!(float_type.is_float_type());
    }

    #[test]
    fn test_declare_function() {
        let ctx = Context::create();
        let registry = Arc::new(TypeRegistry::new());
        let mut codegen = CodegenContext::new(&ctx, "test", registry);
        
        let func = codegen.declare_function("foo", &[], None);
        assert_eq!(func.get_name().to_str().unwrap(), "foo");
    }
}
