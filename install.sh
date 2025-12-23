#!/bin/bash
#
# Doo Programming Language Installer
# One-line install: curl -fsSL https://raw.githubusercontent.com/nynrathod/doolang/main/install.sh | bash
#

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color
BOLD='\033[1m'

# Installation directory
INSTALL_DIR="$HOME/.doo"
BIN_DIR="$INSTALL_DIR/bin"

# GitHub repo info
GITHUB_REPO="nynrathod/doolang"

print_banner() {
    echo -e "${CYAN}"
    echo "  ____              "
    echo " |  _ \  ___   ___  "
    echo " | | | |/ _ \ / _ \ "
    echo " | |_| | (_) | (_) |"
    echo " |____/ \___/ \___/ "
    echo -e "${NC}"
    echo -e "${BOLD}Doo Programming Language Installer${NC}"
    echo ""
}

info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
    exit 1
}

# Detect OS and architecture
detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux*)
            PLATFORM="linux"
            ;;
        Darwin*)
            PLATFORM="mac"
            ;;
        *)
            error "Unsupported operating system: $OS. Use install.ps1 for Windows."
            ;;
    esac

    info "Detected platform: $PLATFORM ($ARCH)"
}

# Get latest release version from GitHub API
get_latest_version() {
    # TEST MODE: Use specific version instead of latest
    # info "Fetching latest version..."
    
    # if command -v curl &> /dev/null; then
    #     VERSION=$(curl -fsSL "https://api.github.com/repos/$GITHUB_REPO/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')
    # elif command -v wget &> /dev/null; then
    #     VERSION=$(wget -qO- "https://api.github.com/repos/$GITHUB_REPO/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')
    # else
    #     error "Neither curl nor wget found. Please install one of them."
    # fi

    # if [ -z "$VERSION" ]; then
    #     error "Failed to fetch latest version. Check your internet connection."
    # fi
    
    # For testing:
    VERSION="v0.3.0-pre"

    # Remove 'v' prefix if present for the download URL
    VERSION_NUM="${VERSION#v}"
    
    info "Using version: $VERSION"
}

