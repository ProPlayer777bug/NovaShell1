//! Per-emulator library of remembered game files.
//!
//! When a ROM is booted the shell records it, so the next time an emulator tile
//! is opened the picker can offer the games that are already set up instead of
//! starting from an empty folder listing.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// `emulator id -> game paths, most recently used first`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RomLibrary {
    #[serde(default)]
    entries: BTreeMap<String, Vec<String>>,
}

fn store_path() -> PathBuf {
    crate::util::data_dir().join("roms.json")
}

impl RomLibrary {
    pub fn load() -> Self {
        std::fs::read_to_string(store_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let path = store_path();
            if let Some(parent) = path.parent() {
                let _ = crate::util::ensure_dir(parent);
            }
            let _ = std::fs::write(path, text);
        }
    }

    /// Games remembered for one emulator, newest first.
    pub fn games_for(&self, emulator_id: &str) -> Vec<String> {
        self.entries
            .get(emulator_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Record a launched game, moving it to the front and dropping duplicates.
    pub fn remember(&mut self, emulator_id: &str, path: &str) {
        let list = self.entries.entry(emulator_id.to_string()).or_default();
        list.retain(|p| p != path);
        list.insert(0, path.to_string());
        list.truncate(50);
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_store_reads_empty() {
        let lib = RomLibrary::default();
        assert!(lib.games_for("emulator-emulator-pcsx2").is_empty());
    }

    #[test]
    fn remember_moves_entry_to_front_and_dedupes() {
        let mut lib = RomLibrary::default();
        lib.entries.insert("emu".into(), vec!["/a.iso".into()]);
        lib.remember("emu", "/b.iso");
        lib.remember("emu", "/a.iso");
        assert_eq!(lib.games_for("emu"), vec!["/a.iso", "/b.iso"]);
    }
}
