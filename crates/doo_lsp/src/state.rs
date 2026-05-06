//! LSP Server State — in-memory document store and analysis cache.

use rustc_hash::FxHashMap;

/// Per-document analysis state.
pub struct DocumentState {
    /// Raw source text.
    pub text: String,
    /// Version counter (from LSP).
    pub version: i32,
    /// Parsed AST items (if parsing succeeded).
    pub ast: Option<Vec<doo_frontend::ast::Item>>,
    /// Parse errors.
    pub parse_errors: Vec<ParseError>,
    /// Symbol definitions in this file.
    pub symbols: Vec<SymbolDef>,
}

/// A parse error with position info.
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

/// A symbol definition found in source.
#[derive(Debug, Clone)]
pub struct SymbolDef {
    /// Symbol name.
    pub name: String,
    /// Kind of symbol.
    pub kind: SymbolKind,
    /// Line number (0-based).
    pub line: u32,
    /// Column (0-based).
    pub col: u32,
    /// Type information (if resolved).
    pub type_info: Option<String>,
    /// Documentation comment.
    pub doc: Option<String>,
    /// Parameters (for functions).
    pub params: Vec<ParamInfo>,
    /// Return type (for functions).
    pub return_type: Option<String>,
}

/// Parameter info for function signatures.
#[derive(Debug, Clone)]
pub struct ParamInfo {
    pub name: String,
    pub type_name: Option<String>,
}

/// Symbol kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    EnumVariant,
    Field,
    Variable,
    Import,
}

/// Global LSP server state.
pub struct ServerState {
    /// Open documents: URI string → state.
    pub documents: FxHashMap<String, DocumentState>,
    /// Workspace root paths.
    pub workspace_roots: Vec<String>,
}

impl ServerState {
    /// Create a new empty server state.
    pub fn new() -> Self {
        Self {
            documents: FxHashMap::default(),
            workspace_roots: Vec::new(),
        }
    }

    /// Update a document's text and re-analyze.
    pub fn update_document(&mut self, uri: &str, text: String, version: i32) {
        let (ast, parse_errors, symbols) = parse_and_extract(&text);

        let state = DocumentState {
            text,
            version,
            ast,
            parse_errors,
            symbols,
        };

        self.documents.insert(uri.to_string(), state);
    }

    /// Remove a document from the cache.
    pub fn remove_document(&mut self, uri: &str) {
        self.documents.remove(uri);
    }

    /// Find a symbol definition across all open documents.
    pub fn find_definition(&self, name: &str) -> Option<(&str, &SymbolDef)> {
        for (uri, doc) in &self.documents {
            for sym in &doc.symbols {
                if sym.name == name {
                    return Some((uri.as_str(), sym));
                }
            }
        }
        None
    }

    /// Get all symbols matching a prefix (for completions).
    pub fn symbols_with_prefix(&self, prefix: &str) -> Vec<(&str, &SymbolDef)> {
        let mut results = Vec::new();
        let prefix_lower = prefix.to_lowercase();
        for (uri, doc) in &self.documents {
            for sym in &doc.symbols {
                if sym.name.to_lowercase().starts_with(&prefix_lower) {
                    results.push((uri.as_str(), sym));
                }
            }
        }
        results
    }

