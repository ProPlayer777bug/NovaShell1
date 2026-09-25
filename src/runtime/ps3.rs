//! PS3 package handling for RPCS3.
//!
//! PS3 games arrive as `.pkg` installers (plus optional `.rap`/`.edat`
//! key files). RPCS3 can install them without its GUI
//! (`rpcs3 --no-gui --installpkg <file>`), which is what the file-manager
//! handler and the Runtime Manager use so a double-clicked package never needs
//! the emulator window. Installed titles are then discovered from RPCS3's
//! `games/` directory and surfaced in the library.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Extensions RPCS3 knows how to install.
pub const INSTALLABLE_EXTENSIONS: [&str; 3] = ["pkg", "rap", "edat"];

/// The package file itself (installed into RPCS3).
pub const PKG_EXTENSION: &str = "pkg";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installable {
    pub path: PathBuf,
    /// `pkg`, `rap` or `edat`.
    pub kind: String,
    /// Title id parsed from a PS3 file name, e.g. `EP4049-NPEB00150_00-BRAID00000000001`.
    pub title_id: Option<String>,
    /// Best-effort display name.
    pub name: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledGame {
    /// RPCS3 title id, i.e. the `USRDIR` directory name.
    pub title_id: String,
    pub name: String,
    pub eboot: PathBuf,
}

/// Locate the RPCS3 binary, if installed.
pub fn find_rpcs3() -> Option<PathBuf> {
    for candidate in ["rpcs3", "/usr/local/bin/rpcs3", "/usr/bin/rpcs3"] {
        if let Ok(path) = which(candidate) {
            return Some(path);
        }
    }
    None
}

fn which(program: &str) -> Result<PathBuf> {
    if program.contains('/') {
        let p = PathBuf::from(program);
        return if p.is_file() { Ok(p) } else { Err(anyhow::anyhow!("missing")) };
    }
    let path = std::env::var("PATH").unwrap_or_default();
    for dir in path.split(':').filter(|d| !d.is_empty()) {
        let p = Path::new(dir).join(program);
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(anyhow::anyhow!("not on PATH"))
}

/// RPCS3's per-user config directory (`~/.config/rpcs3`).
pub fn rpcs3_config_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".config").join("rpcs3"))
}

/// True when PS3 firmware has been installed. Without it RPCS3 can install
/// packages but cannot boot a game, so the UI says so up front.
pub fn firmware_installed() -> bool {
    let Some(cfg) = rpcs3_config_dir() else {
        return false;
    };
    // RPCS3 writes the firmware into dev_flash (or dev_flash2/3 on retries).
    ["dev_flash", "dev_flash2", "dev_flash3"]
        .iter()
        .any(|d| {
            std::fs::read_dir(cfg.join(d))
                .map(|rd| rd.flatten().next().is_some())
                .unwrap_or(false)
        })
}

/// Pull the title id out of a PS3 file name.
///
/// `bkRvBuYxksN4BTr7eWxHE79O2XHPyacbEfS6Avy6uVEB4jyRSjXgUQUqFf5lCi38OAw4W5dIJLjxUaQqPI3HYxPfMigG7FClNvb9v.pkg`
/// has no usable id, but a decrypted package named
/// `EP4049-NPEB00150_00-BRAID00000000001.pkg` yields the full title id.
///
/// PS3 content ids are three dash-separated uppercase blocks: a 4-6 character
/// publisher/title block, an 8-9 character product block (with an optional
/// `_version` suffix) and the disc/content id.
pub fn parse_title_id(file_name: &str) -> Option<String> {
    let stem = Path::new(file_name).file_stem()?.to_string_lossy().to_string();
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let upper_alnum = |s: &str| {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    };
    // The product block may carry a `_00` version suffix.
    let product = parts[1].split('_').next().unwrap_or(parts[1]);
    if upper_alnum(parts[0])
        && parts[0].len() >= 4
        && upper_alnum(product)
        && product.len() >= 8
        && upper_alnum(parts[2])
        && parts[2].len() >= 8
    {
        Some(stem)
    } else {
        None
    }
}

/// A readable name for a package, preferring the title id's readable tail.
pub fn display_name(path: &Path) -> String {
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    match parse_title_id(&file) {
        // `..._00-BRAID00000000001` -> BRAID
        Some(id) => {
            let tail = id.rsplit('-').next().unwrap_or(&id);
            let letters: String = tail
                .chars()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            let cleaned = letters.trim_end_matches(|c: char| c.is_ascii_digit());
            if cleaned.len() >= 2 {
                return cleaned.to_string();
            }
            id
        }
        None => Path::new(&file)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or(file),
    }
}

