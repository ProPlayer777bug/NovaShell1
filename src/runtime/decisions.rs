//! Registry of user decisions for Windows files.
//!
//! A double-clicked `.exe` must never run silently. The handler (`--win-file`)
//! records the file here and asks the shell UI to confirm; this store lets the
//! choice ("run with Wine", "this is a game") survive a restart.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PendingAction {
    /// Show the confirmation dialog and wait for the user.
    Ask,
    /// Launch with Wine.
    Wine,
    /// Launch with Proton.
    Proton,
    /// Only inspect, never run.
    InspectOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingFile {
    pub path: PathBuf,
    pub action: PendingAction,
    pub is_game: bool,
    pub is_installer: bool,
    pub reasons: Vec<String>,
    pub created_at: i64,
}

/// Decisions the user made, keyed by absolute path.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Decisions {
    #[serde(default)]
    entries: BTreeMap<String, String>,
}

fn store_path() -> PathBuf {
    crate::util::data_dir().join("win-decisions.json")
}

impl Decisions {
    pub fn load() -> Decisions {
        std::fs::read_to_string(store_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        if let Ok(text) = serde_json::to_string_pretty(self) {
            if let Some(parent) = store_path().parent() {
                let _ = crate::util::ensure_dir(parent);
            }
            let _ = std::fs::write(store_path(), text);
        }
    }

    /// Remember a choice for a file ("wine" / "proton" / "never").
    pub fn remember(&mut self, path: &Path, runtime: &str) {
        self.entries.insert(path.to_string_lossy().to_string(), runtime.to_string());
        self.save();
    }

    /// The remembered choice, if any.
    pub fn get(&self, path: &Path) -> Option<&str> {
        self.entries.get(&path.to_string_lossy().to_string()).map(|s| s.as_str())
    }

    pub fn forget(&mut self, path: &Path) {
        self.entries.remove(&path.to_string_lossy().to_string());
        self.save();
    }

    /// Record a file that arrived from the browser/file manager and needs a
    /// decision, so the UI can show it in the Runtime Manager.
    pub fn add_pending(&self, file: PendingFile) {
        let path = file.path.clone();
        let mut pending = Decisions::load_pending();
        if let Some(existing) = pending.iter_mut().find(|p| p.path == path) {
            *existing = file;
        } else {
            pending.push(file);
        }
        let _ = std::fs::write(
            crate::util::data_dir().join("win-pending.json"),
            serde_json::to_string_pretty(&pending).unwrap_or_default(),
        );
    }

    pub fn load_pending() -> Vec<PendingFile> {
        std::fs::read_to_string(crate::util::data_dir().join("win-pending.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn clear_pending(path: &Path) {
        let mut pending = Decisions::load_pending();
        pending.retain(|p| p.path != path);
        let _ = std::fs::write(
            crate::util::data_dir().join("win-pending.json"),
            serde_json::to_string_pretty(&pending).unwrap_or_default(),
        );
    }

    // ------------------------- PS3 packages -------------------------

    /// A PS3 package the user still has to confirm installing.
    pub fn add_pending_pkg(&self, pkg: crate::runtime::ps3::Installable) {
        let path = pkg.path.clone();
        let mut pending = Self::load_pending_pkgs();
        if let Some(existing) = pending.iter_mut().find(|p| p.path == path) {
            *existing = pkg;
        } else {
            pending.push(pkg);
        }
        let _ = std::fs::write(
            crate::util::data_dir().join("ps3-pending.json"),
            serde_json::to_string_pretty(&pending).unwrap_or_default(),
        );
    }

    pub fn load_pending_pkgs() -> Vec<crate::runtime::ps3::Installable> {
        std::fs::read_to_string(crate::util::data_dir().join("ps3-pending.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn clear_pending_pkg(path: &Path) {
        let mut pending = Self::load_pending_pkgs();
        pending.retain(|p| p.path != path);
        let _ = std::fs::write(
            crate::util::data_dir().join("ps3-pending.json"),
            serde_json::to_string_pretty(&pending).unwrap_or_default(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_xml_lists_every_windows_type() {
        let xml = super::super::apps::windows_mime_xml();
        for glob in ["*.exe", "*.msi", "*.bat", "*.cmd"] {
            assert!(xml.contains(glob), "missing {glob}");
        }
    }

    #[test]
    fn handler_entry_asks_the_shell() {
        use super::super::apps::HANDLER_DESKTOP_ENTRY;
        assert!(HANDLER_DESKTOP_ENTRY.contains("--win-file"));
        assert!(
            HANDLER_DESKTOP_ENTRY.contains("application/x-ms-dos-executable"),
            "the handler must claim the exe mime type"
        );
    }

    #[test]
    fn associations_install_without_root_and_claim_all_types() {
        let home = std::env::temp_dir().join(format!("nova-assoc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        super::super::apps::install_file_associations(&home).unwrap();
        let entry = home.join(".local/share/applications/novashell-windows-handler.desktop");
        assert!(entry.is_file(), "handler entry must be user-local");
        let defaults = std::fs::read_to_string(home.join(".config/mimeapps.list")).unwrap();
        for mime in super::super::apps::WINDOWS_MIME_TYPES {
            assert!(defaults.contains(mime), "missing default for {mime}");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn decisions_roundtrip_in_a_temp_data_dir() {
        let dir = std::env::temp_dir().join(format!("nova-dec-{}", std::process::id()));
        std::env::set_var("NOVASHELL_DATA_DIR", &dir);
        let mut d = Decisions::default();
        d.remember(Path::new("/games/thing.exe"), "wine");
        assert_eq!(d.get(Path::new("/games/thing.exe")), Some("wine"));
        assert_eq!(d.get(Path::new("/games/other.exe")), None);
        d.forget(Path::new("/games/thing.exe"));
        assert_eq!(d.get(Path::new("/games/thing.exe")), None);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::env::remove_var("NOVASHELL_DATA_DIR");
    }
}
