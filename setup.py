#!/usr/bin/env python3
"""NovaShell setup: install Rust, system deps, then build and install.

Usage:
    python3 setup.py                # full: deps + rust + build + install
    python3 setup.py --user         # per-user install (deps still system-wide)
    python3 setup.py --check        # inspect-only, touches nothing
    python3 setup.py --uninstall    # remove a previous install

Bootstrap (runs automatically if missing):
    - installs rustup + stable Rust via https://sh.rustup.rs
    - installs apt build packages:
        build-essential pkg-config libgtk-4-dev libwebkitgtk-6.0-dev
        libjavascriptcoregtk-6.0-dev libudev-dev

Safe by design (same guarantees as scripts/install.sh):
    - never modifies GRUB, kernel params, initramfs, /etc/fstab,
      display managers, or GNOME configuration
    - GNOME stays installed and reachable as the desktop fallback
    - only writes to standard app / icon / autostart / systemd locations

Requires: python3 >= 3.8, curl (installed automatically), sudo for apt.
"""

import argparse
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path

APP_NAME = "novashell"
BIN_NAME = "novashell"
ICON_SRC = "assets/org.novashell.svg"
DESKTOP_SRC = "data/novashell.desktop"
AUTOSTART_SRC = "data/autostart/novashell.desktop"
SERVICE_SRC = "data/systemd/novashell.service"

BUILD_DEPS = [
    "curl",
    "build-essential",
    "pkg-config",
    "libgtk-4-dev",
    "libwebkitgtk-6.0-dev",
    "libjavascriptcoregtk-6.0-dev",
    "libudev-dev",
]

# Runtime helpers used opportunistically (UI degrades if absent).
RUNTIME_HINTS = [
    ("wpctl", "volume control (WirePlumber/PipeWire)"),
    ("brightnessctl", "backlight control"),
    ("grim", "Wayland screenshots"),
    ("gnome-screenshot", "X11 screenshots"),
    ("nmcli", "Wi-Fi SSID in status bar"),
    ("bluetoothctl", "Bluetooth device list"),
    ("systemctl", "power actions"),
    ("gnome-session-quit", "log out action"),
]

CARGO_HOME = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")))
CARGO_BIN = CARGO_HOME / "bin"
CARGO = CARGO_BIN / "cargo"


def log(msg: str) -> None:
    print(f"==> {msg}", flush=True)


def warn(msg: str) -> None:
    print(f"    WARNING: {msg}", flush=True)


def run(cmd, **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, check=False, **kw)


def which(bin: str) -> Path | None:
    p = shutil.which(bin)
    return Path(p) if p else None


def is_deb_based() -> bool:
    os_release = Path("/etc/os-release")
    text = os_release.read_text() if os_release.exists() else ""
    return os.path.exists("/etc/debian_version") or "debian" in text.lower() or "ubuntu" in text.lower()


def project_root() -> Path:
    """Repo root = the folder containing this setup.py."""
    return Path(__file__).resolve().parent


def ensure_on_path() -> None:
    """Make ~/.cargo/bin visible to this process and children."""
    bin_dir = str(CARGO_BIN)
    if bin_dir not in os.environ.get("PATH", ""):
        os.environ["PATH"] = bin_dir + os.pathsep + os.environ.get("PATH", "")


def ensure_rust() -> None:
    """Install rustup + stable Rust if cargo is missing; refresh PATH."""
    ensure_on_path()
    if which("cargo") is not None:
        ver = subprocess.run(["rustc", "--version"], capture_output=True, text=True).stdout.strip()
        log(f"rustc found: {ver}")
        return
    if which("rustup") is not None:
        log("rustup present; installing stable toolchain")
        if run(["rustup", "default", "stable"], cwd=str(project_root())).returncode != 0:
            print("    rustup default failed. Run manually and retry.")
            sys.exit(1)
        ensure_on_path()
        return
    log("Rust not found; installing rustup + stable (curl | sh)")
    curl = which("curl")
    if curl is None:
        print("    curl is required to install rustup. Install it first:")
        print("        sudo apt update && sudo apt install -y curl")
        sys.exit(1)
    script = subprocess.Popen(
        # -y (default toolchain stable), --profile minimal still gives cargo.
        [str(curl), "--proto", "=https", "--tlsv1.2", "-sSf", "https://sh.rustup.rs"],
        stdout=subprocess.PIPE,
        stdin=subprocess.PIPE,
        text=True,
    )
    stdout, _ = script.communicate(input="-y\n")
    if script.returncode != 0:
        print("    rustup install failed (exit %d). See output above." % script.returncode)
        sys.exit(1)
    ensure_on_path()
    ver = subprocess.run([str(CARGO), "--version"], capture_output=True, text=True).stdout.strip()
    log(f"installed: {ver}")


