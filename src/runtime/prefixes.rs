//! Per-application Wine/Proton prefixes.
//!
//! Layout (root is configurable, default under the shell's data dir):
//!
//! ```text
//! <root>/prefixes/wine/<app-id>/      # 32/64-bit Windows application
//! <root>/prefixes/proton/<app-id>/    # Windows game
//! ```
//!
//! One prefix per application — never a single shared prefix.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::runtime::security;

/// Default prefix root. Reuses the shell's data dir so it follows XDG and the
/// existing `NOVASHELL_DATA_DIR` override.
pub fn default_root() -> PathBuf {
    crate::util::data_dir().join("prefixes")
}

/// Extra roots the user can add in Settings (e.g. a games drive).
fn configured_roots() -> Vec<PathBuf> {
    crate::settings::Config::load().prefix_roots
}

/// The active prefix root.
pub fn root() -> PathBuf {
    default_root()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrefixKind {
    Wine,
    Proton,
}

impl PrefixKind {
    pub fn dir_name(self) -> &'static str {
        match self {
            PrefixKind::Wine => "wine",
            PrefixKind::Proton => "proton",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrefixInfo {
    pub id: String,
    pub kind: PrefixKind,
    pub path: PathBuf,
    pub exists: bool,
    /// Rough size in bytes (`None` when not measured yet).
    pub size_bytes: Option<u64>,
    pub last_used: Option<i64>,
}

impl PrefixInfo {
    pub fn status(&self) -> &'static str {
        if self.exists {
            "ready"
        } else {
            "missing"
        }
    }
}

/// Compute (but do not create) the prefix path for an application under an
/// explicit root. Tests and multi-root callers use this directly.
pub fn prefix_path_in(root: &Path, kind: PrefixKind, app_id: &str) -> anyhow::Result<PathBuf> {
    let segment = security::safe_segment(app_id)?;
    Ok(root.join(kind.dir_name()).join(segment))
}

/// Compute the prefix path under the active root.
pub fn prefix_path(kind: PrefixKind, app_id: &str) -> anyhow::Result<PathBuf> {
    prefix_path_in(&root(), kind, app_id)
}

/// Create the directory for a prefix under an explicit root. Idempotent.
pub fn create_in(root: &Path, kind: PrefixKind, app_id: &str) -> anyhow::Result<PrefixInfo> {
    let path = prefix_path_in(root, kind, app_id)?;
    std::fs::create_dir_all(&path)?;
    // A prefix is useless without the Wine-created marker directory, but we do
    // not run `wineboot` here: detection/installation is the runtime's job.
    let _ = std::fs::create_dir_all(path.join("drive_c"));
    Ok(PrefixInfo {
        id: app_id.to_string(),
        kind,
        exists: path.is_dir(),
        path,
        size_bytes: None,
        last_used: None,
    })
}

/// Create the prefix under the active root.
pub fn create(kind: PrefixKind, app_id: &str) -> anyhow::Result<PrefixInfo> {
    create_in(&root(), kind, app_id)
}

/// Delete a prefix under an explicit root. Only ever touches paths inside it.
pub fn delete_in(root: &Path, kind: PrefixKind, app_id: &str) -> anyhow::Result<()> {
    let path = prefix_path_in(root, kind, app_id)?;
    let safe = security::ensure_within(root, &path)?;
    if !safe.exists() {
        return Ok(());
    }
    if safe == root {
        anyhow::bail!("refusing to delete the prefix root");
    }
    std::fs::remove_dir_all(&safe)?;
    Ok(())
}

/// Delete a prefix under the active root.
pub fn delete(kind: PrefixKind, app_id: &str) -> anyhow::Result<()> {
    delete_in(&root(), kind, app_id)
}

/// "Repair" means: make sure the directory exists and drop a stale lock that
/// would stop a runtime from starting. Destructive resets are a separate,
/// explicitly confirmed action.
pub fn repair_in(root: &Path, kind: PrefixKind, app_id: &str) -> anyhow::Result<PrefixInfo> {
    let path = prefix_path_in(root, kind, app_id)?;
    std::fs::create_dir_all(path.join("drive_c"))?;
    let _ = std::fs::remove_file(path.join(".update-timestamp"));
    Ok(PrefixInfo {
        id: app_id.to_string(),
        kind,
        exists: true,
        path,
        size_bytes: None,
        last_used: None,
    })
}

/// Repair a prefix under the active root.
pub fn repair(kind: PrefixKind, app_id: &str) -> anyhow::Result<PrefixInfo> {
    repair_in(&root(), kind, app_id)
}

/// List prefixes under the active root plus any configured extra roots.
pub fn list() -> Vec<PrefixInfo> {
    let mut roots = vec![root()];
    for r in configured_roots() {
        if !roots.contains(&r) {
            roots.push(r);
        }
    }
    list_roots(&roots)
}

/// List prefixes under the given roots.
pub fn list_roots(roots: &[PathBuf]) -> Vec<PrefixInfo> {
    let mut out = Vec::new();
    for root in roots {
        for kind in [PrefixKind::Wine, PrefixKind::Proton] {
            let dir = root.join(kind.dir_name());
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in entries.flatten() {
                let path = e.path();
                if !path.is_dir() {
                    continue;
                }
                let id = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                out.push(PrefixInfo {
                    id,
                    kind,
                    exists: true,
                    path: path.clone(),
                    size_bytes: dir_size(&path),
                    last_used: None,
                });
            }
        }
    }
    out
}

/// Total size of a directory tree (bounded, so the UI never blocks long).
pub fn dir_size(path: &Path) -> Option<u64> {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    let mut budget = 20_000;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            if budget == 0 {
                return Some(total);
            }
            budget -= 1;
            match e.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(e.path()),
                Ok(ft) if ft.is_file() => {
                    if let Ok(m) = e.metadata() {
                        total += m.len();
                    }
                }
                _ => {}
            }
        }
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each test gets its own root directory, so the suite can run in parallel
    /// without touching the real prefix store or shared env vars.
    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(tag: &str) -> TempRoot {
            let dir = std::env::temp_dir().join(format!(
                "nova-prefix-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempRoot(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn path_is_per_app_and_under_root() {
        let root = TempRoot::new("paths");
        let a = prefix_path_in(root.path(), PrefixKind::Wine, "my-app").unwrap();
        let b = prefix_path_in(root.path(), PrefixKind::Wine, "other-app").unwrap();
        assert_ne!(a, b, "each app gets its own prefix");
        assert!(a.starts_with(root.path()));
        assert!(a.to_string_lossy().contains("wine"));
    }

    #[test]
    fn path_rejects_traversal_ids() {
        let root = TempRoot::new("reject");
        assert!(prefix_path_in(root.path(), PrefixKind::Wine, "../escape").is_err());
        assert!(prefix_path_in(root.path(), PrefixKind::Proton, "a/b").is_err());
    }

    #[test]
    fn create_is_idempotent() {
        let root = TempRoot::new("create");
        let first = create_in(root.path(), PrefixKind::Wine, "app1").unwrap();
        assert!(first.exists);
        assert!(first.path.join("drive_c").is_dir());
        let second = create_in(root.path(), PrefixKind::Wine, "app1").unwrap();
        assert_eq!(first.path, second.path);
    }

    #[test]
    fn delete_removes_only_that_prefix() {
        let root = TempRoot::new("delete");
        let a = create_in(root.path(), PrefixKind::Wine, "app1").unwrap();
        let b = create_in(root.path(), PrefixKind::Wine, "app2").unwrap();
        delete_in(root.path(), PrefixKind::Wine, "app1").unwrap();
        assert!(!a.path.exists());
        assert!(b.path.exists());
    }

    #[test]
    fn delete_is_safe_for_unknown_apps() {
        let root = TempRoot::new("unknown");
        // Deleting something that was never created is a no-op, not an error.
        assert!(delete_in(root.path(), PrefixKind::Proton, "never-created").is_ok());
    }

    #[test]
    fn list_finds_created_prefixes() {
        let root = TempRoot::new("list");
        create_in(root.path(), PrefixKind::Wine, "listed").unwrap();
        create_in(root.path(), PrefixKind::Proton, "gamed").unwrap();
        let ids: Vec<String> = list_roots(&[root.path().to_path_buf()])
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert!(ids.contains(&"listed".to_string()));
        assert!(ids.contains(&"gamed".to_string()));
    }

    #[test]
    fn dir_size_counts_files() {
        let root = TempRoot::new("size");
        let p = create_in(root.path(), PrefixKind::Wine, "sized").unwrap();
        std::fs::write(p.path.join("drive_c").join("a.bin"), vec![0u8; 2048]).unwrap();
        let size = dir_size(&p.path).unwrap();
        assert!(size >= 2048);
    }

    #[test]
    fn repair_creates_missing_prefix() {
        let root = TempRoot::new("repair");
        let info = repair_in(root.path(), PrefixKind::Wine, "repaired").unwrap();
        assert!(info.exists);
        assert!(info.path.is_dir());
    }
}
