//! Emulator runtime: adapts the existing `apps::EMULATORS` catalogue.
//!
//! Nothing about emulator behaviour changes — the table in
//! `integrations::apps` remains the single source of truth. This adapter only
//! presents each console as a runtime and builds the same `Spec` the shell
//! already builds (program, ROM argument, console polish flags).

use std::path::Path;

use crate::integrations::apps::EmulatorDef;
use crate::launcher::Spec;
use crate::runtime::security;
use crate::runtime::{Capability, Check, LaunchTarget, Runtime, RuntimeInfo, RuntimeKind, RuntimeStatus};

/// One console (PS2, PS3, …) as a runtime.
pub struct EmulatorRuntime {
    def: &'static EmulatorDef,
}

impl EmulatorRuntime {
    pub fn new(def: &'static EmulatorDef) -> Self {
        EmulatorRuntime { def }
    }

    /// Stable id, matching the library tile id shape (`emulator-pcsx2-qt`).
    fn id(&self) -> String {
        let bin = self
            .def
            .bins
            .iter()
            .find(|b| crate::integrations::apps::find_bin(b).is_some())
            .or_else(|| self.def.bins.first())
            .copied()
            .unwrap_or("unknown");
        format!("emulator-{bin}")
    }

    fn resolved_bin(&self) -> Option<String> {
        self.def
            .bins
            .iter()
            .find_map(|b| crate::integrations::apps::find_bin(b))
    }
}

/// Every console in the catalogue, as runtimes.
pub fn emulator_runtimes() -> Vec<Box<dyn Runtime>> {
    crate::integrations::apps::EMULATORS
        .iter()
        .map(|d| Box::new(EmulatorRuntime::new(d)) as Box<dyn Runtime>)
        .collect()
}

impl Runtime for EmulatorRuntime {
    fn info(&self) -> RuntimeInfo {
        let mut info = RuntimeInfo::new(&self.id(), self.def.pretty, RuntimeKind::Emulator)
            .with_types(&self.def.exts)
            .with_platforms(&[self.def.platform])
            .with_capabilities(&[Capability::Launch]);
        info.executable = self.resolved_bin().map(Into::into);
        info
    }

    fn detect(&mut self) -> RuntimeStatus {
        match self.resolved_bin() {
            Some(_) => RuntimeStatus::Available,
            None => RuntimeStatus::Missing,
        }
    }

    fn validate(&self, target: &LaunchTarget) -> Vec<Check> {
        let mut checks = Vec::new();
        match self.resolved_bin() {
            Some(bin) => match security::validate_executable(Path::new(&bin)) {
                Ok(()) => checks.push(Check::ok("Emulator")),
                Err(e) => checks.push(Check::fail("Emulator", e.to_string())),
            },
            None => checks.push(Check::fail("Emulator", "not installed")),
        }
        if let Some(rom) = &target.executable {
            if rom.is_file() {
                checks.push(Check::ok("Game file"));
            } else {
                checks.push(Check::fail(
                    "Game file",
                    format!("{} not found", rom.display()),
                ));
            }
        }
        checks
    }

    fn build_spec(&self, target: &LaunchTarget) -> anyhow::Result<Spec> {
        let bin = self
            .resolved_bin()
            .ok_or_else(|| anyhow::anyhow!("{} is not installed", self.def.pretty))?;
        security::validate_args(&target.args)?;
        let mut spec = Spec::new(&target.title, bin);
        // The ROM (if any) is the single positional argument, exactly as the
        // shell passes it today.
        if let Some(rom) = &target.executable {
            if !rom.is_file() {
                anyhow::bail!("game file not found: {}", rom.display());
            }
            spec = spec.arg(rom.to_string_lossy().to_string());
        }
        spec = spec.args(target.args.clone());
        if let Some(cwd) = &target.working_dir {
            spec = spec.cwd(cwd.clone());
        }
        Ok(spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ps2() -> EmulatorRuntime {
        let def = crate::integrations::apps::EMULATORS
            .iter()
            .find(|d| d.platform == "PS2")
            .expect("PS2 entry in the catalogue");
        EmulatorRuntime::new(def)
    }

    #[test]
    fn catalogue_is_exposed_as_runtimes() {
        let rs = emulator_runtimes();
        assert_eq!(rs.len(), crate::integrations::apps::EMULATORS.len());
        assert!(rs
            .iter()
            .any(|r| r.info().supported_platforms == vec!["PS2".to_string()]));
    }

    #[test]
    fn id_is_stable_and_matches_tile_shape() {
        assert!(ps2().id().starts_with("emulator-"));
    }

    #[test]
    fn ps2_advertises_its_extensions() {
        let info = ps2().info();
        assert!(info.supported_file_types.contains(&"iso".to_string()));
        assert!(info.supported_file_types.contains(&"chd".to_string()));
        assert_eq!(info.supported_platforms, vec!["PS2".to_string()]);
    }

    #[test]
    fn build_spec_never_silently_ignores_a_missing_rom() {
        let rt = ps2();
        let t = LaunchTarget::new("e", "PS2").with_executable("/nope/missing.iso");
        // Either the emulator is not installed, or the ROM is missing: both
        // are errors, never a silent launch.
        assert!(rt.build_spec(&t).is_err());
    }
}
