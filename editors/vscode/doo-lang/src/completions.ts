/**
 * Doo language completion provider.
 * Provides completions for keywords, types, standard library, struct fields,
 * enum variants, and user-defined symbols.
 */

import {
    CompletionItem,
    CompletionItemKind,
    InsertTextFormat,
    MarkupKind,
    Position,
} from 'vscode-languageserver/node';
import { DooParseResult } from './parser';

// ─── Keyword completions ───

const KEYWORD_COMPLETIONS: CompletionItem[] = [
    {
        label: 'fn',
        kind: CompletionItemKind.Keyword,
        insertText: 'fn ${1:name}(${2:params}) {\n\t$0\n}',
        insertTextFormat: InsertTextFormat.Snippet,
        detail: 'Function declaration',
    },
    {
        label: 'async fn',
        kind: CompletionItemKind.Keyword,
        insertText: 'async fn ${1:name}(${2:params}) -> ${3:ReturnType} {\n\t$0\n}',
        insertTextFormat: InsertTextFormat.Snippet,
        detail: 'Async function declaration',
    },
    {
        label: 'struct',
        kind: CompletionItemKind.Keyword,
        insertText: 'struct ${1:Name} {\n\t${2:field}: ${3:Type},\n}',
        insertTextFormat: InsertTextFormat.Snippet,
        detail: 'Struct declaration',
    },
    {
        label: 'enum',
        kind: CompletionItemKind.Keyword,
        insertText: 'enum ${1:Name} {\n\t${2:Variant},\n}',
        insertTextFormat: InsertTextFormat.Snippet,
        detail: 'Enum declaration',
    },
    {
        label: 'match',
        kind: CompletionItemKind.Keyword,
        insertText: 'match ${1:value} {\n\t${2:pattern} => ${3:expr},\n\t_ => ${0:default},\n}',
        insertTextFormat: InsertTextFormat.Snippet,
        detail: 'Match expression',
    },
    {
        label: 'if',
        kind: CompletionItemKind.Keyword,
        insertText: 'if ${1:condition} {\n\t$0\n}',
        insertTextFormat: InsertTextFormat.Snippet,
        detail: 'If expression',
    },
    {
        label: 'if else',
        kind: CompletionItemKind.Keyword,
        insertText: 'if ${1:condition} {\n\t${2}\n} else {\n\t$0\n}',
        insertTextFormat: InsertTextFormat.Snippet,
        detail: 'If-else expression',
    },
    {
        label: 'for',
        kind: CompletionItemKind.Keyword,
        insertText: 'for ${1:item} in ${2:collection} {\n\t$0\n}',
        insertTextFormat: InsertTextFormat.Snippet,
        detail: 'For loop',
    },
    {
        label: 'for range',
        kind: CompletionItemKind.Keyword,
        insertText: 'for ${1:i} in ${2:0}..${3:10} {\n\t$0\n}',
        insertTextFormat: InsertTextFormat.Snippet,
        detail: 'For loop with range',
    },
    {
        label: 'let',
        kind: CompletionItemKind.Keyword,
        insertText: 'let ${1:name} = ${0};',
        insertTextFormat: InsertTextFormat.Snippet,
        detail: 'Variable declaration',
    },
    {
        label: 'let mut',
        kind: CompletionItemKind.Keyword,
        insertText: 'let mut ${1:name} = ${0};',
        insertTextFormat: InsertTextFormat.Snippet,
        detail: 'Mutable variable declaration',
    },
    {
        label: 'import',
        kind: CompletionItemKind.Keyword,
        insertText: 'import ${1:std::${2:Module}}::{${0}};',
        insertTextFormat: InsertTextFormat.Snippet,
        detail: 'Import statement',
    },
    {
        label: 'return',
        kind: CompletionItemKind.Keyword,
        detail: 'Return from function',
    },
    {
        label: 'Ok',
        kind: CompletionItemKind.Keyword,
        detail: 'Return success value',
    },
    {
        label: 'Err',
        kind: CompletionItemKind.Keyword,
        detail: 'Return error value',
    },
    {
        label: 'await',
        kind: CompletionItemKind.Keyword,
        detail: 'Await async operation',
    },
    {
        label: 'go',
        kind: CompletionItemKind.Keyword,
        insertText: 'go {\n\t$0\n}',
        insertTextFormat: InsertTextFormat.Snippet,
        detail: 'Spawn concurrent task',
    },
    {
        label: 'scope',
        kind: CompletionItemKind.Keyword,
        insertText: 'scope {\n\t$0\n}',
        insertTextFormat: InsertTextFormat.Snippet,
        detail: 'Structured concurrency scope',
    },
    {
        label: 'sleep',
        kind: CompletionItemKind.Function,
        insertText: 'sleep(${1:ms})',
        insertTextFormat: InsertTextFormat.Snippet,
        detail: 'Sleep for milliseconds',
    },
    {
        label: 'print',
        kind: CompletionItemKind.Function,
        insertText: 'print(${0});',
        insertTextFormat: InsertTextFormat.Snippet,
        detail: 'Print to stdout',
    },
    {
        label: 'self',
        kind: CompletionItemKind.Keyword,
        detail: 'Reference to current struct instance',
    },
    {
        label: 'true',
        kind: CompletionItemKind.Keyword,
        detail: 'Boolean true',
    },
    {
        label: 'false',
        kind: CompletionItemKind.Keyword,
        detail: 'Boolean false',
    },
    {
        label: 'mut',
        kind: CompletionItemKind.Keyword,
        detail: 'Mutable modifier',
    },
    {
        label: 'as',
        kind: CompletionItemKind.Keyword,
        detail: 'Type cast or import alias',
    },
];

