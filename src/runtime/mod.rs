//! Nova Runtime Manager — core abstraction.
//!
//! A *runtime* knows how to turn a [`LaunchTarget`] into a
//! [`crate::launcher::Spec`]. Runtimes never spawn processes themselves: the
//! shell's process manager owns supervision, so a runtime only has to describe
//! the command, its arguments, working directory and environment.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::launcher::Spec;

pub mod apps;
pub mod decisions;
pub mod emulator;
pub mod logs;
pub mod native;
pub mod prefixes;
pub mod procs;
pub mod security;

#[cfg(target_os = "linux")]
pub mod proton;
#[cfg(target_os = "linux")]
pub mod wine;

/// Broad category of a runtime. Serialised for the UI as a lowercase string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeKind {
    /// A plain Linux program.
    Native,
    /// A console emulator driven through the ROM picker.
    Emulator,
    /// A Windows application run through Wine.
    WindowsApp,
    /// A Windows game run through Proton.
    WindowsGame,
    /// Anything added later through the plugin path.
    Plugin,
}

impl RuntimeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeKind::Native => "native",
            RuntimeKind::Emulator => "emulator",
            RuntimeKind::WindowsApp => "windows-app",
            RuntimeKind::WindowsGame => "windows-game",
            RuntimeKind::Plugin => "plugin",
        }
    }
}

/// Optional behaviours a runtime can support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    Launch,
    Install,
    Prefixes,
    Uninstall,
    Backup,
    Environment,
}

/// Availability of a runtime on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "detail", rename_all = "kebab-case")]
pub enum RuntimeStatus {
    /// Present and usable.
    Available,
    /// Not installed (and possibly installable).
    Missing,
    /// Installed but broken (bad version, missing dependency, …).
    Error(String),
    /// Detection has not run yet.
    Unknown,
}

impl RuntimeStatus {
    pub fn is_available(&self) -> bool {
        matches!(self, RuntimeStatus::Available)
    }
}

/// Everything the UI needs to render a runtime row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInfo {
    pub id: String,
    pub name: String,
    pub kind: RuntimeKind,
    pub version: Option<String>,
    pub executable: Option<PathBuf>,
    pub capabilities: Vec<Capability>,
    /// Lower-case extensions without a dot, e.g. `exe`, `msi`.
    pub supported_file_types: Vec<String>,
    /// `linux`, `windows`, or a console name such as `PS2`.
    pub supported_platforms: Vec<String>,
    pub status: RuntimeStatus,
    pub config: BTreeMap<String, String>,
}

impl RuntimeInfo {
    pub fn new(id: &str, name: &str, kind: RuntimeKind) -> Self {
        RuntimeInfo {
            id: id.to_string(),
            name: name.to_string(),
            kind,
            version: None,
            executable: None,
            capabilities: vec![Capability::Launch],
            supported_file_types: Vec::new(),
            supported_platforms: vec!["linux".to_string()],
            status: RuntimeStatus::Unknown,
            config: BTreeMap::new(),
        }
    }

    pub fn with_types(mut self, exts: &[&str]) -> Self {
        self.supported_file_types = exts.iter().map(|e| e.to_lowercase()).collect();
        self
    }

    pub fn with_platforms(mut self, platforms: &[&str]) -> Self {
        self.supported_platforms = platforms.iter().map(|p| p.to_string()).collect();
        self
    }

    pub fn with_capabilities(mut self, caps: &[Capability]) -> Self {
        self.capabilities = caps.to_vec();
        self
    }
}

/// One pre-flight check result shown as a ✓/✗ line before launching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub label: String,
    pub ok: bool,
    pub detail: Option<String>,
}

impl Check {
    pub fn ok(label: &str) -> Self {
        Check { label: label.to_string(), ok: true, detail: None }
    }
    pub fn fail(label: &str, detail: impl Into<String>) -> Self {
        Check {
            label: label.to_string(),
            ok: false,
            detail: Some(detail.into()),
        }
    }
}

/// A request to run something, independent of which runtime will handle it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LaunchTarget {
    /// Library id this launch belongs to (for process tracking and playtime).
    pub app_id: String,
    /// Display name.
    pub title: String,
    /// Path to the executable inside the prefix or on disk.
    pub executable: Option<PathBuf>,
    /// Working directory.
    pub working_dir: Option<PathBuf>,
    /// Extra arguments, passed as an array (never a shell string).
    pub args: Vec<String>,
    /// Explicit environment overrides (allowlisted in `security`).
    pub env: BTreeMap<String, String>,
    /// Prefix directory for Wine/Proton.
    pub prefix: Option<PathBuf>,
    /// Runtime version/id chosen by the user, overriding automatic selection.
    pub runtime_override: Option<String>,
    /// The user marked this as a game (drives Wine vs Proton selection).
    pub is_game: bool,
}

impl LaunchTarget {
    pub fn new(app_id: &str, title: &str) -> Self {
        LaunchTarget {
            app_id: app_id.to_string(),
            title: title.to_string(),
            ..Default::default()
        }
    }

    pub fn with_executable(mut self, path: impl Into<PathBuf>) -> Self {
        self.executable = Some(path.into());
        self
    }

    pub fn with_game(mut self, is_game: bool) -> Self {
        self.is_game = is_game;
        self
    }
}

/// A runtime that can produce a launch specification.
pub trait Runtime {
    /// Static description; `status` is refreshed by `detect`.
    fn info(&self) -> RuntimeInfo;

    /// Probe the machine for this runtime.
    fn detect(&mut self) -> RuntimeStatus;

