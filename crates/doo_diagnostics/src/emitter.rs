//! Diagnostic Emitter — renders `CompilerError`s using a `SourceMap`.

use crate::source_map::{SourceMap, SpanContext};
use doo_core::errors::codes::{CompilerError, ErrorCode, ErrorSeverity};
use doo_core::{FileId, Span};
use std::io::{self, Write};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

pub struct DiagnosticEmitter {
    stream: StandardStream,
    use_color: bool,
}

impl DiagnosticEmitter {
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

    pub fn emit(&mut self, err: &CompilerError, source_map: &SourceMap) -> io::Result<()> {
        let ctx = source_map.span_context(err.file_id, err.span);

        self.write_severity_label(err.severity)?;
        self.set_color(Color::White, true)?;
        write!(self.stream, "[{}]", err.code.code())?;
        self.reset_color()?;
        write!(self.stream, ": ")?;
        self.write_severity_color(err.severity, true)?;
        writeln!(self.stream, "{}", err.code.title())?;

        self.set_color(Color::Blue, true)?;
        write!(self.stream, " --> ")?;
        self.reset_color()?;
        writeln!(self.stream, "{}:{}:{}", ctx.filename, ctx.line, ctx.col)?;

        let source_trimmed = ctx.source_line.trim_end();
        write!(self.stream, "  | ")?;
        writeln!(self.stream, "{}", source_trimmed)?;

        write!(self.stream, "  | ")?;
        let indent = ctx.col.saturating_sub(1) as usize;
        write!(self.stream, "{}", " ".repeat(indent))?;
        self.write_severity_color(err.severity, false)?;
        write!(self.stream, "{}", "^".repeat(ctx.span_len.max(1)))?;
        self.reset_color()?;
        write!(self.stream, " {}", err.message)?;

        if let Some(ref suggestion) = err.suggestion {
            self.set_color(Color::Green, false)?;
            write!(self.stream, " -> {}", suggestion)?;
            self.reset_color()?;
        }
        writeln!(self.stream)?;

        for note in &err.notes {
            self.set_color(Color::Cyan, false)?;
            write!(self.stream, "  = note: ")?;
            self.reset_color()?;
            writeln!(self.stream, "{}", note)?;
        }

        for (file_span, label_msg) in &err.labels {
            let lctx = source_map.span_context(file_span.file_id, file_span.span);
            write!(self.stream, "{}:{}  ", lctx.filename, lctx.line)?;
            writeln!(self.stream, "{}", label_msg)?;
        }

        Ok(())
    }

    pub fn emit_all(&mut self, errors: &[CompilerError], source_map: &SourceMap) -> io::Result<()> {
        if errors.is_empty() {
            return Ok(());
        }
        if errors.len() >= 5 {
            self.emit_summary(errors, source_map)?;
        }
        for err in errors {
            self.emit(err, source_map)?;
        }
        self.emit_summary_line(errors, source_map)?;
        Ok(())
    }

    pub fn emit_summary(
        &mut self,
        errors: &[CompilerError],
        source_map: &SourceMap,
    ) -> io::Result<()> {
        writeln!(self.stream, "--- doo compile ---")?;
        let mut by_file: std::collections::BTreeMap<&str, Vec<&CompilerError>> =
            std::collections::BTreeMap::new();
        for err in errors {
            let filename = source_map.filename(err.file_id);
            by_file.entry(filename).or_default().push(err);
        }
        for (filename, file_errors) in &by_file {
            writeln!(self.stream, "{}", filename)?;
            for err in file_errors {
                let ctx = source_map.span_context(err.file_id, err.span);
                write!(self.stream, "  ")?;
                self.write_severity_label_short(err.severity)?;
                self.set_color(Color::White, false)?;
                write!(self.stream, " :{:<4}", ctx.line)?;
                self.write_severity_color(err.severity, true)?;
                write!(self.stream, " {:<18}", err.code.title())?;
                self.reset_color()?;
                writeln!(self.stream, " {}", err.message)?;
            }
        }
        writeln!(self.stream, "---")?;
        Ok(())
    }

    pub fn explain_code(&mut self, code: ErrorCode) -> io::Result<()> {
        self.set_color(Color::Cyan, true)?;
        writeln!(self.stream, "--- {} ---", code)?;
        self.reset_color()?;
        writeln!(self.stream, "Category: {:?}", code.category())?;
        writeln!(self.stream, "Severity: {}", code.severity())?;
        writeln!(self.stream, "{}", code.explanation())?;
        Ok(())
    }

