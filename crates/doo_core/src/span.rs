//! # Source Span
//!
//! Tracks source locations for error messages and debugging.
//!
//! ## Design
//!
//! - Compact representation (12 bytes)
//! - Zero-cost for release builds
//! - File-relative offsets

use serde::{Deserialize, Serialize};

/// A span in source code with file information.
/// Uses byte offsets for O(1) substring extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Span {
    /// File ID (index into file table)
    pub file_id: u32,
    /// Start byte offset (inclusive)
    pub start: u32,
    /// End byte offset (exclusive)
    pub end: u32,
}

impl Span {
    /// Create a new span with file ID, start and end offsets
    pub const fn new(file_id: u32, start: u32, end: u32) -> Self {
        Self { file_id, start, end }
    }
    
    /// Create a span without file ID (uses file_id = 0)
    pub const fn from_offsets(start: u32, end: u32) -> Self {
        Self { file_id: 0, start, end }
    }
    
    /// Create an empty span at position 0
    pub const fn empty() -> Self {
        Self { file_id: 0, start: 0, end: 0 }
    }
    
    /// Create a dummy span (for generated code)
    pub const fn dummy() -> Self {
        Self { file_id: u32::MAX, start: 0, end: 0 }
    }
    
    /// Get the length in bytes
    pub const fn len(&self) -> u32 {
        self.end - self.start
    }
    
    /// Check if span is empty
    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }
    
    /// Check if this is a dummy span
    pub const fn is_dummy(&self) -> bool {
        self.file_id == u32::MAX
    }
    
    /// Merge two spans (useful for spanning an entire expression)
    /// Uses the file_id from self
    pub fn merge(&self, other: &Span) -> Span {
        Span {
            file_id: self.file_id,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
    
    /// Extract substring from source
    pub fn extract<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start as usize..self.end as usize]
    }
}

/// A span with file information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSpan {
    /// File ID (index into file table)
    pub file_id: u32,
    /// Span within the file
    pub span: Span,
}

impl FileSpan {
    pub fn new(file_id: u32, span: Span) -> Self {
        Self { file_id, span }
    }
}

/// Trait for AST nodes that have a span.
pub trait Spanned {
    fn span(&self) -> Span;
}

// ============================================================================
// Line/Column Computation
// ============================================================================

/// Computes line and column from byte offset.
/// Uses a line index for O(log n) lookup.
#[derive(Debug)]
pub struct LineIndex {
    /// Byte offsets where each line starts
    line_starts: Vec<u32>,
}

impl LineIndex {
    /// Build a line index from source text
    pub fn new(source: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, c) in source.char_indices() {
            if c == '\n' {
                line_starts.push((i + 1) as u32);
            }
        }
        Self { line_starts }
    }
    
    /// Get line and column for a byte offset (1-indexed)
    pub fn line_col(&self, offset: u32) -> (u32, u32) {
        let line = self.line_starts
            .binary_search(&offset)
            .unwrap_or_else(|i| i.saturating_sub(1));
        let col = offset - self.line_starts.get(line).copied().unwrap_or(0);
        (line as u32 + 1, col + 1)
    }
    
    /// Get the line containing the given offset
    pub fn line_range(&self, line: u32) -> Option<Span> {
        let line_idx = (line - 1) as usize;
        let start = *self.line_starts.get(line_idx)?;
        let end = self.line_starts.get(line_idx + 1).copied().unwrap_or(start);
        Some(Span::new(0, start, end))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_span_merge() {
        let a = Span::new(0, 0, 5);
        let b = Span::new(0, 10, 15);
        let merged = a.merge(&b);
        assert_eq!(merged.start, 0);
        assert_eq!(merged.end, 15);
    }

    #[test]
    fn test_span_extract() {
        let source = "hello world";
        let span = Span::new(0, 6, 11);
        assert_eq!(span.extract(source), "world");
    }

    #[test]
    fn test_line_index() {
        let source = "line1\nline2\nline3";
        let idx = LineIndex::new(source);
        
        assert_eq!(idx.line_col(0), (1, 1));   // Start of line 1
        assert_eq!(idx.line_col(6), (2, 1));   // Start of line 2
        assert_eq!(idx.line_col(12), (3, 1));  // Start of line 3
    }
}