    /// Get all symbols in a specific document.
    pub fn document_symbols(&self, uri: &str) -> &[SymbolDef] {
        self.documents
            .get(uri)
            .map(|d| d.symbols.as_slice())
            .unwrap_or(&[])
    }
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse source text and extract symbols using the compiler frontend.
fn parse_and_extract(
    text: &str,
) -> (
    Option<Vec<doo_frontend::ast::Item>>,
    Vec<ParseError>,
    Vec<SymbolDef>,
) {
    use doo_core::LineIndex;

    let line_index = LineIndex::new(text);
    let mut parser = doo_frontend::Parser::new(text, 0);
    let mut parse_errors = Vec::new();
    let mut symbols = Vec::new();

    match parser.parse_program() {
        Ok(program) => {
            // Collect any non-fatal errors
            for err in parser.errors() {
                let (line, col) = line_index.line_col(err.span.start);
                let (end_line, end_col) = line_index.line_col(err.span.end.max(err.span.start + 1));
                parse_errors.push(ParseError {
                    message: format!("{}", err),
                    line: line.saturating_sub(1),
                    column: col.saturating_sub(1),
                    end_line: end_line.saturating_sub(1),
                    end_column: end_col.saturating_sub(1),
                });
            }

            // Extract symbols from AST
            for item in &program.items {
                extract_symbols_from_item(item, &line_index, &mut symbols);
            }

            // Always run type checking when parsing succeeds (even with non-fatal warnings).
            // catch_unwind in run_type_check prevents LSP crash if analysis panics.
            run_type_check(&program, &line_index, &mut parse_errors);

            (Some(program.items), parse_errors, symbols)
        }
        Err(e) => {
            let (line, col) = line_index.line_col(e.span.start);
            let (end_line, end_col) = line_index.line_col(e.span.end.max(e.span.start + 1));
            parse_errors.push(ParseError {
                message: format!("{}", e),
                line: line.saturating_sub(1),
                column: col.saturating_sub(1),
                end_line: end_line.saturating_sub(1),
                end_column: end_col.saturating_sub(1),
            });

            // Also grab any accumulated errors
            for err in parser.errors() {
                let (line, col) = line_index.line_col(err.span.start);
                let (end_line, end_col) = line_index.line_col(err.span.end.max(err.span.start + 1));
                parse_errors.push(ParseError {
                    message: format!("{}", err),
                    line: line.saturating_sub(1),
                    column: col.saturating_sub(1),
                    end_line: end_line.saturating_sub(1),
                    end_column: end_col.saturating_sub(1),
                });
            }

            (None, parse_errors, symbols)
        }
    }
}

/// Run HIR lowering + type checking on a parsed program.
/// Collects type errors, scope errors, and direct compiler errors as diagnostics.
fn run_type_check(
    program: &doo_frontend::ast::Program,
    line_index: &doo_core::LineIndex,
    errors: &mut Vec<ParseError>,
) {
    use doo_frontend::ast::{FunctionDecl, Item};
    use std::sync::Arc;

    // Build a modified program that wraps top-level statements in a synthetic
    // function so the HIR lowerer (which skips Item::Statement) can type-check them.
    let mut items = Vec::new();
    let mut top_level_stmts = Vec::new();

    for item in &program.items {
        match item {
            Item::Statement(stmt) => top_level_stmts.push(stmt.clone()),
            other => items.push(other.clone()),
        }
    }

    if !top_level_stmts.is_empty() {
        let mut synthetic_fn = FunctionDecl::new("__lsp_check__".to_string(), program.span);
        synthetic_fn.body = top_level_stmts;
        items.push(Item::Function(synthetic_fn));
    }

    let check_program = doo_frontend::ast::Program::new(items, program.span);

    // Catch panics so a type-checker bug never crashes the LSP
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut registry = doo_core::types::TypeRegistry::new();
        let mut lowerer = doo_hir::Lower::new();
        let hir = lowerer.lower_program_typed(&check_program, &mut registry);

        let registry = Arc::new(registry);
        let mut checker = doo_analysis::TypeChecker::new(registry.clone());

        // Collect all error categories
        let mut collected: Vec<(String, doo_core::Span)> = Vec::new();

        // Type errors
        if let Err(type_errors) = checker.check(&hir) {
            for e in type_errors {
                let msg = format_type_error_msg(&e.kind, &registry);
                collected.push((msg, e.span));
            }
        }

        // Scope errors (undeclared variables, redeclarations)
        for e in checker.take_scope_errors() {
            let (msg, span) = match &e {
                doo_analysis::ScopeError::Redeclaration { redeclared, .. } => {
                    (e.message(), *redeclared)
                }
                doo_analysis::ScopeError::Undeclared { span, .. } => (e.message(), *span),
            };
            collected.push((msg, span));
        }

        // Direct compiler errors (missing return, unreachable code, assign-to-immutable)
        for e in checker.take_direct_errors() {
            collected.push((e.message.clone(), e.span));
        }

        collected
    }));

    if let Ok(analysis_errors) = result {
        for (msg, span) in analysis_errors {
            let (line, col) = line_index.line_col(span.start);
            let (end_line, end_col) = line_index.line_col(span.end.max(span.start + 1));
            errors.push(ParseError {
                message: msg,
                line: line.saturating_sub(1),
                column: col.saturating_sub(1),
                end_line: end_line.saturating_sub(1),
                end_column: end_col.saturating_sub(1),
            });
        }
    }
}

