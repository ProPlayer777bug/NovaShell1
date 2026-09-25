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
        Box::new(crate::integrations::apps::AppsProvider),
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
    dedupe_candidates(games)
}

/// Remove entries that would launch the very same program twice.
///
/// Providers are registered curated-first, so the curated tile wins and a
/// desktop entry for the same binary is dropped. Without this, a program that
/// ships two `.desktop` files (Brave ships `brave-browser.desktop` and
/// `com.brave.Browser.desktop`) shows up twice under two different names.
pub fn dedupe_candidates(games: Vec<Game>) -> Vec<Game> {
    use crate::games::Launch;
    use std::collections::HashSet;

    let mut seen_programs: HashSet<String> = HashSet::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(games.len());
    for g in games {
        if !seen_ids.insert(g.id.clone()) {
            continue;
        }
        if let Launch::Program { program, args } = &g.launch {
            // The key includes the arguments: "same executable" is not the same
            // launch. `chromium --profile-directory=Work` and
            // `chromium --profile-directory=Gaming` are different apps, and
            // collapsing them deleted a real entry.
            let key = std::fs::canonicalize(program)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| program.clone());
            let key = format!("{key}\u{1}{}", args.join("\u{1}"));
            if !key.is_empty() && !seen_programs.insert(key) {
                log::debug!("dropping duplicate entry {} ({})", g.id, program);
                continue;
            }
        }
        out.push(g);
    }
    out
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

    fn prog(id: &str, program: &str) -> Game {
        Game {
            id: id.into(),
            title: id.into(),
            source: "test".into(),
            launch: crate::games::Launch::Program {
                program: program.into(),
                args: vec![],
            },
            icon: None,
            artwork: None,
            last_played: None,
            playtime_secs: 0,
            favorite: false,
            installed: true,
            platform: None,
            rom_exts: vec![],
            rom_dir: None,
        }
    }

    #[test]
    fn duplicate_programs_collapse_to_the_first_entry() {
        // Brave ships two desktop files for one binary; only one tile may show.
        let out = dedupe_candidates(vec![
            prog("app-brave", "/usr/bin/brave-browser"),
            prog("desktop-brave-browser.desktop", "/usr/bin/brave-browser"),
            prog("desktop-com.brave.Browser.desktop", "/usr/bin/brave-browser"),
            prog("app-chromium", "/usr/bin/chromium"),
        ]);
        let ids: Vec<&str> = out.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, vec!["app-brave", "app-chromium"]);
    }

    #[test]
    fn dedupe_keeps_distinct_programs_and_non_program_launches() {
        let mut steam = prog("steam-570", "steam");
        steam.launch = crate::games::Launch::Steam { app_id: "570".into() };
        let mut same_steam = prog("heroic-dota", "steam");
        same_steam.launch = crate::games::Launch::Steam { app_id: "570".into() };
        let out = dedupe_candidates(vec![
            prog("a", "/usr/bin/one"),
            prog("b", "/usr/bin/two"),
            steam,
            same_steam,
        ]);
        // Steam/heroic entries are keyed by app id, not program path, so they
        // are left alone: they are different launchers for the same game.
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn same_executable_with_different_args_is_not_a_duplicate() {
        // Two Chromium profiles are two different things to launch.
        let mut work = prog("desktop-chrome-work", "/usr/bin/chromium");
        if let crate::games::Launch::Program { args, .. } = &mut work.launch {
            *args = vec!["--profile-directory=Work".into()];
        }
        let mut gaming = prog("desktop-chrome-gaming", "/usr/bin/chromium");
        if let crate::games::Launch::Program { args, .. } = &mut gaming.launch {
            *args = vec!["--profile-directory=Gaming".into()];
        }
        let out = dedupe_candidates(vec![work, gaming]);
        assert_eq!(out.len(), 2, "different arguments are different launches");
    }

    #[test]
    fn duplicate_ids_collapse_regardless_of_launch_kind() {
        let mut a = prog("same", "/usr/bin/one");
        a.launch = crate::games::Launch::Steam { app_id: "1".into() };
        let mut b = prog("same", "/usr/bin/two");
        b.title = "second".into();
        let out = dedupe_candidates(vec![a, b]);
        assert_eq!(out.len(), 1);
    }
}