//! Windows application metadata and installer detection.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Extensions the runtime layer recognises as Windows content.
pub const WINDOWS_EXTENSIONS: &[&str] = &["exe", "msi", "bat", "cmd"];

/// Filenames that are installers regardless of extension.
pub const INSTALLER_NAMES: &[&str] = &["setup", "install", "installer", "unins000"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppKind {
    WindowsApp,
    WindowsGame,
}

impl AppKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AppKind::WindowsApp => "windows-app",
            AppKind::WindowsGame => "windows-game",
        }
    }
}

/// Persisted metadata for an installed Windows application or game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowsApp {
    pub id: String,
    pub name: String,
    pub kind: AppKind,
    pub runtime: String,
    #[serde(default)]
    pub runtime_version: Option<String>,
    pub prefix: PathBuf,
    /// Absolute path to the main executable, or a path inside the prefix.
    pub executable: PathBuf,
    #[serde(default)]
    pub working_directory: Option<PathBuf>,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub environment: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub installed_at: i64,
    #[serde(default)]
    pub last_played: Option<i64>,
    #[serde(default)]
    pub play_time_secs: u64,
    #[serde(default)]
    pub icon: Option<PathBuf>,
}

impl WindowsApp {
    pub fn new(id: &str, name: &str, kind: AppKind, runtime: &str) -> Self {
        WindowsApp {
            id: id.to_string(),
            name: name.to_string(),
            kind,
            runtime: runtime.to_string(),
            runtime_version: None,
            prefix: PathBuf::new(),
            executable: PathBuf::new(),
            working_directory: None,
            arguments: Vec::new(),
            environment: Default::default(),
            installed_at: crate::util::unix_now(),
            last_played: None,
            play_time_secs: 0,
            icon: None,
        }
    }

    /// Validate stored paths: executable must still exist, prefix must still
    /// be a directory. Anything else means the app needs repair.
    pub fn validate_paths(&self) -> Vec<crate::runtime::Check> {
        use crate::runtime::Check;
        let mut checks = Vec::new();
        checks.push(if self.executable.is_file() {
            Check::ok("Executable")
        } else {
            Check::fail("Executable", format!("{} is missing", self.executable.display()))
        });
        checks.push(if self.prefix.is_dir() {
            Check::ok("Prefix")
        } else {
            Check::fail("Prefix", format!("{} is missing", self.prefix.display()))
        });
        checks
    }

    /// Store location for the app catalogue.
    pub fn store_path() -> PathBuf {
        crate::util::data_dir().join("windows-apps.json")
    }

