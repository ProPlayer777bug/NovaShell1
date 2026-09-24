//! Steam integration: discovers the Steam install, parses
//! `libraryfolders.vdf` + `appmanifest_*.acf`, and produces library entries
//! launchable via `steam -applaunch <appid>`.

use crate::games::{Game, Launch};
use crate::integrations::Provider;
use crate::settings::Config;
use crate::util;
use anyhow::{anyhow, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Minimal Valve Data Format (VDF) parser suitable for `libraryfolders.vdf`
/// and `appmanifest_*.acf`.
#[derive(Debug, Clone, PartialEq)]
pub enum Vdf {
    Str(String),
    Int(i64),
    Obj(BTreeMap<String, Vdf>),
}

impl Vdf {
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Obj(m) => m.get(key),
            _ => None,
        }
    }
    pub fn str_at(&self, key: &str) -> Option<String> {
        match self.get(key) {
            Some(Vdf::Str(s)) => Some(s.clone()),
            _ => None,
        }
    }
    pub fn int_at(&self, key: &str) -> Option<i64> {
        match self.get(key) {
            Some(Vdf::Int(i)) => Some(*i),
            // AppManifest values are quoted strings ("570"); parse them too.
            Some(Vdf::Str(s)) => s.trim().parse::<i64>().ok(),
            _ => None,
        }
    }
}

struct Parser<'a> {
    it: std::iter::Peekable<std::str::Chars<'a>>,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Parser {
            it: s.chars().peekable(),
        }
    }

    fn skip_ws(&mut self) {
        while let Some(&c) = self.it.peek() {
            if c.is_whitespace() {
                self.it.next();
            } else {
                break;
            }
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.it.peek().copied()
    }

    fn next(&mut self) -> Option<char> {
        self.it.next()
    }

    fn parse_quoted(&mut self) -> Result<String> {
        if self.next() != Some('"') {
            return Err(anyhow!("expected opening quote"));
        }
        let mut s = String::new();
        while let Some(c) = self.next() {
            match c {
                '\\' => {
                    if let Some(e) = self.next() {
                        s.push(e);
                    }
                }
                '"' => break,
                _ => s.push(c),
            }
        }
        Ok(s)
    }

    fn parse_key(&mut self) -> Result<String> {
        self.skip_ws();
        match self.peek() {
            Some('"') => self.parse_quoted(),
            Some(c) if c != '{' && c != '}' => {
                let mut s = String::new();
                while let Some(c) = self.peek() {
                    if c.is_whitespace() || c == '{' || c == '}' {
                        break;
                    }
                    s.push(self.next().unwrap());
                }
                if s.is_empty() {
                    Err(anyhow!("empty bare key"))
                } else {
                    Ok(s)
                }
            }
            _ => Err(anyhow!("expected key, found {:?}", self.peek())),
        }
    }

    fn parse_pairs(&mut self, out: &mut BTreeMap<String, Vdf>) -> Result<()> {
        loop {
            self.skip_ws();
            match self.peek() {
                None => break,
                Some('}') => {
                    self.next();
                    break;
                }
                Some(_) => {
                    let k = self.parse_key()?;
                    self.skip_ws();
                    let v = match self.peek() {
                        Some('{') => {
                            self.next();
                            let mut m = BTreeMap::new();
                            self.parse_pairs(&mut m)?;
                            Vdf::Obj(m)
                        }
                        Some('"') => Vdf::Str(self.parse_quoted()?),
                        Some(_) => {
                            let raw = self.parse_key()?;
                            Vdf::Int(raw.parse::<i64>().unwrap_or(0))
                        }
                        None => break,
                    };
                    out.insert(k, v);
                }
            }
        }
        Ok(())
    }
}

/// Parse VDF text into an object node.
pub fn parse_vdf(input: &str) -> Result<Vdf> {
    let mut p = Parser::new(input);
    let mut map = BTreeMap::new();
    p.parse_pairs(&mut map)?;
    Ok(Vdf::Obj(map))
}

/// Locate the Steam install root.
pub fn find_steam_root(cfg: &Config) -> Option<PathBuf> {
    if let Some(p) = &cfg.launchers.steam_path {
        let p = util::expand_tilde(&p.to_string_lossy());
        if p.join("steamapps").exists() {
            return Some(p);
        }
    }
    let home = dirs::home_dir()?;
    let candidates = [
        home.join(".steam").join("steam"),
        home.join(".local").join("share").join("Steam"),
        home.join(".steam"),
    ];
    candidates
        .into_iter()
        .find(|cand| cand.join("steamapps").is_dir())
}

