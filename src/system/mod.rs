//! System introspection and control (Linux).
//!
//! Everything here degrades gracefully: a missing tool or file yields `None`
//! rather than an error bubbling to the UI. NovaShell never modifies boot
//! configuration, GRUB, kernel parameters, or system services.

use crate::launcher::run_capture;
use crate::util;
use anyhow::Result;
use std::path::PathBuf;

/// Serialize system state and facts for the UI status bar.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct StatusSnapshot {
    pub user: String,
    pub host: String,
    pub kernel: String,
    pub session_type: String,
    pub cpu_usage_percent: f32,
    pub mem_used_mb: u64,
    pub mem_total_mb: u64,
    pub mem_percent: u32,
    pub disk_free_mb: u64,
    pub disk_total_mb: u64,
    pub battery_percent: Option<u8>,
    pub battery_charging: Option<bool>,
    pub network_up: bool,
    pub network_ssid: Option<String>,
    pub controllers: Vec<ControllerInfo>,
    pub uptime_secs: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct ControllerInfo {
    pub id: usize,
    pub name: String,
    pub battery: Option<u8>,
    pub wired: Option<bool>,
}

fn read_sysfs(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
}

/// CPU usage % measured between two samples.
pub struct CpuSampler {
    prev_idle: u64,
    prev_total: u64,
    started: bool,
}

impl CpuSampler {
    pub fn new() -> Self {
        CpuSampler {
            prev_idle: 0,
            prev_total: 0,
            started: false,
        }
    }

    fn sample() -> (u64, u64) {
        let Ok(text) = std::fs::read_to_string("/proc/stat") else {
            return (0, 0);
        };
        let line = text.lines().next().unwrap_or("");
        let mut parts = line.split_whitespace();
        let _cpu = parts.next();
        let mut idle = 0u64;
        let mut total = 0u64;
        let nums: Vec<u64> = parts.filter_map(|s| s.parse().ok()).collect();
        for (i, v) in nums.iter().enumerate() {
            total += v;
            if i == 3 || i == 4 {
                idle += v;
            }
        }
        (idle, total)
    }

    /// Return percentage used since the previous call.
    pub fn usage_prct(&mut self) -> f32 {
        let (idle, total) = Self::sample();
        if !self.started {
            self.prev_idle = idle;
            self.prev_total = total;
            self.started = true;
            return 0.0;
        }
        let dt = total.saturating_sub(self.prev_total);
        let di = idle.saturating_sub(self.prev_idle);
        self.prev_idle = idle;
        self.prev_total = total;
        if dt == 0 {
            return 0.0;
        }
        ((dt - di) as f32 / dt as f32 * 100.0).clamp(0.0, 100.0)
    }
}

impl Default for CpuSampler {
    fn default() -> Self {
        Self::new()
    }
}

/// Memory usage in MB from /proc/meminfo.
pub fn memory_mb() -> (u64, u64) {
    let Ok(text) = std::fs::read_to_string("/proc/meminfo") else {
        return (0, 0);
    };
    let mut total_kb = 0u64;
    let mut avail_kb = 0u64;
    for line in text.lines() {
        if line.starts_with("MemTotal:") {
            total_kb = line
                .split_whitespace()
                .nth(1)
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        } else if line.starts_with("MemAvailable:") {
            avail_kb = line
                .split_whitespace()
                .nth(1)
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        }
    }
    let total = total_kb / 1024;
    let used = total.saturating_sub(avail_kb / 1024);
    (used, total)
}

/// Root filesystem usage via `df`.
pub fn disk_mb() -> (u64, u64) {
    match run_capture("df", &["-B1", "-P", "/"]) {
        Ok(out) => {
            if let Some(line) = out.lines().nth(1) {
                let mut it = line.split_whitespace();
                it.next(); // device
                let total = it.next().and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
                let used = it.next().and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
                let free = total.saturating_sub(used);
                return (total / 1024 / 1024, free / 1024 / 1024);
            }
            (0, 0)
        }
        Err(_) => (0, 0),
    }
}

