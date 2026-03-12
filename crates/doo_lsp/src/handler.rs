//! LSP Handler — main event loop and request/notification dispatching.

use std::str::FromStr;

use anyhow::Result;
use lsp_server::{Connection, ExtractError, Message, Notification, Request, RequestId, Response};
use lsp_types::{
    notification::{
        DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
        PublishDiagnostics,
    },
    request::{
        Completion, DocumentSymbolRequest, GotoDefinition, HoverRequest, SignatureHelpRequest,
    },
    CompletionItem, CompletionItemKind, CompletionList, CompletionResponse, DocumentSymbol,
    DocumentSymbolResponse, GotoDefinitionResponse, Hover, HoverContents, InitializeParams,
    Location, MarkupContent, MarkupKind, ParameterInformation, Position, PublishDiagnosticsParams,
    Range, SignatureHelp, SignatureInformation, SymbolKind as LspSymbolKind, Uri,
};

use crate::analysis;
use crate::diagnostics;
use crate::state::ServerState;

/// Run the main LSP event loop.
pub fn main_loop(connection: &Connection, init_params: InitializeParams) -> Result<()> {
    let mut state = ServerState::new();

    // Store workspace roots
    if let Some(folders) = &init_params.workspace_folders {
        for folder in folders {
            state.workspace_roots.push(folder.uri.to_string());
        }
    } else if let Some(root_uri) = &init_params.root_uri {
        state.workspace_roots.push(root_uri.to_string());
    }

    tracing::info!(
        "Entering main loop with {} workspace roots",
        state.workspace_roots.len()
    );

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                handle_request(&connection, &mut state, req);
            }
            Message::Notification(not) => {
                handle_notification(&connection, &mut state, not);
            }
            Message::Response(_resp) => {
                // We don't send requests to the client, so no responses to handle
            }
        }
    }

    Ok(())
}

/// Dispatch an LSP request.
fn handle_request(connection: &Connection, state: &mut ServerState, req: Request) {
    let req = match cast_request::<GotoDefinition>(req) {
        Ok((id, params)) => {
            let result = handle_goto_definition(state, &params);
            let resp = Response::new_ok(id, result);
            let _ = connection.sender.send(Message::Response(resp));
            return;
        }
        Err(req) => req,
    };

    let req = match cast_request::<HoverRequest>(req) {
        Ok((id, params)) => {
            let result = handle_hover(state, &params);
            let resp = Response::new_ok(id, result);
            let _ = connection.sender.send(Message::Response(resp));
            return;
        }
        Err(req) => req,
    };

    let req = match cast_request::<Completion>(req) {
        Ok((id, params)) => {
            let result = handle_completion(state, &params);
            let resp = Response::new_ok(id, result);
            let _ = connection.sender.send(Message::Response(resp));
            return;
        }
        Err(req) => req,
    };

    let req = match cast_request::<DocumentSymbolRequest>(req) {
        Ok((id, params)) => {
            let result = handle_document_symbols(state, &params);
            let resp = Response::new_ok(id, result);
            let _ = connection.sender.send(Message::Response(resp));
            return;
        }
        Err(req) => req,
    };

    let _req = match cast_request::<SignatureHelpRequest>(req) {
        Ok((id, params)) => {
            let result = handle_signature_help(state, &params);
            let resp = Response::new_ok(id, result);
            let _ = connection.sender.send(Message::Response(resp));
            return;
        }
        Err(req) => req,
    };

    // Unknown request — just log it
    tracing::warn!("Unhandled request: {}", _req.method);
}

/// Dispatch an LSP notification.
fn handle_notification(connection: &Connection, state: &mut ServerState, not: Notification) {
    let not = match cast_notification::<DidOpenTextDocument>(not) {
        Ok(params) => {
            let uri = params.text_document.uri.to_string();
            state.update_document(
                &uri,
                params.text_document.text,
                params.text_document.version,
            );
            publish_diagnostics(connection, state, &uri, &params.text_document.uri);
            return;
        }
        Err(not) => not,
    };

    let not = match cast_notification::<DidChangeTextDocument>(not) {
        Ok(params) => {
            let uri = params.text_document.uri.to_string();
            // We use Full sync, so there's exactly one change with the full text
            if let Some(change) = params.content_changes.into_iter().next() {
                state.update_document(&uri, change.text, params.text_document.version);
                publish_diagnostics(connection, state, &uri, &params.text_document.uri);
            }
            return;
        }
        Err(not) => not,
    };

    let _not = match cast_notification::<DidCloseTextDocument>(not) {
        Ok(params) => {
            let uri = params.text_document.uri.to_string();
            state.remove_document(&uri);
            // Clear diagnostics on close
            let diag_params = PublishDiagnosticsParams {
                uri: params.text_document.uri,
                diagnostics: Vec::new(),
                version: None,
            };
            let _ = connection
                .sender
                .send(Message::Notification(Notification::new(
                    PublishDiagnostics::METHOD.to_string(),
                    diag_params,
                )));
            return;
        }
        Err(not) => not,
    };

    // Ignore other notifications
}

