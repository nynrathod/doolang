//! Diagnostic Emitter
//!
//! Formats and outputs error messages in four modes:
//!
//! **Compact** (default, 2-4 lines per error):
//! ```text
//! error[E0100]: TYPE MISMATCH
//!  --> handlers.doo:12:15
//!   | let age: Int = "twenty"
//!   |                ~~~~~~~~ Str, expected Int -> use: 20
//! ```
//!
//! **Summary** (grouped by file):
//! ```text
//! --- doo compile ---
//! 3 error(s)  1 warning(s)  in 2 files
//!
//! handlers.doo
//!   :12  TYPE MISMATCH    Int = "twenty"    -> use: 20
//!   :15  UNKNOWN NAME     userName          -> user_name?
//! ---
//! ```
//!
//! **Detailed** (`--explain`):
//! Full explanation with context lines, notes, and docs link.

use crate::source_map::SourceMap;
use doo_core::errors::codes::{CompilerError, ErrorCode, ErrorSeverity};
use std::collections::BTreeMap;
use std::io::{self, Write};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

/// Diagnostic emitter — renders `CompilerError`s using a `SourceMap`.
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

    // -----------------------------------------------------------------------
    // Compact (single error)
    // -----------------------------------------------------------------------

    /// Emit one error in compact format (2-4 lines).
    ///
    /// ```text
    /// error[E0100]: TYPE MISMATCH
    ///  --> handlers.doo:12:15
    ///   | let age: Int = "twenty"
    ///   |                ~~~~~~~~ Str, expected Int -> use: 20
    /// ```
    pub fn emit(&mut self, err: &CompilerError, source_map: &SourceMap) -> io::Result<()> {
        let ctx = source_map.span_context(&err.span);
        let severity = err.severity;

        // Line 1: error[E0100]: TYPE MISMATCH  (or  warning: UNREACHABLE CODE)
        self.write_severity_label(severity)?;
        if severity == ErrorSeverity::Error || severity == ErrorSeverity::Ice {
            self.set_color(Color::White, true)?;
            write!(self.stream, "[{}]", err.code.code())?;
            self.reset_color()?;
        }
        write!(self.stream, ": ")?;
        self.write_severity_color(severity, true)?;
        writeln!(self.stream, "{}", err.code.title())?;
        self.reset_color()?;

        // Line 2:  --> file:line:col
        self.set_color(Color::Blue, true)?;
        write!(self.stream, " --> ")?;
        self.reset_color()?;
        writeln!(self.stream, "{}:{}:{}", ctx.filename, ctx.line, ctx.col)?;

        // Line 3:   | source line
        let source_trimmed = ctx.source_line.trim_end();
        self.set_color(Color::Blue, true)?;
        write!(self.stream, "  | ")?;
        self.reset_color()?;
        writeln!(self.stream, "{}", source_trimmed)?;

        // Line 4:   | ^^^^ message -> suggestion
        self.set_color(Color::Blue, true)?;
        write!(self.stream, "  | ")?;
        self.reset_color()?;
        let indent = ctx.col.saturating_sub(1) as usize;
        write!(self.stream, "{}", " ".repeat(indent))?;
        self.write_severity_color(severity, true)?;
        write!(
            self.stream,
            "{}",
            "^".repeat(
                ctx.span_len
                    .min(source_trimmed.len().saturating_sub(indent))
                    .max(1)
            )
        )?;
        self.reset_color()?;
        write!(self.stream, " {}", err.message)?;
        if let Some(ref suggestion) = err.suggestion {
            self.set_color(Color::Green, false)?;
            write!(self.stream, " -> {}", suggestion)?;
            self.reset_color()?;
        }
        writeln!(self.stream)?;

        // Notes
        for note in &err.notes {
            self.set_color(Color::Cyan, false)?;
            write!(self.stream, "  = note: ")?;
            self.reset_color()?;
            writeln!(self.stream, "{}", note)?;
        }

        // Secondary labels
        for (label_span, label_msg) in &err.labels {
            let lctx = source_map.span_context(label_span);
            self.set_color(Color::Blue, true)?;
            write!(self.stream, " --> ")?;
            self.reset_color()?;
            write!(self.stream, "{}:{}  ", lctx.filename, lctx.line)?;
            writeln!(self.stream, "{}", label_msg)?;
        }

        writeln!(self.stream)?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Emit all (compact + summary line)
    // -----------------------------------------------------------------------

    /// Emit all errors in compact format, followed by a summary line.
    /// When 5+ errors exist, also shows grouped summary view.
    pub fn emit_all(&mut self, errors: &[CompilerError], source_map: &SourceMap) -> io::Result<()> {
        if errors.is_empty() {
            return Ok(());
        }

        if errors.len() >= 5 {
            // Many errors: show summary view (grouped by file)
            self.emit_summary(errors, source_map)?;
        } else {
            // Few errors: show compact view per error + summary line
            for err in errors {
                self.emit(err, source_map)?;
            }
            self.emit_summary_line(errors, source_map)?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Summary (grouped by file)
    // -----------------------------------------------------------------------

    /// Emit grouped summary view.
    pub fn emit_summary(
        &mut self,
        errors: &[CompilerError],
        source_map: &SourceMap,
    ) -> io::Result<()> {
        if errors.is_empty() {
            return Ok(());
        }

        // Header
        self.set_color(Color::White, true)?;
        writeln!(self.stream, "--- doo compile ---")?;
        self.reset_color()?;

        // Count summary
        self.emit_summary_line(errors, source_map)?;
        writeln!(self.stream)?;

        // Group by file
        let mut by_file: BTreeMap<&str, Vec<&CompilerError>> = BTreeMap::new();
        for err in errors {
            let filename = source_map.filename(err.span.file_id);
            by_file.entry(filename).or_default().push(err);
        }

        for (filename, file_errors) in &by_file {
            self.set_color(Color::White, true)?;
            writeln!(self.stream, "{}", filename)?;
            self.reset_color()?;

            for err in file_errors {
                let ctx = source_map.span_context(&err.span);
                let snippet: String = ctx.source_line.trim().chars().take(30).collect();

                write!(self.stream, "  ")?;
                self.write_severity_label_short(err.severity)?;
                self.set_color(Color::White, false)?;
                write!(self.stream, " :{:<4}", ctx.line)?;
                self.reset_color()?;
                self.write_severity_color(err.severity, true)?;
                write!(self.stream, " {:<18}", err.code.title())?;
                self.reset_color()?;
                write!(self.stream, " {:<30}", snippet)?;
                if let Some(ref suggestion) = err.suggestion {
                    self.set_color(Color::Green, false)?;
                    write!(self.stream, " -> {}", suggestion)?;
                    self.reset_color()?;
                }
                writeln!(self.stream)?;
            }
            writeln!(self.stream)?;
        }

        // Footer
        self.set_color(Color::White, true)?;
        writeln!(self.stream, "---")?;
        self.reset_color()?;

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Detailed (--explain)
    // -----------------------------------------------------------------------

    /// Print detailed explanation for one error code.
    pub fn explain_code(&mut self, code: ErrorCode) -> io::Result<()> {
        self.set_color(Color::Cyan, true)?;
        writeln!(self.stream, "--- {} ---", code)?;
        self.reset_color()?;
        writeln!(self.stream)?;

        self.set_color(Color::White, true)?;
        writeln!(self.stream, "Category: {:?}", code.category())?;
        writeln!(self.stream, "Severity: {}", code.severity())?;
        self.reset_color()?;
        writeln!(self.stream)?;

        writeln!(self.stream, "{}", code.explanation())?;
        writeln!(self.stream)?;
        Ok(())
    }

    /// Print detailed view of a specific error (with context lines).
    pub fn emit_detailed(&mut self, err: &CompilerError, source_map: &SourceMap) -> io::Result<()> {
        let ctx = source_map.span_context(&err.span);

        // Header: error[E0100]: TYPE MISMATCH  (or  warning: UNREACHABLE CODE)
        self.write_severity_label(err.severity)?;
        if err.severity == ErrorSeverity::Error || err.severity == ErrorSeverity::Ice {
            self.set_color(Color::White, true)?;
            write!(self.stream, "[{}]", err.code.code())?;
            self.reset_color()?;
        }
        write!(self.stream, ": ")?;
        self.set_color(Color::White, true)?;
        writeln!(self.stream, "{}", err.code.title())?;
        self.reset_color()?;

        // Location
        self.set_color(Color::Blue, true)?;
        write!(self.stream, " --> ")?;
        self.reset_color()?;
        writeln!(self.stream, "{}:{}:{}", ctx.filename, ctx.line, ctx.col)?;

        // Context lines (1 before, error line, 1 after)
        let line_num = ctx.line;
        self.set_color(Color::Blue, true)?;
        writeln!(self.stream, "  |")?;
        self.reset_color()?;

        // Previous line
        if line_num > 1 {
            let prev = source_map.line_text(err.span.file_id, line_num - 1);
            self.set_color(Color::Blue, true)?;
            write!(self.stream, "{:>3} | ", line_num - 1)?;
            self.reset_color()?;
            writeln!(self.stream, "{}", prev)?;
        }

        // Error line
        self.set_color(Color::Blue, true)?;
        write!(self.stream, "{:>3} | ", line_num)?;
        self.reset_color()?;
        writeln!(self.stream, "{}", ctx.source_line)?;

        // Underline
        self.set_color(Color::Blue, true)?;
        write!(self.stream, "  | ")?;
        self.reset_color()?;
        let indent = ctx.col.saturating_sub(1) as usize;
        write!(self.stream, "{}", " ".repeat(indent))?;
        self.write_severity_color(err.severity, true)?;
        write!(self.stream, "{}", "^".repeat(ctx.span_len.max(1)))?;
        self.reset_color()?;
        writeln!(self.stream, " {}", err.message)?;

        // Next line
        let next = source_map.line_text(err.span.file_id, line_num + 1);
        if !next.is_empty() {
            self.set_color(Color::Blue, true)?;
            write!(self.stream, "{:>3} | ", line_num + 1)?;
            self.reset_color()?;
            writeln!(self.stream, "{}", next)?;
        }

        self.set_color(Color::Blue, true)?;
        writeln!(self.stream, "  |")?;
        self.reset_color()?;

        // Suggestion
        if let Some(ref suggestion) = err.suggestion {
            self.set_color(Color::Green, true)?;
            write!(self.stream, "  = help: ")?;
            self.reset_color()?;
            writeln!(self.stream, "{}", suggestion)?;
        }

        // Notes
        for note in &err.notes {
            self.set_color(Color::Cyan, false)?;
            write!(self.stream, "  = note: ")?;
            self.reset_color()?;
            writeln!(self.stream, "{}", note)?;
        }

        // Docs link
        if err.severity == ErrorSeverity::Error {
            self.set_color(Color::White, false)?;
            writeln!(
                self.stream,
                "  = docs: https://doo.dev/errors/{}",
                err.code.code().to_lowercase()
            )?;
            self.reset_color()?;
        }

        writeln!(self.stream)?;

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn emit_summary_line(
        &mut self,
        errors: &[CompilerError],
        source_map: &SourceMap,
    ) -> io::Result<()> {
        let error_count = errors
            .iter()
            .filter(|e| e.severity == ErrorSeverity::Error)
            .count();
        let warning_count = errors
            .iter()
            .filter(|e| e.severity == ErrorSeverity::Warning)
            .count();
        let ice_count = errors
            .iter()
            .filter(|e| e.severity == ErrorSeverity::Ice)
            .count();

        // Count unique files
        let mut files: Vec<u32> = errors.iter().map(|e| e.span.file_id).collect();
        files.sort();
        files.dedup();
        let file_count = files.len();

        if ice_count > 0 {
            self.set_color(Color::Magenta, true)?;
            write!(self.stream, "{} internal error(s)", ice_count)?;
            self.reset_color()?;
            write!(self.stream, "  ")?;
        }
        if error_count > 0 {
            self.set_color(Color::Red, true)?;
            write!(self.stream, "{} error(s)", error_count)?;
            self.reset_color()?;
            write!(self.stream, "  ")?;
        }
        if warning_count > 0 {
            self.set_color(Color::Yellow, true)?;
            write!(self.stream, "{} warning(s)", warning_count)?;
            self.reset_color()?;
            write!(self.stream, "  ")?;
        }

        self.set_color(Color::White, false)?;
        write!(self.stream, "in {} file(s)", file_count)?;
        self.reset_color()?;

        // List files
        let filenames: Vec<&str> = files.iter().map(|&id| source_map.filename(id)).collect();
        if file_count <= 4 {
            write!(self.stream, " ({})", filenames.join(", "))?;
        }
        writeln!(self.stream)?;

        Ok(())
    }

    /// Write the severity label: "error", "warning", "note", "ice" — colored.
    fn write_severity_label(&mut self, severity: ErrorSeverity) -> io::Result<()> {
        let (color, label) = match severity {
            ErrorSeverity::Error => (Color::Red, "error"),
            ErrorSeverity::Warning => (Color::Yellow, "warning"),
            ErrorSeverity::Note => (Color::Cyan, "note"),
            ErrorSeverity::Ice => (Color::Magenta, "ice"),
        };
        self.set_color(color, true)?;
        write!(self.stream, "{}", label)?;
        self.reset_color()?;
        Ok(())
    }

    /// Write a short severity label for summary view.
    fn write_severity_label_short(&mut self, severity: ErrorSeverity) -> io::Result<()> {
        let (color, label) = match severity {
            ErrorSeverity::Error => (Color::Red, "E"),
            ErrorSeverity::Warning => (Color::Yellow, "W"),
            ErrorSeverity::Note => (Color::Cyan, "N"),
            ErrorSeverity::Ice => (Color::Magenta, "!"),
        };
        self.set_color(color, true)?;
        write!(self.stream, "{}", label)?;
        self.reset_color()?;
        Ok(())
    }

    fn write_severity_color(&mut self, severity: ErrorSeverity, bold: bool) -> io::Result<()> {
        let color = match severity {
            ErrorSeverity::Error => Color::Red,
            ErrorSeverity::Warning => Color::Yellow,
            ErrorSeverity::Note => Color::Cyan,
            ErrorSeverity::Ice => Color::Magenta,
        };
        self.set_color(color, bold)
    }

    fn set_color(&mut self, color: Color, bold: bool) -> io::Result<()> {
        if self.use_color {
            self.stream
                .set_color(ColorSpec::new().set_fg(Some(color)).set_bold(bold))?;
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
    use doo_core::Span;

    #[test]
    fn test_emit_compact() {
        let mut sm = SourceMap::new();
        let fid = sm.add_file(
            "handlers.doo",
            "line1\nline2\nline3\nline4\nlet x = y\nline6",
        );

        let err = CompilerError::new(
            ErrorCode::UndefinedVariable,
            "variable 'y' not found",
            Span::new(fid, 32, 33), // 'y' in "let x = y"
        )
        .with_suggestion("did you mean 'x'?");

        let mut emitter = DiagnosticEmitter::new(false);
        emitter.emit(&err, &sm).unwrap();
    }

    #[test]
    fn test_emit_all() {
        let mut sm = SourceMap::new();
        let fid = sm.add_file("main.doo", "let age: Int = \"twenty\"\nlet x = y");

        let err1 = CompilerError::new(
            ErrorCode::TypeMismatch,
            "Str, expected Int",
            Span::new(fid, 15, 23), // "twenty"
        )
        .with_suggestion("use: 20");

        let err2 = CompilerError::new(
            ErrorCode::UndefinedVariable,
            "variable 'y' not found",
            Span::new(fid, 32, 33),
        );

        let mut emitter = DiagnosticEmitter::new(false);
        emitter.emit_all(&[err1, err2], &sm).unwrap();
    }

    #[test]
    fn test_emit_summary() {
        let mut sm = SourceMap::new();
        let fid1 = sm.add_file("handlers.doo", "let age: Int = \"twenty\"\nlet x = unknown");
        let fid2 = sm.add_file("models.doo", "struct User { name: Str }");

        let errors = vec![
            CompilerError::new(
                ErrorCode::TypeMismatch,
                "Str given, Int expected",
                Span::new(fid1, 15, 23),
            )
            .with_suggestion("use: 20"),
            CompilerError::new(
                ErrorCode::UndefinedVariable,
                "'unknown' not defined",
                Span::new(fid1, 31, 38),
            ),
            CompilerError::new(
                ErrorCode::UndefinedType,
                "type 'Str' — did you mean String?",
                Span::new(fid2, 20, 23),
            ),
        ];

        let mut emitter = DiagnosticEmitter::new(false);
        emitter.emit_summary(&errors, &sm).unwrap();
    }

    #[test]
    fn test_emit_detailed() {
        let mut sm = SourceMap::new();
        let fid = sm.add_file(
            "app.doo",
            "fn main() {\n    let age: Int = \"twenty\"\n    print(age)\n}",
        );

        let err = CompilerError::new(
            ErrorCode::TypeMismatch,
            "Str, expected Int",
            Span::new(fid, 31, 39), // "twenty"
        )
        .with_suggestion("use: 20")
        .with_note("Int and Str are not compatible types");

        let mut emitter = DiagnosticEmitter::new(false);
        emitter.emit_detailed(&err, &sm).unwrap();
    }

    #[test]
    fn test_explain_code() {
        let mut emitter = DiagnosticEmitter::new(false);
        emitter.explain_code(ErrorCode::TypeMismatch).unwrap();
        emitter
            .explain_code(ErrorCode::ConcurrentMutableBorrow)
            .unwrap();
    }
}
