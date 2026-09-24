# Troubleshooting

First stop: **logs** — everything NovaShell does is logged.

```bash
tail -n 200 ~/.local/share/novashell/logs/nova-$(date +%Y%m%d).log
```

## Scale, resolution and 4K

The whole UI is **rem-based** and scales with the `ui_scale` setting
(Settings → Appearance). If it looks small on a 4K display, raise UI scale to
~2.0. If you need it finer-grained, edit `config.json` (`ui_scale`) and
restart.

## Screenshots fail

Screenshot mode defaults to `auto`:

- **Wayland**: install `grim` (`sudo apt install grim`).
- **X11**: any of `gnome-screenshot`, `scrot`, or ImageMagick `import`.

Or force one in `config.json`: `"screenshot_mode": "grim"` (also supported:
`gnome-screenshot`, `scrot`, `import`).

## No games appear

1. Check Settings → toggle each launcher.
2. **Refresh library** (Settings → *Refresh library*).
3. Steam: is Steam installed/working outside NovaShell? Games not installed
   are pruned.
4. Heroic: are games in Heroic's installed list?
5. Tail the log; the scan lines (`provider ... returned N games`) show counts.

## Controller not responding

- Check the status bar: it shows *Joypads: N*.
- Try `evtest` / `jstest` — if the kernel sees the pad, NovaShell does too.
- Verify you're not in *adjust* mode on a slider (confirm again to exit).
- Wireless pads must be connected in the OS (settings/bt) first.

## Game launches but NovaShell thinks it's still running (or overlay stays)

Playtime for Steam titles measures the `steam` process, not the game's own
process. When Steam exits the title, `-applaunch` returns. If Steam stays
resident (it can), the overlay ends when Steam relaunches/emits; a further
`game:launch` is always safe. Heroic titles are measured by `legendary`
directly.

Logs will show `... closed after Ns`.

## UI is blank / dev tools

`./scripts/run-debug.sh` enables WebKit developer extras (right-click →
Inspect, or `Ctrl+Shift+I`). Loading a custom UI file works with
`NOVASHELL_UI=/path/to/index.html` (forces windowed mode). The UI script can be
syntax-checked anywhere with `node --check` (see `scripts/test.sh`).

## It won't compile

Usually a toolchain/header mismatch — see [BUILDING.md](BUILDING.md). Common:
older rustc than 1.85 (webkit6 needs edition 2024), or missing
`libwebkitgtk-6.0-dev` (note the `-6.0`, not the GTK3-era
`libwebkit2gtk-4.1-dev`).

## NovaShell won't start (GTK/display)

Ensure you launch it in a graphical session (a Wayland/X11 login), not over a
bare SSH. If a session compositor isn't reachable GTK fails fast and exits
code 2 — see [RECOVERY.md](RECOVERY.md).

## Audio/brightness sliders do nothing

- Volume uses `wpctl` (WirePlumber). Install/setup WirePlumber
  (`sudo apt install wireplumber`); on some systems `pulseaudio` is used
  instead — the slider then silently no-ops (log shows the error).
- Brightness uses `brightnessctl` first, then sysfs. On laptops without a
  backlight device the row is inert.