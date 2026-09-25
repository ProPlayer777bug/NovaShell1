#!/usr/bin/env bash
# Remove NovaShell.  Only touches files NovaShell itself installed.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> Uninstalling NovaShell"

systemctl --user disable novashell >/dev/null 2>&1 || true
systemctl --user stop novashell >/dev/null 2>&1 || true

# The system service installed by scripts/install-service.sh lives here, and it
# runs on boot. The old script only removed /etc/systemd/user/..., so the shell
# kept restarting after an "uninstall".
if [ "$(id -u)" -eq 0 ]; then
  if systemctl list-unit-files novashell.service >/dev/null 2>&1; then
    systemctl disable --now novashell.service >/dev/null 2>&1 || true
  fi
  for f in /etc/systemd/system/novashell.service /etc/tmpfiles.d/novashell.conf; do
    if [ -e "$f" ]; then
      rm -f "$f"
      echo "    removed $f"
    fi
  done
  systemctl daemon-reload >/dev/null 2>&1 || true
else
  echo "    note: run as root (or with sudo) to remove the system service"
fi

for scope in user system; do
  case "$scope" in
    user)
      BIN="$HOME/.local/bin/novashell"
      APP="$HOME/.local/share/applications/novashell.desktop"
      ICON="$HOME/.local/share/icons/hicolor/scalable/apps/org.novashell.svg"
      AUTOSTART="$HOME/.config/autostart/novashell.desktop"
      UNIT="$HOME/.config/systemd/user/novashell.service"
      ;;
    system)
      BIN=/usr/local/bin/novashell
      APP=/usr/local/share/applications/novashell.desktop
      ICON=/usr/local/share/icons/hicolor/scalable/apps/org.novashell.svg
      AUTOSTART=/etc/xdg/autostart/novashell.desktop
      UNIT=/etc/systemd/user/novashell.service
      ;;
  esac
  for f in "$BIN" "$APP" "$ICON" "$AUTOSTART" "$UNIT"; do
    if [ -e "$f" ]; then
      rm -f "$f"
      echo "    removed $f"
    fi
  done
done

echo
echo "==> Done."
echo "    Your games, library and settings are KEPT at:"
echo "        ~/.config/novashell/config.json"
echo "        ~/.local/share/novashell/library.json"
echo "        ~/.local/share/novashell/screenshots/"
echo "    To remove those too (irreversible), pass --purge-data:"
echo "        ./scripts/uninstall.sh --purge-data"

if [ "${1:-}" = "--purge-data" ]; then
  rm -rf "$HOME/.config/novashell" "$HOME/.local/share/novashell"
  echo "    removed configuration and library data"
fi