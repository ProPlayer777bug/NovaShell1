//! Heroic Games Launcher integration.
//!
//! Heroic stores Epic/GOG install info in a legendary-format
//! `installed.json`. We parse it and launch through `legendary` (when
//! available) or `heroic -l <app>`.

use crate::games::{Game, Launch};
use crate::integrations::Provider;
use crate::settings::Config;
use std::path::PathBuf;

#[derive(serde::Deserialize, Debug, Clone)]
struct InstalledEntry {
    #[serde(default)]
    app_name: String,
    #[serde(default)]
    title: String,
    // Consumed by serde for parsing; not surfaced in the UI yet.
    #[serde(default)]
    #[allow(dead_code)]
    install_path: String,
    #[serde(default)]
    is_dlc: bool,
    #[serde(default)]
    #[allow(dead_code)]
    version: String,
    #[serde(default)]
    timestamp: Option<String>,
}

/// Locate Heroic's legendary config dir.
pub fn legendary_config_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let cands = [
        home.join(".config/heroic/legendaryConfig/legendary"),
        home.join(".config/legendary"),
        home.join(".cache/legendary"),
    ];
    cands.into_iter().find(|p| p.is_dir())
}

/// Parse `installed.json` under the legendary config dir.
pub fn scan_heroic_games(cfg: &Config) -> Vec<Game> {
    if !cfg.launchers.heroic_enabled {
        return vec![];
    }
    let Some(dir) = legendary_config_dir() else {
        return vec![];
    };
    let path = dir.join("installed.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return vec![];
    };
    let entries: Vec<InstalledEntry> = match serde_json::from_str(&text) {
        Ok(e) => e,
        Err(_) => return vec![], // maybe a single object or unsupported format
    };

    entries
        .into_iter()
        .filter(|e| !e.app_name.is_empty())
        .filter(|e| !e.is_dlc)
        .map(|e| {
            let id = format!("heroic-{}", sanitize(&e.app_name));
            let title = if e.title.trim().is_empty() {
                humanize(&e.app_name)
            } else {
                e.title.clone()
            };
            let last_played = e
                .timestamp
                .as_ref()
                .and_then(|t| t.parse::<i64>().ok());
            Game {
                id,
                title,
                source: "heroic".into(),
                launch: Launch::Heroic {
                    app_name: app_name_for_launch(&e.app_name),
                },
                icon: None,
                artwork: None,
                last_played,
                playtime_secs: 0,
                favorite: false,
                installed: true,
                platform: None,
                rom_exts: Vec::new(),
            rom_dir: None,
            }
        })
        .collect()
}

/// Heroic app names sometimes include spaces; `legendary launch` expects the
/// literal app_name as recorded.
fn app_name_for_launch(name: &str) -> String {
    name.trim().to_string()
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect()
}

fn humanize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c == '_' || c == '-' { ' ' } else { c })
        .collect();
    let chars: Vec<char> = cleaned.chars().collect();
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for &c in &chars {
        if c == ' ' {
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
            prev = Some(c);
            continue;
        }
        if let Some(p) = prev {
            if p != ' ' {
                let lower_to_upper = p.is_ascii_lowercase() && c.is_ascii_uppercase();
                let digit_to_letter = p.is_ascii_digit() && c.is_ascii_alphabetic();
                if (lower_to_upper || digit_to_letter) && !out.ends_with(' ') {
                    out.push(' ');
                }
            }
        }
        out.push(c);
        prev = Some(c);
    }
    out.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub struct HeroicProvider;

impl Provider for HeroicProvider {
    fn id(&self) -> &'static str {
        "heroic"
    }
    fn scan(&self, cfg: &Config) -> Vec<Game> {
        scan_heroic_games(cfg)
    }
}

/// Best launch command for a Heroic app: `legendary launch` or `heroic -l`.
#[allow(dead_code)]
pub fn heroic_prefix(cfg: &Config) -> Vec<String> {
    if let Some(bin) = &cfg.launchers.heroic_bin {
        vec![bin.clone(), "-l".to_string()]
    } else {
        vec!["legendary".into(), "launch".into()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanize_names() {
        assert_eq!(humanize("LambdaRule"), "Lambda Rule");
        assert_eq!(humanize("cyberpunk_2077"), "Cyberpunk 2077");
    }

    #[test]
    fn parses_legendary_installed_json() {
        let json = r#"[
  {
    "app_name": "LambdaRule",
    "title": "Bioshock Infinite Complete Edition",
    "install_path": "/home/u/Games/BSI",
    "is_dlc": false,
    "version": "1.0.0"
  },
  {
    "app_name": "something-dlc",
    "title": "A DLC",
    "is_dlc": true,
    "version": "1.0.0"
  }
]"#;
        let entries: Vec<InstalledEntry> = serde_json::from_str(json).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(!entries[0].is_dlc);
        assert_eq!(entries[0].title, "Bioshock Infinite Complete Edition");
    }
}