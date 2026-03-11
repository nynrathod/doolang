/**
 * Doo Language Server
 *
 * Provides:
 * - Completions (keywords, types, std library, struct fields, enum variants)
 * - Go-to-definition (Ctrl+Click) — scans all .doo files in workspace
 * - Hover information
 * - Diagnostics (bracket matching, unclosed strings, unknown decorators)
 * - Document symbols (outline)
 * - Signature help
 */

import * as fs from 'fs';
import * as path from 'path';
import {
    createConnection,
    TextDocuments,
    ProposedFeatures,
    InitializeParams,
    InitializeResult,
    TextDocumentSyncKind,
    CompletionParams,
    DefinitionParams,
    HoverParams,
    DocumentSymbolParams,
    SignatureHelpParams,
    SignatureHelp,
    SignatureInformation,
    ParameterInformation,
    DidChangeConfigurationNotification,
} from 'vscode-languageserver/node';
import { TextDocument } from 'vscode-languageserver-textdocument';
import { parseDooSource, DooParseResult } from './parser';
import { getCompletions } from './completions';
import { getDocumentSymbols, findDefinition } from './symbols';
import { getDiagnostics } from './diagnostics';
import { getHoverInfo } from './hover';

// Create connection and document manager
const connection = createConnection(ProposedFeatures.all);
const documents: TextDocuments<TextDocument> = new TextDocuments(TextDocument);

// Cache parse results per document URI
const parseCache = new Map<string, DooParseResult>();

// Workspace root folders
let workspaceFolders: string[] = [];

// ─── Initialization ───

connection.onInitialize((params: InitializeParams): InitializeResult => {
    // Capture workspace folders for scanning .doo files
    if (params.workspaceFolders) {
        workspaceFolders = params.workspaceFolders.map((f) => {
            // Convert URI to file path
            try {
                const url = new URL(f.uri);
                return decodeURIComponent(url.pathname).replace(/^\/([a-zA-Z]:)/, '$1');
            } catch {
                return f.uri;
            }
        });
    } else if (params.rootUri) {
        try {
            const url = new URL(params.rootUri);
            workspaceFolders = [decodeURIComponent(url.pathname).replace(/^\/([a-zA-Z]:)/, '$1')];
        } catch {
            workspaceFolders = [];
        }
    }

    return {
        capabilities: {
            textDocumentSync: TextDocumentSyncKind.Full,
            completionProvider: {
                resolveProvider: false,
                triggerCharacters: ['.', ':', '@', '"'],
            },
            definitionProvider: true,
            hoverProvider: true,
            documentSymbolProvider: true,
            signatureHelpProvider: {
                triggerCharacters: ['(', ','],
            },
        },
    };
});

connection.onInitialized(() => {
    connection.client.register(DidChangeConfigurationNotification.type, undefined);

    // Scan all .doo files in workspace on startup
    for (const folder of workspaceFolders) {
        scanDooFiles(folder);
    }
    connection.console.log(`Doo LSP: Indexed ${parseCache.size} .doo files from workspace`);
});

// ─── Workspace file scanning ───

function scanDooFiles(dir: string): void {
    try {
        const entries = fs.readdirSync(dir, { withFileTypes: true });
        for (const entry of entries) {
            const fullPath = path.join(dir, entry.name);
            if (entry.isDirectory()) {
                // Skip common non-source directories
                if (['node_modules', 'target', '.git', 'out', '__test_repos'].includes(entry.name)) continue;
                scanDooFiles(fullPath);
            } else if (entry.isFile() && entry.name.endsWith('.doo')) {
                try {
                    const text = fs.readFileSync(fullPath, 'utf-8');
                    const uri = pathToUri(fullPath);
                    const result = parseDooSource(text);
                    parseCache.set(uri, result);
                } catch {
                    // Skip files that can't be read
                }
            }
        }
    } catch {
        // Skip directories that can't be read
    }
}

function pathToUri(filePath: string): string {
    // Normalize path separators and create file:// URI
    const normalized = filePath.replace(/\\/g, '/');
    if (/^[a-zA-Z]:/.test(normalized)) {
        return 'file:///' + normalized;
    }
    return 'file://' + normalized;
}

// ─── Document change handling ───

documents.onDidChangeContent((change) => {
    const doc = change.document;
    const text = doc.getText();
    const parseResult = parseDooSource(text);
    parseCache.set(doc.uri, parseResult);

    // Run diagnostics
    const diagnostics = getDiagnostics(text);
    connection.sendDiagnostics({ uri: doc.uri, diagnostics });
});

documents.onDidClose((e) => {
    // Don't remove from cache — keep for cross-file go-to-definition
    connection.sendDiagnostics({ uri: e.document.uri, diagnostics: [] });
});

// ─── Completions ───

connection.onCompletion((params: CompletionParams) => {
    const doc = documents.get(params.textDocument.uri);
    if (!doc) return [];

    const parseResult = parseCache.get(doc.uri) || parseDooSource(doc.getText());
    const line = doc.getText({
        start: { line: params.position.line, character: 0 },
        end: { line: params.position.line, character: Number.MAX_SAFE_INTEGER },
    });

    return getCompletions(parseResult, params.position, line, parseCache);
});

// ─── Go-to-definition ───