// ─── Type completions ───

const TYPE_COMPLETIONS: CompletionItem[] = [
    { label: 'Int', kind: CompletionItemKind.TypeParameter, detail: 'Integer type' },
    { label: 'Str', kind: CompletionItemKind.TypeParameter, detail: 'String type' },
    { label: 'Float', kind: CompletionItemKind.TypeParameter, detail: 'Float type' },
    { label: 'Bool', kind: CompletionItemKind.TypeParameter, detail: 'Boolean type' },
    { label: 'Void', kind: CompletionItemKind.TypeParameter, detail: 'Void type (no return)' },
    { label: 'Request', kind: CompletionItemKind.Class, detail: 'HTTP Request object' },
    { label: 'Response', kind: CompletionItemKind.Class, detail: 'HTTP Response object' },
    { label: 'Next', kind: CompletionItemKind.Class, detail: 'Middleware next handler' },
    { label: 'Server', kind: CompletionItemKind.Class, detail: 'HTTP Server' },
    { label: 'WsConnection', kind: CompletionItemKind.Class, detail: 'WebSocket connection' },
    { label: 'WsError', kind: CompletionItemKind.Class, detail: 'WebSocket error' },
    { label: 'DatabaseError', kind: CompletionItemKind.Class, detail: 'Database error' },
];

// ─── Decorator completions ───

const DECORATOR_COMPLETIONS: CompletionItem[] = [
    { label: '@primary', kind: CompletionItemKind.Property, detail: 'Primary key field', insertText: '@primary' },
    { label: '@auto', kind: CompletionItemKind.Property, detail: 'Auto-generated field', insertText: '@auto' },
    { label: '@unique', kind: CompletionItemKind.Property, detail: 'Unique constraint', insertText: '@unique' },
    { label: '@hash', kind: CompletionItemKind.Property, detail: 'Hash field (passwords)', insertText: '@hash' },
    { label: '@email', kind: CompletionItemKind.Property, detail: 'Email validation', insertText: '@email' },
    { label: '@writeOnly', kind: CompletionItemKind.Property, detail: 'Write-only field (in request, not in response)', insertText: '@writeOnly' },
    { label: '@readOnly', kind: CompletionItemKind.Property, detail: 'Read-only field (not in request, in response)', insertText: '@readOnly' },
    { label: '@internal', kind: CompletionItemKind.Property, detail: 'Internal field (not in request or response)', insertText: '@internal' },
    {
        label: '@default',
        kind: CompletionItemKind.Property,
        detail: 'Default value',
        insertText: '@default(${1:value})',
        insertTextFormat: InsertTextFormat.Snippet,
    },
];

