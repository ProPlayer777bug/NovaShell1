//! Plugin registry.
//!
//! The built-in providers (Steam / Heroic / desktop entries) implement the
//! [`Provider`] trait. Additional launcher integrations can be added at any
//! time by implementing the same trait and registering the instance here —
//! keeping NovaShell future-proof without baking launcher specifics into the
//! shell code.

use crate::games::Game;
use crate::integrations::Provider;
use crate::settings::Config;

/// All active discovery providers in registration order.
pub fn providers() -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(crate::integrations::steam::SteamProvider),
        Box::new(crate::integrations::heroic::HeroicProvider),
        Box::new(crate::integrations::desktop::DesktopProvider),
    ]
}

/// Run every registered provider, respecting the launcher toggles in config.
pub fn scan(cfg: &Config) -> Vec<Game> {
    let mut games = Vec::new();
    for p in providers() {
        if let Some(enabled) = toggled_on(cfg, p.id()) {
            if !enabled {
                continue;
            }
        }
        if let Ok(logged) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.scan(cfg))) {
            log::debug!("provider {} returned {} games", p.id(), logged.len());
            games.extend(logged);
        } else {
            log::error!("provider {} panicked during scan; skipping", p.id());
        }
    }
    games
}

/// Which config field gates a provider id (None = always enabled).
fn toggled_on(cfg: &Config, id: &str) -> Option<bool> {
    match id {
        "steam" => Some(cfg.launchers.steam_enabled),
        "heroic" => Some(cfg.launchers.heroic_enabled),
        "desktop" => Some(cfg.launchers.desktop_enabled),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_toggles_gate_providers() {
        let mut cfg = Config::default();
        cfg.launchers.steam_enabled = false;
        cfg.launchers.heroic_enabled = true;
        cfg.launchers.desktop_enabled = false;
        assert_eq!(toggled_on(&cfg, "steam"), Some(false));
        assert_eq!(toggled_on(&cfg, "heroic"), Some(true));
        assert_eq!(toggled_on(&cfg, "desktop"), Some(false));
        assert_eq!(toggled_on(&cfg, "future-provider"), None);
    }
}