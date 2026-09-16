#!/bin/sh
# agentwiki installer — fetch the latest release binary for this platform.
#   curl -fsSL https://raw.githubusercontent.com/tidusvn05/agentwiki/main/install.sh | sh
set -eu

REPO="tidusvn05/agentwiki"
BIN="agentwiki"
DEST="${DEST:-$HOME/.local/bin}"

os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m)"

case "$os-$arch" in
    linux-x86_64)   target="x86_64-unknown-linux-gnu" ;;
    darwin-arm64)   target="aarch64-apple-darwin" ;;
    darwin-x86_64)  target="x86_64-apple-darwin" ;;
    *)
        echo "unsupported platform: $os-$arch" >&2
        echo "→ build from source instead:" >&2
        echo "  cargo install --git https://github.com/$REPO" >&2
        exit 1
        ;;
esac

url="https://github.com/$REPO/releases/latest/download/$BIN-$target.tar.gz"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "downloading $url"
curl -fsSL "$url" -o "$tmp/$BIN.tar.gz"
tar -xzf "$tmp/$BIN.tar.gz" -C "$tmp"

mkdir -p "$DEST"
install -m 0755 "$tmp/$BIN" "$DEST/$BIN"
echo "installed → $DEST/$BIN"
case ":$PATH:" in
    *":$DEST:"*) ;;
    *) echo "note: $DEST is not on your PATH" ;;
esac
"$DEST/$BIN" --version
