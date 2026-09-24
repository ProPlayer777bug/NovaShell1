//! Curated system apps + emulator tiles.
//!
//! Surfacing a small, intentional set of non-game tools (browsers, a file
//! manager) and per-platform emulator entries. Emulator tiles carry the
//! console they target and the ROM extensions they can open, so the UI can
//! present a folder browser that ends in an emulator + ROM launch.

use crate::games::{Game, Launch};
use crate::integrations::desktop::find_icon;
use crate::integrations::Provider;
use crate::settings::Config;
use std::path::PathBuf;

/// Binaries the curated provider owns; the desktop-entry provider skips these
/// so they never appear twice.
pub const CURATED_BINS: &[&str] = &[
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
    "firefox",
    "firefox-esr",
    "brave-browser",
    "nautilus",
    "nemo",
    "thunar",
    "dolphin",
    "pcmanfm",
    "caja",
    "duckstation",
    "epsxe",
    "pcsx2",
    "pcsx2-qt",
    "rpcs3",
    "xemu",
    "xenia",
    "dolphin-emu",
    "retroarch",
    "mupen64plus",
    "melonDS",
    "citra",
    "yuzu",
    "cemu",
];

/// Resolve a binary name to a full path via PATH (or `None`).
pub fn find_bin(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(name);
        if cand.is_file() {
            return Some(cand.to_string_lossy().to_string());
        }
    }
    None
}

fn bin_ok(name: &str) -> Option<String> {
    find_bin(name)
}

/// Curated browsers, in display priority order. First matching candidate wins.
const BROWSERS: &[(&str, &str, &[&str])] = &[
    ("Google Chrome", "google-chrome", &["google-chrome-stable", "google-chrome"]),
    ("Chromium", "chromium", &["chromium-browser", "chromium"]),
    ("Firefox", "firefox", &["firefox", "firefox-esr"]),
    ("Brave", "brave-browser", &["brave-browser"]),
];

/// Curated file managers, in display priority order.
const FILE_MANAGERS: &[(&str, &str, &[&str])] = &[
    ("Files", "nautilus", &["nautilus"]),
    ("Nemo", "org.nemo.Nemo", &["nemo"]),
    ("Thunar", "Thunar", &["thunar"]),
    ("Dolphin", "dolphin", &["dolphin"]),
    ("PCManFM", "pcmanfm", &["pcmanfm"]),
];

/// The first file manager actually installed, for "open this folder" actions.
pub fn first_installed_file_manager() -> Option<String> {
    FILE_MANAGERS
        .iter()
        .find_map(|(_, bin, _)| find_bin(bin))
}

struct EmulatorDef {
    platform: &'static str,
    pretty: &'static str,
    icon: &'static str,
    bins: &'static [&'static str],
    exts: &'static [&'static str],
    /// Folder under the user's home where this console's games are kept, and
    /// the folder the ROM picker opens on the very first visit.
    rom_dir: &'static str,
}

/// Per-console emulator definitions. Only one tile per console (first
/// installed binary wins) but the `rom_exts` let the UI offer a ROM picker.
const EMULATORS: &[EmulatorDef] = &[
    EmulatorDef {
        platform: "PSX",
        pretty: "PlayStation 1 / PSX",
        icon: "duckstation",
        bins: &["duckstation", "epsxe"],
        exts: &["cue", "bin", "iso", "chd", "pbp", "img", "m3u"],
        rom_dir: "PSX",
    },
    EmulatorDef {
        platform: "PS2",
        pretty: "PlayStation 2",
        icon: "PCSX2",
        bins: &["pcsx2-qt", "pcsx2"],
        exts: &["iso", "chd", "cso", "bin", "img", "gz"],
        rom_dir: "PS2",
    },
    EmulatorDef {
        platform: "PS3",
        pretty: "PlayStation 3",
        icon: "rpcs3",
        bins: &["rpcs3"],
        exts: &["pkg", "iso", "ps3dir"],
        rom_dir: "PS3",
    },
    EmulatorDef {
        platform: "Xbox",
        pretty: "Xbox / Xbox 360",
        icon: "xemu",
        bins: &["xemu", "xenia"],
        exts: &["iso", "xiso", "xbe", "xex"],
        rom_dir: "Xbox",
    },
    EmulatorDef {
        platform: "Wii",
        pretty: "Wii / GameCube",
        icon: "dolphin-emu",
        bins: &["dolphin-emu"],
        exts: &["iso", "rvz", "gcm", "wbfs", "nkit", "wad", "dol"],
        rom_dir: "Wii",
    },
    EmulatorDef {
        platform: "Nintendo",
        pretty: "Nintendo (classic)",
        icon: "retroarch",
        bins: &["retroarch", "mupen64plus", "melonDS", "citra", "yuzu", "cemu"],
        exts: &[
            "nes", "fds", "snes", "sfc", "gb", "gbc", "gba", "nds", "n64", "z64",
            "3ds", "cia", "nsp", "xci",
        ],
        rom_dir: "Nintendo",
    },
];

