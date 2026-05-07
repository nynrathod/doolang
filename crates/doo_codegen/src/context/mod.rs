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
//! - `ModuleLinker`: Utility to link multiple LLVM modules together (see `linker` module)

mod locals;
mod metadata;
mod type_cache;

use doo_core::doo_debug;
use doo_core::types::{TypeId, TypeKind, TypeRegistry};
use inkwell::attributes::AttributeLoc;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::targets::{CodeModel, InitializationConfig, RelocMode, Target, TargetMachine};
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

// ============================================================================
// Unified Value Metadata (C04)
// ============================================================================

/// Unified metadata for a variable/temporary value.
///
/// Consolidates information from `temp_struct_types`, `variable_types`,
/// and `struct_metadata` into a single queryable record. New code should
/// use `CodegenContext::value_info()` to query this instead of accessing
/// the three separate maps directly.
#[derive(Debug, Clone)]
pub struct ValueInfo {
    /// The Doo type of this variable (from `variable_types`).
    pub type_id: Option<TypeId>,
    /// The struct type name if this variable holds a struct instance
    /// (from `temp_struct_types`).
    pub struct_name: Option<String>,
    /// The type kind (resolved from type_id via type registry).
    pub type_kind: Option<TypeKind>,
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
    /// Struct metadata: struct_name -> field names (in declaration order).
    /// Used for looking up field indices by name.
    pub struct_metadata: FxHashMap<String, Vec<String>>,

    /// Struct field reorder map: struct_name -> logical_to_physical index mapping.
    /// When fields are reordered by alignment (P06 optimization), this maps
    /// the declaration-order index to the physical LLVM struct index.
    /// If a struct is NOT in this map, indices are identity (no reorder).
    pub struct_field_remap: FxHashMap<String, Vec<usize>>,

    /// Struct field decorators: struct_name -> Vec<(field_name, Vec<decorator_string>)>.
    /// Populated from MIR struct definitions which carry decorator info from HIR.
    pub struct_field_decorators: FxHashMap<String, Vec<(String, Vec<String>)>>,

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

    /// Whether the current function has an error type (returns Result { i64, ptr }).
    pub current_function_has_error_type: bool,

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
    // FFI Type Signature Registry (Package-Ready — Single Source of Truth)
    // ========================================================================
    /// FFI type signatures: ffi_symbol_name -> (param_type_ids, return_type_id, is_result).
    /// Populated from MIR FfiLinkage which carries the full Doo declaration types.
    /// Used by `declare_ffi_function` to generate correct LLVM signatures
    /// WITHOUT the hardcoded `get_ffi_signature()` match table.
    /// Third-party packages get correct signatures automatically because
    /// their `@extern` declarations flow types through this path.
    pub ffi_type_signatures: FxHashMap<String, (Vec<TypeId>, Option<TypeId>, bool)>,

    // ========================================================================
    // FFI Library Map (Package Dispatch — Symbol → Library)
    // ========================================================================
    /// Reverse mapping: external_symbol → library_name.
    /// Used by the package dispatch system to route FFI calls to the
    /// correct package hooks (http, websocket, database, generic).
    /// Populated from @extern declarations: `@extern("doo_http", "server_new")`
    /// → maps "doo_http_server_new" → "doo_http".
    pub ffi_library_map: FxHashMap<String, String>,

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

    // ========================================================================
    // RBAC Policy Registry (for policy metadata emission)
    // ========================================================================
    /// RBAC policies: struct_name -> policy_json.
    /// Populated from MIR policy definitions.
    /// Emitted as `doo_http_register_policy(struct_name, policy_json)` calls
    /// when an auth/crud endpoint is registered for the struct.
    pub rbac_policies: FxHashMap<String, String>,

    /// Enum inheritance: enum_name -> Vec<(variant_name, Vec<inherited_variant>)>.
    /// Populated from enum variants with `@inherits(...)` decorators.
    /// Emitted as `doo_http_register_role_hierarchy(enum_name, hierarchy_json)`.
    pub enum_inheritance: FxHashMap<String, Vec<(String, Vec<String>)>>,

    // ========================================================================
    // Static Globals (OnceLock-style runtime globals)
    // ========================================================================
    /// Set of static global names (declared with `static Name: Type`).
    /// Used to detect assignments to statics (→ __static_set_X) and reads
    /// from statics (→ __static_get_X).
    pub static_globals: FxHashMap<String, TypeId>,
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

