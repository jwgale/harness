#!/usr/bin/env bash
set -euo pipefail

REPO="jwgale/harness"
INSTALL_DIR="${HARNESS_INSTALL_DIR:-$HOME/.local/bin}"
BINARY_NAME="harness"

info() { printf '\033[1;34m%s\033[0m\n' "$*"; }
ok()   { printf '\033[1;32m%s\033[0m\n' "$*"; }
err()  { printf '\033[1;31m%s\033[0m\n' "$*" >&2; }

# Detect architecture
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64)  ARCH="x86_64" ;;
    aarch64) ARCH="aarch64" ;;
    *)       err "Unsupported architecture: $ARCH"; exit 1 ;;
esac

OS="$(uname -s)"
case "$OS" in
    Linux) OS="linux" ;;
    *)     err "Unsupported OS: $OS (Linux only for now)"; exit 1 ;;
esac

mkdir -p "$INSTALL_DIR"

# Try downloading a pre-built binary from GitHub Releases
try_download() {
    local latest_url="https://api.github.com/repos/$REPO/releases/latest"
    local release_json

    release_json=$(curl -fsSL "$latest_url" 2>/dev/null) || return 1

    local asset_url
    asset_url=$(echo "$release_json" | grep -o "\"browser_download_url\": *\"[^\"]*${BINARY_NAME}-${OS}-${ARCH}[^\"]*\"" | head -1 | cut -d'"' -f4)

    if [ -z "$asset_url" ]; then
        return 1
    fi

    info "Downloading pre-built binary..."
    curl -fsSL -o "$INSTALL_DIR/$BINARY_NAME" "$asset_url" || return 1
    chmod +x "$INSTALL_DIR/$BINARY_NAME"
    return 0
}

# Build from source as fallback
build_from_source() {
    info "No pre-built binary found. Building from source..."

    if ! command -v cargo &>/dev/null; then
        err "cargo not found. Install Rust first: https://rustup.rs"
        exit 1
    fi

    local tmp_dir
    tmp_dir=$(mktemp -d)
    trap 'rm -rf "$tmp_dir"' EXIT

    info "Cloning repository..."
    git clone --depth 1 "https://github.com/$REPO.git" "$tmp_dir/harness" 2>/dev/null

    info "Building (release mode)..."
    cargo build --release --manifest-path "$tmp_dir/harness/Cargo.toml"

    cp "$tmp_dir/harness/target/release/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
    chmod +x "$INSTALL_DIR/$BINARY_NAME"
}

info "Installing harness to $INSTALL_DIR..."

if ! try_download; then
    build_from_source
fi

# Create XDG directories
mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}/harness/plugins"
mkdir -p "${XDG_DATA_HOME:-$HOME/.local/share}/harness"
mkdir -p "${XDG_CACHE_HOME:-$HOME/.cache}/harness"

# Verify install
if "$INSTALL_DIR/$BINARY_NAME" --version &>/dev/null; then
    ok ""
    ok "harness installed successfully!"
    ok ""
    VERSION=$("$INSTALL_DIR/$BINARY_NAME" --version)
    info "  $VERSION"
    info "  Location: $INSTALL_DIR/$BINARY_NAME"
    echo ""

    # Check if install dir is in PATH
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *)
            echo "Add to your PATH if not already:"
            echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
            echo ""
            ;;
    esac

    echo "Get started:"
    echo "  mkdir my-project && cd my-project"
    echo "  harness init \"Build a CLI todo app in Rust\""
    echo "  harness run --backend claude"
else
    err "Installation failed — binary not working."
    exit 1
fi
