#!/usr/bin/env sh
# Interflect installer for macOS and Linux.
#
#     curl -fsSL https://raw.githubusercontent.com/Cherie05/interflect/main/install.sh | sh
#
# Downloads a prebuilt binary from GitHub Releases. No Rust toolchain needed.
# Deliberately short so you can read the whole thing before piping it to a
# shell -- and if you would rather not, download from the Releases page instead:
#     https://github.com/Cherie05/interflect/releases
#
# Options (environment variables):
#     INTERFLECT_VERSION=0.1.0    install a specific version (default: latest)
#     INTERFLECT_BIN_DIR=~/bin    install location (default: ~/.local/bin)

set -eu

REPO="Cherie05/interflect"
BIN_DIR="${INTERFLECT_BIN_DIR:-$HOME/.local/bin}"

say()  { printf '%s\n' "$*"; }
err()  { printf 'error: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || err "this installer needs '$1'"; }

need uname
need mkdir
need tar

# --- pick a downloader -------------------------------------------------------
if command -v curl >/dev/null 2>&1; then
  fetch()      { curl -fsSL "$1"; }
  fetch_file() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
  fetch()      { wget -qO- "$1"; }
  fetch_file() { wget -qO "$2" "$1"; }
else
  err "this installer needs curl or wget"
fi

# --- detect the target triple ------------------------------------------------
os=$(uname -s)
arch=$(uname -m)

case "$os" in
  Darwin) case "$arch" in
            arm64|aarch64) target="aarch64-apple-darwin" ;;
            x86_64)        target="x86_64-apple-darwin" ;;
            *) err "unsupported macOS architecture: $arch" ;;
          esac ;;
  Linux)  case "$arch" in
            x86_64|amd64) target="x86_64-unknown-linux-gnu" ;;
            aarch64|arm64) err "no Linux arm64 build yet. Build from source:
  cargo install interflect" ;;
            *) err "unsupported Linux architecture: $arch" ;;
          esac ;;
  *) err "unsupported OS: $os. Windows users: use install.ps1" ;;
esac

# --- resolve the version -----------------------------------------------------
if [ -n "${INTERFLECT_VERSION:-}" ]; then
  version="$INTERFLECT_VERSION"
else
  say "Finding the latest release..."
  # Parse the tag out of the releases API without needing jq.
  version=$(fetch "https://api.github.com/repos/$REPO/releases/latest" \
            | sed -n 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/p' | head -1)
  [ -n "$version" ] || err "could not determine the latest version.
Download manually from https://github.com/$REPO/releases"
fi

name="interflect-${target}-v${version}"
url="https://github.com/$REPO/releases/download/v${version}/${name}.tar.gz"

say "Installing interflect v${version} (${target})"

# --- download and verify -----------------------------------------------------
tmp=$(mktemp -d 2>/dev/null || mktemp -d -t interflect)
trap 'rm -rf "$tmp"' EXIT INT TERM

fetch_file "$url" "$tmp/pkg.tar.gz" \
  || err "download failed: $url
That version or platform may not exist. See https://github.com/$REPO/releases"

# Checksum is best-effort: verify when a checker is available, warn when not,
# rather than refusing to install on a minimal system.
if fetch_file "$url.sha256" "$tmp/pkg.sha256" 2>/dev/null; then
  expected=$(awk '{print $1}' "$tmp/pkg.sha256")
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$tmp/pkg.tar.gz" | awk '{print $1}')
  elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$tmp/pkg.tar.gz" | awk '{print $1}')
  else
    actual=""
  fi
  if [ -n "$actual" ]; then
    [ "$actual" = "$expected" ] || err "checksum mismatch -- refusing to install.
  expected $expected
  got      $actual"
    say "Checksum verified."
  fi
fi

tar xzf "$tmp/pkg.tar.gz" -C "$tmp"

# --- install -----------------------------------------------------------------
mkdir -p "$BIN_DIR"
cp "$tmp/$name/interflect" "$BIN_DIR/interflect"
cp "$tmp/$name/compare"    "$BIN_DIR/interflect-compare"
chmod +x "$BIN_DIR/interflect" "$BIN_DIR/interflect-compare"

# Example scenes and the offline scene builder go somewhere findable; the
# binary is useless without a scene to render.
share="${XDG_DATA_HOME:-$HOME/.local/share}/interflect"
mkdir -p "$share"
cp -R "$tmp/$name/scenes" "$share/" 2>/dev/null || true
cp -R "$tmp/$name/tools"  "$share/" 2>/dev/null || true

say ""
say "Installed to $BIN_DIR/interflect"
say "Example scenes: $share/scenes"
say "Scene builder:  $share/tools/scene-builder.html"

case ":$PATH:" in
  *":$BIN_DIR:"*)
    say ""
    say "Try it:"
    say "  interflect render $share/scenes/product.rad -o out.png"
    ;;
  *)
    say ""
    say "$BIN_DIR is not on your PATH. Add this to your shell profile:"
    say "  export PATH=\"\$PATH:$BIN_DIR\""
    say ""
    say "Then:  interflect render $share/scenes/product.rad -o out.png"
    ;;
esac