// ─── Standard library completions ───

interface StdLibModule {
    path: string;
    items: CompletionItem[];
}

const STD_LIB: StdLibModule[] = [
    {
        path: 'std::Math',
        items: [
            { label: 'Abs', kind: CompletionItemKind.Function, detail: 'fn Abs(x: Int) -> Int', documentation: 'Returns the absolute value' },
            { label: 'Sqrt', kind: CompletionItemKind.Function, detail: 'fn Sqrt(x: Int) -> Int', documentation: 'Returns the square root' },
            { label: 'Pow', kind: CompletionItemKind.Function, detail: 'fn Pow(base: Int, exp: Int) -> Int', documentation: 'Returns base raised to the power exp' },
        ],
    },
    {
        path: 'std::Array',
        items: [
            { label: 'Sum', kind: CompletionItemKind.Function, detail: 'fn Sum(arr: [Int]) -> Int', documentation: 'Returns the sum of all elements' },
        ],
    },
    {
        path: 'std::Config',
        items: [
            { label: 'get', kind: CompletionItemKind.Function, detail: 'fn get(key: Str) -> Str?', documentation: 'Get env variable (may error)' },
            { label: 'getOr', kind: CompletionItemKind.Function, detail: 'fn getOr(key: Str, default: Str) -> Str', documentation: 'Get env variable with default' },
            { label: 'set', kind: CompletionItemKind.Function, detail: 'fn set(key: Str, value: Str)', documentation: 'Set env variable' },
            { label: 'has', kind: CompletionItemKind.Function, detail: 'fn has(key: Str) -> Bool', documentation: 'Check if env variable exists' },
            { label: 'getInt', kind: CompletionItemKind.Function, detail: 'fn getInt(key: Str, default: Int) -> Int', documentation: 'Get env variable as Int' },
            { label: 'getBool', kind: CompletionItemKind.Function, detail: 'fn getBool(key: Str, default: Bool) -> Bool', documentation: 'Get env variable as Bool' },
        ],
    },
    {
        path: 'std::File',
        items: [
            { label: 'Read', kind: CompletionItemKind.Function, detail: 'fn Read(path: Str) -> Str', documentation: 'Read file contents' },
            { label: 'Write', kind: CompletionItemKind.Function, detail: 'fn Write(content: Str, path: Str)', documentation: 'Write content to file' },
            { label: 'Exists', kind: CompletionItemKind.Function, detail: 'fn Exists(path: Str) -> Bool', documentation: 'Check if file exists' },
            { label: 'Delete', kind: CompletionItemKind.Function, detail: 'fn Delete(path: Str)', documentation: 'Delete a file' },
            { label: 'Metadata', kind: CompletionItemKind.Function, detail: 'fn Metadata(path: Str) -> FileMetadata', documentation: 'Get file metadata' },
        ],
    },
    {
        path: 'std::Random',
        items: [
            { label: 'Int', kind: CompletionItemKind.Function, detail: 'fn Int(min: Int, max: Int) -> Int', documentation: 'Random integer in range' },
        ],
    },
    {
        path: 'std::Http',
        items: [
            { label: 'Server', kind: CompletionItemKind.Class, detail: 'HTTP Server', documentation: 'HTTP server for handling routes' },
            { label: 'Request', kind: CompletionItemKind.Class, detail: 'HTTP Request' },
            { label: 'Response', kind: CompletionItemKind.Class, detail: 'HTTP Response' },
            { label: 'Next', kind: CompletionItemKind.Class, detail: 'Middleware next handler' },
            { label: 'WsConnection', kind: CompletionItemKind.Class, detail: 'WebSocket connection' },
            { label: 'WsError', kind: CompletionItemKind.Class, detail: 'WebSocket error' },
        ],
    },
    {
        path: 'std::Database',
        items: [
            { label: 'Postgres', kind: CompletionItemKind.Function, detail: 'fn Postgres() -> Database?', documentation: 'Connect to PostgreSQL database' },
            { label: 'get', kind: CompletionItemKind.Function, detail: 'fn get() -> Database?', documentation: 'Get database connection' },
        ],
    },
    {
        path: 'std::Auth',
        items: [
            { label: 'Jwt', kind: CompletionItemKind.Function, detail: 'fn Jwt() -> Middleware', documentation: 'JWT authentication middleware' },
        ],
    },
];