        // Set target triple and data layout on the module at creation time.
        // This ensures all struct layout computations (GEPs, struct_gep, size_of)
        // use the correct target-specific alignment from the start.
        // Without this, LLVM defaults to i64 ABI-align=4, but the backend uses
        // the target's i64 ABI-align=8, causing mismatched offsets in mixed-size
        // structs like {i8, i64} (Bool:Int map entries).
        let _ = Target::initialize_native(&InitializationConfig::default());
        let triple = TargetMachine::get_default_triple();
        module.set_triple(&triple);
        if let Ok(target) = Target::from_triple(&triple) {
            if let Some(tm) = target.create_target_machine(
                &triple,
                "generic",
                "",
                inkwell::OptimizationLevel::None,
                RelocMode::Default,
                CodeModel::Default,
            ) {
                module.set_data_layout(&tm.get_target_data().get_data_layout());
            }
        }

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
            struct_field_remap: FxHashMap::default(),
            struct_field_decorators: FxHashMap::default(),
            temp_struct_types: FxHashMap::default(),
            variable_types: FxHashMap::default(),
            function_param_types: FxHashMap::default(),
            function_return_types: FxHashMap::default(),
            function_error_types: FxHashMap::default(),
            current_function_return_type: None,
            current_function_has_error_type: false,
            borrow_origins: FxHashMap::default(),
            is_closure_function: false,
            ffi_symbols: FxHashMap::default(),
            ffi_type_signatures: FxHashMap::default(),
            ffi_library_map: FxHashMap::default(),
            array_element_types: FxHashMap::default(),
            array_element_temps: FxHashMap::default(),
            has_async: false,
            rbac_policies: FxHashMap::default(),
            enum_inheritance: FxHashMap::default(),
            static_globals: FxHashMap::default(),
        }
    }

    pub fn get_type_kind(&self, type_id: TypeId) -> Option<TypeKind> {
        let result = self
            .type_registry
            .get(type_id)
            .map(|info| info.kind.clone());
        if std::env::var(doo_core::constants::env_vars::DOO_DEBUG_TYPES).is_ok() {
            doo_debug!("TYPES", "get_type_kind({:?}) = {:?}", type_id, result);
        }
        result
    }

    /// C04: Unified value metadata query.
    ///
    /// Returns consolidated metadata for a variable by querying all three
    /// internal tracking maps (`variable_types`, `temp_struct_types`, type registry).
    /// New code should prefer this over directly accessing the individual maps.
    pub fn value_info(&self, var_name: &str) -> ValueInfo {
        let type_id = self.variable_types.get(var_name).copied();
        let struct_name = self.temp_struct_types.get(var_name).cloned().or_else(|| {
            // Derive struct name from type_id if not in temp_struct_types
            type_id.and_then(|tid| self.get_struct_name_from_type_id(tid))
        });
        let type_kind = type_id.and_then(|tid| self.get_type_kind(tid));
        ValueInfo {
            type_id,
            struct_name,
            type_kind,
        }
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
    // FFI Type Signature Registry (Package-Ready)
    // ========================================================================

    /// Register FFI type signature from MIR FfiLinkage.
    /// This is populated during function declaration from the Doo declaration types.
    /// Third-party packages automatically get correct signatures through this path.
    pub fn register_ffi_type_signature(
        &mut self,
        symbol: &str,
        param_types: Vec<TypeId>,
        return_type: Option<TypeId>,
        is_result: bool,
    ) {
        self.ffi_type_signatures
            .insert(symbol.to_string(), (param_types, return_type, is_result));
    }

    /// Get FFI type signature for a symbol (if registered from Doo declarations).
    /// Returns Some((param_types, return_type, is_result)) if found.
    pub fn get_ffi_type_signature(
        &self,
        symbol: &str,
    ) -> Option<&(Vec<TypeId>, Option<TypeId>, bool)> {
        self.ffi_type_signatures.get(symbol)
    }

    // ========================================================================
    // FFI Library Map (Package Dispatch)
    // ========================================================================

    /// Register the library name for an FFI symbol.
    /// Maps external_symbol → library_name for package dispatch.
    pub fn register_ffi_library(&mut self, symbol: &str, library: &str) {
        self.ffi_library_map
            .insert(symbol.to_string(), library.to_string());
    }

    /// Get the library name for an FFI symbol.
    /// Returns the library from @extern declaration, or None for C stdlib/unknown.
    pub fn get_ffi_library(&self, symbol: &str) -> Option<&str> {
        self.ffi_library_map.get(symbol).map(|s| s.as_str())
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

        // Prevent LLVM's TailCallElim pass from adding `tail call` annotations.
        // On Windows x64, functions returning structs >8 bytes use sret (implicit
        // stack pointer). Tail calls can reuse the caller's frame while sret still
        // points into it, corrupting the return address → DEP violation.
        let no_tail = self
            .context
            .create_string_attribute("disable-tail-calls", "true");
        func.add_attribute(AttributeLoc::Function, no_tail);

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

    /// Get the Doo bool type (i8 for C ABI compatibility).
    /// NOTE: For LLVM comparison results (i1), use `self.context.bool_type()` directly.
    pub fn bool_type(&self) -> inkwell::types::IntType<'ctx> {
        self.context.i8_type()
    }

    /// Get the f64 type.
    pub fn f64_type(&self) -> inkwell::types::FloatType<'ctx> {
        self.context.f64_type()
    }

    /// Get the pointer type.
    pub fn ptr_type(&self) -> inkwell::types::PointerType<'ctx> {
        self.context.i8_type().ptr_type(AddressSpace::default())
    }

    /// Get the current function from the builder's insert block.
    /// Returns None if the builder has no insert point or the block has no parent function.
    /// Use this instead of `.get_insert_block().unwrap().get_parent().unwrap()`.
    pub fn current_function(&self) -> Option<FunctionValue<'ctx>> {
        self.builder.get_insert_block()?.get_parent()
    }

    /// Create an i64 constant.
    pub fn const_i64(&self, val: i64) -> inkwell::values::IntValue<'ctx> {
        self.context.i64_type().const_int(val as u64, true)
    }

    /// Create an f64 constant.
    pub fn const_f64(&self, val: f64) -> inkwell::values::FloatValue<'ctx> {
        self.context.f64_type().const_float(val)
    }

    /// Create a bool constant (i8 for C ABI compatibility).
    pub fn const_bool(&self, val: bool) -> inkwell::values::IntValue<'ctx> {
        self.context.i8_type().const_int(val as u64, false)
    }

    /// Create a global string constant.
    pub fn const_string(&self, val: &str) -> PointerValue<'ctx> {
        let global = self
            .builder
            .build_global_string_ptr(val, "str")
            .expect("ICE: failed to create global string constant");
        global.as_pointer_value()
    }

    // ========================================================================
    // Opaque Abort Helper (prevents LLVM noreturn inference)
    // ========================================================================

    /// Get or create `@__doo_abort(i32)` — an opaque wrapper around `exit()`.
    ///
    /// LLVM recognizes `exit()` as a standard C library function and infers it
    /// is `noreturn`. When `exit()` appears inside a loop body (e.g. in array
    /// bounds checks), LLVM uses this to prove the panic path never returns,
    /// deduces the loop condition is always true, and removes the loop exit
    /// branch entirely — causing infinite iteration and crashes.
    ///
    /// `__doo_abort` is marked `noinline optnone`, making it completely opaque
    /// to LLVM's interprocedural analysis. LLVM cannot see through it, cannot
    /// infer it is noreturn, and therefore preserves loop exit conditions.
    pub fn get_or_create_doo_abort(&self) -> FunctionValue<'ctx> {
        const DOO_ABORT: &str = "__doo_abort";

        if let Some(f) = self.module.get_function(DOO_ABORT) {
            return f;
        }

        // Create: define void @__doo_abort(i32 %code) noinline optnone { call exit(%code); unreachable }
        let i32_type = self.context.i32_type();
        let fn_type = self.context.void_type().fn_type(&[i32_type.into()], false);
        let func = self.module.add_function(DOO_ABORT, fn_type, None);

        // Mark noinline + optnone to prevent LLVM from analyzing the body
        let noinline = inkwell::attributes::Attribute::get_named_enum_kind_id("noinline");
        let optnone = inkwell::attributes::Attribute::get_named_enum_kind_id("optnone");
        func.add_attribute(
            AttributeLoc::Function,
            self.context.create_enum_attribute(noinline, 0),
        );
        func.add_attribute(
            AttributeLoc::Function,
            self.context.create_enum_attribute(optnone, 0),
        );

        // Build the body: call exit(%code), then unreachable
        let entry = self.context.append_basic_block(func, "entry");
        // Save current position
        let saved_block = self.builder.get_insert_block();

        self.builder.position_at_end(entry);

        // Get or declare exit
        let exit_type = self.context.void_type().fn_type(&[i32_type.into()], false);
        let exit_fn = self
            .module
            .get_function("exit")
            .unwrap_or_else(|| self.module.add_function("exit", exit_type, None));

        let code_param = func.get_first_param().unwrap();
        let _ = self
            .builder
            .build_call(exit_fn, &[code_param.into()], "do_exit");
        let _ = self.builder.build_unreachable();

        // Restore builder position
        if let Some(bb) = saved_block {
            self.builder.position_at_end(bb);
        }

        func
    }

    /// Get or create the `__doo_opaque_ptr` helper function.
    ///
    /// This is a `noinline optnone` identity function for pointers:
    ///   define ptr @__doo_opaque_ptr(ptr %p) noinline optnone { ret ptr %p }
    ///
    /// Purpose: When clone functions produce a null PHI (for optional/error fields
    /// where null is a valid value), subsequent field accesses on the null result
    /// are undefined behavior. LLVM O3 exploits this UB to remove loop exit
    /// conditions and return instructions, causing crashes.
    ///
    /// By passing clone results through this opaque function, LLVM cannot see
    /// through it and therefore cannot prove the pointer is null. This prevents
    /// the UB exploitation chain while preserving correct null handling semantics.
    pub fn get_or_create_doo_opaque_ptr(&self) -> FunctionValue<'ctx> {
        const DOO_OPAQUE_PTR: &str = "__doo_opaque_ptr";

        if let Some(f) = self.module.get_function(DOO_OPAQUE_PTR) {
            return f;
        }

        let ptr_type = self.ptr_type();
        let fn_type = ptr_type.fn_type(&[ptr_type.into()], false);
        let func = self.module.add_function(DOO_OPAQUE_PTR, fn_type, None);

        // Mark noinline + optnone to prevent LLVM from seeing through it
        let noinline = inkwell::attributes::Attribute::get_named_enum_kind_id("noinline");
        let optnone = inkwell::attributes::Attribute::get_named_enum_kind_id("optnone");
        func.add_attribute(
            AttributeLoc::Function,
            self.context.create_enum_attribute(noinline, 0),
        );
        func.add_attribute(
            AttributeLoc::Function,
            self.context.create_enum_attribute(optnone, 0),
        );

        // Build the body: just return the argument
        let entry = self.context.append_basic_block(func, "entry");
        let saved_block = self.builder.get_insert_block();

        self.builder.position_at_end(entry);
        let param = func.get_first_param().unwrap().into_pointer_value();
        let _ = self.builder.build_return(Some(&param));

        // Restore builder position
        if let Some(bb) = saved_block {
            self.builder.position_at_end(bb);
        }

        func
    }

    // ========================================================================
    // Entry-Block Alloca Helper
    // ========================================================================

    /// Emit an `alloca` in the current function's entry block.
    ///
    /// On Windows x64, allocas outside the entry block are "dynamic allocas"
    /// that require runtime stack adjustments. These can interfere with SEH
    /// unwind tables and corrupt the stack frame, leading to DEP violations.
    /// LLVM best practice: ALL allocas must be in the entry block so they
    /// become part of the fixed stack frame allocated in the function prologue.
    ///
    /// This helper saves the current builder position, inserts the alloca at
    /// the start of the entry block, then restores the builder position.
    pub fn alloca_in_entry_block<T: BasicType<'ctx>>(
        &self,
        ty: T,
        name: &str,
    ) -> Option<PointerValue<'ctx>> {
        let current_block = self.builder.get_insert_block()?;
        let current_fn = current_block.get_parent()?;
        let entry_block = current_fn.get_first_basic_block()?;

        // Position at the start of the entry block (before all existing instructions)
        if let Some(first_instr) = entry_block.get_first_instruction() {
            self.builder.position_before(&first_instr);
        } else {
            self.builder.position_at_end(entry_block);
        }

        let basic_ty: BasicTypeEnum<'ctx> = ty.as_basic_type_enum();
        let alloca = self.builder.build_alloca(basic_ty, name).ok()?;

        // Zero-initialize the alloca to prevent reading garbage from the stack.
        // Uninitialized allocas can contain arbitrary values (including -1/0xFFFFFFFFFFFFFFFF)
        // that cause crashes when treated as pointers.
        let zero: Option<BasicValueEnum<'ctx>> = match basic_ty {
            BasicTypeEnum::IntType(t) => Some(t.const_zero().into()),
            BasicTypeEnum::FloatType(t) => Some(t.const_zero().into()),
            BasicTypeEnum::PointerType(t) => Some(t.const_null().into()),
            BasicTypeEnum::StructType(t) => Some(t.const_zero().into()),
            BasicTypeEnum::ArrayType(t) => Some(t.const_zero().into()),
            BasicTypeEnum::VectorType(t) => Some(t.const_zero().into()),
            _ => None,
        };
        if let Some(zero_val) = zero {
            let _ = self.builder.build_store(alloca, zero_val);
        }

        // Restore the builder to the original position
        self.builder.position_at_end(current_block);

        Some(alloca)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linker::{CrossModuleResolver, LinkError, ModuleLinker};
    use doo_core::types::builtin;
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
