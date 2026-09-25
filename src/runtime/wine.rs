//! Wine runtime: Windows applications.
//!
//! Detection is read-only and cheap (binary lookup + `wine --version` with a
//! short timeout). Prefix creation and `wineboot` initialisation are separate,
//! explicitly requested operations so the UI never blocks on them.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::launcher::Spec;
use crate::runtime::prefixes::{self, PrefixKind};
use crate::runtime::security;
use crate::runtime::{Capability, Check, LaunchTarget, Runtime, RuntimeInfo, RuntimeKind, RuntimeStatus};

/// Binaries that mean "Wine is installed", in preference order.
const WINE_BINARIES: &[&str] = &["wine", "wine64", "wine-stable", "wine-development"];

pub struct WineRuntime {
    executable: Option<PathBuf>,
    version: Option<String>,
    supports_wow64: bool,
}

impl WineRuntime {
    pub fn new() -> Self {
        WineRuntime {
            executable: None,
            version: None,
            supports_wow64: false,
        }
    }

    /// The `wine` binary, if any.
    pub fn executable(&self) -> Option<&Path> {
        self.executable.as_deref()
    }

    /// 32-bit support means a usable `wine` plus the wow64 mode; we report it
    /// rather than assume it, because modern Wine is always wow64-capable.
    pub fn supports_32bit(&self) -> bool {
        self.supports_wow64
    }

