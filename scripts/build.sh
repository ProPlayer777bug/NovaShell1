#!/usr/bin/env bash
# Build NovaShell for the current machine.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> NovaShell build"
echo "    checking Rust toolchain"
command -v cargo >/dev/null 2>&1 || {
  echo "cargo not found. Install Rust stable first:"
  echo "    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  exit 1
}
RUST_RELEASE=$(rustc --version | awk '{print $2}')
echo "    rustc $RUST_RELEASE"
if ! grep -Eq '^1\.(8[5-9]|9[0-9])($|\.)' <<<"$RUST_RELEASE"; then
  echo "ERROR: NovaShell requires rustc >= 1.85 (the webkit6 crate uses edition 2024)."
  echo "Update with:  rustup update stable"
  exit 1
fi

echo "    checking GTK / WebKit / libudev headers"
for pkg in gtk4 javascriptcoregtk-6.0 webkitgtk-6.0; do
  if pkg-config --exists "$pkg" 2>/dev/null; then
    echo "    found: $pkg $(pkg-config --modversion "$pkg" 2>/dev/null)"
  fi
done
if ! pkg-config --exists gtk4 2>/dev/null || ! pkg-config --exists javascriptcoregtk-6.0 2>/dev/null; then
  echo "ERROR: missing dev headers. On Ubuntu 24.04 run:"
  echo "    sudo apt update"
  echo "    sudo apt install libgtk-4-dev libwebkitgtk-6.0-dev libjavascriptcoregtk-6.0-dev libudev-dev pkg-config"
  exit 1
fi

echo "    compiling (release)"
cargo build --release
echo "    done: target/release/novashell"