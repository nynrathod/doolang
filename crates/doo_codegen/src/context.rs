//! Codegen Context
//!
//! Central context for LLVM code generation.
//! Holds the LLVM module, builder, and type mappings.
//!
//! ## Multi-File Support
//!
//! The codegen context supports multi-file compilation through:
//! - `function_aliases`: Maps alias names to original function names (for imports with aliases)
//! - `external_functions`: Declared external functions from other modules
//! - `ModuleLinker`: Utility to link multiple LLVM modules together

use doo_core::doo_debug;
use doo_core::types::{builtin, TypeId, TypeKind, TypeRegistry};
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, StructType};
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};
use inkwell::AddressSpace;
use rustc_hash::FxHashMap;
use std::sync::Arc;

// ============================================================================
// External Function Metadata
// ============================================================================

/// Metadata for an external function declaration.
///
/// Used to declare functions from other modules before linking.
#[derive(Debug, Clone)]
pub struct ExternalFunction {
    /// Original function name in the source module.
    pub name: String,
    /// Module the function comes from.
    pub source_module: String,
    /// Parameter types.
    pub param_types: Vec<TypeId>,
    /// Return type (None = void).
    pub return_type: Option<TypeId>,
    /// Whether function is variadic.
    pub variadic: bool,
}

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

    // ========================================================================
    // Multi-File Support
    // ========================================================================
    /// Function aliases: alias_name -> original_name.
    /// Used for import aliasing (e.g., `import std::Math::Sqrt as Sq`).
    pub function_aliases: FxHashMap<String, String>,

    /// External function declarations from other modules.
    /// These are declared with external linkage before linking.
    pub external_functions: FxHashMap<String, ExternalFunction>,

    /// Module dependencies: module_name -> list of function names used.
    /// Tracks which functions are needed from which modules.
    pub module_dependencies: FxHashMap<String, Vec<String>>,

    // ========================================================================
    // Struct Metadata Tracking (for FieldGet/FieldSet)
    // ========================================================================
    /// Struct metadata: struct_name -> field names (in order).
    /// Used for looking up field indices by name.
    pub struct_metadata: FxHashMap<String, Vec<String>>,

    /// Temp/local type tracking: variable_name -> struct_name.
    /// Tracks which struct type each temp/local variable holds.
    pub temp_struct_types: FxHashMap<String, String>,

    // ========================================================================
    // Variable Type Tracking (for Clone/Drop)
    // ========================================================================
    /// Variable types: variable_name -> TypeId.
    /// Tracks the Doo type of each variable for deep clone/drop decisions.
    pub variable_types: FxHashMap<String, TypeId>,

    // ========================================================================
    // Function Parameter Types (for type coercion in calls)
    // ========================================================================
    /// Function parameter types: function_name -> Vec<TypeId>.
    /// Tracks the Doo types of each function's parameters for argument coercion.
    pub function_param_types: FxHashMap<String, Vec<TypeId>>,

    // ========================================================================
    // Function Return Types (for struct serialization in HTTP handlers)
    // ========================================================================
    /// Function return types: function_name -> TypeId.
    /// Tracks the Doo return type of each function for serialization.
    pub function_return_types: FxHashMap<String, TypeId>,

    // ========================================================================
    // Function Error Types (for middleware error handling)
    // ========================================================================
    /// Function error types: function_name -> TypeId.
    /// Tracks the Doo error type for functions that return Result<T, E>.
    pub function_error_types: FxHashMap<String, TypeId>,

    // ========================================================================
    // Current Function Return Type (for type conversion in returns)
    // ========================================================================
    /// Current function's return type (for proper return value conversion).
    pub current_function_return_type: Option<TypeId>,

    // ========================================================================
    // Borrow Origin Tracking (for mutating operations)
    // ========================================================================
    /// Borrow origins: temp_name -> original_local_name.
    /// When a temp is a borrow of a local, this tracks the origin so mutating
    /// operations can store back to the original local's alloca.
    pub borrow_origins: FxHashMap<String, String>,

    // ========================================================================
    // Closure Function Flag
    // ========================================================================
    /// Whether the currently generating function is a closure.
    /// Closures have special calling convention (all params/returns as i64).
    pub is_closure_function: bool,

    // ========================================================================
    // FFI Symbol Tracking (for wrapper generation)
    // ========================================================================
    /// FFI symbols: function_name -> (library, symbol).
    /// Tracks FFI functions so wrapper generator can call correct symbol.
    pub ffi_symbols: FxHashMap<String, (String, String)>,

    // ========================================================================
    // Array Element Type Tracking (for enum serialization in FFI calls)
    // ========================================================================
    /// Array element types: temp_name -> element TypeId.
    /// When an array is created, tracks the element type so FFI calls
    /// can serialize enum arrays to JSON strings.
    pub array_element_types: FxHashMap<String, TypeId>,

    /// Array element temps: array_temp_name -> list of element temp names.
    /// When an array is created, tracks the individual element temps so
    /// mixed-type arrays can be serialized by checking each element.
    pub array_element_temps: FxHashMap<String, Vec<String>>,

    // ========================================================================
    // Async Features Flag
    // ========================================================================
    /// Whether the program uses async features (set during build).
    /// When true, codegen emits `doo_runtime_init()` at the start of main().
    pub has_async: bool,
}

