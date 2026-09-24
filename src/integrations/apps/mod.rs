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

struct EmulatorDef {
    platform: &'static str,
    pretty: &'static str,
    icon: &'static str,
    bins: &'static [&'static str],
    exts: &'static [&'static str],
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
    },
    EmulatorDef {
        platform: "PS2",
        pretty: "PlayStation 2",
        icon: "PCSX2",
        bins: &["pcsx2"],
        exts: &["iso", "chd", "cso", "bin", "img", "gz"],
    },
    EmulatorDef {
        platform: "PS3",
        pretty: "PlayStation 3",
        icon: "rpcs3",
        bins: &["rpcs3"],
        exts: &["pkg", "iso", "ps3dir"],
    },
    EmulatorDef {
        platform: "Xbox",
        pretty: "Xbox / Xbox 360",
        icon: "xemu",
        bins: &["xemu", "xenia"],
        exts: &["iso", "xiso", "xbe", "xex"],
    },
    EmulatorDef {
        platform: "Wii",
        pretty: "Wii / GameCube",
        icon: "dolphin-emu",
        bins: &["dolphin-emu"],
        exts: &["iso", "rvz", "gcm", "wbfs", "nkit", "wad", "dol"],
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
    },
];

fn curated_app(
    title: &str,
    source: &str,
    program: &str,
    icon_name: Option<&str>,
    platform: Option<&str>,
    rom_exts: Vec<String>,
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
        installed: true,
        platform: platform.map(String::from),
        rom_exts,
    }
}

/// Scan curated apps + emulators into library candidates.
pub fn scan_curated_games(_cfg: &Config) -> Vec<Game> {
    let mut out: Vec<Game> = Vec::new();

    for (title, icon, cands) in BROWSERS {
        if let Some(bin) = cands.iter().find_map(|c| bin_ok(c)) {
            out.push(curated_app(title, "app", &bin, Some(icon), None, Vec::new()));
        }
    }
    for (title, icon, cands) in FILE_MANAGERS {
        if let Some(bin) = cands.iter().find_map(|c| bin_ok(c)) {
            out.push(curated_app(title, "app", &bin, Some(icon), None, Vec::new()));
            break;
        }
    }
    for def in EMULATORS {
        if let Some(bin) = def.bins.iter().find_map(|c| bin_ok(c)) {
            out.push(curated_app(
                def.pretty,
                "emulator",
                &bin,
                Some(def.icon),
                Some(def.platform),
                def.exts.iter().map(|e| e.to_string()).collect(),
            ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Config;

    #[test]
    fn curated_bin_detection() {
        assert!(is_curated_bin("firefox"));
        assert!(is_curated_bin("/usr/bin/pcsx2"));
        assert!(is_curated_bin("xdg-desktop-portal-gtk") == false);
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
        use std::sync::{Mutex, MutexGuard, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard: MutexGuard<'static, ()> = LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();

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

        let emus: Vec<_> = games.iter().filter(|g| g.source == "emulator").collect();
        let ps2 = emus.iter().find(|g| g.platform.as_deref() == Some("PS2"));
        assert!(ps2.is_some(), "PS2 tile expected");
        assert_eq!(ps2.unwrap().rom_exts, vec!["iso", "chd", "cso", "bin", "img", "gz"]);
        assert!(
            emus
                .iter()
                .any(|g| g.platform.as_deref() == Some("PS3") && g.launch == Launch::Program {
                    program: bin_dir.join("rpcs3").to_string_lossy().to_string(),
                    args: vec![],
                })
        );
    }
}