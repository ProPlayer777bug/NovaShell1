# Known limitations

Honest notes on behavior rather than masking anything.

## Playtime accuracy (Steam)

Playtime is measured from launching the `steam -applaunch <appid>` child until
that process exits. Steam may exit while the game is still running or stay
resident after the game closes — in both cases the number is approximate.
Heroic/legendary and native programs are measured as their own processes, so
they're exact.

## Screenshots on Wayland

Native Wayland has no global compositor screenshot API for a client to call;
NovaShell shells out to **grim**. On a vanilla Ubuntu 24.04 Wayland session,
install `grim` for screenshots (`sudo apt install grim`). On X11,
`gnome-screenshot`/`scrot`/`import` are used automatically.

## Volume from the console session

Volume control requires PipeWire + WirePlumber (`wpctl`). On systems using
PulseAudio the slider reports an error in the log and stays inert. NovaShell
does not install or start audio stacks.

## Games in "specialized" Steam modes

Steam Deck / Big Picture-style compatibility layers are whatever Steam is
configured to use; NovaShell does not alter Proton or display mode settings.

## Controller remap is applied on session start

Remapping in `config.json` takes effect when NovaShell starts. The Quick Menu
toggles for controller enabled/vibration are runtime toggles of UI input
enabling only — Rust keeps reading the pad and the mapping is loaded next
session.

## Not a display manager

NovaShell is a **session app**, not a login/display manager. It does not
provide a "boot straight into NovaShell" experience and never will — that
would require modifying boot/login configuration, which is explicitly out of
scope (and easily recoverable, but still — a line NovaShell doesn't cross).

## Electron-less memory / GPU

The UI rides on WebKitGTK, which is a full browser engine. Expect several
hundred MB RAM with GPU acceleration on. `performance_mode` disables
animations/filters to help weaker iGPUs.

## One UI file, one major thread

All Rust → JS calls funnel through a single main-context dispatcher; heavy
scans happen on Rust worker threads and publish results as events, so the UI
stays responsive.

## Flatpak-launched Heroic

Heroic installed via Flatpak is detected from its exported config path;
launching goes through `legendary`/`heroic` on PATH. If both are absent, the
title is marked not-launchable.

---
Anything listed here is a candidate to be improved in a future release —
check the issues/roadmap notes in the repository.