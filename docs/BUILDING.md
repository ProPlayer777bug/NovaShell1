# Building NovaShell

Target: **Ubuntu 24.04+** (64-bit). Rust **≥ 1.85** (the `webkit6` crate uses
edition 2024).

The project can be developed on any box for the *logic* parts (config,
library, launchers, integrations — all unit-tested with plain `cargo test`).
The GUI shell requires Linux + GTK4/WebKitGTK headers, so final builds happen
on Ubuntu.

## Dependencies

```bash
sudo apt update
sudo apt install curl build-essential pkg-config \
                 libgtk-4-dev libwebkitgtk-6.0-dev libjavascriptcoregtk-6.0-dev \
                 libudev-dev
```

| Package | Why |
| --- | --- |
| `libgtk-4-dev` | GTK4 windowing |
| `libwebkitgtk-6.0-dev` | WebKitGTK 6 (WebView; provides `webkitgtk-6.0`) |
| `libjavascriptcoregtk-6.0-dev` | JS engine + `javascriptcore6` crate |
| `libudev-dev` | `gilrs` gamepad input backend (evdev) |
| `pkg-config` | locating the above |

Toolchain via rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup update stable

# check
rustc --version   # 1.85.0 or newer
```

Runtime extras the shell *uses if present* (each is optional):
`grim` (Wayland screenshots), `gnome-screenshot` / `scrot` / ImageMagick
`import` (X11 screenshots), `wpctl` (volume, part of `wireplumber`),
`brightnessctl`, `nmcli` (NetworkManager), `bluetoothctl`.

## Build

```bash
./scripts/build.sh        # checks toolchain + headers, then cargo build --release
# or manually:
cargo build --release
```

Binary: `target/release/novashell`.

## Fast iteration

- `./scripts/run-dev.sh` — windowed dev run.
- `./scripts/run-debug.sh` — windowed + verbose logs (also enables the WebKit
  developer extras / inspector).
- `NOVASHELL_UI=/path/to/index.html ./scripts/run-dev.sh` — load a custom UI
  file without recompiling.
- `./scripts/test.sh` — JS syntax check (node) + `cargo test`.

## Release profile

`Cargo.toml` ships with `opt-level = 3`, `lto`, `codegen-units = 1` and `strip`
for a lean binary. The watchdog keeps the experience safe even if the
GTK/WebKit process crashes.