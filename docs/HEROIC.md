# Heroic integration

NovaShell reads games installed through **Heroic Games Launcher** (Epic Games
Store and GOG) and launches them without opening the Heroic GUI.

## Discovery

Installations are indexed from Heroic's config — by default
`~/.config/heroic/legendaryConfig/legendary/installed.json`. Each entry whose
`is_dlc` is false becomes a `heroic-<app_name>` library card. Titles come from
the manifest when available; otherwise a humanized app-name fallback is used.

## Launching

On launch, NovaShell prefers the **`legendary`** command-line (no GUI, lower
latency) and falls back to `heroic -l`:

```
legendary launch <app_name>
heroic -l <app_name>          # fallback when legendary is absent
```

- `launchers.legendary_path` — force a specific legendary binary.
- `launchers.heroic_bin` — override the fallback Heroic binary (default
  `heroic`).

## Notes

- **Installed only.** NovaShell lists what Heroic reports as installed; it
  does not touch the store, downloads or updates — those stay in Heroic.
- **Downloads / updates** stay in Heroic — NovaShell is launch-only.
- If `heroic` literals appear (e.g. `/usr/bin/heroic`), set `heroic_bin`
  accordingly in config.
- Flatpak Heroic writes its config under
  `~/.var/app/com.heroicgameslauncher.hgl/config/...`; NovaShell checks that
  location too.

## Config

```json
{
  "launchers": {
    "heroic_enabled": true,
    "heroic_bin": null,
    "legendary_path": null
  }
}
```