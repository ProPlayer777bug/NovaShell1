#!/usr/bin/env bash
# Remove NovaShell.  Only touches files NovaShell itself installed.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> Uninstalling NovaShell"

systemctl --user disable novashell >/dev/null 2>&1 || true
systemctl --user stop novashell >/dev/null 2>&1 || true

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