// Server method completions (after app.)
const SERVER_METHOD_COMPLETIONS: CompletionItem[] = [
    {
        label: 'get', kind: CompletionItemKind.Method,
        detail: 'app.get(path, handler)',
        insertText: 'get("${1:/path}", ${2:handler})',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Register GET route',
    },
    {
        label: 'post', kind: CompletionItemKind.Method,
        detail: 'app.post(path, handler)',
        insertText: 'post("${1:/path}", ${2:handler})',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Register POST route',
    },
    {
        label: 'put', kind: CompletionItemKind.Method,
        detail: 'app.put(path, handler)',
        insertText: 'put("${1:/path}", ${2:handler})',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Register PUT route',
    },
    {
        label: 'delete', kind: CompletionItemKind.Method,
        detail: 'app.delete(path, handler)',
        insertText: 'delete("${1:/path}", ${2:handler})',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Register DELETE route',
    },
    {
        label: 'group', kind: CompletionItemKind.Method,
        detail: 'app.group(prefix, { ... })',
        insertText: 'group("${1:/prefix}", {\n\t${0}\n})',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Register route group',
    },
    {
        label: 'ws', kind: CompletionItemKind.Method,
        detail: 'app.ws(path, handler)',
        insertText: 'ws("${1:/ws/path}", ${2:handler})',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Register WebSocket route',
    },
    {
        label: 'auth', kind: CompletionItemKind.Method,
        detail: 'app.auth(signupPath, loginPath, Model, db)',
        insertText: 'auth("${1:/signup}", "${2:/login}", ${3:User}, ${4:db})',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Register auth endpoints',
    },
    {
        label: 'crud', kind: CompletionItemKind.Method,
        detail: 'app.crud(path, Model, db)',
        insertText: 'crud("${1:/resource}", ${2:Model}, ${3:db})',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Register CRUD endpoints',
    },
    {
        label: 'use', kind: CompletionItemKind.Method,
        detail: 'app.use(middleware)',
        insertText: 'use(${1:middleware})',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Register global middleware',
    },
    {
        label: 'start', kind: CompletionItemKind.Method,
        detail: 'app.start()',
        insertText: 'start()',
        documentation: 'Start the server',
    },
    {
        label: 'cors', kind: CompletionItemKind.Method,
        detail: 'app.cors()',
        insertText: 'cors()',
        documentation: 'Enable CORS middleware',
    },
    {
        label: 'logger', kind: CompletionItemKind.Method,
        detail: 'app.logger()',
        insertText: 'logger()',
        documentation: 'Enable request logger',
    },
    {
        label: 'broadcast', kind: CompletionItemKind.Method,
        detail: 'app.broadcast(event, data)',
        insertText: 'broadcast("${1:event}", ${2:data})',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Broadcast to all WebSocket clients',
    },
    {
        label: 'toRoomEmit', kind: CompletionItemKind.Method,
        detail: 'app.toRoomEmit(room, event, data)',
        insertText: 'toRoomEmit("${1:room}", "${2:event}", ${3:data})',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Emit to specific room',
    },
    {
        label: 'activeWsConnections', kind: CompletionItemKind.Method,
        detail: 'app.activeWsConnections() -> Int',
        insertText: 'activeWsConnections()',
        documentation: 'Get active WebSocket connection count',
    },
    {
        label: 'new', kind: CompletionItemKind.Constructor,
        detail: 'Server::new(addr)',
        insertText: 'new("${1::3000}")',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Create new server instance',
    },
];

