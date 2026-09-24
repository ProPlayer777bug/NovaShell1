//! Desktop-entry integration: scans standard application directories and
//! surfaces native (and Flatpak) applications in the library.

use crate::games::{Game, Launch};
use crate::integrations::Provider;
use crate::launcher::parse_desktop_exec;
use crate::settings::Config;
use crate::util;
use std::path::{Path, PathBuf};

/// Directories scanned for `.desktop` files.
pub fn desktop_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
    ];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/share/applications"));
        dirs.push(home.join(".local/share/flatpak/exports/share/applications"));
        dirs.push(home.join(".snap/apps"));
    }
    dirs
}

/// Categories that are never shown as launchable apps.
const SKIP_CATEGORIES: &[&str] = &[
    "System",
    "Settings",
    "X-GNOME-SystemSettings",
    "Core",
    "Development",
];

#[derive(Debug)]
struct DesktopEntry {
    name: String,
    exec: Option<String>,
    icon: Option<String>,
    nodisplay: bool,
    hidden: bool,
    terminal: bool,
}

fn parse_desktop(path: &Path) -> Option<DesktopEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut entry = DesktopEntry {
        name: String::new(),
        exec: None,
        icon: None,
        nodisplay: false,
        hidden: false,
        terminal: false,
    };
    let mut in_main = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "Name" => entry.name = value.to_string(),
                "Exec" => entry.exec = Some(value.to_string()),
                "Icon" => entry.icon = Some(value.to_string()),
                "NoDisplay" => entry.nodisplay = value == "true",
                "Hidden" => entry.hidden = value == "true",
                "Terminal" => entry.terminal = value == "true",
                "Categories" => {
                    let cats: Vec<&str> = value.split(';').collect();
                    if cats.iter().any(|c| SKIP_CATEGORIES.contains(c)) {
                        entry.nodisplay = true;
                    }
                }
                _ => {}
            }
        }
    }
    if entry.name.is_empty() {
        return None;
    }
    Some(entry)
}

/// Resolve a desktop icon name to a real path when possible.
pub fn find_icon(name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    let name = name.strip_prefix("file://").unwrap_or(name).to_string();
    let p = PathBuf::from(&name);
    if p.is_absolute() && p.exists() {
        return Some(p);
    }
    let base_dirs = vec![
        PathBuf::from("/usr/share/icons/hicolor"),
        if let Some(h) = dirs::home_dir() {
            h.join(".local/share/icons/hicolor")
        } else {
            PathBuf::new()
        },
    ];
    let theme_sizes = ["256x256", "128x128", "96x96", "64x64", "48x48", "32x32", "scalable"];
    for base in base_dirs {
        for size in &theme_sizes {
            let cands = [
                base.join("apps").join(format!("{name}.png")),
                base.join(size).join("apps").join(format!("{name}.png")),
                base.join("apps").join(format!("{name}.svg")),
                base.join(size).join("apps").join(format!("{name}.svg")),
            ];
            for c in cands {
                if c.exists() {
                    return Some(c);
                }
            }
        }
    }
    for ext in ["png", "svg", "xpm"] {
        let c = PathBuf::from("/usr/share/pixmaps").join(format!("{name}.{ext}"));
        if c.exists() {
            return Some(c);
        }
    }
    None
}

/// Scan desktop entries into library candidates.
pub fn scan_desktop_games(cfg: &Config) -> Vec<Game> {
    let mut out: Vec<Game> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let mut dirs = desktop_dirs();
    // Extra user-configured game/app directories.
    for d in &cfg.game_dirs {
        dirs.push(util::expand_tilde(&d.to_string_lossy()));
    }

    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for f in rd.flatten() {
            let path = f.path();
            let Some(ext) = path.extension().map(|e| e.to_string_lossy().to_string())
            else {
                continue;
            };
            if ext != "desktop" {
                continue;
            }
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let id = format!("desktop-{file_name}");
            if !seen.insert(id.clone()) {
                continue;
            }
            let Some(entry) = parse_desktop(&path) else {
                continue;
            };
            if entry.nodisplay || entry.hidden || entry.terminal || entry.exec.is_none() {
                continue;
            }
            let (program, args) = match parse_desktop_exec(entry.exec.as_ref().unwrap()) {
                Some(x) => x,
                None => continue,
            };
            // Curated apps/emulators (browsers, file managers, emulators) are
            // owned by the `apps` provider. Skip here only when the curated
            // provider also resolves the binary, so flatpak/snap installs the
            // PATH lookup misses still get a tile from the desktop scan.
            if crate::integrations::apps::is_curated_bin(&program)
                && crate::integrations::apps::find_bin(
                    Path::new(&program)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default()
                        .as_str(),
                )
                .is_some()
            {
                continue;
            }
            out.push(Game {
                id,
                title: entry.name,
                source: "desktop".into(),
                launch: Launch::Program { program, args },
                icon: entry.icon.as_deref().and_then(find_icon),
                artwork: None,
                last_played: None,
                playtime_secs: 0,
                favorite: false,
                installed: true,
                platform: None,
                rom_exts: Vec::new(),
            });
        }
    }
    out
}

pub struct DesktopProvider;

impl Provider for DesktopProvider {
    fn id(&self) -> &'static str {
        "desktop"
    }
    fn scan(&self, cfg: &Config) -> Vec<Game> {
        scan_desktop_games(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_desktop() -> &'static str {
        "[Desktop Entry]\nType=Application\nName=GNOME Files\nExec=nautilus %U\nIcon=org.gnome.Nautilus\nCategories=GNOME;GTK;Utility;\n"
    }

    #[test]
    fn parses_gnome_files_entry() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("org.gnome.Nautilus.desktop");
        std::fs::write(&p, sample_desktop()).unwrap();
        let e = parse_desktop(&p).unwrap();
        assert_eq!(e.name, "GNOME Files");
        assert!(!e.hidden);
        assert_eq!(e.exec.as_deref(), Some("nautilus %U"));
    }

    #[test]
    fn icon_resolution_returns_none_on_trash() {
        // Should never panic, and missing icons are None.
        assert!(find_icon("definitely-not-a-real-icon-name-xyz").is_none());
    }
}