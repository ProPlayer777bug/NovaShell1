# Controller support

NovaShell reads any gamepad that Linux sees via **evdev** (through the `gilrs`
Rust crate): Xbox, DualShock/DualSense-compatible pads in Switch/XInput/SDL
mode, Steam Controller, 8BitDo, etc. Hotplug works — plug a pad in at any time
and it appears in the status bar immediately.

> The gamepad support is purely additive: no kernel modules, no custom
> udev rules, nothing system-wide. If NovaShell is removed, pads behave
> exactly as before.

## Mapping

Buttons use **logical** (SDL/gilrs) names, which makes layouts consistent
across pad brands:

| Log. button | Default action |
| --- | --- |
| `south` (A / ✕) | `confirm` |
| `east` (B / ◯) | `back` |
| `west` (X / □) | `context` |
| `north` (Y / △) | `detail` |
| `start` (Menu) | `menu` (opens search) |
| `select` (View) | `context` |
| `guide` (Home) | `screenshot` |
| `l1` / `r1` (LB/RB) | `tab_prev` / `tab_next` |
| `l2` / `r2` (LT/RT) | `quick_menu` |
| d-pad + left stick | directional focus |

Actions recognized: `up`, `down`, `left`, `right`, `confirm`, `back`, `menu`,
`context`, `detail`, `tab_prev`, `tab_next`, `quick_menu`, `screenshot`.

## Remapping

Edit `~/.config/novashell/config.json` under `controller.mapping`. For
example, to swap confirm onto the east button and use LB/LT for tabs and the
quick menu:

```json
{
  "controller": {
    "enabled": true,
    "vibration": true,
    "mapping": {
      "south": "back",
      "east": "confirm",
      "l2": "tab_prev",
      "r2": "tab_next",
      "l1": "quick_menu",
      "r1": "quick_menu"
    }
  }
}
```

Settings → Experience offers toggles for `enabled` and `vibration`
(reapplied on the next start; the running session uses the loaded snapshot).

## Navigation rules

- **D-pad and left stick** move the focus ring (left/right within a row,
  up/down between rows).
- **Confirm** activates the focused tile. On a **slider** (volume, brightness,
  UI scale) confirm enters *adjust* mode, left/right change the value, confirm
  or back exits.
- **Back** closes overlays first, then drops a tab, then (on Home) is a no-op.
- **Quick menu** (LT/RT, `Q`) is a slide-in control center with volume,
  brightness, network, Bluetooth, performance mode, screenshot, *Return to
  desktop*, and power actions.
- **Screenshot** (Share, `F2`) grabs the screen via the best available tool.

## Notes

- Battery report for pads appears in the status bar when the pad exposes it.
- On some cheap dongles a pad may appear as several `/dev/input` nodes — gilrs
  merges them; if you see double handling, use a quality adapter with a clean
  single-report mode.
- Controller input on **Wayland** and **X11** works identically.