/// Battery info from sysfs.
pub fn battery() -> Option<(u8, bool)> {
    // Determine the primary battery device.
    let devices = std::fs::read_dir("/sys/class/power_supply").ok()?;
    let mut candidates = Vec::new();
    for d in devices.flatten() {
        let name = d.file_name().to_string_lossy().to_string();
        let t = read_sysfs(&format!("/sys/class/power_supply/{name}/type"));
        if t.as_deref() == Some("Battery") {
            candidates.push(name);
        }
    }
    let name = candidates.into_iter().next()?;
    let cap = read_sysfs(&format!("/sys/class/power_supply/{name}/capacity"))?;
    let status = read_sysfs(&format!("/sys/class/power_supply/{name}/status"));
    let percent = cap.parse::<u8>().ok()?;
    let charging = status.as_deref().map(|s| s == "Charging").unwrap_or(false);
    Some((percent.clamp(0, 100), charging))
}

/// Is any network interface up (beyond loopback)?
pub fn network_up() -> bool {
    let Ok(rd) = std::fs::read_dir("/sys/class/net") else {
        return false;
    };
    for d in rd.flatten() {
        let name = d.file_name().to_string_lossy().to_string();
        if name == "lo" {
            continue;
        }
        if let Some(state) = read_sysfs(&format!("/sys/class/net/{name}/operstate")) {
            if state == "up" {
                return true;
            }
        }
    }
    false
}

/// SSID of the active Wi-Fi connection via `nmcli` (NetworkManager).
pub fn network_ssid() -> Option<String> {
    let out = run_capture(
        "nmcli",
        &[
            "-t",
            "-f",
            "ACTIVE,SSID",
            "-c",
            "no",
            "dev",
            "wifi",
        ],
    )
    .ok()?;
    for line in out.lines() {
        if let Some((active, ssid)) = line.split_once(':') {
            if active == "yes" && !ssid.is_empty() {
                return Some(ssid.to_string());
            }
        }
    }
    None
}

/// Kernel version string.
pub fn kernel_version() -> String {
    read_sysfs("/proc/sys/kernel/osrelease")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "linux".to_string())
}

/// Session type: wayland or x11.
pub fn session_type() -> String {
    std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "x11".into())
}

pub fn uptime_secs() -> u64 {
    read_sysfs("/proc/uptime")
        .and_then(|s| {
            s.split_whitespace()
                .next()
                .and_then(|v| v.parse::<f64>().ok())
        })
        .map(|v| v as u64)
        .unwrap_or(0)
}

/// Full status snapshot sent to the UI.
pub fn snapshot(cpu: &mut CpuSampler, controllers: &[crate::system::ControllerInfo]) -> StatusSnapshot {
    let (used, total) = memory_mb();
    let (total_d, free_d) = disk_mb();
    let (battery_percent, battery_charging) = battery().map(|(p, c)| (Some(p), Some(c))).unwrap_or((None, None));
    StatusSnapshot {
        user: util::user_name(),
        host: util::host_name(),
        kernel: kernel_version(),
        session_type: session_type(),
        cpu_usage_percent: cpu.usage_prct(),
        mem_used_mb: used,
        mem_total_mb: total,
        mem_percent: used
            .checked_mul(100)
            .and_then(|n| n.checked_div(total.max(1)))
            .map(|p| p.min(100))
            .unwrap_or(0) as u32,
        disk_total_mb: total_d,
        disk_free_mb: free_d,
        battery_percent,
        battery_charging,
        network_up: network_up(),
        network_ssid: network_ssid(),
        controllers: controllers.to_vec(),
        uptime_secs: uptime_secs(),
    }
}

// ---------------------------------------------------------------------------
// Audio, brightness
// ---------------------------------------------------------------------------

