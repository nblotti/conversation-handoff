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
      *)
        echo "Unsupported Linux architecture: $uname_m" >&2
        echo "Published Linux builds: ${NAME}-linux-x86_64 and ${NAME}-linux-aarch64 (static musl)." >&2
        exit 1
        ;;
    esac
    ;;
  Darwin)
    case "$uname_m" in
      x86_64) asset="${NAME}-macos-x86_64" ;;
      arm64) asset="${NAME}-macos-aarch64" ;;
      *)
        echo "Unsupported macOS architecture: $uname_m" >&2
        exit 1
        ;;
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
download_failed() {
  echo "No prebuilt binary for this platform: OS=$uname_s arch=$uname_m asset=$asset" >&2
  echo "Linux releases are static musl binaries and do not require GLIBC or Rust." >&2
  echo "This installer does not compile from source." >&2
  rm -f "$tmp"
  exit 1
}
if command -v curl >/dev/null 2>&1; then
  curl -fL --retry 3 -o "$tmp" "$url" || download_failed
elif command -v wget >/dev/null 2>&1; then
  wget -O "$tmp" "$url" || download_failed
else
  echo "Need curl or wget." >&2
  exit 1
fi
chmod +x "$tmp"

if ! "$tmp" --version >/tmp/ch-version.out 2>/tmp/ch-version.err; then
  echo "The downloaded binary cannot run on this system." >&2
  echo "OS=$uname_s arch=$uname_m asset=$asset" >&2
  if [[ -s /tmp/ch-version.err ]]; then
    cat /tmp/ch-version.err >&2
  fi
  echo "This platform is not supported by the published release. This installer does not fall back to cargo build." >&2
  rm -f "$tmp" /tmp/ch-version.out /tmp/ch-version.err
  exit 1
fi
rm -f /tmp/ch-version.out /tmp/ch-version.err

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
