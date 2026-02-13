#![allow(dead_code)]

/// Centralized diagnostics and error formatting for doo.
/// Provides colorized output, error code extraction, and source snippet rendering.
/// Used for both semantic and parse errors, as well as grouped reporting.
use crate::analyzer::types::SemanticError;
use crate::parser::parser::ParseError;
use std::collections::HashMap;

/// Color helpers for terminal output (ANSI escape codes).

fn color_red(s: &str) -> String {
    format!("\x1b[31m{}\x1b[0m", s)
}
fn color_bold_red(s: &str) -> String {
    format!("\x1b[1;31m{}\x1b[0m", s)
}
fn color_bold_green(s: &str) -> String {
    format!("\x1b[1;32m{}\x1b[0m", s)
}
fn color_yellow(s: &str) -> String {
    format!("\x1b[33m{}\x1b[0m", s)
}
fn color_bold_yellow(s: &str) -> String {
    format!("\x1b[1;33m{}\x1b[0m", s)
}
fn color_cyan(s: &str) -> String {
    format!("\x1b[36m{}\x1b[0m", s)
}
fn color_bold_cyan(s: &str) -> String {
    format!("\x1b[1;36m{}\x1b[0m", s)
}
fn color_dim(s: &str) -> String {
    format!("\x1b[2m{}\x1b[0m", s)
}
fn color_gray(s: &str) -> String {
    format!("\x1b[90m{}\x1b[0m", s)
}

/// Renders a source code snippet with a highlighted caret at the error location.
/// Used for parse errors with line/column info.
fn render_source_snippet(source: &str, line: usize, col: usize) {
    // 1-based line/column expected
    if line == 0 {
        return;
    }
    if let Some(src_line) = source.lines().nth(line - 1) {
        // Simple single-line snippet with a gutter and caret (no extra '|' line for caret)
        let gutter = format!("{:>4} {} ", line, color_gray("|"));
        // Highlight character under caret
        let idx = if col > 0 { col - 1 } else { 0 };
        let mut highlighted = String::new();
        for (i, ch) in src_line.chars().enumerate() {
            if i == idx {
                highlighted.push_str(&color_bold_cyan(&ch.to_string()));
            } else {
                highlighted.push(ch);
            }
        }
        eprintln!("{}{}", gutter, highlighted);
        let caret_pos = if col > 0 { col - 1 } else { 0 };
        let mut spaces = String::new();
        // account for gutter width plus a space
        let gutter_width = 4 + 1 + 1; // digits + space + '|'
        for _ in 0..gutter_width {
            spaces.push(' ');
        }
        for _ in 0..(caret_pos + 1) {
            spaces.push(' ');
        }
        eprintln!("{}{}", spaces, color_bold_red("^"));
    }
}

/// Colorizes diagnostic messages for expected/found errors, unexpected tokens, etc.
/// Used to improve readability of error output.
fn colorize_message(msg: &str) -> String {
    if let Some(exp_idx) = msg.find("expected ") {
        if let Some(found_idx) = msg.find(", found ") {
            let before = &msg[..exp_idx + 9];
            let expected = &msg[exp_idx + 9..found_idx];
            let found = &msg[found_idx + 8..];
            return format!(
                "{}{}, {}{}",
                before,
                color_bold_green(expected.trim()),
                "found ",
                color_bold_red(found.trim())
            );
        }
    }
    if msg.starts_with("Expected ") && msg.contains(", got ") {
        let parts: Vec<&str> = msg.split(", got ").collect();
        if parts.len() == 2 {
            let expected = parts[0].trim_start_matches("Expected ");
            let got = parts[1];
            return format!(
                "Expected {}, got {}",
                color_bold_green(expected.trim()),
                color_bold_red(got.trim())
            );
        }
    }
    if msg.starts_with("Unexpected token:") {
        let tok = msg.trim_start_matches("Unexpected token:").trim();
        return format!("Unexpected token: {}", color_bold_cyan(tok));
    }
    msg.to_string()
}

/// Colorizes quoted names (e.g., variable or function names in single quotes).
/// Used for highlighting identifiers in error messages.
fn colorize_quoted_names(input: &str) -> String {
    let mut out = String::new();
    let mut in_quote = false;
    let mut buf = String::new();
    for ch in input.chars() {
        if ch == '\'' {
            if in_quote {
                // Close quote: append colored content then closing quote
                out.push_str(&color_bold_cyan(&buf));
                out.push('\'');
                buf.clear();
                in_quote = false;
            } else {
                in_quote = true;
                out.push('\''); // Fix: print opening quote
            }
        } else if in_quote {
            buf.push(ch);
        } else {
            out.push(ch);
        }
    }
    if in_quote {
        // Unbalanced quote: flush raw content and a single quote
        out.push_str(&buf);
        out.push('\'');
    }
    out
}

