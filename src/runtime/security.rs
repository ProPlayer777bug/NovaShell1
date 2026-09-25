//! Security rules for the runtime layer.
//!
//! Downloaded Windows executables are untrusted. Nothing here spawns anything;
//! it decides whether a path, id or environment variable is allowed to reach a
//! process. The rules are deliberately strict and centralised so a new runtime
//! cannot accidentally bypass them.

use std::path::{Component, Path, PathBuf};

/// Environment variables a Windows application may see. Anything not listed is
/// dropped, so host secrets (tokens, SSH agent sockets, cloud creds) are not
/// handed to Wine/Proton.
const ENV_ALLOWLIST: &[&str] = &[
    "WINEPREFIX",
    "WINEDEBUG",
    "WINEARCH",
    "WINESYNC",
    "WINEPROTON",
    "DXVK_HUD",
    "DXVK_LOG_LEVEL",
    "DXVK_STATE_CACHE_PATH",
    "VKD3D_CONFIG",
    "VKD3D_DEBUG",
    "MESA_VK_DEVICE_SELECT",
    "RADV_PERFTEST",
    "GAMSCOPE",
    "PROTON_LOG",
    "PROTON_DUMP_DEBUG_COMMANDS",
    "STEAM_COMPAT_DATA_PATH",
    "STEAM_COMPAT_CLIENT_INSTALL_PATH",
    "LANG",
    "LC_ALL",
];

/// Ids we are willing to build filesystem paths from.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        && !id.starts_with('.')
}

/// Turn a user-supplied app id into a single safe path segment.
pub fn safe_segment(id: &str) -> anyhow::Result<String> {
    if !valid_id(id) {
        anyhow::bail!("unsafe id: {id:?}");
    }
    Ok(id.to_string())
}

/// Reject traversal, NUL bytes and absolute paths in user-supplied locations.
pub fn validate_relative(path: &str) -> anyhow::Result<PathBuf> {
    if path.is_empty() {
        anyhow::bail!("empty path");
    }
    if path.contains('\0') {
        anyhow::bail!("path contains NUL");
    }
    let p = Path::new(path);
    if p.is_absolute() {
        anyhow::bail!("absolute paths are not allowed here");
    }
    for c in p.components() {
        match c {
            Component::Normal(_) => {}
            Component::CurDir => {}
            // `..`, root, prefix — all rejected.
            _ => anyhow::bail!("path escapes its root: {path}"),
        }
    }
    Ok(p.to_path_buf())
}

/// Ensure `candidate` is inside `root`.
///
/// The lexical check (after resolving `..`) always runs, so the result is
/// stable whether or not the paths exist. When both exist, the canonical forms
/// are compared as well, which catches symlinks that point outside the root.
pub fn ensure_within(root: &Path, candidate: &Path) -> anyhow::Result<PathBuf> {
    let norm_root = normalise(root);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        norm_root.join(candidate)
    };
    let normalised = normalise(&joined);
    if !normalised.starts_with(&norm_root) {
        anyhow::bail!("path is outside the allowed root");
    }
    // Defence in depth: if both sides exist, compare canonical forms too.
    if let (Ok(canon_root), Ok(canon_path)) = (root.canonicalize(), normalised.canonicalize()) {
        if !canon_path.starts_with(&canon_root) {
            anyhow::bail!("path resolves outside the allowed root");
        }
    }
    Ok(normalised)
}

/// Lexically resolve `.`/`..` without touching the filesystem.
pub fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Windows applications and emulators must never run as root.
pub fn ensure_not_root() -> anyhow::Result<()> {
    if crate::util::user_name() == "root" || effective_uid() == 0 {
        anyhow::bail!("refusing to run a Windows application as root");
    }
    Ok(())
}