impl<'ctx> CodegenContext<'ctx> {
    /// Create a new codegen context.
    pub fn new(
        context: &'ctx Context,
        module_name: &str,
        type_registry: Arc<TypeRegistry>,
    ) -> Self {
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
            function_aliases: FxHashMap::default(),
            external_functions: FxHashMap::default(),
            module_dependencies: FxHashMap::default(),
            struct_metadata: FxHashMap::default(),
            temp_struct_types: FxHashMap::default(),
            variable_types: FxHashMap::default(),
            function_param_types: FxHashMap::default(),
            function_return_types: FxHashMap::default(),
            function_error_types: FxHashMap::default(),
            current_function_return_type: None,
            borrow_origins: FxHashMap::default(),
            is_closure_function: false,
            ffi_symbols: FxHashMap::default(),
            array_element_types: FxHashMap::default(),
            array_element_temps: FxHashMap::default(),
            has_async: false,
        }
    }

    pub fn get_type_kind(&self, type_id: TypeId) -> Option<TypeKind> {
        let result = self
            .type_registry
            .get(type_id)
            .map(|info| info.kind.clone());
        if std::env::var("DOO_DEBUG_TYPES").is_ok() {
            doo_debug!("TYPES", "get_type_kind({:?}) = {:?}", type_id, result);
        }
        result
    }

    /// Get enum variant index by enum name and variant name.
    /// Returns the index of the variant in the enum's variant list.
    /// This is the single source of truth for variant indices.
    pub fn get_enum_variant_index(&self, enum_name: &str, variant_name: &str) -> Option<u32> {
        // Look up the enum type by name
        let type_id = self.type_registry.lookup(enum_name)?;
        let type_info = self.type_registry.get(type_id)?;

        // Extract variants from the enum type
        if let TypeKind::Enum { variants, .. } = &type_info.kind {
            // Find the variant by name and return its index
            for (idx, (vname, _payload)) in variants.iter().enumerate() {
                if vname == variant_name {
                    return Some(idx as u32);
                }
            }
        }
        None
    }

    /// Get the payload TypeId for an enum variant.
    /// Returns Some(TypeId) if the variant has a payload, None otherwise.
    pub fn get_enum_variant_payload_type(
        &self,
        enum_name: &str,
        variant_name: &str,
    ) -> Option<TypeId> {
        // Look up the enum type by name
        let type_id = self.type_registry.lookup(enum_name)?;
        let type_info = self.type_registry.get(type_id)?;

        // Extract variants from the enum type
        if let TypeKind::Enum { variants, .. } = &type_info.kind {
            // Find the variant by name and return its payload type
            for (vname, payload) in variants.iter() {
                if vname == variant_name {
                    return payload.clone();
                }
            }
        }
        None
    }

    /// Register function parameter types for argument coercion during calls.
    pub fn register_function_param_types(&mut self, func_name: &str, param_types: Vec<TypeId>) {
        self.function_param_types
            .insert(func_name.to_string(), param_types);
    }

    /// Get function parameter types (for argument coercion during calls).
    pub fn get_function_param_types(&self, func_name: &str) -> Option<&Vec<TypeId>> {
        self.function_param_types.get(func_name)
    }

    /// Register function return type.
    pub fn register_function_return_type(&mut self, func_name: &str, return_type: TypeId) {
        self.function_return_types
            .insert(func_name.to_string(), return_type);
    }

    /// Get function return type (for struct serialization).
    pub fn get_function_return_type(&self, func_name: &str) -> Option<TypeId> {
        self.function_return_types.get(func_name).copied()
    }

    /// Register function error type (for Result<T, E> functions).
    pub fn register_function_error_type(&mut self, func_name: &str, error_type: TypeId) {
        self.function_error_types
            .insert(func_name.to_string(), error_type);
    }

    /// Get function error type (for middleware error handling).
    pub fn get_function_error_type(&self, func_name: &str) -> Option<TypeId> {
        self.function_error_types.get(func_name).copied()
    }

    // ========================================================================
    // FFI Symbol Tracking (Single Source of Truth)
    // ========================================================================

    /// Register FFI function symbol mapping.
    /// This allows wrapper generator to call the correct external symbol.
    pub fn register_ffi_symbol(&mut self, func_name: &str, library: &str, symbol: &str) {
        self.ffi_symbols.insert(
            func_name.to_string(),
            (library.to_string(), symbol.to_string()),
        );
    }

    /// Get FFI symbol for a function (if it's an FFI function).
    /// Returns Some((library, symbol)) if the function is FFI, None otherwise.
    pub fn get_ffi_symbol(&self, func_name: &str) -> Option<(&str, &str)> {
        self.ffi_symbols
            .get(func_name)
            .map(|(lib, sym)| (lib.as_str(), sym.as_str()))
    }

    /// Check if a function is an FFI function.
    pub fn is_ffi_function(&self, func_name: &str) -> bool {
        self.ffi_symbols.contains_key(func_name)
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
                TypeKind::Bool => self.context.bool_type().into(),
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
        // Special handling for main function - must return i32
        let fn_type = if name == "main" {
            self.context.i32_type().fn_type(
                &param_types.iter().map(|t| (*t).into()).collect::<Vec<_>>(),
                false,
            )
        } else {
            match return_type {
                Some(ret) => ret.fn_type(
                    &param_types.iter().map(|t| (*t).into()).collect::<Vec<_>>(),
                    false,
                ),
                None => self.context.void_type().fn_type(
                    &param_types.iter().map(|t| (*t).into()).collect::<Vec<_>>(),
                    false,
                ),
            }
        };

        let func = self.module.add_function(name, fn_type, None);
        self.function_cache.insert(name.to_string(), func);
        func
    }

    /// Get a function by name.
    pub fn get_function(&self, name: &str) -> Option<FunctionValue<'ctx>> {
        self.function_cache
            .get(name)
            .copied()
            .or_else(|| self.module.get_function(name))
    }

    /// Register a function in the cache with an alias name.
    /// Used for FFI functions where the Doo name differs from the symbol name.
    pub fn register_function_alias(&mut self, alias: &str, func: FunctionValue<'ctx>) {
        self.function_cache.insert(alias.to_string(), func);
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

    /// Get a local variable pointer.
    pub fn get_local(&self, name: &str) -> Option<PointerValue<'ctx>> {
        self.locals.get(name).map(|(ptr, _)| *ptr)
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
            if std::env::var("DOO_DEBUG").is_ok() {
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
                        if std::env::var("DOO_DEBUG").is_ok() {
                            doo_debug!(
                                "CODEGEN",
                                "set_local '{}': converted ptr->int via ptrtoint",
                                name
                            );
                        }
                        return;
                    }
                }

                // No conversion possible - store as temp (shadows the local for this scope)
                // get_value checks temps first, so this will be found before the alloca
                if std::env::var("DOO_DEBUG").is_ok() {
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
    // Function Alias Management (Multi-File Support)
    // ========================================================================

    /// Register a function alias.
    ///
    /// Used for import aliasing (e.g., `import std::Math::Sqrt as Sq`).
    pub fn register_alias(&mut self, alias: String, original: String) {
        self.function_aliases.insert(alias, original);
    }

    /// Resolve a function name through aliases.
    ///
    /// Returns the original function name if aliased, or the input name otherwise.
    pub fn resolve_function_name(&self, name: &str) -> String {
        self.function_aliases
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    /// Check if a function name is an alias.
    pub fn is_alias(&self, name: &str) -> bool {
        self.function_aliases.contains_key(name)
    }

    // ========================================================================
    // External Function Declarations (Multi-File Support)
    // ========================================================================

    /// Declare an external function from another module.
    ///
    /// This creates an LLVM function declaration with external linkage,
    /// allowing calls to functions that will be linked later.
    pub fn declare_external_function(
        &mut self,
        name: &str,
        param_types: &[BasicTypeEnum<'ctx>],
        return_type: Option<BasicTypeEnum<'ctx>>,
        source_module: &str,
    ) -> FunctionValue<'ctx> {
        // Check if already declared
        if let Some(func) = self.function_cache.get(name) {
            return *func;
        }

        // Build function type
        let param_meta: Vec<BasicMetadataTypeEnum> =
            param_types.iter().map(|t| (*t).into()).collect();

        let fn_type = match return_type {
            Some(ret) => ret.fn_type(&param_meta, false),
            None => self.context.void_type().fn_type(&param_meta, false),
        };

        // Declare with external linkage
        let func = self
            .module
            .add_function(name, fn_type, Some(Linkage::External));
        self.function_cache.insert(name.to_string(), func);

        // Track module dependency
        self.module_dependencies
            .entry(source_module.to_string())
            .or_default()
            .push(name.to_string());

        func
    }

    /// Register external function metadata.
    ///
    /// Stores information about an external function for later linking.
    pub fn register_external_function(&mut self, ext_fn: ExternalFunction) {
        self.external_functions.insert(ext_fn.name.clone(), ext_fn);
    }

    /// Get external function metadata.
    pub fn get_external_function(&self, name: &str) -> Option<&ExternalFunction> {
        self.external_functions.get(name)
    }

    /// Check if a function is declared as external.
    pub fn is_external_function(&self, name: &str) -> bool {
        self.external_functions.contains_key(name)
    }

    /// Get all module dependencies.
    pub fn get_dependencies(&self) -> &FxHashMap<String, Vec<String>> {
        &self.module_dependencies
    }

    // ========================================================================
    // Temporary Management
    // ========================================================================

    /// Store a temporary value.
    pub fn set_temp(&mut self, name: &str, value: BasicValueEnum<'ctx>) {
        if std::env::var("DOO_DEBUG").is_ok() {
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
            if std::env::var("DOO_DEBUG").is_ok() {
                doo_debug!("CODEGEN", "get_value({}) found in temps", name);
            }
            return Some(*v);
        }
        // Check locals - return loaded value
        if let Some((ptr, ty)) = self.locals.get(name) {
            if std::env::var("DOO_DEBUG").is_ok() {
                doo_debug!(
                    "CODEGEN",
                    "get_value({}) loading from local, ty={:?}",
                    name,
                    ty
                );
            }
            let result = self.builder.build_load(*ty, *ptr, name);
            if result.is_err() {
                if std::env::var("DOO_DEBUG").is_ok() {
                    doo_debug!(
                        "CODEGEN",
                        "ERROR: build_load failed for {}: {:?}",
                        name,
                        result
                    );
                }
            } else if std::env::var("DOO_DEBUG").is_ok() {
                doo_debug!("CODEGEN", "get_value({}) loaded successfully", name);
            }
            return result.ok();
        }
        if std::env::var("DOO_DEBUG").is_ok() {
            doo_debug!(
                "CODEGEN",
                "WARNING: Variable {} not found in temps or locals",
                name
            );
        }
        None
    }

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
// Module Linker (Multi-File Support)
// ============================================================================

/// Error type for module linking operations.
#[derive(Debug, Clone)]
pub enum LinkError {
    /// LLVM module linking failed.
    LinkFailed(String),
    /// Symbol not found in any module.
    SymbolNotFound(String),
    /// Duplicate symbol definition.
    DuplicateSymbol(String),
    /// Module not found.
    ModuleNotFound(String),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::LinkFailed(msg) => write!(f, "Link failed: {}", msg),
            LinkError::SymbolNotFound(sym) => write!(f, "Symbol not found: {}", sym),
            LinkError::DuplicateSymbol(sym) => write!(f, "Duplicate symbol: {}", sym),
            LinkError::ModuleNotFound(name) => write!(f, "Module not found: {}", name),
        }
    }
}

