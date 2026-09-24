# Uninstalling NovaShell

NovaShell only touches the files it installs. Removing it is clean and leaves
your Ubuntu session exactly as it was (GNOME still works, obviously).

```bash
./scripts/uninstall.sh
```

It disables and stops the user service, then removes:

- `novashell` binary (`/usr/local/bin` and/or `~/.local/bin`)
- `novashell.desktop` app + autostart entries
- `org.novashell.svg` icon
- `novashell.service` systemd unit

**Your data is kept by default** (nothing user-generated is deleted):
- `~/.config/novashell/config.json`
- `~/.local/share/novashell/library.json` (favorites, playtime, last played)
- `~/.local/share/novashell/screenshots/`
- `~/.local/share/novashell/logs/`

To also delete that data permanently:

```bash
./scripts/uninstall.sh --purge-data
```

## Manual removal (if you cloned into a working tree)

```bash
systemctl --user disable --now novashell 2>/dev/null; true
rm -f  /usr/local/bin/novashell ~/.local/bin/novashell
rm -f  /usr/local/share/applications/novashell.desktop ~/.local/share/applications/novashell.desktop
rm -f  /etc/xdg/autostart/novashell.desktop ~/.config/autostart/novashell.desktop
rm -f  /etc/systemd/user/novashell.service ~/.config/systemd/user/novashell.service
rm -rf ~/.config/novashell ~/.local/share/novashell   # optional: drop data too
```

Nothing NovaShell does requires reversing any system-level change — because it
never made any.