fn curated_app(
    title: &str,
    source: &str,
    program: &str,
    icon_name: Option<&str>,
    platform: Option<&str>,
    rom_exts: Vec<String>,
    installed: bool,
    rom_dir: Option<&str>,
) -> Game {
    let key = PathBuf::from(program)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| program.to_string());
    Game {
        id: format!("{source}-{key}"),
        title: title.to_string(),
        source: source.to_string(),
        launch: Launch::Program {
            program: program.to_string(),
            args: Vec::new(),
        },
        icon: icon_name.and_then(find_icon),
        artwork: None,
        last_played: None,
        playtime_secs: 0,
        favorite: false,
        installed,
        platform: platform.map(String::from),
        rom_exts,
        rom_dir: rom_dir.map(String::from),
    }
}

fn resolve_bin(cands: &[&str]) -> Option<String> {
    cands.iter().find_map(|c| bin_ok(c))
}

/// Scan curated apps + emulators into library candidates.
///
/// The curated set is always listed so the home screen is never empty — tiles
/// whose binary is missing are marked `installed: false` and the UI grey-bars
/// them. The desktop provider still supplies a real entry when the binary is
/// installed via a PATH it can see but the curated lookup could not (e.g.
/// flatpak), because it only skips curated bins it knows are resolvable.
pub fn scan_curated_games(_cfg: &Config) -> Vec<Game> {
    let mut out: Vec<Game> = Vec::new();

    for (title, icon, cands) in BROWSERS {
        match resolve_bin(cands) {
            Some(bin) => out.push(curated_app(title, "app", &bin, Some(icon), None, Vec::new(), true, None)),
            None => out.push(curated_app(title, "app", cands[0], Some(icon), None, Vec::new(), false, None)),
        }
    }

    // Exactly one file-manager tile: the first installed one, else Files.
    let fm = FILE_MANAGERS
        .iter()
        .find(|(_, _, cands)| resolve_bin(cands).is_some())
        .unwrap_or(&FILE_MANAGERS[0]);
    match resolve_bin(fm.2) {
        Some(bin) => out.push(curated_app(fm.0, "app", &bin, Some(fm.1), None, Vec::new(), true, None)),
        None => out.push(curated_app(fm.0, "app", fm.2[0], Some(fm.1), None, Vec::new(), false, None)),
    }

    for def in EMULATORS {
        let exts = def.exts.iter().map(|e| e.to_string()).collect();
        match resolve_bin(def.bins) {
            Some(bin) => out.push(curated_app(
                def.pretty,
                "emulator",
                &bin,
                Some(def.icon),
                Some(def.platform),
                exts,
                true,
                Some(def.rom_dir),
            )),
            None => out.push(curated_app(
                def.pretty,
                "emulator",
                def.bins[0],
                Some(def.icon),
                Some(def.platform),
                exts,
                false,
                Some(def.rom_dir),
            )),
        }
    }
    out
}

pub struct AppsProvider;

impl Provider for AppsProvider {
    fn id(&self) -> &'static str {
        "apps"
    }
    fn scan(&self, cfg: &Config) -> Vec<Game> {
        scan_curated_games(cfg)
    }
}

