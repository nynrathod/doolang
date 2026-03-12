/**
 * Hover provider for Doo language.
 * Shows type information, function signatures, and documentation on hover.
 */

import {
    Hover,
    MarkupKind,
    Position,
} from 'vscode-languageserver/node';
import { DooParseResult } from './parser';

/**
 * Get hover information for the word at the given position.
 */
export function getHoverInfo(
    word: string,
    position: Position,
    parseResult: DooParseResult,
    allParseResults: Map<string, DooParseResult>
): Hover | null {
    // Check built-in types
    const builtinType = getBuiltinTypeHover(word);
    if (builtinType) return builtinType;

    // Check built-in keywords
    const keywordHover = getKeywordHover(word);
    if (keywordHover) return keywordHover;

    // Check current document
    const localHover = getSymbolHover(word, parseResult);
    if (localHover) return localHover;

    // Check other documents
    for (const [, pr] of allParseResults) {
        const hover = getSymbolHover(word, pr);
        if (hover) return hover;
    }

    return null;
}

function getSymbolHover(word: string, pr: DooParseResult): Hover | null {
    // Functions
    for (const fn of pr.functions) {
        if (fn.name === word || fn.fullName === word) {
            const asyncPrefix = fn.isAsync ? 'async ' : '';
            const methodPrefix = fn.structName ? `${fn.structName}.` : '';
            const params = fn.params.map((p) => (p.type ? `${p.name}: ${p.type}` : p.name)).join(', ');
            let sig = `${asyncPrefix}fn ${methodPrefix}${fn.name}(${params})`;
            if (fn.returnType) sig += ` -> ${fn.returnType}`;
            if (fn.errorType) sig += ` ! ${fn.errorType}`;

            let content = '```doo\n' + sig + '\n```';
            if (fn.doc) {
                content += '\n\n---\n\n' + fn.doc;
            }

            return {
                contents: { kind: MarkupKind.Markdown, value: content },
            };
        }
    }

    // Structs
    for (const s of pr.structs) {
        if (s.name === word) {
            const fields = s.fields
                .map((f) => {
                    const opt = f.optional ? '?' : '';
                    const decos = f.decorators.length ? ' ' + f.decorators.map((d) => '@' + d).join(' ') : '';
                    return `    ${f.name}${opt}: ${f.type}${decos}`;
                })
                .join(',\n');
            const content = '```doo\nstruct ' + s.name + ' {\n' + fields + ',\n}\n```';
            return {
                contents: { kind: MarkupKind.Markdown, value: content },
            };
        }
    }

    // Enums
    for (const en of pr.enums) {
        if (en.name === word) {
            if (en.inline) {
                const variants = en.variants.map((v) => v.name).join(' | ');
                const content = '```doo\nenum ' + en.name + ': ' + variants + ';\n```';
                return {
                    contents: { kind: MarkupKind.Markdown, value: content },
                };
            }
            const variants = en.variants
                .map((v) => {
                    const payload = v.payloadTypes.length ? `(${v.payloadTypes.join(', ')})` : '';
                    return `    ${v.name}${payload}`;
                })
                .join(',\n');
            const content = '```doo\nenum ' + en.name + ' {\n' + variants + ',\n}\n```';
            return {
                contents: { kind: MarkupKind.Markdown, value: content },
            };
        }

        // Enum variants
        for (const v of en.variants) {
            if (v.name === word) {
                const payload = v.payloadTypes.length ? `(${v.payloadTypes.join(', ')})` : '';
                const content = `\`\`\`doo\n${en.name}::${v.name}${payload}\n\`\`\``;
                return {
                    contents: { kind: MarkupKind.Markdown, value: content },
                };
            }
        }
    }

    // Variables
    for (const v of pr.variables) {
        if (v.name === word) {
            const mut = v.mutable ? 'mut ' : '';
            const type = v.type ? `: ${v.type}` : '';
            return {
                contents: {
                    kind: MarkupKind.Markdown,
                    value: `\`\`\`doo\nlet ${mut}${v.name}${type}\n\`\`\``,
                },
            };
        }
    }

    return null;
}