    /// Lightweight checks to run before launching.
    fn validate(&self, _target: &LaunchTarget) -> Vec<Check> {
        let info = self.info();
        if info.status.is_available() {
            vec![Check::ok("Runtime")]
        } else {
            vec![Check::fail("Runtime", "not installed")]
        }
    }

    /// Build the process specification. Must not spawn anything.
    fn build_spec(&self, target: &LaunchTarget) -> anyhow::Result<Spec>;
}

/// Every runtime the shell knows about, in display order.
pub fn registry() -> Vec<Box<dyn Runtime>> {
    let mut list: Vec<Box<dyn Runtime>> = Vec::new();
    list.push(Box::new(native::NativeRuntime::new()));
    list.extend(emulator::emulator_runtimes());
    #[cfg(target_os = "linux")]
    {
        list.push(Box::new(wine::WineRuntime::new()));
        list.extend(proton::proton_runtimes());
    }
    list
}

/// Detect every runtime once and return their current state.
pub fn detect_all() -> Vec<RuntimeInfo> {
    registry()
        .into_iter()
        .map(|mut r| {
            let status = r.detect();
            let mut info = r.info();
            info.status = status;
            info
        })
        .collect()
}

/// Pre-flight checks for one runtime id, detecting it first. Used by the
/// bridge so the UI never has to hold a runtime handle.
pub fn validate_runtime(runtime_id: &str, target: &LaunchTarget) -> Vec<Check> {
    for mut r in registry() {
        if r.info().id == runtime_id {
            r.detect();
            return r.validate(target);
        }
    }
    vec![Check::fail("Runtime", "unknown runtime")]
}

/// Pick the runtime that should handle a target, honouring an explicit
/// override. Games prefer Proton, normal Windows applications prefer Wine.
pub fn select<'a>(runtimes: &'a [Box<dyn Runtime>], target: &LaunchTarget) -> Option<&'a dyn Runtime> {
    if let Some(want) = &target.runtime_override {
        return runtimes.iter().find(|r| &r.info().id == want).map(|r| r.as_ref());
    }
    let by_kind = |kind: RuntimeKind| -> Option<&dyn Runtime> {
        runtimes
            .iter()
            .find(|r| r.info().kind == kind && r.info().status.is_available())
            .map(|r| r.as_ref())
    };
    if target.is_game {
        by_kind(RuntimeKind::WindowsGame).or_else(|| by_kind(RuntimeKind::WindowsApp))
    } else {
        by_kind(RuntimeKind::WindowsApp).or_else(|| by_kind(RuntimeKind::WindowsGame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        id: &'static str,
        kind: RuntimeKind,
        available: bool,
    }

    impl Runtime for Fake {
        fn info(&self) -> RuntimeInfo {
            let mut i = RuntimeInfo::new(self.id, self.id, self.kind);
            i.status = if self.available {
                RuntimeStatus::Available
            } else {
                RuntimeStatus::Missing
            };
            i
        }
        fn detect(&mut self) -> RuntimeStatus {
            self.info().status
        }
        fn build_spec(&self, _t: &LaunchTarget) -> anyhow::Result<Spec> {
            Ok(Spec::new(self.id, self.id))
        }
    }

    fn boxed(v: Vec<Fake>) -> Vec<Box<dyn Runtime>> {
        v.into_iter()
            .map(|f| Box::new(f) as Box<dyn Runtime>)
            .collect()
    }

    #[test]
    fn windows_app_prefers_wine_over_proton() {
        let rs = boxed(vec![
            Fake { id: "wine", kind: RuntimeKind::WindowsApp, available: true },
            Fake { id: "proton", kind: RuntimeKind::WindowsGame, available: true },
        ]);
        let t = LaunchTarget::new("app-1", "Notepad");
        assert_eq!(select(&rs, &t).unwrap().info().id, "wine");
    }

    #[test]
    fn game_prefers_proton_over_wine() {
        let rs = boxed(vec![
            Fake { id: "wine", kind: RuntimeKind::WindowsApp, available: true },
            Fake { id: "proton", kind: RuntimeKind::WindowsGame, available: true },
        ]);
        let t = LaunchTarget::new("game-1", "My Game").with_game(true);
        assert_eq!(select(&rs, &t).unwrap().info().id, "proton");
    }

    #[test]
    fn unavailable_runtimes_are_not_auto_selected() {
        let rs = boxed(vec![
            Fake { id: "proton", kind: RuntimeKind::WindowsGame, available: false },
        ]);
        let t = LaunchTarget::new("game-1", "My Game").with_game(true);
        assert!(select(&rs, &t).is_none());
    }

    #[test]
    fn explicit_override_beats_automatic_choice() {
        let rs = boxed(vec![
            Fake { id: "wine", kind: RuntimeKind::WindowsApp, available: true },
            Fake { id: "proton", kind: RuntimeKind::WindowsGame, available: true },
        ]);
        let mut t = LaunchTarget::new("game-1", "My Game").with_game(true);
        t.runtime_override = Some("wine".into());
        assert_eq!(select(&rs, &t).unwrap().info().id, "wine");
    }

    #[test]
    fn override_for_unknown_runtime_selects_nothing() {
        let rs = boxed(vec![Fake { id: "wine", kind: RuntimeKind::WindowsApp, available: true }]);
        let mut t = LaunchTarget::new("a", "A");
        t.runtime_override = Some("does-not-exist".into());
        assert!(select(&rs, &t).is_none());
    }
}
