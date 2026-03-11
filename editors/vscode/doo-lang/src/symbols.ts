/**
 * Document symbols and go-to-definition for Doo.
 */

import {
    DocumentSymbol,
    SymbolKind,
    Range,
    Location,
    Position,
} from 'vscode-languageserver/node';
import { DooParseResult, DooFunction, DooStruct, DooEnum } from './parser';

/**
 * Build document symbols for the outline view.
 */
export function getDocumentSymbols(
    parseResult: DooParseResult,
    lineCount: number
): DocumentSymbol[] {
    const symbols: DocumentSymbol[] = [];

    for (const fn of parseResult.functions) {
        const range = makeRange(fn.line, fn.endLine);
        const selRange = makeRange(fn.line, fn.line);
        const kind = fn.isMethod ? SymbolKind.Method : SymbolKind.Function;
        const detail = buildFnDetail(fn);
        symbols.push(DocumentSymbol.create(fn.fullName, detail, kind, range, selRange));
    }

    for (const s of parseResult.structs) {
        const range = makeRange(s.line, s.endLine);
        const selRange = makeRange(s.line, s.line);
        const structSymbol = DocumentSymbol.create(
            s.name,
            `${s.fields.length} fields`,
            SymbolKind.Struct,
            range,
            selRange
        );
        // Add fields as children
        structSymbol.children = s.fields.map((f) =>
            DocumentSymbol.create(
                f.name,
                f.type,
                SymbolKind.Field,
                makeRange(f.line, f.line),
                makeRange(f.line, f.line)
            )
        );
        symbols.push(structSymbol);
    }

    for (const en of parseResult.enums) {
        const range = makeRange(en.line, en.endLine);
        const selRange = makeRange(en.line, en.line);
        const enumSymbol = DocumentSymbol.create(
            en.name,
            `${en.variants.length} variants`,
            SymbolKind.Enum,
            range,
            selRange
        );
        enumSymbol.children = en.variants.map((v) =>
            DocumentSymbol.create(
                v.name,
                v.payloadTypes.length ? `(${v.payloadTypes.join(', ')})` : '',
                SymbolKind.EnumMember,
                makeRange(v.line, v.line),
                makeRange(v.line, v.line)
            )
        );
        symbols.push(enumSymbol);
    }

    return symbols;
}

/**
 * Find definition of a symbol at the given position.
 */
export function findDefinition(
    word: string,
    position: Position,
    uri: string,
    parseResult: DooParseResult,
    allParseResults: Map<string, DooParseResult>
): Location | null {
    // Search in current document first
    const loc = findSymbolInParseResult(word, uri, parseResult);
    if (loc) return loc;

    // Search in all documents
    for (const [docUri, pr] of allParseResults) {
        if (docUri === uri) continue;
        const loc2 = findSymbolInParseResult(word, docUri, pr);
        if (loc2) return loc2;
    }

    return null;
}

function findSymbolInParseResult(
    word: string,
    uri: string,
    parseResult: DooParseResult
): Location | null {
    // Functions
    for (const fn of parseResult.functions) {
        if (fn.name === word || fn.fullName === word) {
            return Location.create(uri, makeRange(fn.line, fn.line));
        }
    }

    // Structs
    for (const s of parseResult.structs) {
        if (s.name === word) {
            return Location.create(uri, makeRange(s.line, s.line));
        }
        // Struct fields
        for (const f of s.fields) {
            if (f.name === word) {
                return Location.create(uri, makeRange(f.line, f.line));
            }
        }
    }

    // Enums
    for (const en of parseResult.enums) {
        if (en.name === word) {
            return Location.create(uri, makeRange(en.line, en.line));
        }
        for (const v of en.variants) {
            if (v.name === word) {
                return Location.create(uri, makeRange(v.line, v.line));
            }
        }
    }

    // Variables
    for (const v of parseResult.variables) {
        if (v.name === word) {
            return Location.create(uri, makeRange(v.line, v.line));
        }
    }

    return null;
}

function buildFnDetail(fn: DooFunction): string {
    const params = fn.params.map((p) => (p.type ? `${p.name}: ${p.type}` : p.name)).join(', ');
    let sig = `(${params})`;
    if (fn.returnType) sig += ` -> ${fn.returnType}`;
    if (fn.errorType) sig += ` ! ${fn.errorType}`;
    return sig;
}

function makeRange(startLine: number, endLine: number): Range {
    return Range.create(
        Position.create(startLine, 0),
        Position.create(endLine, Number.MAX_SAFE_INTEGER)
    );
}
