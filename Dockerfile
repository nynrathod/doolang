# Doo Runtime — Base image for compiling and running Doo programs
# Two install modes:
#   CI: DOO_LOCAL_BUNDLE=doo-linux-X.Y.Z.zip  → copies from build context (no download)
#   Manual: leave empty → downloads from GitHub releases using DOO_VERSION tag

FROM ubuntu:24.04

ARG DOO_VERSION=""
ARG DOO_LOCAL_BUNDLE=""

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

# Copy local bundle if provided (CI fast path), otherwise download from GitHub
COPY . /tmp/doo-build-context/
RUN if [ -n "$DOO_LOCAL_BUNDLE" ] && [ -f "/tmp/doo-build-context/${DOO_LOCAL_BUNDLE}" ]; then \
	echo "CI mode: installing from local bundle ${DOO_LOCAL_BUNDLE}..." \
	&& cp "/tmp/doo-build-context/${DOO_LOCAL_BUNDLE}" /tmp/doo.zip; \
	else \
	DOO_TAG="${DOO_VERSION:-}" \
	&& if [ -z "$DOO_TAG" ]; then \
	DOO_TAG=$(curl -fsSL https://api.github.com/repos/nynrathod/doolang/releases/latest \
	| grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' | tr -d '\r'); \
	fi \
	&& DOO_VER=${DOO_TAG#v} \
	&& echo "Downloading doo ${DOO_TAG} (ver=${DOO_VER})..." \
	&& curl -fsSL "https://github.com/nynrathod/doolang/releases/download/${DOO_TAG}/doo-linux-${DOO_VER}.zip" -o /tmp/doo.zip; \
	fi \
	&& unzip -q /tmp/doo.zip -d /tmp \
	&& EXTRACT_DIR=$(find /tmp/doo-linux-* -maxdepth 0 -type d | head -1) \
	&& cp "$EXTRACT_DIR/doo" /usr/local/bin/doo \
	&& chmod +x /usr/local/bin/doo \
	&& mkdir -p /usr/local/lib /usr/local/share/doo \
	&& (cp "$EXTRACT_DIR"/lib/*.a /usr/local/lib/ 2>/dev/null || true) \
	&& (cp -r "$EXTRACT_DIR/std" /usr/local/share/doo/std 2>/dev/null || true) \
	&& (cp -r "$EXTRACT_DIR/packages" /usr/local/share/doo/packages 2>/dev/null || true) \
	&& rm -rf /tmp/doo.zip /tmp/doo-linux-* /tmp/doo-build-context

# Environment
ENV DOO_STDLIB_PATH=/usr/local/share/doo/std
ENV DOO_PACKAGES_PATH=/usr/local/share/doo/packages

# Verify installation
RUN doo --version

WORKDIR /app
