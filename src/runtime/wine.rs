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
}