// WsConnection method completions
const WS_CONNECTION_METHODS: CompletionItem[] = [
    {
        label: 'on', kind: CompletionItemKind.Method,
        detail: 'conn.on(event, handler)',
        insertText: 'on("${1:event}", ${2:handler})',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Listen for WebSocket event',
    },
    {
        label: 'emit', kind: CompletionItemKind.Method,
        detail: 'conn.emit(event, data)',
        insertText: 'emit("${1:event}", ${2:data})',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Emit WebSocket event',
    },
    {
        label: 'join', kind: CompletionItemKind.Method,
        detail: 'conn.join(room)',
        insertText: 'join("${1:room}")',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Join a room',
    },
    {
        label: 'leave', kind: CompletionItemKind.Method,
        detail: 'conn.leave(room)',
        insertText: 'leave("${1:room}")',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Leave a room',
    },
    {
        label: 'close', kind: CompletionItemKind.Method,
        detail: 'conn.close()',
        insertText: 'close()',
        documentation: 'Close WebSocket connection',
    },
    {
        label: 'isClosed', kind: CompletionItemKind.Method,
        detail: 'conn.isClosed() -> Bool',
        insertText: 'isClosed()',
        documentation: 'Check if connection is closed',
    },
    {
        label: 'onConnect', kind: CompletionItemKind.Method,
        detail: 'conn.onConnect(handler)',
        insertText: 'onConnect(${1:handler})',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Handle connection event',
    },
    {
        label: 'onDisconnect', kind: CompletionItemKind.Method,
        detail: 'conn.onDisconnect(handler)',
        insertText: 'onDisconnect(${1:handler})',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Handle disconnection event',
    },
    {
        label: 'onError', kind: CompletionItemKind.Method,
        detail: 'conn.onError(handler)',
        insertText: 'onError(${1:handler})',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Handle error event',
    },
];

// Request property completions
const REQUEST_MEMBERS: CompletionItem[] = [
    { label: 'Path', kind: CompletionItemKind.Property, detail: 'Str', documentation: 'Request path' },
    { label: 'Method', kind: CompletionItemKind.Property, detail: 'Str', documentation: 'HTTP method (GET, POST, etc.)' },
    { label: 'header', kind: CompletionItemKind.Method, detail: 'fn header(name: Str) -> Str', insertText: 'header("${1:name}")', insertTextFormat: InsertTextFormat.Snippet, documentation: 'Get request header value' },
];

// Next method completions
const NEXT_MEMBERS: CompletionItem[] = [
    { label: 'call', kind: CompletionItemKind.Method, detail: 'fn call() -> Response', insertText: 'call()', documentation: 'Call next middleware/handler' },
];

// Response property completions
const RESPONSE_MEMBERS: CompletionItem[] = [
    { label: 'Status', kind: CompletionItemKind.Property, detail: 'Int', documentation: 'Response status code' },
];

// Database instance methods
const DB_METHODS: CompletionItem[] = [
    {
        label: 'raw', kind: CompletionItemKind.Method,
        detail: 'db.raw(query: Str)',
        insertText: 'raw("${1:SELECT * FROM table}")',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Execute raw SQL query',
    },
    {
        label: 'rawWithParams', kind: CompletionItemKind.Method,
        detail: 'db.rawWithParams(query: Str, params: [Any])',
        insertText: 'rawWithParams("${1:SELECT * FROM table WHERE id = \\$1}", ${2:params})',
        insertTextFormat: InsertTextFormat.Snippet,
        documentation: 'Execute parameterized SQL query',
    },
];

// Array method completions
const ARRAY_METHODS: CompletionItem[] = [
    { label: 'push', kind: CompletionItemKind.Method, detail: 'arr.push(item)', insertText: 'push(${1:item})', insertTextFormat: InsertTextFormat.Snippet, documentation: 'Add item to end of array' },
    { label: 'length', kind: CompletionItemKind.Property, detail: 'Int', documentation: 'Array length' },
];