/// Format a type error kind into a human-readable message using the registry
/// for proper type names instead of raw TypeId numbers.
fn format_type_error_msg(
    kind: &doo_analysis::TypeErrorKind,
    registry: &doo_core::types::TypeRegistry,
) -> String {
    use doo_analysis::TypeErrorKind;

    match kind {
        TypeErrorKind::Mismatch { expected, found } => {
            format!(
                "type mismatch: expected `{}`, found `{}`",
                registry.display_name(*expected),
                registry.display_name(*found)
            )
        }
        TypeErrorKind::Undefined(name, suggestion) => match suggestion {
            Some(s) => format!("'{}' is not defined — did you mean '{}'?", name, s),
            None => format!("'{}' is not defined", name),
        },
        TypeErrorKind::UndefinedFunction(name) => {
            format!("function '{}' is not defined", name)
        }
        TypeErrorKind::UndefinedType(name) => {
            format!("type '{}' is not defined", name)
        }
        TypeErrorKind::UndefinedField { type_name, field } => {
            format!("no field '{}' on type '{}'", field, type_name)
        }
        TypeErrorKind::UndefinedMethod { type_name, method } => {
            format!("no method '{}' on type '{}'", method, type_name)
        }
        TypeErrorKind::UndefinedVariant { enum_name, variant } => {
            format!("no variant '{}' in enum '{}'", variant, enum_name)
        }
        TypeErrorKind::InvalidOp(msg) => msg.clone(),
        TypeErrorKind::ArgMismatch { expected, found } => {
            format!("expected {} argument(s), found {}", expected, found)
        }
        TypeErrorKind::ReturnTypeMismatch {
            function,
            expected,
            found,
        } => {
            format!(
                "return type mismatch in '{}': expected `{}`, found `{}`",
                function,
                registry.display_name(*expected),
                registry.display_name(*found)
            )
        }
        TypeErrorKind::Incompatible {
            left,
            right,
            operation,
        } => {
            format!(
                "incompatible types `{}` and `{}` for '{}'",
                registry.display_name(*left),
                registry.display_name(*right),
                operation
            )
        }
        TypeErrorKind::InvalidCondition { found } => {
            format!(
                "condition must be Bool, found `{}`",
                registry.display_name(*found)
            )
        }
        TypeErrorKind::InvalidCast { from, to } => {
            format!(
                "cannot cast `{}` to `{}`",
                registry.display_name(*from),
                registry.display_name(*to)
            )
        }
        TypeErrorKind::CannotConvert { from, to } => {
            format!(
                "cannot convert `{}` to `{}`",
                registry.display_name(*from),
                registry.display_name(*to)
            )
        }
        // Fallback: use Debug formatting for less common variants
        _ => format!("{:?}", kind),
    }
}

/// Extract symbol definitions from an AST item.
fn extract_symbols_from_item(
    item: &doo_frontend::ast::Item,
    line_index: &doo_core::LineIndex,
    symbols: &mut Vec<SymbolDef>,
) {
    use doo_frontend::ast::Item;

    match item {
        Item::Function(func) => {
            let (line, col) = line_index.line_col(func.span.start);
            let params: Vec<ParamInfo> = func
                .params
                .iter()
                .map(|(name, type_expr)| ParamInfo {
                    name: name.clone(),
                    type_name: type_expr.as_ref().map(|t| format!("{:?}", t.kind)),
                })
                .collect();

            let return_type = func.return_type.as_ref().map(|t| format!("{:?}", t.kind));

            symbols.push(SymbolDef {
                name: func.name.clone(),
                kind: SymbolKind::Function,
                line: line.saturating_sub(1),
                col: col.saturating_sub(1),
                type_info: return_type.clone(),
                doc: None,
                params,
                return_type,
            });
        }
        Item::Struct(s) => {
            let (line, col) = line_index.line_col(s.span.start);
            symbols.push(SymbolDef {
                name: s.name.clone(),
                kind: SymbolKind::Struct,
                line: line.saturating_sub(1),
                col: col.saturating_sub(1),
                type_info: None,
                doc: None,
                params: Vec::new(),
                return_type: None,
            });

            for field in &s.fields {
                let (fl, fc) = line_index.line_col(field.span.start);
                symbols.push(SymbolDef {
                    name: field.name.clone(),
                    kind: SymbolKind::Field,
                    line: fl.saturating_sub(1),
                    col: fc.saturating_sub(1),
                    type_info: Some(format!("{:?}", field.type_expr.kind)),
                    doc: None,
                    params: Vec::new(),
                    return_type: None,
                });
            }
        }
        Item::Enum(e) => {
            let (line, col) = line_index.line_col(e.span.start);
            symbols.push(SymbolDef {
                name: e.name.clone(),
                kind: SymbolKind::Enum,
                line: line.saturating_sub(1),
                col: col.saturating_sub(1),
                type_info: None,
                doc: None,
                params: Vec::new(),
                return_type: None,
            });

            for variant in &e.variants {
                let (vl, vc) = line_index.line_col(variant.span.start);
                symbols.push(SymbolDef {
                    name: variant.name.clone(),
                    kind: SymbolKind::EnumVariant,
                    line: vl.saturating_sub(1),
                    col: vc.saturating_sub(1),
                    type_info: variant.payload.as_ref().map(|t| format!("{:?}", t.kind)),
                    doc: None,
                    params: Vec::new(),
                    return_type: None,
                });
            }
        }
        Item::Import(imp) => {
            let (line, col) = line_index.line_col(imp.span.start);
            let path_str = imp.path.join("::");
            symbols.push(SymbolDef {
                name: path_str,
                kind: SymbolKind::Import,
                line: line.saturating_sub(1),
                col: col.saturating_sub(1),
                type_info: None,
                doc: None,
                params: Vec::new(),
                return_type: None,
            });
        }
        Item::Statement(_) => {
            // Top-level let bindings could be extracted later
        }
        Item::Policy(_) => {
            // Policy blocks are not exposed as LSP symbols
        }
        Item::Interface(_) => {
            // Interface definitions are not exposed as LSP symbols
        }
    }
}
