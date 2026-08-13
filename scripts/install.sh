#!/usr/bin/env bash
# Install conversation-handoff from GitHub Releases (no Rust required).
set -euo pipefail

REPO="${CONVERSATION_HANDOFF_REPO:-nblotti/conversation-handoff}"
INSTALL_DIR="${CONVERSATION_HANDOFF_BIN:-$HOME/.local/bin}"
NAME="conversation-handoff"

uname_s="$(uname -s)"
uname_m="$(uname -m)"
case "$uname_s" in
  Linux)
    case "$uname_m" in
      x86_64|amd64) asset="${NAME}-linux-x86_64" ;;
      aarch64|arm64) asset="${NAME}-linux-aarch64" ;;
      *) echo "Unsupported Linux arch: $uname_m" >&2; exit 1 ;;
    esac
    ;;
  Darwin)
    case "$uname_m" in
      x86_64) asset="${NAME}-macos-x86_64" ;;
      arm64) asset="${NAME}-macos-aarch64" ;;
      *) echo "Unsupported macOS arch: $uname_m" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "Use scripts/install.ps1 on Windows." >&2
    exit 1
    ;;
esac

mkdir -p "$INSTALL_DIR"
url="https://github.com/${REPO}/releases/latest/download/${asset}"
echo "Downloading $url"
tmp="$(mktemp)"
if command -v curl >/dev/null 2>&1; then
  curl -fL --retry 3 -o "$tmp" "$url"
elif command -v wget >/dev/null 2>&1; then
  wget -O "$tmp" "$url"
else
  echo "Need curl or wget." >&2
  exit 1
fi
chmod +x "$tmp"
mv "$tmp" "$INSTALL_DIR/$NAME"
echo "Installed $INSTALL_DIR/$NAME"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo "Add $INSTALL_DIR to PATH if $NAME is not found after this script."
    ;;
esac

"$INSTALL_DIR/$NAME" install --write-instructions
echo "Done. Start a new Claude Code or Codex session and approve the tools."
