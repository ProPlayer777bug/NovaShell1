use anyhow::Result;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub fn config_dir() -> PathBuf {
    if let Ok(dir) = env::var("NOVASHELL_CONFIG_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    dirs::config_dir()
        .map(|p| p.join("novashell"))
        .unwrap_or_else(|| PathBuf::from(".config").join("novashell"))
}

pub fn data_dir() -> PathBuf {
    if let Ok(dir) = env::var("NOVASHELL_DATA_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    dirs::data_dir()
        .map(|p| p.join("novashell"))
        .unwrap_or_else(|| PathBuf::from(".local/share").join("novashell"))
}

/// `~/.cache/novashell` — transient data.
pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .map(|p| p.join("novashell"))
        .unwrap_or_else(|| data_dir().join("cache"))
}

pub fn logs_dir() -> PathBuf {
    data_dir().join("logs")
}

pub fn screenshots_dir() -> PathBuf {
    data_dir().join("screenshots")
}

/// Create a directory (and parents) if missing.
pub fn ensure_dir(p: &Path) -> Result<()> {
    fs::create_dir_all(p)?;
    Ok(())
}

/// Create all NovaShell data directories at startup.
pub fn ensure_dirs() -> Result<()> {
    ensure_dir(&config_dir())?;
    for d in [&data_dir(), &cache_dir(), &logs_dir(), &screenshots_dir()] {
        ensure_dir(d)?;
    }
    Ok(())
}

/// Current user name (falls back to the USER/LOGNAME env or "user").
pub fn user_name() -> String {
    if let Ok(name) = env::var("USER") {
        if !name.is_empty() {
            return name;
        }
    }
    if let Ok(name) = env::var("LOGNAME") {
        if !name.is_empty() {
            return name;
        }
    }
    "user".to_string()
}

/// Current host name.
pub fn host_name() -> String {
    if let Ok(h) = env::var("HOSTNAME") {
        if !h.is_empty() {
            return h;
        }
    }
    // Avoid depending on nix just for this: read from /proc when present.
    #[cfg(target_os = "linux")]
    if let Ok(content) = fs::read_to_string("/proc/sys/kernel/hostname") {
        let h = content.trim();
        if !h.is_empty() {
            return h.to_string();
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = ();
    }
    "host".to_string()
}

/// Replace a leading `~/` with the user's home directory.
pub fn expand_tilde(p: &str) -> PathBuf {
    if p == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    }
    if let Some(rest) = p.strip_prefix("~/") {
        return dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or_else(|| PathBuf::from(p));
    }
    PathBuf::from(p)
}

/// Current UNIX time in seconds.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}