/// `true` if `program` is one of the curated binaries, so the desktop provider
/// can skip it (single source of truth for these tiles).
pub fn is_curated_bin(program: &str) -> bool {
    let prog = program.trim();
    let name = PathBuf::from(prog)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    CURATED_BINS
        .iter()
        .any(|b| prog == *b || name == *b || prog.ends_with(&format!("/{b}")))
}

/// Ubuntu/Debian package names used to install a curated tile, keyed by its
/// binary name. An empty list means there is no first-class package (the app
/// needs a third-party repo), so the tile reports "no install path".
pub fn apt_packages_for_program(program: &str) -> Option<&'static [&'static str]> {
    let key = PathBuf::from(program)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let map: &[(&str, &[&str])] = &[
        // Ubuntu >= 23.10 ships firefox/chromium as snap-only transitional
        // packages that hang without a working snapd, so no apt install here.
        ("firefox", &[]),
        ("firefox-esr", &[]),
        ("chromium", &[]),
        ("chromium-browser", &[]),
        ("google-chrome", &[]),
        ("google-chrome-stable", &["google-chrome-stable"]),
        ("brave-browser", &["brave-browser"]),
        ("nautilus", &["nautilus"]),
        ("nemo", &["nemo"]),
        ("thunar", &["thunar"]),
        ("pcmanfm", &["pcmanfm"]),
        ("dolphin", &["dolphin"]),
        ("pcsx2", &["pcsx2"]),
        ("pcsx2-qt", &["pcsx2"]),
        ("duckstation", &[]),
        ("epsxe", &[]),
        ("rpcs3", &[]),
        ("xemu", &[]),
        ("xenia", &[]),
        ("dolphin-emu", &["dolphin-emu"]),
        ("retroarch", &["retroarch"]),
        ("mupen64plus", &["mupen64plus"]),
        ("melonDS", &["melonds"]),
        ("citra", &[]),
        ("yuzu", &[]),
        ("cemu", &[]),
    ];
    map.iter().find(|(k, _)| *k == key).map(|(_, pkgs)| *pkgs)
}

/// Packages that are only available from a third-party apt repository.
pub fn needs_repo(pkg: &str) -> bool {
    matches!(pkg, "brave-browser" | "google-chrome-stable")
}

