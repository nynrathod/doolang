/**
 * Lightweight regex-based parser for Doo source files.
 * Extracts functions, structs, enums, imports, and variables for LSP features.
 */

export interface DooFunction {
    name: string;
    fullName: string; // e.g. "User.isAdult" for methods
    params: { name: string; type: string }[];
    returnType: string;
    errorType: string;
    isAsync: boolean;
    isMethod: boolean;
    structName?: string;
    line: number;
    endLine: number;
    doc: string;
}

export interface DooStructField {
    name: string;
    type: string;
    decorators: string[];
    optional: boolean;
    line: number;
}

export interface DooStruct {
    name: string;
    fields: DooStructField[];
    line: number;
    endLine: number;
}

export interface DooEnumVariant {
    name: string;
    payloadTypes: string[];
    line: number;
}

export interface DooEnum {
    name: string;
    variants: DooEnumVariant[];
    inline: boolean;
    line: number;
    endLine: number;
}

export interface DooImport {
    path: string; // e.g. "std::Http"
    items: { name: string; alias?: string }[];
    line: number;
}

export interface DooVariable {
    name: string;
    type: string;
    mutable: boolean;
    line: number;
}

export interface DooParseResult {
    functions: DooFunction[];
    structs: DooStruct[];
    enums: DooEnum[];
    imports: DooImport[];
    variables: DooVariable[];
}

