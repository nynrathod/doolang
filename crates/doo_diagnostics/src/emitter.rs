//! Diagnostic Emitter
//!
//! Formats and outputs error messages in compact, readable format.

use std::io::{self, Write};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};
use doo_core::Span;
use crate::codes::ErrorCode;

/// A diagnostic message.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// Error code
    pub code: ErrorCode,
    /// File name
    pub file: String,
    /// Line number (1-indexed)
    pub line: u32,
    /// Column number (1-indexed)
    pub column: u32,
    /// Source line content
    pub source_line: String,
    /// Description of the error
    pub description: String,
    /// Suggested fix (optional)
    pub fix: Option<String>,
    /// Length of the error span
    pub span_len: usize,
}

impl Diagnostic {
    /// Create a new diagnostic from a span.
    /// Create a new diagnostic from a span.
    pub fn new(
        code: ErrorCode,
        span: Span,
        source: &str,
        description: impl Into<String>,
    ) -> Self {
        let line_index = doo_core::span::LineIndex::new(source);
        let (line, column) = line_index.line_col(span.start);
        
        let lines: Vec<&str> = source.lines().collect();
        let line_idx = line.saturating_sub(1) as usize;
        let source_line = lines.get(line_idx).unwrap_or(&"").to_string();

        // Temporary: use placeholder filename since we can't resolve file_id yet without a SourceMap
        let filename = format!("file_{}", span.file_id);

        Self {
            code,
            file: filename,
            line,
            column,
            source_line,
            description: description.into(),
            fix: None,
            span_len: span.len().max(1) as usize,
        }
    }

    /// Add a suggested fix.
    pub fn with_fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }
}

/// Diagnostic emitter - outputs formatted errors.
pub struct DiagnosticEmitter {
    stream: StandardStream,
    use_color: bool,
}

impl DiagnosticEmitter {
    /// Create a new emitter.
    pub fn new(use_color: bool) -> Self {
        let choice = if use_color {
            ColorChoice::Auto
        } else {
            ColorChoice::Never
        };
        Self {
            stream: StandardStream::stderr(choice),
            use_color,
        }
    }

    /// Emit a diagnostic.
    pub fn emit(&mut self, diag: &Diagnostic) -> io::Result<()> {
        // Line 1: ❌ file:line  ERROR_TYPE
        self.set_color(Color::Red, true)?;
        write!(self.stream, "❌ ")?;
        self.reset_color()?;
        
        write!(self.stream, "{}:{}", diag.file, diag.line)?;
        write!(self.stream, "  ")?;
        
        self.set_color(Color::Red, true)?;
        writeln!(self.stream, "{}", diag.code.message())?;
        self.reset_color()?;

        // Line 2: source line
        self.set_color(Color::Cyan, false)?;
        write!(self.stream, "   ")?;
        self.reset_color()?;
        writeln!(self.stream, "{}", diag.source_line)?;

        // Line 3: underline + description → fix
        write!(self.stream, "   ")?;
        let indent = diag.column.saturating_sub(1) as usize;
        write!(self.stream, "{}", " ".repeat(indent))?;
        
        self.set_color(Color::Red, true)?;
        write!(self.stream, "{}", "~".repeat(diag.span_len))?;
        self.reset_color()?;
        
        write!(self.stream, " {}", diag.description)?;
        
        if let Some(fix) = &diag.fix {
            self.set_color(Color::Green, false)?;
            write!(self.stream, " → {}", fix)?;
            self.reset_color()?;
        }
        writeln!(self.stream)?;
        writeln!(self.stream)?;

        Ok(())
    }

    /// Emit multiple diagnostics.
    pub fn emit_all(&mut self, diags: &[Diagnostic]) -> io::Result<()> {
        for diag in diags {
            self.emit(diag)?;
        }
        
        // Summary
        let errors = diags.len();
        if errors > 0 {
            self.set_color(Color::Red, true)?;
            writeln!(self.stream, "error: aborting due to {} error(s)", errors)?;
            self.reset_color()?;
        }
        
        Ok(())
    }

    /// Print --explain output for an error code.
    pub fn explain(&mut self, code: ErrorCode) -> io::Result<()> {
        self.set_color(Color::Cyan, true)?;
        writeln!(self.stream, "Error {}: {}", code, code.message())?;
        self.reset_color()?;
        writeln!(self.stream)?;
        writeln!(self.stream, "{}", code.explanation())?;
        Ok(())
    }

    fn set_color(&mut self, color: Color, bold: bool) -> io::Result<()> {
        if self.use_color {
            self.stream.set_color(ColorSpec::new().set_fg(Some(color)).set_bold(bold))?;
        }
        Ok(())
    }

    fn reset_color(&mut self) -> io::Result<()> {
        if self.use_color {
            self.stream.reset()?;
        }
        Ok(())
    }
}

impl Default for DiagnosticEmitter {
    fn default() -> Self {
        Self::new(true)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostic_creation() {
        // Source:
        // line1 (0-5)
        // line2 (6-11)
        // line3 (12-17)
        // line4 (18-23)
        // let x = y (24-33)
        // line6 (34-39)
        
        let source = "line1\nline2\nline3\nline4\nlet x = y\nline6";
        let start = 24; // start of "let"
        let end = 27;   // end of "let"
        let span = Span::new(1, start, end);
        
        let diag = Diagnostic::new(ErrorCode::E0300, span, source, "variable not found");
        
        assert_eq!(diag.line, 5); // 5th line (1-indexed)
        assert_eq!(diag.column, 1); // 1st column (1-indexed)
        assert_eq!(diag.source_line, "let x = y");
    }
}
