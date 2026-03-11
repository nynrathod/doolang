//! Diagnostics — converts compiler errors to LSP Diagnostic objects.

use crate::state::ServerState;
use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

/// Publish diagnostics for a document.
/// Returns a list of LSP Diagnostics built from the document's parse errors.
pub fn diagnostics_for_document(state: &ServerState, uri: &str) -> Vec<Diagnostic> {
    let doc = match state.documents.get(uri) {
        Some(d) => d,
        None => return Vec::new(),
    };

    let mut diagnostics = Vec::new();

    for err in &doc.parse_errors {
        let start = Position::new(err.line, err.column);
        let end = Position::new(err.end_line, err.end_column);

        diagnostics.push(Diagnostic {
            range: Range::new(start, end),
            severity: Some(DiagnosticSeverity::ERROR),
            code: None,
            code_description: None,
            source: Some("doo".to_string()),
            message: err.message.clone(),
            related_information: None,
            tags: None,
            data: None,
        });
    }

    diagnostics
}
