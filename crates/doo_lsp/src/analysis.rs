//! Analysis bridge — connects LSP to the compiler's semantic analysis.
//!
//! Provides helpers for hover info, go-to-definition, completions, and
//! document symbols using the parsed AST and state.

use crate::state::{ServerState, SymbolDef, SymbolKind};

/// Hover information for a position.
pub struct HoverInfo {
    pub contents: String,
}

/// A completion item.
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    pub insert_text: Option<String>,
}

/// Completion kinds (maps to lsp_types::CompletionItemKind).
#[derive(Debug, Clone, Copy)]
pub enum CompletionKind {
    Function,
    Struct,
    Enum,
    EnumMember,
    Field,
    Variable,
    Constant,
    Keyword,
    Module,
}

/// Doo language keywords for completion.
const KEYWORDS: &[&str] = &[
    "fn", "struct", "enum", "import", "let", "mut", "if", "else", "for", "in", "while", "loop",
    "break", "continue", "return", "match", "const", "true", "false", "nil", "async", "await", "try",
    "catch", "throw", "spawn",
];

/// Built-in types for completion.
const BUILTIN_TYPES: &[&str] = &[
    "int", "float", "string", "bool", "void", "Map", "Array", "Optional",
];

/// Built-in functions/methods for completion.
const BUILTIN_FUNCTIONS: &[&str] = &[
    "print",
    "println",
    "len",
    "push",
    "pop",
    "contains",
    "keys",
    "values",
    "toString",
    "toInt",
    "toFloat",
    "split",
    "trim",
    "replace",
    "startsWith",
    "endsWith",
    "substring",
    "charAt",
    "indexOf",
];

/// Get hover info for a symbol at a given position.
pub fn hover_at(state: &ServerState, uri: &str, line: u32, col: u32) -> Option<HoverInfo> {
    let doc = state.documents.get(uri)?;

    // Find the symbol at or near this position
    let symbol = find_symbol_at(&doc.symbols, line, col)?;

    let contents = format_symbol_hover(symbol);
    Some(HoverInfo { contents })
}

/// Get completions for a position/prefix.
pub fn completions(state: &ServerState, prefix: &str) -> Vec<CompletionItem> {
    let mut items = Vec::new();

    // Keywords
    for kw in KEYWORDS {
        if kw.starts_with(prefix) || prefix.is_empty() {
            items.push(CompletionItem {
                label: kw.to_string(),
                kind: CompletionKind::Keyword,
                detail: Some("keyword".to_string()),
                documentation: None,
                insert_text: None,
            });
        }
    }

    // Built-in types
    for ty in BUILTIN_TYPES {
        if ty.starts_with(prefix) || prefix.is_empty() {
            items.push(CompletionItem {
                label: ty.to_string(),
                kind: CompletionKind::Struct,
                detail: Some("built-in type".to_string()),
                documentation: None,
                insert_text: None,
            });
        }
    }

    // Built-in functions
    for func in BUILTIN_FUNCTIONS {
        if func.starts_with(prefix) || prefix.is_empty() {
            items.push(CompletionItem {
                label: func.to_string(),
                kind: CompletionKind::Function,
                detail: Some("built-in function".to_string()),
                documentation: None,
                insert_text: Some(format!("{}($0)", func)),
            });
        }
    }

    // User-defined symbols from all open documents
    let matches = state.symbols_with_prefix(prefix);
    for (_uri, sym) in matches {
        items.push(symbol_to_completion(sym));
    }

    items
}

/// Get document symbols (outline) for a URI.
pub fn document_symbols<'a>(state: &'a ServerState, uri: &str) -> Vec<&'a SymbolDef> {
    state.document_symbols(uri).iter().collect()
}

/// Find definition of a symbol by name.
pub fn find_definition<'a>(state: &'a ServerState, name: &str) -> Option<(&'a str, &'a SymbolDef)> {
    state.find_definition(name)
}