/// List installable PS3 files in a directory (non-recursive, sorted).
pub fn find_installables(dir: &Path) -> Vec<Installable> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Installable> = rd
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if !path.is_file() {
                return None;
            }
            let ext = path.extension()?.to_string_lossy().to_lowercase();
            if !INSTALLABLE_EXTENSIONS.contains(&ext.as_str()) {
                return None;
            }
            let size_bytes = e.metadata().map(|m| m.len()).unwrap_or(0);
            let file = path.file_name()?.to_string_lossy().to_string();
            Some(Installable {
                title_id: parse_title_id(&file),
                name: display_name(&path),
                kind: ext,
                path,
                size_bytes,
            })
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Where RPCS3 expects a title's license file.
pub fn exdata_dir() -> Option<PathBuf> {
    rpcs3_config_dir().map(|c| {
        c.join("dev_hdd0")
            .join("home")
            .join("00000001")
            .join("exdata")
    })
}

/// A real `.rap` license is a few KB; a stub or an HTML error page is not.
const MIN_LICENSE_BYTES: u64 = 1024;

/// What still stands between a package and a bootable game.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallPrereqs {
    /// PS3 firmware is installed.
    pub firmware: bool,
    /// A plausible license file was found next to the package.
    pub license_found: bool,
    /// License is already in RPCS3's exdata folder.
    pub license_in_place: bool,
    /// Blocking problems, ready to show the user.
    pub blockers: Vec<String>,
}

