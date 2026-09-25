//! Proton runtime: Windows games.
//!
//! Several Proton builds can coexist, so this is a *family* of runtimes: one
//! per discovered build (Steam's bundled Proton, Proton-GE, custom paths), all
//! sharing the same launch logic. Nothing is hardcoded — every candidate comes
//! from a search of the usual locations plus user-configured paths.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::launcher::Spec;
use crate::runtime::prefixes::{self, PrefixKind};
use crate::runtime::security;
use crate::runtime::{Capability, Check, LaunchTarget, Runtime, RuntimeInfo, RuntimeKind, RuntimeStatus};

/// Directories that may contain Proton builds, in preference order.
fn proton_search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let home = dirs::home_dir().unwrap_or_default();
    for base in [
        home.join("Steam/steamapps/common"),
        home.join(".steam/steam/steamapps/common"),
        home.join(".local/share/Steam/steamapps/common"),
        home.join("Games"),
        home.join("Proton"),
    ] {
        if base.is_dir() {
            dirs.push(base);
        }
    }
    // Custom locations the user configured.
    if let Ok(cfg) = std::fs::read_to_string(
        crate::util::config_dir().join("proton-paths.json"),
    ) {
        if let Ok(list) = serde_json::from_str::<Vec<String>>(&cfg) {
            for p in list {
                let path = PathBuf::from(p);
                if path.is_dir() {
                    dirs.push(path);
                }
            }
        }
    }
    dirs
}

/// A single Proton build: the `proton` script that fronts the real binary.
#[derive(Clone)]
pub struct ProtonBuild {
    /// Short id used in the UI, e.g. `proton-ge-8-32`.
    pub id: String,
    pub name: String,
    /// Path to the `proton` python entry point.
    pub script: PathBuf,
    /// Where this build came from, for display.
    pub origin: String,
}

impl ProtonBuild {
    /// `proton run <exe>` is the documented entry point.
    fn run_args(&self, exe: &Path, args: &[String]) -> Vec<String> {
        let mut v = vec!["run".to_string(), exe.to_string_lossy().to_string()];
        v.extend(args.iter().cloned());
        v
    }
}

/// Discover every Proton build we can see, without hardcoding versions.
pub fn discover_proton() -> Vec<ProtonBuild> {
    let mut out = Vec::new();
    for dir in proton_search_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            let lower = name.to_lowercase();
            if !(lower.starts_with("proton") || lower.starts_with("steamproton")) {
                continue;
            }
            // Layout: <dir>/proton and <dir>/files/bin/wine
            for candidate in [path.join("proton"), path.join("dist/proton")] {
                if candidate.is_file() {
                    let is_ge = lower.contains("ge");
                    out.push(ProtonBuild {
                        id: format!(
                            "proton-{}-{}",
                            if is_ge { "ge" } else { "steam" },
                            slug(&name)
                        ),
                        name: name.clone(),
                        script: candidate,
                        origin: dir.to_string_lossy().to_string(),
                    });
                    break;
                }
            }
        }
    }
    out
}

fn slug(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect()
}

pub fn proton_runtimes() -> Vec<Box<dyn Runtime>> {
    let builds = discover_proton();
    if builds.is_empty() {
        // Always expose Proton itself so the UI can offer installation.
        return vec![Box::new(ProtonRuntime::missing()) as Box<dyn Runtime>];
    }
    builds
        .into_iter()
        .map(|b| Box::new(ProtonRuntime::found(b)) as Box<dyn Runtime>)
        .collect()
}

pub struct ProtonRuntime {
    build: Option<ProtonBuild>,
    version: Option<String>,
}

impl ProtonRuntime {
    pub fn missing() -> Self {
        ProtonRuntime { build: None, version: None }
    }

    pub fn found(build: ProtonBuild) -> Self {
        ProtonRuntime { build: Some(build), version: None }
    }

    /// Path to the build's `proton` script, if discovered.
    pub fn script(&self) -> Option<&Path> {
        self.build.as_ref().map(|b| b.script.as_path())
    }