impl std::error::Error for LinkError {}

impl From<LinkError> for doo_core::errors::codes::CompilerError {
    fn from(e: LinkError) -> Self {
        use doo_core::errors::codes::ErrorCode;
        let (code, msg) = match &e {
            LinkError::LinkFailed(m) => (ErrorCode::LlvmError, format!("link failed: {}", m)),
            LinkError::SymbolNotFound(s) => {
                (ErrorCode::CodegenFailed, format!("symbol not found: {}", s))
            }
            LinkError::DuplicateSymbol(s) => (
                ErrorCode::NameAlreadyDefined,
                format!("duplicate symbol: {}", s),
            ),
            LinkError::ModuleNotFound(n) => (
                ErrorCode::ModuleNotFound,
                format!("module not found: {}", n),
            ),
        };
        doo_core::errors::codes::CompilerError::new(code, msg, doo_core::Span::new(0, 0, 0))
    }
}

/// Module linker for multi-file compilation.
///
/// Links multiple LLVM modules together, resolving cross-module references.
///
/// ## Design
///
/// The Doo compiler uses a "merge all" approach:
/// 1. Each source file is compiled to a separate LLVM module
/// 2. Imported functions are declared as external in the importing module
/// 3. All modules are linked together into a single module for final codegen
///
/// This matches the legacy compiler behavior where all imported functions
/// are merged into the AST before MIR generation.
pub struct ModuleLinker<'ctx> {
    /// LLVM context (shared across all modules).
    context: &'ctx Context,
    /// Modules to be linked.
    modules: Vec<Module<'ctx>>,
    /// Symbol table: function_name -> defining_module_index.
    symbol_table: FxHashMap<String, usize>,
    /// Function aliases across all modules.
    function_aliases: FxHashMap<String, String>,
}