/**
 * Get completions based on the current context.
 */
export function getCompletions(
    parseResult: DooParseResult,
    position: Position,
    lineText: string,
    allParseResults: Map<string, DooParseResult>
): CompletionItem[] {
    const items: CompletionItem[] = [];
    const textBeforeCursor = lineText.substring(0, position.character);

    // After @ → decorator completions
    if (textBeforeCursor.match(/@\w*$/)) {
        return DECORATOR_COMPLETIONS;
    }

    // After . (dot) → method/property completions
    const dotMatch = textBeforeCursor.match(/(\w+)\.\w*$/);
    if (dotMatch) {
        const varName = dotMatch[1];
        // Determine the type of the variable
        const varType = resolveVariableType(varName, parseResult, position.line);
        if (varType) {
            return getTypeMembers(varType, parseResult);
        }
        // Fallback: offer common method completions
        return [...SERVER_METHOD_COMPLETIONS, ...WS_CONNECTION_METHODS, ...DB_METHODS, ...ARRAY_METHODS];
    }

    // After :: → namespace member completions
    const nsMatch = textBeforeCursor.match(/([\w:]+)::\w*$/);
    if (nsMatch) {
        const ns = nsMatch[1];
        // Check std lib first
        for (const mod of STD_LIB) {
            if (mod.path === ns || mod.path.endsWith('::' + ns) || ns === 'std' || mod.path.includes(ns)) {
                items.push(...mod.items);
            }
        }
        // Check enum variants
        for (const en of parseResult.enums) {
            if (en.name === ns) {
                for (const v of en.variants) {
                    items.push({
                        label: v.name,
                        kind: CompletionItemKind.EnumMember,
                        detail: `${en.name}::${v.name}${v.payloadTypes.length ? '(' + v.payloadTypes.join(', ') + ')' : ''}`,
                    });
                }
            }
        }
        // Check all documents for enums
        for (const [, pr] of allParseResults) {
            for (const en of pr.enums) {
                if (en.name === ns) {
                    for (const v of en.variants) {
                        items.push({
                            label: v.name,
                            kind: CompletionItemKind.EnumMember,
                            detail: `${en.name}::${v.name}`,
                        });
                    }
                }
            }
        }
        if (items.length > 0) return items;

        // If we matched "Server", also offer static methods
        if (ns === 'Server' || ns.endsWith('::Server')) {
            items.push({
                label: 'new',
                kind: CompletionItemKind.Constructor,
                detail: 'Server::new(addr: Str) -> Server',
                insertText: 'new("${1::3000}")',
                insertTextFormat: InsertTextFormat.Snippet,
            });
            return items;
        }

        if (ns === 'Database' || ns.endsWith('::Database')) {
            items.push({
                label: 'Postgres',
                kind: CompletionItemKind.Function,
                detail: 'Database::Postgres() -> Database?',
                insertText: 'Postgres()',
            });
            items.push({
                label: 'get',
                kind: CompletionItemKind.Function,
                detail: 'Database::get() -> Database?',
                insertText: 'get()',
            });
            return items;
        }
    }

    // After "import " → std module suggestions
    if (textBeforeCursor.match(/import\s+$/)) {
        return STD_LIB.map((mod) => ({
            label: mod.path,
            kind: CompletionItemKind.Module,
            detail: `Import ${mod.path}`,
        }));
    }

    // Default completions: keywords + types + user symbols
    items.push(...KEYWORD_COMPLETIONS);
    items.push(...TYPE_COMPLETIONS);

    // Add user-defined functions
    for (const fn of parseResult.functions) {
        items.push({
            label: fn.name,
            kind: CompletionItemKind.Function,
            detail: formatFunctionSignature(fn),
            documentation: fn.doc ? { kind: MarkupKind.Markdown, value: fn.doc } : undefined,
        });
    }

    // Add user-defined structs
    for (const s of parseResult.structs) {
        items.push({
            label: s.name,
            kind: CompletionItemKind.Struct,
            detail: `struct ${s.name}`,
            documentation: {
                kind: MarkupKind.Markdown,
                value: `Fields: ${s.fields.map((f) => `${f.name}: ${f.type}`).join(', ')}`,
            },
        });
    }

    // Add user-defined enums
    for (const en of parseResult.enums) {
        items.push({
            label: en.name,
            kind: CompletionItemKind.Enum,
            detail: `enum ${en.name}`,
            documentation: {
                kind: MarkupKind.Markdown,
                value: `Variants: ${en.variants.map((v) => v.name).join(', ')}`,
            },
        });
    }

    // Add symbols from other documents
    for (const [, pr] of allParseResults) {
        for (const fn of pr.functions) {
            if (!items.find((i) => i.label === fn.name)) {
                items.push({
                    label: fn.name,
                    kind: CompletionItemKind.Function,
                    detail: formatFunctionSignature(fn),
                });
            }
        }
        for (const s of pr.structs) {
            if (!items.find((i) => i.label === s.name)) {
                items.push({
                    label: s.name,
                    kind: CompletionItemKind.Struct,
                    detail: `struct ${s.name}`,
                });
            }
        }
        for (const en of pr.enums) {
            if (!items.find((i) => i.label === en.name)) {
                items.push({
                    label: en.name,
                    kind: CompletionItemKind.Enum,
                    detail: `enum ${en.name}`,
                });
            }
        }
    }

    return items;
}

