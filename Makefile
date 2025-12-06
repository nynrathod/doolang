# Makefile for doo compiler
# Simple workspace-based build system

.PHONY: all build release clean test help

# Default target
all: build

# Build everything (debug mode) - ONE COMMAND
build:
	@echo "🔨 Building doo compiler (debug)..."
	@cargo build --workspace
	@echo "📦 Copying FFI libraries to target/debug..."
	@cp -f target/debug/deps/libdoo_file.* target/debug/ 2>/dev/null || true

# Build release version - ONE COMMAND
release:
	@echo "🚀 Building doo compiler (release)..."
	@cargo build --workspace --release
	@echo "📦 Copying FFI libraries to target/release..."
	@cp -f target/release/deps/libdoo_file.* target/release/ 2>/dev/null || true
	@echo "✅ Build complete! Files in target/release/"
	@echo ""
	@echo "Add to PATH:"
	@echo "  export PATH=\"$$PWD/target/release:\$$PATH\""

# Clean build artifacts
clean:
	@echo "🧹 Cleaning build artifacts..."
	@cargo clean

# Run tests
test:
	@echo "🧪 Running tests..."
	@cargo test

# Quick rebuild (no clean, just rebuild changed files)
quick:
	@cargo build --workspace --release
	@cp -f target/release/deps/libdoo_file.* target/release/ 2>/dev/null || true

# Check code without building
check:
	@cargo check

# Format code
fmt:
	@cargo fmt --all

# Run clippy linter
lint:
	@cargo clippy --all

# Show help
help:
	@echo "Doo Compiler - Build System"
	@echo ""
	@echo "Available targets:"
	@echo "  make           - Build debug version"
	@echo "  make release   - Build release version (RECOMMENDED)"
	@echo "  make quick     - Fast rebuild (only changed files)"
	@echo "  make clean     - Remove all build artifacts"
	@echo "  make test      - Run tests"
	@echo "  make check     - Check code without building"
	@echo "  make fmt       - Format all code"
	@echo "  make lint      - Run clippy linter"
	@echo "  make help      - Show this help"
	@echo ""
	@echo "Quick start:"
	@echo "  make release   # Build everything with one command"
	@echo ""
	@echo "Note: Workspace automatically rebuilds only changed files!"
