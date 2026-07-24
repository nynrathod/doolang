//! DooStr — a fat-pointer string type for FFI and runtime string representation.
//!
//! `DooStr` is a compact string handle (`ptr + len`) that points to UTF-8 data
//! on the heap. It is the runtime representation of Doo's `Str` type, used
//! for FFI boundaries and LLVM codegen layout.

use std::fmt;

// ============================================================================
// DooStr
// ============================================================================

/// A fat-pointer string pointing to UTF-8 data.
///
/// This is the runtime representation of Doo's `Str` type. The pointer and
/// length are stored inline (12 bytes on 64-bit with `#[repr(C)]`), while
/// the actual string data lives on the heap (or in an arena, or in static
/// memory for string literals).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DooStr {
    /// Pointer to UTF-8 string data.
    pub ptr: *const u8,
    /// Length of the string data in bytes (not including any null terminator).
    pub len: u32,
}

// DooStr is Send because the pointer points to data that can be safely
// accessed from any thread (no interior mutability, no shared mutable state).
unsafe impl Send for DooStr {}

// DooStr is Sync because it provides shared read-only access to the string
// data (like &str). Multiple threads can safely read the same DooStr.
unsafe impl Sync for DooStr {}

impl DooStr {
    /// Create a `DooStr` from a string slice.
    #[inline]
    pub fn from_str(s: &str) -> Self {
        Self {
            ptr: s.as_ptr(),
            len: s.len() as u32,
        }
    }

    /// Create a `DooStr` from an arena-allocated string.
    #[inline]
    pub fn from_arena_str(s: &str) -> Self {
        Self::from_str(s)
    }

    /// Create a `DooStr` from a raw pointer and length.
    #[inline]
    pub const unsafe fn from_raw(ptr: *const u8, len: u32) -> Self {
        Self { ptr, len }
    }

    /// Convert to a string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        if self.ptr.is_null() || self.len == 0 {
            return "";
        }
        unsafe {
            let bytes = std::slice::from_raw_parts(self.ptr, self.len as usize);
            std::str::from_utf8_unchecked(bytes)
        }
    }

    /// Convert to a byte slice.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        if self.ptr.is_null() || self.len == 0 {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(self.ptr, self.len as usize) }
    }

    /// Check if the string is empty.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Get the length in bytes.
    #[inline]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Create an empty `DooStr`.
    #[inline]
    pub const fn empty() -> Self {
        Self {
            ptr: std::ptr::null(),
            len: 0,
        }
    }

    /// Check if the pointer is null.
    #[inline]
    pub const fn is_null(&self) -> bool {
        self.ptr.is_null()
    }

    /// Get the raw pointer.
    #[inline]
    pub const fn ptr(&self) -> *const u8 {
        self.ptr
    }

    /// Get the raw length as u32.
    #[inline]
    pub const fn raw_len(&self) -> u32 {
        self.len
    }
}

impl Default for DooStr {
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

impl fmt::Debug for DooStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for DooStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl PartialEq for DooStr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for DooStr {}

impl PartialEq<&str> for DooStr {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<str> for DooStr {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<DooStr> for &str {
    #[inline]
    fn eq(&self, other: &DooStr) -> bool {
        *self == other.as_str()
    }
}

impl std::hash::Hash for DooStr {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl From<&str> for DooStr {
    #[inline]
    fn from(s: &str) -> Self {
        Self::from_str(s)
    }
}

impl From<&String> for DooStr {
    #[inline]
    fn from(s: &String) -> Self {
        Self::from_str(s.as_str())
    }
}

impl AsRef<str> for DooStr {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<[u8]> for DooStr {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}