// === Request Handlers ===

fn handle_goto_definition(
    state: &ServerState,
    params: &lsp_types::GotoDefinitionParams,
) -> Option<GotoDefinitionResponse> {
    let uri = params
        .text_document_position_params
        .text_document
        .uri
        .to_string();
    let pos = params.text_document_position_params.position;

    // Get the word at the cursor position
    let word = word_at_position(state, &uri, pos)?;

    // Find the definition
    let (def_uri, sym) = analysis::find_definition(state, &word)?;

    let target_uri = Uri::from_str(def_uri).ok()?;
    let range = Range::new(
        Position::new(sym.line, sym.col),
        Position::new(sym.line, sym.col + sym.name.len() as u32),
    );

    Some(GotoDefinitionResponse::Scalar(Location::new(
        target_uri, range,
    )))
}

fn handle_hover(state: &ServerState, params: &lsp_types::HoverParams) -> Option<Hover> {
    let uri = params
        .text_document_position_params
        .text_document
        .uri
        .to_string();
    let pos = params.text_document_position_params.position;

    let info = analysis::hover_at(state, &uri, pos.line, pos.character)?;

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: info.contents,
        }),
        range: None,
    })
}

fn handle_completion(
    state: &ServerState,
    params: &lsp_types::CompletionParams,
) -> Option<CompletionResponse> {
    let uri = params.text_document_position.text_document.uri.to_string();
    let pos = params.text_document_position.position;

    // Get the prefix being typed
    let prefix = word_prefix_at_position(state, &uri, pos).unwrap_or_default();

    let items = analysis::completions(state, &prefix);

    let lsp_items: Vec<CompletionItem> = items
        .into_iter()
        .map(|item| {
            let kind = match item.kind {
                analysis::CompletionKind::Function => CompletionItemKind::FUNCTION,
                analysis::CompletionKind::Struct => CompletionItemKind::STRUCT,
                analysis::CompletionKind::Enum => CompletionItemKind::ENUM,
                analysis::CompletionKind::EnumMember => CompletionItemKind::ENUM_MEMBER,
                analysis::CompletionKind::Field => CompletionItemKind::FIELD,
                analysis::CompletionKind::Variable => CompletionItemKind::VARIABLE,
                analysis::CompletionKind::Keyword => CompletionItemKind::KEYWORD,
                analysis::CompletionKind::Module => CompletionItemKind::MODULE,
            };

            CompletionItem {
                label: item.label,
                kind: Some(kind),
                detail: item.detail,
                documentation: item.documentation.map(|d| {
                    lsp_types::Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: d,
                    })
                }),
                insert_text: item.insert_text,
                ..Default::default()
            }
        })
        .collect();

    Some(CompletionResponse::List(CompletionList {
        is_incomplete: false,
        items: lsp_items,
    }))
}

fn handle_document_symbols(
    state: &ServerState,
    params: &lsp_types::DocumentSymbolParams,
) -> Option<DocumentSymbolResponse> {
    let uri = params.text_document.uri.to_string();
    let symbols = analysis::document_symbols(state, &uri);

    let lsp_symbols: Vec<DocumentSymbol> = symbols
        .iter()
        .filter_map(|sym| {
            let kind = match sym.kind {
                crate::state::SymbolKind::Function => LspSymbolKind::FUNCTION,
                crate::state::SymbolKind::Struct => LspSymbolKind::STRUCT,
                crate::state::SymbolKind::Enum => LspSymbolKind::ENUM,
                crate::state::SymbolKind::EnumVariant => LspSymbolKind::ENUM_MEMBER,
                crate::state::SymbolKind::Field => LspSymbolKind::FIELD,
                crate::state::SymbolKind::Variable => LspSymbolKind::VARIABLE,
                crate::state::SymbolKind::Import => LspSymbolKind::MODULE,
            };

            let range = Range::new(
                Position::new(sym.line, sym.col),
                Position::new(sym.line, sym.col + sym.name.len() as u32),
            );

            #[allow(deprecated)]
            Some(DocumentSymbol {
                name: sym.name.clone(),
                detail: sym.type_info.clone(),
                kind,
                tags: None,
                deprecated: None,
                range,
                selection_range: range,
                children: None,
            })
        })
        .collect();

    Some(DocumentSymbolResponse::Nested(lsp_symbols))
}

