# Nova Runtime Manager — Architecture & Design

Status: Phase 2 (design). Implementation follows in `src/runtime/`.

## 1. Goal

One place that knows how to *run* things. Today the shell launches a program in
five different places (`Launch::to_spec`, `console_polish`, `play_game_file`,
`roms:*` routes, the apt installer). The Runtime Manager becomes the single
source of truth for turning a library entry into a spawned process, and grows to
cover Wine, Proton and future runtimes without touching the UI or the library.

## 2. Existing architecture this plugs into (from the Phase 1 audit)

| Concern | Existing home | How the runtime layer reuses it |
|---|---|---|
| Process spawn | `launcher::Spec`, `launcher::spawn` (`process_group(0)`) | Runtimes *build* a `Spec`; they never spawn directly |
| Launch lifecycle | `shell::launch_spec`, `RunningSession` | Process manager takes over supervision, keyed per app |
| Library | `games::Game`, `Library` (`library.json`) | New `source: "windows-app" / "windows-game"` entries |
| Emulators | `apps::EMULATORS` table | Wrapped by `EmulatorRuntime`; table stays the single source |
| ROM memory | `roms::RomLibrary` (`roms.json`) | Untouched |
| Bridge | `shell::route` + `ui/index.html` `handleEvent` | New commands/events, same pattern |
| Config | `settings::Config` (`config.json`) | New `runtime` section |

## 3. Module layout

```
src/runtime/
  mod.rs        Runtime trait, RuntimeKind, RuntimeInfo, registry, selection
  native.rs     NativeRuntime  — wraps Launch::Program
  emulator.rs   EmulatorRuntime— wraps apps::EMULATORS (PS1/2/3, Wii, …)
  wine.rs       WineRuntime     — Phase 4
  proton.rs     ProtonRuntime   — Phase 5
  prefixes.rs   PrefixStore     — per-app prefixes, multiple roots
  procs.rs      ProcessManager  — keyed sessions, stop/kill/restart
  apps.rs       WindowsApp      — metadata, installer detection
  security.rs   validation      — paths, ids, env, root refusal
  logs.rs       per-app structured logs
```

## 4. Core types

```rust
pub enum RuntimeKind { Native, Emulator, WindowsApp, WindowsGame, Plugin }

pub struct RuntimeInfo {
    pub id: String,            // "wine", "proton", "ps2"
    pub name: String,          // "Wine", "Proton", "PlayStation 2"
    pub kind: RuntimeKind,
    pub version: Option<String>,
    pub executable: Option<PathBuf>,
    pub capabilities: Vec<Capability>,   // Launch, Install, Prefixes, Vulkan…
    pub supported_file_types: Vec<String>,
    pub supported_platforms: Vec<String>,
    pub status: RuntimeStatus,            // Available | Missing | Error(String)
    pub config: BTreeMap<String, String>,
}

pub trait Runtime {
    fn info(&self) -> RuntimeInfo;
    fn detect(&mut self) -> RuntimeStatus;
    fn validate(&self, target: &LaunchTarget) -> Vec<Check>;   // pre-flight
    fn build_spec(&self, target: &LaunchTarget) -> Result<Spec>;
}
```

`LaunchTarget` describes *what* to run (library id, executable, prefix, args,
env, cwd) so every runtime answers the same question.

## 5. Design decisions (defaults I chose — say the word to change)

1. **Prefix roots**: `~/.local/share/novashell/prefixes/{wine,proton}/` by
   default (reuses `util::data_dir()`, so it follows XDG and the existing
   `NOVASHELL_DATA_DIR` override), plus a user-configurable list of extra roots
   (e.g. `/mnt/games/nova`). *Chosen over a literal `~/.nova/` because the shell
   already standardises on `util::data_dir()` and changing that would split the
   library.*
2. **Process model**: `AppState.running` becomes a keyed map so a Wine game, an
   emulator and the file manager can coexist. The existing single-session code
   already assumed one at a time; the map is a superset, so nothing regresses.
3. **Wine/Proton**: detection-only in Phase 3. Phase 4/5 add install support; I
   will not install Wine/Proton on the machine without your go-ahead.
4. **Existing emulators are wrapped, never replaced.** `EMULATORS` stays the
   catalogue; `EmulatorRuntime` adapts it.

## 6. Security model

- Downloads are untrusted: nothing executes without an explicit UI confirmation.
- Refuse to run Wine/Proton/Windows apps as root.
- All paths validated: no `..` traversal, no NUL, must resolve under an allowed
  root, and must exist (or be creatable) before spawn.
- Process spawn always uses argument arrays (`Command`), never a shell string.
- Host secrets are not forwarded into prefixes; only an explicit allowlist of
  env vars (`DXVK_*`, `VKD3D_*`, `MESA_*`, `WINEDEBUG`, `WINEPREFIX`, …).
- Prefix deletion requires confirmation and is restricted to known roots.

## 7. Bridge additions (Phase 3)

Commands: `runtimes:list`, `runtimes:detect`, `runtimes:validate`,
`prefixes:list`, `prefixes:create`, `prefixes:delete`, `prefixes:repair`,
`procs:list`, `procs:stop`, `procs:kill`.
Events: `runtime.detected`, `runtime.status`, `prefix.created`, `prefix.deleted`,
`process.started`, `process.exited`.

## 8. Extending

Adding a runtime = implement `Runtime`, register it in `runtime::registry()`.
No core changes. Future targets named in the brief (Bottles, Lutris, Heroic,
DOSBox) are each one adapter.