    fn probe_version(&self) -> Option<String> {
        let script = self.script()?;
        let child = std::process::Command::new("python3")
            .arg(script)
            .arg("--version")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .ok()?;
        let mut child = child;
        let deadline = std::time::Instant::now() + Duration::from_millis(2000);
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                return status.success().then(|| String::new());
            }
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Runtime for ProtonRuntime {
    fn info(&self) -> RuntimeInfo {
        let (id, name) = match &self.build {
            Some(b) => (b.id.clone(), b.name.clone()),
            None => ("proton".to_string(), "Proton".to_string()),
        };
        let mut info = RuntimeInfo::new(&id, &name, RuntimeKind::WindowsGame)
            .with_types(&["exe"])
            .with_platforms(&["windows"])
            .with_capabilities(&[
                Capability::Launch,
                Capability::Install,
                Capability::Prefixes,
                Capability::Environment,
            ]);
        info.executable = self.script().map(Path::to_path_buf);
        info.version = self.version.clone();
        info.status = if self.build.is_some() {
            RuntimeStatus::Available
        } else {
            RuntimeStatus::Missing
        };
        info
    }

    fn detect(&mut self) -> RuntimeStatus {
        match &self.build {
            Some(b) => {
                if b.script.is_file() {
                    self.version = self.probe_version();
                    RuntimeStatus::Available
                } else {
                    RuntimeStatus::Error(format!("{} is missing", b.script.display()))
                }
            }
            None => RuntimeStatus::Missing,
        }
    }

    fn validate(&self, target: &LaunchTarget) -> Vec<Check> {
        let mut checks = Vec::new();
        match self.script() {
            Some(s) if s.is_file() => checks.push(Check::ok("Runtime")),
            Some(s) => checks.push(Check::fail("Runtime", format!("{} is missing", s.display()))),
            None => checks.push(Check::fail("Runtime", "Proton is not installed")),
        }
        if let Some(exe) = &target.executable {
            checks.push(if exe.is_file() {
                Check::ok("Executable")
            } else {
                Check::fail("Executable", format!("{} not found", exe.display()))
            });
        }
        if let Some(prefix) = &target.prefix {
            checks.push(if prefix.is_dir() {
                Check::ok("Prefix")
            } else {
                Check::fail("Prefix", "prefix does not exist")
            });
        }
        checks
    }

    fn build_spec(&self, target: &LaunchTarget) -> anyhow::Result<Spec> {
        let script = self
            .script()
            .ok_or_else(|| anyhow::anyhow!("Proton is not installed"))?
            .to_path_buf();
        let exe = target
            .executable
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no game executable selected"))?;
        if !exe.is_file() {
            anyhow::bail!("game not found: {}", exe.display());
        }
        security::validate_args(&target.args)?;
        let build = self
            .build
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no Proton build"))?;
        let args = build.run_args(exe, &target.args);
        let mut spec = Spec::new(&target.title, "python3").arg(script.to_string_lossy().to_string());
        spec = spec.args(args);
        if let Some(prefix) = &target.prefix {
            spec = spec.env("STEAM_COMPAT_DATA_PATH", prefix.to_string_lossy().to_string());
            spec = spec.env("WINEPREFIX", prefix.join("pfx").to_string_lossy().to_string());
        }
        for (k, v) in security::filter_env(&target.env) {
            spec = spec.env(k, v);
        }
        if let Some(cwd) = &target.working_dir {
            spec = spec.cwd(cwd.clone());
        }
        Ok(spec)
    }
}

impl ProtonRuntime {
    pub fn ensure_prefix(&self, app_id: &str) -> anyhow::Result<PathBuf> {
        Ok(prefixes::create(PrefixKind::Proton, app_id)?.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_proton_reports_missing() {
        let mut rt = ProtonRuntime::missing();
        assert_eq!(rt.detect(), RuntimeStatus::Missing);
        assert_eq!(rt.info().kind, RuntimeKind::WindowsGame);
    }

    #[test]
    fn registry_always_exposes_proton() {
        let rs = proton_runtimes();
        assert!(!rs.is_empty());
        assert!(rs.iter().any(|r| r.info().kind == RuntimeKind::WindowsGame));
    }

    #[test]
    fn build_spec_fails_without_proton() {
        let rt = ProtonRuntime::missing();
        let t = LaunchTarget::new("g", "G").with_executable("/bin/sh");
        assert!(rt.build_spec(&t).is_err());
    }

    #[test]
    fn slug_is_path_safe() {
        let s = slug("Proton - GE 8.32");
        assert!(s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }
}
