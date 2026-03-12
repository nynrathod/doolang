import * as path from "path";
import * as fs from "fs";
import * as vscode from "vscode";
import { execSync } from "child_process";
import {
	LanguageClient,
	LanguageClientOptions,
	ServerOptions,
	TransportKind,
	Executable,
} from "vscode-languageclient/node";
import {
	parseDooSource,
	DooParseResult,
	DooFunction,
	DooStruct,
	DooEnum,
} from "./parser";

let client: LanguageClient;

// Global symbol index: uri → parse result
const symbolIndex = new Map<string, DooParseResult>();

/**
 * Find the source doo-lsp binary from cargo target directories.
 */
function findSourceLspBinary(): string | null {
	const binaryName = process.platform === "win32" ? "doo-lsp.exe" : "doo-lsp";
	const workspaceFolders = vscode.workspace.workspaceFolders;
	if (workspaceFolders) {
		for (const folder of workspaceFolders) {
			const candidates = [
				path.join(folder.uri.fsPath, "target", "release", binaryName),
				path.join(
					folder.uri.fsPath,
					"target-windows",
					"release",
					binaryName,
				),
				path.join(folder.uri.fsPath, "target", "debug", binaryName),
			];
			for (const candidate of candidates) {
				if (fs.existsSync(candidate)) {
					return candidate;
				}
			}
		}
	}
	return null;
}

/**
 * Try to find the native doo-lsp binary.
 * Like rust-analyzer: copies binary to globalStoragePath so the source
 * binary in target/ is never locked during cargo rebuilds.
 */
function findNativeLspBinary(globalStoragePath: string): string | null {
	const config = vscode.workspace.getConfiguration("doo.lsp");
	const configPath = config.get<string>("path", "");

	// 1. Explicit config path — user override, use directly
	if (configPath && fs.existsSync(configPath)) {
		return configPath;
	}

	// 2. Check PATH
	const binaryName = process.platform === "win32" ? "doo-lsp.exe" : "doo-lsp";
	try {
		const whichCmd = process.platform === "win32" ? "where" : "which";
		const result = execSync(`${whichCmd} ${binaryName}`, {
			encoding: "utf8",
		}).trim();
		if (result && fs.existsSync(result.split("\n")[0])) {
			return result.split("\n")[0];
		}
	} catch {
		// Not in PATH
	}

	// 3. Copy from cargo target to globalStoragePath (rust-analyzer pattern)
	const sourceBinary = findSourceLspBinary();
	if (sourceBinary) {
		if (!fs.existsSync(globalStoragePath)) {
			fs.mkdirSync(globalStoragePath, { recursive: true });
		}
		const destBinary = path.join(globalStoragePath, binaryName);

		// Check if we need to update (source newer than dest)
		let needsCopy = !fs.existsSync(destBinary);
		if (!needsCopy) {
			try {
				const srcStat = fs.statSync(sourceBinary);
				const dstStat = fs.statSync(destBinary);
				needsCopy = srcStat.mtimeMs > dstStat.mtimeMs;
			} catch {
				needsCopy = true;
			}
		}

		if (needsCopy) {
			try {
				// On Windows, rename the running binary first (allowed by OS),
				// then copy the new one in its place
				const oldDest = destBinary + ".old";
				if (fs.existsSync(destBinary)) {
					try {
						fs.renameSync(destBinary, oldDest);
					} catch {
						/* ignore */
					}
				}
				fs.copyFileSync(sourceBinary, destBinary);
				// Clean up old binary (may fail if still running — that's okay)
				try {
					fs.unlinkSync(oldDest);
				} catch {
					/* ignore */
				}
			} catch {
				// Copy failed — use existing copy if available, else source directly
				if (fs.existsSync(destBinary)) {
					return destBinary;
				}
				return sourceBinary;
			}
		}

		return destBinary;
	}

	return null;
}

