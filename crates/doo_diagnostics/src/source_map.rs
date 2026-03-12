//! Source Map
//!
//! Maps file IDs to source contents and filenames for error rendering.
//! Single source of truth for all source file tracking during compilation.

use doo_core::span::LineIndex;
use doo_core::Span;

/// A loaded source file.
#[derive(Debug)]
struct SourceFile {
    /// Display name (e.g., "handlers.doo").
    name: String,
    /// Full source content.
    source: String,
    /// Precomputed line index for O(log n) lookups.
    line_index: LineIndex,
}

/// Maps file IDs → source content for error rendering.
#[derive(Debug, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self { files: Vec::new() }
    }

    /// Register a file and return its file_id.
    pub fn add_file(&mut self, name: impl Into<String>, source: impl Into<String>) -> u32 {
        let source = source.into();
        let line_index = LineIndex::new(&source);
        let id = self.files.len() as u32;
        self.files.push(SourceFile {
            name: name.into(),
            source,
            line_index,
        });
        id
    }

    /// Get filename for a file_id.
    pub fn filename(&self, file_id: u32) -> &str {
        self.files
            .get(file_id as usize)
            .map(|f| f.name.as_str())
            .unwrap_or("<unknown>")
    }

    /// Get source text for a file_id.
    pub fn source(&self, file_id: u32) -> &str {
        self.files
            .get(file_id as usize)
            .map(|f| f.source.as_str())
            .unwrap_or("")
    }

    /// Get (line, column) for a byte offset (both 1-indexed).
    pub fn line_col(&self, file_id: u32, offset: u32) -> (u32, u32) {
        self.files
            .get(file_id as usize)
            .map(|f| f.line_index.line_col(offset))
            .unwrap_or((1, 1))
    }

    /// Get the source text of a specific line (1-indexed).
    pub fn line_text(&self, file_id: u32, line: u32) -> &str {
        let source = self.source(file_id);
        source
            .lines()
            .nth((line.saturating_sub(1)) as usize)
            .unwrap_or("")
    }

    /// Extract the text covered by a span.
    pub fn span_text(&self, span: &Span) -> &str {
        let source = self.source(span.file_id);
        let start = span.start as usize;
        let end = (span.end as usize).min(source.len());
        if start <= end && end <= source.len() {
            &source[start..end]
        } else {
            ""
        }
    }

    /// Get context: the source line, line number, and column for a span.
    pub fn span_context(&self, span: &Span) -> SpanContext<'_> {
        let (line, col) = self.line_col(span.file_id, span.start);
        let source_line = self.line_text(span.file_id, line);
        let filename = self.filename(span.file_id);
        SpanContext {
            filename,
            line,
            col,
            source_line,
            span_len: span.len().max(1) as usize,
        }
    }
}

/// Resolved context for a span — ready for rendering.
#[derive(Debug)]
pub struct SpanContext<'a> {
    pub filename: &'a str,
    pub line: u32,
    pub col: u32,
    pub source_line: &'a str,
    pub span_len: usize,
}