impl<'ctx> ModuleLinker<'ctx> {
    /// Create a new module linker.
    pub fn new(context: &'ctx Context) -> Self {
        Self {
            context,
            modules: Vec::new(),
            symbol_table: FxHashMap::default(),
            function_aliases: FxHashMap::default(),
        }
    }

    /// Add a module to be linked.
    ///
    /// Builds the symbol table from the module's function definitions.
    pub fn add_module(&mut self, module: Module<'ctx>) -> Result<(), LinkError> {
        let module_index = self.modules.len();

        // Build symbol table from function definitions
        let mut func_iter = module.get_first_function();
        while let Some(func) = func_iter {
            let name = func.get_name().to_str().unwrap_or("").to_string();

            // Only add functions with bodies (definitions, not declarations)
            if func.count_basic_blocks() > 0 {
                if self.symbol_table.contains_key(&name) {
                    // Allow redefinitions if the existing one is just a declaration
                    let existing_idx = self.symbol_table[&name];
                    if let Some(existing_module) = self.modules.get(existing_idx) {
                        if let Some(existing_func) = existing_module.get_function(&name) {
                            if existing_func.count_basic_blocks() > 0 {
                                return Err(LinkError::DuplicateSymbol(name));
                            }
                        }
                    }
                }
                self.symbol_table.insert(name, module_index);
            }

            func_iter = func.get_next_function();
        }

        self.modules.push(module);
        Ok(())
    }

