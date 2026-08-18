//! Macro Expansion Hook — walks AST, invokes macros, re-parses output.
//!
//! Lives in `doo_driver` (not `doo_macro`) because it needs access to
//! `doo_frontend` for AST types and Parser. The `doo_macro` crate
//! itself only sees TokenStream — never AST.

use doo_core::errors::codes::{CompilerError, ErrorCode};
use doo_core::span::FileId;
use doo_core::{Span, Symbol};
use doo_frontend::ast::{Decorator, Item};
use doo_frontend::Parser;
use doo_macro::{Macro, MacroContext, MacroError, MacroRegistry, TokenStream};
use doo_session::CompileSession;

/// Expand all macro invocations in the AST.
///
/// Walks each top-level item, looking for `@decorator(...)` annotations.
/// For each decorator, looks up the macro in the registry. If found,
/// the item is converted to a TokenStream, the macro is invoked, and
/// the output is re-parsed as ordinary Doolang AST.
///
/// Pipeline placement: strictly between Parse (step 2) and Type Check (step 3).
/// Expansion order: declaration order (deterministic).
pub fn expand_macros(
    registry: &MacroRegistry,
    items: Vec<Item>,
    session: &CompileSession,
    file_id: FileId,
    source: &str,
) -> Result<Vec<Item>, Vec<CompilerError>> {
    if registry.is_empty() {
        return Ok(items);
    }

    let mut result = Vec::with_capacity(items.len());
    let mut errors = Vec::new();

    for item in items {
        let decorators = get_decorators(&item);

        if decorators.is_empty() {
            result.push(item);
            continue;
        }

        let mut expanded = false;

        for decorator in &decorators {
            let macro_name = Symbol::intern(&decorator.name);

            if let Some(macro_impl) = registry.get(macro_name) {
                match expand_single_item(&item, macro_impl, decorator, file_id, source) {
                    Ok(expanded_items) => {
                        result.extend(expanded_items);
                        expanded = true;
                        break;
                    }
                    Err(e) => {
                        errors.push(e);
                        expanded = true;
                        break;
                    }
                }
            }
        }

        if !expanded {
            for decorator in &decorators {
                let macro_name = Symbol::intern(&decorator.name);
                if !registry.has(macro_name) {
                    errors.push(CompilerError::new(
                        ErrorCode::InvalidDecorator,
                        format!(
                            "unknown decorator '{}' — no macro crate provides this decorator",
                            decorator.name
                        ),
                        decorator.span,
                    ));
                }
            }
            result.push(item);
        }
    }

    if errors.is_empty() {
        Ok(result)
    } else {
        Err(errors)
    }
}

/// Expand a single item using the given macro.
fn expand_single_item(
    item: &Item,
    macro_impl: &dyn Macro,
    decorator: &Decorator,
    file_id: FileId,
    source: &str,
) -> Result<Vec<Item>, CompilerError> {
    let item_span = item.span();

    let token_input = extract_token_stream(item_span, source);

    let ctx = MacroContext {
        crate_name: Symbol::intern("doo"),
        macro_name: Symbol::intern(&decorator.name),
        source_file: file_id,
        invocation_span: decorator.span,
    };

    let output = macro_impl.expand(token_input, &ctx).map_err(|e| {
        CompilerError::new(
            ErrorCode::InvalidDecorator,
            format!("macro '{}' expansion failed: {}", decorator.name, e),
            decorator.span,
        )
    })?;

    let output_source = output.to_string();

    if output_source.is_empty() {
        return Ok(Vec::new());
    }

    let mut parser = Parser::new(&output_source, file_id.0);
    match parser.parse_program() {
        Ok(program) => Ok(program.items),
        Err(parse_errors) => {
            let first = parse_errors
                .first()
                .map(|e| e.message.clone())
                .unwrap_or_default();
            Err(CompilerError::new(
                ErrorCode::InvalidDecorator,
                format!(
                    "macro '{}' output failed to parse: {}",
                    decorator.name, first
                ),
                decorator.span,
            ))
        }
    }
}

/// Extract the token stream for an item by getting its source text.
///
/// Uses the item's span to slice the source text, then tokenizes it.
/// The span starts after any decorators (the parser sets it at the
/// first keyword like `fn`, `struct`, `enum`).
fn extract_token_stream(item_span: Span, source: &str) -> TokenStream {
    let start = item_span.start as usize;
    let end = item_span.end as usize;

    if start >= source.len() || end > source.len() || start >= end {
        return TokenStream::new();
    }

    let source_text = &source[start..end];
    TokenStream::from_str(source_text)
}

/// Get all decorators from an AST item.
fn get_decorators(item: &Item) -> Vec<&Decorator> {
    match item {
        Item::Function(f) => f.decorators.iter().collect(),
        Item::Struct(s) => s.decorators.iter().collect(),
        Item::Impl(i) => i.decorators.iter().collect(),
        _ => Vec::new(),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use doo_frontend::ast::{FunctionDecl, Program};
    use doo_frontend::Parser;

    #[test]
    fn test_expand_macros_empty_registry() {
        let registry = MacroRegistry::new();
        let items = vec![];
        let session = create_test_session();
        let source = "";

        let result = expand_macros(&registry, items, &session, FileId::new(0), source);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_expand_macros_no_decorators() {
        let source = "fn foo() { print(1) }";
        let mut parser = Parser::new(source, 0);
        let program = parser.parse_program().unwrap();

        let registry = MacroRegistry::new();
        let session = create_test_session();

        let result = expand_macros(&registry, program.items, &session, FileId::new(0), source);

        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 1);
    }

    #[test]
    fn test_extract_token_stream() {
        let source = "fn foo() { }";
        let span = Span::new(0, source.len() as u32);
        let stream = extract_token_stream(span, source);
        assert!(!stream.is_empty());
        assert!(stream.to_string().contains("fn"));
    }

    #[test]
    fn test_extract_token_stream_empty() {
        let stream = extract_token_stream(Span::new(0, 0), "");
        assert!(stream.is_empty());
    }

    fn create_test_session() -> CompileSession {
        use doo_session::{CompileOptions, ProjectPaths, TargetTriple};
        use std::path::PathBuf;

        CompileSession {
            options: CompileOptions::default(),
            paths: ProjectPaths {
                root: PathBuf::from("."),
                src: PathBuf::from("./src"),
                out: PathBuf::from("./target"),
            },
            target: TargetTriple::host(),
            stdlib_path: PathBuf::new(),
            package_graph: doo_session::PackageGraph::default(),
            source_map: std::rc::Rc::new(
                std::cell::RefCell::new(doo_diagnostics::SourceMap::new()),
            ),
            interner: doo_core::intern::Interner::new(),
            arena: doo_core::arena::CompilerArena::new(),
            type_registry: doo_core::types::TypeRegistry::new(),
            query_cache: doo_core::query::QueryCache::new(),
            diagnostics: doo_diagnostics::DiagnosticEmitter::new(true),
            errors: Vec::new(),
            file_id_map: std::collections::HashMap::new(),
        }
    }
}
