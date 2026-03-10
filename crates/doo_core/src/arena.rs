//! Arena Allocation — fast, bump-allocated memory for compiler data structures.
//!
//! Provides a compiler-wide arena that allocates nodes contiguously in memory
//! (cache-friendly) and frees all memory at once when the arena is dropped.
//!
//! ## Usage
//!
//! ```ignore
//! use doo_core::arena::CompilerArena;
//!
//! let arena = CompilerArena::new();
//! let node = arena.alloc(MyNode { ... });  // Returns &MyNode
//! let slice = arena.alloc_slice(&[1, 2, 3]);
//! // All memory freed automatically when `arena` is dropped.
//! ```
//!
//! ## Design
//!
//! - One `CompilerArena` per compilation unit (file or program).
//! - AST, HIR, and MIR nodes can all be allocated from the arena.
//! - Returned references have the arena's lifetime — no use-after-free.

use bumpalo::Bump;

/// A bump-allocating arena for compiler data structures.
///
/// Allocations are contiguous in memory (excellent cache locality).
/// All memory is freed in one operation when the arena is dropped.
pub struct CompilerArena {
    bump: Bump,
}

impl Default for CompilerArena {
    fn default() -> Self {
        Self::new()
    }
}

impl CompilerArena {
    /// Create a new arena with default initial capacity.
    pub fn new() -> Self {
        Self {
            bump: Bump::new(),
        }
    }

    /// Create a new arena with a specific initial capacity (bytes).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bump: Bump::with_capacity(capacity),
        }
    }

    /// Allocate a value in the arena, returning a reference with the arena's lifetime.
    #[inline]
    pub fn alloc<T>(&self, val: T) -> &T {
        self.bump.alloc(val)
    }

    /// Allocate a mutable value in the arena.
    #[inline]
    pub fn alloc_mut<T>(&self, val: T) -> &mut T {
        self.bump.alloc(val)
    }

    /// Allocate a string slice in the arena.
    #[inline]
    pub fn alloc_str(&self, s: &str) -> &str {
        self.bump.alloc_str(s)
    }

    /// Allocate a slice by copying from an existing slice.
    #[inline]
    pub fn alloc_slice_copy<T: Copy>(&self, src: &[T]) -> &[T] {
        self.bump.alloc_slice_copy(src)
    }

    /// Allocate a slice by cloning from an existing slice.
    #[inline]
    pub fn alloc_slice_clone<T: Clone>(&self, src: &[T]) -> &[T] {
        self.bump.alloc_slice_clone(src)
    }

    /// Create a `Vec`-like builder in the arena.
    /// Useful for building collections incrementally.
    #[inline]
    pub fn alloc_vec<T>(&self) -> bumpalo::collections::Vec<'_, T> {
        bumpalo::collections::Vec::new_in(&self.bump)
    }

    /// How many bytes have been allocated so far.
    pub fn allocated_bytes(&self) -> usize {
        self.bump.allocated_bytes()
    }

    /// Reset the arena, freeing all allocated memory.
    /// After calling this, all previously allocated references are invalidated.
    pub fn reset(&mut self) {
        self.bump.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_alloc() {
        let arena = CompilerArena::new();
        let x = arena.alloc(42i32);
        assert_eq!(*x, 42);
    }

    #[test]
    fn test_alloc_str() {
        let arena = CompilerArena::new();
        let s = arena.alloc_str("hello");
        assert_eq!(s, "hello");
    }

    #[test]
    fn test_alloc_slice() {
        let arena = CompilerArena::new();
        let slice = arena.alloc_slice_copy(&[1, 2, 3]);
        assert_eq!(slice, &[1, 2, 3]);
    }

    #[test]
    fn test_arena_vec() {
        let arena = CompilerArena::new();
        let mut v = arena.alloc_vec::<i32>();
        v.push(10);
        v.push(20);
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn test_with_capacity() {
        let arena = CompilerArena::with_capacity(1024);
        let _ = arena.alloc(1u64);
        assert!(arena.allocated_bytes() > 0);
    }
}
