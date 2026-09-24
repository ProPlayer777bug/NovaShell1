# Installing NovaShell

Prereqs: see [BUILDING.md](BUILDING.md) — you need Rust ≥ 1.85 and the GTK/
WebKit dev headers to compile once during install.

## Option A — system-wide (root)

```bash
./scripts/build.sh                     # optional: compile explicitly
sudo ./scripts/install.sh
```

Installs to:

| File | Location |
| --- | --- |
| Binary | `/usr/local/bin/novashell` |
| App entry | `/usr/local/share/applications/novashell.desktop` |
| Icon | `/usr/local/share/icons/hicolor/scalable/apps/org.novashell.svg` |
| Autostart | `/etc/xdg/autostart/novashell.desktop` |
| systemd unit | `/etc/systemd/user/novashell.service` |

## Option B — per-user (no root)

```bash
./scripts/install.sh --user
```

Uses `~/.local/bin`, `~/.local/share/applications`,
`~/.local/share/icons/...`, `~/.config/autostart/`,
`~/.config/systemd/user/novashell.service`.

## Start it

```bash
# right now (windowed, safe)
novashell --windowed

# via systemd user service (recommended for living room setups)
systemctl --user enable novashell
systemctl --user start novashell

# or simply log in — the autostart entry launches it
```

## Autostart only vs. always-on

- **Autostart entry (default)** — NovaShell opens at login over GNOME. Press
  **Return to desktop** (Quick Menu) or `Esc` on Home to drop back to GNOME.
- **systemd service** — same window, but it also restarts NovaShell if it
  crashes (see [RECOVERY.md](RECOVERY.md)).

> Safety: GNOME and the login manager are never disabled. If you ever want the
> "pure console" boot, you can simply not start it — no changes were made to
> the system. NovaShell is a session app, not a display manager.

## Verifying

```bash
systemctl --user status novashell        # active/running
novashell --help                         # prints options
```

Logs land in `~/.local/share/novashell/logs/`.