    pub fn emit_detailed(&mut self, err: &CompilerError, source_map: &SourceMap) -> io::Result<()> {
        let ctx = source_map.span_context(err.file_id, err.span);
        let line_num = ctx.line;

        self.write_severity_label(err.severity)?;
        self.set_color(Color::White, true)?;
        write!(self.stream, "[{}]", err.code.code())?;
        self.reset_color()?;
        write!(self.stream, ": ")?;
        self.write_severity_color(err.severity, true)?;
        writeln!(self.stream, "{}", err.code.title())?;

        self.set_color(Color::Blue, true)?;
        write!(self.stream, " --> ")?;
        self.reset_color()?;
        writeln!(self.stream, "{}:{}:{}", ctx.filename, line_num, ctx.col)?;

        writeln!(self.stream, "  |")?;

        if line_num > 1 {
            let prev = source_map.line_text(err.file_id, line_num - 1);
            write!(self.stream, "{:>3} | ", line_num - 1)?;
            writeln!(self.stream, "{}", prev)?;
        }

        write!(self.stream, "{:>3} | ", line_num)?;
        writeln!(self.stream, "{}", ctx.source_line)?;

        write!(self.stream, "  | ")?;
        let indent = ctx.col.saturating_sub(1) as usize;
        write!(self.stream, "{}", " ".repeat(indent))?;
        self.write_severity_color(err.severity, true)?;
        write!(self.stream, "{}", "^".repeat(ctx.span_len.max(1)))?;
        self.reset_color()?;
        writeln!(self.stream, " {}", err.message)?;

        let next = source_map.line_text(err.file_id, line_num + 1);
        if !next.is_empty() {
            write!(self.stream, "{:>3} | ", line_num + 1)?;
            writeln!(self.stream, "{}", next)?;
        }

        if let Some(ref suggestion) = err.suggestion {
            self.set_color(Color::Green, true)?;
            write!(self.stream, "  = help: ")?;
            self.reset_color()?;
            writeln!(self.stream, "{}", suggestion)?;
        }

        Ok(())
    }

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

        let mut files: Vec<FileId> = errors.iter().map(|e| e.file_id).collect();
        files.sort();
        files.dedup();
        let file_count = files.len();

        write!(self.stream, "  ")?;
        if ice_count > 0 {
            self.set_color(Color::Magenta, true)?;
            write!(self.stream, "{} internal error(s)  ", ice_count)?;
            self.reset_color()?;
        }
        if error_count > 0 {
            self.set_color(Color::Red, true)?;
            write!(self.stream, "{} error(s)  ", error_count)?;
            self.reset_color()?;
        }
        if warning_count > 0 {
            self.set_color(Color::Yellow, true)?;
            write!(self.stream, "{} warning(s)  ", warning_count)?;
            self.reset_color()?;
        }
        write!(self.stream, "in {} file(s)", file_count)?;

        let filenames: Vec<&str> = files.iter().map(|&id| source_map.filename(id)).collect();
        write!(self.stream, " ({})", filenames.join(", "))?;
        writeln!(self.stream)?;

        Ok(())
    }

    fn write_severity_label(&mut self, severity: ErrorSeverity) -> io::Result<()> {
        let (label, color) = match severity {
            ErrorSeverity::Error => ("error", Color::Red),
            ErrorSeverity::Warning => ("warning", Color::Yellow),
            ErrorSeverity::Note => ("note", Color::Cyan),
            ErrorSeverity::Ice => ("internal compiler error", Color::Magenta),
        };
        self.set_color(color, true)?;
        write!(self.stream, "{}", label)?;
        self.reset_color()?;
        Ok(())
    }

    fn write_severity_label_short(&mut self, severity: ErrorSeverity) -> io::Result<()> {
        let (label, color) = match severity {
            ErrorSeverity::Error => ("E", Color::Red),
            ErrorSeverity::Warning => ("W", Color::Yellow),
            ErrorSeverity::Note => ("N", Color::Cyan),
            ErrorSeverity::Ice => ("I", Color::Magenta),
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
        self.stream
            .set_color(ColorSpec::new().set_fg(Some(color)).set_bold(bold))
    }

    fn reset_color(&mut self) -> io::Result<()> {
        self.stream.reset()
    }
}

impl Default for DiagnosticEmitter {
    fn default() -> Self {
        Self::new(true)
    }
}
