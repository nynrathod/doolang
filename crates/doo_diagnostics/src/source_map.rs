//! Source Map — maps file IDs to source contents for error rendering.

use doo_core::span::LineIndex;
use doo_core::{FileId, Span};

#[derive(Debug)]
struct SourceFile {
    name: String,
    source: String,
    line_index: LineIndex,
}

#[derive(Debug, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_file(&mut self, name: impl Into<String>, source: impl Into<String>) -> FileId {
        let source = source.into();
        let line_index = LineIndex::new(&source);
        let id = self.files.len() as u32;
        self.files.push(SourceFile {
            name: name.into(),
            source,
            line_index,
        });
        FileId(id)
    }

    pub fn filename(&self, file_id: FileId) -> &str {
        self.files
            .get(file_id.0 as usize)
            .map(|f| f.name.as_str())
            .unwrap_or("<unknown>")
    }

    pub fn source(&self, file_id: FileId) -> &str {
        self.files
            .get(file_id.0 as usize)
            .map(|f| f.source.as_str())
            .unwrap_or("")
    }

    pub fn line_col(&self, file_id: FileId, offset: u32) -> (u32, u32) {
        self.files
            .get(file_id.0 as usize)
            .map(|f| f.line_index.line_col(offset))
            .unwrap_or((1, 1))
    }

    pub fn line_text(&self, file_id: FileId, line: u32) -> &str {
        let source = self.source(file_id);
        source
            .lines()
            .nth((line.saturating_sub(1)) as usize)
            .unwrap_or("")
    }

    pub fn span_text(&self, file_id: FileId, span: Span) -> &str {
        let source = self.source(file_id);
        let start = (span.start as usize).min(source.len());
        let end = (span.end as usize).min(source.len());
        if start <= end {
            &source[start..end]
        } else {
            ""
        }
    }

    pub fn span_context(&self, file_id: FileId, span: Span) -> SpanContext<'_> {
        let (line, col) = self.line_col(file_id, span.start);
        let source_line = self.line_text(file_id, line);
        let filename = self.filename(file_id);
        SpanContext {
            filename,
            source_line,
            line,
            col,
            span_len: span.len().max(1) as usize,
        }
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

#[derive(Debug)]
pub struct SpanContext<'a> {
    pub filename: &'a str,
    pub source_line: &'a str,
    pub line: u32,
    pub col: u32,
    pub span_len: usize,
}