function formatFunctionSignature(fn: {
    name: string;
    params: { name: string; type: string }[];
    returnType: string;
    errorType: string;
    isAsync: boolean;
    isMethod: boolean;
    structName?: string;
}): string {
    const async = fn.isAsync ? 'async ' : '';
    const prefix = fn.structName ? `${fn.structName}.` : '';
    const params = fn.params.map((p) => (p.type ? `${p.name}: ${p.type}` : p.name)).join(', ');
    let sig = `${async}fn ${prefix}${fn.name}(${params})`;
    if (fn.returnType) sig += ` -> ${fn.returnType}`;
    if (fn.errorType) sig += ` ! ${fn.errorType}`;
    return sig;
}

function resolveVariableType(
    varName: string,
    parseResult: DooParseResult,
    currentLine: number
): string | null {
    // Check variable declarations
    for (const v of parseResult.variables) {
        if (v.name === varName && v.line < currentLine) {
            if (v.type) return v.type;
        }
    }
    // Check function parameters
    for (const fn of parseResult.functions) {
        if (currentLine >= fn.line && currentLine <= fn.endLine) {
            for (const p of fn.params) {
                if (p.name === varName) return p.type;
            }
        }
    }
    return null;
}

function getTypeMembers(type: string, parseResult: DooParseResult): CompletionItem[] {
    switch (type) {
        case 'Request':
            return REQUEST_MEMBERS;
        case 'Response':
            return RESPONSE_MEMBERS;
        case 'Next':
            return NEXT_MEMBERS;
        case 'Server':
            return SERVER_METHOD_COMPLETIONS;
        case 'WsConnection':
            return WS_CONNECTION_METHODS;
        default:
            break;
    }

    // Check if it's a struct type
    for (const s of parseResult.structs) {
        if (s.name === type) {
            const items: CompletionItem[] = s.fields.map((f) => ({
                label: f.name,
                kind: CompletionItemKind.Field,
                detail: f.type,
                documentation: f.decorators.length ? `Decorators: ${f.decorators.map((d) => '@' + d).join(' ')}` : undefined,
            }));
            // Add methods
            for (const fn of parseResult.functions) {
                if (fn.isMethod && fn.structName === type) {
                    items.push({
                        label: fn.name,
                        kind: CompletionItemKind.Method,
                        detail: formatFunctionSignature(fn),
                        documentation: fn.doc || undefined,
                    });
                }
            }
            return items;
        }
    }

    // If type starts with '[', it's an array
    if (type.startsWith('[')) return ARRAY_METHODS;

    // DB-like types
    if (type.includes('Database') || type === 'db') return DB_METHODS;

    return [];
}