def require_sudo() -> None:
    """Ensure we can run sudo; prompts for the password when needed."""
    if os.geteuid() == 0:
        return
    r = run(["sudo", "-n", "true"])
    if r.returncode != 0:
        print("    sudo password needed for system packages.")
        run(["sudo", "true"], check=True)


def ensure_system_deps() -> None:
    """apt update + install BUILD_DEPS (skipped on non-deb systems)."""
    if not is_deb_based():
        warn("Not a Debian/Ubuntu OS; install these yourself:")
        print("        " + " ".join(BUILD_DEPS))
        return
    if which("apt-get") is None:
        warn("apt-get not found; skipping system dependency install.")
        return
    require_sudo()
    log("installing system packages (apt)")
    cmd = [] if os.geteuid() == 0 else ["sudo"]
    log("apt update")
    run(cmd + ["apt-get", "update"])
    log("apt install: " + " ".join(BUILD_DEPS))
    r = run(cmd + ["apt-get", "install", "-y"] + BUILD_DEPS)
    if r.returncode != 0:
        print("    apt install failed; cannot continue.")
        sys.exit(1)


def check_dev_headers() -> None:
    if which("pkg-config") is None:
        print("    pkg-config not found; rerun setup (system deps step installs it).")
        sys.exit(1)
    missing = []
    for pkg in ("gtk4", "javascriptcoregtk-6.0", "webkitgtk-6.0"):
        if run(["pkg-config", "--exists", pkg]).returncode != 0:
            missing.append(pkg)
    if missing:
        print(f"    missing dev packages: {', '.join(missing)}")
        print("        Re-run: python3 setup.py   (installs them via apt)")
        sys.exit(1)
    for pkg in ("gtk4", "javascriptcoregtk-6.0", "webkitgtk-6.0"):
        v = subprocess.run(
            ["pkg-config", "--modversion", pkg], capture_output=True, text=True
        ).stdout.strip()
        print(f"    found: {pkg} {v}")


def check_rust_version() -> None:
    """Enforce rustc >= 1.85 (webkit6 needs edition 2024)."""
    import re

    ver = subprocess.run(["rustc", "--version"], capture_output=True, text=True).stdout.strip()
    m = re.search(r"rustc (\d+)\.(\d+)", ver)
    if not m:
        print("    cannot parse rustc version:", ver)
        sys.exit(1)
    maj, minor = int(m.group(1)), int(m.group(2))
    if maj > 1 or (maj == 1 and minor >= 85):
        return
    print(f"    rustc {maj}.{minor} is too old (need >= 1.85). Updating: rustup update stable")
    r = run(["rustup", "update", "stable"])
    if r.returncode != 0:
        sys.exit(1)


def build() -> None:
    log("building release binary (cargo build --release)")
    r = run([str(CARGO), "build", "--release"], cwd=str(project_root()))
    if r.returncode != 0:
        print("    build failed. See output above.")
        sys.exit(1)
    bin_path = project_root() / "target" / "release" / BIN_NAME
    if not bin_path.exists():
        print(f"    expected binary not found: {bin_path}")
        sys.exit(1)


