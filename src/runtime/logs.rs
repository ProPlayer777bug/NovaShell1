//! Structured per-application launch logs.
//!
//! Logs live under the shell's data dir and are written with plain
//! `std::fs`, never through a shell. Secrets are never written: only the
//! command, exit status and the runtime's own output are recorded, and
//! obviously sensitive environment values are skipped entirely.

use std::path::PathBuf;

fn log_root() -> PathBuf {
    crate::util::data_dir().join("logs").join("runtime")
}

/// Directory for one application's logs.
pub fn app_log_dir(app_id: &str) -> anyhow::Result<PathBuf> {
    let segment = crate::runtime::security::safe_segment(app_id)?;
    let dir = log_root().join(segment);
    crate::util::ensure_dir(&dir)?;
    Ok(dir)
}

/// A timestamped log file plus the `latest.log` convenience link/copy.
pub struct LaunchLog {
    pub path: PathBuf,
    pub latest: PathBuf,
}

impl LaunchLog {
    /// Create a log inside an explicit directory (used by tests and tooling).
    pub fn create_in(dir: &std::path::Path, app_id: &str) -> anyhow::Result<LaunchLog> {
        let segment = crate::runtime::security::safe_segment(app_id)?;
        let dir = dir.join(segment);
        crate::util::ensure_dir(&dir)?;
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f");
        Ok(LaunchLog {
            path: dir.join(format!("{stamp}.log")),
            latest: dir.join("latest.log"),
        })
    }

    pub fn create(app_id: &str) -> anyhow::Result<LaunchLog> {
        Self::create_in(&log_root(), app_id)
    }

    /// Append a line to both the timestamped file and `latest.log`.
    pub fn write(&self, line: &str) {
        for path in [&self.path, &self.latest] {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(f, "{line}");
            }
        }
    }

    pub fn write_launch(&self, program: &str, args: &[String], env_keys: &[&str]) {
        // Only the *names* of environment variables are recorded, never values.
        self.write(&format!(
            "launch: {} {}",
            program,
            args.join(" ")
        ));
        if !env_keys.is_empty() {
            self.write(&format!("env keys: {}", env_keys.join(", ")));
        }
    }

    pub fn write_exit(&self, code: Option<i32>) {
        self.write(&format!("exit: {}", code.map(|c| c.to_string()).unwrap_or_else(|| "signal".into())));
    }

    pub fn write_error(&self, err: &str) {
        self.write(&format!("error: {err}"));
    }
}

/// Tail the most recent log lines for the UI, from an explicit root.
pub fn tail_in(dir: &std::path::Path, app_id: &str, lines: usize) -> Vec<String> {
    let Ok(segment) = crate::runtime::security::safe_segment(app_id) else {
        return Vec::new();
    };
    let app_dir = dir.join(segment);
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&app_dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map(|x| x == "log").unwrap_or(false))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let Some(last) = files.pop() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(last) else {
        return Vec::new();
    };
    let all: Vec<&str> = text.lines().collect();
    all.iter()
        .skip(all.len().saturating_sub(lines))
        .map(|s| s.to_string())
        .collect()
}

/// Tail the most recent log lines for the UI.
pub fn tail(app_id: &str, lines: usize) -> Vec<String> {
    tail_in(&log_root(), app_id, lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let dir = std::env::temp_dir().join(format!(
                "nova-logs-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn writes_and_tails() {
        let root = TempDir::new("write");
        let log = LaunchLog::create_in(root.0.as_path(), "app1").unwrap();
        log.write_launch("/usr/bin/wine", &["game.exe".into()], &["WINEPREFIX"]);
        log.write_exit(Some(0));
        let lines = tail_in(root.0.as_path(), "app1", 10);
        assert!(lines.iter().any(|l| l.contains("/usr/bin/wine")));
        assert!(lines.iter().any(|l| l.starts_with("exit: 0")));
    }

    #[test]
    fn latest_log_is_written_too() {
        let root = TempDir::new("latest");
        let log = LaunchLog::create_in(root.0.as_path(), "app1").unwrap();
        log.write("hello");
        assert!(log.latest.is_file());
        let text = std::fs::read_to_string(&log.latest).unwrap();
        assert!(text.contains("hello"));
    }

    #[test]
    fn rejects_unsafe_app_ids() {
        let root = TempDir::new("unsafe");
        assert!(LaunchLog::create_in(root.0.as_path(), "../escape").is_err());
    }

    #[test]
    fn tail_of_unknown_app_is_empty() {
        let root = TempDir::new("unknown");
        assert!(tail_in(root.0.as_path(), "nothing-here", 5).is_empty());
    }
}
