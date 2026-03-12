//! Memory Management Module
//!
//! Centralized memory operations for the Doo ownership model.
//! No reference counting - pure ownership with Move/Clone/Drop.

use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::AddressSpace;
use doo_core::constants::ffi_names;

/// Memory manager for ownership-based memory.
pub struct MemoryManager {
    /// Whether to use custom allocator
    use_custom_allocator: bool,
}

impl MemoryManager {
    /// Create a new memory manager.
    pub fn new() -> Self {
        Self {
            use_custom_allocator: false,
        }
    }

    /// Declare memory functions in the module.
    pub fn declare_runtime_functions<'ctx>(&self, context: &'ctx Context, module: &Module<'ctx>) {
        // Use i8.ptr_type() for pointer types (modern inkwell API)
        let ptr_type = context.i8_type().ptr_type(AddressSpace::default());
        let i64_type = context.i64_type();
        let void_type = context.void_type();
        
        // doo_alloc: (size: i64) -> ptr
        let alloc_type = ptr_type.fn_type(&[i64_type.into()], false);
        module.add_function(ffi_names::DOO_ALLOC, alloc_type, None);

        // doo_realloc: (ptr, new_size: i64) -> ptr
        let realloc_type = ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
        module.add_function(ffi_names::DOO_REALLOC, realloc_type, None);

        // doo_free: (ptr) -> void
        let free_type = void_type.fn_type(&[ptr_type.into()], false);
        module.add_function(ffi_names::DOO_FREE, free_type, None);

        // doo_clone: (ptr, size: i64) -> ptr
        let clone_type = ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
        module.add_function(ffi_names::DOO_CLONE, clone_type, None);

        // doo_memcpy: (dest, src, size: i64) -> void
        let memcpy_type = void_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false);
        module.add_function(ffi_names::DOO_MEMCPY, memcpy_type, None);

        // doo_zero: (ptr, size: i64) -> void
        let zero_type = void_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
        module.add_function(ffi_names::DOO_ZERO, zero_type, None);

        // printf for debugging
        let printf_type = context.i32_type().fn_type(&[ptr_type.into()], true);
        module.add_function(ffi_names::PRINTF, printf_type, None);
    }
}

impl Default for MemoryManager {
    fn default() -> Self {
        Self::new()
    }
}
