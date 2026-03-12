# Doo Language — VS Code Extension

> **Work in Progress** — This extension is under active development. Features may change, and some functionality is incomplete. Contributions and feedback are welcome!

Full-featured language support for the **Doo** programming language.

## Features

- **Syntax Highlighting** — Rust-like color theming for all Doo constructs
- **Type Error Diagnostics** — Real-time type checking powered by the native Doo compiler (type mismatches, undefined variables, missing returns, etc.)
- **Parse Error Diagnostics** — Missing semicolons, unmatched brackets, unclosed strings
- **Autocomplete** — Keywords, types, std library functions, struct fields, enum variants, decorators
- **Go-to-Definition** — Ctrl+Click to jump to function, struct, enum, or variable declarations
- **Hover Information** — Type signatures, documentation, and struct/enum details on hover
- **Document Outline** — Functions, structs, enums in the sidebar outline view
- **Signature Help** — Parameter hints when typing function calls
- **Snippet Support** — Smart snippets for `fn`, `struct`, `enum`, `match`, `for`, `go`, `scope`, `import`, etc.

## Supported Syntax

| Construct | Examples |
|---|---|
| Functions | `fn name()`, `async fn name() -> Type`, `fn Type.method(self)` |
| Structs | `struct Name { field: Type @decorator }` |
| Enums | `enum Name { Variant(Type) }`, `enum Name: A \| B \| C;` |
| Variables | `let name = value`, `let mut name: Type = value` |
| Control flow | `if/else`, `match value { pattern => expr }`, `for i in 0..10` |
| Error handling | `-> Type ! ErrorType`, `Ok value`, `Err value`, `?` propagation |
| Async | `async fn`, `await`, `go { }`, `scope { }`, `sleep(ms)` |
| Imports | `import std::Module::{Item}`, `import std::Module::{Item as Alias}` |
| Decorators | `@primary`, `@auto`, `@hash`, `@email`, `@unique`, `@writeOnly`, `@readOnly`, `@internal`, `@default(value)` |
| Types | `Int`, `Str`, `Float`, `Bool`, `[T]`, `{K: V}` |
| String interpolation | `"Hello ${name}"` |
| Spread | `[...arr1, ...arr2]` |
| Optionals | `field?: Type` |

## Getting Started

1. Open the extension folder in VS Code
2. Run `npm install`
3. Run `npm run compile`
4. Press **F5** to launch Extension Development Host
5. Open any `.doo` file to see the extension in action

## Standard Library Completions

The extension provides completions for:

- `std::Math` — `Abs`, `Sqrt`, `Pow`
- `std::Array` — `Sum`
- `std::Config` — `get`, `getOr`, `set`, `has`, `getInt`, `getBool`
- `std::File` — `Read`, `Write`, `Exists`, `Delete`, `Metadata`
- `std::Random` — `Int`
- `std::Http` — `Server`, `Request`, `Response`, `Next`, `WsConnection`
- `std::Database` — `Postgres`, `get`, `raw`, `rawWithParams`
- `std::Auth` — `Jwt`

## Architecture

The extension runs in two modes:

- **Native LSP** (recommended) — Uses the `doo-lsp` binary compiled from `crates/doo_lsp/`. Provides full parsing, type checking, and semantic analysis powered by the Doo compiler. The extension automatically discovers the binary from `target/release/` or `target-windows/release/`.
- **TypeScript Fallback** — If no native binary is found, falls back to a lightweight TypeScript server with basic bracket/string diagnostics.

## Contributing

This extension is a work in progress and contributions are welcome! Here's how to get started:

1. **Clone the repo** and open the `editors/vscode/doo-lang/` folder
2. Run `npm install` to get dependencies
3. Run `npm run compile` to build
4. Press **F5** to launch an Extension Development Host for testing
5. To install locally: `npm run install-ext`

### Areas that need help

- **Improved autocomplete** — Context-aware method suggestions (e.g., `app.` showing methods of `app`'s type)
- **Better diagnostics** — More comprehensive error messages and quick-fix suggestions
- **Formatter** — Auto-formatting for Doo source files
- **Debugger integration** — Step-through debugging support
- **Multi-file analysis** — Cross-file type checking and go-to-definition

### Reporting issues

If you find a bug or have a feature request, please open an issue on the [Doo repository](https://github.com/nicholasgasior/doo).