/// Effective uid, read from `/proc/self/status` (avoids a libc dependency).
///
/// `/proc/self/status` lists `Uid: real effective saved filesystem`, so the
/// *second* field is the effective uid. Reading the first (real) uid let a
/// setuid process with a non-zero real uid and effective uid 0 pass the check.
fn effective_uid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                // "Uid:\t<real>\t<effective>\t<saved>\t<fs>" — index 1 is the
                // *real* uid. Reading it let a setuid process with real uid
                // 1000 but effective uid 0 pass the root check.
                .and_then(|l| l.split_whitespace().nth(2).map(|v| v.to_string()))
        })
        .and_then(|v| v.parse().ok())
        .unwrap_or(u32::MAX)
}

/// Filter an environment map down to the allowlist.
pub fn filter_env(env: &std::collections::BTreeMap<String, String>) -> std::collections::BTreeMap<String, String> {
    env.iter()
        .filter(|(k, _)| ENV_ALLOWLIST.iter().any(|a| a.eq_ignore_ascii_case(k)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Reject arguments that could be interpreted by a shell. Arguments are passed
/// as an array, so this only catches control characters and absurd lengths.
pub fn validate_args(args: &[String]) -> anyhow::Result<()> {
    if args.len() > 256 {
        anyhow::bail!("too many arguments");
    }
    for a in args {
        if a.len() > 4096 {
            anyhow::bail!("argument is too long");
        }
        if a.contains('\0') {
            anyhow::bail!("argument contains NUL");
        }
    }
    Ok(())
}

/// An executable must exist, be a regular file and be executable.
pub fn validate_executable(path: &Path) -> anyhow::Result<()> {
    let meta = std::fs::metadata(path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    if !meta.is_file() {
        anyhow::bail!("not a regular file: {}", path.display());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            anyhow::bail!("not executable: {}", path.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal() {
        assert!(validate_relative("../etc/passwd").is_err());
        assert!(validate_relative("a/../../b").is_err());
        // `..` is rejected wherever it appears, even if it would resolve back
        // inside the root: callers normalise first (see `normalise`).
        assert!(validate_relative("a/b/../c").is_err());
        assert!(validate_relative("a/b/c").is_ok());
    }

    #[test]
    fn rejects_absolute_and_empty() {
        assert!(validate_relative("/etc/passwd").is_err());
        assert!(validate_relative("").is_err());
    }

    #[test]
    fn rejects_nul_bytes() {
        assert!(validate_relative("a\0b").is_err());
    }

    #[test]
    fn rejects_unsafe_ids() {
        assert!(safe_segment("my-game_1.0").is_ok());
        assert!(safe_segment("../evil").is_err());
        assert!(safe_segment("..").is_err());
        assert!(safe_segment(".hidden").is_err());
        assert!(safe_segment("a/b").is_err());
        assert!(safe_segment("").is_err());
        assert!(safe_segment(&"x".repeat(65)).is_err());
    }

    #[test]
    fn ensure_within_blocks_escapes() {
        let root = std::env::temp_dir();
        assert!(ensure_within(&root, Path::new("../etc")).is_err());
        assert!(ensure_within(&root, Path::new("child/file")).is_ok());
    }

    #[test]
    fn env_filter_drops_unknown_keys() {
        let mut env = std::collections::BTreeMap::new();
        env.insert("WINEPREFIX".to_string(), "/tmp/p".to_string());
        env.insert("AWS_SECRET_ACCESS_KEY".to_string(), "nope".to_string());
        let filtered = filter_env(&env);
        assert!(filtered.contains_key("WINEPREFIX"));
        assert!(!filtered.contains_key("AWS_SECRET_ACCESS_KEY"));
    }

    #[test]
    fn args_validation_blocks_nul_and_floods() {
        assert!(validate_args(&["ok".into()]).is_ok());
        assert!(validate_args(&["bad\0".into()]).is_err());
        let many: Vec<String> = (0..300).map(|_| "x".into()).collect();
        assert!(validate_args(&many).is_err());
    }

    #[test]
    fn executable_validation_rejects_missing() {
        assert!(validate_executable(Path::new("/definitely/not/here")).is_err());
    }
}