def install_files(user: bool) -> None:
    home = Path.home()
    if user:
        bin_dir = Path(os.environ.get("NOVA_BIN_DIR", str(home / ".local" / "bin")))
        apps_dir = home / ".local" / "share" / "applications"
        icons_dir = home / ".local" / "share" / "icons" / "hicolor" / "scalable" / "apps"
        autostart_dir = home / ".config" / "autostart"
        systemd_dir = home / ".config" / "systemd" / "user"
    else:
        if os.geteuid() != 0:
            print("System-wide install needs root. Re-run: sudo python3 setup.py  (or use --user)")
            sys.exit(1)
        bin_dir = Path("/usr/local/bin")
        apps_dir = Path("/usr/local/share/applications")
        icons_dir = Path("/usr/local/share/icons/hicolor/scalable/apps")
        autostart_dir = Path("/etc/xdg/autostart")
        systemd_dir = Path("/etc/systemd/user")

    for d in (bin_dir, apps_dir, icons_dir, autostart_dir, systemd_dir):
        d.mkdir(parents=True, exist_ok=True)

    root = project_root()
    shutil.copy2(root / "target" / "release" / BIN_NAME, bin_dir / BIN_NAME)
    os.chmod(bin_dir / BIN_NAME, 0o755)
    shutil.copy2(root / ICON_SRC, icons_dir / "org.novashell.svg")
    shutil.copy2(root / DESKTOP_SRC, apps_dir / "novashell.desktop")

    autostart = (autostart_dir / "novashell.desktop").read_text()
    autostart = autostart.replace("@BINARY@", str(bin_dir / BIN_NAME))
    (autostart_dir / "novashell.desktop").write_text(autostart)

    service = (systemd_dir / "novashell.service").read_text()
    service = service.replace("@BINARY@", str(bin_dir / BIN_NAME))
    (systemd_dir / "novashell.service").write_text(service)

    log("installed files:")
    for f in (
        bin_dir / BIN_NAME,
        apps_dir / "novashell.desktop",
        icons_dir / "org.novashell.svg",
        autostart_dir / "novashell.desktop",
        systemd_dir / "novashell.service",
    ):
        print(f"    {f}")

    if which("gtk-update-icon-cache"):
        run(["gtk-update-icon-cache", str(icons_dir.parent.parent.parent)], check=False)
    if which("systemctl"):
        run(["systemctl", "--user", "daemon-reload"], check=False)


def uninstall(user: bool) -> None:
    home = Path.home()
    if user:
        bin_dir = Path(os.environ.get("NOVA_BIN_DIR", str(home / ".local" / "bin")))
        apps_dir = home / ".local" / "share" / "applications"
        icons_dir = home / ".local" / "share" / "icons" / "hicolor" / "scalable" / "apps"
        autostart_dir = home / ".config" / "autostart"
        systemd_dir = home / ".config" / "systemd" / "user"
    else:
        if os.geteuid() != 0:
            print("System-wide uninstall needs root. Re-run: sudo python3 setup.py --uninstall")
            sys.exit(1)
        bin_dir, apps_dir = Path("/usr/local/bin"), Path("/usr/local/share/applications")
        icons_dir = Path("/usr/local/share/icons/hicolor/scalable/apps")
        autostart_dir, systemd_dir = Path("/etc/xdg/autostart"), Path("/etc/systemd/user")

    for f in (
        bin_dir / BIN_NAME,
        apps_dir / "novashell.desktop",
        icons_dir / "org.novashell.svg",
        autostart_dir / "novashell.desktop",
        systemd_dir / "novashell.service",
    ):
        if f.exists():
            f.unlink()
            print(f"    removed {f}")
    if which("systemctl"):
        run(["systemctl", "--user", "daemon-reload"], check=False)


def check_mode() -> None:
    log("checking environment (no changes made)")
    check_rust_version()
    check_dev_headers()
    for tool, use in RUNTIME_HINTS:
        print(f"    {'OK ' if which(tool) else '---'} {tool:16s} {use}")
    print("    Environment check done.")


def main() -> None:
    ap = argparse.ArgumentParser(description="Bootstrap, build and install NovaShell")
    ap.add_argument("--user", action="store_true", help="per-user install")
    ap.add_argument("--check", action="store_true", help="verify only, change nothing")
    ap.add_argument("--uninstall", action="store_true", help="remove NovaShell")
    args = ap.parse_args()

    if platform.system() != "Linux":
        print("NovaShell targets Ubuntu/Debian Linux. This looks like:", platform.system())
        sys.exit(1)

    if args.check:
        check_mode()
        return
    if args.uninstall:
        uninstall(user=args.user)
        log("NovaShell removed.")
        return

    log(f"NovaShell setup ({'user' if args.user else 'system'})")
    ensure_rust()
    ensure_system_deps()
    check_rust_version()
    check_dev_headers()
    build()
    install_files(user=args.user)
    print()
    log("Installed.")
    print("    Test once now:   novashell --windowed")
    print("    Autostart next login:")
    print("        systemctl --user enable novashell")
    print("        systemctl --user start novashell")
    print("    Launch from GNOME:  Activities -> NovaShell")
    print("    Revert:             python3 setup.py --uninstall")


if __name__ == "__main__":
    main()