/// Check — without installing anything — whether a package can actually be
/// booted once installed. Installing a 250 MB package only to fail 16 minutes
/// later is the worst possible outcome, so this runs first.
pub fn check_install_prereqs(pkg: &Path) -> InstallPrereqs {
    let mut out = InstallPrereqs {
        firmware: firmware_installed(),
        ..Default::default()
    };
    let title_id = pkg
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(parse_title_id);

    // A licence is optional for some titles but required for most, so report it
    // as a blocker only when neither a licence nor firmware is present.
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = pkg.parent() {
        for ext in ["rap", "edat"] {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().map(|x| x.to_string_lossy().to_lowercase())
                        == Some(ext.to_string())
                        && e.metadata().map(|m| m.len() >= MIN_LICENSE_BYTES).unwrap_or(false)
                    {
                        candidates.push(p);
                    }
                }
            }
        }
    }
    if let (Some(tid), Some(ex)) = (title_id.clone(), exdata_dir()) {
        out.license_in_place = ex.join(format!("{tid}.rap")).is_file()
            || ex.join(format!("{tid}.edat")).is_file();
    }
    out.license_found = !candidates.is_empty() || out.license_in_place;

    if !out.firmware {
        out.blockers.push(
            "PS3 firmware is not installed. RPCS3 needs firmware before it can boot any PS3 game; add it in RPCS3's Guide (F1)."
                .into(),
        );
    }
    if !out.license_found {
        // A common case is a stub file: a real .rap is a few KB, a placeholder
        // is a few bytes. Say so, because "no licence" is otherwise baffling
        // when a .rap is sitting right there in the folder.
        let stubs: Vec<String> = pkg
            .parent()
            .map(|d| {
                std::fs::read_dir(d)
                    .map(|rd| {
                        rd.flatten()
                            .filter(|e| {
                                matches!(
                                    e.path()
                                        .extension()
                                        .map(|x| x.to_string_lossy().to_lowercase())
                                        .as_deref(),
                                    Some("rap") | Some("edat")
                                )
                            })
                            .filter(|e| {
                                e.metadata().map(|m| m.len() < MIN_LICENSE_BYTES).unwrap_or(false)
                            })
                            .map(|e| e.file_name().to_string_lossy().to_string())
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        out.blockers.push(if stubs.is_empty() {
            "No license file for this title. RPCS3 needs the matching .rap in its exdata folder."
                .into()
        } else {
            format!(
                "The license file(s) {} are too small to be real (a genuine .rap is a few KB). Dump the license from your own console, or place a valid .rap next to the package.",
                stubs.join(", ")
            )
        });
    }
    out
}

/// Copy a license file next to its package into RPCS3's exdata folder, which
/// is the documented location RPCS3 looks in.
pub fn place_license(license: &Path, title_id: &str) -> Result<PathBuf> {
    let ex = exdata_dir().context("RPCS3 is not installed")?;
    std::fs::create_dir_all(&ex)?;
    let ext = license
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    // Only real license types, and a title id that cannot escape exdata: a
    // crafted id like `../../config` would otherwise write outside it.
    if !matches!(ext.as_str(), "rap" | "edat") {
        anyhow::bail!("{} is not a PS3 license file", license.display());
    }
    let safe_id = crate::runtime::security::safe_segment(title_id)
        .context("refusing an unsafe PS3 title id")?;
    if safe_id.contains(std::path::MAIN_SEPARATOR) || safe_id.contains('/') {
        anyhow::bail!("refusing an unsafe PS3 title id: {title_id}");
    }
    let dest = ex.join(format!("{safe_id}.{ext}"));
    std::fs::copy(license, &dest)
        .with_context(|| format!("copying {} to {}", license.display(), dest.display()))?;
    Ok(dest)
}

/// Install a `.pkg` with RPCS3.
///
/// RPCS3 refuses to install in `--no-gui` mode ("Cannot perform installation in
/// no-gui mode!"), so the install runs in its normal windowed mode. This blocks
/// until RPCS3 exits, so callers must run it off the UI thread.
pub fn install_package(pkg: &Path) -> Result<PathBuf> {
    // Validate the package before looking for the emulator: a caller passing
    // the wrong file should hear about that, not about a missing RPCS3.
    if !pkg.is_file() {
        anyhow::bail!("no such package: {}", pkg.display());
    }
    let ext = pkg
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if ext != PKG_EXTENSION {
        anyhow::bail!("{} is not a .pkg", pkg.display());
    }
    let rpcs3 = find_rpcs3().context("RPCS3 is not installed")?;

    // A licence sitting next to the package is copied into place first, so
    // RPCS3 finds it instead of aborting the install.
    if let Some(tid) = pkg.file_name().and_then(|n| n.to_str()).and_then(parse_title_id) {
        if let Some(dir) = pkg.parent() {
            for ext in ["rap", "edat"] {
                let cand = dir.join(format!("{tid}.{ext}"));
                if cand.is_file()
                    && std::fs::metadata(&cand).map(|m| m.len() >= MIN_LICENSE_BYTES).unwrap_or(false)
                {
                    let _ = place_license(&cand, &tid);
                    break;
                }
            }
        }
    }

    // No --no-gui: this RPCS3 build rejects installation in that mode.
    let child = std::process::Command::new(&rpcs3)
        .arg("--installpkg")
        .arg(pkg)
        .current_dir(rpcs3_config_dir().unwrap_or_else(|| PathBuf::from(".")))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("could not start {}", rpcs3.display()))?;

    let output = child
        .wait_with_output()
        .with_context(|| "waiting for RPCS3 to finish the install")?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if let Ok(log) = crate::runtime::logs::LaunchLog::create_in(
        &crate::runtime::logs::app_log_dir("rpcs3-install")?,
        "install",
    ) {
        log.write(&format!(
            "install {}\nstdout: {}\nstderr: {}",
            pkg.display(),
            stdout,
            stderr
        ));
    }

    // RPCS3 exits 0 even when it aborts, so the outcome is read from its output
    // and, more importantly, from the filesystem: an install that produced no
    // game directory did not install anything.
    let combined = format!("{stdout}{stderr}");
    if combined.contains("Failed to locate the game license file") {
        anyhow::bail!(
            "RPCS3 needs the license file for this title (.rap in its exdata folder); the package was not installed"
        );
    }
    if combined.contains("Cannot perform installation") {
        anyhow::bail!(
            "RPCS3 refused the installation: {}",
            combined.lines().last().unwrap_or("")
        );
    }
    if !output.status.success() {
        anyhow::bail!("RPCS3 could not install the package");
    }

    // A zero exit is not proof of success: verify the title actually landed.
    let before = scan_installed_games().len();
    if let Some(tid) = pkg
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(parse_title_id)
    {
        let landed = || {
            rpcs3_config_dir()
                .map(|c| c.join("games").join(&tid).join("EBOOT.BIN").is_file())
                .unwrap_or(false)
        };
        if !landed() {
            // Give the filesystem a moment before declaring failure.
            std::thread::sleep(std::time::Duration::from_secs(2));
            if !landed() {
                anyhow::bail!(
                    "RPCS3 reported success but {tid} is not in its games folder; nothing was installed"
                );
            }
        }
        return Ok(pkg.to_path_buf());
    }

    let after = scan_installed_games().len();
    if after <= before {
        anyhow::bail!(
            "RPCS3 finished but installed no new game; check its log for a decryption or format error"
        );
    }
    Ok(pkg.to_path_buf())
}

/// Installed PS3 games, discovered from RPCS3's `games/` directory.
pub fn scan_installed_games() -> Vec<InstalledGame> {
    let Some(cfg) = rpcs3_config_dir() else {
        return Vec::new();
    };
    let games_dir = cfg.join("games");
    let Ok(rd) = std::fs::read_dir(&games_dir) else {
        return Vec::new();
    };
    let mut out: Vec<InstalledGame> = rd
        .flatten()
        .filter_map(|e| {
            let dir = e.path();
            if !dir.is_dir() {
                return None;
            }
            let eboot = dir.join("EBOOT.BIN");
            if !eboot.is_file() {
                return None;
            }
            let title_id = dir.file_name()?.to_string_lossy().to_string();
            Some(InstalledGame {
                name: display_name(&dir),
                title_id,
                eboot,
            })
        })
        .collect();
    out.sort_by(|a, b| a.title_id.cmp(&b.title_id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ps3_title_ids() {
        assert_eq!(
            parse_title_id("EP4049-NPEB00150_00-BRAID00000000001.pkg"),
            Some("EP4049-NPEB00150_00-BRAID00000000001".to_string())
        );
        // Wrong block sizes are not title ids.
        assert_eq!(parse_title_id("braid.zip"), None);
        assert_eq!(parse_title_id("random-hash-name.pkg"), None);
    }

    #[test]
    fn derives_a_readable_name() {
        let p = Path::new("/games/EP4049-NPEB00150_00-BRAID00000000001.pkg");
        assert_eq!(display_name(p), "BRAID");
        // A random-named package falls back to its file name.
        assert_eq!(display_name(Path::new("/games/bkRvBuY.pkg")), "bkRvBuY");
    }

    #[test]
    fn finds_only_installable_extensions() {
        let dir = std::env::temp_dir().join(format!("nova-ps3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("EP4049-NPEB00150_00-BRAID00000000001.pkg"), b"x").unwrap();
        std::fs::write(dir.join("EP4049-NPEB00150_00-BRAID00000000001.rap"), b"x").unwrap();
        std::fs::write(dir.join("readme.txt"), b"x").unwrap();
        std::fs::create_dir_all(dir.join("subdir")).unwrap();

        let found = find_installables(&dir);
        assert_eq!(found.len(), 2, "only the pkg and rap are installable");
        let pkg = found.iter().find(|f| f.kind == "pkg").unwrap();
        assert_eq!(pkg.title_id.as_deref(), Some("EP4049-NPEB00150_00-BRAID00000000001"));
        assert_eq!(pkg.name, "BRAID");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_to_install_something_that_is_not_a_pkg() {
        let dir = std::env::temp_dir().join(format!("nova-ps3b-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("game.rap");
        std::fs::write(&fake, b"x").unwrap();
        let err = install_package(&fake).unwrap_err().to_string();
        assert!(err.contains("not a .pkg"), "unexpected error: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_package_is_an_error_not_a_panic() {
        assert!(install_package(Path::new("/definitely/missing.pkg")).is_err());
    }

    #[test]
    fn prereq_check_flags_missing_firmware_and_licence() {
        let dir = std::env::temp_dir().join(format!("nova-ps3c-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pkg = dir.join("EP4049-NPEB00150_00-BRAID00000000001.pkg");
        std::fs::write(&pkg, b"x").unwrap();
        // A stub licence (too small) must not count as a licence.
        std::fs::write(dir.join("EP4049-NPEB00150_00-BRAID00000000001.rap"), b"tiny").unwrap();

        let pre = check_install_prereqs(&pkg);
        assert!(!pre.license_found, "a 4-byte stub is not a licence");
        assert_eq!(pre.blockers.len(), 2, "firmware and licence are both missing");

        // A real-sized licence satisfies that blocker.
        std::fs::write(dir.join("EP4049-NPEB00150_00-BRAID00000000001.rap"), vec![7u8; 4096]).unwrap();
        let pre = check_install_prereqs(&pkg);
        assert!(pre.license_found);

        // A stub licence must be called out specifically, not just "missing".
        std::fs::write(dir.join("EP4049-NPEB00150_00-BRAID00000000001.rap"), b"tiny").unwrap();
        let pre = check_install_prereqs(&pkg);
        assert!(pre
            .blockers
            .iter()
            .any(|b| b.contains("too small")), "unexpected: {:?}", pre.blockers);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