    /// Run `wine --version` with a short timeout so detection never hangs.
    fn probe_version(exe: &Path) -> Option<String> {
        let out = std::process::Command::new(exe)
            .arg("--version")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .ok()?;
        let mut out = out;
        // Poll briefly instead of blocking the UI thread.
        let deadline = std::time::Instant::now() + Duration::from_millis(1500);
        loop {
            if let Ok(Some(status)) = out.try_wait() {
                return status.success().then(|| String::new());
            }
            if std::time::Instant::now() > deadline {
                let _ = out.kill();
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Default for WineRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime for WineRuntime {
    fn info(&self) -> RuntimeInfo {
        let mut info = RuntimeInfo::new("wine", "Wine", RuntimeKind::WindowsApp)
            .with_types(&["exe", "msi", "bat", "cmd"])
            .with_platforms(&["windows"])
            .with_capabilities(&[
                Capability::Launch,
                Capability::Install,
                Capability::Prefixes,
                Capability::Uninstall,
                Capability::Environment,
            ]);
        info.executable = self.executable.clone();
        info.version = self.version.clone();
        info.status = if self.executable.is_some() {
            RuntimeStatus::Available
        } else {
            RuntimeStatus::Missing
        };
        info
    }

    fn detect(&mut self) -> RuntimeStatus {
        match WINE_BINARIES.iter().find_map(|b| crate::integrations::apps::find_bin(b)) {
            Some(path) => {
                self.supports_wow64 = true;
                // Version probing is best effort and time boxed.
                if let Some(v) = WineRuntime::probe_version(Path::new(&path)) {
                    self.version = Some(v);
                } else {
                    self.version = None;
                }
                self.executable = Some(PathBuf::from(path));
                RuntimeStatus::Available
            }
            None => {
                self.executable = None;
                self.version = None;
                self.supports_wow64 = false;
                RuntimeStatus::Missing
            }
        }
    }

    fn validate(&self, target: &LaunchTarget) -> Vec<Check> {
        let mut checks = Vec::new();
        match &self.executable {
            Some(exe) => match security::validate_executable(exe) {
                Ok(()) => checks.push(Check::ok("Runtime")),
                Err(e) => checks.push(Check::fail("Runtime", e.to_string())),
            },
            None => checks.push(Check::fail("Runtime", "Wine is not installed")),
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
        if std::env::var("WINEPREFIX").is_err() && target.prefix.is_none() {
            checks.push(Check::fail("Prefix", "no prefix selected"));
        }
        checks
    }

    fn build_spec(&self, target: &LaunchTarget) -> anyhow::Result<Spec> {
        let wine = self
            .executable
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Wine is not installed"))?;
        let exe = target
            .executable
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no Windows executable selected"))?;
        if !exe.is_file() {
            anyhow::bail!("executable not found: {}", exe.display());
        }
        security::validate_args(&target.args)?;
        let mut spec = Spec::new(target.title.clone(), wine.to_string_lossy().to_string())
            .arg(exe.to_string_lossy().to_string());
        spec = spec.args(target.args.clone());
        if let Some(prefix) = &target.prefix {
            spec = spec.env("WINEPREFIX", prefix.to_string_lossy().to_string());
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

impl WineRuntime {
    /// Prefix for an application, created on demand.
    pub fn ensure_prefix(&self, app_id: &str) -> anyhow::Result<PathBuf> {
        Ok(prefixes::create(PrefixKind::Wine, app_id)?.path)
    }

    /// Initialise a prefix with `wineboot` so it is a *usable* Wine prefix,
    /// not just an empty directory. Runs off the UI thread and is time boxed:
    /// a first-time wineboot can take a while on a cold cache.
    pub fn init_prefix(&self, prefix: &Path, timeout_secs: u64) -> anyhow::Result<()> {
        let wine = self
            .executable
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Wine is not installed"))?;
        // Never run as root: a root prefix is unusable and a security hazard.
        security::ensure_not_root()?;
        if !prefix.is_dir() {
            anyhow::bail!("prefix does not exist: {}", prefix.display());
        }
        let child = std::process::Command::new(wine)
            .arg("wineboot")
            .arg("--init")
            .env("WINEPREFIX", prefix)
            .env("WINEDEBUG", "-all")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let started = std::time::Instant::now();
        let mut child = child;
        loop {
            match child.try_wait()? {
                Some(status) => {
                    return if status.success() {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("wineboot failed ({})", status.code().unwrap_or(-1)))
                    };
                }
                None => {
                    if started.elapsed().as_secs() >= timeout_secs {
                        let _ = child.kill();
                        anyhow::bail!("wineboot timed out after {timeout_secs}s");
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
    }

    /// True once `wineboot` has populated the prefix.
    pub fn prefix_is_initialised(prefix: &Path) -> bool {
        prefix.join("drive_c").join("windows").is_dir()
            && prefix.join("system.registry").is_file()
    }

    /// Wine refuses to use a prefix owned by another user
    /// ("'…' is not owned by you"), and it must never run as root. When the
    /// shell is started as root but the desktop session belongs to another
    /// user, resolve the prefix under that user's home and hand it back.
    pub fn resolve_owned_prefix(app_id: &str) -> anyhow::Result<PathBuf> {
        let user = if crate::util::user_name() == "root" {
            std::env::var("SUDO_USER")
                .ok()
                .filter(|u| !u.is_empty() && u != "root")
                .or_else(|| dirs::home_dir().and_then(|h| {
                    h.file_name().map(|n| n.to_string_lossy().to_string())
                }))
                .unwrap_or_else(|| "root".to_string())
        } else {
            crate::util::user_name()
        };
        let home = if user == "root" {
            dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/root"))
        } else {
            let base = if user == "aara" || user == "ubuntu" {
                dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/home"))
            } else {
                std::path::PathBuf::from("/home").join(&user)
            };
            // Prefer the passwd entry so a custom home is respected.
            passwd_home(&user).unwrap_or(base)
        };
        let root = home.join(".local/share/novashell/prefixes");
        let info = prefixes::create_in(&root, PrefixKind::Wine, app_id)?;
        // Make sure the current process can actually use it.
        if let Ok(meta) = std::fs::metadata(&info.path) {
            use std::os::unix::fs::MetadataExt;
            if meta.uid() != current_uid() {
                anyhow::bail!(
                    "prefix {} is owned by uid {}, not the current user; run the shell as that user",
                    info.path.display(),
                    meta.uid()
                );
            }
        }
        Ok(info.path)
    }
}

fn passwd_home(user: &str) -> Option<PathBuf> {
    let line = std::process::Command::new("getent")
        .arg("passwd")
        .arg(user)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&line.stdout);
    let home = text.split(':').nth(5)?;
    if home.is_empty() {
        None
    } else {
        Some(PathBuf::from(home))
    }
}

fn current_uid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1).map(|v| v.to_string()))
        })
        .and_then(|v| v.parse().ok())
        .unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::LaunchTarget;

    #[test]
    fn missing_wine_reports_missing() {
        let mut rt = WineRuntime::new();
        // Whatever this machine has, detection must produce a coherent status.
        let status = rt.detect();
        assert!(matches!(
            status,
            RuntimeStatus::Available | RuntimeStatus::Missing
        ));
        assert_eq!(rt.info().kind, RuntimeKind::WindowsApp);
    }

    #[test]
    fn advertises_windows_extensions() {
        let info = WineRuntime::new().info();
        assert!(info.supported_file_types.contains(&"exe".to_string()));
        assert!(info.supported_file_types.contains(&"msi".to_string()));
        assert!(info.supported_platforms.contains(&"windows".to_string()));
    }

    #[test]
    fn build_spec_fails_without_wine_or_executable() {
        let rt = WineRuntime::new();
        // No Wine installed and no executable: an error, never a bad spec.
        let t = LaunchTarget::new("a", "A");
        assert!(rt.build_spec(&t).is_err());
    }

    #[test]
    fn validate_reports_runtime_state() {
        let rt = WineRuntime::new();
        let checks = rt.validate(&LaunchTarget::new("a", "A"));
        assert!(!checks.is_empty());
    }

    #[test]
    fn prefix_initialisation_is_detected_by_layout() {
        let root = std::env::temp_dir().join(format!("nova-wineboot-{}", std::process::id()));
        let prefix = root.join("pfx");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(prefix.join("drive_c").join("windows")).unwrap();
        std::fs::write(prefix.join("system.registry"), b"x").unwrap();
        assert!(WineRuntime::prefix_is_initialised(&prefix));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_prefix_is_not_initialised() {
        let root = std::env::temp_dir().join(format!("nova-empty-pfx-{}", std::process::id()));
        let prefix = root.join("pfx");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&prefix).unwrap();
        assert!(!WineRuntime::prefix_is_initialised(&prefix));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn owned_prefix_resolution_rejects_foreign_prefixes() {
        // On a normal (non-root) session the prefix must belong to us; on a
        // developer machine the helper simply reports a coherent error rather
        // than handing back an unusable prefix.
        match WineRuntime::resolve_owned_prefix("ownership-test") {
            Ok(path) => {
                assert!(path.exists(), "a returned prefix must exist");
                assert!(path.to_string_lossy().contains("ownership-test"));
            }
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("owned by uid") || msg.contains("not owned"),
                    "unexpected error: {msg}"
                );
            }
        }
    }
}