function getBuiltinTypeHover(word: string): Hover | null {
    const types: Record<string, string> = {
        'Int': 'Integer type — 64-bit signed integer',
        'Str': 'String type — UTF-8 string',
        'Float': 'Float type — 64-bit floating point',
        'Bool': 'Boolean type — `true` or `false`',
        'Void': 'Void type — no return value',
        'Request': 'HTTP Request object\n\nProperties: `Path`, `Method`\nMethods: `header(name: Str) -> Str`',
        'Response': 'HTTP Response object\n\nProperties: `Status`',
        'Next': 'Middleware next handler\n\nMethods: `call() -> Response`',
        'Server': 'HTTP Server\n\nConstructor: `Server::new(addr: Str)`\nMethods: `get`, `post`, `put`, `delete`, `group`, `ws`, `auth`, `crud`, `use`, `start`, `cors`, `logger`',
        'WsConnection': 'WebSocket connection\n\nMethods: `on`, `emit`, `join`, `leave`, `close`, `isClosed`, `onConnect`, `onDisconnect`, `onError`',
        'DatabaseError': 'Database error type',
        'WsError': 'WebSocket error type',
    };

    if (word in types) {
        return {
            contents: {
                kind: MarkupKind.Markdown,
                value: `**${word}**\n\n${types[word]}`,
            },
        };
    }
    return null;
}

function getKeywordHover(word: string): Hover | null {
    const keywords: Record<string, string> = {
        'fn': 'Declare a function\n\n```doo\nfn name(params) -> ReturnType { }\n```',
        'async': 'Declare an async function\n\n```doo\nasync fn name() -> ReturnType { }\n```',
        'let': 'Declare a variable\n\n```doo\nlet name = value;\nlet name: Type = value;\n```',
        'mut': 'Mutable modifier\n\n```doo\nlet mut name = value;\n```',
        'struct': 'Declare a struct\n\n```doo\nstruct Name {\n    field: Type,\n}\n```',
        'enum': 'Declare an enum\n\n```doo\nenum Name { Variant1, Variant2(Type) }\nenum Name: A | B | C;\n```',
        'match': 'Pattern matching expression\n\n```doo\nmatch value {\n    pattern => expr,\n    _ => default,\n}\n```',
        'import': 'Import a module\n\n```doo\nimport std::Module::{Item};\nimport std::Module::{Item as Alias};\n```',
        'return': 'Return a value from a function',
        'Ok': 'Return a success value in error-handling functions',
        'Err': 'Return an error value in error-handling functions',
        'await': 'Await an async operation\n\n```doo\nlet result = await asyncFn();\n```',
        'go': 'Spawn a concurrent task\n\n```doo\ngo { /* runs concurrently */ }\nlet handle = go { /* awaitable */ };\n```',
        'scope': 'Structured concurrency scope — waits for all tasks\n\n```doo\nscope {\n    go { task1() }\n    go { task2() }\n}\n// both tasks complete before continuing\n```',
        'sleep': 'Sleep for the specified milliseconds\n\n```doo\nsleep(100);\n```',
        'self': 'Reference to the current struct instance in methods',
        'for': 'For loop\n\n```doo\nfor item in collection { }\nfor i in 0..10 { }\nfor i, item in array { }\n```',
        'print': 'Print values to stdout\n\n```doo\nprint("Hello", value);\n```',
        'as': 'Type cast or import alias\n\n```doo\nlet f = (x as Float);\nimport std::Math::{Sqrt as Sq};\n```',
    };

    if (word in keywords) {
        return {
            contents: {
                kind: MarkupKind.Markdown,
                value: keywords[word],
            },
        };
    }
    return null;
}
