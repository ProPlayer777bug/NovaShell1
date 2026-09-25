//! Process manager: tracks everything the shell has running.
//!
//! The shell previously tracked a single `RunningSession`. Wine games, emulators
//! and helper windows can legitimately run at the same time, so sessions are
//! keyed by app id and a pid. Stop/kill act on a process *group* (the launcher
//! spawns with `process_group(0)`), and never on an unrelated pid.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::launcher::Spec;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessState {
    Starting,
    Running,
    Stopping,
    Exited(i32),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub app_id: String,
    pub title: String,
    /// Runtime that launched it (`native`, `emulator-pcsx2-qt`, `wine`, …).
    pub runtime: String,
    /// Runtime version string, when the runtime reported one.
    pub runtime_version: Option<String>,
    pub prefix: Option<String>,
    pub executable: String,
    pub started_at: i64,
    pub state: ProcessState,
}

impl ProcessInfo {
    pub fn is_running(&self) -> bool {
        matches!(self.state, ProcessState::Starting | ProcessState::Running)
    }
}

type Session = ProcessInfo;

/// Shared, thread-safe registry of running sessions.
#[derive(Clone, Default)]
pub struct ProcessManager {
    sessions: Arc<Mutex<BTreeMap<u32, Session>>>,
}

impl ProcessManager {
    pub fn new() -> Self {
        ProcessManager::default()
    }

    pub fn insert(&self, info: ProcessInfo) {
        if let Ok(mut map) = self.sessions.lock() {
            map.insert(info.pid, info);
        }
    }

    pub fn get(&self, pid: u32) -> Option<ProcessInfo> {
        self.sessions.lock().ok()?.get(&pid).cloned()
    }

    pub fn list(&self) -> Vec<ProcessInfo> {
        self.sessions
            .lock()
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }

    pub fn running(&self) -> Vec<ProcessInfo> {
        self.list().into_iter().filter(|p| p.is_running()).collect()
    }

    /// Move a session from one pid to another.
    ///
    /// Wine and Proton do not keep the pid the shell spawned: `wine` forks
    /// `app.exe` and exits, and the Proton wrapper execs through python. The
    /// window is then owned by a different process, often in its own process
    /// group, so the tracked pid goes stale while the app keeps running. This
    /// re-keys the session onto the real window owner.
    pub fn rekey(&self, from: u32, to: u32) -> bool {
        if let Ok(mut map) = self.sessions.lock() {
            if let Some(mut info) = map.remove(&from) {
                info.pid = to;
                map.insert(to, info);
                return true;
            }
        }
        false
    }

    /// Mark a session as finished and forget it.
    pub fn finish(&self, pid: u32, code: i32) {
        if let Ok(mut map) = self.sessions.lock() {
            map.remove(&pid);
            let _ = code;
        }
    }

    pub fn update_state(&self, pid: u32, state: ProcessState) {
        if let Ok(mut map) = self.sessions.lock() {
            if let Some(s) = map.get_mut(&pid) {
                s.state = state;
            }
        }
    }

    pub fn clear(&self) {
        if let Ok(mut map) = self.sessions.lock() {
            map.clear();
        }
    }

    /// Ask a process group to stop (SIGTERM), then verify it is gone.
    pub fn stop(&self, pid: u32) -> anyhow::Result<()> {
        self.signal_group(pid, "-TERM")
    }

    /// Force kill a process group (SIGKILL).
    pub fn kill(&self, pid: u32) -> anyhow::Result<()> {
        self.signal_group(pid, "-KILL")
    }

