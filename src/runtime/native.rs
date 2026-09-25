//! Native Linux runtime: runs a program directly.
//!
//! This is the adapter for the shell's existing `Launch::Program` path. It is
//! deliberately transparent so the current behaviour (including the console
//! polish flags and ROM arguments) is unchanged — it only expresses the launch
//! as a runtime.

use std::collections::BTreeMap;

use crate::launcher::Spec;
use crate::runtime::{Check, LaunchTarget, Runtime, RuntimeInfo, RuntimeKind, RuntimeStatus};

pub struct NativeRuntime {
    /// Program to run when no executable is supplied by the target.
    default_program: Option<String>,
    /// Display name for the UI row.
    label: String,
    id: String,
}

impl NativeRuntime {
    pub fn new() -> Self {
        NativeRuntime {
            default_program: None,
            label: "Native application".to_string(),
            id: "native".to_string(),
        }
    }

    /// A named native runtime, e.g. `native-brave` for a browser tile.
    pub fn named(id: &str, label: &str, program: &str) -> Self {
        NativeRuntime {
            default_program: Some(program.to_string()),
            label: label.to_string(),
            id: id.to_string(),
        }
    }
}

impl Default for NativeRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime for NativeRuntime {
    fn info(&self) -> RuntimeInfo {
        let mut info = RuntimeInfo::new(&self.id, &self.label, RuntimeKind::Native)
            .with_capabilities(&[crate::runtime::Capability::Launch]);
        info.executable = self.default_program.as_ref().map(|p| p.into());
        if self.default_program.is_some() {
            info.status = RuntimeStatus::Available;
        }
        info
    }

    fn detect(&mut self) -> RuntimeStatus {
        match &self.default_program {
            Some(p) => {
                if crate::integrations::apps::find_bin(p).is_some() {
                    RuntimeStatus::Available
                } else {
                    RuntimeStatus::Missing
                }
            }
            // The generic runtime has nothing specific to detect.
            None => RuntimeStatus::Available,
        }
    }

    fn validate(&self, target: &LaunchTarget) -> Vec<Check> {
        let mut checks = Vec::new();
        let program = target
            .executable
            .clone()
            .map(|p| p.to_string_lossy().to_string())
            .or_else(|| self.default_program.clone());
        match program {
            Some(p) => match crate::integrations::apps::find_bin(&p) {
                Some(found) => {
                    if let Err(e) = crate::runtime::security::validate_executable(std::path::Path::new(&found)) {
                        checks.push(Check::fail("Executable", e.to_string()));
                    } else {
                        checks.push(Check::ok("Executable"));
                    }
                }
                None => checks.push(Check::fail("Runtime", format!("{p} not found"))),
            },
            None => checks.push(Check::fail("Executable", "no program configured")),
        }
        checks
    }

    fn build_spec(&self, target: &LaunchTarget) -> anyhow::Result<Spec> {
        let program = target
            .executable
            .clone()
            .map(|p| p.to_string_lossy().to_string())
            .or_else(|| self.default_program.clone())
            .ok_or_else(|| anyhow::anyhow!("no program to launch"))?;
        crate::runtime::security::validate_args(&target.args)?;
        let mut spec = Spec::new(&target.title, &program).args(target.args.clone());
        if let Some(cwd) = &target.working_dir {
            spec = spec.cwd(cwd.clone());
        }
        let env: BTreeMap<String, String> = crate::runtime::security::filter_env(&target.env);
        for (k, v) in env {
            spec = spec.env(k, v);
        }
        Ok(spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_spec_uses_target_program() {
        let rt = NativeRuntime::new();
        let t = LaunchTarget::new("a", "A").with_executable("/bin/echo");
        let spec = rt.build_spec(&t).unwrap();
        assert_eq!(spec.program, "/bin/echo");
        assert_eq!(spec.name, "A");
    }

    #[test]
    fn build_spec_fails_without_program() {
        let rt = NativeRuntime::new();
        let t = LaunchTarget::new("a", "A");
        assert!(rt.build_spec(&t).is_err());
    }

    #[test]
    fn named_runtime_uses_its_default() {
        let rt = NativeRuntime::named("native-sh", "Shell", "/bin/sh");
        let t = LaunchTarget::new("a", "A");
        let spec = rt.build_spec(&t).unwrap();
        assert_eq!(spec.program, "/bin/sh");
        // Availability depends on the host (there is no /bin/sh on Windows),
        // so only assert that detection reports a coherent status.
        let mut rt = rt;
        assert!(matches!(
            rt.detect(),
            crate::runtime::RuntimeStatus::Available | crate::runtime::RuntimeStatus::Missing
        ));
    }

    #[test]
    fn env_is_filtered_to_allowlist() {
        let rt = NativeRuntime::new();
        let mut t = LaunchTarget::new("a", "A").with_executable("/bin/true");
        t.env.insert("SECRET_TOKEN".into(), "x".into());
        t.env.insert("WINEPREFIX".into(), "/tmp/p".into());
        let spec = rt.build_spec(&t).unwrap();
        assert!(spec.env.iter().all(|(k, _)| k == "WINEPREFIX"));
    }
}