/// Install the matching vendor apt repo so `apt-get` can then find the
/// package. Idempotent; safe to call before every install attempt.
pub fn ensure_third_party_repo(pkg: &str) -> Result<(), String> {
    let script = match pkg {
        "brave-browser" => include_str!("repos/brave.sh"),
        "google-chrome-stable" => include_str!("repos/chrome.sh"),
        _ => return Ok(()),
    };
    let status = std::process::Command::new("sh")
        .args(["-c", script])
        .status()
        .map_err(|e| format!("could not run repo setup: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("could not add the {pkg} repository"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Config;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// Serialises tests that mutate the process-wide `PATH`.
    static PATH_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    fn lock_path() -> MutexGuard<'static, ()> {
        PATH_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    #[test]
    fn curated_bin_detection() {
        assert!(is_curated_bin("firefox"));
        assert!(is_curated_bin("/usr/bin/pcsx2"));
        assert!(is_curated_bin("xdg-desktop-portal-gtk") == false);
    }

    #[test]
    fn apt_package_lookup() {
        assert_eq!(apt_packages_for_program("nautilus"), Some(&["nautilus"][..]));
        assert_eq!(apt_packages_for_program("pcsx2"), Some(&["pcsx2"][..]));
        assert_eq!(apt_packages_for_program("pcsx2-qt"), Some(&["pcsx2"][..]));
        assert_eq!(apt_packages_for_program("dolphin-emu"), Some(&["dolphin-emu"][..]));
        assert_eq!(apt_packages_for_program("retroarch"), Some(&["retroarch"][..]));
        assert_eq!(apt_packages_for_program("/usr/bin/firefox"), Some(&[][..]));
        assert_eq!(apt_packages_for_program("brave-browser"), Some(&["brave-browser"][..]));
        assert!(needs_repo("brave-browser"));
        assert!(!needs_repo("nautilus"));
    }

    #[test]
    fn scan_never_panics() {
        let games = scan_curated_games(&Config::default());
        // No crash with or without installed binaries; at most one tile per
        // browser / file manager / console.
        let apps: Vec<_> = games.iter().filter(|g| g.source == "app").collect();
        assert!(apps.len() <= BROWSERS.len() + FILE_MANAGERS.len());
        assert!(
            games
                .iter()
                .filter(|g| g.source == "emulator")
                .all(|g| !g.rom_exts.is_empty())
        );
    }

    #[test]
    fn finds_curated_tiles_when_binaries_present() {
        // A temp dir with fake executables on PATH must surface curated tiles
        // with the right source / platform / rom extensions.
        let _guard = lock_path();

        let dir = tempfile::tempdir().unwrap();
        let bin_dir = dir.path();
        for name in ["firefox", "brave-browser", "pcsx2", "rpcs3", "dolphin-emu", "duckstation"] {
            let p = bin_dir.join(name);
            std::fs::write(&p, "#!/bin/sh\nexec whatever".as_bytes()).unwrap();
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in ["firefox", "brave-browser", "pcsx2", "rpcs3", "dolphin-emu", "duckstation"] {
                let p = bin_dir.join(name);
                let mut perms = std::fs::metadata(&p).unwrap().permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&p, perms).unwrap();
            }
        }

        let old_path = std::env::var_os("PATH");
        let mut base_paths: Vec<std::path::PathBuf> = old_path
            .as_ref()
            .map(|p| std::env::split_paths(p).collect())
            .unwrap_or_default();
        base_paths.push(bin_dir.to_path_buf());
        let new_path = std::env::join_paths(base_paths).unwrap();
        std::env::set_var("PATH", &new_path);
        let games = scan_curated_games(&Config::default());
        match old_path {
            Some(p) => std::env::set_var("PATH", p),
            None => std::env::remove_var("PATH"),
        }

        let browsers: Vec<_> = games.iter().filter(|g| g.source == "app").collect();
        assert!(
            browsers.iter().any(|g| g.title == "Firefox"),
            "Firefox tile expected, got: {:?}",
            browsers.iter().map(|g| g.title.clone()).collect::<Vec<_>>()
        );
        assert!(browsers.iter().any(|g| g.title == "Brave"));
        assert!(browsers.iter().any(|g| g.installed), "found binaries should be installed");

        let emus: Vec<_> = games.iter().filter(|g| g.source == "emulator").collect();
        let ps2 = emus.iter().find(|g| g.platform.as_deref() == Some("PS2"));
        assert!(ps2.is_some(), "PS2 tile expected");
        assert_eq!(ps2.unwrap().rom_exts, vec!["iso", "chd", "cso", "bin", "img", "gz"]);
        assert!(ps2.unwrap().installed);
        assert!(
            emus
                .iter()
                .any(|g| g.platform.as_deref() == Some("PS3") && g.launch == Launch::Program {
                    program: bin_dir.join("rpcs3").to_string_lossy().to_string(),
                    args: vec![],
                })
        );
    }

    #[test]
    fn always_lists_curated_tiles_when_binaries_missing() {
        let _guard = lock_path();

        let empty_dir = tempfile::tempdir().unwrap();
        let old_path = std::env::var_os("PATH");
        std::env::set_var("PATH", empty_dir.path());
        let games = scan_curated_games(&Config::default());
        match old_path {
            Some(p) => std::env::set_var("PATH", p),
            None => std::env::remove_var("PATH"),
        }

        assert_eq!(
            games.iter().filter(|g| g.source == "app").count(),
            BROWSERS.len() + 1,
            "all browsers plus exactly one file manager should be listed even when missing"
        );
        assert_eq!(
            games.iter().filter(|g| g.source == "emulator").count(),
            EMULATORS.len(),
            "all consoles should be listed even when missing"
        );
        assert!(
            games.iter().all(|g| !g.installed),
            "with an empty PATH every curated tile must be marked not installed"
        );
    }
}