    pub fn load_all() -> Vec<WindowsApp> {
        std::fs::read_to_string(Self::store_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save_all(apps: &[WindowsApp]) -> anyhow::Result<()> {
        let path = Self::store_path();
        if let Some(parent) = path.parent() {
            crate::util::ensure_dir(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(apps)?)?;
        Ok(())
    }

    pub fn upsert(apps: &mut Vec<WindowsApp>, app: WindowsApp) {
        match apps.iter_mut().find(|a| a.id == app.id) {
            Some(slot) => *slot = app,
            None => apps.push(app),
        }
    }
}

/// Is this path a Windows executable we know how to handle?
pub fn is_windows_executable(path: &Path) -> bool {
    path.extension()
        .map(|e| {
            let e = e.to_string_lossy().to_lowercase();
            WINDOWS_EXTENSIONS.contains(&e.as_str())
        })
        .unwrap_or(false)
}

/// XDG desktop entries for the Windows file associations, so double-clicking an
/// `.exe`/`.msi` in the file manager goes through the Runtime Manager (which
/// asks for confirmation) instead of trying to execute it directly.
pub const HANDLER_DESKTOP_ENTRY: &str = include_str!("windows-handler.desktop");

/// XDG desktop entry for PS3 packages, so double-clicking a `.pkg` installs it
/// through NovaShell instead of failing in Archive Manager ("not an archive").
pub const PS3_HANDLER_DESKTOP_ENTRY: &str = include_str!("ps3-handler.desktop");

/// Mime types Nova claims for PS3 packages.
pub const PS3_MIME_TYPES: [&str; 3] = [
    "application/x-ps3-pkg",
    "application/x-ps3-rap",
    "application/x-ps3-edat",
];

/// Install/refresh the Nova Windows handler associations for the given user.
///
/// Uses the per-user XDG directories so this needs no root: the handler lives
/// in `~/.local/share/applications`, the mime definitions in
/// `~/.local/share/mime`, and the defaults in `~/.config/mimeapps.list`.
pub fn install_file_associations(user_home: &Path) -> anyhow::Result<()> {
    let apps_dir = user_home.join(".local/share/applications");
    let mime_dir = user_home.join(".local/share/mime/packages");
    let mime_root = user_home.join(".local/share/mime");
    std::fs::create_dir_all(&apps_dir)?;
    std::fs::create_dir_all(&mime_dir)?;
    std::fs::write(
        apps_dir.join("novashell-windows-handler.desktop"),
        HANDLER_DESKTOP_ENTRY,
    )?;
    std::fs::write(
        apps_dir.join("novashell-ps3-handler.desktop"),
        PS3_HANDLER_DESKTOP_ENTRY,
    )?;
    std::fs::write(mime_dir.join("novashell-windows.xml"), windows_mime_xml())?;
    std::fs::write(mime_dir.join("novashell-ps3.xml"), ps3_mime_xml())?;
    set_default_handler(user_home)?;
    // Refresh the user-local caches. These are best-effort: the files above are
    // already correct without them, and the tools may not be installed.
    quiet_status(&[
        "update-desktop-database",
        apps_dir.to_string_lossy().as_ref(),
    ]);
    quiet_status(&["update-mime-database", mime_root.to_string_lossy().as_ref()]);
    for mime in WINDOWS_MIME_TYPES {
        quiet_status(&[
            "xdg-mime",
            "default",
            "novashell-windows-handler.desktop",
            mime,
        ]);
    }
    for mime in PS3_MIME_TYPES {
        quiet_status(&[
            "xdg-mime",
            "default",
            "novashell-ps3-handler.desktop",
            mime,
        ]);
    }
    Ok(())
}

fn quiet_status(args: &[&str]) {
    if let Some((program, rest)) = args.split_first() {
        let _ = std::process::Command::new(program)
            .args(rest)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// The mime types Nova claims handlers for.
pub const WINDOWS_MIME_TYPES: [&str; 4] = [
    "application/x-ms-dos-executable",
    "application/x-msi",
    "application/x-bat",
    "application/x-cmd",
];

pub fn windows_mime_xml() -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">\n",
    );
    for (mime, glob, icon) in [
        ("application/x-ms-dos-executable", "*.exe", "application-x-executable"),
        ("application/x-msi", "*.msi", "package-x-generic"),
        ("application/x-bat", "*.bat", "application-x-executable"),
        ("application/x-cmd", "*.cmd", "application-x-executable"),
    ] {
        xml.push_str(&format!(
            "  <mime-type type=\"{mime}\">\n    <comment>Windows file</comment>\n    <glob pattern=\"{glob}\"/>\n    <icon name=\"{icon}\"/>\n  </mime-type>\n"
        ));
    }
    xml.push_str("</mime-info>\n");
    xml
}

/// Mime definitions for PS3 packages, so a `.pkg` is recognised as a package
/// instead of being handed to Archive Manager.
pub fn ps3_mime_xml() -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">\n",
    );
    for (mime, glob, comment) in [
        ("application/x-ps3-pkg", "*.pkg", "PlayStation 3 package"),
        ("application/x-ps3-rap", "*.rap", "PlayStation 3 key file"),
        ("application/x-ps3-edat", "*.edat", "PlayStation 3 encrypted data"),
    ] {
        xml.push_str(&format!(
            "  <mime-type type=\"{mime}\">\n    <comment>{comment}</comment>\n    <glob pattern=\"{glob}\"/>\n    <icon name=\"package-x-generic\"/>\n  </mime-type>\n"
        ));
    }
    xml.push_str("</mime-info>\n");
    xml
}

/// Set the default handler for the Windows mime types in a user's config.
pub fn set_default_handler(user_home: &Path) -> anyhow::Result<()> {
    let config = user_home.join(".config");
    std::fs::create_dir_all(&config)?;
    let mut list = String::from("[Default Applications]\n");
    for mime in WINDOWS_MIME_TYPES {
        list.push_str(&format!(
            "{mime}=novashell-windows-handler.desktop\n"
        ));
    }
    for mime in PS3_MIME_TYPES {
        list.push_str(&format!("{mime}=novashell-ps3-handler.desktop\n"));
    }
    std::fs::write(config.join("mimeapps.list"), list)?;
    Ok(())
}

/// Heuristic: does this look like an installer rather than the game itself?
pub fn is_installer(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if ext == "msi" {
        return true;
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if INSTALLER_NAMES.iter().any(|n| stem == *n) {
        return true;
    }
    // Vendors prefix their installers, so match installer words as separate
    // tokens: `RetroArena-Setup.exe`, `setup_2.1.exe`, `Fable Install.exe`.
    // This must not fire on names that merely contain the letters, e.g.
    // `SetupTest.exe` is a game, not an installer.
    let tokens: Vec<&str> = stem
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    tokens
        .iter()
        .any(|t| INSTALLER_NAMES.contains(t) || *t == "installer" || *t == "installers")
}

/// Heuristics for "this executable is probably a game".
///
/// Deliberately conservative and never authoritative: the result is a
/// *suggestion* that the user can always override in the launch dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameGuess {
    pub is_game: bool,
    pub reasons: Vec<String>,
}

pub fn guess_if_game(path: &Path) -> GameGuess {
    let mut reasons = Vec::new();
    let mut score = 0;

    if let Ok(meta) = std::fs::metadata(path) {
        // Big executables are usually games.
        if meta.len() > 64 * 1024 * 1024 {
            score += 2;
            reasons.push("large executable".into());
        } else if meta.len() < 2 * 1024 * 1024 {
            score -= 1;
            reasons.push("small executable".into());
        }
    }

    if let Some(dir) = path.parent() {
        let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default().to_lowercase();
        if ["program files (x86)", "program files", "steamapps", "common"].iter().any(|m| name.contains(m)) {
            score += 1;
            reasons.push(format!("installed under {name}"));
        }
        // A DirectX/Vulkan payload next to the exe is a strong hint.
        if dir.join("d3d11.dll").is_file() || dir.join("dxgi.dll").is_file() || dir.join("vulkan-1.dll").is_file() {
            score += 2;
            reasons.push("ships a graphics runtime".into());
        }
        let siblings = std::fs::read_dir(dir).map(|r| r.count()).unwrap_or(0);
        if siblings > 30 {
            score += 1;
            reasons.push("large game directory".into());
        }
    }

    GameGuess {
        is_game: score >= 3,
        reasons,
    }
}

/// Find executables created by an installer inside a prefix, so the user can
/// choose the real main executable.
pub fn find_installed_executables(prefix: &Path, limit: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![prefix.to_path_buf()];
    let mut budget = 50_000;
    while let Some(dir) = stack.pop() {
        if found.len() >= limit || budget == 0 {
            break;
        }
        budget -= 1;
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            match e.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(p),
                Ok(ft) if ft.is_file() && is_windows_executable(&p) && !is_installer(&p) => {
                    found.push(p);
                    if found.len() >= limit {
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_windows_extensions() {
        assert!(is_windows_executable(Path::new("game.EXE")));
        assert!(is_windows_executable(Path::new("setup.msi")));
        assert!(is_windows_executable(Path::new("run.bat")));
        assert!(!is_windows_executable(Path::new("game.iso")));
    }

    #[test]
    fn detects_installers() {
        assert!(is_installer(Path::new("/d/setup.exe")));
        assert!(is_installer(Path::new("/d/Install.exe")));
        assert!(is_installer(Path::new("/d/anything.msi")));
        assert!(!is_installer(Path::new("/d/MyGame.exe")));
    }

    #[test]
    fn detects_vendored_installer_names() {
        // Vendors prefix installers: token matching must catch these.
        assert!(is_installer(Path::new("/d/RetroArena-Setup.exe")));
        assert!(is_installer(Path::new("/d/setup_2.1.exe")));
        assert!(is_installer(Path::new("/d/Retro Arena Installer.exe")));
        // ...but a word that merely contains "setup" is not an installer.
        assert!(!is_installer(Path::new("/d/SetupTest.exe")));
        assert!(!is_installer(Path::new("/d/installation_tool.exe")));
    }

    #[test]
    fn metadata_roundtrips() {
        let mut app = WindowsApp::new("my-game", "My Game", AppKind::WindowsGame, "proton");
        app.prefix = "/p".into();
        app.executable = "/p/drive_c/MyGame.exe".into();
        let text = serde_json::to_string(&app).unwrap();
        let back: WindowsApp = serde_json::from_str(&text).unwrap();
        assert_eq!(back, app);
        assert_eq!(back.kind.as_str(), "windows-game");
    }

    #[test]
    fn path_validation_flags_missing_files() {
        let mut app = WindowsApp::new("a", "A", AppKind::WindowsApp, "wine");
        app.executable = "/definitely/missing.exe".into();
        app.prefix = "/definitely/missing-prefix".into();
        let checks = app.validate_paths();
        assert_eq!(checks.len(), 2);
        assert!(checks.iter().all(|c| !c.ok));
    }

    #[test]
    fn upsert_replaces_matching_id() {
        let mut apps = Vec::new();
        let mut a = WindowsApp::new("x", "First", AppKind::WindowsApp, "wine");
        WindowsApp::upsert(&mut apps, a.clone());
        a.name = "Renamed".into();
        WindowsApp::upsert(&mut apps, a);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "Renamed");
    }

    #[test]
    fn game_guess_is_only_a_suggestion() {
        // A missing file must not claim to be a game.
        let g = guess_if_game(Path::new("/nope/huge.exe"));
        assert!(!g.is_game);
    }

    #[test]
    fn finds_executables_in_a_prefix() {
        let dir = std::env::temp_dir().join(format!("nova-scan-{}", std::process::id()));
        let drive = dir.join("drive_c").join("Game");
        std::fs::create_dir_all(&drive).unwrap();
        std::fs::write(drive.join("Game.exe"), b"x").unwrap();
        std::fs::write(drive.join("setup.exe"), b"x").unwrap();
        std::fs::write(drive.join("readme.txt"), b"x").unwrap();
        let found = find_installed_executables(&dir, 10);
        assert_eq!(found.len(), 1, "installers are excluded from the main-exe list");
        assert!(found[0].ends_with("Game.exe"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