/// All library folders declared in `libraryfolders.vdf` plus the root itself.
pub fn library_folders(root: &Path) -> Vec<PathBuf> {
    let mut folders = vec![root.to_path_buf()];
    let vdf_path = root.join("steamapps").join("libraryfolders.vdf");
    if let Ok(text) = std::fs::read_to_string(&vdf_path) {
        if let Ok(Vdf::Obj(top)) = parse_vdf(&text) {
            if let Some(Vdf::Obj(lf)) = top.get("libraryfolders") {
                for (k, v) in lf {
                    if let Some(path) = v.str_at("path") {
                        let p = PathBuf::from(path);
                        if p.is_dir() {
                            folders.push(p);
                        }
                    }
                    let _ = k;
                }
            } else if let Some(path) = top.get("path").and_then(|v| v.str_at("path")) {
                // Flat layout with a single "path" key.
                let p = PathBuf::from(path);
                if p.is_dir() {
                    folders.push(p);
                }
            }
        }
    }
    folders.sort();
    folders.dedup();
    folders
}

/// Scan appmanifest files and build candidate games.
pub fn scan_steam_games(cfg: &Config) -> Vec<Game> {
    let Some(root) = find_steam_root(cfg) else {
        return vec![];
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    for folder in library_folders(&root) {
        let apps = folder.join("steamapps");
        let Ok(rd) = std::fs::read_dir(&apps) else {
            continue;
        };
        for entry in rd.flatten() {
            let name = entry.file_name();
            let file = name.to_string_lossy();
            if !file.starts_with("appmanifest_") || !file.ends_with(".acf") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            let Ok(Vdf::Obj(top)) = parse_vdf(&text) else {
                continue;
            };
            let Some(state) = top.get("AppState") else {
                continue;
            };
            let Some(appid) = state.int_at("appid") else {
                continue;
            };
            let id = format!("steam-{appid}");
            if !seen.insert(id.clone()) {
                continue;
            }
            let title = state
                .str_at("name")
                .unwrap_or_else(|| format!("Steam game {appid}"));
            let installed = match state.int_at("StateFlags") {
                Some(f) => f & 1 == 1, // STATE_FLAG_FULLY_INSTALLED
                None => true,
            };
            let artwork = state
                .str_at("installdir")
                .map(|dir| folder.join("steamapps").join("common").join(dir))
                .filter(|p| p.is_dir())
                .map(|dir| dir.join("library_capsule.jpg"))
                .filter(|p| p.exists());

            out.push(Game {
                id,
                title,
                source: "steam".into(),
                launch: Launch::Steam {
                    app_id: appid.to_string(),
                },
                icon: None,
                artwork,
                last_played: None,
                playtime_secs: 0,
                favorite: false,
                installed,
                platform: None,
                rom_exts: Vec::new(),
            rom_dir: None,
            });
        }
    }
    out
}

/// Best available `steam` executable (or a flatpak launch string).
pub fn steam_binary() -> Option<String> {
    let cands = [
        "/usr/bin/steam".to_string(),
        "/usr/games/steam".to_string(),
        "steam".to_string(),
    ];
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    let home_cands = [
        home.join(".local/share/Steam/steam.sh")
            .to_string_lossy()
            .to_string(),
        home.join(".steam/steam/steam.sh").to_string_lossy().to_string(),
    ];
    for c in cands.iter().chain(home_cands.iter()) {
        if std::path::Path::new(c).exists() && c != "steam" {
            return Some(c.clone());
        }
    }
    // `steam` on PATH
    if std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v steam >/dev/null 2>&1")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Some("steam".to_string());
    }
    if std::path::Path::new("/var/lib/flatpak/app/com.valvesoftware.Steam").exists() {
        return Some("flatpak run com.valvesoftware.Steam".to_string());
    }
    None
}

pub struct SteamProvider;

impl Provider for SteamProvider {
    fn id(&self) -> &'static str {
        "steam"
    }
    fn scan(&self, cfg: &Config) -> Vec<Game> {
        scan_steam_games(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_libraryfolders_vdf() {
        let vdf = r#"
"libraryfolders"
{
    "path"     "C:\\Program Files (x86)\\Steam"
    "contentstatsid"   "1337"
    "0"
    {
        "path"          "D:\\Games\\SteamLibrary"
        "label"         ""
    }
    "1"
    {
        "path"          "E:\\SteamGames"
        "label"         "extras"
    }
}
"#;
        let v = parse_vdf(vdf).unwrap();
        let folders = v.get("libraryfolders").unwrap();
        assert_eq!(folders.str_at("path").unwrap(), "C:\\Program Files (x86)\\Steam");
        assert_eq!(folders.get("0").unwrap().str_at("path").unwrap(), "D:\\Games\\SteamLibrary");
    }

    #[test]
    fn parses_appmanifest_acf() {
        let acf = r#"
"AppState"
{
    "appid"     "570"
    "Universe"     "1"
    "name"     "Dota 2"
    "StateFlags"     "4"
    "installdir"     "dota 2 beta"
    "SizeOnDisk"     "32325959627"
}
"#;
        let v = parse_vdf(acf).unwrap();
        let state = v.get("AppState").unwrap();
        assert_eq!(state.int_at("appid").unwrap(), 570);
        assert_eq!(state.str_at("name").unwrap(), "Dota 2");
    }

    #[test]
    fn steam_binary_never_panics() {
        // Must always return Some or None — never crash.
        let _ = steam_binary();
    }
}