connection.onDefinition((params: DefinitionParams) => {
    const doc = documents.get(params.textDocument.uri);
    if (!doc) return null;

    const line = doc.getText({
        start: { line: params.position.line, character: 0 },
        end: { line: params.position.line, character: Number.MAX_SAFE_INTEGER },
    });

    // Get the word under cursor, but also check for qualified names like Type::Variant
    const word = getWordAtPosition(line, params.position.character);
    if (!word) return null;

    // Also check for "Type::Variant" or "Module::Function" pattern
    const qualifiedMatch = getQualifiedName(line, params.position.character);

    const parseResult = parseCache.get(doc.uri) || parseDooSource(doc.getText());

    // Try qualified name first (e.g., Color::Red → find "Red" inside enum "Color")
    if (qualifiedMatch) {
        const loc = findDefinition(qualifiedMatch.member, params.position, doc.uri, parseResult, parseCache);
        if (loc) return loc;
        // Try the type/namespace itself
        const typeLoc = findDefinition(qualifiedMatch.namespace, params.position, doc.uri, parseResult, parseCache);
        if (typeLoc) return typeLoc;
    }

    // Try the plain word
    return findDefinition(word, params.position, doc.uri, parseResult, parseCache);
});

// ─── Hover ───

connection.onHover((params: HoverParams) => {
    const doc = documents.get(params.textDocument.uri);
    if (!doc) return null;

    const line = doc.getText({
        start: { line: params.position.line, character: 0 },
        end: { line: params.position.line, character: Number.MAX_SAFE_INTEGER },
    });

    const word = getWordAtPosition(line, params.position.character);
    if (!word) return null;

    const parseResult = parseCache.get(doc.uri) || parseDooSource(doc.getText());
    return getHoverInfo(word, params.position, parseResult, parseCache);
});

// ─── Document symbols ───

connection.onDocumentSymbol((params: DocumentSymbolParams) => {
    const doc = documents.get(params.textDocument.uri);
    if (!doc) return [];

    const parseResult = parseCache.get(doc.uri) || parseDooSource(doc.getText());
    return getDocumentSymbols(parseResult, doc.lineCount);
});

// ─── Signature help ───

connection.onSignatureHelp((params: SignatureHelpParams): SignatureHelp | null => {
    const doc = documents.get(params.textDocument.uri);
    if (!doc) return null;

    const line = doc.getText({
        start: { line: params.position.line, character: 0 },
        end: params.position,
    });

    // Find the function call context
    const callMatch = line.match(/(\w+)\s*\(([^)]*)$/);
    if (!callMatch) return null;

    const fnName = callMatch[1];
    const argsBefore = callMatch[2];
    const activeParam = argsBefore.split(',').length - 1;

    // Look up function in ALL parse results
    for (const [, pr] of parseCache) {
        for (const fn of pr.functions) {
            if (fn.name === fnName) {
                const params = fn.params.map((p) => {
                    const label = p.type ? `${p.name}: ${p.type}` : p.name;
                    return ParameterInformation.create(label);
                });
                const paramLabels = fn.params.map((p) => (p.type ? `${p.name}: ${p.type}` : p.name)).join(', ');
                let sigLabel = `fn ${fn.name}(${paramLabels})`;
                if (fn.returnType) sigLabel += ` -> ${fn.returnType}`;
                if (fn.errorType) sigLabel += ` ! ${fn.errorType}`;

                const sig = SignatureInformation.create(sigLabel, fn.doc || undefined, ...params);
                return {
                    signatures: [sig],
                    activeSignature: 0,
                    activeParameter: Math.min(activeParam, fn.params.length - 1),
                };
            }
        }
    }

    // Check built-in functions
    const builtinSig = getBuiltinSignature(fnName, activeParam);
    if (builtinSig) return builtinSig;

    return null;
});

function getBuiltinSignature(name: string, activeParam: number): SignatureHelp | null {
    const builtins: Record<string, { label: string; params: string[] }> = {
        'print': { label: 'print(values...)', params: ['values...'] },
        'sleep': { label: 'sleep(ms: Int)', params: ['ms: Int'] },
        'Abs': { label: 'Abs(x: Int) -> Int', params: ['x: Int'] },
        'Sqrt': { label: 'Sqrt(x: Int) -> Int', params: ['x: Int'] },
        'Pow': { label: 'Pow(base: Int, exp: Int) -> Int', params: ['base: Int', 'exp: Int'] },
    };

    const builtin = builtins[name];
    if (!builtin) return null;

    const params = builtin.params.map((p) => ParameterInformation.create(p));
    const sig = SignatureInformation.create(builtin.label, undefined, ...params);
    return {
        signatures: [sig],
        activeSignature: 0,
        activeParameter: Math.min(activeParam, params.length - 1),
    };
}

// ─── Utilities ───

function getWordAtPosition(line: string, character: number): string | null {
    let start = character;
    let end = character;

    while (start > 0 && /[\w]/.test(line[start - 1])) start--;
    while (end < line.length && /[\w]/.test(line[end])) end++;

    const word = line.substring(start, end);
    return word || null;
}

function getQualifiedName(line: string, character: number): { namespace: string; member: string } | null {
    // Expand to the full qualified identifier including ::
    let start = character;
    let end = character;

    // Expand right
    while (end < line.length && /[\w]/.test(line[end])) end++;

    // Expand left through word chars and ::
    while (start > 0) {
        if (/[\w]/.test(line[start - 1])) {
            start--;
        } else if (start >= 2 && line[start - 1] === ':' && line[start - 2] === ':') {
            start -= 2;
        } else {
            break;
        }
    }

    const full = line.substring(start, end);
    const parts = full.split('::');
    if (parts.length >= 2) {
        return {
            namespace: parts[parts.length - 2],
            member: parts[parts.length - 1],
        };
    }
    return null;
}

// ─── Start ───

documents.listen(connection);
connection.listen();
