# Architecture

NovaShell is a single Rust binary (named `novashell`) that opens one GTK4
`ApplicationWindow`, hosts a `WebKitWebView`, and talks to a JavaScript UI that
fills the window. All game launching, input handling, settings, scans and
system calls live in Rust; all rendering and interaction live in the UI.

```
                    ┌────────────────────────────────────────┐
                    │            NovaShell binary            │
                    │                                        │
  gamepad (evdev) ─▶│  controllers ──┐                      │
  keyboard / mouse ─▶│  (gilrs)      │                      │
                    │                ▼                      │
  Steam / Heroic /  │ │  shell (GTK4 + WebKitGTK6)          │
  desktop FreeDesktop files ─▶ plugins → library ──────▶ WebKitWebView
  system files (/proc, sysfs, wpctl…) ─▶ system ──────────▶  |
                    │                ▲                      │
                    │                └── bridge (JSON) ─────┘
                    └────────────────────────────────────────┘
                                 JS UI (ui/index.html)
```

## Modules (`src/`)

| Module | Role | Platform |
| --- | --- | --- |
| `shell` | Application, window, WebView, message router, game-launch supervision, watchdog, logging | Linux |
| `ui` | `evaluate_javascript` + script-message decoding helpers | Linux |
| `controllers` | gilrs gamepad thread → abstract `Action`s | Linux |
| `system` | CPU/RAM/disk/battery/network, volume, brightness, power, screenshots | Linux |
| `games` | `Game` model, `Library` merge/prune, favorites, playtime, persistence | all |
| `integrations` | `Provider` trait + Steam / Heroic / desktop providers | all |
| `plugins` | provider registry and safe scan orchestration | all |
| `launcher` | `Spec` spawn (own process group), `.desktop` Exec parsing, `run_capture` | all |
| `settings` | `Config` model + JSON persistence | all |
| `util` | XDG paths with `NOVASHELL_CONFIG_DIR` / `NOVASHELL_DATA_DIR` overrides | all |

Logic modules have no GTK/webkit dependency, so `cargo test` runs them on any
platform (including on a headless dev machine).

## The bridge (the only UI⇄core channel)

- **JS → Rust.** The UI calls `window.webkit.messageHandlers.novashell`.
  `postMessage(JSON.stringify({cmd, id, ...data}))`. Every command may expect a
  reply `{reply: true, id, data}` which the UI routes to a promise.
- **Rust → JS.** `ui::dispatch()` evaluates
  `window.novaShellDispatch(JSON)` on the main context. Payloads are either
  `{reply, id, data}` responses or `{event, ...}` announcements (status ticks,
  controller presses, game start/exit, settings changes).

Controller presses and game-exit notifications cross threads through a
`glib::MainContext::channel`, so all WebView calls happen on the main thread.

## Launch lifecycle

1. UI sends `game:launch {id}`.
2. Rust resolves the `Launch` spec (`Program` / Steam `-applaunch <id>` /
   Heroic `legendary launch <name>`), spawns it with `process_group(0)` so a
   NovaShell exit can never take the game down.
3. `_internal` bookkeeping records `game_start` (hides the window in
   fullscreen mode), a watcher thread waits for exit, then
   `game_exit` (playtime added, window restored) is dispatched back.
4. Everything (favorites, playtime, last-played) persists to `library.json`.

## Status flow

A 4-second timer on the main context samples `/proc/stat`, `/proc/meminfo`,
`df`, sysfs battery, network state, and the connected gamepad list, then pushes
`{event:"status"}` to the UI status bar. No UI polling is needed.

## Safety properties

- **No bootloader/kernel/GRUB/system-service writes.** Power actions only call
  `systemctl suspend/reboot/poweroff` and `gnome-session-quit`.
- **GNOME untouched.** NovaShell runs *over* the existing session; unlinking it
  leaves your machine exactly as it was.
- **Watchdog.** In systemd/autostart mode, `newashell` is supervised by
  `--watchdog` (max 3 restarts) and `Restart=on-failure` as a second net.
- **Crash surface.** A UI crash takes down only the WebView process group;
  games are outside it.

## Config

`~/.config/novashell/config.json` — see `src/settings/mod.rs` for the full
schema (theme, accent, ui_scale, animations, performance mode, fullscreen,
launcher toggles, controller mapping, screenshot mode). Missing fields default
safely.