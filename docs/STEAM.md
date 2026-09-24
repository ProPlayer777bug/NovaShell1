# Steam integration

NovaShell discovers your **installed** Steam games and launches them through
the Steam client in its native way.

## Discovery

1. Steam's root is found in order: the `launchers.steam_path` config value (if
   set), then the standard install paths and the **Flatpak** install
   (`~/.var/app/com.valvesoftware.Steam`).
2. Each `steamapps` library folder's `libraryfolders.vdf` is parsed (Steam can
   spread games across multiple drives).
3. Game installs are read from `steamapps/appmanifest_<appid>.acf`. Installed
   games (`StateFlags` bit 1) become `steam-<appid>` entries in your library.
4. Box-art (if present) is taken from `library_capsule.jpg` in the game's
   install folder.

## Launching

Every Steam game launches as `steam -applaunch <appid>`:

- Steam opens the game and **closes automatically** when the app exits (the
  `-applaunch` mode) — exactly what a launcher front-end wants.
- The first launch may take a few seconds (Steam startup + game process);
  NovaShell shows the **playing** overlay meanwhile.
- If Steam is already running, `-applaunch` just starts the title inside it.

## FAQ

**Steam isn't installed?** Steam games simply don't appear; the provider is
skipped. Install Steam (`.deb`, Steam launcher, or `flatpak install
flathub com.valvesoftware.Steam`) and refresh the library in Settings.

**I don't want Steam library entries.** Settings → Launchers → disable
*Steam*. Your favorites/playtime for those ids stay stored and reappear if you
re-enable.

**Non-Steam shortcuts** (added in Steam client) do not show up — NovaShell
indexes only real app manifests. Add them as **manual** entries if wanted.

**Proton / Steam Deck tools.** NovaShell uses whatever Steam is configured
with (Proton compatibility, Deck mode, etc.). It doesn't alter compatibility
settings.

## Config

```json
{
  "launchers": {
    "steam_enabled": true,
    "steam_path": null
  }
}
```

Set `steam_path` only if your install lives somewhere unusual.