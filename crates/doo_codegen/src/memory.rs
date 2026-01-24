//! Memory Management Module
//!
//! Centralized memory operations for the Doo ownership model.
//! No reference counting - pure ownership with Move/Clone/Drop.

use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::AddressSpace;

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
        module.add_function("doo_alloc", alloc_type, None);

        // doo_realloc: (ptr, new_size: i64) -> ptr
        let realloc_type = ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
        module.add_function("doo_realloc", realloc_type, None);

        // doo_free: (ptr) -> void
        let free_type = void_type.fn_type(&[ptr_type.into()], false);
        module.add_function("doo_free", free_type, None);

        // doo_clone: (ptr, size: i64) -> ptr
        let clone_type = ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
        module.add_function("doo_clone", clone_type, None);

        // doo_memcpy: (dest, src, size: i64) -> void
        let memcpy_type = void_type.fn_type(&[ptr_type.into(), ptr_type.into(), i64_type.into()], false);
        module.add_function("doo_memcpy", memcpy_type, None);

        // doo_zero: (ptr, size: i64) -> void
        let zero_type = void_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
        module.add_function("doo_zero", zero_type, None);

        // printf for debugging
        let printf_type = context.i32_type().fn_type(&[ptr_type.into()], true);
        module.add_function("printf", printf_type, None);
    }
}

impl Default for MemoryManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Ownership decision for a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipDecision {
    /// Move: transfer ownership (zero cost)
    Move,
    /// Copy: bitwise copy (for primitives)
    Copy,
    /// Clone: deep copy (for non-primitives with future uses)
    Clone,
    /// Borrow: create reference (for function args)
    Borrow { mutable: bool },
}

impl OwnershipDecision {
    /// Get the decision based on type and usage.
    pub fn decide(is_primitive: bool, future_uses: usize) -> Self {
        if future_uses == 0 {
            Self::Move
        } else if is_primitive {
            Self::Copy
        } else {
            Self::Clone
        }
    }
}
