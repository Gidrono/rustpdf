#!/usr/bin/env bash
# Fetch a prebuilt PDFium dylib for macOS (Apple Silicon or Intel).
# Source: https://github.com/bblanchon/pdfium-binaries
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/vendor/pdfium"
mkdir -p "$OUT"

ARCH="$(uname -m)"
case "$ARCH" in
  arm64) ASSET="pdfium-mac-arm64.tgz" ;;
  x86_64) ASSET="pdfium-mac-x64.tgz" ;;
  *)
    echo "Unsupported arch: $ARCH" >&2
    exit 1
    ;;
esac

# Pin to a release that matches pdfium-render's pdfium_latest bindings when possible.
# Override with: PDFIUM_RELEASE=chromium/6955 ./scripts/fetch-pdfium.sh
RELEASE="${PDFIUM_RELEASE:-latest}"
BASE="https://github.com/bblanchon/pdfium-binaries/releases"
if [[ "$RELEASE" == "latest" ]]; then
  URL="$BASE/latest/download/$ASSET"
else
  URL="$BASE/download/$RELEASE/$ASSET"
fi

TMP="$(mktemp -d)"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT

echo "Downloading $URL"
curl -L --fail -o "$TMP/pdfium.tgz" "$URL"
tar -xzf "$TMP/pdfium.tgz" -C "$TMP"

# Archive layout: lib/libpdfium.dylib (and headers under include/)
if [[ -f "$TMP/lib/libpdfium.dylib" ]]; then
  cp "$TMP/lib/libpdfium.dylib" "$OUT/libpdfium.dylib"
elif [[ -f "$TMP/libpdfium.dylib" ]]; then
  cp "$TMP/libpdfium.dylib" "$OUT/libpdfium.dylib"
else
  echo "Could not find libpdfium.dylib in archive" >&2
  find "$TMP" -name 'libpdfium*' >&2 || true
  exit 1
fi

# Clear macOS quarantine so dyld can load it.
xattr -dr com.apple.quarantine "$OUT/libpdfium.dylib" 2>/dev/null || true

echo "Installed $OUT/libpdfium.dylib"
echo "Run the app from the repo root so vendor/pdfium is discovered, or set:"
echo "  export DOXO_PDFIUM_DIR=\"$OUT\""
