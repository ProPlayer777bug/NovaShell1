//! Launcher integrations. Each provider discovers locally installed games and
//! returns library candidates. The shell merges them into the persistent
//! library (preserving favorites/playtime). `plugins` orchestrates the scan.

pub mod desktop;
pub mod heroic;
pub mod steam;

use crate::games::Game;
use crate::settings::Config;

/// A game discovery source. Additional providers can be registered by
/// implementing this trait (see `plugins`).
pub trait Provider: Send + Sync {
    /// Stable provider id, e.g. `"steam"`.
    fn id(&self) -> &'static str;
    /// Discovery logic. Must be cheap enough to run on the UI thread during a
    /// manual refresh, or delegate heavy work to a thread.
    fn scan(&self, cfg: &Config) -> Vec<Game>;
}