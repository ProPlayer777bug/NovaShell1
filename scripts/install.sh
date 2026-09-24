#!/usr/bin/env bash
# Install NovaShell.
#
# Usage:
#   ./scripts/install.sh            # system-wide install (run as root / sudo)
#   ./scripts/install.sh --user     # per-user install (no root needed)
#
# Safe by design:
#   - never modifies GRUB, kernel params, boot entries or GNOME configuration
#   - only writes files to standard application / icon / autostart locations
#   - GNOME stays installed and reachable as the desktop fallback
set -euo pipefail
cd "$(dirname "$0")/.."

SCOPE=system
[ "${1:-}" = "--user" ] && SCOPE=user

command -v cargo >/dev/null 2>&1 || { echo "cargo not found; install Rust first (rustup.rs)."; exit 1; }

if [ "$SCOPE" = "user" ]; then
  BIN_DIR="${NOVA_BIN_DIR:-$HOME/.local/bin}"
  APPS_DIR="$HOME/.local/share/applications"
  ICONS_DIR="$HOME/.local/share/icons/hicolor/scalable/apps"
  AUTOSTART_DIR="$HOME/.config/autostart"
  SYSTEMD_DIR="$HOME/.config/systemd/user"
else
  if [ "$(id -u)" != "0" ]; then
    echo "System-wide install needs root. Run:  sudo ./scripts/install.sh"
    exit 1
  fi
  BIN_DIR="/usr/local/bin"
  APPS_DIR="/usr/local/share/applications"
  ICONS_DIR="/usr/local/share/icons/hicolor/scalable/apps"
  AUTOSTART_DIR="/etc/xdg/autostart"
  SYSTEMD_DIR="/etc/systemd/user"
fi

echo "==> Installing NovaShell ($SCOPE)"

./scripts/build.sh

echo "    preparing directories"
mkdir -p "$BIN_DIR" "$APPS_DIR" "$ICONS_DIR" "$AUTOSTART_DIR" "$SYSTEMD_DIR"

echo "    copying binary + data"
install -m 0755 target/release/novashell "$BIN_DIR/novashell"
install -m 0644 assets/org.novashell.svg "$ICONS_DIR/org.novashell.svg"
install -m 0644 data/novashell.desktop "$APPS_DIR/novashell.desktop"
sed "s|@BINARY@|$BIN_DIR/novashell|g" data/autostart/novashell.desktop > "$AUTOSTART_DIR/novashell.desktop"
sed "s|@BINARY@|$BIN_DIR/novashell|g" data/systemd/novashell.service > "$SYSTEMD_DIR/novashell.service"

IS_SYSTEMD_USER_DIR=false
if [[ "$SYSTEMD_DIR" == "$HOME/.config/systemd/user"* ]]; then IS_SYSTEMD_USER_DIR=true; fi
if command -v systemctl >/dev/null 2>&1; then
  if [ "$IS_SYSTEMD_USER_DIR" = "true" ]; then
    systemctl --user daemon-reload >/dev/null 2>&1 || true
  else
    systemctl --user daemon-reload >/dev/null 2>&1 || true
  fi
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache "$(dirname "$ICONS_DIR")" >/dev/null 2>&1 || true
fi

echo
echo "==> Installed."
echo "    Test once now:              $BIN_DIR/novashell --windowed"
echo "    Autostart next login:"
if [ "$SCOPE" = "user" ]; then
  echo "        systemctl --user enable novashell"
  echo "        systemctl --user start novashell"
else
  echo "        systemctl --user enable novashell   (as your user)"
  echo "        systemctl --user start novashell"
fi
echo "    Launch from GNOME:         Activities -> NovaShell"
echo "    Revert:                    ./scripts/uninstall.sh"