/// Get signature help for a function at position.
pub fn signature_help<'a>(state: &'a ServerState, func_name: &str) -> Option<(&'a SymbolDef,)> {
    // Find the function in all documents
    for (_uri, doc) in &state.documents {
        for sym in &doc.symbols {
            if sym.name == func_name && sym.kind == SymbolKind::Function {
                return Some((sym,));
            }
        }
    }
    None
}

// === Internal helpers ===

/// Find the symbol closest to a position.
fn find_symbol_at(symbols: &[SymbolDef], line: u32, _col: u32) -> Option<&SymbolDef> {
    // Simple approach: find a symbol on the same line
    // A more advanced version would use byte offsets and span ranges
    symbols.iter().find(|s| s.line == line)
}

/// Format a symbol for hover display.
fn format_symbol_hover(sym: &SymbolDef) -> String {
    match sym.kind {
        SymbolKind::Function => {
            let params_str = sym
                .params
                .iter()
                .map(|p| match &p.type_name {
                    Some(ty) => format!("{}: {}", p.name, ty),
                    None => p.name.clone(),
                })
                .collect::<Vec<_>>()
                .join(", ");

            let ret = sym.return_type.as_deref().unwrap_or("void");

            format!("```doo\nfn {}({}) -> {}\n```", sym.name, params_str, ret)
        }
        SymbolKind::Struct => {
            format!("```doo\nstruct {}\n```", sym.name)
        }
        SymbolKind::Enum => {
            format!("```doo\nenum {}\n```", sym.name)
        }
        SymbolKind::EnumVariant => {
            format!("```doo\n{}\n```\n\nEnum variant", sym.name)
        }
        SymbolKind::Field => {
            let ty = sym.type_info.as_deref().unwrap_or("unknown");
            format!("```doo\n{}: {}\n```\n\nStruct field", sym.name, ty)
        }
        SymbolKind::Variable => {
            let ty = sym.type_info.as_deref().unwrap_or("unknown");
            format!("```doo\nlet {}: {}\n```", sym.name, ty)
        }
        SymbolKind::Import => {
            format!("```doo\nimport {}\n```", sym.name)
        }
        SymbolKind::Const => {
            format!("```doo\nconst {} = ...\n```", sym.name)
        }
    }
}

/// Convert a SymbolDef to a CompletionItem.
fn symbol_to_completion(sym: &SymbolDef) -> CompletionItem {
    let (kind, detail) = match sym.kind {
        SymbolKind::Function => {
            let sig = format_function_signature(sym);
            (CompletionKind::Function, Some(sig))
        }
        SymbolKind::Struct => (CompletionKind::Struct, Some("struct".to_string())),
        SymbolKind::Enum => (CompletionKind::Enum, Some("enum".to_string())),
        SymbolKind::EnumVariant => (CompletionKind::EnumMember, Some("variant".to_string())),
        SymbolKind::Field => (CompletionKind::Field, sym.type_info.clone()),
        SymbolKind::Variable => (CompletionKind::Variable, sym.type_info.clone()),
        SymbolKind::Import => (CompletionKind::Module, None),
        SymbolKind::Const => (CompletionKind::Constant, None),
    };

    let insert_text = if sym.kind == SymbolKind::Function {
        Some(format!("{}($0)", sym.name))
    } else {
        None
    };

    CompletionItem {
        label: sym.name.clone(),
        kind,
        detail,
        documentation: sym.doc.clone(),
        insert_text,
    }
}

/// Format a function signature for display (public API for handler).
pub fn format_function_signature_external(sym: &SymbolDef) -> String {
    format_function_signature(sym)
}

/// Format a function signature for display.
fn format_function_signature(sym: &SymbolDef) -> String {
    let params_str = sym
        .params
        .iter()
        .map(|p| match &p.type_name {
            Some(ty) => format!("{}: {}", p.name, ty),
            None => p.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ");

    let ret = sym.return_type.as_deref().unwrap_or("void");
    format!("fn {}({}) -> {}", sym.name, params_str, ret)
}
