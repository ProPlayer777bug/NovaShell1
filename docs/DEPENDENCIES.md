# Dependencies

Build-time and runtime requirements for NovaShell on Ubuntu 24.04.

## Build (crates)

| Crate | Version | Purpose |
| --- | --- | --- |
| `gtk4` | 0.11 | GTK4 UI toolkit |
| `webkit6` | 0.6 | WebKitGTK 6 — renders the offline UI |
| `gilrs` | 0.11 | Gamepad input (evdev on Linux) |
| `serde` / `serde_json` | 1 | Config + bridge serialization |
| `dirs` | 5 | XDG directories |
| `log` | 0.4 | Logging abstractions |
| `chrono` | 0.4 | Timestamps / log file naming |
| `anyhow` | 1 | Error handling |
| `tempfile` | 3 | (dev) test isolation |

## Build (system packages, Ubuntu 24.04)

```bash
sudo apt install curl build-essential pkg-config \
                 libgtk-4-dev libwebkitgtk-6.0-dev libjavascriptcoregtk-6.0-dev \
                 libudev-dev
```

Rust toolchain: **rustc ≥ 1.85** (edition 2024 used by `webkit6`).

## Runtime (each optional when present)

| Tool | Used for | When needed |
| --- | --- | --- |
| `wpctl` (wireplumber) | Volume control | desktop with PipeWire/WirePlumber |
| `brightnessctl` | Backlight control | laptops; sysfs fallback exists |
| `grim` | Wayland screenshots | `screenshot_mode: auto` on Wayland |
| `gnome-screenshot` | X11 screenshots | fallback |
| `scrot`, `import` | X11 screenshots | fallback |
| `nmcli` | Wi-Fi SSID (status bar) | NetworkManager |
| `bluetoothctl` | Bluetooth device list | BlueZ |
| `systemctl` | Power actions | logind/elogind |
| `gnome-session-quit` | Log out action | GNOME session |

If a tool is absent the related UI just degrades gracefully (log entries note
the misses).

## Not modified

NovaShell never depends on nor modifies: GRUB, kernel parameters/initramfs,
`/etc/fstab`, display managers, GNOME configurations, or the Steam/Heroic
install trees. Removing NovaShell (see `UNINSTALL.md`) needs no other
reversal.