# Download and extract the release
download_and_extract() {
    DOWNLOAD_URL="https://github.com/$GITHUB_REPO/releases/download/$VERSION/doo-$PLATFORM-$VERSION_NUM.zip"
    TEMP_DIR=$(mktemp -d)
    ZIP_FILE="$TEMP_DIR/doo.zip"

    info "Downloading from: $DOWNLOAD_URL"
    
    if command -v curl &> /dev/null; then
        curl -fsSL "$DOWNLOAD_URL" -o "$ZIP_FILE" || error "Download failed. Please check if the release exists for your platform."
    elif command -v wget &> /dev/null; then
        wget -q "$DOWNLOAD_URL" -O "$ZIP_FILE" || error "Download failed. Please check if the release exists for your platform."
    fi

    info "Extracting files..."
    
    # Create installation directory
    mkdir -p "$BIN_DIR"
    
    # Extract zip file
    if command -v unzip &> /dev/null; then
        unzip -q -o "$ZIP_FILE" -d "$TEMP_DIR/extracted"
    else
        error "unzip not found. Please install unzip: sudo apt install unzip (Linux) or brew install unzip (macOS)"
    fi

    # Find and copy all files to bin directory
    # Handle various folder structures: look for doo binary in the extracted content
    DOO_FOUND=false
    
    # Check if doo is directly in extracted dir
    if [ -f "$TEMP_DIR/extracted/doo" ]; then
        cp -r "$TEMP_DIR/extracted"/* "$BIN_DIR/"
        DOO_FOUND=true
    else
        # Look for doo in subdirectories
        for dir in "$TEMP_DIR/extracted"/*/; do
            if [ -f "${dir}doo" ]; then
                cp -r "${dir}"* "$BIN_DIR/"
                DOO_FOUND=true
                break
            fi
        done
    fi
    
    # Fallback: just copy everything from first subdirectory
    if [ "$DOO_FOUND" = false ]; then
        first_dir=$(find "$TEMP_DIR/extracted" -mindepth 1 -maxdepth 1 -type d | head -1)
        if [ -n "$first_dir" ]; then
            cp -r "$first_dir"/* "$BIN_DIR/"
        else
            cp -r "$TEMP_DIR/extracted"/* "$BIN_DIR/"
        fi
    fi

    # Make the binary executable
    chmod +x "$BIN_DIR/doo"
    
    # Also chmod any .so files
    find "$BIN_DIR" -name "*.so" -exec chmod +x {} \; 2>/dev/null || true
    find "$BIN_DIR" -name "*.dylib" -exec chmod +x {} \; 2>/dev/null || true

    # Cleanup
    rm -rf "$TEMP_DIR"
    
    success "Files extracted to $BIN_DIR"
}

# Setup PATH
setup_path() {
    info "Setting up PATH..."
    
    SHELL_NAME=$(basename "$SHELL")
    PROFILE_FILE=""
    
    case "$SHELL_NAME" in
        bash)
            if [ -f "$HOME/.bash_profile" ]; then
                PROFILE_FILE="$HOME/.bash_profile"
            elif [ -f "$HOME/.bashrc" ]; then
                PROFILE_FILE="$HOME/.bashrc"
            else
                PROFILE_FILE="$HOME/.bashrc"
            fi
            ;;
        zsh)
            PROFILE_FILE="$HOME/.zshrc"
            ;;
        fish)
            PROFILE_FILE="$HOME/.config/fish/config.fish"
            ;;
        *)
            PROFILE_FILE="$HOME/.profile"
            ;;
    esac

    # Check if PATH already contains doo
    if echo "$PATH" | grep -q "$BIN_DIR"; then
        info "PATH already contains $BIN_DIR"
    else
        EXPORT_LINE="export PATH=\"\$PATH:$BIN_DIR\""
        
        if [ "$SHELL_NAME" = "fish" ]; then
            EXPORT_LINE="set -gx PATH \$PATH $BIN_DIR"
        fi
        
        # Check if the line already exists in the profile file
        if [ -f "$PROFILE_FILE" ] && grep -q "$BIN_DIR" "$PROFILE_FILE"; then
            info "PATH already configured in $PROFILE_FILE"
        else
            echo "" >> "$PROFILE_FILE"
            echo "# Doo Programming Language" >> "$PROFILE_FILE"
            echo "$EXPORT_LINE" >> "$PROFILE_FILE"
            success "Added PATH to $PROFILE_FILE"
        fi
    fi
    
    # Export for current session
    export PATH="$PATH:$BIN_DIR"
}

# Install clang if needed (for linking)
check_dependencies() {
    info "Checking dependencies..."
    
    if ! command -v clang &> /dev/null; then
        warn "clang is not installed. Doo requires clang for linking."
        echo ""
        if [ "$PLATFORM" = "linux" ]; then
            echo -e "  Install with: ${CYAN}sudo apt install clang${NC}"
            echo -e "  Or: ${CYAN}sudo yum install clang${NC}"
        else
            echo -e "  Install with: ${CYAN}xcode-select --install${NC}"
        fi
        echo ""
    else
        success "clang is installed"
    fi
}

# Verify installation
verify_installation() {
    info "Verifying installation..."
    
    if [ -f "$BIN_DIR/doo" ]; then
        success "Doo installed successfully!"
        echo ""
        echo -e "${BOLD}Installation complete!${NC}"
        echo ""
        echo -e "  Binary location: ${CYAN}$BIN_DIR/doo${NC}"
        echo ""
        
        # Check if doo is accessible
        if command -v doo &> /dev/null; then
            echo -e "${GREEN}✓${NC} doo command is available in current session"
            echo ""
            echo -e "  Run ${CYAN}doo --help${NC} to get started"
        else
            echo -e "${YELLOW}!${NC} To use doo in this terminal session, run:"
            echo ""
            echo -e "  ${CYAN}source $PROFILE_FILE${NC}"
            echo ""
            echo -e "  Or open a new terminal window."
        fi
        echo ""
    else
        error "Installation verification failed. Binary not found."
    fi
}

# Anonymous install tracking via PostHog (no private data, fire-and-forget)
# Get your API key from: https://app.posthog.com → Project Settings
POSTHOG_API_KEY="phc_REPLACE_WITH_YOUR_KEY"
POSTHOG_HOST="https://us.i.posthog.com"

send_analytics() {
    # Skip if API key not configured
    if [[ "$POSTHOG_API_KEY" == *"REPLACE"* ]]; then
        return
    fi
    
    # Generate anonymous ID from hostname hash
    ANON_ID="doo_$(echo -n "$(hostname)_$PLATFORM" | md5sum | cut -c1-16)"
    
    (curl -s -X POST "$POSTHOG_HOST/capture/" \
        -H "Content-Type: application/json" \
        -d "{
            \"api_key\": \"$POSTHOG_API_KEY\",
            \"event\": \"install_complete\",
            \"distinct_id\": \"$ANON_ID\",
            \"properties\": {
                \"\$os\": \"$PLATFORM\",
                \"doo_version\": \"$VERSION\"
            }
        }" \
        --max-time 2 \
        >/dev/null 2>&1 || true) &
}

# Main installation flow
main() {
    print_banner
    detect_platform
    get_latest_version
    download_and_extract
    setup_path
    check_dependencies
    verify_installation
    send_analytics
}

main