    /// Add function aliases from a module context.
    pub fn add_aliases(&mut self, aliases: &FxHashMap<String, String>) {
        self.function_aliases.extend(aliases.clone());
    }

    /// Link all modules into a single output module.
    ///
    /// Creates a new module containing all functions from all input modules.
    pub fn link(&mut self, output_name: &str) -> Result<Module<'ctx>, LinkError> {
        if self.modules.is_empty() {
            let output = self.context.create_module(output_name);
            return Ok(output);
        }

        // Take the first module as the base
        let output = self.modules.remove(0);
        let module = output;
        module.set_name(output_name);

        // Link remaining modules into the base
        for additional in self.modules.drain(..) {
            module
                .link_in_module(additional)
                .map_err(|e| LinkError::LinkFailed(e.to_string()))?;
        }

        Ok(module)
    }

    /// Check if a symbol is defined in any module.
    pub fn has_symbol(&self, name: &str) -> bool {
        self.symbol_table.contains_key(name)
    }

    /// Get the module index where a symbol is defined.
    pub fn get_symbol_module(&self, name: &str) -> Option<usize> {
        self.symbol_table.get(name).copied()
    }

    /// Resolve a function name through aliases.
    pub fn resolve_alias(&self, name: &str) -> String {
        self.function_aliases
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    /// Get all defined symbols.
    pub fn symbols(&self) -> impl Iterator<Item = &String> {
        self.symbol_table.keys()
    }

    /// Get the number of modules.
    pub fn module_count(&self) -> usize {
        self.modules.len()
    }
}