fn handle_signature_help(
    state: &ServerState,
    params: &lsp_types::SignatureHelpParams,
) -> Option<SignatureHelp> {
    let uri = params
        .text_document_position_params
        .text_document
        .uri
        .to_string();
    let pos = params.text_document_position_params.position;

    // Find the function name at/before cursor
    let func_name = word_at_position(state, &uri, pos)?;

    let (sym,) = analysis::signature_help(state, &func_name)?;

    let params_info: Vec<ParameterInformation> = sym
        .params
        .iter()
        .map(|p| {
            let label = match &p.type_name {
                Some(ty) => format!("{}: {}", p.name, ty),
                None => p.name.clone(),
            };
            ParameterInformation {
                label: lsp_types::ParameterLabel::Simple(label),
                documentation: None,
            }
        })
        .collect();

    let sig_label = analysis::format_function_signature_external(sym);

    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label: sig_label,
            documentation: None,
            parameters: Some(params_info),
            active_parameter: None,
        }],
        active_signature: Some(0),
        active_parameter: None,
    })
}

// === Publishing diagnostics ===

fn publish_diagnostics(connection: &Connection, state: &ServerState, uri_str: &str, uri: &Uri) {
    let diags = diagnostics::diagnostics_for_document(state, uri_str);
    let params = PublishDiagnosticsParams {
        uri: uri.clone(),
        diagnostics: diags,
        version: None,
    };
    let _ = connection
        .sender
        .send(Message::Notification(Notification::new(
            PublishDiagnostics::METHOD.to_string(),
            params,
        )));
}

// === Text helpers ===

/// Get the word at a cursor position from the document text.
fn word_at_position(state: &ServerState, uri: &str, pos: Position) -> Option<String> {
    let doc = state.documents.get(uri)?;
    let text = &doc.text;

    let line = text.lines().nth(pos.line as usize)?;
    let col = pos.character as usize;

    if col > line.len() {
        return None;
    }

    // Find word boundaries
    let start = line[..col]
        .rfind(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|i| i + 1)
        .unwrap_or(0);

    let end = line[col..]
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|i| col + i)
        .unwrap_or(line.len());

    if start >= end {
        return None;
    }

    Some(line[start..end].to_string())
}

/// Get the word prefix (up to cursor) at a position.
fn word_prefix_at_position(state: &ServerState, uri: &str, pos: Position) -> Option<String> {
    let doc = state.documents.get(uri)?;
    let text = &doc.text;

    let line = text.lines().nth(pos.line as usize)?;
    let col = pos.character as usize;

    if col > line.len() {
        return None;
    }

    let start = line[..col]
        .rfind(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|i| i + 1)
        .unwrap_or(0);

    Some(line[start..col].to_string())
}

// === Cast helpers ===

/// Try to extract a typed request. Returns Err(original_req) if method doesn't match.
fn cast_request<R: lsp_types::request::Request>(
    req: Request,
) -> Result<(RequestId, R::Params), Request> {
    req.extract(R::METHOD).map_err(|e| match e {
        ExtractError::MethodMismatch(req) => req,
        ExtractError::JsonError { method, error } => {
            tracing::error!("Failed to deserialize {}: {}", method, error);
            panic!("malformed request")
        }
    })
}

/// Try to extract a typed notification. Returns Err(original_not) if method doesn't match.
fn cast_notification<N: lsp_types::notification::Notification>(
    not: Notification,
) -> Result<N::Params, Notification> {
    not.extract(N::METHOD).map_err(|e| match e {
        ExtractError::MethodMismatch(not) => not,
        ExtractError::JsonError { method, error } => {
            tracing::error!("Failed to deserialize notification {}: {}", method, error);
            panic!("malformed notification")
        }
    })
}