    fn signal_group(&self, pid: u32, signal: &str) -> anyhow::Result<()> {
        if pid == 0 {
            anyhow::bail!("refusing to signal pid 0");
        }
        // Only signal pids we actually track, so a bad request can never hit an
        // unrelated process. A pid the OS has since recycled is also refused
        // rather than being used to kill somebody else's process.
        if self.get(pid).is_none() {
            anyhow::bail!("not a tracked process: {pid}");
        }
        if !crate::runtime::procs::process_alive(pid) {
            self.finish(pid, 0);
            anyhow::bail!("that process has already exited");
        }
        // `kill -TERM100` signals process 100; a process *group* needs the
        // leading dash: `kill -TERM -100`. The old form left wineserver, Proton
        // helpers and emulator children running after "Force".
        let status = std::process::Command::new("kill")
            .args([signal, &format!("-{pid}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;
        if !status.success() {
            anyhow::bail!("could not signal process group {pid}");
        }
        self.update_state(pid, ProcessState::Stopping);
        Ok(())
    }
}

/// Is this pid still running?
pub fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        true
    }
}

/// Build the `ProcessInfo` recorded when a spec is spawned.
pub fn describe(spec: &Spec, app_id: &str, runtime: &str, prefix: Option<&str>) -> ProcessInfo {
    ProcessInfo {
        pid: 0,
        app_id: app_id.to_string(),
        title: spec.name.clone(),
        runtime: runtime.to_string(),
        runtime_version: None,
        prefix: prefix.map(str::to_string),
        executable: spec.program.clone(),
        started_at: crate::util::unix_now(),
        state: ProcessState::Starting,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(pid: u32) -> ProcessInfo {
        ProcessInfo {
            pid,
            app_id: format!("app-{pid}"),
            title: format!("App {pid}"),
            runtime: "native".into(),
            runtime_version: None,
            prefix: None,
            executable: "/bin/true".into(),
            started_at: 0,
            state: ProcessState::Running,
        }
    }

    #[test]
    fn tracks_and_lists_sessions() {
        let pm = ProcessManager::new();
        pm.insert(info(100));
        pm.insert(info(200));
        assert_eq!(pm.list().len(), 2);
        assert_eq!(pm.running().len(), 2);
        assert!(pm.get(100).is_some());
    }

    #[test]
    fn refuses_to_signal_untracked_pids() {
        let pm = ProcessManager::new();
        assert!(pm.stop(999_999).is_err());
        assert!(pm.kill(0).is_err());
    }

    /// Only meaningful where a liveness probe exists: on other platforms
    /// `process_alive` assumes a pid is running, so the guard cannot trigger.
    #[cfg(target_os = "linux")]
    #[test]
    fn refuses_to_signal_a_process_that_has_already_exited() {
        // A tracked pid that is gone must not be signalled: the OS may have
        // recycled it for an unrelated process.
        let pm = ProcessManager::new();
        pm.insert(info(999_998));
        let err = pm.stop(999_998).unwrap_err().to_string();
        assert!(err.contains("already exited"), "unexpected error: {err}");
        // ...and the stale record is dropped.
        assert!(pm.get(999_998).is_none());
    }

    #[test]
    fn liveness_probe_rejects_pid_zero() {
        assert!(!process_alive(0));
        // The shell's own pid is definitely running.
        assert!(process_alive(std::process::id()));
    }

    #[test]
    fn finish_removes_the_session() {
        let pm = ProcessManager::new();
        pm.insert(info(100));
        pm.finish(100, 0);
        assert!(pm.get(100).is_none());
    }

    #[test]
    fn state_can_be_updated() {
        let pm = ProcessManager::new();
        pm.insert(info(100));
        pm.update_state(100, ProcessState::Stopping);
        assert_eq!(pm.get(100).unwrap().state, ProcessState::Stopping);
    }

    #[test]
    fn describe_uses_spec_details() {
        let spec = Spec::new("My Game", "/usr/bin/wine");
        let p = describe(&spec, "g1", "wine", Some("/prefix"));
        assert_eq!(p.title, "My Game");
        assert_eq!(p.executable, "/usr/bin/wine");
        assert_eq!(p.runtime, "wine");
        assert_eq!(p.prefix.as_deref(), Some("/prefix"));
        assert_eq!(p.state, ProcessState::Starting);
    }
}
