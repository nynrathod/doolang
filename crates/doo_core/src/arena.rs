//! Arena Allocation — fast, bump-allocated memory for compiler data structures.
//!
//! Provides two arena types:
//! - [`Arena<T>`]: Type-safe typed arena for allocating values of a single type.
//! - [`CompilerArena`]: Type-erased arena for mixed-type allocation (strings,
//!   slices of different types, etc.).
//!
//! Both wrap [`bumpalo::Bump`] for contiguous, cache-friendly allocation with
//! O(1) alloc and bulk-free semantics.

use bumpalo::Bump;

// ============================================================================
// Arena Statistics
// ============================================================================

/// Arena allocation statistics for observability and debugging.
#[derive(Debug, Clone, Copy, Default)]
pub struct ArenaStats {
    /// Total bytes allocated across all chunks.
    pub bytes_allocated: usize,
    /// Number of allocation chunks.
    pub chunk_count: usize,
}

// ============================================================================
// Typed Arena<T>
// ============================================================================

/// A typed bump-allocating arena for compiler data structures.
///
/// Each `Arena<T>` allocates values of type `T` contiguously in memory,
/// providing excellent cache locality. All memory is freed in one operation
/// when the arena is dropped.
pub struct Arena<T> {
    bump: Bump,
    _marker: std::marker::PhantomData<T>,
}

// SAFETY: Arena<T> is Send/Sync if T is, because the Bump allocator is
// thread-safe for single-threaded access patterns (compiler is single-threaded
// per compilation unit). The PhantomData propagates T's bounds.
unsafe impl<T: Send> Send for Arena<T> {}
unsafe impl<T: Sync> Sync for Arena<T> {}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> std::fmt::Debug for Arena<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Arena")
            .field("type", &std::any::type_name::<T>())
            .field("bytes_allocated", &self.bump.allocated_bytes())
            .finish()
    }
}

impl<T> Arena<T> {
    /// Create a new arena with default initial capacity (4096 bytes).
    #[inline]
    pub fn new() -> Self {
        Self {
            bump: Bump::with_capacity(4096),
            _marker: std::marker::PhantomData,
        }
    }

    /// Create a new arena with a specific initial capacity in bytes.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bump: Bump::with_capacity(capacity),
            _marker: std::marker::PhantomData,
        }
    }

    /// Allocate a value in the arena, returning a mutable reference.
    #[inline]
    pub fn alloc(&self, val: T) -> &mut T {
        self.bump.alloc(val)
    }

    /// Allocate a slice by copying from an existing slice.
    #[inline]
    pub fn alloc_slice_copy(&self, src: &[T]) -> &mut [T]
    where
        T: Copy,
    {
        self.bump.alloc_slice_copy(src)
    }

    /// Allocate a slice by cloning from an existing slice.
    #[inline]
    pub fn alloc_slice_clone(&self, src: &[T]) -> &mut [T]
    where
        T: Clone,
    {
        self.bump.alloc_slice_clone(src)
    }

    /// Allocate a slice from an iterator.
    pub fn alloc_iter<I>(&self, iter: I) -> &[T]
    where
        I: IntoIterator<Item = T>,
    {
        let iter = iter.into_iter();
        let (lower, _) = iter.size_hint();
        let mut vec = bumpalo::collections::Vec::with_capacity_in(lower, &self.bump);
        vec.extend(iter);
        vec.into_bump_slice()
    }

    /// Create a `Vec`-like builder in the arena for incremental construction.
    #[inline]
    pub fn alloc_vec(&self) -> bumpalo::collections::Vec<'_, T> {
        bumpalo::collections::Vec::new_in(&self.bump)
    }

    /// Total bytes allocated so far.
    #[inline]
    pub fn allocated_bytes(&self) -> usize {
        self.bump.allocated_bytes()
    }

    /// Get arena allocation statistics.
    pub fn stats(&self) -> ArenaStats {
        ArenaStats {
            bytes_allocated: self.bump.allocated_bytes(),
            chunk_count: 1, // Bumpalo manages chunks internally
        }
    }

    /// Reset the arena, freeing all allocated memory and reusing chunks.
    pub fn reset(&mut self) {
        self.bump.reset();
    }

    /// Get a reference to the underlying bump allocator.
    #[inline]
    pub fn raw(&self) -> &Bump {
        &self.bump
    }
}

// ============================================================================
// Type-Erased CompilerArena
// ============================================================================

/// A type-erased bump-allocating arena for general-purpose allocation.
///
/// Unlike [`Arena<T>`], `CompilerArena` can allocate values of any type,
/// strings, and slices in a single arena. This is the primary arena used
/// for AST/HIR/THIR/MIR construction where mixed types are common.
pub struct CompilerArena {
    bump: Bump,
}

impl Default for CompilerArena {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CompilerArena {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompilerArena")
            .field("bytes_allocated", &self.bump.allocated_bytes())
            .finish()
    }
}

impl CompilerArena {
    /// Create a new arena with default initial capacity (4096 bytes).
    #[inline]
    pub fn new() -> Self {
        Self {
            bump: Bump::with_capacity(4096),
        }
    }

    /// Create a new arena with a specific initial capacity in bytes.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bump: Bump::with_capacity(capacity),
        }
    }

    /// Allocate a value of any type, returning an immutable reference.
    #[inline]
    pub fn alloc<T>(&self, val: T) -> &T {
        self.bump.alloc(val)
    }

    /// Allocate a value of any type, returning a mutable reference.
    #[inline]
    pub fn alloc_mut<T>(&self, val: T) -> &mut T {
        self.bump.alloc(val)
    }

    /// Allocate a string slice in the arena.
    #[inline]
    pub fn alloc_str(&self, s: &str) -> &str {
        self.bump.alloc_str(s)
    }

    /// Allocate a slice by copying from an existing slice (requires `T: Copy`).
    #[inline]
    pub fn alloc_slice_copy<T: Copy>(&self, src: &[T]) -> &[T] {
        self.bump.alloc_slice_copy(src)
    }

    /// Allocate a slice by cloning from an existing slice (requires `T: Clone`).
    #[inline]
    pub fn alloc_slice_clone<T: Clone>(&self, src: &[T]) -> &[T] {
        self.bump.alloc_slice_clone(src)
    }

    /// Create a `Vec`-like builder in the arena for incremental construction.
    #[inline]
    pub fn alloc_vec<T>(&self) -> bumpalo::collections::Vec<'_, T> {
        bumpalo::collections::Vec::new_in(&self.bump)
    }

    /// Create a `String`-like builder in the arena for incremental string construction.
    #[inline]
    pub fn alloc_string(&self) -> bumpalo::collections::String<'_> {
        bumpalo::collections::String::new_in(&self.bump)
    }

    /// Total bytes allocated so far.
    #[inline]
    pub fn allocated_bytes(&self) -> usize {
        self.bump.allocated_bytes()
    }

    /// Get arena allocation statistics.
    pub fn stats(&self) -> ArenaStats {
        ArenaStats {
            bytes_allocated: self.bump.allocated_bytes(),
            chunk_count: 1,
        }
    }

    /// Reset the arena, freeing all allocated memory and reusing chunks.
    pub fn reset(&mut self) {
        self.bump.reset();
    }

    /// Get a reference to the underlying bump allocator.
    #[inline]
    pub fn raw(&self) -> &Bump {
        &self.bump
    }
}
