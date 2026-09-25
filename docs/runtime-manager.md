# Nova Runtime Manager — Architecture & Design

Status: **implemented** (`src/runtime/`) and wired into the shell bridge and the
Runtime Manager screen. Phases 1–10 are complete; this document records the
design and the commands the UI actually uses.

## 1. Goal

One place that knows how to *run* things. The shell launches a program in
several places (`Launch::to_spec`, `console_polish`, `play_game_file`,
`roms:*` routes, the apt installer). The Runtime Manager is the single
source of truth for turning a library entry into a spawned process, and covers
Wine, Proton and PS3 packages without the UI or the library knowing about them.

## 1a. What the user sees

- **Runtimes tab**: detected runtimes and their status, per-app Wine/Proton
  prefixes with Repair/Delete, live sessions with Stop/Force, per-runtime logs,
  Windows files and PS3 packages waiting for confirmation, and a preflight that
  explains *why* a PS3 package cannot be installed (missing firmware/licence).
- **Taskbar**: the shell plus every app, emulator, Wine/Proton session and
  helper window the shell opened, with focus and close per entry. Entries are
  reconciled against `/proc`, so a closed app disappears on its own.

## 1b. Bridge commands

| Command | Purpose |
|---|---|
| `runtimes:list` / `runtimes:detect` | runtime inventory (`detect` also emits `runtime.detected`) |
| `runtimes:validate` | pre-flight checks for one runtime + target |
| `prefixes:list` / `:create` / `:delete` / `:repair` | per-app prefix management |
| `procs:list` / `:stop` / `:kill` | session control (signals the process **group**) |
| `windows:list` / `:focus` / `:close` | taskbar contents and window control |
| `windows:apps` / `:inspect` / `:app:add` / `:app:remove` / `:launch` / `:install` | Windows app catalogue and Wine/Proton launches |
| `windows:pending` / `:decide` / `:associations` | confirmation-gated `.exe` handling |
| `ps3:installables` / `:pending` / `:install` / `:dismiss` | PS3 packages via RPCS3 |
| `runtime:logs` | per-app launch log tail |

Events emitted to the UI: `runtime.detected`, `prefix.created`, `prefix.deleted`,
`application.started`, `application.exited`, `application.installation.*`,
`windows.file_decided`, `ps3.installed`, `ps3.install_failed`, `game_start`,
`game_exit`, `library_changed`.

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
  wine.rs       WineRuntime     — per-app prefixes, wineboot, ownership checks
  proton.rs     ProtonRuntime   — multi-build discovery, per-game compat data
  prefixes.rs   PrefixStore     — per-app prefixes, multiple roots
  procs.rs      ProcessManager  — keyed sessions, group stop/kill, liveness
  apps.rs       WindowsApp      — metadata, installer detection, file handlers
  decisions.rs  per-file choices for .exe/.ps3 handling
  ps3.rs        RPCS3 packages  — discovery, install, licence placement
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