export function activate(context: ExtensionContext) {
	const config = vscode.workspace.getConfiguration("doo.lsp");
	const useNative = config.get<boolean>("useNative", true);

	let serverOptions: ServerOptions;

	const nativeBinary = useNative
		? findNativeLspBinary(context.globalStorageUri.fsPath)
		: null;

	if (nativeBinary) {
		// ─── Use native doo-lsp binary (compiler-backed) ───
		const run: Executable = {
			command: nativeBinary,
			transport: TransportKind.stdio,
		};
		const debug: Executable = {
			command: nativeBinary,
			transport: TransportKind.stdio,
			options: { env: { ...process.env, DOO_LSP_LOG: "debug" } },
		};
		serverOptions = { run, debug };
		vscode.window.showInformationMessage(
			`Doo LSP: using native server at ${nativeBinary}`,
		);
	} else {
		// ─── Fallback to TypeScript server ───
		const serverModule = context.asAbsolutePath(
			path.join("out", "server.js"),
		);
		serverOptions = {
			run: { module: serverModule, transport: TransportKind.ipc },
			debug: {
				module: serverModule,
				transport: TransportKind.ipc,
				options: { execArgv: ["--nolazy", "--inspect=6009"] },
			},
		};
	}
	const clientOptions: LanguageClientOptions = {
		documentSelector: [{ scheme: "file", language: "doo" }],
		synchronize: {
			fileEvents: vscode.workspace.createFileSystemWatcher("**/*.doo"),
		},
	};
	client = new LanguageClient(
		"dooLanguageServer",
		"Doo Language Server",
		serverOptions,
		clientOptions,
	);
	client.start();

	// ─── Index all .doo files in workspace ───
	indexWorkspace();

	// Re-index when files change
	const watcher = vscode.workspace.createFileSystemWatcher("**/*.doo");
	watcher.onDidChange((uri) => indexFile(uri.fsPath));
	watcher.onDidCreate((uri) => indexFile(uri.fsPath));
	watcher.onDidDelete((uri) => symbolIndex.delete(uri.toString()));
	context.subscriptions.push(watcher);

	// Re-index on save
	vscode.workspace.onDidSaveTextDocument((doc) => {
		if (doc.languageId === "doo") {
			symbolIndex.set(doc.uri.toString(), parseDooSource(doc.getText()));
		}
	});

	// Index when a .doo file is opened
	vscode.workspace.onDidOpenTextDocument((doc) => {
		if (doc.languageId === "doo") {
			symbolIndex.set(doc.uri.toString(), parseDooSource(doc.getText()));
		}
	});

	// ─── Register Go-to-Definition Provider (directly in client) ───
	const defProvider = vscode.languages.registerDefinitionProvider(
		{ language: "doo", scheme: "file" },
		new DooDefinitionProvider(),
	);
	context.subscriptions.push(defProvider);
}

// ─── Definition Provider ───

class DooDefinitionProvider implements vscode.DefinitionProvider {
	provideDefinition(
		document: vscode.TextDocument,
		position: vscode.Position,
		_token: vscode.CancellationToken,
	): vscode.Definition | null {
		// Get the word under cursor
		const wordRange = document.getWordRangeAtPosition(
			position,
			/[a-zA-Z_]\w*/,
		);
		if (!wordRange) return null;
		const word = document.getText(wordRange);
		if (!word) return null;

		// Parse current document (always fresh)
		const currentUri = document.uri.toString();
		const currentParse = parseDooSource(document.getText());
		symbolIndex.set(currentUri, currentParse);

		// Search current document first
		const localResult = findSymbol(word, currentParse, document.uri);
		if (localResult) return localResult;

		// Search all indexed files
		for (const [uriStr, parseResult] of symbolIndex) {
			if (uriStr === currentUri) continue;
			const uri = vscode.Uri.parse(uriStr);
			const result = findSymbol(word, parseResult, uri);
			if (result) return result;
		}

		return null;
	}
}

function findSymbol(
	word: string,
	parseResult: DooParseResult,
	uri: vscode.Uri,
): vscode.Location | null {
	// Functions
	for (const fn of parseResult.functions) {
		if (fn.name === word || fn.fullName === word) {
			return new vscode.Location(uri, new vscode.Position(fn.line, 0));
		}
	}

	// Structs
	for (const s of parseResult.structs) {
		if (s.name === word) {
			return new vscode.Location(uri, new vscode.Position(s.line, 0));
		}
	}

	// Enums
	for (const en of parseResult.enums) {
		if (en.name === word) {
			return new vscode.Location(uri, new vscode.Position(en.line, 0));
		}
		// Enum variants
		for (const v of en.variants) {
			if (v.name === word) {
				return new vscode.Location(uri, new vscode.Position(v.line, 0));
			}
		}
	}

	// Variables (only in same file)
	for (const v of parseResult.variables) {
		if (v.name === word) {
			return new vscode.Location(uri, new vscode.Position(v.line, 0));
		}
	}

	return null;
}

// ─── Workspace Indexing ───

async function indexWorkspace(): Promise<void> {
	const files = await vscode.workspace.findFiles(
		"**/*.doo",
		"**/node_modules/**",
	);
	for (const file of files) {
		indexFile(file.fsPath);
	}
}

function indexFile(filePath: string): void {
	try {
		const text = fs.readFileSync(filePath, "utf-8");
		const uri = vscode.Uri.file(filePath).toString();
		symbolIndex.set(uri, parseDooSource(text));
	} catch {
		// Skip files that can't be read
	}
}

// ─── Type alias for cleaner code ───
type ExtensionContext = vscode.ExtensionContext;

export function deactivate(): Thenable<void> | undefined {
	if (!client) {
		return undefined;
	}
	return client.stop();
}
