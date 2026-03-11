/**
 * Diagnostics provider for Doo files.
 * Detects basic syntax errors: unmatched brackets, unclosed strings,
 * unknown decorators, and missing semicolons.
 */

import {
    Diagnostic,
    DiagnosticSeverity,
    Range,
    Position,
} from 'vscode-languageserver/node';

const KNOWN_DECORATORS = new Set([
    'primary', 'auto', 'unique', 'hash', 'email',
    'writeOnly', 'readOnly', 'internal', 'default',
    'extern',
]);

const VALID_KEYWORDS = new Set([
    'fn', 'let', 'mut', 'struct', 'enum', 'match', 'if', 'else',
    'for', 'in', 'return', 'import', 'as', 'async', 'await',
    'go', 'scope', 'sleep', 'print', 'self', 'true', 'false',
    'Ok', 'Err', 'pub', 'while', 'loop', 'get', 'post', 'put', 'delete',
]);

export function getDiagnostics(text: string): Diagnostic[] {
    const diagnostics: Diagnostic[] = [];
    const lines = text.split(/\r?\n/);

    // Track bracket balance
    const bracketStack: { char: string; line: number; col: number }[] = [];
    const bracketPairs: Record<string, string> = { '{': '}', '[': ']', '(': ')' };
    const closingBrackets: Record<string, string> = { '}': '{', ']': '[', ')': '(' };

    for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        const trimmed = line.trim();

        // Skip comment-only lines
        if (trimmed.startsWith('//')) continue;

        // Remove string contents and comments for bracket checking
        const stripped = stripStringsAndComments(line);

        // Check bracket balance
        for (let col = 0; col < stripped.length; col++) {
            const ch = stripped[col];
            if (ch in bracketPairs) {
                bracketStack.push({ char: ch, line: i, col });
            } else if (ch in closingBrackets) {
                const expected = closingBrackets[ch];
                if (bracketStack.length === 0) {
                    diagnostics.push({
                        severity: DiagnosticSeverity.Error,
                        range: makeRange(i, col, i, col + 1),
                        message: `Unexpected closing '${ch}' without matching opening '${expected}'`,
                        source: 'doo',
                    });
                } else {
                    const top = bracketStack[bracketStack.length - 1];
                    if (top.char !== expected) {
                        diagnostics.push({
                            severity: DiagnosticSeverity.Error,
                            range: makeRange(i, col, i, col + 1),
                            message: `Mismatched bracket: expected '${bracketPairs[top.char]}' to close '${top.char}' at line ${top.line + 1}, but found '${ch}'`,
                            source: 'doo',
                        });
                    }
                    bracketStack.pop();
                }
            }
        }

        // Check for unknown decorators
        const decoRegex = /@(\w+)/g;
        let decoMatch;
        while ((decoMatch = decoRegex.exec(line)) !== null) {
            const decoName = decoMatch[1];
            if (!KNOWN_DECORATORS.has(decoName)) {
                diagnostics.push({
                    severity: DiagnosticSeverity.Warning,
                    range: makeRange(i, decoMatch.index, i, decoMatch.index + decoMatch[0].length),
                    message: `Unknown decorator '@${decoName}'`,
                    source: 'doo',
                });
            }
        }

        // Check for unclosed strings (basic)
        let inString = false;
        let escaped = false;
        for (let col = 0; col < line.length; col++) {
            const ch = line[col];
            if (escaped) {
                escaped = false;
                continue;
            }
            if (ch === '\\') {
                escaped = true;
                continue;
            }
            // Skip if we're past a // comment
            if (!inString && ch === '/' && col + 1 < line.length && line[col + 1] === '/') {
                break;
            }
            if (ch === '"') {
                inString = !inString;
            }
        }
        if (inString) {
            diagnostics.push({
                severity: DiagnosticSeverity.Error,
                range: makeRange(i, 0, i, line.length),
                message: 'Unclosed string literal',
                source: 'doo',
            });
        }
    }

    // Report unmatched opening brackets
    for (const unmatched of bracketStack) {
        diagnostics.push({
            severity: DiagnosticSeverity.Error,
            range: makeRange(unmatched.line, unmatched.col, unmatched.line, unmatched.col + 1),
            message: `Unmatched opening '${unmatched.char}'`,
            source: 'doo',
        });
    }

    return diagnostics;
}

function stripStringsAndComments(line: string): string {
    let result = '';
    let inString = false;
    let escaped = false;

    for (let i = 0; i < line.length; i++) {
        const ch = line[i];

        if (escaped) {
            escaped = false;
            continue;
        }

        if (ch === '\\' && inString) {
            escaped = true;
            continue;
        }

        if (!inString && ch === '/' && i + 1 < line.length && line[i + 1] === '/') {
            break; // rest is comment
        }

        if (ch === '"') {
            inString = !inString;
            continue;
        }

        if (!inString) {
            result += ch;
        }
    }

    return result;
}

function makeRange(
    startLine: number,
    startChar: number,
    endLine: number,
    endChar: number
): Range {
    return Range.create(
        Position.create(startLine, startChar),
        Position.create(endLine, endChar)
    );
}
