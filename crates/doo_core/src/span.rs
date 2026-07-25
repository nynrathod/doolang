//! Source Span — compact byte-range tracking for error reporting and debugging.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::hash::{Hash, Hasher};

/// A unique identifier for a source file.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct FileId(pub u32);

impl FileId {
    #[inline]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }
    pub const DUMMY: Self = Self(0);
    #[inline]
    pub const fn is_dummy(self) -> bool {
        self.0 == 0
    }
}

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "file{}", self.0)
    }
}

impl From<u32> for FileId {
    #[inline]
    fn from(id: u32) -> Self {
        Self(id)
    }
}

/// A span in source code: a byte range within a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Hash for Span {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.start.hash(state);
        self.end.hash(state);
    }
}

impl Span {
    #[inline]
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
    #[inline]
    pub const fn dummy() -> Self {
        Self { start: 0, end: 0 }
    }
    #[inline]
    pub const fn empty() -> Self {
        Self::dummy()
    }
    #[inline]
    pub const fn from_offsets(start: u32, end: u32) -> Self {
        Self::new(start, end)
    }
    #[inline]
    pub const fn len(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }
    #[inline]
    pub const fn is_dummy(&self) -> bool {
        self.start == 0 && self.end == 0
    }

    #[inline]
    pub fn merge(self, other: Span) -> Span {
        if self.is_dummy() {
            return other;
        }
        if other.is_dummy() {
            return self;
        }
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    #[inline]
    pub const fn offset(self, offset: u32) -> Span {
        Span {
            start: self.start + offset,
            end: self.end + offset,
        }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// A span with associated file information.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileSpan {
    pub file_id: FileId,
    pub span: Span,
}

impl FileSpan {
    #[inline]
    pub const fn new(file_id: FileId, span: Span) -> Self {
        Self { file_id, span }
    }
    #[inline]
    pub const fn dummy() -> Self {
        Self {
            file_id: FileId::DUMMY,
            span: Span::dummy(),
        }
    }
    #[inline]
    pub const fn span(&self) -> Span {
        self.span
    }
    #[inline]
    pub const fn file_id(&self) -> FileId {
        self.file_id
    }
    #[inline]
    pub const fn start(&self) -> u32 {
        self.span.start
    }
    #[inline]
    pub const fn end(&self) -> u32 {
        self.span.end
    }
    #[inline]
    pub const fn is_dummy(&self) -> bool {
        self.file_id.is_dummy() && self.span.is_dummy()
    }

    pub fn merge(self, other: FileSpan) -> FileSpan {
        FileSpan {
            file_id: self.file_id,
            span: self.span.merge(other.span),
        }
    }
}

impl From<Span> for FileSpan {
    #[inline]
    fn from(span: Span) -> Self {
        FileSpan::new(FileId::DUMMY, span)
    }
}

/// Precomputed line index for O(log n) line/column lookup.
#[derive(Debug, Clone)]
pub struct LineIndex {
    line_starts: Vec<u32>,
}

impl LineIndex {
    pub fn new(source: &str) -> Self {
        let mut line_starts = vec![0u32];
        for (byte_idx, ch) in source.char_indices() {
            if ch == '\n' {
                line_starts.push((byte_idx + 1) as u32);
            }
        }
        Self { line_starts }
    }

    pub fn line_col(&self, offset: u32) -> (u32, u32) {
        let line_idx = self
            .line_starts
            .binary_search(&offset)
            .unwrap_or_else(|i| i.saturating_sub(1));
        let line = (line_idx as u32) + 1;
        let col = offset - self.line_starts.get(line_idx).copied().unwrap_or(0) + 1;
        (line, col)
    }

    pub fn line_range(&self, line: u32) -> Option<Span> {
        let line_idx = (line as usize).checked_sub(1)?;
        let start = *self.line_starts.get(line_idx)?;
        let end = self.line_starts.get(line_idx + 1).copied().unwrap_or(start);
        Some(Span::new(start, end))
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }
}

impl Default for LineIndex {
    fn default() -> Self {
        Self {
            line_starts: vec![0],
        }
    }
}

pub trait Spanned {
    fn span(&self) -> Span;
}

impl Spanned for Span {
    #[inline]
    fn span(&self) -> Span {
        *self
    }
}
impl Spanned for FileSpan {
    #[inline]
    fn span(&self) -> Span {
        self.span
    }
}