/// Read the WirePlumber default sink volume (0-100).
pub fn audio_volume() -> Option<u8> {
    let out = run_capture("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]).ok()?;
    let mut vol: f64 = 0.0;
    let mut muted = false;
    for tok in out.split_whitespace() {
        if let Some(v) = tok.strip_prefix("Volume:") {
            vol = v.parse().unwrap_or(0.0);
        }
        if tok == "[MUTED]" {
            muted = true;
        }
    }
    let _ = muted;
    Some((vol * 100.0).round().clamp(0.0, 100.0) as u8)
}

pub fn set_audio_volume(percent: u8) -> Result<()> {
    let v = (percent.clamp(0, 100) as f64 / 100.0).to_string();
    run_capture("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &v])?;
    Ok(())
}

fn backlight_sysfs() -> Option<PathBuf> {
    for entry in std::fs::read_dir("/sys/class/backlight").ok()? {
        let Ok(d) = entry else { continue };
        let name = d.file_name().to_string_lossy().to_string();
        let max = read_sysfs(&format!("/sys/class/backlight/{name}/max_brightness"));
        if let Some(m) = max {
            if let Ok(v) = m.parse::<u64>() {
                if v > 0 {
                    return Some(PathBuf::from(format!("/sys/class/backlight/{name}")));
                }
            }
        }
    }
    None
}

/// Screen brightness percent via sysfs (best-effort).
pub fn brightness() -> Option<u8> {
    let dir = backlight_sysfs()?;
    let cur = read_sysfs(&format!("{}/brightness", dir.display()))?;
    let max = read_sysfs(&format!("{}/max_brightness", dir.display()))?;
    let c = cur.parse::<f64>().ok()?;
    let m = max.parse::<f64>().ok()?.max(1.0);
    Some(((c / m) * 100.0).round().clamp(0.0, 100.0) as u8)
}

pub fn set_brightness(percent: u8) -> Result<()> {
    // Prefer brightnessctl when present (handles permissions + backends).
    if let Ok(out) = run_capture("brightnessctl", &[]) {
        let _ = out;
        let p = percent.to_string();
        run_capture("brightnessctl", &["set", &format!("{p}%")])?;
        return Ok(());
    }
    if let Some(dir) = backlight_sysfs() {
        let max_path = dir.join("max_brightness");
        if let Ok(m) = std::fs::read_to_string(&max_path) {
            if let Ok(max) = m.trim().parse::<u64>() {
                let value = ((percent.clamp(0, 100) as u64) * max / 100).max(if percent > 0 { 1 } else { 0 });
                std::fs::write(dir.join("brightness"), value.to_string())?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Power actions
// ---------------------------------------------------------------------------

/// Fire-and-forget system actions. These require working polkit/elogind in
/// the session; NovaShell never touches the bootloader.
pub fn suspend() -> Result<()> {
    run_capture("systemctl", &["suspend"])?;
    Ok(())
}

pub fn reboot() -> Result<()> {
    run_capture("systemctl", &["reboot"])?;
    Ok(())
}

pub fn poweroff() -> Result<()> {
    run_capture("systemctl", &["poweroff"])?;
    Ok(())
}

/// End the GNOME session gracefully.
pub fn logout() -> Result<()> {
    run_capture("gnome-session-quit", &["--logout"])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Screenshots
// ---------------------------------------------------------------------------

/// Take a screenshot into `dir`, returning the saved path.
pub fn take_screenshot(mode: &str) -> Option<PathBuf> {
    util::ensure_dir(&util::screenshots_dir()).ok()?;
    let dir = util::screenshots_dir();
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("nova-{ts}.png"));

    #[cfg(target_os = "linux")]
    {
        let sess = session_type();
        if mode == "auto" && sess == "wayland" && run_capture("grim", &[path.to_str()?]).is_ok()
        {
            return Some(path);
        }
        // Don't auto-run grim on x11.
        let tools: &[&str] = match mode {
            "grim" => &["grim"],
            "gnome-screenshot" => &["gnome-screenshot"],
            "scrot" => &["scrot"],
            "import" => &["import"],
            _ if sess == "wayland" => &["grim"],
            _ => &["gnome-screenshot", "scrot", "import"],
        };
        for tool in tools {
            let status = match *tool {
                "grim" => run_capture(tool, &[path.to_str().unwrap_or("")]),
                "gnome-screenshot" => run_capture(tool, &["-f", path.to_str().unwrap_or("")]),
                "scrot" => run_capture(tool, &[path.to_str().unwrap_or("")]),
                "import" => run_capture(tool, &["-window", "root", path.to_str().unwrap_or("")]),
                _ => Ok(String::new()),
            };
            if status.is_ok() {
                return Some(path);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Networking / Bluetooth
// ---------------------------------------------------------------------------

/// Connected Bluetooth device names via `bluetoothctl`.
pub fn bluetooth_devices() -> Vec<String> {
    let Ok(out) = run_capture("bluetoothctl", &["devices", "Connected"]) else {
        return vec![];
    };
    out.lines()
        .filter_map(|l| {
            l.split_whitespace()
                .last()
                .map(|s| s.to_string())
        })
        .collect()
}

/// Trigger a `bluetoothctl` scan refresh (best-effort).
pub fn bluetooth_refresh() -> Result<()> {
    run_capture("bluetoothctl", &["scan", "on"])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Date & time
// ---------------------------------------------------------------------------

pub fn human_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_uptime_format() {
        assert_eq!(human_uptime(45), "0m");
        assert_eq!(human_uptime(7200), "2h 0m");
        assert_eq!(human_uptime(90000), "1d 1h");
    }
}