# Recovery

NovaShell is deliberately reversible. Everything below assumes the worst-case
flow and shows you how to get back to a stock Ubuntu session.

## If NovaShell is open and you want out

- **Quick menu** (`L2/R2`, `Q`) → **Return to desktop** — cleanly exits to
  your existing GNOME session.
- `Esc` on Home then `Q` also gives you the same menu.
- `Alt+F4` / `killall novashell` also works (games are in their own process
  groups and survive).

## If it crashes (black screen but system works)

The systemd user service (`Restart=on-failure`) plus the internal watchdog
restart it. If it truly won't come back:

```bash
systemctl --user stop novashell
systemctl --user disable novashell
killall novashell 2>/dev/null; true
# three seconds later GNOME is all you have: done, nothing to restore.
```

## If you can't see any desktop at all

NovaShell never touched your session, so:

- `Ctrl+Alt+F2` (or `F3`/`F4`) → TTY login →
  `systemctl --user stop novashell; systemctl --user disable novashell`
- `Ctrl+Alt+F1` back to your GNOME session.

Or from another machine/SSH login, run the same two commands.

## If a game left your screen gridlocked

Games run under their own process groups; quitting NovaShell never kills them.
When it's the game that froze, quit via its own means or
`ps aux | grep <game>` + `kill`.

## Full nuclear reset

```bash
./scripts/uninstall.sh --purge-data   # removes NovaShell + its data
```

The system is byte-identical to a non-NovaShell Ubuntu except for deleted
files. No bootloader, fstab, GRUB, kernel, or service changes were ever made.

## Crash marker

On a panic, NovaShell writes `~/.local/share/novashell/crash.marker` and
logs the reason. Include both when reporting an issue:

```
cat ~/.local/share/novashell/crash.marker
```