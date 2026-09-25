//! User configuration for NovaShell, stored as JSON under
//! `~/.config/novashell/config.json`. All paths are resolved via XDG
//! directories at runtime (see `util`); nothing is hard-coded.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::util;

/// Default logical button -> action mapping (SDL/Xbox-style labels).
/// Keys are gilrs logical button names; values are high-level actions.
pub const DEFAULT_CONTROLLER_MAP: &[(&str, &str)] = &[
    ("south", "confirm"),
    ("east", "back"),
    ("west", "context"),
    ("north", "detail"),
    ("start", "menu"),
    ("select", "context"),
    ("l1", "tab_prev"),
    ("r1", "tab_next"),
    ("l2", "quick_menu"),
    ("r2", "quick_menu"),
    ("guide", "screenshot"),
    ("dpad_up", "up"),
    ("dpad_down", "down"),
    ("dpad_left", "left"),
    ("dpad_right", "right"),
];

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ControllerConfig {
    /// Master controller input toggle.
    pub enabled: bool,
    /// Rumble / force-feedback toggle (per-game via Steam/Heroic too).
    pub vibration: bool,
    /// Logical button name -> action name.
    pub mapping: BTreeMap<String, String>,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        ControllerConfig {
            enabled: true,
            vibration: true,
            mapping: DEFAULT_CONTROLLER_MAP
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct LauncherConfig {
    pub steam_enabled: bool,
    pub heroic_enabled: bool,
    pub desktop_enabled: bool,
    /// Optional explicit Steam install path (auto-detected otherwise).
    pub steam_path: Option<PathBuf>,
    /// Optional explicit Heroic binary name/path.
    pub heroic_bin: Option<String>,
    /// Optional explicit legendary binary path.
    pub legendary_path: Option<PathBuf>,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        LauncherConfig {
            steam_enabled: true,
            heroic_enabled: true,
            desktop_enabled: true,
            steam_path: None,
            heroic_bin: None,
            legendary_path: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    /// Schema version. Defaults to 1 so a hand-written or partial config file
    /// still loads instead of being thrown away wholesale.
    #[serde(default = "default_version")]
    pub version: u32,
    /// UI theme id (currently "nova").
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Accent color, CSS hex e.g. "#7c3aed".
    #[serde(default = "default_accent")]
    pub accent: String,
    /// UI scale multiplier (1.0 = 100%).
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f64,
    /// Smooth animations toggle.
    #[serde(default = "default_bool_true")]
    pub animations: bool,
    /// Performance mode: disable costly effects, prioritize low latency.
    #[serde(default = "default_bool_false")]
    pub performance_mode: bool,
    /// Start fullscreen (used by session autostart; off in dev).
    #[serde(default = "default_bool_true")]
    pub fullscreen: bool,
    /// Extra directories scanned for native games (.desktop style entries).
    #[serde(default)]
    pub game_dirs: Vec<PathBuf>,
    /// Additional storage roots for Wine/Proton prefixes, e.g. a games drive.
    /// The default prefix root always applies; these are extra locations.
    #[serde(default)]
    pub prefix_roots: Vec<PathBuf>,
    #[serde(default)]
    pub launchers: LauncherConfig,
    #[serde(default)]
    pub controller: ControllerConfig,
    /// Which distributable powers screenshots (auto / grim / gnome-screenshot / scrot).
    #[serde(default = "default_screenshot_mode")]
    pub screenshot_mode: String,
}

fn default_theme() -> String { "nova".into() }
fn default_version() -> u32 { 1 }
fn default_accent() -> String { "#7c3aed".into() }
fn default_ui_scale() -> f64 { 1.0 }
fn default_bool_true() -> bool { true }
fn default_bool_false() -> bool { false }
fn default_screenshot_mode() -> String { "auto".into() }

impl Default for Config {
    fn default() -> Self {
        Config {
            version: 1,
            theme: default_theme(),
            accent: default_accent(),
            ui_scale: default_ui_scale(),
            animations: default_bool_true(),
            performance_mode: default_bool_false(),
            fullscreen: default_bool_true(),
            game_dirs: vec![],
            prefix_roots: vec![],
            launchers: LauncherConfig::default(),
            controller: ControllerConfig::default(),
            screenshot_mode: default_screenshot_mode(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        util::config_dir().join("config.json")
    }

    /// Load with all-missing fields defaulted (serde `default`).
    pub fn load() -> Config {
        let p = Self::path();
        match std::fs::read_to_string(&p) {
            Ok(text) => match serde_json::from_str::<Config>(&text) {
                Ok(cfg) => cfg,
                Err(err) => {
                    log::warn!(
                        "Config at {} is invalid ({err}); falling back to defaults.",
                        p.display()
                    );
                    Config::default()
                }
            },
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        util::ensure_dir(&util::config_dir())?;
        let p = Self::path();
        // Unique temp name per writer, so two concurrent saves cannot
        // interleave and leave a truncated config behind.
        let tmp = p.with_extension(format!("json.tmp-{}", std::process::id()));
        let json = serde_json::to_string_pretty(self).context("serializing config")?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &p)?;
        Ok(())
    }
}

/// Lightweight in-memory overlay of the config for runtime tweaks that
/// haven't been persisted yet (used by the Settings UI preview).
#[derive(Clone, Debug)]
pub struct ConfigState {
    pub config: Config,
}

impl ConfigState {
    pub fn new() -> Self {
        ConfigState {
            config: Config::load(),
        }
    }

    pub fn apply(self) -> Result<()> {
        self.config.save()
    }
}

impl Default for ConfigState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Config {
        Config::default()
    }

    #[test]
    fn defaults_are_sane() {
        let c = defaults();
        assert_eq!(c.version, 1);
        assert!(!c.accent.is_empty());
        assert!(c.ui_scale >= 0.5 && c.ui_scale <= 3.0);
        assert_eq!(c.controller.mapping.get("south"), Some(&"confirm".to_string()));
        assert!(c.launchers.steam_enabled);
    }

    #[test]
    fn serde_roundtrip() {
        let c = defaults();
        let json = serde_json::to_string(&c).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.accent, c.accent);
        assert_eq!(back.controller.mapping.len(), c.controller.mapping.len());
    }

    #[test]
    fn missing_fields_defaulted() {
        // A minimal JSON document should parse, filling everything with defaults.
        let json = r#"{ "version": 1 }"#;
        let c: Config = serde_json::from_str(json).unwrap();
        assert_eq!(c.theme, "nova");
        assert!(c.animations);
        assert!(c.controller.enabled);
    }

    #[test]
    fn controller_default_map_covers_core_buttons() {
        let c = defaults();
        for k in ["south", "east", "north", "west", "start", "l1", "r1"] {
            assert!(c.controller.mapping.contains_key(k), "missing mapping {k}");
        }
    }
}