// ============================================================================
// Cross-Module Reference Resolver
// ============================================================================

/// Resolves cross-module function references.
///
/// When a function is called from another module, we need to:
/// 1. Declare the function as external in the calling module
/// 2. Track the dependency for linking
pub struct CrossModuleResolver {
    /// External function declarations needed: (calling_module, function_name).
    pending_declarations: Vec<(String, String)>,
    /// Resolved references: function_name -> source_module.
    resolved: FxHashMap<String, String>,
}

impl CrossModuleResolver {
    /// Create a new cross-module resolver.
    pub fn new() -> Self {
        Self {
            pending_declarations: Vec::new(),
            resolved: FxHashMap::default(),
        }
    }

    /// Record a cross-module function call.
    ///
    /// Called when the current module calls a function from another module.
    pub fn record_call(&mut self, current_module: &str, function_name: &str, source_module: &str) {
        if !self.resolved.contains_key(function_name) {
            self.pending_declarations
                .push((current_module.to_string(), function_name.to_string()));
            self.resolved
                .insert(function_name.to_string(), source_module.to_string());
        }
    }

    /// Get pending external declarations for a module.
    pub fn get_pending(&self, module: &str) -> Vec<String> {
        self.pending_declarations
            .iter()
            .filter(|(m, _)| m == module)
            .map(|(_, f)| f.clone())
            .collect()
    }

    /// Get the source module for a function.
    pub fn get_source(&self, function_name: &str) -> Option<&String> {
        self.resolved.get(function_name)
    }

    /// Check if a function has been resolved.
    pub fn is_resolved(&self, function_name: &str) -> bool {
        self.resolved.contains_key(function_name)
    }
}

