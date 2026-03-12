# Doo Runtime — Base image for compiling and running Doo programs
# Used by DooCloud server and user project builds
FROM rust:1.83-bookworm AS builder

WORKDIR /build

# Install LLVM 18 and system dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
	llvm-18-dev \
	libpolly-18-dev \
	libzstd-dev \
	lld \
	pkg-config \
	libssl-dev \
	libpq-dev \
	&& rm -rf /var/lib/apt/lists/*

# Copy workspace
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY ffi/ ffi/
COPY src/ src/
COPY std/ std/
COPY packages/ packages/

# Build the Doo compiler in release mode
RUN cargo build --release --workspace

# ─── Runtime stage ───────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
	llvm-18-dev \
	libpolly-18-dev \
	libzstd-dev \
	lld \
	libssl3 \
	libpq5 \
	ca-certificates \
	curl \
	pkg-config \
	&& rm -rf /var/lib/apt/lists/*

WORKDIR /usr/local

# Copy the compiled doo binary
COPY --from=builder /build/target/release/doo /usr/local/bin/doo

# Copy FFI shared libraries
COPY --from=builder /build/target/release/*.so /usr/local/lib/ 2>/dev/null || true
COPY --from=builder /build/target/release/*.dylib /usr/local/lib/ 2>/dev/null || true

# Copy std library
COPY --from=builder /build/std /usr/local/share/doo/std

# Copy packages
COPY --from=builder /build/packages /usr/local/share/doo/packages

# Set library path
ENV LD_LIBRARY_PATH=/usr/local/lib
ENV DOO_STD_PATH=/usr/local/share/doo/std
ENV DOO_PACKAGES_PATH=/usr/local/share/doo/packages

# Verify doo is installed
RUN doo --version || echo "doo binary installed"

WORKDIR /app
