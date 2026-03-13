# Doo Runtime — Base image for compiling and running Doo programs
# Downloads pre-built doo binary from GitHub releases (no Rust compilation needed)
# LLVM is statically linked in the doo binary — only needs clang as system linker

FROM debian:bookworm-slim

ARG DOO_VERSION=""

# clang = system linker, build-essential = linking tools
RUN apt-get update && apt-get install -y --no-install-recommends \
	curl \
	clang \
	lld \
	build-essential \
	libssl-dev \
	libssl3 \
	libpq-dev \
	libpq5 \
	zlib1g-dev \
	pkg-config \
	ca-certificates \
	unzip \
	&& rm -rf /var/lib/apt/lists/*

# Install doo compiler from GitHub release
RUN DOO_TAG=${DOO_VERSION:-$(curl -fsSL https://api.github.com/repos/nynrathod/doolang/releases/latest | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')} \
	&& DOO_VER=${DOO_TAG#v} \
	&& echo "Installing doo ${DOO_TAG}..." \
	&& curl -fsSL "https://github.com/nynrathod/doolang/releases/download/${DOO_TAG}/doo-linux-${DOO_VER}.zip" -o /tmp/doo.zip \
	&& unzip -q /tmp/doo.zip -d /tmp \
	&& EXTRACT_DIR=$(find /tmp/doo-linux-* -maxdepth 0 -type d | head -1) \
	&& cp "$EXTRACT_DIR/doo" /usr/local/bin/doo \
	&& chmod +x /usr/local/bin/doo \
	&& mkdir -p /usr/local/lib /usr/local/share/doo \
	&& (cp "$EXTRACT_DIR"/lib/*.a /usr/local/lib/ 2>/dev/null || true) \
	&& (cp -r "$EXTRACT_DIR/std" /usr/local/share/doo/std 2>/dev/null || true) \
	&& (cp -r "$EXTRACT_DIR/packages" /usr/local/share/doo/packages 2>/dev/null || true) \
	&& rm -rf /tmp/doo.zip /tmp/doo-linux-*

# Environment
ENV DOO_STDLIB_PATH=/usr/local/share/doo/std
ENV DOO_PACKAGES_PATH=/usr/local/share/doo/packages

# Verify installation
RUN doo --version

WORKDIR /app
