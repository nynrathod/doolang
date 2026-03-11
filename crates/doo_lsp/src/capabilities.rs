//! LSP Capabilities — declares what the server supports.

use lsp_types::{
    CompletionOptions, HoverProviderCapability, OneOf, SaveOptions, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
    TextDocumentSyncSaveOptions,
};

/// Build the server capabilities sent during initialization.
pub fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        // Sync: Send full document text on each change
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::FULL),
                will_save: None,
                will_save_wait_until: None,
                save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                    include_text: Some(true),
                })),
            },
        )),

        // Go-to-definition
        definition_provider: Some(OneOf::Left(true)),

        // Hover (type info)
        hover_provider: Some(HoverProviderCapability::Simple(true)),

        // Completions
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![".".to_string(), ":".to_string(), "@".to_string()]),
            resolve_provider: Some(false),
            ..Default::default()
        }),

        // Document symbols (outline)
        document_symbol_provider: Some(OneOf::Left(true)),

        // Signature help
        signature_help_provider: Some(lsp_types::SignatureHelpOptions {
            trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
            retrigger_characters: None,
            work_done_progress_options: Default::default(),
        }),

        ..Default::default()
    }
}