/// Extracts an error code from a diagnostic message (e.g., "error[E1234]: ...").
/// Returns (code, rest of message) if found.
fn extract_error_code(msg: &str) -> Option<(String, String)> {
    // Try to extract error code from message like "error[E1234]:"
    if let Some(start) = msg.find("error[") {
        if let Some(end) = msg[start..].find("]:") {
            let code = &msg[start..start + end + 2];
            let rest = msg[start + end + 2..].trim_start();
            return Some((code.to_string(), rest.to_string()));
        }
    }
    None
}

/// Colorizes semantic error messages, including error codes.
/// Used for semantic analyzer errors.
fn colorize_semantic_message(msg: &str) -> String {
    if let Some((code, rest)) = extract_error_code(msg) {
        // Only color the error code part in red
        format!("{}: {}", color_bold_red(&code), colorize_message(&rest))
    } else {
        colorize_message(msg)
    }
}

/// Prints a semantic error (from the analyzer) with colorized formatting.
pub fn print_semantic_error(err: &SemanticError) {
    let msg = err.to_string();
    if let Some((code, rest)) = extract_error_code(&msg) {
        // Only color the error code part in red
        eprintln!("{}: {}", color_bold_red(&code), colorize_message(&rest));
    } else {
        eprintln!("{}", colorize_message(&msg));
    }
}

/// Prints a parse error (from the parser) with colorized formatting.
pub fn print_parse_error(err: &ParseError) {
    let code = "error[E2001]"; // Standard parse error code
    eprintln!("{}: {}", color_bold_red(code), err);
}

/// Prints a note (additional info) in yellow.
pub fn print_note(note: &str) {
    eprintln!("{}: {}", color_bold_yellow("note"), note);
}

/// Prints a parse error with source code snippet and caret.
/// Used for errors with line/column info.
pub fn print_parse_error_with_source(err: &ParseError, source: &str, filename: &str) {
    match err {
        ParseError::UnexpectedTokenAt { msg, line, col } => {
            let loc = format!("{}:{}:{}", filename, line, col);
            let code = "error[E2001]"; // Standard parse error code
            eprintln!("{} {}", color_bold_red(code), color_dim(&loc));
            eprintln!("{}", colorize_message(msg));
            render_source_snippet(source, *line, *col);
            eprintln!("");
        }
        _ => {
            let code = "error[E2001]"; // Standard parse error code
            eprintln!("{} {}: {}", color_bold_red(code), color_dim(filename), err);
            eprintln!("");
        }
    }
}

/// Represents a single diagnostic (error or warning) record.
/// Used for grouped reporting and source annotation.
#[derive(Debug, Clone)]
pub struct DiagnosticRecord {
    pub filename: String,
    pub message: String,
    pub line: Option<usize>,
    pub col: Option<usize>,
    pub is_parse: bool,
}

/// Prints grouped diagnostics by file, with colorized output and source snippets.
/// Handles both parse and semantic errors, and annotates locations if available.
pub fn print_grouped(records: &[DiagnosticRecord], sources: &HashMap<String, String>) {
    let mut by_file: HashMap<&str, Vec<&DiagnosticRecord>> = HashMap::new();
    for r in records {
        by_file.entry(&r.filename).or_default().push(r);
    }
    for (file, recs) in by_file {
        eprintln!("\n{} {}", color_cyan("In"), color_dim(file));
        if let Some(src) = sources.get(file) {
            for r in recs {
                if r.is_parse {
                    if let (Some(line), Some(col)) = (r.line, r.col) {
                        let loc = format!("{}:{}", line, col);
                        let code = "error[E2001]"; // Standard parse error code
                        eprintln!("{} {}", color_bold_red(code), color_dim(&loc));
                        eprintln!("{}", colorize_message(&r.message));
                        render_source_snippet(src, line, col);
                        eprintln!("");
                        continue;
                    }
                }

                // Handle semantic errors
                if let Some((code, rest)) = extract_error_code(&r.message) {
                    eprintln!("{}: {}", color_bold_red(&code), colorize_message(&rest));
                } else {
                    eprintln!("{}", colorize_message(&r.message));
                }
                eprintln!("");
            }
        } else {
            // No source available for this file
            for r in recs {
                if let (Some(line), Some(_)) = (r.line, r.col) {
                    if let Some((code, rest)) = extract_error_code(&r.message) {
                        eprintln!(
                            "{} {}:{}: {}",
                            color_bold_red(&code),
                            color_dim(file),
                            line,
                            colorize_message(&rest)
                        );
                    } else {
                        let code = "error[E0000]"; // Generic error code
                        eprintln!(
                            "{} {}:{}: {}",
                            color_bold_red(code),
                            color_dim(file),
                            line,
                            colorize_message(&r.message)
                        );
                    }
                } else {
                    if let Some((code, rest)) = extract_error_code(&r.message) {
                        eprintln!("{}: {}", color_bold_red(&code), colorize_message(&rest));
                    } else {
                        eprintln!("{}", colorize_message(&r.message));
                    }
                }
            }
        }
    }
}