impl Default for CrossModuleResolver {
    fn default() -> Self {
        Self::new()
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

    // ========================================================================
    // Multi-File Codegen Tests
    // ========================================================================

    #[test]
    fn test_function_alias_registration() {
        let ctx = Context::create();
        let registry = Arc::new(TypeRegistry::new());
        let mut codegen = CodegenContext::new(&ctx, "test", registry);

        // Register an alias: Sq -> Sqrt
        codegen.register_alias("Sq".to_string(), "Sqrt".to_string());

        assert!(codegen.is_alias("Sq"));
        assert!(!codegen.is_alias("Sqrt"));
        assert_eq!(codegen.resolve_function_name("Sq"), "Sqrt");
        assert_eq!(codegen.resolve_function_name("Other"), "Other");
    }

    #[test]
    fn test_multiple_aliases() {
        let ctx = Context::create();
        let registry = Arc::new(TypeRegistry::new());
        let mut codegen = CodegenContext::new(&ctx, "test", registry);

        // Multiple aliases
        codegen.register_alias("Sq".to_string(), "Math::Sqrt".to_string());
        codegen.register_alias("Fl".to_string(), "Math::Floor".to_string());
        codegen.register_alias("Ce".to_string(), "Math::Ceil".to_string());

        assert_eq!(codegen.resolve_function_name("Sq"), "Math::Sqrt");
        assert_eq!(codegen.resolve_function_name("Fl"), "Math::Floor");
        assert_eq!(codegen.resolve_function_name("Ce"), "Math::Ceil");
    }

    #[test]
    fn test_external_function_declaration() {
        let ctx = Context::create();
        let registry = Arc::new(TypeRegistry::new());
        let mut codegen = CodegenContext::new(&ctx, "main", registry);

        let i64_type = codegen.context.i64_type().into();
        let func = codegen.declare_external_function(
            "external_add",
            &[i64_type, i64_type],
            Some(i64_type),
            "math_module",
        );

        assert_eq!(func.get_name().to_str().unwrap(), "external_add");

        // Check module dependencies
        let deps = codegen.get_dependencies();
        assert!(deps.contains_key("math_module"));
        assert!(deps["math_module"].contains(&"external_add".to_string()));
    }

    #[test]
    fn test_external_function_metadata() {
        let ctx = Context::create();
        let registry = Arc::new(TypeRegistry::new());
        let mut codegen = CodegenContext::new(&ctx, "test", registry);

        let ext_fn = ExternalFunction {
            name: "imported_func".to_string(),
            source_module: "other_module".to_string(),
            param_types: vec![builtin::INT, builtin::STR],
            return_type: Some(builtin::BOOL),
            variadic: false,
        };

        codegen.register_external_function(ext_fn);

        assert!(codegen.is_external_function("imported_func"));
        assert!(!codegen.is_external_function("nonexistent"));

        let retrieved = codegen.get_external_function("imported_func").unwrap();
        assert_eq!(retrieved.source_module, "other_module");
        assert_eq!(retrieved.param_types.len(), 2);
    }

    #[test]
    fn test_module_linker_empty() {
        let ctx = Context::create();
        let mut linker = ModuleLinker::new(&ctx);

        let output = linker.link("output").unwrap();
        assert_eq!(output.get_name().to_str().unwrap(), "output");
    }

    #[test]
    fn test_module_linker_single_module() {
        let ctx = Context::create();
        let mut linker = ModuleLinker::new(&ctx);

        // Create a module with a function
        let module = ctx.create_module("module1");
        let i64_type = ctx.i64_type();
        let fn_type = i64_type.fn_type(&[], false);
        let func = module.add_function("get_value", fn_type, None);
        let block = ctx.append_basic_block(func, "entry");
        let builder = ctx.create_builder();
        builder.position_at_end(block);
        builder
            .build_return(Some(&i64_type.const_int(42, false)))
            .unwrap();

        linker.add_module(module).unwrap();
        assert!(linker.has_symbol("get_value"));
        assert_eq!(linker.get_symbol_module("get_value"), Some(0));

        let output = linker.link("linked").unwrap();
        assert!(output.get_function("get_value").is_some());
    }

    #[test]
    fn test_module_linker_multiple_modules() {
        let ctx = Context::create();
        let mut linker = ModuleLinker::new(&ctx);

        // Module 1: defines add_one
        let module1 = ctx.create_module("module1");
        let i64_type = ctx.i64_type();
        let fn_type = i64_type.fn_type(&[i64_type.into()], false);
        let func = module1.add_function("add_one", fn_type, None);
        let block = ctx.append_basic_block(func, "entry");
        let builder = ctx.create_builder();
        builder.position_at_end(block);
        let param = func.get_nth_param(0).unwrap().into_int_value();
        let result = builder
            .build_int_add(param, i64_type.const_int(1, false), "result")
            .unwrap();
        builder.build_return(Some(&result)).unwrap();

        // Module 2: defines double
        let module2 = ctx.create_module("module2");
        let fn_type2 = i64_type.fn_type(&[i64_type.into()], false);
        let func2 = module2.add_function("double", fn_type2, None);
        let block2 = ctx.append_basic_block(func2, "entry");
        let builder2 = ctx.create_builder();
        builder2.position_at_end(block2);
        let param2 = func2.get_nth_param(0).unwrap().into_int_value();
        let result2 = builder2
            .build_int_mul(param2, i64_type.const_int(2, false), "result")
            .unwrap();
        builder2.build_return(Some(&result2)).unwrap();

        linker.add_module(module1).unwrap();
        linker.add_module(module2).unwrap();

        assert!(linker.has_symbol("add_one"));
        assert!(linker.has_symbol("double"));
        assert_eq!(linker.get_symbol_module("add_one"), Some(0));
        assert_eq!(linker.get_symbol_module("double"), Some(1));

        let output = linker.link("linked").unwrap();
        assert!(output.get_function("add_one").is_some());
        assert!(output.get_function("double").is_some());
    }

    #[test]
    fn test_module_linker_duplicate_symbol_error() {
        let ctx = Context::create();
        let mut linker = ModuleLinker::new(&ctx);

        let i64_type = ctx.i64_type();
        let fn_type = i64_type.fn_type(&[], false);

        // Module 1: defines foo
        let module1 = ctx.create_module("module1");
        let func1 = module1.add_function("foo", fn_type, None);
        let block1 = ctx.append_basic_block(func1, "entry");
        let builder1 = ctx.create_builder();
        builder1.position_at_end(block1);
        builder1
            .build_return(Some(&i64_type.const_int(1, false)))
            .unwrap();

        // Module 2: also defines foo (duplicate!)
        let module2 = ctx.create_module("module2");
        let func2 = module2.add_function("foo", fn_type, None);
        let block2 = ctx.append_basic_block(func2, "entry");
        let builder2 = ctx.create_builder();
        builder2.position_at_end(block2);
        builder2
            .build_return(Some(&i64_type.const_int(2, false)))
            .unwrap();

        linker.add_module(module1).unwrap();
        let result = linker.add_module(module2);

        assert!(matches!(result, Err(LinkError::DuplicateSymbol(_))));
    }

    #[test]
    fn test_module_linker_with_aliases() {
        let ctx = Context::create();
        let mut linker = ModuleLinker::new(&ctx);

        let mut aliases = FxHashMap::default();
        aliases.insert("Sq".to_string(), "Math::Sqrt".to_string());
        aliases.insert("Add".to_string(), "Math::Add".to_string());

        linker.add_aliases(&aliases);

        assert_eq!(linker.resolve_alias("Sq"), "Math::Sqrt");
        assert_eq!(linker.resolve_alias("Add"), "Math::Add");
        assert_eq!(linker.resolve_alias("Unknown"), "Unknown");
    }

    #[test]
    fn test_cross_module_resolver() {
        let mut resolver = CrossModuleResolver::new();

        // Record a call from main to Math::Sqrt
        resolver.record_call("main", "Math::Sqrt", "std_math");

        assert!(resolver.is_resolved("Math::Sqrt"));
        assert_eq!(
            resolver.get_source("Math::Sqrt"),
            Some(&"std_math".to_string())
        );

        let pending = resolver.get_pending("main");
        assert!(pending.contains(&"Math::Sqrt".to_string()));
    }

    #[test]
    fn test_cross_module_resolver_multiple_modules() {
        let mut resolver = CrossModuleResolver::new();

        // Multiple modules calling various functions
        resolver.record_call("main", "Math::Sqrt", "std_math");
        resolver.record_call("main", "File::Read", "std_file");
        resolver.record_call("handlers", "Http::Get", "std_http");

        // Check pending for each module
        let main_pending = resolver.get_pending("main");
        assert_eq!(main_pending.len(), 2);
        assert!(main_pending.contains(&"Math::Sqrt".to_string()));
        assert!(main_pending.contains(&"File::Read".to_string()));

        let handlers_pending = resolver.get_pending("handlers");
        assert_eq!(handlers_pending.len(), 1);
        assert!(handlers_pending.contains(&"Http::Get".to_string()));

        // Check sources
        assert_eq!(
            resolver.get_source("Math::Sqrt"),
            Some(&"std_math".to_string())
        );
        assert_eq!(
            resolver.get_source("Http::Get"),
            Some(&"std_http".to_string())
        );
    }

    #[test]
    fn test_declaration_caching() {
        let ctx = Context::create();
        let registry = Arc::new(TypeRegistry::new());
        let mut codegen = CodegenContext::new(&ctx, "test", registry);

        let i64_type = codegen.context.i64_type().into();

        // Declare the same function twice
        let func1 = codegen.declare_external_function(
            "cached_func",
            &[i64_type],
            Some(i64_type),
            "other_module",
        );
        let func2 = codegen.declare_external_function(
            "cached_func",
            &[i64_type],
            Some(i64_type),
            "other_module",
        );

        // Should return the same function (cached)
        assert_eq!(func1, func2);
    }
}