export function parseDooSource(text: string): DooParseResult {
    const lines = text.split(/\r?\n/);
    const result: DooParseResult = {
        functions: [],
        structs: [],
        enums: [],
        imports: [],
        variables: [],
    };

    let i = 0;
    while (i < lines.length) {
        const line = lines[i];
        const trimmed = line.trim();

        // Skip comments and empty lines
        if (trimmed.startsWith('//') || trimmed === '') {
            i++;
            continue;
        }

        // --- Import statements ---
        const importMatch = trimmed.match(
            /^import\s+([\w:]+)(?:::(\w+))?(?:::\{(.+)\})?;?\s*$/
        );
        if (importMatch) {
            const basePath = importMatch[1];
            const singleItem = importMatch[2];
            const bracedItems = importMatch[3];
            const items: { name: string; alias?: string }[] = [];

            if (bracedItems) {
                for (const part of bracedItems.split(',')) {
                    const asMatch = part.trim().match(/^(\w+)\s+as\s+(\w+)$/);
                    if (asMatch) {
                        items.push({ name: asMatch[1], alias: asMatch[2] });
                    } else {
                        const name = part.trim();
                        if (name) items.push({ name });
                    }
                }
            } else if (singleItem) {
                items.push({ name: singleItem });
            }

            result.imports.push({
                path: basePath + (singleItem && !bracedItems ? '::' + singleItem : ''),
                items,
                line: i,
            });
            i++;
            continue;
        }

        // --- Inline enum: enum Color: Red | Green | Blue; ---
        const inlineEnumMatch = trimmed.match(
            /^enum\s+([A-Z]\w*)\s*:\s*(.+);$/
        );
        if (inlineEnumMatch) {
            const variants = inlineEnumMatch[2].split('|').map((v) => ({
                name: v.trim(),
                payloadTypes: [] as string[],
                line: i,
            }));
            result.enums.push({
                name: inlineEnumMatch[1],
                variants,
                inline: true,
                line: i,
                endLine: i,
            });
            i++;
            continue;
        }

        // --- Block enum ---
        const blockEnumMatch = trimmed.match(/^enum\s+([A-Z]\w*)\s*\{/);
        if (blockEnumMatch) {
            const enumDef: DooEnum = {
                name: blockEnumMatch[1],
                variants: [],
                inline: false,
                line: i,
                endLine: i,
            };
            i++;
            while (i < lines.length) {
                const eLine = lines[i].trim();
                if (eLine === '}' || eLine === '};') {
                    enumDef.endLine = i;
                    break;
                }
                const variantMatch = eLine.match(
                    /^([A-Z]\w*)(?:\(([^)]*)\))?\s*,?\s*$/
                );
                if (variantMatch) {
                    const payloads = variantMatch[2]
                        ? variantMatch[2].split(',').map((t) => t.trim())
                        : [];
                    enumDef.variants.push({
                        name: variantMatch[1],
                        payloadTypes: payloads,
                        line: i,
                    });
                }
                i++;
            }
            result.enums.push(enumDef);
            i++;
            continue;
        }

        // --- Struct ---
        const structMatch = trimmed.match(/^struct\s+([A-Z]\w*)\s*\{/);
        if (structMatch) {
            const structDef: DooStruct = {
                name: structMatch[1],
                fields: [],
                line: i,
                endLine: i,
            };
            i++;
            while (i < lines.length) {
                const sLine = lines[i].trim();
                if (sLine === '}' || sLine === '};') {
                    structDef.endLine = i;
                    break;
                }
                // Parse field: name?: Type @decorator @decorator2,
                const fieldMatch = sLine.match(
                    /^(\w+)(\?)?:\s*(\[?\{?[\w:\s,\[\]\{\}]+\}?\]?)\s*((?:@\w+(?:\([^)]*\))?\s*)*)\s*,?\s*$/
                );
                if (fieldMatch) {
                    const decorators: string[] = [];
                    const decoMatches = fieldMatch[4].matchAll(/@(\w+(?:\([^)]*\))?)/g);
                    for (const dm of decoMatches) {
                        decorators.push(dm[1]);
                    }
                    structDef.fields.push({
                        name: fieldMatch[1],
                        type: fieldMatch[3].trim(),
                        decorators,
                        optional: !!fieldMatch[2],
                        line: i,
                    });
                }
                i++;
            }
            result.structs.push(structDef);
            i++;
            continue;
        }

        // --- Function (async fn / fn / method fn Type.name) ---
        const fnMatch = trimmed.match(
            /^(async\s+)?fn\s+(?:([A-Z]\w*)\.)?([a-zA-Z_]\w*)\s*\(([^)]*)\)(?:\s*->\s*(\[?\{?[\w:\s,\[\]\{\}|?!]+\}?\]?))?(?:\s*!\s*([A-Z]\w*))?\s*\{?/
        );
        if (fnMatch) {
            const isAsync = !!fnMatch[1];
            const structName = fnMatch[2] || undefined;
            const fnName = fnMatch[3];
            const paramsStr = fnMatch[4] || '';
            let returnType = fnMatch[5] || '';
            const errorType = fnMatch[6] || '';

            // Parse return type for inline error types: -> Str ! Int
            const retErrMatch = returnType.match(/^(.+?)\s*!\s*(\w+)$/);
            let actualErrorType = errorType;
            if (retErrMatch) {
                returnType = retErrMatch[1].trim();
                actualErrorType = retErrMatch[2];
            }

            const params: { name: string; type: string }[] = [];
            if (paramsStr.trim()) {
                for (const p of paramsStr.split(',')) {
                    const pm = p.trim().match(/^(\w+)(?::\s*([\w\[\]\{\}:<>, ]+))?$/);
                    if (pm) {
                        params.push({ name: pm[1], type: pm[2] || '' });
                    }
                }
            }

            // Find end of function
            let braceCount = 0;
            let endLine = i;
            for (let j = i; j < lines.length; j++) {
                for (const ch of lines[j]) {
                    if (ch === '{') braceCount++;
                    if (ch === '}') braceCount--;
                }
                if (braceCount === 0 && j > i) {
                    endLine = j;
                    break;
                }
                if (j === lines.length - 1) {
                    endLine = j;
                }
            }

            // Collect doc comment from preceding lines
            let doc = '';
            let docLine = i - 1;
            while (docLine >= 0 && lines[docLine].trim().startsWith('//')) {
                const comment = lines[docLine].trim().replace(/^\/\/\s?/, '');
                doc = comment + (doc ? '\n' + doc : '');
                docLine--;
            }

            result.functions.push({
                name: fnName,
                fullName: structName ? `${structName}.${fnName}` : fnName,
                params,
                returnType: returnType.trim(),
                errorType: actualErrorType,
                isAsync,
                isMethod: !!structName,
                structName,
                line: i,
                endLine,
                doc,
            });

            i = endLine + 1;
            continue;
        }

        // --- Variable declarations ---
        const letMatch = trimmed.match(
            /^let\s+(mut\s+)?(\w+)(?:\s*(?:,\s*\w+)*)?\s*(?::\s*([\w\[\]\{\}:<>, ]+))?\s*=/
        );
        if (letMatch) {
            result.variables.push({
                name: letMatch[2],
                type: letMatch[3] || '',
                mutable: !!letMatch[1],
                line: i,
            });
        }

        i++;
    }

    return result;
}
